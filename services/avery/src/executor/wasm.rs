use crate::executor::FunctionExecutor;
use crate::proto::{execute_response::Result as ProtoResult, FunctionArgument};
use crate::FunctionExecutorEnvironmentDescriptor;

pub struct WasmExecutor {}

impl FunctionExecutor for WasmExecutor {
    fn execute(
        &self,
        _feed: &FunctionExecutorEnvironmentDescriptor,
        _arguments: &[FunctionArgument],
    ) -> ProtoResult {
        ProtoResult::Ok("hello, world!".to_owned())
    }
}
