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

pub fn lookup_executor(name: &str) -> Result<Box<dyn FunctionExecutor>, ExecutorError> {
    match name {
        "maya" => Ok(Box::new(MayaExecutor {})),
        "wasm" => Ok(Box::new(WasmExecutor {})),
        ee => Err(ExecutorError::ExecutorNotFound(ee.to_owned()).into()),
    }
}

pub fn validate_args<'a, I>(inputs: I, args: &str) -> Result<(), ExecutorError>
where
    I: IntoIterator<Item = &'a FunctionInput>,
{
    let parsed_args: Value = serde_json::from_str(args)
        .map_err(|e| ExecutorError::InvalidArgumentFormat(e.to_string()))?;
    // TODO: This will abort as soon as it finds a bad arg
    // Would be nice to make it show all errors instead of first.

    // TODO: TEST FOR REQUIRED AND DEFAULT. Everything is required right now.
    inputs
        .into_iter()
        .map(|input| {
            parsed_args
                .get(&input.name)
                .ok_or(ExecutorError::RequiredArgumentMissing(input.name.clone()))
                .and_then(|parsed_arg| {
                    let tp = input.r#type;
                    ArgumentType::from_i32(tp)
                        .and_then(|at| {
                            Some(match at {
                                ArgumentType::String => (parsed_arg.is_string(), "string"),
                                ArgumentType::Bool => (parsed_arg.is_boolean(), "bool"),
                                ArgumentType::Int => (parsed_arg.is_i64(), "int"),
                                ArgumentType::Float => (parsed_arg.is_f64(), "float"),
                            })
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
                })
        })
        .collect()
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
