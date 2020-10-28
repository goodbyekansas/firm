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
use firm_protocols::{
    execution::{
        execution_result::Result as ProtoResult,
        execution_server::Execution as ExecutionServiceTrait, ExecutionError, ExecutionId,
        ExecutionParameters, ExecutionResult, InputValue, OutputValue, OutputValues,
    },
    functions::{Attachment, AuthMethod, Function, Input, Output, Type},
    registry::{registry_server::Registry, Filters, Ordering, OrderingKey, VersionRequirement},
    tonic,
    wasi::{Attachments, InputValues},
};
use ExecutorError::AttachmentReadError;

pub struct ExecutionService {
    log: Logger,
    registry: Box<dyn Registry>,
}

impl ExecutionService {
    pub fn new(log: Logger, registry: Box<dyn Registry>) -> Self {
        Self { log, registry }
    }

    /// Lookup an executor for the given `runtime_name`
    ///
    /// If an executor is not supported, an error is returned
    pub async fn lookup_executor_for_runtime(
        &self,
        runtime_name: &str,
    ) -> Result<Box<dyn FunctionExecutor>, ExecutorError> {
        let functions = self.traverse_runtimes(runtime_name).await?;

        // TODO: now we are assuming that the stop condition for the above function
        // was "wasi". This may not be true later
        let executor = Box::new(WasiExecutor::new(
            functions
                .last()
                .map(|(_fd, logger)| logger)
                .unwrap_or(&self.log)
                .new(o!("executor" => "wasi")),
        ));

        Ok(functions
            .into_iter()
            .fold(executor, |prev_executor, (fd, fd_logger)| {
                Box::new(FunctionAdapter::new(prev_executor, fd, fd_logger))
            }))
    }

    async fn traverse_runtimes(
        &self,
        runtime_name: &str,
    ) -> Result<Vec<(Function, Logger)>, ExecutorError> {
        let mut runtime = runtime_name.to_owned();
        let mut functions = vec![];
        let mut ids = HashSet::new();

        loop {
            match runtime.as_str() {
                "wasi" => break,
                rt => {
                    let function = self
                        .get_executor_function_for_runtime(rt, None) // TODO: runtime version requirements
                        .await
                        .ok_or_else(|| ExecutorError::ExecutorNotFound(rt.to_owned()))?;

                    runtime = function
                        .runtime
                        .as_ref()
                        .ok_or_else(|| ExecutorError::MissingRuntime("".to_owned()))?
                        .name
                        .clone();

                    functions.push((
                        function.clone(),
                        functions
                            .last()
                            .map(|(_fd, logger)| logger)
                            .unwrap_or(&self.log)
                            .new(o!("executor" => runtime.clone())),
                    ));

                    if !ids.insert(format!("{}-{}", &function.name, &function.version)) {
                        return Err(ExecutorError::ExecutorDependencyCycle(DependencyCycle {
                            dependencies: functions
                                .iter()
                                .map(|(f, _log)| {
                                    (
                                        f.name.clone(),
                                        f.runtime
                                            .as_ref()
                                            .map(|rt| rt.name.clone())
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

        functions.reverse();
        Ok(functions)
    }

    async fn get_executor_function_for_runtime(
        &self,
        runtime: &str,
        version_requirement: Option<VersionReq>,
    ) -> Option<Function> {
        let mut execution_env_metadata = HashMap::new();
        execution_env_metadata.insert("executor-for".to_owned(), runtime.to_owned());

        let result = self
            .registry
            .list(tonic::Request::new(Filters {
                name_filter: None,
                version_requirement: version_requirement.map(|vr| VersionRequirement {
                    expression: vr.to_string(),
                }),
                metadata_filter: execution_env_metadata,
                order: Some(Ordering {
                    key: OrderingKey::NameVersion as i32,
                    reverse: false,
                    offset: 0,
                    limit: 1,
                }),
            }))
            .await
            .ok()?
            .into_inner();

        result.functions.first().cloned()
    }
}

#[tonic::async_trait]
impl ExecutionServiceTrait for ExecutionService {
    async fn execute(
        &self,
        request: tonic::Request<ExecutionParameters>,
    ) -> Result<tonic::Response<ExecutionResult>, tonic::Status> {
        // lookup function
        let payload = request.into_inner();
        let args = payload.arguments;
        let function = payload.function.ok_or_else(|| {
            tonic::Status::new(
                tonic::Code::InvalidArgument,
                "Execute request needs to contain a function.",
            )
        })?;

        // validate args
        validate_args(function.inputs.iter(), &args).map_err(|e| {
            tonic::Status::new(
                tonic::Code::InvalidArgument,
                format!(
                    "Invalid function arguments: {}",
                    e.iter()
                        .map(|ae| format!("{}", ae))
                        .collect::<Vec<String>>()
                        .join(", ")
                ),
            )
        })?;

        let runtime = function.runtime.as_ref().ok_or_else(|| {
            tonic::Status::new(
                tonic::Code::Internal,
                "Function descriptor did not contain any runtime specification.",
            )
        })?;

        let function_name = function.name.clone();

        // lookup executor and run
        self.lookup_executor_for_runtime(&runtime.name)
            .await
            .map_err(|e| {
                tonic::Status::new(
                    tonic::Code::Internal,
                    format!("Failed to lookup function executor: {}", e),
                )
            })
            .and_then(|executor| {
                let res = executor.execute(
                    ExecutorParameters {
                        function_name: function_name.clone(),
                        entrypoint: runtime.entrypoint.to_owned(),
                        code: function.code.clone(),
                        arguments: runtime.arguments.clone(),
                    },
                    args,
                    function.attachments.clone(),
                );
                match res {
                    Ok(Ok(r)) => validate_results(function.outputs.iter(), &r)
                        .map(|_| {
                            tonic::Response::new(ExecutionResult {
                                // TODO: We do not have a cookie for this yet. Should be a cookie for looking up logs of the execution etc.
                                execution_id: Some(ExecutionId {
                                    uuid: uuid::Uuid::new_v4().to_string(),
                                }),
                                result: Some(ProtoResult::Ok(OutputValues { values: r })),
                            })
                        })
                        .map_err(|e| {
                            tonic::Status::new(
                                tonic::Code::InvalidArgument,
                                format!(
                                    "Function \"{}\" generated invalid result: {}",
                                    &function_name,
                                    e.iter()
                                        .map(|ae| format!("{}", ae))
                                        .collect::<Vec<String>>()
                                        .join(", ")
                                ),
                            )
                        }),
                    Ok(Err(e)) => Ok(tonic::Response::new(ExecutionResult {
                        // TODO: We do not have a cookie for this yet. Should be a cookie for looking up logs of the execution etc.
                        execution_id: Some(ExecutionId {
                            uuid: uuid::Uuid::new_v4().to_string(),
                        }),
                        result: Some(ProtoResult::Error(ExecutionError { msg: e })),
                    })),

                    Err(e) => Err(tonic::Status::new(
                        tonic::Code::Internal,
                        format!("Failed to execute function {}: {}", &function_name, e),
                    )),
                }
            })
    }
}

#[derive(Default, Debug)]
pub struct ExecutorParameters {
    pub function_name: String,
    pub entrypoint: String,
    pub code: Option<Attachment>,
    pub arguments: HashMap<String, String>,
}

pub trait FunctionExecutor: Debug {
    fn execute(
        &self,
        executor_context: ExecutorParameters,
        arguments: Vec<InputValue>,
        attachments: Vec<Attachment>,
    ) -> Result<Result<Vec<OutputValue>, String>, ExecutorError>;
}

pub trait AttachmentDownload {
    fn download(&self) -> Result<Vec<u8>, ExecutorError>;
}

/// Download function attachment from the given URL
///
/// TODO: This is a huge security hole ⛳️ and needs to be managed properly (gpg sign 🔏 things?)
impl AttachmentDownload for Attachment {
    fn download(&self) -> Result<Vec<u8>, ExecutorError> {
        let url = self
            .url
            .as_ref()
            .ok_or_else(|| ExecutorError::InvalidCodeUrl("Attachment missing url.".to_owned()))
            .and_then(|u| {
                Url::parse(&u.url)
                    .map_err(|e| ExecutorError::InvalidCodeUrl(e.to_string()))
                    .map(|b| (b, AuthMethod::from_i32(u.auth_method)))
            })?;

        match (url.0.scheme(), url.1) {
            ("file", _) => {
                let content = fs::read(url.0.path())
                    .map_err(|e| AttachmentReadError(url.0.to_string(), e.to_string()))?;

                // TODO: this should be generalized when we
                // have other transports (like http(s))
                // validate integrity
                self.checksums
                    .as_ref()
                    .ok_or(ExecutorError::MissingChecksums)
                    .and_then(|checksums| {
                        let mut hasher = Sha256::new();
                        hasher.update(&content);

                        let checksum = hasher.finalize();

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
            // ("https", Some(auth)) => {}, // TODO: Support Oauth methods for http
            (s, _) => Err(ExecutorError::UnsupportedTransport(s.to_owned())),
        }
    }
}

#[derive(Debug)]
pub struct FunctionAdapter {
    executor: Box<dyn FunctionExecutor>,
    executor_function: Function,
    logger: Logger,
}

/// Adapter for functions to act as executors
impl FunctionAdapter {
    pub fn new(executor: Box<dyn FunctionExecutor>, function: Function, logger: Logger) -> Self {
        Self {
            executor,
            executor_function: function,
            logger,
        }
    }
}

impl FunctionExecutor for FunctionAdapter {
    fn execute(
        &self,
        executor_context: ExecutorParameters,
        arguments: Vec<InputValue>,
        attachments: Vec<Attachment>,
    ) -> Result<Result<Vec<OutputValue>, String>, ExecutorError> {
        let mut executor_function_arguments = vec![];

        // not having any code for the function is a valid case used for example to execute
        // external functions (gcp, aws lambdas, etc)
        if let Some(code) = executor_context.code {
            let mut code_buf = Vec::with_capacity(code.encoded_len());
            code.encode(&mut code_buf)?;

            executor_function_arguments.push(InputValue {
                name: "_code".to_owned(),
                r#type: Type::Bytes as i32,
                value: code_buf,
            });

            let checksums = code.checksums.ok_or(ExecutorError::MissingChecksums)?;
            executor_function_arguments.push(InputValue {
                name: "_sha256".to_owned(),
                r#type: Type::String as i32,
                value: checksums.sha256.as_bytes().to_vec(),
            });
        }

        executor_function_arguments.push(InputValue {
            name: "_entrypoint".to_owned(),
            r#type: Type::String as i32,
            value: executor_context.entrypoint.as_bytes().to_vec(),
        });

        executor_function_arguments.append(
            &mut executor_context
                .arguments
                .into_iter()
                .map(|(k, v)| InputValue {
                    name: k,
                    r#type: Type::String as i32,
                    value: v.as_bytes().to_vec(),
                })
                .collect(),
        );

        // nest arguments and attachments
        let proto_args = InputValues { values: arguments };
        let mut arguments_buf: Vec<u8> = Vec::with_capacity(proto_args.encoded_len());
        proto_args.encode(&mut arguments_buf)?;
        executor_function_arguments.push(InputValue {
            name: "_arguments".to_owned(),
            r#type: Type::Bytes as i32,
            value: arguments_buf,
        });

        let proto_attachments = Attachments { attachments };
        let mut attachments_buf: Vec<u8> = Vec::with_capacity(proto_attachments.encoded_len());
        proto_attachments.encode(&mut attachments_buf)?;
        executor_function_arguments.push(InputValue {
            name: "_attachments".to_owned(),
            r#type: Type::Bytes as i32,
            value: attachments_buf,
        });

        let function_exe_env =
            self.executor_function.runtime.clone().ok_or_else(|| {
                ExecutorError::MissingRuntime(self.executor_function.name.clone())
            })?;

        self.executor.execute(
            ExecutorParameters {
                function_name: self.executor_function.name.clone(),
                entrypoint: function_exe_env.entrypoint,
                code: self.executor_function.code.clone(),
                arguments: function_exe_env.arguments,
            },
            executor_function_arguments,
            self.executor_function.attachments.clone(),
        )
    }
}

fn validate_argument_type(arg_type: Type, argument_value: &[u8]) -> Result<(), String> {
    match arg_type {
        Type::String => str::from_utf8(&argument_value)
            .map(|_| ())
            .map_err(|_| arg_type.to_string()),
        Type::Int | Type::Float => {
            if argument_value.len() == 8 {
                Ok(())
            } else {
                Err(arg_type.to_string())
            }
        }
        Type::Bool => {
            if argument_value.len() == 1 {
                Ok(())
            } else {
                Err(arg_type.to_string())
            }
        }
        Type::Bytes => Ok(()), // really do not know a lot about bytes,
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
    results: &[OutputValue],
) -> Result<(), Vec<ExecutorError>>
where
    I: IntoIterator<Item = &'a Output>,
{
    let (_, errors): (Vec<_>, Vec<_>) = outputs
        .into_iter()
        .map(|output| {
            results
                .iter()
                .find(|arg| arg.name == output.name)
                .map_or_else(
                    || Err(ExecutorError::RequiredResultMissing(output.name.clone())),
                    |arg| {
                        if output.r#type == arg.r#type {
                            Type::from_i32(arg.r#type).map_or_else(
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
/// `inputs` is the functions' description of the arguments and `args` is the passed-in arguments
/// as an array of `InputValue`. This function returns all validation errors as a
/// `Vec<ExecutionError>`.
fn validate_args<'a, I>(inputs: I, args: &[InputValue]) -> Result<(), Vec<ExecutorError>>
where
    I: IntoIterator<Item = &'a Input>,
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
                        Type::from_i32(arg.r#type).map_or_else(
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

impl ProtoArgumentTypeToString for Type {
    fn to_string(&self) -> String {
        match self {
            Type::String => "string",
            Type::Int => "int",
            Type::Bool => "bool",
            Type::Float => "float",
            Type::Bytes => "bytes",
        }
        .to_owned()
    }
}

impl ProtoArgumentTypeToString for i32 {
    fn to_string(&self) -> String {
        match Type::from_i32(*self) {
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

    #[error("Function \"{0}\" did not have a runtime specified.")]
    MissingRuntime(String),

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

    #[error("Function is missing checksums.")]
    MissingChecksums,

    #[error("Function is missing id.")]
    FunctionMissingId,

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

    use std::{cell::RefCell, collections::HashMap, rc::Rc};

    use crate::registry::RegistryService;
    use firm_protocols::{functions::Runtime, registry::FunctionData};
    use firm_protocols_test_helpers::{attachment, attachment_file, code_file, function_data};

    macro_rules! null_logger {
        () => {{
            slog::Logger::root(slog::Discard, o!())
        }};
    }

    macro_rules! registry {
        () => {{
            RegistryService::new(null_logger!())
        }};
    }

    #[test]
    fn parse_required() {
        let inputs = vec![Input {
            name: "very_important_argument".to_owned(),
            description: "This is importante!".to_owned(),
            r#type: Type::String as i32,
            required: true,
        }];

        let args = vec![InputValue {
            name: "very_important_argument".to_owned(),
            r#type: Type::String as i32,
            value: b"yes".to_vec(),
        }];

        let r = validate_args(inputs.iter(), &[]);
        assert!(r.is_err());

        let r = validate_args(inputs.iter(), &args);
        assert!(r.is_ok());
    }

    #[test]
    fn parse_optional() {
        let inputs = vec![Input {
            name: "not_very_important_argument".to_owned(),
            description: "I do not like this".to_owned(),
            r#type: Type::String as i32,
            required: false,
        }];

        let r = validate_args(inputs.iter(), &[]);
        assert!(r.is_ok());
    }

    #[test]
    fn parse_types() {
        let inputs = vec![
            Input {
                name: "string_arg".to_owned(),
                description: "This is a string arg".to_owned(),
                r#type: Type::String as i32,
                required: true,
            },
            Input {
                name: "bool_arg".to_owned(),
                description: "This is a bool arg".to_owned(),
                r#type: Type::Bool as i32,
                required: true,
            },
            Input {
                name: "int_arg".to_owned(),
                description: "This is an int arg".to_owned(),
                r#type: Type::Int as i32,
                required: true,
            },
            Input {
                name: "float_arg".to_owned(),
                description: "This is a floater 💩".to_owned(),
                r#type: Type::Float as i32,
                required: true,
            },
            Input {
                name: "bytes_arg".to_owned(),
                description: "This is a bytes argument".to_owned(),
                r#type: Type::Bytes as i32,
                required: false,
            },
        ];

        let correct_args = vec![
            InputValue {
                name: "string_arg".to_owned(),
                r#type: Type::String as i32,
                value: b"yes".to_vec(),
            },
            InputValue {
                name: "bool_arg".to_owned(),
                r#type: Type::Bool as i32,
                value: vec![true as u8],
            },
            InputValue {
                name: "int_arg".to_owned(),
                r#type: Type::Int as i32,
                value: 4i64.to_le_bytes().to_vec(),
            },
            InputValue {
                name: "float_arg".to_owned(),
                r#type: Type::Float as i32,
                value: 4.5f64.to_le_bytes().to_vec(),
            },
            InputValue {
                name: "bytes_arg".to_owned(),
                r#type: Type::Bytes as i32,
                value: vec![13, 37, 13, 37, 13, 37],
            },
        ];

        let r = validate_args(inputs.iter(), &correct_args);

        assert!(r.is_ok());

        // one has the wrong type 🤯
        let almost_correct_args = vec![
            InputValue {
                name: "string_arg".to_owned(),
                r#type: Type::String as i32,
                value: b"yes".to_vec(),
            },
            InputValue {
                name: "bool_arg".to_owned(),
                r#type: Type::Bool as i32,
                value: 4i64.to_le_bytes().to_vec(),
            },
            InputValue {
                name: "int_arg".to_owned(),
                r#type: Type::Int as i32,
                value: 4i64.to_le_bytes().to_vec(),
            },
            InputValue {
                name: "float_arg".to_owned(),
                r#type: Type::Float as i32,
                value: 4.5f64.to_le_bytes().to_vec(),
            },
        ];
        let r = validate_args(inputs.iter(), &almost_correct_args);

        assert!(r.is_err());
        assert_eq!(1, r.unwrap_err().len());

        // all of them has the wrong type 🚓💨
        let no_correct_args = vec![
            InputValue {
                name: "string_arg".to_owned(),
                r#type: Type::String as i32,
                value: vec![0, 159, 146, 150], // not a valid utf-8 string,
            },
            InputValue {
                name: "bool_arg".to_owned(),
                r#type: Type::Bool as i32,
                value: 4i64.to_le_bytes().to_vec(),
            },
            InputValue {
                name: "int_arg".to_owned(),
                r#type: Type::Int as i32,
                value: vec![0, 159, 146, 150, 99], // too long to be an int,
            },
            InputValue {
                name: "float_arg".to_owned(),
                r#type: Type::Float as i32,
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
        let outputs = vec![Output {
            name: "very_important_output".to_owned(),
            description: "Yes?".to_owned(),
            r#type: Type::String as i32,
        }];

        let result = vec![OutputValue {
            name: "very_important_output".to_owned(),
            r#type: Type::String as i32,
            value: vec![],
        }];

        // no values
        let r = validate_results(outputs.iter(), &[]);
        assert!(r.is_err());

        // ok values
        let r = validate_results(outputs.iter(), &result);
        assert!(r.is_ok());

        // give bad type
        let result = vec![OutputValue {
            name: "very_important_output".to_owned(),
            r#type: Type::String as i32,
            value: vec![0, 159, 146, 150], // not a valid utf-8 string,,
        }];

        let r = validate_results(outputs.iter(), &result);
        assert!(r.is_err());
        let err = r.unwrap_err();
        assert_eq!(1, err.len());
        assert!(matches!(err.first().unwrap(), ExecutorError::InvalidResultValue { .. }));
    }

    #[test]
    fn test_download() {
        // non-existent file
        let attachment = attachment!("file://this-file-does-not-exist");
        let r = attachment.download();
        assert!(r.is_err());
        assert!(matches!(
            r.unwrap_err(),
            ExecutorError::AttachmentReadError(..)
        ));

        // invalid url
        let attachment = attachment!("this-is-not-url");
        let r = attachment.download();
        assert!(r.is_err());
        assert!(matches!(r.unwrap_err(), ExecutorError::InvalidCodeUrl(..)));

        // unsupported scheme
        let attachment = attachment!("unsupported://that-scheme.fabrikam.com");
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

    #[derive(Default, Debug)]
    pub struct FakeExecutor {
        executor_context: RefCell<ExecutorParameters>,
        arguments: RefCell<Vec<InputValue>>,
        attachments: RefCell<Vec<Attachment>>,
        downloaded_code: RefCell<Vec<u8>>,
    }

    impl FunctionExecutor for Rc<FakeExecutor> {
        fn execute(
            &self,
            executor_context: ExecutorParameters,
            arguments: Vec<InputValue>,
            attachments: Vec<Attachment>,
        ) -> Result<Result<Vec<OutputValue>, String>, ExecutorError> {
            *self.downloaded_code.borrow_mut() = executor_context
                .code
                .as_ref()
                .map(|c| c.download().unwrap())
                .unwrap_or_default();
            *self.executor_context.borrow_mut() = executor_context;
            *self.arguments.borrow_mut() = arguments;
            *self.attachments.borrow_mut() = attachments;
            Ok(Ok(Vec::new()))
        }
    }

    /*#[test]
    // TODO:
    fn test_bad_checksum_for_code() {
    }*/

    #[test]
    fn test_nested_executor() {
        let fake = Rc::new(FakeExecutor::default());

        let mut runtime_args = HashMap::new();
        runtime_args.insert("sune".to_owned(), "bune".to_owned());

        let s = "some data 🖥️";
        let nested = FunctionAdapter::new(
            Box::new(fake.clone()),
            Function {
                runtime: Some(Runtime {
                    name: "Avlivningsmiljö 🗡️".to_owned(),
                    entrypoint: "ingångspoäng 💯".to_owned(),
                    arguments: HashMap::new(),
                }),
                code: Some(code_file!(s.as_bytes())),
                attachments: vec![],
                name: "wienerbröööööööö".to_owned(),
                version: "2019.3-5-PR2".to_owned(),
                metadata: HashMap::new(),
                inputs: vec![],
                outputs: vec![],
                created_at: 0,
            },
            null_logger!(),
        );
        let code = "asd";
        let args = vec![InputValue {
            name: "test-arg".to_owned(),
            r#type: Type::String as i32,
            value: b"test-value".to_vec(),
        }];
        let entry = "entry";
        let attachments = vec![attachment!("fake://")];
        let result = nested.execute(
            ExecutorParameters {
                function_name: "test".to_owned(),
                entrypoint: entry.to_owned(),
                code: Some(code_file!(code.as_bytes())),
                arguments: runtime_args.clone(),
            },
            args,
            attachments,
        );
        // Test that code got passed
        assert!(result.is_ok());
        assert_eq!(s.as_bytes(), fake.downloaded_code.borrow().as_slice());

        // Test that the argument we send in is passed through
        {
            let arguments = fake.arguments.borrow();

            assert_eq!(arguments.len(), 6);
            let code_attachment = Attachment::decode(
                arguments
                    .iter()
                    .find(|a| a.name == "_code")
                    .unwrap()
                    .value
                    .as_slice(),
            )
            .unwrap();
            assert_eq!(code_attachment.download().unwrap(), code.as_bytes());
            assert_eq!(
                arguments
                    .iter()
                    .find(|a| a.name == "_entrypoint")
                    .unwrap()
                    .value,
                entry.as_bytes()
            );
            assert_eq!(
                arguments
                    .iter()
                    .find(|a| a.name == "_sha256")
                    .unwrap()
                    .value,
                "688787d8ff144c502c7f5cffaafe2cc588d86079f9de88304c26b0cb99ce91c6".as_bytes()
            );

            let inner_arguments: InputValues = InputValues::decode(
                arguments
                    .iter()
                    .find(|a| a.name == "_arguments")
                    .unwrap()
                    .value
                    .as_slice(),
            )
            .unwrap();
            assert_eq!(
                inner_arguments
                    .values
                    .iter()
                    .find(|a| a.name == "test-arg")
                    .unwrap()
                    .value,
                b"test-value"
            );

            // Test that we get the execution environment args we supplied earlier
            let fake_exe_args = &fake.executor_context.borrow().arguments.clone();
            assert_eq!(fake_exe_args.len(), 0);
            assert_eq!(
                arguments.iter().find(|v| v.name == "sune").unwrap().value,
                b"bune"
            );
        }

        // Test president! 🕴
        {
            let args = vec![InputValue {
                name: "_code".to_owned(), // deliberately use a reserved word 👿
                r#type: Type::String as i32,
                value: b"this-is-not-code".to_vec(),
            }];

            let mut runtime_args = HashMap::new();
            runtime_args.insert("the-arg".to_owned(), "this-is-exec-arg".to_owned());
            let result = nested.execute(
                ExecutorParameters {
                    function_name: "test".to_owned(),
                    entrypoint: "entry".to_owned(),
                    code: Some(code_file!(b"code")),
                    arguments: runtime_args,
                },
                args,
                vec![],
            );

            assert!(result.is_ok());
            let args = &fake.arguments.borrow();
            assert!(args.iter().any(|a| a.name == "_code"));
            assert!(args.iter().any(|a| a.name == "_sha256"));
            assert!(args.iter().any(|a| a.name == "_entrypoint"));

            // make sure that we get the actual code and not the argument named _code
            let code_attachment = Attachment::decode(
                args.iter()
                    .find(|a| a.name == "_code")
                    .unwrap()
                    .value
                    .as_slice(),
            )
            .unwrap();
            assert_eq!(
                String::from_utf8(code_attachment.download().unwrap()).unwrap(),
                "code"
            );
        }

        // Double nested 🎰
        let nested2 = FunctionAdapter::new(
            Box::new(nested),
            Function {
                runtime: Some(Runtime {
                    name: "Avlivningsmiljö 🗡️".to_owned(),
                    entrypoint: "ingångspoäng 💯".to_owned(),
                    arguments: HashMap::new(),
                }),
                code: Some(code_file!(s.as_bytes())),
                attachments: vec![],
                name: "mandelkubb".to_owned(),
                version: "2022.1-5-PR50".to_owned(),
                metadata: HashMap::new(),
                inputs: vec![],
                outputs: vec![],
                created_at: 0u64,
            },
            null_logger!(),
        );
        {
            let args = vec![InputValue {
                name: "test-arg".to_owned(),
                r#type: Type::String as i32,
                value: b"test-value".to_vec(),
            }];
            let result = nested2.execute(
                ExecutorParameters {
                    function_name: "test".to_owned(),
                    entrypoint: "entry".to_owned(),
                    code: Some(code_file!(b"vec")),
                    arguments: runtime_args,
                },
                args,
                vec![],
            );

            assert!(result.is_ok());
            let args = &fake.arguments.borrow();
            assert!(args.iter().any(|a| a.name == "_code"));
            assert!(args.iter().any(|a| a.name == "_sha256"));
            assert!(args.iter().any(|a| a.name == "_entrypoint"));
        }
    }

    #[test]
    fn test_lookup_executor() {
        // get wasi executor
        let fr = registry!();
        let execution_service = ExecutionService::new(null_logger!(), Box::new(fr.clone()));
        let res =
            futures::executor::block_on(execution_service.lookup_executor_for_runtime("wasi"));
        assert!(res.is_ok());

        // get non existing executor
        let res =
            futures::executor::block_on(execution_service.lookup_executor_for_runtime("ur-sula!"));
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            ExecutorError::ExecutorNotFound(..)
        ));

        // get function executor
        let mut wasi_executor_metadata = HashMap::new();
        wasi_executor_metadata.insert("executor-for".to_owned(), "oran-malifant".to_owned());

        let mut nested_executor_metadata = HashMap::new();
        nested_executor_metadata.insert("executor-for".to_owned(), "precious-granag".to_owned());

        let mut broken_executor_metadata = HashMap::new();
        broken_executor_metadata.insert(
            "executor-for".to_owned(),
            "broken-chain-executor".to_owned(),
        );

        vec![
            FunctionData {
                runtime: Some(Runtime {
                    name: "wasi".to_owned(),
                    entrypoint: "wasi.kexe".to_owned(),
                    arguments: HashMap::new(),
                }),
                code_attachment_id: None,
                name: "oran-func".to_owned(),
                version: "0.1.1".to_owned(),
                metadata: wasi_executor_metadata,
                inputs: vec![],
                outputs: vec![],
                attachment_ids: vec![],
            },
            FunctionData {
                runtime: Some(Runtime {
                    name: "oran-malifant".to_owned(),
                    entrypoint: "oran hehurr".to_owned(),
                    arguments: HashMap::new(),
                }),
                code_attachment_id: None,
                name: "precious-granag".to_owned(),
                version: "8.1.5".to_owned(),
                metadata: nested_executor_metadata,
                inputs: vec![],
                outputs: vec![],
                attachment_ids: vec![],
            },
            FunctionData {
                runtime: Some(Runtime {
                    name: "oran-elefant".to_owned(),
                    entrypoint: "oran hehurr".to_owned(),
                    arguments: HashMap::new(),
                }),
                code_attachment_id: None,
                name: "precious-granag".to_owned(),
                version: "3.2.2".to_owned(),
                metadata: broken_executor_metadata,
                inputs: vec![],
                outputs: vec![],
                attachment_ids: vec![],
            },
        ]
        .into_iter()
        .for_each(|rr| {
            futures::executor::block_on(fr.register(tonic::Request::new(rr)))
                .map_or_else(|e| panic!(e.to_string()), |_| ())
        });

        let res = futures::executor::block_on(
            execution_service.lookup_executor_for_runtime("oran-malifant"),
        );
        assert!(res.is_ok());

        // Get two smetadatae executor
        let res = futures::executor::block_on(
            execution_service.lookup_executor_for_runtime("precious-granag"),
        );
        assert!(res.is_ok());

        // get function executor missing link
        let res = futures::executor::block_on(
            execution_service.lookup_executor_for_runtime("broken-chain-executor"),
        );
        assert!(res.is_err());

        assert!(matches!(
            res.unwrap_err(),
            ExecutorError::ExecutorNotFound(..)
        ));
    }

    #[test]
    fn test_cyclic_dependency_check() {
        let fr = registry!();
        let execution_service = ExecutionService::new(null_logger!(), Box::new(fr.clone()));
        let res =
            futures::executor::block_on(execution_service.lookup_executor_for_runtime("wasi"));
        assert!(res.is_ok());

        // get non existing executor
        let res =
            futures::executor::block_on(execution_service.lookup_executor_for_runtime("ur-sula!"));
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            ExecutorError::ExecutorNotFound(..)
        ));

        // get function executor
        let mut wasi_executor_metadata = HashMap::new();
        wasi_executor_metadata.insert("executor-for".to_owned(), "aa-exec".to_owned());

        let mut nested_executor_metadata = HashMap::new();
        nested_executor_metadata.insert("executor-for".to_owned(), "bb-exec".to_owned());

        let mut broken_executor_metadata = HashMap::new();
        broken_executor_metadata.insert("executor-for".to_owned(), "cc-exec".to_owned());

        vec![
            function_data!(
                "aa-func",
                "0.1.1",
                Runtime {
                    name: "bb-exec".to_owned(),
                    entrypoint: "wasi.kexe".to_owned(),
                    arguments: HashMap::new(),
                },
                { "executor-for" => "aa-exec" }
            ),
            function_data!(
                "bb-func",
                "0.1.5",
                Runtime {
                    name: "cc-exec".to_owned(),
                    entrypoint: "wasi.kexe".to_owned(),
                    arguments: HashMap::new(),
                },
                { "executor-for" => "bb-exec" }
            ),
            function_data!(
                "cc-func",
                "3.2.2",
                Runtime {
                    name: "bb-exec".to_owned(),
                    entrypoint: "wasi.kexe".to_owned(),
                    arguments: HashMap::new(),
                },
                { "executor-for" => "cc-exec" }
            ),
        ]
        .into_iter()
        .for_each(|rr| {
            futures::executor::block_on(fr.register(tonic::Request::new(rr)))
                .map_or_else(|e| panic!(e.to_string()), |_| ())
        });

        let res =
            futures::executor::block_on(execution_service.lookup_executor_for_runtime("aa-exec"));
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            ExecutorError::ExecutorDependencyCycle(..)
        ));
    }
}
