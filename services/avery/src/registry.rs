use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::PathBuf,
    sync::{Arc, RwLock},
};

use futures::{Stream, StreamExt};
use regex::Regex;
use semver::{Version, VersionReq};
use sha2::{Digest, Sha256};
use slog::{info, warn, Logger};
use tempfile::NamedTempFile;
use uuid::Uuid;

use gbk_protocols::{
    functions::{
        functions_registry_server::FunctionsRegistry, AttachmentStreamUpload, AttachmentUpload,
        AttachmentUploadResponse, AuthMethod, ExecutionEnvironment, Function as ProtoFunction,
        FunctionAttachment, FunctionAttachmentId, FunctionDescriptor, FunctionId, FunctionInput,
        FunctionOutput, ListRequest, OrderingDirection, OrderingKey, RegisterAttachmentRequest,
        RegisterRequest, RegistryListResponse,
    },
    tonic,
};

#[derive(Debug, Clone)]
pub struct FunctionsRegistryService {
    functions: Arc<RwLock<HashMap<Uuid, Function>>>,
    function_attachments: Arc<RwLock<HashMap<Uuid, (FunctionAttachment, PathBuf)>>>,
    logger: Logger,
}

#[derive(Debug)]
struct Function {
    id: Uuid,
    name: String,
    version: Version,
    execution_environment: ExecutionEnvironment,
    inputs: Vec<FunctionInput>,
    outputs: Vec<FunctionOutput>,
    metadata: HashMap<String, String>,
    code: Option<FunctionAttachmentId>,
    attachments: Vec<FunctionAttachmentId>,
}

impl Drop for FunctionsRegistryService {
    fn drop(&mut self) {
        match self.function_attachments.write() {
            Err(e) => warn!(
                self.logger,
                "Failed to acquire write lock for cleaning up attachments: {}. \
                            Attachments will be leaked (restart computer to clean up)",
                e
            ),
            Ok(mut fas) => {
                if !fas.is_empty() {
                    info!(
                        self.logger,
                        "🧹 Shutting down registry, cleaning up attachments..."
                    );
                    fas.drain().for_each(|(_id, (att, att_path))| {
                        fs::remove_file(&att_path).map_or_else(
                            |e| {
                                warn!(
                                    self.logger,
                                    "Failed to clean up attachment \"{}\" at \"{}\": {}",
                                    att.name,
                                    att_path.display(),
                                    e
                                )
                            },
                            |_| (),
                        )
                    });
                    info!(self.logger, "🧹💨  Function attachments cleaned up");
                }
            }
        }
    }
}

impl FunctionsRegistryService {
    pub fn new(logger: Logger) -> Self {
        Self {
            functions: Arc::new(RwLock::new(HashMap::new())),
            function_attachments: Arc::new(RwLock::new(HashMap::new())),
            logger,
        }
    }

    pub async fn upload_stream_attachment<S>(
        &self,
        attachment_stream_upload_request: tonic::Request<S>,
    ) -> Result<tonic::Response<AttachmentUploadResponse>, tonic::Status>
    where
        S: std::marker::Unpin + Stream<Item = Result<AttachmentStreamUpload, tonic::Status>>,
    {
        let mut stream = attachment_stream_upload_request.into_inner();

        let mut hasher = Sha256::new();
        let mut maybe_attachment: Option<FunctionAttachment> = None;
        let mut maybe_file: Option<(Result<fs::File, tonic::Status>, PathBuf)> = None;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| {
                warn!(self.logger, "Error reading attachment upload chunk: {}", e);
                maybe_file
                    .take()
                    .map(|(_, path)| {
                        std::fs::remove_file(&path).map_or_else(
                            |e| {
                                warn!(
                                    self.logger,
                                    "Failed to remove partially uploaded file \"{}\": {}",
                                    path.display(),
                                    e
                                );
                            },
                            |_| {
                                info!(
                                    self.logger,
                                    "Removed partially uploaded file \"{}\"",
                                    path.display()
                                );
                            },
                        )
                    })
                    .unwrap_or_default();
                e
            })?;

            let (attachment, path) = chunk
                .id
                .ok_or_else(|| {
                    tonic::Status::new(
                        tonic::Code::InvalidArgument,
                        "Failed to get function attachment with None as id. 🤷".to_owned(),
                    )
                })
                .and_then(|idd| self.get_attachment(&idd))?;

            // Make sure we only open the file once and re-use the file handle for later writes.
            // Since we get the path inside the chunk we got no other option but to open the file
            // inside the scope
            let file = match *maybe_file.get_or_insert_with(|| {
                (
                    fs::OpenOptions::new()
                        .create(true)
                        .write(true)
                        .open(&path)
                        .map_err(|e| {
                            tonic::Status::new(
                                tonic::Code::Internal,
                                format!("Failed to open attachment file {}: {}", path.display(), e),
                            )
                        }),
                    path.clone(),
                )
            }) {
                (Ok(ref mut f), _) => Ok(f),
                (Err(ref mut e), _) => Err(e.clone()),
            }?;

            hasher.update(&chunk.content);
            file.write(&chunk.content).map_err(|e| {
                tonic::Status::new(
                    tonic::Code::Internal,
                    format!(
                        "Failed to save the attachment in {} 🐼: {}",
                        path.display(),
                        e
                    ),
                )
            })?;

            maybe_attachment = maybe_attachment.or_else(|| Some(attachment.clone()));
        }

        // validate integrity of uploaded file
        let uploaded_content_checksum = hasher.finalize();
        maybe_attachment
            .as_ref()
            .and_then(|a| a.checksums.as_ref())
            .ok_or_else(|| {
                tonic::Status::new(
                    tonic::Code::Internal,
                    "Attachment is missing checksums. This should have been validated when registering 🤷".to_string(),
                )
            })
            .and_then(|checksums| {
                if &uploaded_content_checksum[..]
                    != hex::decode(&checksums.sha256)
                        .unwrap_or_default()
                        .as_slice()
                {
                    Err(tonic::Status::new(
                        tonic::Code::InvalidArgument,
                        format!(
                            "Uploaded attachment checksum mismatch. Registered with: {}, got from uploaded content: {}",
                            &checksums.sha256,
                            hex::encode(uploaded_content_checksum)
                        ),
                    ))
                } else {
                    Ok(())
                }
            })?;

        Ok(tonic::Response::new(AttachmentUploadResponse {
            url: maybe_attachment
                .as_ref()
                .map(|att| att.url.clone())
                .unwrap_or_default(),
            auth_method: AuthMethod::None as i32,
        }))
    }

    fn get_attachment(
        &self,
        id: &FunctionAttachmentId,
    ) -> Result<(FunctionAttachment, PathBuf), tonic::Status> {
        Uuid::parse_str(&id.id)
            .map_err(|e| {
                tonic::Status::new(
                    tonic::Code::InvalidArgument,
                    format!("failed to parse UUID from attachment id: {}", e),
                )
            })
            .and_then(|attachment_id| {
                self.function_attachments
                    .read()
                    .map_err(|e| {
                        tonic::Status::new(
                            tonic::Code::Internal,
                            format!("Failed to get write lock for function attachments: {}", e),
                        )
                    })
                    .map(|function_attachments| (function_attachments, attachment_id))
            })
            .and_then(|(function_attachments, attachment_id)| {
                function_attachments
                    .get(&attachment_id)
                    .ok_or_else(|| {
                        tonic::Status::new(
                            tonic::Code::NotFound,
                            format!("failed to find attachment with id: {}", attachment_id),
                        )
                    })
                    .map(|(attachment, path)| (attachment.clone(), path.clone()))
            })
    }

    fn get_function_descriptor(&self, f: &Function) -> Result<FunctionDescriptor, tonic::Status> {
        let code = f
            .code
            .clone()
            .map(|c| self.get_attachment(&c))
            .map_or(Ok(None), |r| r.map(|(attach, _)| Some(attach)))?;

        let attachments = f
            .attachments
            .iter()
            .map(|v| self.get_attachment(v).map(|(attach, _)| attach))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(FunctionDescriptor {
            function: Some(ProtoFunction {
                id: Some(FunctionId {
                    value: f.id.to_string(),
                }),
                name: f.name.clone(),
                version: f.version.to_string(),
                metadata: f.metadata.clone(),
                inputs: f.inputs.clone(),
                outputs: f.outputs.clone(),
            }),
            execution_environment: Some(f.execution_environment.clone()),
            code,
            attachments,
        })
    }
}

#[tonic::async_trait]
impl FunctionsRegistry for FunctionsRegistryService {
    async fn list(
        &self,
        list_request: tonic::Request<ListRequest>,
    ) -> Result<tonic::Response<RegistryListResponse>, tonic::Status> {
        let reader = self.functions.read().map_err(|e| {
            tonic::Status::new(
                tonic::Code::Internal,
                format!("Failed to get read lock for functions: {}", e),
            )
        })?;

        let payload = list_request.into_inner();
        let required_metadata = if payload.metadata_filter.is_empty() {
            None
        } else {
            Some(payload.metadata_filter.clone())
        };
        let required_metadata_keys = if payload.metadata_key_filter.is_empty() {
            None
        } else {
            Some(payload.metadata_key_filter.clone())
        };
        let offset: usize = payload.offset as usize;
        let limit: usize = payload.limit as usize;
        let version_req = payload
            .version_requirement
            .clone()
            .map(|vr| {
                VersionReq::parse(&vr.expression).map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::InvalidArgument,
                        format!("Supplied version requirement is invalid: {}", e),
                    )
                })
            })
            .map_or(Ok(None), |v| v.map(Some))?;
        let mut filtered_functions = reader
            .values()
            .filter(|func| {
                func.name.contains(&payload.name_filter)
                    && version_req
                        .as_ref()
                        .map_or(true, |ver_req| ver_req.matches(&func.version))
                    && required_metadata.as_ref().map_or(true, |filters| {
                        filters.iter().all(|filter| {
                            func.metadata
                                .iter()
                                .any(|(k, v)| filter.0 == k && filter.1 == v)
                        })
                    })
                    && required_metadata_keys.as_ref().map_or(true, |keys| {
                        keys.iter().all(|key| func.metadata.contains_key(key))
                    })
            })
            .collect::<Vec<&Function>>();
        filtered_functions.sort_unstable_by(|a, b| {
            match (
                OrderingKey::from_i32(payload.order_by),
                OrderingDirection::from_i32(payload.order_direction),
            ) {
                (Some(OrderingKey::Name), Some(OrderingDirection::Ascending))
                | (Some(OrderingKey::Name), None)
                | (None, None)
                | (None, Some(OrderingDirection::Ascending)) => match a.name.cmp(&b.name) {
                    std::cmp::Ordering::Equal => b.version.cmp(&a.version),
                    o => o,
                },
                (Some(OrderingKey::Name), Some(OrderingDirection::Descending))
                | (None, Some(OrderingDirection::Descending)) => match b.name.cmp(&a.name) {
                    std::cmp::Ordering::Equal => b.version.cmp(&a.version),
                    o => o,
                },
            }
        });

        Ok(tonic::Response::new(RegistryListResponse {
            functions: filtered_functions
                .iter()
                .skip(offset)
                .take(limit)
                .filter_map(|f| self.get_function_descriptor(*f).ok())
                .collect::<Vec<_>>(),
        }))
    }

    async fn get(
        &self,
        function_id_request: tonic::Request<FunctionId>,
    ) -> Result<tonic::Response<FunctionDescriptor>, tonic::Status> {
        let fn_id = function_id_request.into_inner();

        let reader = self.functions.read().map_err(|e| {
            tonic::Status::new(
                tonic::Code::Internal,
                format!("Failed to get read lock for functions: {}", e),
            )
        })?;

        Uuid::parse_str(&fn_id.value)
            .map_err(|e| {
                tonic::Status::new(
                    tonic::Code::InvalidArgument,
                    format!("failed to parse UUID from function id: {}", e),
                )
            })
            .and_then(|fun_uuid| {
                reader.get(&fun_uuid).ok_or_else(|| {
                    tonic::Status::new(
                        tonic::Code::NotFound,
                        format!("failed to find function with id: {}", fun_uuid),
                    )
                })
            })
            .and_then(|f| self.get_function_descriptor(f))
            .map(tonic::Response::new)
    }

    async fn register(
        &self,
        register_request: tonic::Request<RegisterRequest>,
    ) -> Result<tonic::Response<FunctionId>, tonic::Status> {
        let payload = register_request.into_inner();

        validate_name(&payload.name).map_err(|e| {
            tonic::Status::new(
                tonic::Code::InvalidArgument,
                format!("Invalid function name \"{}\": {}", payload.name, e),
            )
        })?;

        let mut version = validate_version(&payload.version).map_err(|e| {
            tonic::Status::new(
                tonic::Code::InvalidArgument,
                format!("Invalid function version \"{}\": {}", payload.version, e),
            )
        })?;

        let mut functions = self.functions.write().map_err(|e| {
            tonic::Status::new(
                tonic::Code::Internal,
                format!("Failed to get write lock for functions: {}", e),
            )
        })?;

        // this is the local case, always add dev to any function version
        version
            .pre
            .push(semver::Identifier::AlphaNumeric("dev".to_owned()));

        // remove function if name and version matches (after the -dev has been appended)
        // TODO: Remove corresponding attachments
        functions.retain(|_, v| v.name != payload.name || v.version != version);

        let execution_environment = payload.execution_environment.ok_or_else(|| {
            tonic::Status::new(
                tonic::Code::InvalidArgument,
                String::from("Execution environment is required when registering function"),
            )
        })?;

        // validate attachments
        payload
            .attachment_ids
            .iter()
            .chain(payload.code.iter())
            .fold(Ok(()), |r, id| match self.get_attachment(id) {
                Ok(_) => r,
                Err(e) => match r {
                    Ok(_) => Err(format!("{} ({})", id.id, e.message())),
                    Err(e2) => Err(format!("{}, {} ({})", e2, id.id, e.message())),
                },
            })
            .map_err(|msg| {
                tonic::Status::new(
                    tonic::Code::InvalidArgument,
                    format!("Failed to get attachment for ids: [{}]", msg),
                )
            })?;

        let id = Uuid::new_v4();
        functions.insert(
            id,
            Function {
                id,
                name: payload.name,
                version,
                execution_environment,
                metadata: payload.metadata,
                inputs: payload.inputs,
                outputs: payload.outputs,
                code: payload.code,
                attachments: payload.attachment_ids,
            },
        );

        Ok(tonic::Response::new(FunctionId {
            value: id.to_string(),
        }))
    }

    async fn register_attachment(
        &self,
        register_attachment_request: tonic::Request<RegisterAttachmentRequest>,
    ) -> Result<tonic::Response<FunctionAttachmentId>, tonic::Status> {
        let payload = register_attachment_request.into_inner();

        if payload.name.is_empty() {
            return Err(tonic::Status::new(
                tonic::Code::InvalidArgument,
                String::from("Name is required when registering attachment"),
            ));
        }

        let checksum = payload.checksums.ok_or_else(|| {
            tonic::Status::new(
                tonic::Code::InvalidArgument,
                String::from("Checksum is required when registering attachment"),
            )
        })?;

        let mut function_attachments = self.function_attachments.write().map_err(|e| {
            tonic::Status::new(
                tonic::Code::Internal,
                format!("Failed to get write lock for function attachments: {}", e),
            )
        })?;

        let attachment_file = NamedTempFile::new().map_err(|e| {
            tonic::Status::new(
                tonic::Code::Internal,
                format!("Failed to create temp file to save attachment in 😿: {}", e),
            )
        })?;

        let (_, path) = attachment_file.keep().map_err(|e| {
            tonic::Status::new(
                tonic::Code::Internal,
                format!("Failed to persist temp file with attachment: {}", e),
            )
        })?;

        let id = Uuid::new_v4();
        function_attachments.insert(
            id,
            (
                FunctionAttachment {
                    id: Some(FunctionAttachmentId { id: id.to_string() }),
                    name: payload.name,
                    url: format!("file://{}", path.display()),
                    metadata: payload.metadata,
                    checksums: Some(checksum),
                },
                path,
            ),
        );

        Ok(tonic::Response::new(FunctionAttachmentId {
            id: id.to_string(),
        }))
    }

    async fn upload_streamed_attachment(
        &self,
        attachment_stream_upload_request: tonic::Request<tonic::Streaming<AttachmentStreamUpload>>,
    ) -> Result<tonic::Response<AttachmentUploadResponse>, tonic::Status> {
        // TODO: use metadata for "global" upload data such as FunctionAttachmentId
        self.upload_stream_attachment(attachment_stream_upload_request)
            .await
    }

    async fn upload_attachment_url(
        &self,
        _: tonic::Request<AttachmentUpload>,
    ) -> Result<tonic::Response<AttachmentUploadResponse>, tonic::Status> {
        Err(tonic::Status::new(
            tonic::Code::Unimplemented,
            "The Avery registry does not support uploading via URL. Use streaming upload instead."
                .to_owned(),
        ))
    }
}

fn validate_name(name: &str) -> Result<(), String> {
    const MAX_LEN: usize = 128;
    const MIN_LEN: usize = 3;
    if name.len() > MAX_LEN {
        Err(format!(
            "Function name is too long! Max {} characters",
            MAX_LEN
        ))
    } else if name.len() < MIN_LEN {
        Err(format!(
            "Function name is too short! Min {} characters",
            MIN_LEN
        ))
    } else {
        let regex = Regex::new(r"^[a-z][a-z0-9]{1,}([a-z0-9\-]?[a-z0-9]+)+$|^[a-z][a-z0-9]{2,}$")
            .map_err(|e| format!("Invalid regex: {}", e))?;
        if regex.is_match(name) {
            Ok(())
        } else {
            Err(String::from("Name contains invalid characters. Only lower case characters, numbers and dashes are allowed"))
        }
    }
}

fn validate_version(version: &str) -> Result<Version, String> {
    if version.is_empty() {
        return Err(String::from(
            "Version cannot be empty when registering function",
        ));
    }

    Version::parse(version).map_err(|e| format!("Invalid semantic version specified: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_validate_name() {
        assert!(validate_name("a").is_err());
        assert!(validate_name("ab").is_err());
        assert!(validate_name("abc").is_ok());
        assert!(validate_name("-ab").is_err());
        assert!(validate_name("ab-").is_err());
        assert!(validate_name("ab-c").is_ok());
        assert!(validate_name("ab-C").is_err());
        assert!(validate_name("1ab").is_err());
        assert!(validate_name("a1b").is_ok());
        assert!(validate_name("ab1").is_ok());
        assert!(validate_name(&vec!['a'; 129].iter().collect::<String>()).is_err());
        assert!(validate_name("abc!").is_err());
        assert!(validate_name("😭").is_err());
    }

    #[test]
    fn test_validate_version() {
        assert!(validate_version("").is_err());
        assert!(validate_version("1.0,3").is_err());
        assert!(validate_version("1.0.5-alpha").is_ok());
    }
}
