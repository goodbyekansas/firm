use std::{convert::TryFrom, io, io::Read, io::Write, str::Utf8Error, sync::Arc, sync::Mutex};

use crate::{auth::AuthService, runtime::FunctionDirectory};

use super::{output::Output, sandbox::Sandbox, WasiError};
use firm_types::functions::{Attachment, Stream};
use slog::Logger;
use wasmer::{Array, HostEnvInitError, Instance, Item, Memory, ValueType, WasmPtr, WasmerEnv};
use wasmer_wasi::WasiEnv;

pub mod host {
    use super::{ApiState, WasmBuffer, WasmItemPtr, WasmString};
    use crate::runtime::wasi::{
        error::{ToErrorCode, WasiError},
        net, process,
    };
    use std::{convert::TryFrom, io::Write, path::Path};
    use wasmer::{Array, Item, WasmPtr};

    pub fn path_exists(
        api_state: &ApiState,
        path: WasmPtr<u8, Array>,
        path_len: u32,
        exists: WasmPtr<u8, Item>,
    ) -> u32 {
        String::try_from(WasmString::new(WasmBuffer::new(
            api_state.wasi_env.memory(),
            path,
            path_len,
        )))
        .map_err(|e| WasiError::FailedToReadStringPointer("path".to_owned(), e))
        .and_then(|p| {
            let exists = WasmItemPtr::new(api_state.wasi_env.memory(), exists);
            exists.set(Path::new(&p).exists() as u8)
        })
        .to_error_code()
    }

    pub fn get_os(
        api_state: &ApiState,
        os_name: WasmPtr<u8, Array>,
        len_written: WasmPtr<u32, Item>,
    ) -> u32 {
        let len = std::env::consts::OS.len();
        WasmItemPtr::new(api_state.wasi_env.memory(), len_written)
            .set(len as u32)
            .and_then(|_| {
                // TODO: Proper error for OS names longer than 128 when we have
                // better error marshalling.
                let len = std::cmp::min(128, len);
                WasmBuffer::new(api_state.wasi_env.memory(), os_name, len as u32)
                    .write_all(&std::env::consts::OS.as_bytes()[..len])
                    .map_err(WasiError::FailedToWriteBuffer)
            })
            .to_error_code()
    }

    pub fn start_process(
        api_state: &ApiState,
        s: WasmPtr<u8, Array>,
        len: u32,
        pid_out: WasmPtr<u64, Item>,
    ) -> u32 {
        process::start_process(
            &api_state.logger,
            &[
                api_state.sandbox.clone(),
                api_state.attachment_sandbox.clone(),
                api_state.cache_sandbox.clone(),
            ],
            &api_state.stdout,
            &api_state.stderr,
            WasmBuffer::new(api_state.wasi_env.memory(), s, len),
            WasmItemPtr::new(api_state.wasi_env.memory(), pid_out),
        )
        .to_error_code()
    }

    pub fn run_process(
        api_state: &ApiState,
        s: WasmPtr<u8, Array>,
        len: u32,
        exit_code_out: WasmPtr<i32, Item>,
    ) -> u32 {
        process::run_process(
            &api_state.logger,
            &[
                api_state.sandbox.clone(),
                api_state.attachment_sandbox.clone(),
                api_state.cache_sandbox.clone(),
            ],
            &api_state.stdout,
            &api_state.stderr,
            WasmBuffer::new(api_state.wasi_env.memory(), s, len),
            WasmItemPtr::new(api_state.wasi_env.memory(), exit_code_out),
        )
        .to_error_code()
    }

    pub fn socket_connect(
        api_state: &ApiState,
        addr: WasmPtr<u8, Array>,
        addr_len: u32,
        fd_out: WasmPtr<i32, Item>,
    ) -> u32 {
        net::connect(
            &mut api_state.wasi_env.state().fs,
            WasmString::new(WasmBuffer::new(api_state.wasi_env.memory(), addr, addr_len)),
            WasmItemPtr::new(api_state.wasi_env.memory(), fd_out),
        )
        .to_error_code()
    }
}

pub mod connections {
    use super::{ApiState, WasmBuffer, WasmItemPtr, WasmString};
    use crate::runtime::wasi::{
        error::{ToErrorCode, WasiError},
        function,
    };
    use firm_types::stream::StreamExt;
    use wasmer::{Array, Item, WasmPtr};

    pub fn get_input_len(
        api_state: &ApiState,
        key: WasmPtr<u8, Array>,
        keylen: u32,
        value: WasmPtr<u32, Item>,
    ) -> u32 {
        function::get_input_len(
            WasmString::new(WasmBuffer::new(api_state.wasi_env.memory(), key, keylen)),
            WasmItemPtr::new(api_state.wasi_env.memory(), value),
            &api_state.arguments,
        )
        .to_error_code()
    }

    pub fn get_input(
        api_state: &ApiState,
        key: WasmPtr<u8, Array>,
        keylen: u32,
        value: WasmPtr<u8, Array>,
        valuelen: u32,
    ) -> u32 {
        function::get_input(
            WasmString::new(WasmBuffer::new(api_state.wasi_env.memory(), key, keylen)),
            &mut WasmBuffer::new(api_state.wasi_env.memory(), value, valuelen),
            &api_state.arguments,
        )
        .to_error_code()
    }

    pub fn set_output(
        api_state: &ApiState,
        key: WasmPtr<u8, Array>,
        keylen: u32,
        val: WasmPtr<u8, Array>,
        vallen: u32,
    ) -> u32 {
        function::set_output(
            WasmString::new(WasmBuffer::new(api_state.wasi_env.memory(), key, keylen)),
            WasmBuffer::new(api_state.wasi_env.memory(), val, vallen),
        )
        .and_then(|v| {
            api_state
                .results
                .lock()
                .map(|mut current_stream| {
                    current_stream.merge(v);
                })
                .map_err(|e| WasiError::Unknown(format!("{}", e)))
        })
        .to_error_code()
    }

    pub fn set_error(api_state: &ApiState, msg: WasmPtr<u8, Array>, msglen: u32) -> u32 {
        function::set_error(WasmString::new(WasmBuffer::new(
            api_state.wasi_env.memory(),
            msg,
            msglen,
        )))
        .and_then(|v| {
            api_state
                .errors
                .lock()
                .map(|mut errors| errors.push(v))
                .map_err(|e| WasiError::Unknown(format!("{}", e)))
        })
        .to_error_code()
    }
}

pub mod attachments {
    use wasmer::{Array, Item, WasmPtr};

    use crate::runtime::wasi::{error::ToErrorCode, function};

    use super::{ApiState, WasmBuffer, WasmItemPtr, WasmString};

    pub fn get_path_len(
        api_state: &ApiState,
        attachment_name: WasmPtr<u8, Array>,
        attachment_name_len: u32,
        path_len: WasmPtr<u32, Item>,
    ) -> u32 {
        function::get_attachment_path_len(
            &api_state.attachments,
            WasmString::new(WasmBuffer::new(
                api_state.wasi_env.memory(),
                attachment_name,
                attachment_name_len,
            )),
            WasmItemPtr::new(api_state.wasi_env.memory(), path_len),
        )
        .to_error_code()
    }

    pub fn map(
        api_state: &ApiState,
        attachment_name: WasmPtr<u8, Array>,
        attachment_name_len: u32,
        unpack: u8,
        path_ptr: WasmPtr<u8, Array>,
        path_buffer_len: u32,
    ) -> u32 {
        api_state.async_runtime.block_on(async {
            function::map_attachment(
                &api_state.attachments,
                function::DownloadAttachmentContext {
                    function_dir: &api_state.function_dir,
                    auth: &api_state.auth_service,
                },
                &api_state.attachment_sandbox,
                WasmString::new(WasmBuffer::new(
                    api_state.wasi_env.memory(),
                    attachment_name,
                    attachment_name_len,
                )),
                unpack != 0,
                &mut WasmBuffer::new(api_state.wasi_env.memory(), path_ptr, path_buffer_len),
                &api_state.logger,
            )
            .await
            .to_error_code()
        })
    }

    pub fn get_path_len_from_descriptor(
        api_state: &ApiState,
        attachment_descriptor_ptr: WasmPtr<u8, Array>,
        attachment_descriptor_len: u32,
        path_len: WasmPtr<u32, Item>,
    ) -> u32 {
        function::get_attachment_path_len_from_descriptor(
            WasmBuffer::new(
                api_state.wasi_env.memory(),
                attachment_descriptor_ptr,
                attachment_descriptor_len,
            ),
            WasmItemPtr::new(api_state.wasi_env.memory(), path_len),
        )
        .to_error_code()
    }

    pub fn map_from_descriptor(
        api_state: &ApiState,
        attachment_descriptor_ptr: WasmPtr<u8, Array>,
        attachment_descriptor_len: u32,
        unpack: u8,
        path_ptr: WasmPtr<u8, Array>,
        path_buffer_len: u32,
    ) -> u32 {
        api_state.async_runtime.block_on(async {
            function::map_attachment_from_descriptor(
                &api_state.attachment_sandbox,
                WasmBuffer::new(
                    api_state.wasi_env.memory(),
                    attachment_descriptor_ptr,
                    attachment_descriptor_len,
                ),
                function::DownloadAttachmentContext {
                    function_dir: &api_state.function_dir,
                    auth: &api_state.auth_service,
                },
                unpack != 0,
                &mut WasmBuffer::new(api_state.wasi_env.memory(), path_ptr, path_buffer_len),
                &api_state.logger,
            )
            .await
            .to_error_code()
        })
    }
}

#[derive(Clone)]
pub struct ApiState {
    pub arguments: Arc<Stream>,
    pub attachments: Arc<Vec<Attachment>>,
    pub sandbox: Sandbox,
    pub attachment_sandbox: Sandbox,
    pub cache_sandbox: Sandbox,
    pub logger: Logger,
    pub stdout: Output,
    pub stderr: Output,
    pub results: Arc<Mutex<Stream>>,
    pub errors: Arc<Mutex<Vec<String>>>,
    pub wasi_env: WasiEnv,
    pub auth_service: AuthService,
    pub function_dir: FunctionDirectory,

    // TODO: this assumes that all clones of this Arc
    // actually ends up on the same thread, otherwise
    // it would not actually work
    pub async_runtime: Arc<tokio::runtime::Runtime>,
}

impl WasmerEnv for ApiState {
    fn init_with_instance(&mut self, instance: &Instance) -> Result<(), HostEnvInitError> {
        self.wasi_env.init_with_instance(instance)
    }
}

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

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Get a view of the underlying data in this buffer
    pub fn buffer(&self) -> &[u8] {
        if self.ptr.offset().saturating_add(self.len) > self.memory.size().bytes().0 as u32 {
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
        if self.ptr.offset().saturating_add(self.len) > self.memory.size().bytes().0 as u32 {
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
        Ok(std::str::from_utf8(s.buffer.buffer())?.to_owned())
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
