mod sandbox;

use std::{
    cell::Cell,
    collections::HashMap,
    fs::OpenOptions,
    io,
    process::Command,
    str,
    sync::{Arc, RwLock},
};

use prost::Message;
use regex::Regex;
use slog::{info, o, Logger};
use thiserror::Error;
use wasmer_runtime::{compile, func, imports, memory::Memory, Array, Ctx, Func, Item, WasmPtr};
use wasmer_wasi::{
    generate_import_object_from_state, get_wasi_version,
    state::{HostFile, WasiState},
};

use crate::executor::FunctionExecutor;
use crate::proto::{
    execute_response::Result as ProtoResult, ExecutionError, FunctionArgument, FunctionResult,
    ReturnValue, StartProcessRequest,
};
use sandbox::Sandbox;

trait WasmPtrExt<'a> {
    fn as_byte_array_mut(&self, mem: &'a Memory, len: usize) -> Option<&'a mut [u8]>;
}

impl<'a> WasmPtrExt<'a> for WasmPtr<u8, Array> {
    fn as_byte_array_mut(&self, mem: &'a Memory, len: usize) -> Option<&'a mut [u8]> {
        unsafe {
            // black magic casting (Cell doesn't contain any data which is why this works)
            self.deref_mut(&mem, 0, len as u32)
                .map(|cells| (&mut *(cells as *mut [Cell<u8>] as *mut Cell<[u8]>)).get_mut())
        }
    }
}

#[derive(Error, Debug)]
enum WasmError {
    #[error("Unknown: {0}")]
    Unknown(String),

    #[error("{0}")]
    ConversionError(String),

    #[error("Failed to read string pointer for \"{0}\"")]
    FailedToReadStringPointer(String),

    #[error("Failed to find key: {0}")]
    FailedToFindKey(String),

    #[error("Failed to deref pointer.")]
    FailedToDerefPointer(),

    #[error("Failed to start process: {0}.")]
    FailedToStartProcess(#[from] io::Error),

    #[error("Failed to decode value from protobuf: {0}")]
    FailedToDecodeProtobuf(#[from] prost::DecodeError),

    #[error("Failed to encode value from protobuf: {0}")]
    FailedToEncodeProtobuf(#[from] prost::EncodeError),
}

type Result<T> = std::result::Result<T, WasmError>;

trait ToErrorCode<T> {
    fn to_error_code(self) -> u32;
}

impl<T> ToErrorCode<T> for Result<T> {
    fn to_error_code(self) -> u32 {
        match self {
            Ok(_) => 0,
            Err(e) => e.into(),
        }
    }
}

impl From<WasmError> for u32 {
    fn from(err: WasmError) -> Self {
        match err {
            WasmError::Unknown(_) => 1,
            WasmError::FailedToDerefPointer() => 2,
            WasmError::FailedToDecodeProtobuf(_) => 3,
            WasmError::ConversionError(_) => 4,
            WasmError::FailedToReadStringPointer(_) => 5,
            WasmError::FailedToFindKey(_) => 6,
            WasmError::FailedToEncodeProtobuf(_) => 7,
            WasmError::FailedToStartProcess(_) => 8,
        }
    }
}

fn map_sandbox_dir(sandbox: &Sandbox, arg: &str) -> String {
    let regex = Regex::new(r"(^|[=\s;:])sandbox(\b)").unwrap();

    regex
        .replace_all(arg, |caps: &regex::Captures| {
            format!(
                "{}{}{}",
                &caps[1],
                &sandbox.path().to_string_lossy(),
                &caps[2]
            )
        })
        .into_owned()
}

fn get_args_and_envs(
    request: &StartProcessRequest,
    sandbox: &Sandbox,
) -> (Vec<String>, HashMap<String, String>) {
    (
        request
            .args
            .iter()
            .map(|arg| map_sandbox_dir(sandbox, arg))
            .collect(),
        request
            .environment_variables
            .iter()
            .map(|(k, v)| (k.clone(), map_sandbox_dir(sandbox, v)))
            .collect(),
    )
}

fn start_process(
    logger: &Logger,
    sandbox: &Sandbox,
    vm_memory: &Memory,
    request: WasmPtr<u8, Array>,
    len: u32,
    pid_out: WasmPtr<u64, Item>,
) -> Result<()> {
    let request: StartProcessRequest = request
        .deref(vm_memory, 0, len)
        .ok_or_else(WasmError::FailedToDerefPointer)
        .and_then(|cells| {
            StartProcessRequest::decode(
                cells
                    .iter()
                    .map(|v| v.get())
                    .collect::<Vec<u8>>()
                    .as_slice(),
            )
            .map_err(WasmError::FailedToDecodeProtobuf)
        })?;

    let (args, env) = get_args_and_envs(&request, sandbox);
    info!(
        logger,
        "Launching host process {}, args: {:#?}", request.command, &args
    );

    Command::new(request.command)
        .args(args)
        .envs(&env)
        .spawn()
        .map_err(|e| {
            println!("Failed to launch host process: {}", e);
            WasmError::FailedToStartProcess(e)
        })
        .and_then(|c| unsafe {
            pid_out
                .deref_mut(&vm_memory)
                .ok_or_else(WasmError::FailedToDerefPointer)
                .map(|cell| cell.set(c.id() as u64))
        })
}

fn run_process(
    logger: &Logger,
    sandbox: &Sandbox,
    vm_memory: &Memory,
    request: WasmPtr<u8, Array>,
    len: u32,
    exit_code_out: WasmPtr<i32, Item>,
) -> Result<()> {
    let request: StartProcessRequest = request
        .deref(vm_memory, 0, len)
        .ok_or_else(WasmError::FailedToDerefPointer)
        .and_then(|cells| {
            StartProcessRequest::decode(
                cells
                    .iter()
                    .map(|v| v.get())
                    .collect::<Vec<u8>>()
                    .as_slice(),
            )
            .map_err(WasmError::FailedToDecodeProtobuf)
        })?;

    let (args, env) = get_args_and_envs(&request, sandbox);
    info!(
        logger,
        "Running host process {} (and waiting for exit), args: {:#?}", request.command, &args
    );

    Command::new(request.command)
        .args(args)
        .envs(&env)
        .status()
        .map_err(|e| {
            println!("Failed to run host process: {}", e);
            WasmError::FailedToStartProcess(e)
        })
        .and_then(|c| unsafe {
            exit_code_out
                .deref_mut(&vm_memory)
                .ok_or_else(WasmError::FailedToDerefPointer)
                .map(|cell| cell.set(c.code().unwrap_or(-1)))
        })
}

fn get_input_len(
    vm_memory: &Memory,
    key: WasmPtr<u8, Array>,
    keylen: u32,
    value: WasmPtr<u64, Item>,
    arguments: &[FunctionArgument],
) -> Result<()> {
    let key = key
        .get_utf8_string(vm_memory, keylen)
        .ok_or_else(|| WasmError::FailedToReadStringPointer("key".to_owned()))?;

    arguments
        .iter()
        .find(|a| a.name == key)
        .ok_or_else(|| WasmError::FailedToFindKey(key.to_string()))
        .and_then(|a| {
            let len = a.encoded_len();
            unsafe {
                value
                    .deref_mut(vm_memory)
                    .ok_or_else(WasmError::FailedToDerefPointer)
                    .map(|c| {
                        c.set(len as u64);
                    })
            }
        })
}

fn get_input(
    vm_memory: &Memory,
    key: WasmPtr<u8, Array>,
    keylen: u32,
    value: WasmPtr<u8, Array>,
    valuelen: u32,
    arguments: &[FunctionArgument],
) -> Result<()> {
    let key = key
        .get_utf8_string(vm_memory, keylen)
        .ok_or_else(|| WasmError::FailedToReadStringPointer("key".to_owned()))?;

    arguments
        .iter()
        .find(|a| a.name == key)
        .ok_or_else(|| WasmError::FailedToFindKey(key.to_string()))
        .and_then(|a| {
            value
                .as_byte_array_mut(&vm_memory, valuelen as usize)
                .ok_or_else(|| {
                    WasmError::ConversionError(
                        "Failed to convert provided input buffer to mut byte array.".to_owned(),
                    )
                })
                .and_then(|mut buff| {
                    a.encode(&mut buff)
                        .map_err(WasmError::FailedToEncodeProtobuf)
                })
        })
}

fn set_output(vm_memory: &Memory, val: WasmPtr<u8, Array>, vallen: u32) -> Result<ReturnValue> {
    val.deref(vm_memory, 0, vallen)
        .ok_or_else(WasmError::FailedToDerefPointer)
        .and_then(|cells| {
            ReturnValue::decode(
                cells
                    .iter()
                    .map(|v| v.get())
                    .collect::<Vec<u8>>()
                    .as_slice(),
            )
            .map_err(WasmError::FailedToDecodeProtobuf)
        })
}

fn set_error(vm_memory: &Memory, msg: WasmPtr<u8, Array>, msglen: u32) -> Result<String> {
    msg.get_utf8_string(vm_memory, msglen)
        .ok_or_else(|| WasmError::FailedToReadStringPointer("msg".to_owned()))
        .map(|s| s.to_owned())
}

fn execute_function(
    logger: Logger,
    function_name: &str,
    _entrypoint: &str,
    code: &[u8],
    arguments: &[FunctionArgument],
) -> std::result::Result<Vec<ReturnValue>, String> {
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

    let mut import_object = generate_import_object_from_state(wasi_state, wasi_version);

    // inject gbk specific functions in the wasm state
    let a = arguments.to_vec();
    let a2 = arguments.to_vec();
    let v: Vec<std::result::Result<ReturnValue, String>> = Vec::new();
    let results = Arc::new(RwLock::new(v));
    let res = Arc::clone(&results);
    let res2 = Arc::clone(&results);
    let start_process_logger = logger.new(o!("scope" => "start_process"));
    let run_process_logger = logger.new(o!("scope" => "run_process"));
    let gbk_imports = imports! {
        "gbk" => {
            "start_host_process" => func!(move |ctx: &mut Ctx, s: WasmPtr<u8, Array>, len: u32, pid_out: WasmPtr<u64, Item>| {
                start_process(&start_process_logger, &sandbox, ctx.memory(0), s, len, pid_out).to_error_code()
            }),

            "run_host_process" => func!(move |ctx: &mut Ctx, s: WasmPtr<u8, Array>, len: u32, exit_code_out: WasmPtr<i32, Item>| {
                run_process(&run_process_logger, &sandbox2, ctx.memory(0), s, len, exit_code_out).to_error_code()
            }),

            "get_input_len" => func!(move |ctx: &mut Ctx, key: WasmPtr<u8, Array>, keylen: u32, value: WasmPtr<u64, Item>| {
                get_input_len(ctx.memory(0), key, keylen, value, &a).to_error_code()
            }),
            "get_input" => func!(move |ctx: &mut Ctx, key: WasmPtr<u8, Array>, keylen: u32, value: WasmPtr<u8, Array>, valuelen: u32| {
                get_input(ctx.memory(0), key, keylen, value, valuelen, &a2).to_error_code()
            }),
            "set_output" => func!(move |ctx: &mut Ctx, val: WasmPtr<u8, Array>, vallen: u32| {
                set_output(ctx.memory(0), val, vallen).and_then(|v| {
                    res.write().map(|mut writer| {
                        writer.push(Ok(v));
                    }).map_err(|e| {WasmError::Unknown(format!("{}", e))})
                }).to_error_code()
            }),
            "set_error" => func!(move |ctx: &mut Ctx, msg: WasmPtr<u8, Array>, msglen: u32 | {
                set_error(ctx.memory(0), msg, msglen).and_then(|v| {
                    res2.write().map(|mut writer| {
                        writer.push(Err(v));
                    }).map_err(|e| {WasmError::Unknown(format!("{}", e))})
                }).to_error_code()
            }),
        },
    };
    import_object.extend(gbk_imports);

    let instance = module
        .instantiate(&import_object)
        .map_err(|e| format!("failed to instantiate WASI module: {}", e))?;

    let entry_function: Func<(), ()> = instance
        .func(ENTRY)
        .map_err(|e| format!("Failed to resolve entrypoint {}: {}", ENTRY, e))?;

    // TODO: capture STDOUT and store/log
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
        arguments: &[FunctionArgument],
    ) -> ProtoResult {
        execute_function(
            self.logger.new(o!("function" => function_name.to_owned())),
            function_name,
            entrypoint,
            code,
            arguments,
        )
        .map_or_else(
            |e| ProtoResult::Error(ExecutionError { msg: e }),
            |v| ProtoResult::Ok(FunctionResult { values: v }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::ArgumentType;
    use std::path::Path;
    use wasmer_runtime::{types::MemoryDescriptor, units::Pages};

    macro_rules! null_logger {
        () => {{
            slog::Logger::root(slog::Discard, o!())
        }};
    }

    fn write_to_ptr(ptr: &WasmPtr<u8, Array>, mem: &Memory, data: &[u8]) {
        unsafe {
            ptr.deref_mut(&mem, 0, data.len() as u32).map(|cells| {
                cells.iter().zip(data).for_each(|(cell, byte)| {
                    cell.set(*byte);
                });
            });
        }
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
            &vec![],
        );

        assert!(res.is_ok());
    }

    macro_rules! create_mem {
        () => {{
            Memory::new(MemoryDescriptor::new(Pages(1), None, false).unwrap()).unwrap()
        }};
    }

    #[test]
    fn test_get_input_len() {
        // get with bad key ptr
        let mem = create_mem!();

        // Will fail to parse key as str if the size is larger than its
        // memory available
        let key_ptr: WasmPtr<u8, Array> = WasmPtr::new(std::u32::MAX);
        let val: WasmPtr<u64, Item> = WasmPtr::new(0);
        let res = get_input_len(
            &mem,
            key_ptr,
            5 as u32,
            val,
            &[FunctionArgument {
                name: "chorizo korvén".to_owned(),
                r#type: ArgumentType::Bytes as i32,
                value: vec![1, 2, 3],
            }],
        );

        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            WasmError::FailedToReadStringPointer(..)
        ));

        // get non existant input
        let mem = create_mem!();
        let key_ptr: WasmPtr<u8, Array> = WasmPtr::new(0);

        let arg_name = "sune was here".to_owned();
        let arg_name_bytes = arg_name.as_bytes();
        write_to_ptr(&key_ptr, &mem, arg_name_bytes);

        let val: WasmPtr<u64, Item> = WasmPtr::new(arg_name_bytes.len() as u32);
        let res = get_input_len(
            &mem,
            key_ptr,
            arg_name.len() as u32,
            val,
            &[FunctionArgument {
                name: "chorizo korvén".to_owned(),
                r#type: ArgumentType::Bytes as i32,
                value: vec![1, 2, 3],
            }],
        );

        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), WasmError::FailedToFindKey(..)));

        // get existing input
        let mem = create_mem!();
        let key_ptr: WasmPtr<u8, Array> = WasmPtr::new(0);
        let arg_name = "input1".to_owned();
        let arg_name_bytes = arg_name.as_bytes();

        let function_argument = FunctionArgument {
            name: "input1".to_owned(),
            r#type: ArgumentType::Bytes as i32,
            value: vec![1, 2, 3],
        };

        write_to_ptr(&key_ptr, &mem, arg_name_bytes);
        let val: WasmPtr<u64, Item> = WasmPtr::new(arg_name_bytes.len() as u32);
        let res = get_input_len(
            &mem,
            key_ptr,
            arg_name.len() as u32,
            val,
            &[function_argument.clone()],
        );
        assert!(res.is_ok());

        let write_len: u64 = val.deref(&mem).map(|cell| cell.get() as u64).unwrap();
        assert_eq!(function_argument.encoded_len(), write_len as usize);

        // get existing input with invalid pointer
        let mem = create_mem!();
        let key_ptr: WasmPtr<u8, Array> = WasmPtr::new(0);
        let arg_name = "input1".to_owned();
        let arg_name_bytes = arg_name.as_bytes();

        let function_argument = FunctionArgument {
            name: "input1".to_owned(),
            r#type: ArgumentType::Bytes as i32,
            value: vec![1, 2, 3],
        };

        write_to_ptr(&key_ptr, &mem, arg_name_bytes);
        let val: WasmPtr<u64, Item> = WasmPtr::new(std::u32::MAX);
        let res = get_input_len(
            &mem,
            key_ptr,
            arg_name.len() as u32,
            val,
            &[function_argument.clone()],
        );

        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            WasmError::FailedToDerefPointer()
        ));
    }

    #[test]
    fn test_get_input() {
        // testing invalid key pointer
        let mem = create_mem!();
        let key_ptr: WasmPtr<u8, Array> = WasmPtr::new(std::u32::MAX);
        let value_ptr: WasmPtr<u8, Array> = WasmPtr::new(0);

        let res = get_input(&mem, key_ptr, 5 as u32, value_ptr, 0 as u32, &vec![]);
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            WasmError::FailedToReadStringPointer(..)
        ));

        // testing failed to find key
        let mem = create_mem!();
        let key_ptr: WasmPtr<u8, Array> = WasmPtr::new(0);
        let key_name = "input1".to_owned();
        write_to_ptr(&key_ptr, &mem, key_name.as_bytes());

        let res = get_input(
            &mem,
            key_ptr,
            key_name.len() as u32,
            WasmPtr::new(0),
            0 as u32,
            &vec![],
        );

        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), WasmError::FailedToFindKey(..)));

        // testing failing to convert provided input
        let mem = create_mem!();
        let key_ptr: WasmPtr<u8, Array> = WasmPtr::new(0);
        let key_name = "input1".to_owned();
        write_to_ptr(&key_ptr, &mem, key_name.as_bytes());

        let function_argument = FunctionArgument {
            name: "input1".to_owned(),
            r#type: ArgumentType::Bytes as i32,
            value: vec![1, 2, 3],
        };

        let res = get_input(
            &mem,
            key_ptr,
            key_name.len() as u32,
            WasmPtr::new(std::u32::MAX),
            1 as u32,
            &vec![function_argument],
        );

        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), WasmError::ConversionError(..)));

        // testing failed to encode protobuf
        let mem = create_mem!();
        let key_ptr: WasmPtr<u8, Array> = WasmPtr::new(0);
        let key_name = "input1".to_owned();

        let function_argument = FunctionArgument {
            name: "input1".to_owned(),
            r#type: ArgumentType::Bytes as i32,
            value: vec![1, 2, 3],
        };

        write_to_ptr(&key_ptr, &mem, key_name.as_bytes());
        let encoded_len = function_argument.encoded_len();

        let mut reference_value = Vec::with_capacity(encoded_len);
        function_argument.encode(&mut reference_value).unwrap();

        let value_ptr: WasmPtr<u8, Array> = WasmPtr::new(key_name.as_bytes().len() as u32);
        let res = get_input(
            &mem,
            key_ptr,
            key_name.len() as u32,
            value_ptr,
            (encoded_len - 1) as u32,
            &vec![function_argument],
        );

        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            WasmError::FailedToEncodeProtobuf(..)
        ));

        // testing getting valid input
        let mem = create_mem!();
        let key_ptr: WasmPtr<u8, Array> = WasmPtr::new(0);
        let key_name = "input1".to_owned();

        let function_argument = FunctionArgument {
            name: "input1".to_owned(),
            r#type: ArgumentType::Bytes as i32,
            value: vec![1, 2, 3],
        };

        write_to_ptr(&key_ptr, &mem, key_name.as_bytes());
        let encoded_len = function_argument.encoded_len();

        let mut reference_value = Vec::with_capacity(encoded_len);
        function_argument.encode(&mut reference_value).unwrap();

        let value_ptr: WasmPtr<u8, Array> = WasmPtr::new(key_name.as_bytes().len() as u32);
        let res = get_input(
            &mem,
            key_ptr,
            key_name.len() as u32,
            value_ptr,
            encoded_len as u32,
            &vec![function_argument],
        );

        assert!(res.is_ok());

        // check that the byte patterns are identical
        let encoded = value_ptr
            .deref(&mem, 0, encoded_len as u32)
            .unwrap()
            .iter()
            .map(|c| c.get())
            .collect::<Vec<u8>>();

        assert_eq!(reference_value, encoded);
    }

    #[test]
    fn test_set_output() {
        let mem = create_mem!();
        let ptr: WasmPtr<u8, Array> = WasmPtr::new(0);

        // testing bad pointer

        let res = set_output(&mem, WasmPtr::new(0), std::u32::MAX);

        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            WasmError::FailedToDerefPointer()
        ));

        let return_value = ReturnValue {
            name: "sune".to_owned(),
            r#type: ArgumentType::Int as i32,
            value: vec![1, 2, 3, 4, 5, 6, 7, 8],
        };

        let encoded_len = return_value.encoded_len();
        let mut return_value_bytes = Vec::with_capacity(encoded_len);
        return_value.encode(&mut return_value_bytes).unwrap();

        // Try with empty pointer
        let res = set_output(&mem, ptr, encoded_len as u32);
        assert!(matches!(
            res.unwrap_err(),
            WasmError::FailedToDecodeProtobuf(..)
        ));

        // Try with written pointer
        write_to_ptr(&ptr, &mem, &return_value_bytes);
        let res = set_output(&mem, ptr, encoded_len as u32);

        assert!(res.is_ok());
        assert_eq!(return_value, res.unwrap());
    }

    #[test]
    fn test_map_sandbox_dir() {
        let sandbox = Sandbox::new();
        assert_eq!(
            sandbox.path().join("some").join("dir"),
            Path::new(&map_sandbox_dir(&sandbox, "sandbox/some/dir"))
        );

        assert_eq!(
            format!("--some-arg={}", sandbox.path().display()),
            map_sandbox_dir(&sandbox, "--some-arg=sandbox")
        );

        assert_eq!(
            format!("{0};{0}", sandbox.path().display()),
            map_sandbox_dir(&sandbox, "sandbox;sandbox")
        );

        assert_eq!(
            format!("{0}:{0}", sandbox.path().display()),
            map_sandbox_dir(&sandbox, "sandbox:sandbox")
        );

        assert_eq!(
            "some/dir/sandbox/something/else",
            &map_sandbox_dir(&sandbox, "some/dir/sandbox/something/else")
        );

        assert_eq!(
            format!("{};kallekula/sandbox", sandbox.path().display()),
            map_sandbox_dir(&sandbox, "sandbox;kallekula/sandbox")
        );

        assert_eq!(
            format!("sandboxno;{}/yes", sandbox.path().display()),
            map_sandbox_dir(&sandbox, "sandboxno;sandbox/yes")
        );
    }
}
