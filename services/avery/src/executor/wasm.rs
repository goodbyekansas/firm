use wasmer_runtime::{instantiate, Func};
use wasmer_wasi::{generate_import_object_from_state, state::WasiState};

use crate::executor::FunctionExecutor;
use crate::proto::{
    execute_response::Result as ProtoResult, ExecutionError, FunctionArgument, FunctionResult,
};

fn execute_function(
    _entrypoint: &str,
    code: &[u8],
    _arguments: &[FunctionArgument],
) -> Result<(), String> {
    const ENTRY: &str = "_start";
    let wasi_state = WasiState::new("some-wasi-state-name")
        .build()
        .map_err(|e| format!("Failed to create wasi state: {:?}", e))?;

    let import_object =
        generate_import_object_from_state(wasi_state, wasmer_wasi::WasiVersion::Snapshot0);

    let instance = instantiate(code, &import_object)
        .map_err(|e| format!("Failed to instantiate function: {}", e))?;
    let entry_function: Func<(), ()> = instance
        .func(ENTRY)
        .map_err(|e| format!("Failed to resolve entrypoint {}: {}", ENTRY, e))?;

    entry_function
        .call()
        .map_err(|e| format!("Failed to call entrypoint function {}: {}", ENTRY, e))
}

pub struct WasmExecutor {}

impl FunctionExecutor for WasmExecutor {
    fn execute(
        &self,
        entrypoint: &str,
        code: &[u8],
        arguments: &[FunctionArgument],
    ) -> ProtoResult {
        execute_function(entrypoint, code, arguments).map_or_else(
            |e| ProtoResult::Error(ExecutionError { msg: e }),
            |_| ProtoResult::Ok(FunctionResult { values: vec![] }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl ProtoResult {
        fn is_ok(&self) -> bool {
            match self {
                ProtoResult::Ok(_) => true,
                _ => false,
            }
        }
    }

    #[test]
    fn test_execution() {
        let executor = WasmExecutor {};
        let res = executor.execute("could-be-anything", include_bytes!("hello.wasm"), &vec![]);

        assert!(res.is_ok());
    }
}
