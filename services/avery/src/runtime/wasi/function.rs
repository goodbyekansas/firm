use std::{
    convert::TryInto,
    fs::File,
    io::Write,
    path::{Path, PathBuf},
};

use flate2::read::GzDecoder;
use futures::TryFutureExt;
use prost::Message;
use tar::Archive;

use super::{
    api::{WasmBuffer, WasmItemPtr, WasmString},
    error::{WasiError, WasiResult},
    sandbox::Sandbox,
};
use crate::{
    auth::AuthenticationSource,
    channels::{ChannelReader, ChannelSet, ChannelWriter},
    executor::AttachmentDownload,
    runtime::FunctionDirectory,
};
use firm_types::functions::{channel::Value as ProtoValue, Attachment, Channel as ProtoChannel};

use slog::{info, Logger};

fn wasi_attachment_path_from_descriptor(attachment_data: &Attachment) -> String {
    // Manually joining paths to ensure we always get a valid path for WASI (no backslash)
    format!("attachments/{}", &attachment_data.name)
}

fn native_attachment_path_from_descriptor(
    attachment_data: &Attachment,
    sandbox: &Sandbox,
) -> PathBuf {
    sandbox.host_path().join(&attachment_data.name)
}

pub struct DownloadAttachmentContext<'a> {
    pub function_dir: &'a FunctionDirectory,
    pub auth: &'a dyn AuthenticationSource,
}

async fn download_and_map_at(
    attachment_data: &Attachment,
    download_ctx: DownloadAttachmentContext<'_>,
    path: &Path,
    unpack: bool,
    logger: &Logger,
) -> WasiResult<()> {
    if !path.exists() {
        attachment_data
            .download_cached(
                download_ctx.function_dir.attachments_path(),
                download_ctx.auth,
            )
            .await
            .map_err(|e| {
                WasiError::FailedToMapAttachment(attachment_data.name.to_owned(), Box::new(e))
            })
            .and_then(|downloaded_path| {
                if unpack {
                    info!(
                        logger,
                        "Unpacking attachment {} at {}",
                        attachment_data.name,
                        path.display()
                    );
                    let mut ar = Archive::new(GzDecoder::new(
                        File::open(downloaded_path).map_err(|e| {
                            WasiError::FailedToUnpackAttachment(
                                attachment_data.name.to_owned(),
                                Box::new(e),
                            )
                        })?,
                    ));
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

                    std::fs::hard_link(downloaded_path, path).map_err(|e| {
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
    attachments: &[Attachment],
    attachment_name: WasmString,
    path_len: WasmItemPtr<u32>,
) -> WasiResult<()> {
    let attachment_key: String = attachment_name
        .try_into()
        .map_err(|e| WasiError::FailedToReadStringPointer("attachment_name".to_owned(), e))?;

    let attachment_data = attachments
        .iter()
        .find(|a| a.name == attachment_key)
        .ok_or(WasiError::FailedToFindAttachment(attachment_key))?;
    path_len.set(
        wasi_attachment_path_from_descriptor(attachment_data)
            .as_bytes()
            .len() as u32,
    )
}

pub async fn map_attachment(
    attachments: &[Attachment],
    download_ctx: DownloadAttachmentContext<'_>,
    sandbox: &Sandbox,
    attachment_name: WasmString,
    unpack: bool,
    path_buffer: &mut WasmBuffer,
    logger: &Logger,
) -> WasiResult<()> {
    let attachment_key: String = attachment_name
        .try_into()
        .map_err(|e| WasiError::FailedToReadStringPointer("attachment_key".to_owned(), e))?;

    let attachment_data = attachments
        .iter()
        .find(|a| a.name == attachment_key)
        .ok_or(WasiError::FailedToFindAttachment(attachment_key))?;

    download_and_map_at(
        attachment_data,
        download_ctx,
        &native_attachment_path_from_descriptor(attachment_data, sandbox),
        unpack,
        logger,
    )
    .await?;

    path_buffer
        .write(wasi_attachment_path_from_descriptor(attachment_data).as_bytes())
        .map_err(WasiError::FailedToWriteBuffer)
        .map(|_bytes_written| ())
}

pub fn get_attachment_path_len_from_descriptor(
    attachment_descriptor: WasmBuffer,
    path_len: WasmItemPtr<u32>,
) -> WasiResult<()> {
    let fa = Attachment::decode(attachment_descriptor.buffer())
        .map_err(WasiError::FailedToDecodeProtobuf)?;

    path_len.set(wasi_attachment_path_from_descriptor(&fa).as_bytes().len() as u32)
}

pub async fn map_attachment_from_descriptor(
    sandbox: &Sandbox,
    attachment_descriptor: WasmBuffer,
    download_ctx: DownloadAttachmentContext<'_>,
    unpack: bool,
    path_buffer: &mut WasmBuffer,
    logger: &Logger,
) -> WasiResult<()> {
    let fa = Attachment::decode(attachment_descriptor.buffer())
        .map_err(WasiError::FailedToDecodeProtobuf)?;

    download_and_map_at(
        &fa,
        download_ctx,
        &native_attachment_path_from_descriptor(&fa, sandbox),
        unpack,
        logger,
    )
    .await?;

    path_buffer
        .write(wasi_attachment_path_from_descriptor(&fa).as_bytes())
        .map_err(WasiError::FailedToWriteBuffer)
        .map(|_bytes_written| ())
}

// TODO: Note that in order to get data to wasi we copy it twice. We
// drag the data out of the channel and then convert it into a proto
// type (this is where copy happens) to just get the length of the
// encoded data. After that we get the data in get_input which will
// copy the data again... Not very effective.
pub async fn get_input_len(
    channel_name: WasmString,
    len: WasmItemPtr<u32>,
    read_length: u32,
    reader: &ChannelSet<ChannelReader>,
) -> WasiResult<()> {
    // Get a new reader from the reader so we get a new seek position
    // for each channel in the channel set. This ensures we get the
    // same values later when we do get_input where we want to move
    // the actual seek position.
    let reader = reader.reader();

    futures::future::ready(
        channel_name
            .try_into()
            .map_err(|e| WasiError::FailedToReadStringPointer("input channel".to_owned(), e))
            .and_then(|channel_name: String| {
                reader
                    .channel(&channel_name)
                    .ok_or(WasiError::FailedToFindChannel(channel_name))
            }),
    )
    .and_then(|channel| async { Ok(channel.read(read_length as usize).await) })
    .await
    .and_then(|data_view| len.set(data_view.to_proto_channel().encoded_len() as u32))
}

pub async fn get_input(
    channel_name: WasmString,
    value: &mut WasmBuffer,
    read_length: u32,
    reader: &ChannelSet<ChannelReader>,
) -> WasiResult<()> {
    futures::future::ready(
        channel_name
            .try_into()
            .map_err(|e| WasiError::FailedToReadStringPointer("input channel".to_owned(), e))
            .and_then(|channel_name: String| {
                reader
                    .channel(&channel_name)
                    .ok_or(WasiError::FailedToFindChannel(channel_name))
            }),
    )
    .and_then(|channel| async { Ok(channel.read(read_length as usize).await) })
    .await
    .and_then(|data_view| {
        data_view
            .to_proto_channel()
            .encode(&mut value.buffer_mut())
            .map_err(WasiError::FailedToEncodeProtobuf)
    })
}

pub async fn set_output(
    channel_name: WasmString,
    value: WasmBuffer,
    writer: &mut ChannelSet<ChannelWriter>,
) -> WasiResult<()> {
    futures::future::ready(
        channel_name
            .try_into()
            .map_err(|e| WasiError::FailedToReadStringPointer("output channel".to_owned(), e))
            .and_then(|channel_name: String| {
                ProtoChannel::decode(value.buffer())
                    .map(|proto_channel| (channel_name, proto_channel))
                    .map_err(WasiError::FailedToDecodeProtobuf)
            }),
    )
    .and_then(|(channel_name, proto_channel)| async move {
        match proto_channel.value {
            Some(ProtoValue::Strings(s)) => writer
                .append_channel(&channel_name, s.values.as_slice())
                .await
                .map_err(|e| {
                    WasiError::FailedToAppendToOutputChannel(channel_name.to_owned(), e.to_string())
                }),
            Some(ProtoValue::Integers(i)) => writer
                .append_channel(&channel_name, i.values.as_slice())
                .await
                .map_err(|e| {
                    WasiError::FailedToAppendToOutputChannel(channel_name.to_owned(), e.to_string())
                }),
            Some(ProtoValue::Floats(f)) => writer
                .append_channel(&channel_name, f.values.as_slice())
                .await
                .map_err(|e| {
                    WasiError::FailedToAppendToOutputChannel(channel_name.to_owned(), e.to_string())
                }),
            Some(ProtoValue::Booleans(b)) => writer
                .append_channel(&channel_name, b.values.as_slice())
                .await
                .map_err(|e| {
                    WasiError::FailedToAppendToOutputChannel(channel_name.to_owned(), e.to_string())
                }),
            Some(ProtoValue::Bytes(b)) => writer
                .append_channel(&channel_name, b.values.as_slice())
                .await
                .map_err(|e| {
                    WasiError::FailedToAppendToOutputChannel(channel_name.to_owned(), e.to_string())
                }),
            None => Err(WasiError::FailedToAppendToOutputChannel(
                channel_name.to_owned(),
                String::from("Tried to set a None value."),
            )),
        }
    })
    .await
}

pub fn set_error(msg: WasmString) -> WasiResult<String> {
    msg.try_into()
        .map_err(|e| WasiError::FailedToReadStringPointer("msg".to_owned(), e))
}

#[cfg(test)]
mod tests {
    use crate::auth::AuthService;

    use super::*;

    use std::convert::TryFrom;

    use crate::channel_writer;
    use firm_types::{attachment, stream::ToChannel};
    use tempfile::Builder;
    use wasmer::{Memory, MemoryType, Store, WasmPtr};

    macro_rules! create_mem {
        () => {{
            let store = Store::default();
            Memory::new(&store, MemoryType::new(1, None, false)).unwrap()
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

    macro_rules! function_directory {
        ($dir:expr) => {{
            FunctionDirectory::new($dir, "function", "0.1.0", "checksum", "execution-id").unwrap()
        }};
    }

    #[tokio::test]
    #[should_panic]
    async fn test_bad_input_len_key() {
        let channel_set = channel_writer!({"chorizo korvén" => firm_types::functions::ChannelType::Int | [1u8, 2u8, 3u8]});
        let mem = create_mem!();
        get_input_len(
            invalid_wasm_string!(&mem),
            WasmItemPtr::new(&mem, WasmPtr::new(0)),
            1,
            &channel_set.reader(),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_get_input_len() {
        let channel_set = channel_writer!({"chorizo korvén" => firm_types::functions::ChannelType::Int | [1u8, 2u8, 3u8]});
        // get non existant input
        let mem = create_mem!();
        let key = wasm_string!(&mem, 0, "inte chorizo korvén");
        let res = get_input_len(
            key.clone(),
            WasmItemPtr::new(
                &mem,
                WasmPtr::new(key.buffer_len() /* after the string in memory */),
            ),
            1,
            &channel_set.reader(),
        )
        .await;

        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            WasiError::FailedToFindChannel(..)
        ));

        // TODO: Figure out what this is actually testing.
        // get existing input (how could this ever work!?)
        let mem = create_mem!();
        let key = wasm_string!(&mem, 0, "input1");
        let channel_set = channel_writer!({"input1" => firm_types::functions::ChannelType::Int | [1u8, 2u8, 3u8]});

        let out_len = WasmItemPtr::new(
            &mem,
            WasmPtr::new(
                key.buffer_len(), /* put it after the string in memory */
            ),
        );

        let res = get_input_len(key, out_len.clone(), 3, &channel_set.reader()).await;
        assert!(res.is_ok());
        assert_eq!(
            channel_set
                .channel("input1")
                .unwrap()
                .to_proto_channel()
                .await
                .encoded_len(),
            out_len.get().unwrap() as usize
        );

        // get existing input with invalid pointer
        let mem = create_mem!();
        let key = wasm_string!(&mem, 0, "input1");

        // creates a pointer that points beyond the end of memory
        let val = WasmItemPtr::new(&mem, WasmPtr::new(std::u32::MAX));
        let res = get_input_len(key, val, 1, &channel_set.reader()).await;

        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            WasiError::FailedToDerefPointer()
        ));
    }

    #[tokio::test]
    async fn test_get_input() {
        // testing failed to find key
        let mem = create_mem!();
        let channel_set = ChannelSet::default();

        let res = get_input(
            wasm_string!(&mem, 0, "input1"),
            &mut out_buffer!(&mem, 0u32, 0u32), // no point in creating a valid buffer
            5,
            &channel_set.reader(),
        )
        .await;

        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            WasiError::FailedToFindChannel(..)
        ));

        // testing failed to encode protobuf
        let mem = create_mem!();
        let channel_set = channel_writer!({"input1" => firm_types::functions::ChannelType::Int | [1u8, 2u8, 3u8]});

        let encoded_len = channel_set
            .channel("input1")
            .unwrap()
            .to_proto_channel()
            .await
            .encoded_len();

        let key = wasm_string!(&mem, 0, "input1");
        let res = get_input(
            key.clone(),
            &mut out_buffer!(&mem, key.buffer_len(), (encoded_len - 1) as u32), // make buffer 1 too small
            3,
            &channel_set.reader(),
        )
        .await;

        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            WasiError::FailedToEncodeProtobuf(..)
        ));

        // testing getting valid input
        let mem = create_mem!();

        let channel_set = channel_writer!({"input1" => firm_types::functions::ChannelType::Int | [1u8, 2u8, 3u8]});

        let key = wasm_string!(&mem, 0, "input1");

        let function_argument = channel_set
            .channel("input1")
            .unwrap()
            .to_proto_channel()
            .await;
        let encoded_len = function_argument.encoded_len();
        let mut reference_value = Vec::with_capacity(encoded_len);
        function_argument.encode(&mut reference_value).unwrap();

        let out_ptr = out_buffer!(&mem, key.buffer_len(), encoded_len as u32);
        let res = get_input(key, &mut out_ptr.clone(), 3, &channel_set.reader()).await;

        assert!(res.is_ok());

        // check that the byte patterns are identical
        assert_eq!(reference_value, out_ptr.buffer());
    }

    #[tokio::test]
    async fn test_set_output() {
        let mem = create_mem!();

        let return_values = vec![1u8, 2u8, 3u8, 4u8, 5u8, 6u8, 7u8, 8u8].to_channel();
        let name = wasm_string!(&mem, 0, "sune");
        let mut expected_buf = WasmBuffer::new(
            &mem,
            WasmPtr::new(name.buffer_len()),
            return_values.encoded_len() as u32,
        );
        return_values
            .encode(&mut expected_buf.buffer_mut())
            .unwrap();

        let bytes: Vec<u8> = Vec::new(); // TODO: Inline this in some way.
        let mut channel_set =
            channel_writer!({"sune" => firm_types::functions::ChannelType::Bytes | bytes});
        let res = set_output(name.clone(), expected_buf, &mut channel_set).await;
        channel_set.close_all_channels();
        assert!(res.is_ok());

        let values = channel_set
            .read_channel("sune", 10)
            .await
            .unwrap()
            .to_proto_channel();

        assert_eq!(return_values, values);
    }

    #[tokio::test]
    async fn test_map_attachment() {
        let file = Builder::new()
            .prefix("my-temporary-note")
            .suffix(".txt")
            .tempfile()
            .unwrap();
        let file_path = file.path();
        std::fs::write(file_path, "hejhej").unwrap();
        let mem = create_mem!();
        let tmp_dir = tempfile::tempdir().unwrap();
        let sandbox = Sandbox::new(tmp_dir.path(), Path::new("whatever")).unwrap();
        let attachments = vec![attachment!(
            format!("file://{}", file_path.display()),
            "sune",
            "e7cab684e3eb1b7c4652c363daf2ad88406b1f0e8a079a1cdc760f92b46f9afe"
        )];

        let attachment_name = wasm_string!(&mem, 0, "sune");
        let expected_path = "attachments/sune";
        let expected_path_bytes_len = expected_path.as_bytes().len() as u32;

        let mut out_path = out_buffer!(&mem, attachment_name.buffer_len(), expected_path_bytes_len);
        let function_dir = tempfile::tempdir().unwrap();

        // Test that we get the expected file path
        let res = map_attachment(
            &attachments,
            DownloadAttachmentContext {
                function_dir: &function_directory!(function_dir.path()),
                auth: &AuthService::default(),
            },
            &sandbox,
            attachment_name,
            false,
            &mut out_path,
            &null_logger!(),
        )
        .await;
        assert!(res.is_ok());
        assert_eq!(
            String::try_from(WasmString::new(out_path)).unwrap(),
            expected_path
        );

        // Test non existing attachment
        let mem = create_mem!();
        let res = map_attachment(
            &attachments,
            DownloadAttachmentContext {
                function_dir: &function_directory!(function_dir.path()),
                auth: &AuthService::default(),
            },
            &sandbox,
            wasm_string!(&mem, 0, "i-am-not-here"),
            false,
            &mut out_buffer!(&mem, 0, 0u32), // no point in having a valid buffer here
            &null_logger!(),
        )
        .await;
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            WasiError::FailedToFindAttachment(_)
        ));

        // Test bad attachment transport
        let mem = create_mem!();
        let sandbox = Sandbox::new(tmp_dir.path(), Path::new("whatever")).unwrap();
        let attachments = vec![attachment!(
            "fule://din-mamma",
            "rune",
            "e7cab684e3eb1b7c4652c363daf2ad88406b1f0e8a079a1cdc760f92b46f9afe"
        )];

        let res = map_attachment(
            &attachments,
            DownloadAttachmentContext {
                function_dir: &function_directory!(function_dir.path()),
                auth: &AuthService::default(),
            },
            &sandbox,
            wasm_string!(&mem, 0, "rune"),
            false,
            &mut out_buffer!(&mem, 0, 0u32), // no point in having a valid buffer here
            &null_logger!(),
        )
        .await;
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            WasiError::FailedToMapAttachment(..)
        ));
    }

    #[test]
    fn test_get_attachment_path_len() {
        let mem = create_mem!();
        let attachments = vec![attachment!(
            "file://doesnt-matter",
            "sune",
            "e7cab684e3eb1b7c4652c363daf2ad88406b1f0e8a079a1cdc760f92b46f9afe"
        )];

        let attachment_name = wasm_string!(&mem, 0, "sune");
        let out_path_len = WasmItemPtr::new(&mem, WasmPtr::new(attachment_name.buffer_len()));
        let res = get_attachment_path_len(&attachments, attachment_name, out_path_len.clone());

        assert!(res.is_ok());
        assert_eq!(
            out_path_len.get().unwrap(),
            b"attachments/sune".len() as u32
        );

        // Test non existing attachment
        let attachment_name = wasm_string!(&mem, 0, "i-am-not-there");
        let out_path_len = WasmItemPtr::new(&mem, WasmPtr::new(attachment_name.buffer_len()));
        let res = get_attachment_path_len(&attachments, attachment_name, out_path_len);

        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            WasiError::FailedToFindAttachment(_)
        ));
    }
}
