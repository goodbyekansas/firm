mod error;
mod function;
mod net;
mod process;
mod sandbox;

use std::{
    collections::HashMap,
    fs::OpenOptions,
    str,
    sync::{Arc, RwLock},
};

use slog::{info, o, Logger};

use wasmer_runtime::{compile, func, imports, Array, Ctx, Func, Item, WasmPtr};
use wasmer_wasi::{
    generate_import_object_from_state, get_wasi_version,
    state::{HostFile, WasiState},
};

use crate::executor::{ExecutorError, FunctionExecutor};
use crate::proto::{
    execute_response::Result as ProtoResult, Checksums, ExecutionError, FunctionArgument,
    FunctionResult, ReturnValue,
};
use error::{ToErrorCode, WasmError};
use sandbox::Sandbox;

fn execute_function(
    logger: Logger,
    function_name: &str,
    _entrypoint: &str,
    code: &[u8],
    arguments: &[FunctionArgument],
) -> Result<Vec<ReturnValue>, String> {
    const ENTRY: &str = "_start";
    let module = compile(code).map_err(|e| format!("failed to compile wasm: {}", e))?;

    let wasi_version = get_wasi_version(&module, true).unwrap_or(wasmer_wasi::WasiVersion::Latest);

    let sandbox = Arc::new(Sandbox::new());
    let sandbox2 = Arc::clone(&sandbox);

    info!(
        logger,
        "using sandbox directory: {}",
        sandbox.path().display()
    );

    // create stdout and stderr
    let stdout = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(sandbox.path().join("stdout"))
        .map_err(|e| format!("failed to open stdout file: {}", e))?;

    let stderr = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(sandbox.path().join("stderr"))
        .map_err(|e| format!("failed to open stderr file: {}", e))?;

    let wasi_state = WasiState::new(&format!("wasi-{}", function_name))
        .stdout(Box::new(HostFile::new(
            stdout,
            sandbox.path().join("stdout"),
            true,
            true,
            true,
        )))
        .stderr(Box::new(HostFile::new(
            stderr,
            sandbox.path().join("stderr"),
            true,
            true,
            true,
        )))
        .preopen(|p| {
            p.directory(sandbox.path())
                .alias("sandbox")
                .read(true)
                .write(true)
                .create(true)
        })
        .and_then(|state| state.build())
        .map_err(|e| format!("Failed to create wasi state: {:?}", e))?;

    // inject gbk specific functions in the wasm state
    let a = arguments.to_vec();
    let a2 = arguments.to_vec();
    let v: Vec<Result<ReturnValue, String>> = Vec::new();
    let results = Arc::new(RwLock::new(v));
    let res = Arc::clone(&results);
    let res2 = Arc::clone(&results);

    let start_process_logger = logger.new(o!("scope" => "start_process"));
    let run_process_logger = logger.new(o!("scope" => "run_process"));
    let gbk_imports = imports! {
        "gbk" => {
            "start_host_process" => func!(move |ctx: &mut Ctx, s: WasmPtr<u8, Array>, len: u32, pid_out: WasmPtr<u64, Item>| {
                process::start_process(&start_process_logger, &sandbox, ctx.memory(0), s, len, pid_out).to_error_code()
            }),

            "run_host_process" => func!(move |ctx: &mut Ctx, s: WasmPtr<u8, Array>, len: u32, exit_code_out: WasmPtr<i32, Item>| {
                process::run_process(&run_process_logger, &sandbox2, ctx.memory(0), s, len, exit_code_out).to_error_code()
            }),

            "get_input_len" => func!(move |ctx: &mut Ctx, key: WasmPtr<u8, Array>, keylen: u32, value: WasmPtr<u64, Item>| {
                function::get_input_len(ctx.memory(0), key, keylen, value, &a).to_error_code()
            }),

            "get_input" => func!(move |ctx: &mut Ctx, key: WasmPtr<u8, Array>, keylen: u32, value: WasmPtr<u8, Array>, valuelen: u32| {
                function::get_input(ctx.memory(0), key, keylen, value, valuelen, &a2).to_error_code()
            }),

            "set_output" => func!(move |ctx: &mut Ctx, val: WasmPtr<u8, Array>, vallen: u32| {
                function::set_output(ctx.memory(0), val, vallen).and_then(|v| {
                    res.write().map(|mut writer| {
                        writer.push(Ok(v));
                    }).map_err(|e| {WasmError::Unknown(format!("{}", e))})
                }).to_error_code()
            }),

            "set_error" => func!(move |ctx: &mut Ctx, msg: WasmPtr<u8, Array>, msglen: u32| {
                function::set_error(ctx.memory(0), msg, msglen).and_then(|v| {
                    res2.write().map(|mut writer| {
                        writer.push(Err(v));
                    }).map_err(|e| {WasmError::Unknown(format!("{}", e))})
                }).to_error_code()
            }),

            "connect" => func!(move |ctx: &mut Ctx, addr: WasmPtr<u8, Array>, addr_len: u32, fd_out: WasmPtr<u32, Item>| {
                let mem = ctx.memory(0).clone();
                let state = unsafe { wasmer_wasi::state::get_wasi_state(ctx) };
                net::connect(&mut state.fs, &mem, addr, addr_len, fd_out).to_error_code()
            }),
        },
    };

    let mut import_object = generate_import_object_from_state(wasi_state, wasi_version);
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
        .and_then(|_| {
            results
                .read()
                .map_err(|e| format!("Failed to read function results: {}", e))
        })
        .and_then(|reader| reader.iter().cloned().collect())
}

#[derive(Debug)]
pub struct WasmExecutor {
    logger: Logger,
}

impl WasmExecutor {
    pub fn new(logger: Logger) -> Self {
        Self { logger }
    }
}

impl FunctionExecutor for WasmExecutor {
    fn execute(
        &self,
        function_name: &str,
        entrypoint: &str,
        code: &[u8],
        _checksums: &Checksums, // TODO: Use checksum
        _executor_arguments: &HashMap<String, String>,
        function_arguments: &[FunctionArgument],
    ) -> Result<ProtoResult, ExecutorError> {
        // TODO: separate host and guest errors
        Ok(execute_function(
            self.logger.new(o!("function" => function_name.to_owned())),
            function_name,
            entrypoint,
            code,
            function_arguments,
        )
        .map_or_else(
            |e| ProtoResult::Error(ExecutionError { msg: e }),
            |v| ProtoResult::Ok(FunctionResult { values: v }),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! null_logger {
        () => {{
            slog::Logger::root(slog::Discard, o!())
        }};
    }

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
        let executor = WasmExecutor::new(null_logger!());
        let res = executor.execute(
            "hello-world",
            "could-be-anything",
            include_bytes!("hello.wasm"),
            &Checksums {
                sha256: "c455c4bc68c1afcdafa7c2f74a499810b0aa5d12f7a009d493789d595847af72"
                    .to_owned(),
            },
            &HashMap::new(),
            &vec![],
        );

        assert!(res.is_ok());
        assert!(res.unwrap().is_ok());
    }
}
