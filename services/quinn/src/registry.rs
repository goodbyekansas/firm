use slog::{o, Logger};
use tonic::Status;

use gbk_protocols::{functions::functions_registry_server::FunctionsRegistry, tonic};

use std::{
    collections::HashMap,
    convert::{TryFrom, TryInto},
    sync::{Arc, RwLock},
};

use crate::{
    config,
    storage::{self, StorageError},
    validation,
};

pub struct FunctionRegistryService {
    function_storage: Arc<RwLock<Box<dyn storage::FunctionStorage>>>,
    function_attachment_storage: Arc<RwLock<Box<dyn storage::FunctionAttachmentStorage>>>,
}

impl FunctionRegistryService {
    pub fn new(config: config::Configuration, log: Logger) -> Result<Self, String> {
        Ok(Self {
            function_storage: Arc::new(RwLock::new(
                storage::create_storage(
                    &config.functions_storage_uri,
                    log.new(o!("storage" => "functions")),
                )
                .map_err(|e| format!("Failed to create storage backend: {}", e))?,
            )),
            function_attachment_storage: Arc::new(RwLock::new(
                storage::create_attachment_storage(
                    &config.attachment_storage_uri,
                    log.new(o!("storage" => "attachments")),
                )
                .map_err(|e| format!("Failed to create attachment storage backed! {}", e))?,
            )),
        })
    }
}

trait CheckEmptyString {
    fn check_empty(self, field_name: &str) -> Result<String, tonic::Status>;
}

impl CheckEmptyString for String {
    fn check_empty(self, field_name: &str) -> Result<String, tonic::Status> {
        if self.is_empty() {
            Err(tonic::Status::new(
                tonic::Code::InvalidArgument,
                format!("Field \"{}\" is required but was empty", field_name),
            ))
        } else {
            Ok(self)
        }
    }
}

impl TryFrom<gbk_protocols::functions::ExecutionEnvironment> for storage::ExecutionEnvironment {
    type Error = tonic::Status;

    fn try_from(
        value: gbk_protocols::functions::ExecutionEnvironment,
    ) -> Result<Self, Self::Error> {
        Ok(storage::ExecutionEnvironment {
            name: value.name.check_empty("execution_environment.name")?,
            entrypoint: value.entrypoint, // TODO investigate if it's valid that this is empty
            function_arguments: value
                .args
                .into_iter()
                .map(|a| {
                    let n = a.name;
                    String::from_utf8(a.value).map(|v| (n, v))
                })
                .collect::<Result<HashMap<String, String>, _>>()
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::InvalidArgument,
                        format!(
                            "Found invalid string in execution environment argument: {}",
                            e
                        ),
                    )
                })?,
        })
    }
}

impl TryFrom<gbk_protocols::functions::FunctionInput> for storage::FunctionInput {
    type Error = tonic::Status;

    fn try_from(value: gbk_protocols::functions::FunctionInput) -> Result<Self, Self::Error> {
        let tp = value.r#type;
        Ok(storage::FunctionInput {
            name: value.name.check_empty("Function Input Name")?,
            required: value.required,
            argument_type: gbk_protocols::functions::ArgumentType::from_i32(tp).ok_or_else(
                || {
                    tonic::Status::new(
                        tonic::Code::InvalidArgument,
                        format!("Argument type {} is out of range for enum", tp),
                    )
                },
            )?,
            default_value: value.default_value,
            from_execution_environment: value.from_execution_environment, // TODO remove this, it belongs to out data
        })
    }
}

impl TryFrom<gbk_protocols::functions::FunctionOutput> for storage::FunctionOutput {
    type Error = tonic::Status;

    fn try_from(value: gbk_protocols::functions::FunctionOutput) -> Result<Self, Self::Error> {
        let tp = value.r#type;
        Ok(storage::FunctionOutput {
            name: value.name.check_empty("Function Output Name")?,
            argument_type: gbk_protocols::functions::ArgumentType::from_i32(tp).ok_or_else(
                || {
                    tonic::Status::new(
                        tonic::Code::InvalidArgument,
                        format!("Argument type {} is out of range for enum", tp),
                    )
                },
            )?,
        })
    }
}
trait ToUuid {
    type Error;
    fn to_uuid(&self) -> Result<uuid::Uuid, Self::Error>;
}
impl ToUuid for gbk_protocols::functions::FunctionAttachmentId {
    type Error = tonic::Status;

    fn to_uuid(&self) -> Result<uuid::Uuid, Self::Error> {
        uuid::Uuid::parse_str(&self.id).map_err(|e| {
            tonic::Status::new(
                tonic::Code::InvalidArgument,
                format!("Invalid function attachment id \"{}\": {}", self.id, e),
            )
        })
    }
}

impl TryFrom<gbk_protocols::functions::RegisterRequest> for storage::FunctionData {
    type Error = tonic::Status;

    fn try_from(value: gbk_protocols::functions::RegisterRequest) -> Result<Self, Self::Error> {
        Ok(storage::FunctionData {
            name: validation::validate_name(&value.name)
                .map_err(|e| tonic::Status::new(tonic::Code::InvalidArgument, e.to_string()))?,
            version: validation::validate_version(&value.version)
                .map_err(|e| tonic::Status::new(tonic::Code::InvalidArgument, e.to_string()))?,
            execution_environment: value
                .execution_environment
                .ok_or_else(|| {
                    tonic::Status::new(
                        tonic::Code::InvalidArgument,
                        "Registering a function requires an execution environment",
                    )
                })
                .and_then(|ee| ee.try_into())?,
            inputs: value
                .inputs
                .into_iter()
                .map(|a| a.try_into())
                .collect::<Result<Vec<_>, _>>()?,
            outputs: value
                .outputs
                .into_iter()
                .map(|a| a.try_into())
                .collect::<Result<Vec<_>, _>>()?,
            metadata: value.metadata,
            code: value.code.map(|a| a.to_uuid()).transpose()?,
            attachments: value
                .attachment_ids
                .iter()
                .map(|a| a.to_uuid())
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

impl TryFrom<gbk_protocols::functions::Checksums> for storage::Checksums {
    type Error = tonic::Status;

    fn try_from(value: gbk_protocols::functions::Checksums) -> Result<Self, Self::Error> {
        Ok(storage::Checksums {
            sha256: value.sha256.check_empty("sha256")?,
        })
    }
}

impl TryFrom<gbk_protocols::functions::RegisterAttachmentRequest>
    for storage::FunctionAttachmentData
{
    type Error = tonic::Status;

    fn try_from(
        value: gbk_protocols::functions::RegisterAttachmentRequest,
    ) -> Result<Self, Self::Error> {
        Ok(storage::FunctionAttachmentData {
            name: value.name.check_empty("name")?,
            metadata: value.metadata,
            checksums: value
                .checksums
                .ok_or_else(|| {
                    tonic::Status::new(
                        tonic::Code::InvalidArgument,
                        "Attachment requires checksums",
                    )
                })
                .and_then(|c| c.try_into())?,
        })
    }
}

impl From<StorageError> for tonic::Status {
    fn from(se: StorageError) -> Self {
        match se {
            StorageError::VersionExists { .. } => {
                tonic::Status::new(tonic::Code::InvalidArgument, se.to_string())
            }
            _ => tonic::Status::new(tonic::Code::Unknown, format!("Storage error: {}", se)),
        }
    }
}

#[tonic::async_trait]
impl FunctionsRegistry for FunctionRegistryService {
    async fn list(
        &self,
        _request: tonic::Request<gbk_protocols::functions::ListRequest>,
    ) -> Result<tonic::Response<gbk_protocols::functions::RegistryListResponse>, tonic::Status>
    {
        Err(Status::new(tonic::Code::Unimplemented, "TODO"))
    }
    async fn get(
        &self,
        _request: tonic::Request<gbk_protocols::functions::FunctionId>,
    ) -> Result<tonic::Response<gbk_protocols::functions::FunctionDescriptor>, tonic::Status> {
        Err(Status::new(tonic::Code::Unimplemented, "TODO"))
    }
    async fn register(
        &self,
        request: tonic::Request<gbk_protocols::functions::RegisterRequest>,
    ) -> Result<tonic::Response<gbk_protocols::functions::FunctionId>, tonic::Status> {
        self.function_storage
            .write()
            .map_err(|e| {
                tonic::Status::new(
                    tonic::Code::Internal,
                    format!(
                        "Failed to acquire write lock for registering function: {}",
                        e
                    ),
                )
            })?
            .insert(storage::FunctionData::try_from(request.into_inner())?)
            .map_err(|se| se.into())
            .map(|uuid| {
                tonic::Response::new(gbk_protocols::functions::FunctionId {
                    value: uuid.to_string(),
                })
            })
    }
    async fn register_attachment(
        &self,
        request: tonic::Request<gbk_protocols::functions::RegisterAttachmentRequest>,
    ) -> Result<tonic::Response<gbk_protocols::functions::FunctionAttachmentId>, tonic::Status>
    {
        self.function_storage
            .write()
            .map_err(|e| {
                tonic::Status::new(
                    tonic::Code::Internal,
                    format!(
                        "Failed to acquire write lock for registering function attachment: {}",
                        e
                    ),
                )
            })?
            .insert_attachment(storage::FunctionAttachmentData::try_from(
                request.into_inner(),
            )?)
            .map_err(|se| se.into())
            .map(|uuid| {
                tonic::Response::new(gbk_protocols::functions::FunctionAttachmentId {
                    id: uuid.to_string(),
                })
            })
    }
    async fn upload_streamed_attachment(
        &self,
        _request: tonic::Request<
            tonic::Streaming<gbk_protocols::functions::AttachmentStreamUpload>,
        >,
    ) -> Result<tonic::Response<gbk_protocols::functions::AttachmentUploadResponse>, tonic::Status>
    {
        Err(tonic::Status::new(
            tonic::Code::Unimplemented,
            "The Quinn registry does not support uploading via streaming upload, use URL instead."
                .to_owned(),
        ))
    }
    async fn upload_attachment_url(
        &self,
        request: tonic::Request<gbk_protocols::functions::AttachmentUpload>,
    ) -> Result<tonic::Response<gbk_protocols::functions::AttachmentUploadResponse>, tonic::Status>
    {
        request
        .into_inner()
        .id
        .ok_or_else(|| tonic::Status::new(tonic::Code::InvalidArgument, "attachment id is required for obtaining upload url".to_owned()))
        .and_then(|id| uuid::Uuid::parse_str(&id.id).map_err(|e| tonic::Status::new(tonic::Code::InvalidArgument, format!("Invalid uuid: {}: {}", &id.id, e))))
        .and_then(|id| {
            let function_storage = self.function_storage
            .write()
            .map_err(|e| {
                tonic::Status::new(
                    tonic::Code::Internal,
                    format!(
                        "Failed to acquire write lock on function storage for generating upload attachment url: {}",
                        e
                    ),
                )
            })?;
            self.function_attachment_storage.read().map_err(|e|tonic::Status::new(
                tonic::Code::Internal,
                format!(
                    "Failed to acquire read lock on function attachment storage for generating upload attachment url: {}",
                    e
                ),
            ))?.get_upload_url(id, function_storage.as_ref()).map_err(|e| e.into()).map(tonic::Response::new)
        })
    }
}
