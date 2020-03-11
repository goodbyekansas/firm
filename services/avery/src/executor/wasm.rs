use crate::executor::FunctionExecutor;
use crate::proto::{
    execute_response::Result as ProtoResult, ArgumentType, FunctionArgument, FunctionResult,
    ReturnValue,
};

pub struct WasmExecutor {}

impl FunctionExecutor for WasmExecutor {
    fn execute(
        &self,
        _entrypoint: &str,
        _code: &[u8],
        _arguments: &[FunctionArgument],
    ) -> ProtoResult {
        ProtoResult::Ok(FunctionResult {
            values: vec![ReturnValue {
                name: "output_string".to_owned(),
                r#type: ArgumentType::String as i32,
                value: b"hello world".to_vec(),
            }],
        })
    }
}
