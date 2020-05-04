use std::cell::Cell;

use super::error::{WasiResult, WasmError};
use prost::Message;
use wasmer_runtime::{memory::Memory, Array, Item, WasmPtr};

use gbk_protocols::functions::{FunctionArgument, ReturnValue};

pub trait WasmPtrExt<'a> {
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

pub fn get_input_len(
    vm_memory: &Memory,
    key: WasmPtr<u8, Array>,
    keylen: u32,
    value: WasmPtr<u64, Item>,
    arguments: &[FunctionArgument],
) -> WasiResult<()> {
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

pub fn get_input(
    vm_memory: &Memory,
    key: WasmPtr<u8, Array>,
    keylen: u32,
    value: WasmPtr<u8, Array>,
    valuelen: u32,
    arguments: &[FunctionArgument],
) -> WasiResult<()> {
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

pub fn set_output(
    vm_memory: &Memory,
    val: WasmPtr<u8, Array>,
    vallen: u32,
) -> WasiResult<ReturnValue> {
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

pub fn set_error(vm_memory: &Memory, msg: WasmPtr<u8, Array>, msglen: u32) -> WasiResult<String> {
    msg.get_utf8_string(vm_memory, msglen)
        .ok_or_else(|| WasmError::FailedToReadStringPointer("msg".to_owned()))
        .map(|s| s.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbk_protocols::functions::ArgumentType;
    use wasmer_runtime::memory::Memory;
    use wasmer_runtime::{types::MemoryDescriptor, units::Pages};

    macro_rules! create_mem {
        () => {{
            Memory::new(MemoryDescriptor::new(Pages(1), None, false).unwrap()).unwrap()
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
}
