use crate::executor::FunctionExecutor;
use crate::proto::{execute_response::Result as ProtoResult, FunctionArgument};

pub struct WasmExecutor {}

impl FunctionExecutor for WasmExecutor {
    fn execute(
        &self,
        _entrypoint: &str,
        _code: &[u8],
        _arguments: &[FunctionArgument],
    ) -> ProtoResult {
        ProtoResult::Ok("hello, world!".to_owned())
    }
}
