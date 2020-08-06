mod wasi;

use std::{
    collections::{HashMap, HashSet},
    fmt::{self, Debug, Display},
    fs, str,
};

use prost::Message;
use semver::VersionReq;
use sha2::{Digest, Sha256};
use slog::{o, Logger};
use thiserror::Error;
use url::Url;

use crate::executor::wasi::WasiExecutor;
use gbk_protocols::{
    functions::{
        execute_response::Result as ProtoResult, functions_registry_server::FunctionsRegistry,
        ArgumentType, FunctionArgument, FunctionAttachment, FunctionContext, FunctionDescriptor,
        FunctionInput, FunctionOutput, FunctionResult, ListRequest, OrderingDirection, OrderingKey,
        VersionRequirement,
    },
    tonic,
};
use ExecutorError::AttachmentReadError;

#[derive(Default, Debug)]
pub struct ExecutorContext {
    pub function_name: String,
    pub entrypoint: String,
    pub code: Option<FunctionAttachment>,
    pub arguments: Vec<FunctionArgument>,
}

pub trait FunctionContextExt {
    fn new(
        function_arguments: Vec<FunctionArgument>,
        function_attachments: Vec<FunctionAttachment>,
    ) -> Self;

    fn get_argument<S: AsRef<str>>(&self, key: S) -> Option<&FunctionArgument>;

    fn get_attachment<S: AsRef<str>>(&self, key: S) -> Option<&FunctionAttachment>;
}

impl FunctionContextExt for FunctionContext {
    fn new(
        function_arguments: Vec<FunctionArgument>,
        function_attachments: Vec<FunctionAttachment>,
    ) -> Self {
        Self {
            arguments: function_arguments,
            attachments: function_attachments,
        }
    }

    fn get_argument<S: AsRef<str>>(&self, key: S) -> Option<&FunctionArgument> {
        self.arguments.iter().find(|a| a.name == key.as_ref())
    }

    fn get_attachment<S: AsRef<str>>(&self, key: S) -> Option<&FunctionAttachment> {
        self.attachments.iter().find(|a| a.name == key.as_ref())
    }
}

pub trait FunctionExecutor: Debug {
    fn execute(
        &self,
        executor_context: ExecutorContext,
        function_context: FunctionContext,
    ) -> Result<ProtoResult, ExecutorError>;
}

pub trait AttachmentDownload {
    fn download(&self) -> Result<Vec<u8>, ExecutorError>;
}

/// Download function attachment from the given URL
///
/// TODO: This is a huge security hole ⛳️ and needs to be managed properly (gpg sign 🔏 things?)
impl AttachmentDownload for FunctionAttachment {
    fn download(&self) -> Result<Vec<u8>, ExecutorError> {
        let url =
            Url::parse(&self.url).map_err(|e| ExecutorError::InvalidCodeUrl(e.to_string()))?;
        match url.scheme() {
            "file" => {
                let content = fs::read(url.path())
                    .map_err(|e| AttachmentReadError(url.to_string(), e.to_string()))?;

                // TODO: this should be generalized when we
                // have other transports (like http(s))
                // validate integrity
                self.checksums
                    .as_ref()
                    .ok_or(ExecutorError::MissingChecksums)
                    .and_then(|checksums| {
                        let mut hasher = Sha256::new();
                        hasher.input(&content);

                        let checksum = hasher.result();

                        if &checksum[..] != hex::decode(checksums.sha256.clone())?.as_slice() {
                            Err(ExecutorError::ChecksumMismatch {
                                attachment_name: self.name.clone(),
                                wanted: checksums.sha256.clone(),
                                got: hex::encode(checksum),
                            })
                        } else {
                            Ok(())
                        }
                    })?;

                Ok(content)
            }
            s => Err(ExecutorError::UnsupportedTransport(s.to_owned())),
        }
    }
}

#[derive(Debug)]
pub struct FunctionAdapter {
    executor: Box<dyn FunctionExecutor>,
    function_descriptor: FunctionDescriptor,
    logger: Logger,
}

/// Adapter for functions to act as executors
impl FunctionAdapter {
    pub fn new(
        executor: Box<dyn FunctionExecutor>,
        function_descriptor: FunctionDescriptor,
        logger: Logger,
    ) -> Self {
        Self {
            executor,
            function_descriptor,
            logger,
        }
    }
}

impl FunctionExecutor for FunctionAdapter {
    fn execute(
        &self,
        executor_context: ExecutorContext,
        function_context: FunctionContext,
    ) -> Result<ProtoResult, ExecutorError> {
        let mut function_arguments = vec![];

        // not having any code for the function is a valid case used for example to execute
        // external functions (gcp, aws lambdas, etc)
        if let Some(code) = executor_context.code {
            let mut code_buf = Vec::with_capacity(code.encoded_len());
            code.encode(&mut code_buf)?;

            function_arguments.push(FunctionArgument {
                name: "_code".to_owned(),
                r#type: ArgumentType::Bytes as i32,
                value: code_buf,
            });

            let checksums = code.checksums.ok_or(ExecutorError::MissingChecksums)?;
            function_arguments.push(FunctionArgument {
                name: "_sha256".to_owned(),
                r#type: ArgumentType::String as i32,
                value: checksums.sha256.as_bytes().to_vec(),
            });
        }

        function_arguments.push(FunctionArgument {
            name: "_entrypoint".to_owned(),
            r#type: ArgumentType::String as i32,
            value: executor_context.entrypoint.as_bytes().to_vec(),
        });

        let mut manifest_executor_arguments = executor_context.arguments;
        function_arguments.append(&mut manifest_executor_arguments);

        // nest arguments and attachments
        let mut buf: Vec<u8> = Vec::with_capacity(function_context.encoded_len());
        function_context.encode(&mut buf)?;

        function_arguments.push(FunctionArgument {
            name: "_context".to_owned(),
            r#type: ArgumentType::Bytes as i32,
            value: buf,
        });

        let function_name = self
            .function_descriptor
            .function
            .as_ref()
            .ok_or(ExecutorError::FunctionDescriptorMissingFunction)?
            .name
            .clone();

        let function_exe_env = self
            .function_descriptor
            .execution_environment
            .clone()
            .ok_or_else(|| ExecutorError::MissingExecutionEnvironment(function_name.clone()))?;

        self.executor.execute(
            ExecutorContext {
                function_name,
                entrypoint: function_exe_env.entrypoint,
                code: self.function_descriptor.code.clone(),
                arguments: function_exe_env.args,
            },
            FunctionContext::new(
                function_arguments,
                self.function_descriptor.attachments.clone(),
            ),
        )
    }
}

async fn get_function_with_execution_environment(
    registry: &dyn FunctionsRegistry,
    exec_env: &str,
    version_requirement: Option<VersionReq>,
) -> Option<FunctionDescriptor> {
    let mut execution_env_metadata = HashMap::new();
    execution_env_metadata.insert("type".to_owned(), "execution-environment".to_owned());
    execution_env_metadata.insert("execution-environment".to_owned(), exec_env.to_owned());

    let result = registry
        .list(tonic::Request::new(ListRequest {
            name_filter: "".to_owned(),
            metadata_filter: execution_env_metadata,
            metadata_key_filter: vec![],
            offset: 0,
            limit: 1,
            exact_name_match: false,
            version_requirement: version_requirement.map(|vr| VersionRequirement {
                expression: vr.to_string(),
            }),
            order_direction: OrderingDirection::Descending as i32,
            order_by: OrderingKey::Name as i32,
        }))
        .await
        .ok()?
        .into_inner();

    result.functions.first().cloned()
}

async fn traverse_execution_environments<'a>(
    logger: &'a Logger,
    name: &'a str,
    registry: &'a dyn FunctionsRegistry,
) -> Result<Vec<(FunctionDescriptor, Logger)>, ExecutorError> {
    let mut exec_env = name.to_owned();
    let mut function_descriptors = vec![];
    let mut ids = HashSet::new();

    loop {
        match exec_env.as_str() {
            "wasi" => break,
            ee => {
                let function_descriptor =
                    get_function_with_execution_environment(registry, ee, None)
                        .await
                        .ok_or_else(|| ExecutorError::ExecutorNotFound(ee.to_owned()))?;

                exec_env = function_descriptor
                    .execution_environment
                    .as_ref()
                    .ok_or_else(|| ExecutorError::MissingExecutionEnvironment("".to_owned()))?
                    .name
                    .clone();

                function_descriptors.push((
                    function_descriptor.clone(),
                    function_descriptors
                        .last()
                        .map(|(_fd, logger)| logger)
                        .unwrap_or(logger)
                        .new(o!("executor" => exec_env.clone())),
                ));

                if !ids.insert(
                    function_descriptor
                        .function
                        .and_then(|f| f.id)
                        .map(|id| id.value)
                        .unwrap_or_else(|| "invalid-function-id".to_owned()), // This should never happen lol
                ) {
                    return Err(ExecutorError::ExecutorDependencyCycle(DependencyCycle {
                        dependencies: function_descriptors
                            .iter()
                            .map(|(fd, _log)| {
                                (
                                    fd.function
                                        .as_ref()
                                        .map(|f| f.name.clone())
                                        .unwrap_or_else(|| "invalid function".to_owned()),
                                    fd.execution_environment
                                        .as_ref()
                                        .map(|ee| ee.name.clone())
                                        .unwrap_or_else(|| {
                                            "invalid execution environment".to_owned()
                                        }),
                                )
                            })
                            .collect(),
                    }));
                }
            }
        }
    }

    function_descriptors.reverse();
    Ok(function_descriptors)
}

pub async fn get_execution_env_inputs<'a>(
    logger: Logger,
    registry: &'a dyn FunctionsRegistry,
    name: &'a str,
) -> Result<Vec<FunctionInput>, ExecutorError> {
    let function_descriptors = traverse_execution_environments(&logger, name, registry).await?;
    Ok(function_descriptors
        .into_iter()
        .filter_map(|(fd, _logger)| fd.function.map(|f| f.inputs))
        .flatten()
        .map(|mut i| {
            i.from_execution_environment = true;
            i
        })
        .collect())
}

/// Lookup an executor for the given `name`
///
/// If an executor is not supported, an error is returned
pub async fn lookup_executor<'a>(
    logger: Logger,
    name: &'a str,
    registry: &'a dyn FunctionsRegistry,
) -> Result<Box<dyn FunctionExecutor>, ExecutorError> {
    let function_descriptors = traverse_execution_environments(&logger, name, registry).await?;

    // TODO: now we are assuming that the stop condition for the above function
    // was "wasi". This may not be true later
    let executor = Box::new(WasiExecutor::new(
        function_descriptors
            .last()
            .map(|(_fd, logger)| logger)
            .unwrap_or(&logger)
            .new(o!("executor" => "wasi")),
    ));

    Ok(function_descriptors
        .into_iter()
        .fold(executor, |prev_executor, (fd, fd_logger)| {
            Box::new(FunctionAdapter::new(prev_executor, fd, fd_logger))
        }))
}

fn validate_argument_type(arg_type: ArgumentType, argument_value: &[u8]) -> Result<(), String> {
    match arg_type {
        ArgumentType::String => str::from_utf8(&argument_value)
            .map(|_| ())
            .map_err(|_| arg_type.to_string()),
        ArgumentType::Int | ArgumentType::Float => {
            if argument_value.len() == 8 {
                Ok(())
            } else {
                Err(arg_type.to_string())
            }
        }
        ArgumentType::Bool => {
            if argument_value.len() == 1 {
                Ok(())
            } else {
                Err(arg_type.to_string())
            }
        }
        ArgumentType::Bytes => Ok(()), // really do not know a lot about bytes,
    }
}

fn get_reasonable_value_string(argument_value: &[u8]) -> String {
    const MAX_PRINTABLE_VALUE_LENGTH: usize = 256;
    if argument_value.len() < MAX_PRINTABLE_VALUE_LENGTH {
        String::from_utf8(argument_value.to_vec())
            .unwrap_or_else(|_| String::from("invalid utf-8 string 🚑"))
    } else {
        format!(
            "too long value (> {} bytes, vaccuum tubes will explode) 💣",
            MAX_PRINTABLE_VALUE_LENGTH
        )
    }
}

pub fn validate_results<'a, I>(
    outputs: I,
    results: &FunctionResult,
) -> Result<(), Vec<ExecutorError>>
where
    I: IntoIterator<Item = &'a FunctionOutput>,
{
    let (_, errors): (Vec<_>, Vec<_>) = outputs
        .into_iter()
        .map(|output| {
            results
                .values
                .iter()
                .find(|arg| arg.name == output.name)
                .map_or_else(
                    || Err(ExecutorError::RequiredResultMissing(output.name.clone())),
                    |arg| {
                        if output.r#type == arg.r#type {
                            ArgumentType::from_i32(arg.r#type).map_or_else(
                                || Err(ExecutorError::ResultTypeOutOfRange(arg.r#type)),
                                |at| {
                                    validate_argument_type(at, &arg.value).map_err(|tp| {
                                        ExecutorError::InvalidResultValue {
                                            result_name: arg.name.clone(),
                                            tp,
                                            value: get_reasonable_value_string(&arg.value),
                                        }
                                    })
                                },
                            )
                        } else {
                            Err(ExecutorError::MismatchedResultType {
                                result_name: output.name.clone(),
                                expected: ProtoArgumentTypeToString::to_string(&output.r#type),
                                got: ProtoArgumentTypeToString::to_string(&arg.r#type),
                            })
                        }
                    },
                )
        })
        .partition(Result::is_ok);

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.into_iter().map(Result::unwrap_err).collect())
    }
}

/// Validate arguments
///
/// `inputs` is the functions' description of the arguments and `args` is the passed in arguments
/// as an array of `FunctionArgument`. This function returns all validation errors as a
/// `Vec<ExecutionError>`.
pub fn validate_args<'a, I>(inputs: I, args: &[FunctionArgument]) -> Result<(), Vec<ExecutorError>>
where
    I: IntoIterator<Item = &'a FunctionInput>,
{
    // TODO: Currently we do not error on unknown arguments that were supplied
    // this can be done by generating a list of the arguments that we have used.
    // This list must be equal in size to the supplied arguments list.

    let (_, errors): (Vec<_>, Vec<_>) = inputs
        .into_iter()
        .map(|input| {
            args.iter().find(|arg| arg.name == input.name).map_or_else(
                // argument was not found in the sent in args
                || {
                    if input.required {
                        Err(ExecutorError::RequiredArgumentMissing(input.name.clone()))
                    } else {
                        Ok(())
                    }
                },
                // argument was found in the sent in args, validate it
                |arg| {
                    if input.r#type == arg.r#type {
                        ArgumentType::from_i32(arg.r#type).map_or_else(
                            || Err(ExecutorError::OutOfRangeArgumentType(arg.r#type)),
                            |at| {
                                validate_argument_type(at, &arg.value).map_err(|tp| {
                                    ExecutorError::InvalidArgumentValue {
                                        argument_name: arg.name.clone(),
                                        tp,
                                        value: get_reasonable_value_string(&arg.value),
                                    }
                                })
                            },
                        )
                    } else {
                        Err(ExecutorError::MismatchedArgumentType {
                            argument_name: input.name.clone(),
                            expected: ProtoArgumentTypeToString::to_string(&input.r#type),
                            got: ProtoArgumentTypeToString::to_string(&arg.r#type),
                        })
                    }
                },
            )
        })
        .partition(Result::is_ok);

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.into_iter().map(Result::unwrap_err).collect())
    }
}

trait ProtoArgumentTypeToString {
    fn to_string(&self) -> String;
}

impl ProtoArgumentTypeToString for ArgumentType {
    fn to_string(&self) -> String {
        match self {
            ArgumentType::String => "string",
            ArgumentType::Int => "int",
            ArgumentType::Bool => "bool",
            ArgumentType::Float => "float",
            ArgumentType::Bytes => "bytes",
        }
        .to_owned()
    }
}

impl ProtoArgumentTypeToString for i32 {
    fn to_string(&self) -> String {
        match ArgumentType::from_i32(*self) {
            Some(at) => ProtoArgumentTypeToString::to_string(&at),
            None => "invalid type".to_owned(),
        }
    }
}

#[derive(Debug)]
pub struct DependencyCycle {
    dependencies: Vec<(String, String)>,
}

impl Display for DependencyCycle {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.dependencies
            .iter()
            .map(|(fn_name, ee)| write!(f, "{} ({}) ➡️ ", fn_name, ee))
            .collect::<fmt::Result>()?;
        write!(f, "💥")
    }
}

#[derive(Error, Debug)]
pub enum ExecutorError {
    #[error("Unsupported code transport mechanism: \"{0}\"")]
    UnsupportedTransport(String),

    #[error("Cyclic depencency detected for execution environments: \"{0}\"")]
    ExecutorDependencyCycle(DependencyCycle),

    #[error("Invalid code url: {0}")]
    InvalidCodeUrl(String),

    #[error("Failed to read code from {0}: {1}")]
    AttachmentReadError(String, String),

    #[error("Failed to find executor for execution environment \"{0}\"")]
    ExecutorNotFound(String),

    #[error("Function \"{0}\" did not have an execution environment.")]
    MissingExecutionEnvironment(String),

    #[error("Out of range argument type found: {0}. Protobuf definitions out of date?")]
    OutOfRangeArgumentType(i32),

    #[error(
        "Argument \"{argument_name}\" has unexpected type. Expected \"{expected}\", got \"{got}\""
    )]
    MismatchedArgumentType {
        argument_name: String,
        expected: String,
        got: String,
    },

    #[error("Argument \"{argument_name}\" could not be parsed into \"{tp}\". Value: \"{value}\"")]
    InvalidArgumentValue {
        argument_name: String,
        tp: String,
        value: String,
    },

    #[error("Failed to find required argument {0}")]
    RequiredArgumentMissing(String),

    #[error("Failed to find mandatory result \"{0}\"")]
    RequiredResultMissing(String),

    #[error(
        "Output result \"{result_name}\" has unexpected type. Expected \"{expected}\", got \"{got}\""
    )]
    MismatchedResultType {
        result_name: String,
        expected: String,
        got: String,
    },
    #[error("Out of range result type found: {0}. Protobuf definitions out of date?")]
    ResultTypeOutOfRange(i32),

    #[error("Result \"{result_name}\" could not be parsed into \"{tp}\". Value: \"{value}\"")]
    InvalidResultValue {
        result_name: String,
        tp: String,
        value: String,
    },

    #[error("Function descriptor is missing checksums.")]
    MissingChecksums,

    #[error("Function descriptor is missing field function.")]
    FunctionDescriptorMissingFunction,

    #[error("Failed to encode proto data: {0}")]
    EncodeError(#[from] prost::EncodeError),

    #[error("Code is missing even though it is required for the \"{0}\" executor.")]
    MissingCode(String),

    #[error(
        "Checksum mismatch for attachment \"{attachment_name}\". Wanted: {wanted}, got: {got}"
    )]
    ChecksumMismatch {
        attachment_name: String,
        wanted: String,
        got: String,
    },

    #[error("Failed to decode checksum to bytes: {0}")]
    FailedToDecodeChecksum(#[from] hex::FromHexError),
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;

    use crate::registry::FunctionsRegistryService;
    use gbk_protocols::functions::{
        ExecutionEnvironment, Function, FunctionArgument, FunctionId, RegisterRequest, ReturnValue,
    };
    use gbk_protocols_test_helpers::{
        attachment_file, code_file, function_attachment, register_request,
    };

    macro_rules! null_logger {
        () => {{
            slog::Logger::root(slog::Discard, o!())
        }};
    }

    macro_rules! registry {
        () => {{
            FunctionsRegistryService::new(null_logger!())
        }};
    }

    #[test]
    fn parse_required() {
        let inputs = vec![FunctionInput {
            name: "very_important_argument".to_owned(),
            r#type: ArgumentType::String as i32,
            required: true,
            default_value: String::new(),
            from_execution_environment: false,
        }];

        let args = vec![FunctionArgument {
            name: "very_important_argument".to_owned(),
            r#type: ArgumentType::String as i32,
            value: b"yes".to_vec(),
        }];

        let r = validate_args(inputs.iter(), &vec![]);
        assert!(r.is_err());

        let r = validate_args(inputs.iter(), &args);
        assert!(r.is_ok());
    }

    #[test]
    fn parse_optional() {
        let inputs = vec![FunctionInput {
            name: "not_very_important_argument".to_owned(),
            r#type: ArgumentType::String as i32,
            required: false,
            default_value: "something".to_owned(),
            from_execution_environment: false,
        }];

        let r = validate_args(inputs.iter(), &vec![]);
        assert!(r.is_ok());
    }

    #[test]
    fn parse_types() {
        let inputs = vec![
            FunctionInput {
                name: "string_arg".to_owned(),
                r#type: ArgumentType::String as i32,
                required: true,
                default_value: String::new(),
                from_execution_environment: false,
            },
            FunctionInput {
                name: "bool_arg".to_owned(),
                r#type: ArgumentType::Bool as i32,
                required: true,
                default_value: String::new(),
                from_execution_environment: false,
            },
            FunctionInput {
                name: "int_arg".to_owned(),
                r#type: ArgumentType::Int as i32,
                required: true,
                default_value: String::new(),
                from_execution_environment: false,
            },
            FunctionInput {
                name: "float_arg".to_owned(),
                r#type: ArgumentType::Float as i32,
                required: true,
                default_value: String::new(),
                from_execution_environment: false,
            },
            FunctionInput {
                name: "bytes_arg".to_owned(),
                r#type: ArgumentType::Bytes as i32,
                required: false,
                default_value: String::new(),
                from_execution_environment: false,
            },
        ];

        let correct_args = vec![
            FunctionArgument {
                name: "string_arg".to_owned(),
                r#type: ArgumentType::String as i32,
                value: "yes".as_bytes().to_vec(),
            },
            FunctionArgument {
                name: "bool_arg".to_owned(),
                r#type: ArgumentType::Bool as i32,
                value: vec![true as u8],
            },
            FunctionArgument {
                name: "int_arg".to_owned(),
                r#type: ArgumentType::Int as i32,
                value: 4i64.to_le_bytes().to_vec(),
            },
            FunctionArgument {
                name: "float_arg".to_owned(),
                r#type: ArgumentType::Float as i32,
                value: 4.5f64.to_le_bytes().to_vec(),
            },
            FunctionArgument {
                name: "bytes_arg".to_owned(),
                r#type: ArgumentType::Bytes as i32,
                value: vec![13, 37, 13, 37, 13, 37],
            },
        ];

        let r = validate_args(inputs.iter(), &correct_args);

        assert!(r.is_ok());

        // one has the wrong type 🤯
        let almost_correct_args = vec![
            FunctionArgument {
                name: "string_arg".to_owned(),
                r#type: ArgumentType::String as i32,
                value: "yes".as_bytes().to_vec(),
            },
            FunctionArgument {
                name: "bool_arg".to_owned(),
                r#type: ArgumentType::Bool as i32,
                value: 4i64.to_le_bytes().to_vec(),
            },
            FunctionArgument {
                name: "int_arg".to_owned(),
                r#type: ArgumentType::Int as i32,
                value: 4i64.to_le_bytes().to_vec(),
            },
            FunctionArgument {
                name: "float_arg".to_owned(),
                r#type: ArgumentType::Float as i32,
                value: 4.5f64.to_le_bytes().to_vec(),
            },
        ];
        let r = validate_args(inputs.iter(), &almost_correct_args);

        assert!(r.is_err());
        assert_eq!(1, r.unwrap_err().len());

        // all of them has the wrong type 🚓💨
        let no_correct_args = vec![
            FunctionArgument {
                name: "string_arg".to_owned(),
                r#type: ArgumentType::String as i32,
                value: vec![0, 159, 146, 150], // not a valid utf-8 string,
            },
            FunctionArgument {
                name: "bool_arg".to_owned(),
                r#type: ArgumentType::Bool as i32,
                value: 4i64.to_le_bytes().to_vec(),
            },
            FunctionArgument {
                name: "int_arg".to_owned(),
                r#type: ArgumentType::Int as i32,
                value: vec![0, 159, 146, 150, 99], // too long to be an int,
            },
            FunctionArgument {
                name: "float_arg".to_owned(),
                r#type: ArgumentType::Float as i32,
                value: vec![0, 159, 146, 150, 99], // too long to be a float,
            },
        ];
        let r = validate_args(inputs.iter(), &no_correct_args);

        assert!(r.is_err());
        assert_eq!(4, r.unwrap_err().len());
    }

    // Tests for validating results
    #[test]
    fn validate_outputs() {
        let outputs = vec![FunctionOutput {
            name: "very_important_output".to_owned(),
            r#type: ArgumentType::String as i32,
        }];

        let result = FunctionResult {
            values: vec![ReturnValue {
                name: "very_important_output".to_owned(),
                r#type: ArgumentType::String as i32,
                value: vec![],
            }],
        };

        // no values
        let r = validate_results(outputs.iter(), &FunctionResult { values: vec![] });
        assert!(r.is_err());

        // ok values
        let r = validate_results(outputs.iter(), &result);
        assert!(r.is_ok());

        // give bad type
        let result = FunctionResult {
            values: vec![ReturnValue {
                name: "very_important_output".to_owned(),
                r#type: ArgumentType::String as i32,
                value: vec![0, 159, 146, 150], // not a valid utf-8 string,,
            }],
        };

        let r = validate_results(outputs.iter(), &result);
        assert!(r.is_err());
        let err = r.unwrap_err();
        assert_eq!(1, err.len());
        assert!(matches!(err.first().unwrap(), ExecutorError::InvalidResultValue { .. }));
    }

    #[test]
    fn test_download() {
        // non-existent file
        let attachment = function_attachment!("file://this-file-does-not-exist");
        let r = attachment.download();
        assert!(r.is_err());
        assert!(matches!(
            r.unwrap_err(),
            ExecutorError::AttachmentReadError(..)
        ));

        // invalid url
        let attachment = function_attachment!("this-is-not-url");
        let r = attachment.download();
        assert!(r.is_err());
        assert!(matches!(r.unwrap_err(), ExecutorError::InvalidCodeUrl(..)));

        // unsupported scheme
        let attachment = function_attachment!("unsupported://that-scheme.fabrikam.com");
        let r = attachment.download();
        assert!(r.is_err());
        assert!(matches!(
            r.unwrap_err(),
            ExecutorError::UnsupportedTransport(..)
        ));

        // actual file
        let s = "some data 🖥️";
        let attachment = attachment_file!(s.as_bytes(), "somename");
        let r = attachment.download();
        assert!(r.is_ok());
        assert_eq!(s.as_bytes(), r.unwrap().as_slice());
    }

    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Default, Debug)]
    pub struct FakeExecutor {
        executor_context: RefCell<ExecutorContext>,
        function_context: RefCell<FunctionContext>,
        downloaded_code: RefCell<Vec<u8>>,
    }

    impl FunctionExecutor for Rc<FakeExecutor> {
        fn execute(
            &self,
            executor_context: ExecutorContext,
            function_context: FunctionContext,
        ) -> Result<ProtoResult, ExecutorError> {
            *self.downloaded_code.borrow_mut() = executor_context
                .code
                .as_ref()
                .map(|c| c.download().unwrap())
                .unwrap_or_default();
            *self.executor_context.borrow_mut() = executor_context;
            *self.function_context.borrow_mut() = function_context;
            Ok(ProtoResult::Ok(FunctionResult { values: Vec::new() }))
        }
    }

    /*#[test]
    // TODO:
    fn test_bad_checksum_for_code() {
    }*/

    #[test]
    fn test_nested_executor() {
        let fake = Rc::new(FakeExecutor::default());

        let exe_env_args = vec![FunctionArgument {
            name: "sune".to_owned(),
            r#type: ArgumentType::String as i32,
            value: "bune".as_bytes().to_vec(),
        }];

        let s = "some data 🖥️";
        let nested = FunctionAdapter::new(
            Box::new(fake.clone()),
            FunctionDescriptor {
                execution_environment: Some(ExecutionEnvironment {
                    name: "Avlivningsmiljö 🗡️".to_owned(),
                    entrypoint: "ingångspoäng 💯".to_owned(),
                    args: vec![],
                }),
                code: Some(code_file!(s.as_bytes())),
                attachments: vec![],
                host_folder_mounts: vec![],
                function: Some(Function {
                    id: Some(FunctionId {
                        value: "huuuuuus".to_owned(),
                    }),
                    name: "wienerbröööööööö".to_owned(),
                    version: "2019.3-5-PR2".to_owned(),
                    metadata: HashMap::new(),
                    inputs: vec![],
                    outputs: vec![],
                }),
            },
            null_logger!(),
        );
        let code = "asd";
        let args = vec![FunctionArgument {
            name: "test-arg".to_owned(),
            r#type: ArgumentType::String as i32,
            value: "test-value".as_bytes().to_vec(),
        }];
        let entry = "entry";
        let attachments = vec![function_attachment!("fake://")];
        let result = nested.execute(
            ExecutorContext {
                function_name: "test".to_owned(),
                entrypoint: entry.to_owned(),
                code: Some(code_file!(code.as_bytes())),
                arguments: exe_env_args.clone(),
            },
            FunctionContext::new(args, attachments.clone()),
        );
        // Test that code got passed
        assert!(result.is_ok());
        assert_eq!(s.as_bytes(), fake.downloaded_code.borrow().as_slice());

        // Test that the argument we send in is passed through
        {
            let fc = fake.function_context.borrow();
            let fake_args = fc.arguments.clone();
            assert_eq!(fake_args.len(), 5);
            let code_attachment =
                FunctionAttachment::decode(fc.get_argument("_code").unwrap().value.as_slice())
                    .unwrap();
            assert_eq!(code_attachment.download().unwrap(), code.as_bytes());
            assert_eq!(
                fc.get_argument("_entrypoint").unwrap().value,
                entry.as_bytes()
            );
            assert_eq!(
                fc.get_argument("_sha256").unwrap().value,
                "688787d8ff144c502c7f5cffaafe2cc588d86079f9de88304c26b0cb99ce91c6".as_bytes()
            );

            let inner_context =
                FunctionContext::decode(fc.get_argument("_context").unwrap().value.as_slice())
                    .unwrap();
            assert_eq!(
                inner_context.get_argument("test-arg").unwrap().value,
                "test-value".as_bytes()
            );

            // Test that we get the execution environment args we supplied earlier
            let fake_exe_args = &fake.executor_context.borrow().arguments.clone();
            assert_eq!(fake_exe_args.len(), 0);
            assert_eq!(
                fake_args.iter().find(|v| v.name == "sune").unwrap().value,
                "bune".as_bytes()
            );
        }

        // Test president! 🕴
        {
            let args = vec![FunctionArgument {
                name: "_code".to_owned(), // deliberately use a reserved word 👿
                r#type: ArgumentType::String as i32,
                value: "this-is-not-code".as_bytes().to_vec(),
            }];

            let exec_args = vec![FunctionArgument {
                name: "the-arg".to_owned(),
                r#type: ArgumentType::String as i32,
                value: "this-is-exec-arg".as_bytes().to_vec(),
            }];
            let result = nested.execute(
                ExecutorContext {
                    function_name: "test".to_owned(),
                    entrypoint: "entry".to_owned(),
                    code: Some(code_file!("code".as_bytes())),
                    arguments: exec_args,
                },
                FunctionContext::new(args, vec![]),
            );

            assert!(result.is_ok());
            let fc = &fake.function_context.borrow();
            assert!(fc.get_argument("_code").is_some());
            assert!(fc.get_argument("_sha256").is_some());
            assert!(fc.get_argument("_entrypoint").is_some());

            // make sure that we get the actual code and not the argument named _code
            let code_attachment =
                FunctionAttachment::decode(fc.get_argument("_code").unwrap().value.as_slice())
                    .unwrap();
            assert_eq!(
                String::from_utf8(code_attachment.download().unwrap()).unwrap(),
                "code"
            );
        }

        // Double nested 🎰
        let nested2 = FunctionAdapter::new(
            Box::new(nested),
            FunctionDescriptor {
                execution_environment: Some(ExecutionEnvironment {
                    name: "Avlivningsmiljö 🗡️".to_owned(),
                    entrypoint: "ingångspoäng 💯".to_owned(),
                    args: vec![],
                }),
                code: Some(code_file!(s.as_bytes())),
                attachments: vec![],
                host_folder_mounts: vec![],
                function: Some(Function {
                    id: Some(FunctionId {
                        value: "pieceee of pie".to_owned(),
                    }),
                    name: "mandelkubb".to_owned(),
                    version: "2022.1-5-PR50".to_owned(),
                    metadata: HashMap::new(),
                    inputs: vec![],
                    outputs: vec![],
                }),
            },
            null_logger!(),
        );
        {
            let args = vec![FunctionArgument {
                name: "test-arg".to_owned(),
                r#type: ArgumentType::String as i32,
                value: "test-value".as_bytes().to_vec(),
            }];
            let result = nested2.execute(
                ExecutorContext {
                    function_name: "test".to_owned(),
                    entrypoint: "entry".to_owned(),
                    code: Some(code_file!("vec".as_bytes())),
                    arguments: exe_env_args,
                },
                FunctionContext::new(args, vec![]),
            );

            assert!(result.is_ok());
            let fc = &fake.function_context.borrow();
            assert!(fc.get_argument("_code").is_some());
            assert!(fc.get_argument("_sha256").is_some());
            assert!(fc.get_argument("_entrypoint").is_some());
        }
    }

    #[test]
    fn test_lookup_executor() {
        // get wasi executor
        let fr = registry!();
        let res = futures::executor::block_on(lookup_executor(null_logger!(), "wasi", &fr));
        assert!(res.is_ok());

        // get non existing executor
        let res = futures::executor::block_on(lookup_executor(null_logger!(), "ur-sula!", &fr));
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            ExecutorError::ExecutorNotFound(..)
        ));

        // get function executor
        let mut wasi_executor_metadata = HashMap::new();
        wasi_executor_metadata.insert("type".to_owned(), "execution-environment".to_owned());
        wasi_executor_metadata.insert(
            "execution-environment".to_owned(),
            "oran-malifant".to_owned(),
        );

        let mut nested_executor_metadata = HashMap::new();
        nested_executor_metadata.insert("type".to_owned(), "execution-environment".to_owned());
        nested_executor_metadata.insert(
            "execution-environment".to_owned(),
            "precious-granag".to_owned(),
        );

        let mut broken_executor_metadata = HashMap::new();
        broken_executor_metadata.insert("type".to_owned(), "execution-environment".to_owned());
        broken_executor_metadata.insert(
            "execution-environment".to_owned(),
            "broken-chain-executor".to_owned(),
        );

        vec![
            RegisterRequest {
                execution_environment: Some(ExecutionEnvironment {
                    name: "wasi".to_owned(),
                    entrypoint: "wasi.kexe".to_owned(),
                    args: vec![],
                }),
                code: None,
                name: "oran-func".to_owned(),
                version: "0.1.1".to_owned(),
                metadata: wasi_executor_metadata,
                inputs: vec![],
                outputs: vec![],
                attachment_ids: vec![],
                host_folder_mounts: vec![],
            },
            RegisterRequest {
                execution_environment: Some(ExecutionEnvironment {
                    name: "oran-malifant".to_owned(),
                    entrypoint: "oran hehurr".to_owned(),
                    args: vec![],
                }),
                code: None,
                name: "precious-granag".to_owned(),
                version: "8.1.5".to_owned(),
                metadata: nested_executor_metadata,
                inputs: vec![],
                outputs: vec![],
                attachment_ids: vec![],
                host_folder_mounts: vec![],
            },
            RegisterRequest {
                execution_environment: Some(ExecutionEnvironment {
                    name: "oran-elefant".to_owned(),
                    entrypoint: "oran hehurr".to_owned(),
                    args: vec![],
                }),
                code: None,
                name: "precious-granag".to_owned(),
                version: "3.2.2".to_owned(),
                metadata: broken_executor_metadata,
                inputs: vec![],
                outputs: vec![],
                attachment_ids: vec![],
                host_folder_mounts: vec![],
            },
        ]
        .into_iter()
        .for_each(|rr| {
            futures::executor::block_on(fr.register(tonic::Request::new(rr)))
                .map_or_else(|e| panic!(e.to_string()), |_| ())
        });

        let res =
            futures::executor::block_on(lookup_executor(null_logger!(), "oran-malifant", &fr));
        assert!(res.is_ok());

        // Get two smetadatae executor
        let res =
            futures::executor::block_on(lookup_executor(null_logger!(), "precious-granag", &fr));
        assert!(res.is_ok());

        // get function executor missing link
        let res = futures::executor::block_on(lookup_executor(
            null_logger!(),
            "broken-chain-executor",
            &fr,
        ));
        assert!(res.is_err());

        assert!(matches!(
            res.unwrap_err(),
            ExecutorError::ExecutorNotFound(..)
        ));
    }

    #[test]
    fn test_cyclic_dependency_check() {
        let fr = registry!();
        let res = futures::executor::block_on(lookup_executor(null_logger!(), "wasi", &fr));
        assert!(res.is_ok());

        // get non existing executor
        let res = futures::executor::block_on(lookup_executor(null_logger!(), "ur-sula!", &fr));
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            ExecutorError::ExecutorNotFound(..)
        ));

        // get function executor
        let mut wasi_executor_metadata = HashMap::new();
        wasi_executor_metadata.insert("type".to_owned(), "execution-environment".to_owned());
        wasi_executor_metadata.insert("execution-environment".to_owned(), "aa-exec".to_owned());

        let mut nested_executor_metadata = HashMap::new();
        nested_executor_metadata.insert("type".to_owned(), "execution-environment".to_owned());
        nested_executor_metadata.insert("execution-environment".to_owned(), "bb-exec".to_owned());

        let mut broken_executor_metadata = HashMap::new();
        broken_executor_metadata.insert("type".to_owned(), "execution-environment".to_owned());
        broken_executor_metadata.insert("execution-environment".to_owned(), "cc-exec".to_owned());

        vec![
            register_request!(
                "aa-func",
                "0.1.1",
                ExecutionEnvironment {
                    name: "bb-exec".to_owned(),
                    entrypoint: "wasi.kexe".to_owned(),
                    args: vec![],
                },
                { "type" => "execution-environment", "execution-environment" => "aa-exec" }
            ),
            register_request!(
                "bb-func",
                "0.1.5",
                ExecutionEnvironment {
                    name: "cc-exec".to_owned(),
                    entrypoint: "wasi.kexe".to_owned(),
                    args: vec![],
                },
                { "type" => "execution-environment", "execution-environment" => "bb-exec" }
            ),
            register_request!(
                "cc-func",
                "3.2.2",
                ExecutionEnvironment {
                    name: "bb-exec".to_owned(),
                    entrypoint: "wasi.kexe".to_owned(),
                    args: vec![],
                },
                { "type" => "execution-environment", "execution-environment" => "cc-exec" }
            ),
        ]
        .into_iter()
        .for_each(|rr| {
            futures::executor::block_on(fr.register(tonic::Request::new(rr)))
                .map_or_else(|e| panic!(e.to_string()), |_| ())
        });

        let res = futures::executor::block_on(lookup_executor(null_logger!(), "aa-exec", &fr));
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            ExecutorError::ExecutorDependencyCycle(..)
        ));
    }
}
