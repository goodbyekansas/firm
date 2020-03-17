use std::{
    cell::Cell,
    process::Command,
    str,
    sync::{Arc, RwLock},
};

use prost::Message;
use wasmer_runtime::{compile, func, imports, memory::Memory, Array, Ctx, Func, WasmPtr};
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
    arguments: &[FunctionArgument],
) -> u64 {
    // TODO: Do not do unwrap_or
    let key = key.get_utf8_string(vm_memory, keylen).unwrap_or("");

    arguments
        .iter()
        .find(|a| a.name == key)
        .map(|a| a.encoded_len())
        .unwrap_or(0) as u64
}

fn get_input(
    vm_memory: &Memory,
    key: WasmPtr<u8, Array>,
    keylen: u32,
    value: WasmPtr<u8, Array>,
    valuelen: u32,
    arguments: &[FunctionArgument],
) -> u32 {
    let key = key.get_utf8_string(vm_memory, keylen).unwrap_or("");
    arguments
        .iter()
        .find(|a| a.name == key)
        .and_then(|a| {
            value.as_byte_array_mut(&vm_memory, valuelen as usize).and_then(
                |mut buff| {
                    a.encode(&mut buff).ok()?;

                    // note that we cannot use buff here since it is
                    // consumed by the above
                    Some(valuelen as u32)
                },
            )
        })
        .unwrap_or(0u32)
}

fn set_output(vm_memory: &Memory, val: WasmPtr<u8, Array>, vallen: u32) -> Option<ReturnValue> {
    val.deref(vm_memory, 0, vallen).and_then(|cells| {
        ReturnValue::decode(
            cells
                .iter()
                .map(|v| v.get())
                .collect::<Vec<u8>>()
                .as_slice(),
        )
        .ok()
    })
}

fn execute_function(
    function_name: &str,
    _entrypoint: &str,
    code: &[u8],
    arguments: &[FunctionArgument],
) -> Result<Vec<ReturnValue>, String> {
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
            "get_input_len" => func!(move |ctx: &mut Ctx, key: WasmPtr<u8, Array>, keylen: u32| get_input_len(ctx.memory(0), key, keylen, &a)),
            "get_input" => func!(move |ctx: &mut Ctx, key: WasmPtr<u8, Array>, keylen: u32, value: WasmPtr<u8, Array>, valuelen: u32| get_input(ctx.memory(0), key, keylen, value, valuelen, &a2)),
            "set_ouptut" => func!(move |ctx: &mut Ctx, val: WasmPtr<u8, Array>, vallen: u32| {
                set_output(ctx.memory(0), val, vallen).and_then(|v| {
                    res.write().map(|mut writer| {
                        writer.push(v);
                    }).ok()
                }).unwrap_or(());
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
        // get non existant input
        let mem = create_mem!();
        let ptr: WasmPtr<u8, Array> = WasmPtr::new(0);
        let arg_name = "sune was here".to_owned();
        write_to_ptr(&ptr, &mem, arg_name.as_bytes());

        let res = get_input_len(
            &mem,
            ptr,
            arg_name.len() as u32,
            &[FunctionArgument {
                name: "chorizo korvén".to_owned(),
                r#type: ArgumentType::Bytes as i32,
                value: vec![1, 2, 3],
            }],
        );

        assert_eq!(0, res);

        // get existing input
        let mem = create_mem!();
        let ptr: WasmPtr<u8, Array> = WasmPtr::new(0);
        let arg_name = "input1".to_owned();

        let function_argument = FunctionArgument {
            name: "input1".to_owned(),
            r#type: ArgumentType::Bytes as i32,
            value: vec![1, 2, 3],
        };

        write_to_ptr(&ptr, &mem, arg_name.as_bytes());
        let res = get_input_len(
            &mem,
            ptr,
            arg_name.len() as u32,
            &[function_argument.clone()],
        );

        assert_eq!(function_argument.encoded_len(), res as usize);
    }

    #[test]
    fn test_get_input() {
        let mem = create_mem!();
        let ptr: WasmPtr<u8, Array> = WasmPtr::new(0);
        let arg_name = "input1".to_owned();

        let function_argument = FunctionArgument {
            name: "input1".to_owned(),
            r#type: ArgumentType::Bytes as i32,
            value: vec![1, 2, 3],
        };

        write_to_ptr(&ptr, &mem, arg_name.as_bytes());
        let encoded_len = function_argument.encoded_len();

        let mut reference_value = Vec::with_capacity(encoded_len);
        function_argument.encode(&mut reference_value).unwrap();

        let value_ptr: WasmPtr<u8, Array> = WasmPtr::new(arg_name.as_bytes().len() as u32);
        let res = get_input(
            &mem,
            ptr,
            arg_name.len() as u32,
            value_ptr,
            encoded_len as u32,
            &vec![function_argument],
        );

        assert_eq!(encoded_len, res as usize);

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
        assert!(res.is_none());

        // Try with written pointer
        write_to_ptr(&ptr, &mem, &return_value_bytes);
        let res = set_output(&mem, ptr, encoded_len as u32);

        assert!(res.is_some());
        assert_eq!(return_value, res.unwrap());
    }
}
