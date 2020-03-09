use serde_json::{self, Value};
use thiserror::Error;

use crate::proto::execute_response::Result as ProtoResult;
use crate::proto::{ArgumentType, FunctionInput};

pub trait FunctionExecutor {
    fn execute(&self, code: &[u8]) -> ProtoResult;
}

#[derive(Default, Debug)]
struct MayaExecutor {}

impl FunctionExecutor for MayaExecutor {
    fn execute(&self, _code: &[u8]) -> ProtoResult {
        ProtoResult::Ok("hello, world!".to_owned())
    }
}

#[derive(Default, Debug)]
struct WasmExecutor {}

impl FunctionExecutor for WasmExecutor {
    fn execute(&self, _code: &[u8]) -> ProtoResult {
        ProtoResult::Ok("".to_owned())
    }
}

/// Lookup an executor for the given `name`
///
/// If an executor is not supported, an error is returned
pub fn lookup_executor(name: &str) -> Result<Box<dyn FunctionExecutor>, ExecutorError> {
    match name {
        "maya" => Ok(Box::new(MayaExecutor {})),
        "wasm" => Ok(Box::new(WasmExecutor {})),
        ee => Err(ExecutorError::ExecutorNotFound(ee.to_owned())),
    }
}

/// Validate arguments in json format
///
/// `inputs` is the functions' description of the arguments and `args` is the passed in arguments
/// as a JSON formatted string. This function returns all validation errors as a
/// `Vec<ExecutionError>`.
pub fn validate_args<'a, I>(inputs: I, args: &str) -> Result<(), Vec<ExecutorError>>
where
    I: IntoIterator<Item = &'a FunctionInput>,
{
    let parsed_args: Value = serde_json::from_str(args)
        .map_err(|e| vec![ExecutorError::InvalidArgumentFormat(e.to_string())])?;

    let (_, errors): (Vec<_>, Vec<_>) = inputs
        .into_iter()
        .map(|input| {
            parsed_args.get(&input.name).map_or_else(
                // argument was not found in the sent in args
                || {
                    if input.required {
                        Err(ExecutorError::RequiredArgumentMissing(input.name.clone()))
                    } else {
                        Ok(())
                    }
                },
                // argument was found in the sent in args, validate it
                |parsed_arg| {
                    let tp = input.r#type;
                    ArgumentType::from_i32(tp)
                        .map(|at| match at {
                            ArgumentType::String => (parsed_arg.is_string(), "string"),
                            ArgumentType::Bool => (parsed_arg.is_boolean(), "bool"),
                            ArgumentType::Int => (parsed_arg.is_i64(), "int"),
                            ArgumentType::Float => (parsed_arg.is_f64(), "float"),
                        })
                        .map_or(
                            Err(ExecutorError::OutOfRangeArgumentType(tp)),
                            |(valid, type_name)| {
                                if valid {
                                    Ok(())
                                } else {
                                    Err(ExecutorError::MismatchedArgumentType {
                                        argument_name: input.name.clone(),
                                        expected: type_name.to_owned(),
                                        value: parsed_arg.to_string(),
                                    })
                                }
                            },
                        )
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

#[derive(Error, Debug)]
pub enum ExecutorError {
    #[error("Failed to find executor for execution environment \"{0}\"")]
    ExecutorNotFound(String),

    #[error("Failed to interpret passed arguments: {0}")]
    InvalidArgumentFormat(String),

    #[error("Out of range argument type found: {0}. Protobuf definitions out of date?")]
    OutOfRangeArgumentType(i32),

    #[error("Argument \"{argument_name}\" has unexpected type. Failed to parse \"{value}\" to {expected}")]
    MismatchedArgumentType {
        argument_name: String,
        expected: String,
        value: String,
    },

    #[error("Failed to find required argument {0}")]
    RequiredArgumentMissing(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_required() {
        let inputs = vec![FunctionInput {
            name: "very_important_argument".to_owned(),
            r#type: ArgumentType::String as i32,
            required: true,
            default_value: String::new(),
        }];

        let r = validate_args(inputs.iter(), "{}");
        assert!(r.is_err());

        let r = validate_args(
            inputs.iter(),
            r#"{
        "very_important_argument": "yes"
        }"#,
        );
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

        let r = validate_args(inputs.iter(), "{}");
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
        ];

        let r = validate_args(
            inputs.iter(),
            r#"
            {
                "string_arg": "yes",
                "bool_arg": true,
                "int_arg": 4,
                "float_arg": 4.5
            }"#,
        );

        assert!(r.is_ok());

        // one has the wrong type 🤯
        let r = validate_args(
            inputs.iter(),
            r#"
            {
                "string_arg": 5,
                "bool_arg": true,
                "int_arg": 4,
                "float_arg": 4.5
            }"#,
        );

        assert!(r.is_err());
        assert_eq!(1, r.unwrap_err().len());

        // all of them has the wrong type 🚓💨
        let r = validate_args(
            inputs.iter(),
            r#"
            {
                "string_arg": 5,
                "bool_arg": "ture",
                "int_arg": false,
                "float_arg": "a"
            }"#,
        );

        assert!(r.is_err());
        assert_eq!(4, r.unwrap_err().len());
    }
}
