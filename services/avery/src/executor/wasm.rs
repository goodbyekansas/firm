use std::{
    cell::Cell,
    process::Command,
    str,
    sync::{Arc, RwLock},
};

use prost::Message;
use thiserror::Error;
use wasmer_runtime::{compile, func, imports, memory::Memory, Array, Ctx, Func, Item, WasmPtr};
use wasmer_wasi::{generate_import_object_from_state, get_wasi_version, state::WasiState};

use crate::executor::FunctionExecutor;
use crate::proto::{
    execute_response::Result as ProtoResult, ExecutionError, FunctionArgument, FunctionResult,
    ReturnValue,
};

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

    #[error("Failed to decode value from protobuf: {0}")]
    FailedToDecodeProtobuf(#[from] prost::DecodeError),

    #[error("Failed to encode value from protobuf: {0}")]
    FailedToEncodeProtobuf(#[from] prost::EncodeError),
}

type Result<T> = std::result::Result<T, WasmError>;

trait ToErrorCode<T> {
    fn to_error_code(self) -> i32;
}

impl<T> ToErrorCode<T> for Result<T> {
    fn to_error_code(self) -> i32 {
        match self {
            Ok(_) => 0,
            Err(e) => e.into(),
        }
    }
}

impl From<WasmError> for i32 {
    fn from(err: WasmError) -> Self {
        match err {
            WasmError::Unknown(_) => 1,
            WasmError::FailedToDerefPointer() => 2,
            WasmError::FailedToDecodeProtobuf(_) => 3,
            _ => -1, // TODO som fan
        }
    }
}

fn start_process(ctx: &mut Ctx, s: WasmPtr<u8, Array>, len: u32) -> i64 {
    let memory = ctx.memory(0);
    match s.get_utf8_string(memory, len) {
        Some(command) => match Command::new(command).spawn() {
            Ok(_) => 1,
            Err(_) => 0,
        },
        _ => 0,
    }
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
                    .ok_or_else(|| WasmError::FailedToDerefPointer())
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
                        .map_err(|e| WasmError::FailedToEncodeProtobuf(e))
                })
        })
}

fn set_output(vm_memory: &Memory, val: WasmPtr<u8, Array>, vallen: u32) -> Result<ReturnValue> {
    val.deref(vm_memory, 0, vallen)
        .ok_or_else(|| WasmError::FailedToDerefPointer())
        .and_then(|cells| {
            ReturnValue::decode(
                cells
                    .iter()
                    .map(|v| v.get())
                    .collect::<Vec<u8>>()
                    .as_slice(),
            )
            .map_err(|e| WasmError::FailedToDecodeProtobuf(e))
        })
}

fn execute_function(
    function_name: &str,
    _entrypoint: &str,
    code: &[u8],
    arguments: &[FunctionArgument],
) -> std::result::Result<Vec<ReturnValue>, String> {
    const ENTRY: &str = "_start";
    let module = compile(code).map_err(|e| format!("failed to compile wasm: {}", e))?;

    let wasi_version = get_wasi_version(&module, true).unwrap_or(wasmer_wasi::WasiVersion::Latest);

    let wasi_state = WasiState::new(&format!("wasi-{}", function_name))
        .build()
        .map_err(|e| format!("Failed to create wasi state: {:?}", e))?;

    let mut import_object = generate_import_object_from_state(wasi_state, wasi_version);

    // inject gbk specific functions in the wasm state
    let a = arguments.to_vec();
    let a2 = arguments.to_vec();
    let results = Arc::new(RwLock::new(Vec::new()));
    let res = Arc::clone(&results);
    let gbk_imports = imports! {
        "gbk" => {
            "start_host_process" => func!(start_process),
            "get_input_len" => func!(move |ctx: &mut Ctx, key: WasmPtr<u8, Array>, keylen: u32, value: WasmPtr<u64, Item>| get_input_len(ctx.memory(0), key, keylen, value, &a).to_error_code()),
            "get_input" => func!(move |ctx: &mut Ctx, key: WasmPtr<u8, Array>, keylen: u32, value: WasmPtr<u8, Array>, valuelen: u32| get_input(ctx.memory(0), key, keylen, value, valuelen, &a2).to_error_code()),
            "set_ouptut" => func!(move |ctx: &mut Ctx, val: WasmPtr<u8, Array>, vallen: u32| {
                set_output(ctx.memory(0), val, vallen).and_then(|v| {
                    res.write().map(|mut writer| {
                        writer.push(v);
                    }).map_err(|e| {WasmError::Unknown(format!("{}", e))})
                }).to_error_code();
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
                .map(|reader| reader.iter().cloned().collect())
                .map_err(|e| format!("Failed to read function results: {}", e))
        })
}

pub struct WasmExecutor {}

impl FunctionExecutor for WasmExecutor {
    fn execute(
        &self,
        function_name: &str,
        entrypoint: &str,
        code: &[u8],
        arguments: &[FunctionArgument],
    ) -> ProtoResult {
        execute_function(function_name, entrypoint, code, arguments).map_or_else(
            |e| ProtoResult::Error(ExecutionError { msg: e }),
            |_| ProtoResult::Ok(FunctionResult { values: vec![] }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::ArgumentType;
    use wasmer_runtime::{types::MemoryDescriptor, units::Pages};

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
        let executor = WasmExecutor {};
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
            dbg!(res).unwrap_err(),
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
}
