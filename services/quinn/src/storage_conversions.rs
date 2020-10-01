use std::{
    collections::HashMap,
    convert::{TryFrom, TryInto},
};

use crate::{
    storage::{self, StorageError},
    validation,
};
use gbk_protocols::tonic;

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
