use serde_json::{self, Value};

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

pub fn lookup_executor(name: &str) -> Result<Box<dyn FunctionExecutor>, String> {
    match name {
        "maya" => Ok(Box::new(MayaExecutor {})),
        "wasm" => Ok(Box::new(WasmExecutor {})),
        ee => Err(format!("Failed to find execution environment: {}", ee)),
    }
}

fn invalid_arg_type(arg_type: i32) -> String {
    format!(
        "Invalid argument type \"{}\". (protobuf enum definition is most likely outdated)",
        arg_type
    )
}

fn valid_to_result(valid: bool, error_msg: String) -> Result<(), String> {
    if valid {
        Ok(())
    } else {
        Err(error_msg)
    }
}

pub fn validate_args<'a, I>(inputs: I, args: &str) -> Result<(), String>
where
    I: IntoIterator<Item = &'a FunctionInput>,
{
    let parsed_args: Value =
        serde_json::from_str(args).map_err(|e| format!("Failed to parse json args: {}", e))?;
    // TODO: This will abort as soon as it finds a bad arg
    // Would be nice to make it show all errors instead of first.

    // TODO: TEST FOR REQUIRED AND DEFAULT. Everything is required right now.
    inputs
        .into_iter()
        .map(|input| {
            parsed_args
                .get(&input.name)
                .ok_or(format!("Failed to find input \"{}\".", &input.name))
                .and_then(|parsed_arg| {
                    let tp = input.r#type;
                    ArgumentType::from_i32(tp)
                        .and_then(|at| {
                            Some(match at {
                                ArgumentType::String => parsed_arg.is_string(),
                                ArgumentType::Bool => parsed_arg.is_boolean(),
                                ArgumentType::Int => parsed_arg.is_i64(),
                                ArgumentType::Float => parsed_arg.is_f64(),
                            })
                        })
                        .map_or(Err(invalid_arg_type(tp)), |valid| {
                            valid_to_result(
                                valid,
                                format!(
                                    r#"Argument "{}" expected type "{}". Failed to parse "{}" to type."#,
                                    input.name, tp, parsed_arg
                                ),
                            )
                        })
                })
        })
        .collect()
}
