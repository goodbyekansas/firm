use std::{process::Command, str};

use wasmer_runtime::{compile, func, imports, Array, Ctx, Func, WasmPtr};
use wasmer_wasi::{generate_import_object_from_state, get_wasi_version, state::WasiState};

use crate::executor::FunctionExecutor;
use crate::proto::{
    execute_response::Result as ProtoResult, ExecutionError, FunctionArgument, FunctionResult,
};

fn start_process(ctx: &mut Ctx, s: WasmPtr<u8, Array>, len: u32) -> i64 {
    let memory = ctx.memory(0);
    match s.get_utf8_string(memory, len) {
        Some("maya") => match Command::new("/usr/autodesk/maya2019/bin/maya").spawn() {
            Ok(_) => 1,
            Err(_) => 0,
        },
        _ => 0,
    }
}

fn execute_function(
    _entrypoint: &str,
    code: &[u8],
    _arguments: &[FunctionArgument],
) -> Result<(), String> {
    const ENTRY: &str = "_start";
    let module = compile(code).map_err(|e| format!("failed to compile wasm: {}", e))?;

    let wasi_version = get_wasi_version(&module, true).unwrap_or(wasmer_wasi::WasiVersion::Latest);

    let wasi_state = WasiState::new("some-wasi-state-name")
        .build()
        .map_err(|e| format!("Failed to create wasi state: {:?}", e))?;

    let mut import_object = generate_import_object_from_state(wasi_state, wasi_version);
    let gbk_imports = imports! {
        "gbk" => {
            "start_process" => func!(start_process),
        },
    };

    import_object.extend(gbk_imports);
    let instance = module
        .instantiate(&import_object)
        .map_err(|e| format!("failed to instantiate WASI module: {}", e))?;

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

    /*#[test]
    fn test_sune() {
        let executor = WasmExecutor {};
        let res = executor.execute(
            "could-be-anything",
            include_bytes!(
                "../../../../functions/start-maya/target/wasm32-wasi/debug/start-maya.wasm"
            ),
            &vec![],
        );

        assert!(dbg!(res).is_ok());
    }*/
}
