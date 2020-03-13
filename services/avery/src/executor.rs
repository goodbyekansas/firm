mod wasm;

use std::{fmt, fs, str};

use thiserror::Error;
use url::Url;

use crate::executor::wasm::WasmExecutor;
use crate::proto::execute_response::Result as ProtoResult;
use crate::proto::{ArgumentType, FunctionArgument, FunctionInput, FunctionOutput, FunctionResult};

pub trait FunctionExecutor {
    fn execute(
        &self,
        function_name: &str,
        entrypoint: &str,
        code: &[u8],
        arguments: &[FunctionArgument],
    ) -> ProtoResult;
}

/// Lookup an executor for the given `name`
///
/// If an executor is not supported, an error is returned
pub fn lookup_executor(name: &str) -> Result<Box<dyn FunctionExecutor>, ExecutorError> {
    match name {
        "wasm" => Ok(Box::new(WasmExecutor {})),
        ee => Err(ExecutorError::ExecutorNotFound(ee.to_owned())),
    }
}

/// Donwload function code from the given URL
///
/// This is a huge security hole and needs to be managed properly (gpg sign things?)
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
    use crate::proto::ReturnValue;
    use std::io::Write;
    use tempfile::NamedTempFile;

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
}
