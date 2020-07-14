use std::{
    cell::Cell,
    path::{Path, PathBuf},
};

use super::error::{WasiError, WasiResult};
use prost::Message;
use wasmer_runtime::{memory::Memory, Array, Item, WasmPtr};

use super::sandbox::Sandbox;
use crate::executor::{AttachmentDownload, FunctionContextExt};
use gbk_protocols::functions::{FunctionAttachment, FunctionContext, ReturnValue};

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

fn attachment_path_from_descriptor(attachment_data: &FunctionAttachment) -> String {
    let attachment_filename = if attachment_data.filename.is_empty() {
        &attachment_data.name
    } else {
        &attachment_data.filename
    };

    // Manually joining paths to ensure we get a valid path for WASI (no backslash)
    format!("attachments/{}", attachment_filename)
}

fn write_length_to_ptr(
    length_ptr: WasmPtr<u32, Item>,
    length: u32,
    vm_memory: &Memory,
) -> WasiResult<()> {
    unsafe {
        length_ptr
            .deref_mut(vm_memory)
            .ok_or_else(WasiError::FailedToDerefPointer)
            .map(|c| {
                c.set(length);
            })
    }
}

fn write_path_to_ptr(
    path_ptr: WasmPtr<u8, Array>,
    path_buffer_len: u32,
    path: &str,
    vm_memory: &Memory,
) -> WasiResult<()> {
    path_ptr
        .as_byte_array_mut(&vm_memory, path_buffer_len as usize)
        .ok_or_else(|| {
            WasiError::ConversionError(
                "Failed to convert provided input path buffer to mut byte array.".to_owned(),
            )
        })
        .and_then(|buff| {
            buff.clone_from_slice(path.as_bytes());
            Ok(())
        })
}

fn download_and_map_at(attachment_data: &FunctionAttachment, path: &Path) -> WasiResult<()> {
    if !path.exists() {
        attachment_data
            .download()
            .map_err(|e| {
                WasiError::FailedToMapAttachment(attachment_data.name.to_owned(), Box::new(e))
            })
            .and_then(|data| {
                // TODO: Map attachment differently depending on metadata.
                // We need to support mapping folders as well.
                std::fs::write(path, data).map_err(|e| {
                    WasiError::FailedToMapAttachment(attachment_data.name.to_owned(), Box::new(e))
                })
            })?;
    }

    Ok(())
}

pub fn get_attachment_path_len(
    function_context: &FunctionContext,
    vm_memory: &Memory,
    attachment_name: WasmPtr<u8, Array>,
    attachment_name_len: u32,
    path_len: WasmPtr<u32, Item>,
) -> WasiResult<()> {
    let attachment_key = attachment_name
        .get_utf8_string(vm_memory, attachment_name_len)
        .ok_or_else(|| WasiError::FailedToReadStringPointer("attachment_name".to_owned()))?;

    let attachment_data = function_context
        .get_attachment(attachment_key)
        .ok_or_else(|| WasiError::FailedToFindAttachment(attachment_key.to_owned()))?;
    write_length_to_ptr(
        path_len,
        attachment_path_from_descriptor(&attachment_data)
            .as_bytes()
            .len() as u32,
        vm_memory,
    )
}

pub fn map_attachment(
    function_context: &FunctionContext,
    sandbox: &Sandbox,
    vm_memory: &Memory,
    attachment_name: WasmPtr<u8, Array>,
    attachment_name_len: u32,
    path_ptr: WasmPtr<u8, Array>,
    path_buffer_len: u32,
) -> WasiResult<()> {
    let attachment_key = attachment_name
        .get_utf8_string(vm_memory, attachment_name_len)
        .ok_or_else(|| WasiError::FailedToReadStringPointer("attachment_name".to_owned()))?;

    let attachment_data = function_context
        .get_attachment(attachment_key)
        .ok_or_else(|| WasiError::FailedToFindAttachment(attachment_key.to_owned()))?;

    // Ensure path is platform specific to host and not the function
    let wasi_attachment_path = attachment_path_from_descriptor(&attachment_data);
    let native_attachment_path = sandbox.path().join(PathBuf::from(&wasi_attachment_path));
    download_and_map_at(&attachment_data, &native_attachment_path)?;
    write_path_to_ptr(path_ptr, path_buffer_len, &wasi_attachment_path, vm_memory)
}

pub fn get_attachment_path_len_from_descriptor(
    vm_memory: &Memory,
    attachment_descriptor_ptr: WasmPtr<u8, Array>,
    attachment_descriptor_len: u32,
    path_len: WasmPtr<u32, Item>,
) -> WasiResult<()> {
    let fa = attachment_descriptor_ptr
        .deref(vm_memory, 0, attachment_descriptor_len)
        .ok_or_else(WasiError::FailedToDerefPointer)
        .and_then(|cells| {
            FunctionAttachment::decode(
                cells
                    .iter()
                    .map(|v| v.get())
                    .collect::<Vec<u8>>()
                    .as_slice(),
            )
            .map_err(WasiError::FailedToDecodeProtobuf)
        })?;

    write_length_to_ptr(
        path_len,
        attachment_path_from_descriptor(&fa).as_bytes().len() as u32,
        vm_memory,
    )
}

pub fn map_attachment_from_descriptor(
    sandbox: &Sandbox,
    vm_memory: &Memory,
    attachment_descriptor_ptr: WasmPtr<u8, Array>,
    attachment_descriptor_len: u32,
    path_ptr: WasmPtr<u8, Array>,
    path_buffer_len: u32,
) -> WasiResult<()> {
    let fa = attachment_descriptor_ptr
        .deref(vm_memory, 0, attachment_descriptor_len)
        .ok_or_else(WasiError::FailedToDerefPointer)
        .and_then(|cells| {
            FunctionAttachment::decode(
                cells
                    .iter()
                    .map(|v| v.get())
                    .collect::<Vec<u8>>()
                    .as_slice(),
            )
            .map_err(WasiError::FailedToDecodeProtobuf)
        })?;

    // Ensure path is platform specific to host and not the function
    let wasi_attachment_path = attachment_path_from_descriptor(&fa);
    let native_attachment_path = sandbox.path().join(PathBuf::from(&wasi_attachment_path));
    download_and_map_at(&fa, &native_attachment_path)?;
    write_path_to_ptr(path_ptr, path_buffer_len, &wasi_attachment_path, vm_memory)
}

pub fn get_input_len(
    vm_memory: &Memory,
    key: WasmPtr<u8, Array>,
    keylen: u32,
    value: WasmPtr<u64, Item>,
    function_context: &FunctionContext,
) -> WasiResult<()> {
    let key = key
        .get_utf8_string(vm_memory, keylen)
        .ok_or_else(|| WasiError::FailedToReadStringPointer("key".to_owned()))?;

    function_context
        .get_argument(key)
        .ok_or_else(|| WasiError::FailedToFindKey(key.to_string()))
        .and_then(|a| {
            let len = a.encoded_len();
            unsafe {
                value
                    .deref_mut(vm_memory)
                    .ok_or_else(WasiError::FailedToDerefPointer)
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
    function_context: &FunctionContext,
) -> WasiResult<()> {
    let key = key
        .get_utf8_string(vm_memory, keylen)
        .ok_or_else(|| WasiError::FailedToReadStringPointer("key".to_owned()))?;

    function_context
        .get_argument(key)
        .ok_or_else(|| WasiError::FailedToFindKey(key.to_string()))
        .and_then(|a| {
            value
                .as_byte_array_mut(&vm_memory, valuelen as usize)
                .ok_or_else(|| {
                    WasiError::ConversionError(
                        "Failed to convert provided input buffer to mut byte array.".to_owned(),
                    )
                })
                .and_then(|mut buff| {
                    a.encode(&mut buff)
                        .map_err(WasiError::FailedToEncodeProtobuf)
                })
        })
}

pub fn set_output(
    vm_memory: &Memory,
    val: WasmPtr<u8, Array>,
    vallen: u32,
) -> WasiResult<ReturnValue> {
    val.deref(vm_memory, 0, vallen)
        .ok_or_else(WasiError::FailedToDerefPointer)
        .and_then(|cells| {
            ReturnValue::decode(
                cells
                    .iter()
                    .map(|v| v.get())
                    .collect::<Vec<u8>>()
                    .as_slice(),
            )
            .map_err(WasiError::FailedToDecodeProtobuf)
        })
}

pub fn set_error(vm_memory: &Memory, msg: WasmPtr<u8, Array>, msglen: u32) -> WasiResult<String> {
    msg.get_utf8_string(vm_memory, msglen)
        .ok_or_else(|| WasiError::FailedToReadStringPointer("msg".to_owned()))
        .map(|s| s.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbk_protocols::functions::{ArgumentType, FunctionArgument};
    use gbk_protocols_test_helpers::function_attachment;

    use tempfile::Builder;
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
            &FunctionContext::new(
                vec![FunctionArgument {
                    name: "chorizo korvén".to_owned(),
                    r#type: ArgumentType::Bytes as i32,
                    value: vec![1, 2, 3],
                }],
                vec![],
            ),
        );

        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            WasiError::FailedToReadStringPointer(..)
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
            &FunctionContext::new(
                vec![FunctionArgument {
                    name: "chorizo korvén".to_owned(),
                    r#type: ArgumentType::Bytes as i32,
                    value: vec![1, 2, 3],
                }],
                vec![],
            ),
        );

        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), WasiError::FailedToFindKey(..)));

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
            &FunctionContext::new(vec![function_argument.clone()], vec![]),
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
            &FunctionContext::new(vec![function_argument.clone()], vec![]),
        );

        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            WasiError::FailedToDerefPointer()
        ));
    }

    #[test]
    fn test_get_input() {
        // testing invalid key pointer
        let mem = create_mem!();
        let key_ptr: WasmPtr<u8, Array> = WasmPtr::new(std::u32::MAX);
        let value_ptr: WasmPtr<u8, Array> = WasmPtr::new(0);

        let res = get_input(
            &mem,
            key_ptr,
            5 as u32,
            value_ptr,
            0 as u32,
            &FunctionContext::default(),
        );
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            WasiError::FailedToReadStringPointer(..)
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
            &FunctionContext::new(vec![], vec![]),
        );

        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), WasiError::FailedToFindKey(..)));

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
            &FunctionContext::new(vec![function_argument], vec![]),
        );

        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), WasiError::ConversionError(..)));

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
            &FunctionContext::new(vec![function_argument], vec![]),
        );

        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            WasiError::FailedToEncodeProtobuf(..)
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
            &FunctionContext::new(vec![function_argument], vec![]),
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
            WasiError::FailedToDerefPointer()
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
            WasiError::FailedToDecodeProtobuf(..)
        ));

        // Try with written pointer
        write_to_ptr(&ptr, &mem, &return_value_bytes);
        let res = set_output(&mem, ptr, encoded_len as u32);

        assert!(res.is_ok());
        assert_eq!(return_value, res.unwrap());
    }

    #[test]
    fn test_map_attachment() {
        let file = Builder::new()
            .prefix("my-temporary-note")
            .suffix(".txt")
            .tempfile()
            .unwrap();
        let file_path = file.path();
        std::fs::write(file_path, "hejhej").unwrap();
        let mem = create_mem!();
        let sandbox = Sandbox::new(Path::new("whatever")).unwrap();
        let fc = FunctionContext::new(
            vec![],
            vec![function_attachment!(
                format!("file://{}", file_path.display()),
                "sune",
                "e7cab684e3eb1b7c4652c363daf2ad88406b1f0e8a079a1cdc760f92b46f9afe",
                "bune.txt"
            )],
        );

        let attachment_name_ptr: WasmPtr<u8, Array> = WasmPtr::new(0);
        let attachment_name = "sune".to_owned();
        let attachment_bytes = attachment_name.as_bytes();
        write_to_ptr(&attachment_name_ptr, &mem, attachment_bytes);
        let attachment_bytes_len = attachment_bytes.len() as u32;
        let path_ptr = WasmPtr::new(attachment_bytes_len);
        let expected_path = "attachments/bune.txt";
        let expected_path_bytes_len = expected_path.as_bytes().len() as u32;
        // Test that we get the expected file path
        let res = map_attachment(
            &fc,
            &sandbox,
            &mem,
            attachment_name_ptr,
            attachment_bytes_len,
            path_ptr,
            expected_path_bytes_len,
        );
        assert!(res.is_ok());
        let return_path = path_ptr
            .deref(&mem, 0, expected_path_bytes_len)
            .unwrap()
            .iter()
            .map(|c| c.get())
            .collect::<Vec<u8>>();
        let return_path = std::str::from_utf8(&return_path).unwrap();

        assert_eq!(return_path, expected_path);

        // Test attachment with no filename specified
        let mem = create_mem!();
        let sandbox = Sandbox::new(Path::new("whatever")).unwrap();
        let fc = FunctionContext::new(
            vec![],
            vec![function_attachment!(
                format!("file://{}", file_path.display()),
                "sune",
                "e7cab684e3eb1b7c4652c363daf2ad88406b1f0e8a079a1cdc760f92b46f9afe"
            )],
        );
        let attachment_name_ptr: WasmPtr<u8, Array> = WasmPtr::new(0);
        let attachment_name = "sune".to_owned();
        let attachment_bytes = attachment_name.as_bytes();
        write_to_ptr(&attachment_name_ptr, &mem, attachment_bytes);
        let attachment_bytes_len = attachment_bytes.len() as u32;
        let path_ptr = WasmPtr::new(attachment_bytes_len);
        let expected_path = "attachments/sune";
        let expected_path_bytes_len = expected_path.as_bytes().len() as u32;
        let res = map_attachment(
            &fc,
            &sandbox,
            &mem,
            attachment_name_ptr,
            attachment_bytes_len,
            path_ptr,
            expected_path_bytes_len,
        );
        assert!(res.is_ok());
        let return_path = path_ptr
            .deref(&mem, 0, expected_path_bytes_len)
            .unwrap()
            .iter()
            .map(|c| c.get())
            .collect::<Vec<u8>>();
        let return_path = std::str::from_utf8(&return_path).unwrap();
        assert_eq!(return_path, expected_path);

        // Test non existing attachment
        let mem = create_mem!();
        let attachment_name_ptr: WasmPtr<u8, Array> = WasmPtr::new(0);
        let attachment_name = "not-a-thing".to_owned();
        let attachment_bytes = attachment_name.as_bytes();
        write_to_ptr(&attachment_name_ptr, &mem, attachment_bytes);
        let res = map_attachment(
            &fc,
            &sandbox,
            &mem,
            attachment_name_ptr,
            attachment_bytes_len,
            path_ptr,
            expected_path_bytes_len,
        );
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            WasiError::FailedToFindAttachment(_)
        ));

        // Test bad attachment name
        let mem = create_mem!();
        let attachment_name_ptr: WasmPtr<u8, Array> = WasmPtr::new(0);
        let res = map_attachment(
            &fc,
            &sandbox,
            &mem,
            attachment_name_ptr,
            (mem.size().bytes().0 + 1) as u32,
            path_ptr,
            expected_path_bytes_len,
        );
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            WasiError::FailedToReadStringPointer(_)
        ));

        // Test bad attachment transport
        let mem = create_mem!();
        let sandbox = Sandbox::new(Path::new("whatever")).unwrap();
        let fc = FunctionContext::new(
            vec![],
            vec![function_attachment!(
                "fule://din-mamma",
                "sune",
                "e7cab684e3eb1b7c4652c363daf2ad88406b1f0e8a079a1cdc760f92b46f9afe"
            )],
        );
        let attachment_name_ptr: WasmPtr<u8, Array> = WasmPtr::new(0);
        let attachment_name = "sune".to_owned();
        let attachment_bytes = attachment_name.as_bytes();
        write_to_ptr(&attachment_name_ptr, &mem, attachment_bytes);
        let attachment_bytes_len = attachment_bytes.len() as u32;
        let path_ptr = WasmPtr::new(attachment_bytes_len);
        let expected_path = "attachments/sune";
        let expected_path_bytes_len = expected_path.as_bytes().len() as u32;
        let res = map_attachment(
            &fc,
            &sandbox,
            &mem,
            attachment_name_ptr,
            attachment_bytes_len,
            path_ptr,
            expected_path_bytes_len,
        );
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            WasiError::FailedToMapAttachment(..)
        ));
    }

    #[test]
    fn test_get_attachment_path_len() {
        let mem = create_mem!();
        let fc = FunctionContext::new(
            vec![],
            vec![function_attachment!(
                "file://doesnt-matter",
                "sune",
                "e7cab684e3eb1b7c4652c363daf2ad88406b1f0e8a079a1cdc760f92b46f9afe",
                "rune.txt"
            )],
        );

        let attachment_name_ptr: WasmPtr<u8, Array> = WasmPtr::new(0);
        let attachment_name = "sune".to_owned();
        let attachment_bytes = attachment_name.as_bytes();
        write_to_ptr(&attachment_name_ptr, &mem, attachment_bytes);
        let attachment_bytes_len = attachment_bytes.len() as u32;
        let path_len_ptr: WasmPtr<u64, Item> = WasmPtr::new(attachment_bytes_len);
        let res = get_attachment_path_len(
            &fc,
            &mem,
            attachment_name_ptr,
            attachment_bytes_len,
            path_len_ptr,
        );
        assert!(res.is_ok());
        let path_len: u64 = path_len_ptr
            .deref(&mem)
            .map(|cell| cell.get() as u64)
            .unwrap();
        assert_eq!(path_len, "attachments/rune.txt".as_bytes().len() as u64);

        // Test non existing attachment
        let mem = create_mem!();
        let attachment_name_ptr: WasmPtr<u8, Array> = WasmPtr::new(0);
        let attachment_name = "not-a-thing".to_owned();
        let attachment_bytes = attachment_name.as_bytes();
        let path_len_ptr: WasmPtr<u64, Item> = WasmPtr::new(0);
        write_to_ptr(&attachment_name_ptr, &mem, attachment_bytes);
        let res = get_attachment_path_len(
            &fc,
            &mem,
            attachment_name_ptr,
            attachment_bytes_len,
            path_len_ptr,
        );

        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            WasiError::FailedToFindAttachment(_)
        ));

        // Test bad attachment name
        let mem = create_mem!();
        let attachment_name_ptr: WasmPtr<u8, Array> = WasmPtr::new(0);
        let res = get_attachment_path_len(
            &fc,
            &mem,
            attachment_name_ptr,
            (mem.size().bytes().0 + 1) as u32,
            path_len_ptr,
        );
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            WasiError::FailedToReadStringPointer(_)
        ));
    }
}
