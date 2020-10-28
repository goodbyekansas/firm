use std::convert::{TryFrom, TryInto};

use firm_protocols::{
    functions::Function as ProtoFunction,
    registry::{Filters, FunctionId, OrderingKey},
    tonic,
};

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

impl TryFrom<FunctionId> for storage::FunctionId {
    type Error = tonic::Status;

    fn try_from(value: FunctionId) -> Result<Self, Self::Error> {
        Ok(Self {
            name: value.name.clone(),
            version: semver::Version::parse(&value.version).map_err(|e| {
                tonic::Status::invalid_argument(format!(
                    "Invalid semantic version {}: {}",
                    value.version, e
                ))
            })?,
        })
    }
}

impl TryFrom<Filters> for storage::Filters {
    type Error = tonic::Status;

    fn try_from(req: Filters) -> Result<Self, Self::Error> {
        Ok(Self {
            name: req.name_filter.map(|nf| storage::NameFilter {
                pattern: nf.pattern,
                exact_match: nf.exact_match,
            }),

            metadata: req
                .metadata_filter
                .into_iter()
                .map(|(k, v)| (k, if v.is_empty() { None } else { Some(v) }))
                .collect(),
            order: req.order.map(|o| storage::Ordering {
                key: OrderingKey::from_i32(o.key).unwrap_or(OrderingKey::NameVersion),
                reverse: o.reverse,
                offset: o.offset as usize,
                limit: std::cmp::min(if o.limit == 0 { 100 } else { o.limit }, 1000) as usize,
            }),
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
        })
    }
}

impl TryFrom<firm_protocols::functions::Runtime> for storage::Runtime {
    type Error = tonic::Status;

    fn try_from(value: firm_protocols::functions::Runtime) -> Result<Self, Self::Error> {
        Ok(storage::Runtime {
            name: value.name.check_empty("execution_environment.name")?,
            entrypoint: value.entrypoint, // TODO investigate if it's valid that this is empty
            arguments: value.arguments,
        })
    }
}

impl TryFrom<firm_protocols::functions::Input> for storage::FunctionInput {
    type Error = tonic::Status;

    fn try_from(value: firm_protocols::functions::Input) -> Result<Self, Self::Error> {
        let tp = value.r#type;
        Ok(storage::FunctionInput {
            name: value.name.check_empty("Function Input Name")?,
            description: value.description,
            required: value.required,
            argument_type: firm_protocols::functions::Type::from_i32(tp).ok_or_else(|| {
                tonic::Status::new(
                    tonic::Code::InvalidArgument,
                    format!("Input type {} is out of range for enum", tp),
                )
            })?,
        })
    }
}

impl TryFrom<firm_protocols::functions::Output> for storage::FunctionOutput {
    type Error = tonic::Status;

    fn try_from(value: firm_protocols::functions::Output) -> Result<Self, Self::Error> {
        let tp = value.r#type;
        Ok(storage::FunctionOutput {
            name: value.name.check_empty("Function Output Name")?,
            description: value.description,
            argument_type: firm_protocols::functions::Type::from_i32(tp).ok_or_else(|| {
                tonic::Status::new(
                    tonic::Code::InvalidArgument,
                    format!("Argument type {} is out of range for enum", tp),
                )
            })?,
        })
    }
}

trait ToUuid {
    type Error;
    fn to_uuid(&self) -> Result<uuid::Uuid, Self::Error>;
}

impl ToUuid for firm_protocols::registry::AttachmentId {
    type Error = tonic::Status;

    fn to_uuid(&self) -> Result<uuid::Uuid, Self::Error> {
        uuid::Uuid::parse_str(&self.uuid).map_err(|e| {
            tonic::Status::new(
                tonic::Code::InvalidArgument,
                format!("Invalid function attachment id \"{}\": {}", self.uuid, e),
            )
        })
    }
}

impl TryFrom<firm_protocols::registry::FunctionData> for storage::Function {
    type Error = tonic::Status;

    fn try_from(value: firm_protocols::registry::FunctionData) -> Result<Self, Self::Error> {
        Ok(storage::Function {
            name: validation::validate_name(&value.name)
                .map_err(|e| tonic::Status::new(tonic::Code::InvalidArgument, e.to_string()))?,
            version: validation::validate_version(&value.version)
                .map_err(|e| tonic::Status::new(tonic::Code::InvalidArgument, e.to_string()))?,
            runtime: value
                .runtime
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
            code: value.code_attachment_id.map(|a| a.to_uuid()).transpose()?,
            attachments: value
                .attachment_ids
                .iter()
                .map(|a| a.to_uuid())
                .collect::<Result<Vec<_>, _>>()?,
            created_at: 0, // set on the way out, not in
        })
    }
}

impl TryFrom<firm_protocols::functions::Checksums> for storage::Checksums {
    type Error = tonic::Status;

    fn try_from(value: firm_protocols::functions::Checksums) -> Result<Self, Self::Error> {
        Ok(storage::Checksums {
            sha256: value.sha256.check_empty("sha256")?,
        })
    }
}

impl TryFrom<firm_protocols::registry::AttachmentData> for storage::FunctionAttachmentData {
    type Error = tonic::Status;

    fn try_from(value: firm_protocols::registry::AttachmentData) -> Result<Self, Self::Error> {
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
pub trait FunctionResolver {
    async fn resolve_function(
        self,
        function_store: &dyn FunctionStorage,
        attachment_store: &dyn AttachmentStorage,
    ) -> Result<ProtoFunction, StorageError>;
}

struct AttachmentResolver<'a>(&'a dyn AttachmentStorage, FunctionAttachment);

impl<'a> From<AttachmentResolver<'a>> for firm_protocols::functions::Attachment {
    fn from(attachment_resolver: AttachmentResolver) -> Self {
        let (attachment_storage, att) = (attachment_resolver.0, attachment_resolver.1);
        Self {
            name: att.data.name.clone(),
            url: attachment_storage.get_download_url(&att).ok(), // TODO: no good, error here
            metadata: att.data.metadata,
            checksums: Some(firm_protocols::functions::Checksums {
                sha256: att.data.checksums.sha256.to_string(),
            }),
            created_at: att.created_at,
        }
    }
}

#[async_trait::async_trait]
impl FunctionResolver for &Function {
    #[allow(clippy::eval_order_dependence)] // clippy firing on things it shouldn't (https://github.com/rust-lang/rust-clippy/issues/4637)
    async fn resolve_function(
        self,
        function_store: &dyn FunctionStorage,
        attachment_store: &dyn AttachmentStorage,
    ) -> Result<ProtoFunction, StorageError> {
        Ok(ProtoFunction {
            runtime: Some(firm_protocols::functions::Runtime {
                name: self.runtime.name.clone(),
                entrypoint: self.runtime.entrypoint.clone(),
                arguments: self.runtime.arguments.clone(),
            }),
            code: futures::future::OptionFuture::from(
                self.code
                    .map(|id| async move { function_store.get_attachment(&id).await }),
            )
            .await
            .transpose()?
            .map(|attachment_data| AttachmentResolver(attachment_store, attachment_data).into()),
            name: self.name.clone(),
            version: self.version.to_string(),
            metadata: self.metadata.clone(),
            inputs: self
                .inputs
                .iter()
                .map(|i| firm_protocols::functions::Input {
                    name: i.name.clone(),
                    description: i.description.clone(),
                    required: i.required,
                    r#type: i.argument_type as i32,
                })
                .collect(),
            outputs: self
                .outputs
                .iter()
                .map(|o| firm_protocols::functions::Output {
                    name: o.name.clone(),
                    description: o.description.clone(),
                    r#type: o.argument_type as i32,
                })
                .collect(),
            attachments: futures::future::try_join_all(self.attachments.iter().map(
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
            created_at: self.created_at,
        })
    }
}
