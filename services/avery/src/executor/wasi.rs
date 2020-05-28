mod error;
mod function;
mod net;
mod process;
mod sandbox;

use std::{
    fs::OpenOptions,
    path::Path,
    str,
    sync::{Arc, RwLock},
};

use slog::{info, o, Logger};

use wasmer_runtime::{compile, func, imports, Array, Ctx, Func, Item, WasmPtr};
use wasmer_wasi::{
    generate_import_object_from_state, get_wasi_version,
    state::{HostFile, WasiState},
};

use crate::executor::{
    AttachmentDownload, ExecutorContext, ExecutorError, FunctionContext, FunctionExecutor,
};
use error::{ToErrorCode, WasiError};
use gbk_protocols::functions::{
    execute_response::Result as ProtoResult, ExecutionError, FunctionArgument, FunctionAttachment,
    FunctionResult, ReturnValue,
};
use sandbox::Sandbox;

fn execute_function(
    logger: Logger,
    function_name: &str,
    _entrypoint: &str,
    code: &[u8],
    arguments: &[FunctionArgument],
    function_attachments: &[FunctionAttachment],
) -> Result<Vec<ReturnValue>, String> {
    const ENTRY: &str = "_start";
    let module = compile(code).map_err(|e| format!("failed to compile wasm: {}", e))?;

    let wasi_version = get_wasi_version(&module, true).unwrap_or(wasmer_wasi::WasiVersion::Latest);

    let sandbox = Sandbox::new(Path::new("sandbox")).map_err(|e| e.to_string())?;
    let attachment_sandbox = Sandbox::new(Path::new("attachments")).map_err(|e| e.to_string())?;

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
        .and_then(|state| {
            state.preopen(|p| {
                p.directory(attachment_sandbox.path())
                    .alias("attachments")
                    .read(true)
                    .write(false)
                    .create(false)
            })
        })
        .and_then(|state| state.build())
        .map_err(|e| format!("Failed to create wasi state: {:?}", e))?;

    let sandboxes = [sandbox, attachment_sandbox.clone()];
    let sandboxes2 = sandboxes.clone();

    // inject gbk specific functions in the wasm state
    let a = arguments.to_vec();
    let a2 = arguments.to_vec();
    let v: Vec<Result<ReturnValue, String>> = Vec::new();
    let results = Arc::new(RwLock::new(v));
    let res = Arc::clone(&results);
    let res2 = Arc::clone(&results);
    let function_attachments = function_attachments.to_vec();

    let start_process_logger = logger.new(o!("scope" => "start_process"));
    let run_process_logger = logger.new(o!("scope" => "run_process"));
    let gbk_imports = imports! {
        "gbk" => {
            "map_attachment" => func!(move |ctx: &mut Ctx, attachment_name: WasmPtr<u8, Array>, attachment_name_len: u32| {
                function::map_attachment(&function_attachments, &attachment_sandbox, ctx.memory(0), attachment_name, attachment_name_len).to_error_code()
            }),
            "start_host_process" => func!(move |ctx: &mut Ctx, s: WasmPtr<u8, Array>, len: u32, pid_out: WasmPtr<u64, Item>| {
                process::start_process(&start_process_logger, &sandboxes, ctx.memory(0), s, len, pid_out).to_error_code()
            }),

            "run_host_process" => func!(move |ctx: &mut Ctx, s: WasmPtr<u8, Array>, len: u32, exit_code_out: WasmPtr<i32, Item>| {
                process::run_process(&run_process_logger, &sandboxes2, ctx.memory(0), s, len, exit_code_out).to_error_code()
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
                    }).map_err(|e| {WasiError::Unknown(format!("{}", e))})
                }).to_error_code()
            }),

            "set_error" => func!(move |ctx: &mut Ctx, msg: WasmPtr<u8, Array>, msglen: u32| {
                function::set_error(ctx.memory(0), msg, msglen).and_then(|v| {
                    res2.write().map(|mut writer| {
                        writer.push(Err(v));
                    }).map_err(|e| {WasiError::Unknown(format!("{}", e))})
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
pub struct WasiExecutor {
    logger: Logger,
}

impl WasiExecutor {
    pub fn new(logger: Logger) -> Self {
        Self { logger }
    }
}

impl FunctionExecutor for WasiExecutor {
    fn execute(
        &self,
        executor_context: ExecutorContext,
        function_context: FunctionContext,
    ) -> Result<ProtoResult, ExecutorError> {
        let code = executor_context
            .code
            .ok_or_else(|| ExecutorError::MissingCode("wasi".to_owned()))?;
        let downloaded_code = code.download()?;

        // TODO: separate host and guest errors
        Ok(execute_function(
            self.logger
                .new(o!("function" => executor_context.function_name.to_owned())),
            &executor_context.function_name,
            &executor_context.entrypoint,
            &downloaded_code,
            &function_context.arguments,
            &function_context.attachments,
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
    use gbk_protocols::functions::Checksums;

    macro_rules! null_logger {
        () => {{
            slog::Logger::root(slog::Discard, o!())
        }};
    }

    trait ProtoResultExt {
        fn is_ok(&self) -> bool;
    }

    impl ProtoResultExt for ProtoResult {
        fn is_ok(&self) -> bool {
            match self {
                ProtoResult::Ok(_) => true,
                _ => false,
            }
        }
    }

    #[test]
    fn test_execution() {
        let executor = WasiExecutor::new(null_logger!());
        let res = executor.execute(
            ExecutorContext {
                function_name: "hello-world".to_owned(),
                entrypoint: "could-be-anything".to_owned(),
                code: include_bytes!("hello.wasm").to_vec(),
                checksums: Checksums {
                    sha256: "c455c4bc68c1afcdafa7c2f74a499810b0aa5d12f7a009d493789d595847af72"
                        .to_owned(),
                },
                arguments: vec![],
            },
            FunctionContext {
                arguments: vec![],
                attachments: vec![],
            },
        );

        assert!(res.is_ok());
        assert!(res.unwrap().is_ok());
    }
}
