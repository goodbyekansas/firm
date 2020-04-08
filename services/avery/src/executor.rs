mod wasm;

use std::{
    fmt::{self, Debug},
    fs, str,
};

use slog::{o, Logger};
use thiserror::Error;
use url::Url;

use crate::executor::wasm::WasmExecutor;
use crate::proto::execute_response::Result as ProtoResult;
use crate::proto::{
    ArgumentType, FunctionArgument, FunctionDescriptor, FunctionInput, FunctionOutput,
    FunctionResult,
};

pub trait FunctionExecutor: Debug {
    fn execute(
        &self,
        function_name: &str,
        entrypoint: &str,
        code: &[u8],
        arguments: &[FunctionArgument],
    ) -> Result<ProtoResult, ExecutorError>;
}

#[derive(Debug)]
pub struct NestedExecutor {
    inner: Box<dyn FunctionExecutor>,
    code_url: String,
}

/// Adapter for functions to act as executors
impl NestedExecutor {
    pub fn new<S: AsRef<str>>(inner: Box<dyn FunctionExecutor>, code_url: S) -> Self {
        Self {
            inner,
            code_url: code_url.as_ref().to_owned(),
        }
    }
}

impl FunctionExecutor for NestedExecutor {
    fn execute(
        &self,
        function_name: &str,
        entrypoint: &str,
        code: &[u8],
        arguments: &[FunctionArgument],
    ) -> Result<ProtoResult, ExecutorError> {
        let mut nest_arguments = arguments.to_vec();

        // inject executor arguments to function
        nest_arguments.push(FunctionArgument {
            name: "code".to_owned(),
            r#type: ArgumentType::Bytes as i32,
            value: code.to_vec(),
        });

        nest_arguments.push(FunctionArgument {
            name: "entrypoint".to_owned(),
            r#type: ArgumentType::String as i32,
            value: entrypoint.as_bytes().to_vec(),
        });

        let inner_code = download_code(&self.code_url)?;

        self.inner
            .execute(function_name, entrypoint, &inner_code, &nest_arguments)
    }
}

/// Lookup an executor for the given `name`
///
/// If an executor is not supported, an error is returned
pub fn lookup_executor(
    logger: Logger,
    name: &str,
    available_executor_functions: &[FunctionDescriptor],
) -> Result<Box<dyn FunctionExecutor>, ExecutorError> {
    match name {
        "wasm" => Ok(Box::new(WasmExecutor::new(
            logger.new(o!("executor" => "wasm")),
        ))),
        ee => {
            let matching_functions: Vec<&FunctionDescriptor> = available_executor_functions
                .iter()
                .filter(|e| {
                    e.function.as_ref().map_or(false, |f| {
                        f.tags
                            .iter()
                            .any(|(k, v)| k.as_str() == "execution_environment" && v.as_str() == ee)
                    })
                })
                .collect();

            let function = matching_functions
                .first()
                .ok_or_else(|| ExecutorError::ExecutorNotFound(ee.to_owned()))?;

            let exe_env_name = function
                .execution_environment
                .clone()
                .ok_or_else(|| {
                    ExecutorError::MissingExecutionEnvironment(
                        function
                            .function
                            .as_ref()
                            .map(|f| f.name.clone())
                            .unwrap_or_else(|| "Unknown".to_owned()),
                    )
                })?
                .name;

            Ok(Box::new(NestedExecutor::new(
                lookup_executor(
                    logger.new(o!("executor" => exe_env_name.clone())),
                    &exe_env_name,
                    available_executor_functions,
                )?,
                function.code_url.clone(),
            )))
        }
    }
}

/// Download function code from the given URL
///
/// TODO: This is a huge security hole and needs to be managed properly (gpg sign things?)
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
                                expected: ArgumentType::from_i32(output.r#type)
                                    .map_or("invalid_type".to_owned(), |t| t.to_string()),
                                got: ArgumentType::from_i32(arg.r#type)
                                    .map_or("invalid_type".to_owned(), |t| t.to_string()),
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
                            expected: ArgumentType::from_i32(input.r#type)
                                .map_or("invalid_type".to_owned(), |t| t.to_string()),
                            got: ArgumentType::from_i32(arg.r#type)
                                .map_or("invalid_type".to_owned(), |t| t.to_string()),
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

impl fmt::Display for ArgumentType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                ArgumentType::String => "string",
                ArgumentType::Int => "int",
                ArgumentType::Bool => "bool",
                ArgumentType::Float => "float",
                ArgumentType::Bytes => "bytes",
            }
        )
    }
}

#[derive(Error, Debug)]
pub enum ExecutorError {
    #[error("Unsupported code transport mechanism: \"{0}\"")]
    UnsupportedTransport(String),

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{ExecutionEnvironment, Function, FunctionId, ReturnValue};
    use std::{collections::HashMap, io::Write};
    use tempfile::NamedTempFile;

    macro_rules! null_logger {
        () => {{
            slog::Logger::root(slog::Discard, o!())
        }};
    }

    #[test]
    fn parse_required() {
        let inputs = vec![FunctionInput {
            name: "very_important_argument".to_owned(),
            r#type: ArgumentType::String as i32,
            required: true,
            default_value: String::new(),
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
            },
            FunctionInput {
                name: "bool_arg".to_owned(),
                r#type: ArgumentType::Bool as i32,
                required: true,
                default_value: String::new(),
            },
            FunctionInput {
                name: "int_arg".to_owned(),
                r#type: ArgumentType::Int as i32,
                required: true,
                default_value: String::new(),
            },
            FunctionInput {
                name: "float_arg".to_owned(),
                r#type: ArgumentType::Float as i32,
                required: true,
                default_value: String::new(),
            },
            FunctionInput {
                name: "bytes_arg".to_owned(),
                r#type: ArgumentType::Bytes as i32,
                required: false,
                default_value: String::new(),
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
        arguments: RefCell<Vec<FunctionArgument>>,
    }

    impl FunctionExecutor for Rc<FakeExecutor> {
        fn execute(
            &self,
            function_name: &str,
            entrypoint: &str,
            code: &[u8],
            arguments: &[FunctionArgument],
        ) -> Result<ProtoResult, ExecutorError> {
            *self.function_name.borrow_mut() = function_name.to_owned();
            *self.entrypoint.borrow_mut() = entrypoint.to_owned();
            *self.code.borrow_mut() = code.to_vec();
            *self.arguments.borrow_mut() = arguments.to_vec();
            Ok(ProtoResult::Ok(FunctionResult { values: Vec::new() }))
        }
    }

    #[test]
    fn test_nested_executor() {
        let mut tf = NamedTempFile::new().unwrap();
        let s = "some data 🖥️";
        write!(tf, "{}", s).unwrap();
        let code_url = format!("file://{}", tf.path().display());
        let fake = Rc::new(FakeExecutor::default());
        let nested = NestedExecutor::new(Box::new(fake.clone()), code_url);
        let result = nested.execute("test", "entry", "vec".as_bytes(), &[]);
        assert!(result.is_ok());
        assert_eq!(s.as_bytes(), fake.code.clone().into_inner().as_slice());
    }

    #[test]
    fn test_lookup_executor() {
        // get wasm executor
        let res = lookup_executor(null_logger!(), "wasm", &[]);
        assert!(res.is_ok());

        // get non existing executor
        let res = lookup_executor(null_logger!(), "ur-sula!", &[]);
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            ExecutorError::ExecutorNotFound(..)
        ));

        // get function executor
        let mut wasm_executor_tags = HashMap::new();
        wasm_executor_tags.insert("type".to_owned(), "execution_environment".to_owned());
        wasm_executor_tags.insert(
            "execution_environment".to_owned(),
            "oran-malifant".to_owned(),
        );

        let mut nested_executor_tags = HashMap::new();
        nested_executor_tags.insert("type".to_owned(), "execution_environment".to_owned());
        nested_executor_tags.insert(
            "execution_environment".to_owned(),
            "precious-granag".to_owned(),
        );

        let mut broken_executor_tags = HashMap::new();
        broken_executor_tags.insert("type".to_owned(), "execution_environment".to_owned());
        broken_executor_tags.insert(
            "execution_environment".to_owned(),
            "broken-chain-executor".to_owned(),
        );

        let function_executors = vec![
            FunctionDescriptor {
                execution_environment: Some(ExecutionEnvironment {
                    name: "wasm".to_owned(),
                }),
                entrypoint: "wasm.kexe".to_owned(),
                code_url: "No real url".to_owned(),
                function: Some(Function {
                    id: Some(FunctionId {
                        value: "".to_owned(),
                    }),
                    name: "oran-func".to_owned(),
                    version: "malifant".to_owned(),
                    tags: wasm_executor_tags,
                    inputs: Vec::new(),
                    outputs: Vec::new(),
                }),
            },
            FunctionDescriptor {
                execution_environment: Some(ExecutionEnvironment {
                    name: "oran-malifant".to_owned(),
                }),
                entrypoint: "oran hehurr".to_owned(),
                code_url: "Nah".to_owned(),
                function: Some(Function {
                    id: Some(FunctionId {
                        value: "nah".to_owned(),
                    }),
                    name: "precious-granag".to_owned(),
                    version: "granag".to_owned(),
                    tags: nested_executor_tags,
                    inputs: Vec::new(),
                    outputs: Vec::new(),
                }),
            },
            FunctionDescriptor {
                execution_environment: Some(ExecutionEnvironment {
                    name: "oran-elefant".to_owned(),
                }),
                entrypoint: "oran hehurr".to_owned(),
                code_url: "Nah".to_owned(),
                function: Some(Function {
                    id: Some(FunctionId {
                        value: "nah".to_owned(),
                    }),
                    name: "precious-granag".to_owned(),
                    version: "granag".to_owned(),
                    tags: broken_executor_tags,
                    inputs: Vec::new(),
                    outputs: Vec::new(),
                }),
            },
        ];

        let res = lookup_executor(null_logger!(), "oran-malifant", &function_executors);
        assert!(res.is_ok());

        // Get two stage executor
        let res = lookup_executor(null_logger!(), "precious-granag", &function_executors);
        assert!(res.is_ok());

        // get function executor missing link
        let res = lookup_executor(null_logger!(), "broken-chain-executor", &function_executors);
        assert!(res.is_err());

        assert!(matches!(
            res.unwrap_err(),
            ExecutorError::ExecutorNotFound(..)
        ));
    }
}
