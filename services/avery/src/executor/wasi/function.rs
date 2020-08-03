use std::{
    convert::TryInto,
    io::{Cursor, Write},
    path::{Path, PathBuf},
};

use super::error::{WasiError, WasiResult};
use flate2::read::GzDecoder;
use prost::Message;
use tar::Archive;

use super::{sandbox::Sandbox, WasmBuffer, WasmItemPtr, WasmString};
use crate::executor::{AttachmentDownload, FunctionContextExt};
use gbk_protocols::functions::{FunctionAttachment, FunctionContext, ReturnValue};

use slog::{info, Logger};

fn wasi_attachment_path_from_descriptor(attachment_data: &FunctionAttachment) -> String {
    // Manually joining paths to ensure we always get a valid path for WASI (no backslash)
    format!("attachments/{}", &attachment_data.name)
}

fn native_attachment_path_from_descriptor(
    attachment_data: &FunctionAttachment,
    sandbox: &Sandbox,
) -> PathBuf {
    sandbox.path().join(&attachment_data.name)
}

fn download_and_map_at(
    attachment_data: &FunctionAttachment,
    path: &Path,
    unpack: bool,
    logger: &Logger,
) -> WasiResult<()> {
    if !path.exists() {
        attachment_data
            .download()
            .map_err(|e| {
                WasiError::FailedToMapAttachment(attachment_data.name.to_owned(), Box::new(e))
            })
            .and_then(|data| {
                if unpack {
                    info!(
                        logger,
                        "Unpacking attachment {} at {}",
                        attachment_data.name,
                        path.display()
                    );
                    let mut ar = Archive::new(GzDecoder::new(Cursor::new(data)));
                    ar.unpack(path).map_err(|e| {
                        WasiError::FailedToUnpackAttachment(
                            attachment_data.name.to_owned(),
                            Box::new(e),
                        )
                    })
                } else {
                    info!(
                        logger,
                        "Mapping attachment {} at {}",
                        attachment_data.name,
                        path.display()
                    );
                    std::fs::write(path, data).map_err(|e| {
                        WasiError::FailedToMapAttachment(
                            attachment_data.name.to_owned(),
                            Box::new(e),
                        )
                    })
                }
            })?;
    }

    Ok(())
}

pub fn get_attachment_path_len(
    function_context: &FunctionContext,
    attachment_name: WasmString,
    path_len: WasmItemPtr<u32>,
) -> WasiResult<()> {
    let attachment_key: String = attachment_name
        .try_into()
        .map_err(|e| WasiError::FailedToReadStringPointer("attachment_name".to_owned(), e))?;

    let attachment_data = function_context
        .get_attachment(&attachment_key)
        .ok_or_else(|| WasiError::FailedToFindAttachment(attachment_key))?;
    path_len.set(
        wasi_attachment_path_from_descriptor(&attachment_data)
            .as_bytes()
            .len() as u32,
    )
}

pub fn map_attachment(
    function_context: &FunctionContext,
    sandbox: &Sandbox,
    attachment_name: WasmString,
    unpack: bool,
    path_buffer: &mut WasmBuffer,
    logger: &Logger,
) -> WasiResult<()> {
    let attachment_key: String = attachment_name
        .try_into()
        .map_err(|e| WasiError::FailedToReadStringPointer("attachment_key".to_owned(), e))?;

    let attachment_data = function_context
        .get_attachment(&attachment_key)
        .ok_or_else(|| WasiError::FailedToFindAttachment(attachment_key))?;

    download_and_map_at(
        &attachment_data,
        &native_attachment_path_from_descriptor(&attachment_data, &sandbox),
        unpack,
        logger,
    )?;

    path_buffer
        .write(&wasi_attachment_path_from_descriptor(&attachment_data).as_bytes())
        .map_err(WasiError::FailedToWriteBuffer)
        .map(|_bytes_written| ())
}

pub fn get_attachment_path_len_from_descriptor(
    attachment_descriptor: WasmBuffer,
    path_len: WasmItemPtr<u32>,
) -> WasiResult<()> {
    let fa = FunctionAttachment::decode(attachment_descriptor.buffer())
        .map_err(WasiError::FailedToDecodeProtobuf)?;

    path_len.set(wasi_attachment_path_from_descriptor(&fa).as_bytes().len() as u32)
}

pub fn map_attachment_from_descriptor(
    sandbox: &Sandbox,
    attachment_descriptor: WasmBuffer,
    unpack: bool,
    path_buffer: &mut WasmBuffer,
    logger: &Logger,
) -> WasiResult<()> {
    let fa = FunctionAttachment::decode(attachment_descriptor.buffer())
        .map_err(WasiError::FailedToDecodeProtobuf)?;

    download_and_map_at(
        &fa,
        &native_attachment_path_from_descriptor(&fa, &sandbox),
        unpack,
        logger,
    )?;

    path_buffer
        .write(&wasi_attachment_path_from_descriptor(&fa).as_bytes())
        .map_err(WasiError::FailedToWriteBuffer)
        .map(|_bytes_written| ())
}

pub fn get_input_len(
    key: WasmString,
    len: WasmItemPtr<u32>,
    function_context: &FunctionContext,
) -> WasiResult<()> {
    let key: String = key
        .try_into()
        .map_err(|e| WasiError::FailedToReadStringPointer("input key".to_owned(), e))?;

    function_context
        .get_argument(&key)
        .ok_or_else(|| WasiError::FailedToFindKey(key))
        .and_then(|a| len.set(a.encoded_len() as u32))
}

pub fn get_input(
    key: WasmString,
    value: &mut WasmBuffer,
    function_context: &FunctionContext,
) -> WasiResult<()> {
    let key: String = key
        .try_into()
        .map_err(|e| WasiError::FailedToReadStringPointer("input key".to_owned(), e))?;

    function_context
        .get_argument(&key)
        .ok_or_else(|| WasiError::FailedToFindKey(key))
        .and_then(|a| {
            a.encode(&mut value.buffer_mut())
                .map_err(WasiError::FailedToEncodeProtobuf)
        })
}

pub fn set_output(value: WasmBuffer) -> WasiResult<ReturnValue> {
    ReturnValue::decode(value.buffer()).map_err(WasiError::FailedToDecodeProtobuf)
}

pub fn set_error(msg: WasmString) -> WasiResult<String> {
    msg.try_into()
        .map_err(|e| WasiError::FailedToReadStringPointer("msg".to_owned(), e))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::convert::TryFrom;

    use gbk_protocols::functions::{ArgumentType, FunctionArgument};
    use gbk_protocols_test_helpers::function_attachment;

    use tempfile::Builder;
    use wasmer_runtime::{memory::Memory, WasmPtr};
    use wasmer_runtime::{types::MemoryDescriptor, units::Pages};

    macro_rules! create_mem {
        () => {{
            Memory::new(MemoryDescriptor::new(Pages(1), None, false).unwrap()).unwrap()
        }};
    }

    macro_rules! null_logger {
        () => {{
            slog::Logger::root(slog::Discard, slog::o!())
        }};
    }

    macro_rules! wasm_string {
        ($mem:expr, $offset:expr, $val:expr) => {{
            let s: &str = $val.as_ref();
            let byte_len = s.as_bytes().len();
            let mut buf = WasmBuffer::new($mem, WasmPtr::new($offset), byte_len as u32);
            buf.write_all(s.as_bytes()).unwrap();
            WasmString::new(buf)
        }};
    }

    macro_rules! invalid_wasm_string {
        ($mem: expr) => {{
            WasmString::new(WasmBuffer::new($mem, WasmPtr::new(std::u32::MAX), 1337u32))
        }};
    }

    macro_rules! out_buffer {
        ($mem: expr, $offset: expr, $size: expr) => {{
            WasmBuffer::new($mem, WasmPtr::new($offset), $size)
        }};
    }

    #[test]
    #[should_panic]
    fn test_bad_input_len_key() {
        let mem = create_mem!();
        get_input_len(
            invalid_wasm_string!(&mem),
            WasmItemPtr::new(&mem, WasmPtr::new(0)),
            &FunctionContext::new(
                vec![FunctionArgument {
                    name: "chorizo korvén".to_owned(),
                    r#type: ArgumentType::Bytes as i32,
                    value: vec![1, 2, 3],
                }],
                vec![],
            ),
        )
        .unwrap();
    }

    #[test]
    fn test_get_input_len() {
        // get non existant input
        let mem = create_mem!();
        let key = wasm_string!(&mem, 0, "inte chorizo korvén");
        let res = get_input_len(
            key.clone(),
            WasmItemPtr::new(
                &mem,
                WasmPtr::new(key.buffer_len() /* after the string in memory */),
            ),
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
        let key = wasm_string!(&mem, 0, "input1");
        let function_argument = FunctionArgument {
            name: "input1".to_owned(),
            r#type: ArgumentType::Bytes as i32,
            value: vec![1, 2, 3],
        };

        let out_len = WasmItemPtr::new(
            &mem,
            WasmPtr::new(
                key.buffer_len(), /* put it after the string in memory */
            ),
        );
        let res = get_input_len(
            key,
            out_len.clone(),
            &FunctionContext::new(vec![function_argument.clone()], vec![]),
        );
        assert!(res.is_ok());
        assert_eq!(
            function_argument.encoded_len(),
            out_len.get().unwrap() as usize
        );

        // get existing input with invalid pointer
        let mem = create_mem!();
        let key = wasm_string!(&mem, 0, "input1");

        let function_argument = FunctionArgument {
            name: "input1".to_owned(),
            r#type: ArgumentType::Bytes as i32,
            value: vec![1, 2, 3],
        };

        // creates a pointer that points beyond the end of memory
        let val = WasmItemPtr::new(&mem, WasmPtr::new(std::u32::MAX));
        let res = get_input_len(
            key,
            val,
            &FunctionContext::new(vec![function_argument], vec![]),
        );

        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            WasiError::FailedToDerefPointer()
        ));
    }

    #[test]
    fn test_get_input() {
        // testing failed to find key
        let mem = create_mem!();

        let res = get_input(
            wasm_string!(&mem, 0, "input1"),
            &mut out_buffer!(&mem, 0u32, 0u32), // no point in creating a valid buffer
            &FunctionContext::default(),
        );

        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), WasiError::FailedToFindKey(..)));

        // testing failed to encode protobuf
        let mem = create_mem!();
        let function_argument = FunctionArgument {
            name: "input1".to_owned(),
            r#type: ArgumentType::Bytes as i32,
            value: vec![1, 2, 3],
        };

        let encoded_len = function_argument.encoded_len();

        let key = wasm_string!(&mem, 0, "input1");
        let res = get_input(
            key.clone(),
            &mut out_buffer!(&mem, key.buffer_len(), (encoded_len - 1) as u32), // make buffer 1 too small
            &FunctionContext::new(vec![function_argument], vec![]),
        );

        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            WasiError::FailedToEncodeProtobuf(..)
        ));

        // testing getting valid input
        let mem = create_mem!();

        let function_argument = FunctionArgument {
            name: "input1".to_owned(),
            r#type: ArgumentType::Bytes as i32,
            value: vec![1, 2, 3],
        };

        let key = wasm_string!(&mem, 0, "input1");
        let encoded_len = function_argument.encoded_len();

        let mut reference_value = Vec::with_capacity(encoded_len);
        function_argument.encode(&mut reference_value).unwrap();

        let out_ptr = out_buffer!(&mem, key.buffer_len(), encoded_len as u32);
        let res = get_input(
            key,
            &mut out_ptr.clone(),
            &FunctionContext::new(vec![function_argument], vec![]),
        );

        assert!(res.is_ok());

        // check that the byte patterns are identical
        assert_eq!(reference_value, out_ptr.buffer());
    }

    #[test]
    fn test_set_output() {
        let mem = create_mem!();
        let return_value = ReturnValue {
            name: "sune".to_owned(),
            r#type: ArgumentType::Int as i32,
            value: vec![1, 2, 3, 4, 5, 6, 7, 8],
        };

        let mut buf = WasmBuffer::new(&mem, WasmPtr::new(0), return_value.encoded_len() as u32);
        return_value.encode(&mut buf.buffer_mut()).unwrap();

        let res = set_output(buf);

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
                "e7cab684e3eb1b7c4652c363daf2ad88406b1f0e8a079a1cdc760f92b46f9afe"
            )],
        );

        let attachment_name = wasm_string!(&mem, 0, "sune");
        let expected_path = "attachments/sune";
        let expected_path_bytes_len = expected_path.as_bytes().len() as u32;

        let mut out_path = out_buffer!(&mem, attachment_name.buffer_len(), expected_path_bytes_len);

        // Test that we get the expected file path
        let res = map_attachment(
            &fc,
            &sandbox,
            attachment_name,
            false,
            &mut out_path,
            &null_logger!(),
        );
        assert!(res.is_ok());
        assert_eq!(
            String::try_from(WasmString::new(out_path)).unwrap(),
            expected_path
        );

        // Test non existing attachment
        let mem = create_mem!();
        let res = map_attachment(
            &fc,
            &sandbox,
            wasm_string!(&mem, 0, "i-am-not-here"),
            false,
            &mut out_buffer!(&mem, 0, 0u32), // no point in having a valid buffer here
            &null_logger!(),
        );
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            WasiError::FailedToFindAttachment(_)
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

        let res = map_attachment(
            &fc,
            &sandbox,
            wasm_string!(&mem, 0, "sune"),
            false,
            &mut out_buffer!(&mem, 0, 0u32), // no point in having a valid buffer here
            &null_logger!(),
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
                "e7cab684e3eb1b7c4652c363daf2ad88406b1f0e8a079a1cdc760f92b46f9afe"
            )],
        );

        let attachment_name = wasm_string!(&mem, 0, "sune");
        let out_path_len = WasmItemPtr::new(&mem, WasmPtr::new(attachment_name.buffer_len()));
        let res = get_attachment_path_len(&fc, attachment_name, out_path_len.clone());

        assert!(res.is_ok());
        assert_eq!(
            out_path_len.get().unwrap(),
            b"attachments/sune".len() as u32
        );

        // Test non existing attachment
        let attachment_name = wasm_string!(&mem, 0, "i-am-not-there");
        let out_path_len = WasmItemPtr::new(&mem, WasmPtr::new(attachment_name.buffer_len()));
        let res = get_attachment_path_len(&fc, attachment_name, out_path_len);

        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            WasiError::FailedToFindAttachment(_)
        ));
    }
}
