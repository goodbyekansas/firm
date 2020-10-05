use std::{
    collections::HashMap,
    convert::{TryFrom, TryInto},
};

use gbk_protocols::{functions::FunctionDescriptor, tonic};

use crate::{
    storage::{self, AttachmentStorage, FunctionStorage, StorageError},
    validation,
};
use futures::FutureExt;
use storage::{Function, FunctionAttachment};

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

impl TryFrom<gbk_protocols::functions::ListRequest> for storage::Filters {
    type Error = tonic::Status;

    fn try_from(req: gbk_protocols::functions::ListRequest) -> Result<Self, Self::Error> {
        Ok(Self {
            name: req.name_filter,
            metadata: req
                .metadata_key_filter
                .into_iter()
                .map(|n| (n, None))
                .chain(req.metadata_filter.into_iter().map(|(k, v)| (k, Some(v))))
                .collect(),
            offset: req.offset as usize,
            limit: if req.limit == 0 {
                100
            } else {
                req.limit as usize
            },
            exact_name_match: req.exact_name_match,
            version_requirement: req
                .version_requirement
                .map(|req| {
                    semver::VersionReq::parse(&req.expression).map_err(|e| {
                        tonic::Status::new(
                            tonic::Code::InvalidArgument,
                            format!(
                                "Invalid semantic version requirement \"{}\": {}",
                                &req.expression, e
                            ),
                        )
                    })
                })
                .transpose()?,
            order_descending: match gbk_protocols::functions::OrderingDirection::from_i32(
                req.order_direction,
            ) {
                Some(gbk_protocols::functions::OrderingDirection::Descending) => true,
                Some(gbk_protocols::functions::OrderingDirection::Ascending) => false,
                None => true,
            },
            order_by: gbk_protocols::functions::OrderingKey::from_i32(req.order_by)
                .unwrap_or(gbk_protocols::functions::OrderingKey::Name),
        })
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
            StorageError::FunctionNotFound { .. } | StorageError::AttachmentNotFound { .. } => {
                tonic::Status::new(tonic::Code::NotFound, se.to_string())
            }
            _ => tonic::Status::new(tonic::Code::Unknown, format!("Storage error: {}", se)),
        }
    }
}

#[async_trait::async_trait]
pub trait ToFunctionDescriptor {
    async fn to_function_descriptor(
        self,
        function_store: &dyn FunctionStorage,
        attachment_store: &dyn AttachmentStorage,
    ) -> Result<FunctionDescriptor, StorageError>;
}

struct AttachmentResolver<'a>(&'a dyn AttachmentStorage, FunctionAttachment);

impl<'a> From<AttachmentResolver<'a>> for gbk_protocols::functions::FunctionAttachment {
    fn from(attachment_resolver: AttachmentResolver) -> Self {
        let (attachment_storage, att) = (attachment_resolver.0, attachment_resolver.1);
        Self {
            id: Some(gbk_protocols::functions::FunctionAttachmentId {
                id: att.id.to_string(),
            }),
            name: att.data.name.clone(),
            url: attachment_storage
                .get_download_url(&att)
                .unwrap()
                .to_string(),
            metadata: att.data.metadata,
            checksums: Some(gbk_protocols::functions::Checksums {
                sha256: att.data.checksums.sha256.to_string(),
            }),
        }
    }
}

#[async_trait::async_trait]
impl ToFunctionDescriptor for &Function {
    #[allow(clippy::eval_order_dependence)] // clippy firing on things it shouldn't (https://github.com/rust-lang/rust-clippy/issues/4637)
    async fn to_function_descriptor(
        self,
        function_store: &dyn FunctionStorage,
        attachment_store: &dyn AttachmentStorage,
    ) -> Result<FunctionDescriptor, StorageError> {
        Ok(FunctionDescriptor {
            execution_environment: Some(gbk_protocols::functions::ExecutionEnvironment {
                name: self.function_data.execution_environment.name.clone(),
                entrypoint: self.function_data.execution_environment.entrypoint.clone(),
                args: self
                    .function_data
                    .execution_environment
                    .function_arguments
                    .iter()
                    .map(|(k, v)| gbk_protocols::functions::FunctionArgument {
                        name: k.clone(),
                        value: v.as_bytes().to_vec(),
                        r#type: gbk_protocols::functions::ArgumentType::String as i32,
                    })
                    .collect(),
            }),
            code: futures::future::OptionFuture::from(
                self.function_data
                    .code
                    .map(|id| async move { function_store.get_attachment(&id).await }),
            )
            .await
            .transpose()?
            .map(|attachment_data| AttachmentResolver(attachment_store, attachment_data).into()),
            function: Some(gbk_protocols::functions::Function {
                id: Some(gbk_protocols::functions::FunctionId {
                    value: self.id.to_string(),
                }),
                name: self.function_data.name.clone(),
                version: self.function_data.version.to_string(),
                metadata: self.function_data.metadata.clone(),
                inputs: self
                    .function_data
                    .inputs
                    .iter()
                    .map(|i| gbk_protocols::functions::FunctionInput {
                        name: i.name.clone(),
                        default_value: i.default_value.clone(),
                        from_execution_environment: i.from_execution_environment,
                        required: i.required,
                        r#type: i.argument_type as i32,
                    })
                    .collect(),
                outputs: self
                    .function_data
                    .outputs
                    .iter()
                    .map(|o| gbk_protocols::functions::FunctionOutput {
                        name: o.name.clone(),
                        r#type: o.argument_type as i32,
                    })
                    .collect(),
            }),
            attachments: futures::future::try_join_all(self.function_data.attachments.iter().map(
                |attachment_id| async move {
                    function_store
                        .get_attachment(attachment_id)
                        .map(|at_res| {
                            at_res.map(|at| AttachmentResolver(attachment_store, at).into())
                        })
                        .await
                },
            ))
            .await?,
        })
    }
}
