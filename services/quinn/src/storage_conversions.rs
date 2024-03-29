use std::{
    collections::HashMap,
    convert::{TryFrom, TryInto},
};

use firm_types::{
    functions::{Filters, Function as ProtoFunction, FunctionId, OrderingKey},
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
            name: req.name,
            metadata: req
                .metadata
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
            publisher_email: req.publisher_email,
        })
    }
}

impl TryFrom<firm_types::functions::RuntimeSpec> for storage::Runtime {
    type Error = tonic::Status;

    fn try_from(value: firm_types::functions::RuntimeSpec) -> Result<Self, Self::Error> {
        Ok(storage::Runtime {
            name: value.name.check_empty("runtime.name")?,
            entrypoint: value.entrypoint,
            arguments: value.arguments,
        })
    }
}

impl TryFrom<firm_types::functions::ChannelSpec> for storage::ChannelSpec {
    type Error = tonic::Status;

    fn try_from(value: firm_types::functions::ChannelSpec) -> Result<Self, Self::Error> {
        let tp = value.r#type;
        Ok(storage::ChannelSpec {
            description: value.description,
            argument_type: firm_types::functions::ChannelType::from_i32(tp).ok_or_else(|| {
                tonic::Status::new(
                    tonic::Code::InvalidArgument,
                    format!("Input type {} is out of range for enum", tp),
                )
            })?,
        })
    }
}

impl From<storage::ChannelSpec> for firm_types::functions::ChannelSpec {
    fn from(value: storage::ChannelSpec) -> Self {
        firm_types::functions::ChannelSpec {
            description: value.description,
            r#type: value.argument_type as i32,
        }
    }
}

trait ToUuid {
    type Error;
    fn to_uuid(&self) -> Result<uuid::Uuid, Self::Error>;
}

impl ToUuid for firm_types::functions::AttachmentId {
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

impl TryFrom<firm_types::functions::FunctionData> for storage::Function {
    type Error = tonic::Status;

    fn try_from(value: firm_types::functions::FunctionData) -> Result<Self, Self::Error> {
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
                        "Registering a function requires a runtime",
                    )
                })
                .and_then(|ee| ee.try_into())?,
            required_inputs: value
                .required_inputs
                .into_iter()
                .map(|(k, cs)| cs.try_into().map(|c| (k, c)))
                .collect::<Result<HashMap<_, _>, _>>()?,
            optional_inputs: value
                .optional_inputs
                .into_iter()
                .map(|(k, cs)| cs.try_into().map(|c| (k, c)))
                .collect::<Result<HashMap<_, _>, _>>()?,
            outputs: value
                .outputs
                .into_iter()
                .map(|(k, cs)| cs.try_into().map(|c| (k, c)))
                .collect::<Result<HashMap<_, _>, _>>()?,
            metadata: value.metadata,
            code: value.code_attachment_id.map(|a| a.to_uuid()).transpose()?,
            attachments: value
                .attachment_ids
                .iter()
                .map(|a| a.to_uuid())
                .collect::<Result<Vec<_>, _>>()?,
            created_at: 0, // set on the way out, not in,
            publisher: value
                .publisher
                .map(|p| storage::Publisher {
                    name: p.name,
                    email: p.email,
                })
                .ok_or_else(|| {
                    tonic::Status::new(
                        tonic::Code::InvalidArgument,
                        "Registering a function requires a publisher",
                    )
                })?,
            signature: value.signature.map(|sig| sig.signature),
        })
    }
}

impl TryFrom<firm_types::functions::Checksums> for storage::Checksums {
    type Error = tonic::Status;

    fn try_from(value: firm_types::functions::Checksums) -> Result<Self, Self::Error> {
        Ok(storage::Checksums {
            sha256: value.sha256.check_empty("sha256")?,
        })
    }
}

impl TryFrom<firm_types::functions::AttachmentData> for storage::FunctionAttachmentData {
    type Error = tonic::Status;

    fn try_from(value: firm_types::functions::AttachmentData) -> Result<Self, Self::Error> {
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
            publisher: value
                .publisher
                .ok_or_else(|| tonic::Status::invalid_argument("Attachment requires publisher"))
                .map(|publisher| storage::Publisher {
                    name: publisher.name,
                    email: publisher.email,
                })?,
            signature: value.signature.map(|sig| sig.signature),
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

impl<'a> From<AttachmentResolver<'a>> for firm_types::functions::Attachment {
    fn from(attachment_resolver: AttachmentResolver) -> Self {
        let (attachment_storage, att) = (attachment_resolver.0, attachment_resolver.1);
        Self {
            name: att.data.name.clone(),
            url: attachment_storage.get_download_url(&att).ok(), // TODO: no good, error here
            metadata: att.data.metadata,
            checksums: Some(firm_types::functions::Checksums {
                sha256: att.data.checksums.sha256.to_string(),
            }),
            created_at: att.created_at,
            publisher: Some(firm_types::functions::Publisher {
                name: att.data.publisher.name,
                email: att.data.publisher.email,
            }),
            signature: att
                .data
                .signature
                .map(|s| firm_types::functions::Signature { signature: s }),
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
            runtime: Some(firm_types::functions::RuntimeSpec {
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
            required_inputs: self
                .required_inputs
                .iter()
                .map(|(k, cs)| (k.to_owned(), cs.clone().into()))
                .collect(),
            optional_inputs: self
                .optional_inputs
                .iter()
                .map(|(k, cs)| (k.to_owned(), cs.clone().into()))
                .collect(),
            outputs: self
                .outputs
                .iter()
                .map(|(k, cs)| (k.to_owned(), cs.clone().into()))
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
            publisher: Some(firm_types::functions::Publisher {
                name: self.publisher.name.clone(),
                email: self.publisher.email.clone(),
            }),
            signature: self
                .signature
                .as_ref()
                .map(|sig| firm_types::functions::Signature {
                    signature: sig.clone(),
                }),
        })
    }
}
