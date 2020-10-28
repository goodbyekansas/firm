mod error;
mod function;
mod net;
mod process;
mod sandbox;

use std::{
    convert::TryFrom,
    io::{self, Read, Write},
    path::Path,
    str,
    str::Utf8Error,
    sync::{Arc, RwLock},
};

use slog::{info, o, Logger};

use wasmer_runtime::{
    compile, func, imports, types::ValueType, Array, Ctx, Func, Item, Memory, WasmPtr,
};
use wasmer_wasi::{generate_import_object_from_state, get_wasi_version, state::WasiState};

use crate::executor::{
    AttachmentDownload, ExecutorError, ExecutorParameters, FunctionExecutor, StreamExt,
};
use error::{ToErrorCode, WasiError};
use firm_types::{execution::Stream, functions::Attachment};
use process::StdIOConfig;
use sandbox::Sandbox;

/// Wrapper type that represents a buffer in WASI memory
///
/// Note that this actually does not allocate anything, it is
/// merely a view on memory
#[derive(Debug, Clone)]
pub struct WasmBuffer {
    memory: Memory,
    ptr: WasmPtr<u8, Array>,
    len: u32,
}

impl WasmBuffer {
    /// Create a new WASM Buffer
    ///
    /// `memory` is the WASI linear memory where the buffer resides
    /// `ptr` is the WasmPtr that points to the start of the buffer
    /// `len` is the size of the buffer in bytes
    pub fn new(memory: &Memory, ptr: WasmPtr<u8, Array>, len: u32) -> Self {
        Self {
            memory: memory.clone(),
            ptr,
            len,
        }
    }

    /// Get the length in bytes for this buffer
    pub fn len(&self) -> u32 {
        self.len
    }

    /// Get a view of the underlying data in this buffer
    pub fn buffer(&self) -> &[u8] {
        if self.ptr.offset() + self.len > self.memory.size().bytes().0 as u32 {
            panic!(
                "WASM buffer (offset: {}, size: {}) goes beyond the memory capacity ({})",
                self.ptr.offset(),
                self.len,
                self.memory.size().bytes().0
            );
        }

        let src_buf = unsafe {
            self.memory
                .view::<u8>()
                .as_ptr()
                .add(self.ptr.offset() as usize) as *const u8
        };
        let slice: &[u8] = unsafe { std::slice::from_raw_parts(src_buf, self.len as usize) };
        slice
    }

    /// Get a mutable view of the underlying data in this buffer
    pub fn buffer_mut(&mut self) -> &mut [u8] {
        if self.ptr.offset() + self.len > self.memory.size().bytes().0 as u32 {
            panic!(
                "WASM buffer (offset: {}, size: {}) goes beyond the memory capacity ({})",
                self.ptr.offset(),
                self.len,
                self.memory.size().bytes().0
            );
        }

        let tgt_buf = unsafe {
            self.memory
                .view::<u8>()
                .as_ptr()
                .add(self.ptr.offset() as usize) as *mut u8
        };
        let slice: &mut [u8] =
            unsafe { std::slice::from_raw_parts_mut(tgt_buf, self.len as usize) };
        slice
    }
}

/// Wrapper type around a `WasmBuffer` to
/// treat it as a String
#[derive(Debug, Clone)]
pub struct WasmString {
    buffer: WasmBuffer,
}

impl WasmString {
    /// Create a new WasmString from a `buffer`
    pub fn new(buffer: WasmBuffer) -> Self {
        Self { buffer }
    }

    /// Get the length of the string in bytes
    /// i.e. the size of the underlying buffer
    pub fn buffer_len(&self) -> u32 {
        self.buffer.len()
    }
}

impl TryFrom<WasmString> for String {
    type Error = Utf8Error;
    fn try_from(s: WasmString) -> Result<Self, Self::Error> {
        Ok(str::from_utf8(s.buffer.buffer())?.to_owned())
    }
}

/// Pointer to a single value in WASI/WASM memory
/// of type T (a Copy-type)
#[derive(Debug, Clone)]
pub struct WasmItemPtr<T: Copy + ValueType> {
    memory: Memory,
    ptr: WasmPtr<T, Item>,
}

impl<T: Copy + ValueType> WasmItemPtr<T> {
    /// Create a new ite pointer
    ///
    /// `memory` is the WASI linear memory where the value resides
    /// `ptr` is the WasmPtr that points to the value
    pub fn new(memory: &Memory, ptr: WasmPtr<T, Item>) -> Self {
        Self {
            memory: memory.clone(),
            ptr,
        }
    }

    #[cfg(test)] // only used in tests for now...
    pub fn get(&self) -> Option<T> {
        self.ptr.deref(&self.memory).map(|v| v.get())
    }

    /// Set the value at the memory pointed to by this
    /// pointer.
    ///
    /// Note: This will return a `WasiError` for
    /// invalid pointers.
    pub fn set(&self, val: T) -> Result<(), WasiError> {
        self.ptr
            .deref(&self.memory)
            .ok_or_else(WasiError::FailedToDerefPointer)
            .map(|v| v.set(val))
    }
}

impl Read for WasmBuffer {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        buf.clone_from_slice(self.buffer());
        Ok(self.len() as usize)
    }
}

impl Write for WasmBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buffer_mut().clone_from_slice(buf);
        Ok(self.len() as usize)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn execute_function(
    logger: Logger,
    function_name: &str,
    _entrypoint: &str,
    code: &[u8],
    arguments: Stream,
    attachments: Vec<Attachment>,
) -> Result<Stream, String> {
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
    info!(
        logger,
        "using sandbox attachments directory: {}",
        attachment_sandbox.path().display()
    );

    // create stdout and stderr
    let stdiofiles = sandbox
        .setup_stdio()
        .map_err(|e| format!("Failed to setup std IO files: {}", e))?;
    let std0 = stdiofiles
        .try_clone()
        .map_err(|e| format!("Failed to clone stdio files: {}", e))?;
    let std1 = stdiofiles
        .try_clone()
        .map_err(|e| format!("Failed to clone stdio files: {}", e))?;

    let wasi_state = WasiState::new(&format!("wasi-{}", function_name))
        .stdout(Box::new(stdiofiles.stdout))
        .stderr(Box::new(stdiofiles.stderr))
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

    // inject firm specific functions in the wasm state
    let v: Result<Stream, String> = Ok(Stream::new());
    let results = Arc::new(RwLock::new(v));
    let res = Arc::clone(&results);
    let res2 = Arc::clone(&results);
    let attachment_sandbox = Arc::new(attachment_sandbox);
    let attachment_sandbox2 = Arc::clone(&attachment_sandbox);

    let attachments0 = Arc::new(attachments);
    let attachments1 = Arc::clone(&attachments0);

    let arguments0 = Arc::new(arguments);
    let arguments1 = Arc::clone(&arguments0);

    let start_process_logger = logger.new(o!("scope" => "start_process"));
    let run_process_logger = logger.new(o!("scope" => "run_process"));
    let map_attachment_logger = logger.new(o!("scope" => "map_attachment"));
    let map_attachment_descriptor_logger = logger.new(o!("scope" => "map_attachment_descriptor"));

    let host_file_exists_closure =
        move |ctx: &mut Ctx, path: WasmPtr<u8, Array>, path_len: u32, exists: WasmPtr<u8, Item>| {
            String::try_from(WasmString::new(WasmBuffer::new(
                ctx.memory(0),
                path,
                path_len,
            )))
            .map_err(|e| WasiError::FailedToReadStringPointer("path".to_owned(), e))
            .and_then(|p| {
                let exists = WasmItemPtr::new(ctx.memory(0), exists);
                exists.set(Path::new(&p).exists() as u8)
            })
            .to_error_code()
        };

    let get_host_os =
        |ctx: &mut Ctx, os_name: WasmPtr<u8, Array>, len_written: WasmPtr<u32, Item>| {
            let len = std::env::consts::OS.len();
            WasmItemPtr::new(ctx.memory(0), len_written)
                .set(len as u32)
                .and_then(|_| {
                    // TODO: Proper error for OS names longer than 128 when we have
                    // better error marshalling.
                    let len = std::cmp::min(128, len);
                    WasmBuffer::new(ctx.memory(0), os_name, len as u32)
                        .write_all(&std::env::consts::OS.as_bytes()[..len])
                        .map_err(WasiError::FailedToWriteBuffer)
                })
                .to_error_code()
        };

    let firm_imports = imports! {
        "firm" => {
            "get_attachment_path_len" => func!(move |ctx: &mut Ctx, attachment_name: WasmPtr<u8, Array>, attachment_name_len: u32, path_len: WasmPtr<u32, Item>| {
                function::get_attachment_path_len(&attachments0,
                                                  WasmString::new(WasmBuffer::new(
                                                      ctx.memory(0),
                                                      attachment_name,
                                                      attachment_name_len)),
                                                  WasmItemPtr::new(ctx.memory(0), path_len)).to_error_code()
            }),
            "map_attachment" => func!(move |ctx: &mut Ctx, attachment_name: WasmPtr<u8, Array>, attachment_name_len: u32, unpack: u8, path_ptr: WasmPtr<u8, Array>, path_buffer_len: u32| {
                function::map_attachment(&attachments1,
                                         &attachment_sandbox,
                                         WasmString::new(
                                             WasmBuffer::new(
                                                 ctx.memory(0),
                                                 attachment_name,
                                                 attachment_name_len
                                             ),
                                         ),
                                         unpack!=0,
                                         &mut WasmBuffer::new(
                                             ctx.memory(0),
                                             path_ptr,
                                             path_buffer_len
                                         ),
                                         &map_attachment_logger).to_error_code()
            }),
            "get_attachment_path_len_from_descriptor" => func!(move |ctx: &mut Ctx, attachment_descriptor_ptr: WasmPtr<u8, Array>, attachment_descriptor_len: u32, path_len: WasmPtr<u32, Item>| {
                function::get_attachment_path_len_from_descriptor(
                    WasmBuffer::new(
                        ctx.memory(0),
                        attachment_descriptor_ptr,
                        attachment_descriptor_len),
                        WasmItemPtr::new(ctx.memory(0), path_len)).to_error_code()
            }),
            "map_attachment_from_descriptor" => func!(move |ctx: &mut Ctx, attachment_descriptor_ptr: WasmPtr<u8, Array>, attachment_descriptor_len: u32, unpack: u8, path_ptr: WasmPtr<u8, Array>, path_buffer_len: u32| {
                function::map_attachment_from_descriptor(
                    &attachment_sandbox2,
                    WasmBuffer::new(
                        ctx.memory(0),
                        attachment_descriptor_ptr,
                        attachment_descriptor_len,
                    ),
                    unpack!=0,
                    &mut WasmBuffer::new(
                        ctx.memory(0),
                        path_ptr,
                        path_buffer_len
                    ),
                    &map_attachment_descriptor_logger).to_error_code()
            }),
            "host_path_exists" => func!(host_file_exists_closure),
            "get_host_os" => func!(get_host_os),
            "start_host_process" => func!(move |ctx: &mut Ctx, s: WasmPtr<u8, Array>, len: u32, pid_out: WasmPtr<u64, Item>| {
                StdIOConfig::new(&std0.stdout.inner, &std0.stderr.inner)
                .map_or_else(
                    |e| WasiError::FailedToSetupStdIO(e).into(),
                    |stdioconfig| process::start_process(
                        &start_process_logger,
                        &sandboxes,
                        stdioconfig,
                        WasmBuffer::new(ctx.memory(0), s, len),
                        WasmItemPtr::new(ctx.memory(0), pid_out),
                    ).to_error_code()
                )
            }),

            "run_host_process" => func!(move |ctx: &mut Ctx, s: WasmPtr<u8, Array>, len: u32, exit_code_out: WasmPtr<i32, Item>| {
                StdIOConfig::new(&std1.stdout.inner, &std1.stderr.inner)
                .map_or_else(
                    |e| WasiError::FailedToSetupStdIO(e).into(),
                    |stdioconfig| process::run_process(
                        &run_process_logger,
                        &sandboxes2,
                        stdioconfig,
                        WasmBuffer::new(ctx.memory(0), s, len),
                        WasmItemPtr::new(ctx.memory(0), exit_code_out),
                    ).to_error_code()
                )
            }),

            "get_input_len" => func!(move |ctx: &mut Ctx, key: WasmPtr<u8, Array>, keylen: u32, value: WasmPtr<u32, Item>| {
                function::get_input_len(
                    WasmString::new(WasmBuffer::new(ctx.memory(0), key, keylen)),
                    WasmItemPtr::new(ctx.memory(0), value), &arguments0).to_error_code()
            }),

            "get_input" => func!(move |ctx: &mut Ctx, key: WasmPtr<u8, Array>, keylen: u32, value: WasmPtr<u8, Array>, valuelen: u32| {
                function::get_input(
                    WasmString::new(WasmBuffer::new(ctx.memory(0), key, keylen)),
                    &mut WasmBuffer::new(ctx.memory(0), value, valuelen),
                    &arguments1).to_error_code()
            }),

            "set_output" => func!(move |ctx: &mut Ctx, key: WasmPtr<u8, Array>, keylen: u32, val: WasmPtr<u8, Array>, vallen: u32| {
                function::set_output(WasmString::new(WasmBuffer::new(ctx.memory(0), key, keylen)),
                WasmBuffer::new(ctx.memory(0), val, vallen)).and_then(|v| {
                    res.write().map(|mut current_stream_or_error| {
                        if let Ok(current_stream) = current_stream_or_error.as_mut() {
                            current_stream.merge(v);
                        }
                    }).map_err(|e| {WasiError::Unknown(format!("{}", e))})
                }).to_error_code()
            }),

            "set_error" => func!(move |ctx: &mut Ctx, msg: WasmPtr<u8, Array>, msglen: u32| {
                function::set_error(WasmString::new(WasmBuffer::new(ctx.memory(0), msg, msglen))).and_then(|v| {
                    res2.write().map(|mut current_stream_or_error| {
                        *current_stream_or_error = Err(v);
                    }).map_err(|e| {WasiError::Unknown(format!("{}", e))})
                }).to_error_code()
            }),

            "connect" => func!(move |ctx: &mut Ctx, addr: WasmPtr<u8, Array>, addr_len: u32, fd_out: WasmPtr<u32, Item>| {
                let mem = ctx.memory(0).clone();
                let state = unsafe { wasmer_wasi::state::get_wasi_state(ctx) };
                net::connect(&mut state.fs, WasmString::new(WasmBuffer::new(&mem, addr, addr_len)), WasmItemPtr::new(&mem, fd_out)).to_error_code()
            }),
        },
    };

    let mut import_object = generate_import_object_from_state(wasi_state, wasi_version);
    import_object.extend(firm_imports);

    let instance = module
        .instantiate(&import_object)
        .map_err(|e| format!("failed to instantiate WASI module: {}", e))?;

    let entry_function: Func<(), ()> = instance
        .exports
        .get(ENTRY)
        .map_err(|e| format!("Failed to resolve entrypoint {}: {}", ENTRY, e))?;

    entry_function
        .call()
        .map_err(|e| format!("Failed to call entrypoint function {}: {}", ENTRY, e))?;

    results
        .read()
        .map_err(|e| format!("Failed to read function results: {}", e))
        .and_then(|results| results.clone()) // TODO: Smart clone for stream
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
        executor_context: ExecutorParameters,
        arguments: Stream,
        attachments: Vec<Attachment>,
    ) -> Result<Result<Stream, String>, ExecutorError> {
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
            arguments,
            attachments,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use firm_protocols_test_helpers::{code_file, stream};

    macro_rules! null_logger {
        () => {{
            slog::Logger::root(slog::Discard, o!())
        }};
    }

    #[test]
    fn test_execution() {
        let executor = WasiExecutor::new(null_logger!());
        let res = executor.execute(
            ExecutorParameters {
                function_name: "hello-world".to_owned(),
                entrypoint: "could-be-anything".to_owned(),
                code: Some(code_file!(include_bytes!("hello.wasm"))),
                arguments: std::collections::HashMap::new(),
            },
            stream!(),
            vec![],
        );

        assert!(res.is_ok());
        assert!(res.unwrap().is_ok());
    }
}
