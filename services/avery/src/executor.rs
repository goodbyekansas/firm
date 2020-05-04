mod wasm;

use std::{
    collections::{HashMap, HashSet},
    fmt::{self, Debug, Display},
    fs, str,
};

use prost::Message;
use semver::VersionReq;
use slog::{o, Logger};
use thiserror::Error;
use url::Url;

use crate::executor::wasm::WasmExecutor;
use gbk_protocols::{
    functions::{
        execute_response::Result as ProtoResult, functions_registry_server::FunctionsRegistry,
        ArgumentType, Checksums, FunctionArgument, FunctionArguments, FunctionDescriptor,
        FunctionInput, FunctionOutput, FunctionResult, ListRequest, OrderingDirection, OrderingKey,
        VersionRequirement,
    },
    tonic,
};

pub trait FunctionExecutor: Debug {
    fn execute(
        &self,
        function_name: &str,
        entrypoint: &str,
        code: &[u8],
        checksums: &Checksums,
        executor_arguments: &[FunctionArgument],
        function_arguments: &[FunctionArgument],
    ) -> Result<ProtoResult, ExecutorError>;
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
        _function_name: &str,
        entrypoint: &str,
        code: &[u8],
        checksums: &Checksums, // TODO: Use checksum to validate code
        executor_arguments: &[FunctionArgument],
        function_arguments: &[FunctionArgument],
    ) -> Result<ProtoResult, ExecutorError> {
        let mut inner_function_arguments = vec![];

        // inject executor arguments to function
        inner_function_arguments.push(FunctionArgument {
            name: "code".to_owned(),
            r#type: ArgumentType::Bytes as i32,
            value: code.to_vec(),
        });

        inner_function_arguments.push(FunctionArgument {
            name: "entrypoint".to_owned(),
            r#type: ArgumentType::String as i32,
            value: entrypoint.as_bytes().to_vec(),
        });

        inner_function_arguments.push(FunctionArgument {
            name: "sha256".to_owned(),
            r#type: ArgumentType::String as i32,
            value: checksums.sha256.as_bytes().to_vec(),
        });

        let mut manifest_executor_arguments = executor_arguments.to_vec();
        inner_function_arguments.append(&mut manifest_executor_arguments);

        let proto_function_arguments = FunctionArguments {
            arguments: function_arguments.to_vec(),
        };

        let mut encoded_function_arguments =
            Vec::with_capacity(proto_function_arguments.encoded_len());
        proto_function_arguments.encode(&mut encoded_function_arguments)?;

        inner_function_arguments.push(FunctionArgument {
            name: "args".to_owned(),
            r#type: ArgumentType::Bytes as i32,
            value: encoded_function_arguments,
        });

        let inner_function_name = self
            .function_descriptor
            .function
            .as_ref()
            .ok_or(ExecutorError::FunctionDescriptorMissingFunction)?
            .name
            .clone();
        let inner_code = download_code(&self.function_descriptor.code_url)?;
        let inner_checksums = self
            .function_descriptor
            .checksums
            .as_ref()
            .ok_or(ExecutorError::MissingChecksums)?;
        let inner_exe_env = self
            .function_descriptor
            .execution_environment
            .clone()
            .ok_or_else(|| {
                ExecutorError::MissingExecutionEnvironment(inner_function_name.clone())
            })?;

        self.executor.execute(
            &inner_function_name,
            &inner_exe_env.entrypoint,
            &inner_code,
            &inner_checksums,
            &inner_exe_env.args,
            &inner_function_arguments,
        )
    }
}

async fn get_function_with_execution_environment(
    registry: &dyn FunctionsRegistry,
    exec_env: &str,
    version_requirement: Option<VersionReq>,
) -> Option<FunctionDescriptor> {
    let mut execution_env_tags = HashMap::new();
    execution_env_tags.insert("type".to_owned(), "execution-environment".to_owned());
    execution_env_tags.insert("execution-environment".to_owned(), exec_env.to_owned());

    let result = registry
        .list(tonic::Request::new(ListRequest {
            name_filter: "".to_owned(),
            tags_filter: execution_env_tags,
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
            "wasm" => break,
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
    // was "wasm". This may not be true later
    let executor = Box::new(WasmExecutor::new(
        function_descriptors
            .last()
            .map(|(_fd, logger)| logger)
            .unwrap_or(&logger)
            .new(o!("executor" => "wasm")),
    ));

    Ok(function_descriptors
        .into_iter()
        .fold(executor, |prev_executor, (fd, fd_logger)| {
            Box::new(FunctionAdapter::new(prev_executor, fd, fd_logger))
        }))
}

/// Download function code from the given URL
///
/// TODO: This is a huge security hole ⛳️ and needs to be managed properly (gpg sign 🔏 things?)
pub fn download_code(url: &str) -> Result<Vec<u8>, ExecutorError> {
    let url = Url::parse(url).map_err(|e| ExecutorError::InvalidCodeUrl(e.to_string()))?;
    match url.scheme() {
        "file" => fs::read(url.path())
            .map_err(|e| ExecutorError::CodeReadError(url.to_string(), e.to_string())),

        s => Err(ExecutorError::UnsupportedTransport(s.to_owned())),
    }
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
    CodeReadError(String, String),

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
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::{collections::HashMap, io::Write};

    use tempfile::NamedTempFile;

    use crate::registry::FunctionsRegistryService;
    use gbk_protocols::functions::{
        ExecutionEnvironment, Function, FunctionId, RegisterRequest, ReturnValue,
    };

    macro_rules! null_logger {
        () => {{
            slog::Logger::root(slog::Discard, o!())
        }};
    }

    macro_rules! registry {
        () => {{
            FunctionsRegistryService::new()
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
            value: "yes".as_bytes().to_vec(),
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
        let r = download_code("file://this-file-does-not-exist");
        assert!(r.is_err());
        assert!(matches!(r.unwrap_err(), ExecutorError::CodeReadError(..)));

        // invalid url
        let r = download_code("this-is-not-url");
        assert!(r.is_err());
        assert!(matches!(r.unwrap_err(), ExecutorError::InvalidCodeUrl(..)));

        // unsupported scheme
        let r = download_code("unsupported://that-scheme.fabrikam.com");
        assert!(r.is_err());
        assert!(matches!(
            r.unwrap_err(),
            ExecutorError::UnsupportedTransport(..)
        ));

        // actual file
        let mut tf = NamedTempFile::new().unwrap();
        let s = "some data 🖥️";
        write!(tf, "{}", s).unwrap();
        let r = download_code(&format!("file://{}", tf.path().display()));
        assert!(r.is_ok());
        assert_eq!(s.as_bytes(), r.unwrap().as_slice());
    }

    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Default, Debug)]
    pub struct FakeExecutor {
        function_name: RefCell<String>,
        entrypoint: RefCell<String>,
        code: RefCell<Vec<u8>>,
        checksums: RefCell<Checksums>,
        executor_arguments: RefCell<Vec<FunctionArgument>>,
        function_arguments: RefCell<Vec<FunctionArgument>>,
    }

    impl FunctionExecutor for Rc<FakeExecutor> {
        fn execute(
            &self,
            function_name: &str,
            entrypoint: &str,
            code: &[u8],
            checksums: &Checksums,
            executor_arguments: &[FunctionArgument],
            function_arguments: &[FunctionArgument],
        ) -> Result<ProtoResult, ExecutorError> {
            *self.function_name.borrow_mut() = function_name.to_owned();
            *self.entrypoint.borrow_mut() = entrypoint.to_owned();
            *self.code.borrow_mut() = code.to_vec();
            *self.checksums.borrow_mut() = checksums.clone();
            *self.executor_arguments.borrow_mut() = executor_arguments.to_vec();
            *self.function_arguments.borrow_mut() = function_arguments.to_vec();
            Ok(ProtoResult::Ok(FunctionResult { values: Vec::new() }))
        }
    }

    /*#[test]
    // TODO:
    fn test_bad_checksum_for_code() {
    }*/

    #[test]
    fn test_nested_executor() {
        let mut tf = NamedTempFile::new().unwrap();
        let s = "some data 🖥️";

        let exe_env_args = [FunctionArgument {
            name: "sune".to_owned(),
            r#type: ArgumentType::String as i32,
            value: "bune".as_bytes().to_vec(),
        }];

        write!(tf, "{}", s).unwrap();
        let checksums = Checksums {
            sha256: "724a8940e46ffa34e930258f708d890dbb3b3243361dfbc41eefcff124407a29".to_owned(),
        };
        let fake = Rc::new(FakeExecutor::default());
        let nested = FunctionAdapter::new(
            Box::new(fake.clone()),
            FunctionDescriptor {
                execution_environment: Some(ExecutionEnvironment {
                    name: "Avlivningsmiljö 🗡️".to_owned(),
                    entrypoint: "ingångspoäng 💯".to_owned(),
                    args: vec![],
                }),
                checksums: Some(checksums.clone()),
                code_url: format!("file://{}", tf.path().display()),
                function: Some(Function {
                    id: Some(FunctionId {
                        value: "huuuuuus".to_owned(),
                    }),
                    name: "wienerbröööööööö".to_owned(),
                    version: "2019.3-5-PR2".to_owned(),
                    tags: HashMap::new(),
                    inputs: vec![],
                    outputs: vec![],
                }),
            },
            null_logger!(),
        );
        let args = [FunctionArgument {
            name: "test-arg".to_owned(),
            r#type: ArgumentType::String as i32,
            value: "test-value".as_bytes().to_vec(),
        }];
        let code = "asd️".as_bytes();
        let entry = "entry";
        let result = nested.execute(
            "test",
            entry.clone(),
            code.clone(),
            &checksums,
            &exe_env_args,
            &args,
        );
        // Test that code got passed
        assert!(result.is_ok());
        assert_eq!(s.as_bytes(), fake.code.clone().into_inner().as_slice());

        // Test that the argument we send in is passed through
        let fake_args = fake.function_arguments.clone().into_inner();
        assert_eq!(fake_args.len(), 5);
        assert_eq!(
            fake_args.iter().find(|v| v.name == "code").unwrap().value,
            code
        );
        assert_eq!(
            fake_args
                .iter()
                .find(|v| v.name == "entrypoint")
                .unwrap()
                .value,
            entry.as_bytes()
        );
        assert_eq!(
            fake_args.iter().find(|v| v.name == "sha256").unwrap().value,
            checksums.sha256.as_bytes()
        );
        assert!(fake_args.iter().find(|v| v.name == "args").is_some());
        assert!(fake_args.iter().find(|v| v.name == "test-arg").is_none());

        // Test that we get the execution environment args we supplied earlier
        let fake_exe_args = fake.executor_arguments.clone().into_inner();
        assert_eq!(fake_exe_args.len(), 0);
        assert_eq!(
            fake_args.iter().find(|v| v.name == "sune").unwrap().value,
            "bune".as_bytes()
        );

        let nested2 = FunctionAdapter::new(
            Box::new(nested),
            FunctionDescriptor {
                execution_environment: Some(ExecutionEnvironment {
                    name: "Avlivningsmiljö 🗡️".to_owned(),
                    entrypoint: "ingångspoäng 💯".to_owned(),
                    args: vec![],
                }),
                checksums: Some(checksums.clone()),
                code_url: format!("file://{}", tf.path().display()),
                function: Some(Function {
                    id: Some(FunctionId {
                        value: "pieceee of pie".to_owned(),
                    }),
                    name: "mandelkubb".to_owned(),
                    version: "2022.1-5-PR50".to_owned(),
                    tags: HashMap::new(),
                    inputs: vec![],
                    outputs: vec![],
                }),
            },
            null_logger!(),
        );
        let args = vec![FunctionArgument {
            name: "test-arg".to_owned(),
            r#type: ArgumentType::String as i32,
            value: "test-value".as_bytes().to_vec(),
        }];
        let result = nested2.execute(
            "test",
            "entry",
            "vec".as_bytes(),
            &checksums,
            &exe_env_args,
            &args,
        );
        assert!(result.is_ok());
        let fake_args = fake.function_arguments.clone().into_inner();
        assert_eq!(fake_args.iter().filter(|a| a.name == "code").count(), 1);
        assert_eq!(fake_args.iter().filter(|a| a.name == "sha256").count(), 1);
        assert_eq!(
            fake_args.iter().filter(|a| a.name == "entrypoint").count(),
            1
        );
        assert_eq!(fake_args.iter().filter(|a| a.name == "args").count(), 1);
    }

    #[test]
    fn test_lookup_executor() {
        // get wasm executor
        let fr = registry!();
        let res = futures::executor::block_on(lookup_executor(null_logger!(), "wasm", &fr));
        assert!(res.is_ok());

        // get non existing executor
        let res = futures::executor::block_on(lookup_executor(null_logger!(), "ur-sula!", &fr));
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            ExecutorError::ExecutorNotFound(..)
        ));

        // get function executor
        let mut wasm_executor_tags = HashMap::new();
        wasm_executor_tags.insert("type".to_owned(), "execution-environment".to_owned());
        wasm_executor_tags.insert(
            "execution-environment".to_owned(),
            "oran-malifant".to_owned(),
        );

        let mut nested_executor_tags = HashMap::new();
        nested_executor_tags.insert("type".to_owned(), "execution-environment".to_owned());
        nested_executor_tags.insert(
            "execution-environment".to_owned(),
            "precious-granag".to_owned(),
        );

        let mut broken_executor_tags = HashMap::new();
        broken_executor_tags.insert("type".to_owned(), "execution-environment".to_owned());
        broken_executor_tags.insert(
            "execution-environment".to_owned(),
            "broken-chain-executor".to_owned(),
        );

        let checksums = Some(Checksums {
            sha256: "724a8940e46ffa34e930258f708d890dbb3b3243361dfbc41eefcff124407a29".to_owned(),
        });

        vec![
            RegisterRequest {
                execution_environment: Some(ExecutionEnvironment {
                    name: "wasm".to_owned(),
                    entrypoint: "wasm.kexe".to_owned(),
                    args: vec![],
                }),
                checksums: checksums.clone(),
                code: vec![],
                name: "oran-func".to_owned(),
                version: "0.1.1".to_owned(),
                tags: wasm_executor_tags,
                inputs: vec![],
                outputs: vec![],
            },
            RegisterRequest {
                execution_environment: Some(ExecutionEnvironment {
                    name: "oran-malifant".to_owned(),
                    entrypoint: "oran hehurr".to_owned(),
                    args: vec![],
                }),
                checksums: checksums.clone(),
                code: vec![],
                name: "precious-granag".to_owned(),
                version: "8.1.5".to_owned(),
                tags: nested_executor_tags,
                inputs: vec![],
                outputs: vec![],
            },
            RegisterRequest {
                execution_environment: Some(ExecutionEnvironment {
                    name: "oran-elefant".to_owned(),
                    entrypoint: "oran hehurr".to_owned(),
                    args: vec![],
                }),
                checksums,
                code: vec![],
                name: "precious-granag".to_owned(),
                version: "3.2.2".to_owned(),
                tags: broken_executor_tags,
                inputs: vec![],
                outputs: vec![],
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

        // Get two stage executor
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
        let res = futures::executor::block_on(lookup_executor(null_logger!(), "wasm", &fr));
        assert!(res.is_ok());

        // get non existing executor
        let res = futures::executor::block_on(lookup_executor(null_logger!(), "ur-sula!", &fr));
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            ExecutorError::ExecutorNotFound(..)
        ));

        // get function executor
        let mut wasm_executor_tags = HashMap::new();
        wasm_executor_tags.insert("type".to_owned(), "execution-environment".to_owned());
        wasm_executor_tags.insert("execution-environment".to_owned(), "aa-exec".to_owned());

        let mut nested_executor_tags = HashMap::new();
        nested_executor_tags.insert("type".to_owned(), "execution-environment".to_owned());
        nested_executor_tags.insert("execution-environment".to_owned(), "bb-exec".to_owned());

        let mut broken_executor_tags = HashMap::new();
        broken_executor_tags.insert("type".to_owned(), "execution-environment".to_owned());
        broken_executor_tags.insert("execution-environment".to_owned(), "cc-exec".to_owned());

        let checksums = Some(Checksums {
            sha256: "724a8940e46ffa34e930258f708d890dbb3b3243361dfbc41eefcff124407a29".to_owned(),
        });

        vec![
            RegisterRequest {
                name: "aa-func".to_owned(),
                execution_environment: Some(ExecutionEnvironment {
                    name: "bb-exec".to_owned(),
                    entrypoint: "wasm.kexe".to_owned(),
                    args: vec![],
                }),
                checksums: checksums.clone(),
                code: vec![],
                version: "0.1.1".to_owned(),
                tags: wasm_executor_tags,
                inputs: vec![],
                outputs: vec![],
            },
            RegisterRequest {
                name: "bb-func".to_owned(),
                execution_environment: Some(ExecutionEnvironment {
                    name: "cc-exec".to_owned(),
                    entrypoint: "wasm.kexe".to_owned(),
                    args: vec![],
                }),
                checksums: checksums.clone(),
                code: vec![],
                version: "8.1.5".to_owned(),
                tags: nested_executor_tags,
                inputs: vec![],
                outputs: vec![],
            },
            RegisterRequest {
                name: "cc-func".to_owned(),
                execution_environment: Some(ExecutionEnvironment {
                    name: "aa-exec".to_owned(),
                    entrypoint: "wasm.kexe".to_owned(),
                    args: vec![],
                }),
                checksums,
                code: vec![],
                version: "3.2.2".to_owned(),
                tags: broken_executor_tags,
                inputs: vec![],
                outputs: vec![],
            },
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
