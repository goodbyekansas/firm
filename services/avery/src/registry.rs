use std::{
    collections::{hash_map::Entry, HashMap},
    fs,
    io::Write,
    path::PathBuf,
    sync::{Arc, RwLock},
};

use either::Either;
use futures::{Stream, StreamExt};
use regex::Regex;
use semver::{Version, VersionReq};
use sha2::{Digest, Sha256};
use slog::{debug, info, warn, Logger};
use tempfile::TempDir;
use uuid::Uuid;

use crate::config::InternalRegistryConfig;
use firm_types::{
    functions::{
        registry_server::Registry, Attachment, AttachmentData, AttachmentHandle, AttachmentId,
        AttachmentStreamUpload, AttachmentUrl, AuthMethod, ChannelSpec, Filters,
        Function as ProtoFunction, FunctionData, FunctionId, Functions, Ordering, OrderingKey,
        Publisher as ProtoPublisher, RuntimeSpec, Signature,
    },
    tonic,
};

#[derive(Debug, Clone)]
pub struct RegistryService {
    functions: Arc<RwLock<Vec<Function>>>,
    function_attachments: Arc<RwLock<HashMap<Uuid, Attachment>>>,
    config: InternalRegistryConfig,
    logger: Logger,
    attachment_directory: Arc<TempDir>,
}

#[derive(Debug, Clone)]
struct Publisher {
    name: String,
    email: String,
}

#[derive(Debug, Clone)]
struct Function {
    name: String,
    created_at: u64,
    version: Version,
    runtime: RuntimeSpec,
    required_inputs: HashMap<String, ChannelSpec>,
    optional_inputs: HashMap<String, ChannelSpec>,
    outputs: HashMap<String, ChannelSpec>,
    code: Option<AttachmentId>,
    attachments: Vec<AttachmentId>,
    metadata: HashMap<String, String>,
    publisher: Publisher,
    signature: Option<Vec<u8>>,
}

impl RegistryService {
    pub fn new(config: InternalRegistryConfig, logger: Logger) -> Result<Self, std::io::Error> {
        Ok(Self {
            functions: Arc::new(RwLock::new(Vec::new())),
            function_attachments: Arc::new(RwLock::new(HashMap::new())),
            config,
            logger,
            attachment_directory: Arc::new(
                tempfile::Builder::new()
                    .prefix("avery-registry-attachments-")
                    .tempdir()?,
            ),
        })
    }

    pub async fn upload_stream_attachment<S>(
        &self,
        attachment_stream_upload_request: tonic::Request<S>,
    ) -> Result<tonic::Response<firm_types::functions::Nothing>, tonic::Status>
    where
        S: std::marker::Unpin + Stream<Item = Result<AttachmentStreamUpload, tonic::Status>>,
    {
        let mut stream = attachment_stream_upload_request.into_inner();

        let mut hasher = Sha256::new();
        let mut maybe_attachment: Option<Attachment> = None;
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
                .and_then(|idd| {
                    self.get_attachment(&idd)
                        .map(|a| (a, self.attachment_directory.path().join(idd.uuid)))
                })?;

            // Make sure we only open the file once and re-use the file handle for later writes.
            // Since we get the path inside the chunk we got no other option but to open the file
            // inside the scope
            let file = match maybe_file.get_or_insert_with(|| {
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
                (Ok(f), _) => Ok(f),
                (Err(e), _) => Err(tonic::Status::new(e.code(), e.message())),
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

        Ok(tonic::Response::new(firm_types::functions::Nothing {}))
    }

    fn get_attachment(&self, id: &AttachmentId) -> Result<Attachment, tonic::Status> {
        Uuid::parse_str(&id.uuid)
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
                    .map(|attachment| attachment.clone())
            })
    }

    fn get_function(&self, f: &Function) -> Result<ProtoFunction, tonic::Status> {
        let code = f
            .code
            .clone()
            .map(|c| self.get_attachment(&c))
            .map_or(Ok(None), |r| r.map(Some))?;

        let attachments = f
            .attachments
            .iter()
            .map(|v| self.get_attachment(v))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(ProtoFunction {
            name: f.name.clone(),
            version: f.version.to_string(),
            metadata: f.metadata.clone(),
            required_inputs: f.required_inputs.clone(),
            optional_inputs: f.optional_inputs.clone(),
            outputs: f.outputs.clone(),
            runtime: Some(f.runtime.clone()),
            code,
            attachments,
            created_at: f.created_at,
            publisher: Some(ProtoPublisher {
                name: f.publisher.name.clone(),
                email: f.publisher.email.clone(),
            }),
            signature: f.signature.as_ref().map(|sig| Signature {
                signature: sig.to_vec(),
            }),
        })
    }

    fn list(&self, filters: Filters, group_by_version: bool) -> Result<Functions, tonic::Status> {
        let reader = self.functions.read().map_err(|e| {
            tonic::Status::new(
                tonic::Code::Internal,
                format!("Failed to get read lock for functions: {}", e),
            )
        })?;

        let required_metadata = if filters.metadata.is_empty() {
            None
        } else {
            Some(filters.metadata.clone())
        };

        let name_filter = filters.name;

        let order = filters.order.unwrap_or(Ordering {
            key: OrderingKey::NameVersion as i32,
            reverse: false,
            offset: 0,
            limit: 100,
        });

        let offset: usize = order.offset as usize;
        let limit: usize = order.limit as usize;
        let version_req = filters
            .version_requirement
            .map(|vr| {
                VersionReq::parse_compat(&vr.expression, semver::Compat::Npm).map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::InvalidArgument,
                        format!("Supplied version requirement is invalid: {}", e),
                    )
                })
            })
            .map_or(Ok(None), |v| v.map(Some))?;
        let filtered_functions = reader.iter().filter(|func| {
            (match group_by_version {
                false => func.name == name_filter,
                true => func.name.contains(&name_filter),
            }) && version_req.as_ref().map_or(true, |ver_req| {
                let res = ver_req.matches(&func.version);
                debug!(
                    self.logger,
                    "Matching \"{}\" with \"{}\": {}", &ver_req, &func.version, res,
                );
                res
            })
        });
        let mut filtered_functions = (if group_by_version {
            Either::Left(
                filtered_functions
                    .fold(
                        HashMap::new(),
                        |mut map: HashMap<String, &Function>, function| match map
                            .entry(function.name.clone())
                        {
                            Entry::Occupied(mut entry) => {
                                if entry.get().version < function.version {
                                    entry.insert(function);
                                }
                                map
                            }
                            Entry::Vacant(entry) => {
                                entry.insert(function);
                                map
                            }
                        },
                    )
                    .into_values(),
            )
        } else {
            Either::Right(filtered_functions)
        })
        .filter(|func| {
            required_metadata.as_ref().map_or(true, |filters| {
                filters.iter().all(|filter| {
                    func.metadata
                        .iter()
                        .any(|(k, v)| filter.0 == k && (filter.1.is_empty() || filter.1 == v))
                })
            }) && func.publisher.email.contains(&filters.publisher_email)
        })
        .collect::<Vec<&Function>>();

        if OrderingKey::from_i32(order.key).is_none() {
            warn!(
                self.logger,
                "Ordering key was out of range ({}). Out of date protobuf definitions?", order.key
            );
        }

        filtered_functions.sort_unstable_by(|a, b| match OrderingKey::from_i32(order.key) {
            Some(OrderingKey::NameVersion) | None => match a.name.cmp(&b.name) {
                std::cmp::Ordering::Equal => b.version.cmp(&a.version),
                o => o,
            },
        });

        Ok(Functions {
            functions: if order.reverse {
                filtered_functions
                    .iter()
                    .rev()
                    .skip(offset)
                    .take(limit)
                    .filter_map(|f| self.get_function(*f).ok())
                    .collect::<Vec<_>>()
            } else {
                filtered_functions
                    .iter()
                    .skip(offset)
                    .take(limit)
                    .filter_map(|f| self.get_function(*f).ok())
                    .collect::<Vec<_>>()
            },
        })
    }
}

#[tonic::async_trait]
impl Registry for RegistryService {
    async fn list(
        &self,
        list_request: tonic::Request<Filters>,
    ) -> Result<tonic::Response<Functions>, tonic::Status> {
        RegistryService::list(self, list_request.into_inner(), true).map(tonic::Response::new)
    }

    async fn list_versions(
        &self,
        list_request: tonic::Request<firm_types::functions::Filters>,
    ) -> Result<tonic::Response<firm_types::functions::Functions>, tonic::Status> {
        RegistryService::list(self, list_request.into_inner(), false).map(tonic::Response::new)
    }

    async fn get(
        &self,
        function_id_request: tonic::Request<FunctionId>,
    ) -> Result<tonic::Response<ProtoFunction>, tonic::Status> {
        let fn_id = function_id_request.into_inner();

        self.functions
            .read()
            .map_err(|e| {
                tonic::Status::new(
                    tonic::Code::Internal,
                    format!("Failed to get read lock for functions: {}", e),
                )
            })
            .and_then(|reader| {
                reader
                    .iter()
                    .rev()
                    .find(|f| f.name == fn_id.name && f.version.to_string() == fn_id.version)
                    .ok_or_else(|| {
                        tonic::Status::new(
                            tonic::Code::NotFound,
                            format!(
                                "failed to find function with id (name \"{}\" and version \"{}\")",
                                fn_id.name, fn_id.version
                            ),
                        )
                    })
                    .and_then(|f| self.get_function(f))
                    .map(tonic::Response::new)
            })
    }

    async fn register(
        &self,
        register_request: tonic::Request<FunctionData>,
    ) -> Result<tonic::Response<ProtoFunction>, tonic::Status> {
        let mut payload = register_request.into_inner();

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

        // this is the local case, always add dev to any function version
        if !self.config.version_suffix.is_empty() {
            version.pre.push(semver::Identifier::AlphaNumeric(
                self.config.version_suffix.clone(),
            ));
        }

        let mut functions = self.functions.write().map_err(|e| {
            tonic::Status::new(
                tonic::Code::Internal,
                format!("Failed to get write lock for functions: {}", e),
            )
        })?;

        // remove function if name and version matches (after the suffix has been appended)
        // TODO: Remove corresponding attachments
        functions.retain(|v| v.name != payload.name || v.version != version);

        let runtime = payload.runtime.ok_or_else(|| {
            tonic::Status::new(
                tonic::Code::InvalidArgument,
                String::from("Runtime is required when registering function"),
            )
        })?;

        // validate attachments
        let combined_checksum = payload
            .attachment_ids
            .iter()
            .chain(payload.code_attachment_id.iter())
            .fold(Ok(Sha256::new()), |r, id| {
                match (r, self.get_attachment(id)) {
                    (Ok(mut cs), Ok(a)) => {
                        cs.update(a.checksums.unwrap_or_default().sha256);
                        Ok(cs)
                    }
                    (Ok(_), Err(e)) => Err(format!("{} ({})", id.uuid, e.message())),
                    (Err(e), Ok(_)) => Err(e),
                    (Err(e1), Err(e2)) => Err(format!("{}, {} ({})", e1, id.uuid, e2.message())),
                }
            })
            .map_err(|msg| {
                tonic::Status::new(
                    tonic::Code::InvalidArgument,
                    format!("Failed to get attachment for ids: [{}]", msg),
                )
            })?;

        payload.metadata.insert(
            String::from("_dev-checksum"),
            format!("{:x}", combined_checksum.finalize()),
        );

        let publisher = payload
            .publisher
            .ok_or_else(|| {
                tonic::Status::invalid_argument("Publisher is required when registering function")
            })
            .and_then(|publisher| match publisher {
                ProtoPublisher { ref name, .. } if name.is_empty() => {
                    Err(tonic::Status::invalid_argument(
                        "Publisher name is required when registering a function",
                    ))
                }
                ProtoPublisher { ref email, .. } if email.is_empty() => {
                    Err(tonic::Status::invalid_argument(
                        "Publisher email is required when registering a function",
                    ))
                }
                p => Ok(Publisher {
                    name: p.name,
                    email: p.email,
                }),
            })?;

        let function = Function {
            name: payload.name,
            version,
            runtime,
            metadata: payload.metadata,
            required_inputs: payload.required_inputs,
            optional_inputs: payload.optional_inputs,
            outputs: payload.outputs,
            code: payload.code_attachment_id,
            attachments: payload.attachment_ids,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or_default(),
            publisher,
            signature: payload.signature.map(|sig| sig.signature),
        };

        functions.push(function.clone());

        Ok(tonic::Response::new(self.get_function(&function)?))
    }

    async fn register_attachment(
        &self,
        register_attachment_request: tonic::Request<AttachmentData>,
    ) -> Result<tonic::Response<AttachmentHandle>, tonic::Status> {
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

        let id = Uuid::new_v4();
        let path = self.attachment_directory.path().join(id.to_string());
        let upload_url = Some(AttachmentUrl {
            // TODO: This should be the local socket path.
            // Currently bendini assumes that if the transport is grpc
            // the upload host is the same as the registry host which is
            // obviously not correct but works in this case.
            url: String::from("grpc://"),
            auth_method: AuthMethod::None as i32,
        });
        let publisher = payload
            .publisher
            .ok_or_else(|| {
                tonic::Status::invalid_argument("Publisher is required when registering attachment")
            })
            .and_then(|publisher| match publisher {
                ProtoPublisher { ref name, .. } if name.is_empty() => {
                    Err(tonic::Status::invalid_argument(
                        "Publisher name is required when registering an attachment",
                    ))
                }
                ProtoPublisher { ref email, .. } if email.is_empty() => {
                    Err(tonic::Status::invalid_argument(
                        "Publisher email is required when registering an attachment",
                    ))
                }
                p => Ok(Some(p)),
            })?;

        fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|e| {
                tonic::Status::internal(format!(
                    "Failed to create attachment file {}: {}",
                    path.display(),
                    e
                ))
            })?;

        function_attachments.insert(
            id,
            Attachment {
                name: payload.name,
                url: Some(AttachmentUrl {
                    url: path
                        .canonicalize()
                        .map_err(|e| {
                            tonic::Status::internal(format!(
                                r#"Failed to canonicalize attachment path "{}": {}"#,
                                path.display(),
                                e
                            ))
                        })
                        .and_then(|p| {
                            url::Url::from_file_path(&p).map_err(|_| {
                                tonic::Status::internal(format!(
                                    r#"Attachment path "{}" could not be made into a valid url"#,
                                    p.display()
                                ))
                            })
                        })?
                        .to_string(),
                    auth_method: AuthMethod::None as i32,
                }),
                metadata: payload.metadata,
                checksums: Some(checksum),
                created_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or_default(),
                publisher,
                signature: payload.signature.map(|sig| Signature {
                    signature: sig.signature,
                }),
            },
        );

        Ok(tonic::Response::new(AttachmentHandle {
            id: Some(AttachmentId {
                uuid: id.to_string(),
            }),
            upload_url,
        }))
    }

    async fn upload_streamed_attachment(
        &self,
        attachment_stream_upload_request: tonic::Request<tonic::Streaming<AttachmentStreamUpload>>,
    ) -> Result<tonic::Response<firm_types::functions::Nothing>, tonic::Status> {
        // TODO: use metadata for "global" upload data such as AttachmentId
        self.upload_stream_attachment(attachment_stream_upload_request)
            .await
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
