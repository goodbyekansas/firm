mod api;
mod output;
mod sandbox;

use std::{
    collections::HashMap,
    ffi::CStr,
    fs::OpenOptions,
    io::LineWriter,
    ops::Deref,
    os::raw::{c_char, c_void},
    path::Path,
    path::PathBuf,
    str::Utf8Error,
    time::Instant,
};

use super::{Runtime, RuntimeParameters};
use crate::{
    channels::{ChannelReader, ChannelSet, ChannelWriter},
    executor::{AttachmentDownload, RuntimeError},
};
use firm_types::functions::Attachment;
use futures::TryFutureExt;
use output::{NamedFunctionOutputSink, Output};
use sandbox::Sandbox;
use slog::{debug, info, o, warn, Logger};
use thiserror::Error;
use wasi_common::WasiCtx;
use wasmtime::{
    AsContext, AsContextMut, Engine, Func, Instance, Linker, Memory, Module, Store, Val,
};
use wasmtime_wasi::WasiCtxBuilder;

#[derive(Debug, Clone)]
pub struct WasmRuntime {
    logger: Logger,
    host_dirs: HashMap<String, PathBuf>,
    is_wasi: bool,
}

impl WasmRuntime {
    pub fn new(logger: Logger, is_wasi: bool) -> Self {
        Self {
            logger,
            host_dirs: HashMap::new(),
            is_wasi,
        }
    }

    pub fn with_host_dir<P>(mut self, wasi_name: &str, host_path: P) -> Self
    where
        P: AsRef<Path>,
    {
        self.host_dirs
            .insert(wasi_name.to_owned(), host_path.as_ref().to_owned());
        self
    }
}

impl From<String> for RuntimeError {
    fn from(message: String) -> Self {
        Self::RuntimeError {
            name: "wasm".to_owned(),
            message,
        }
    }
}

#[derive(Error, Debug)]
pub enum WasmError {
    #[error("Allocation of WASM memory failed: {0}")]
    AllocationFailure(String),

    #[error("Memory access error: {0}")]
    MemoryAccessError(#[from] wasmtime::MemoryAccessError),

    #[error("Utf-8 error: {0}")]
    Utf8Error(#[from] std::str::Utf8Error),

    #[error("WASM Module has no associated memory")]
    NoMemory,

    #[error("Usage error: {0}")]
    UsageError(String),

    #[error("Process error: {0}")]
    ProcessError(String),
}

trait WasmResult {
    fn to_string_ptr<Allocator: WasmAllocator>(
        &self,
        store: impl AsContextMut<Data = WasmState<Allocator>>,
    ) -> i32;
}

impl WasmResult for Result<(), WasmError> {
    fn to_string_ptr<Allocator: WasmAllocator>(
        &self,
        mut store: impl AsContextMut<Data = WasmState<Allocator>>,
    ) -> i32 {
        match self {
            Ok(_) => 0,
            Err(e) => WasmString::try_from_str(&mut store, &e.to_string())
                .map(|ptr| ptr.guest_offset())
                .map_err(|e| {
                    warn!(
                        store.as_context().data().logger,
                        "failed to allocate wasm error string: {}", e
                    );
                    e
                })
                .unwrap_or(u32::MAX) as i32,
        }
    }
}

#[allow(dead_code)]
pub struct FunctionContext {
    error: Option<String>,
    stdout: Output,
    stderr: Output,
    attachments: Vec<Attachment>,
}

impl FunctionContext {
    fn new(stdout: Output, stderr: Output, attachments: Vec<Attachment>) -> Self {
        FunctionContext {
            error: None,
            stdout,
            stderr,
            attachments,
        }
    }
}

pub struct WasmState<Allocator: WasmAllocator> {
    wasi_ctx: WasiCtx,
    allocator: Allocator,
    memory: Option<Memory>,
    logger: Logger,
    function_context: FunctionContext,
}

impl<Allocator: WasmAllocator> WasmState<Allocator> {
    fn new_wasi(
        ctx: WasiCtx,
        allocator: Allocator,
        logger: Logger,
        function_context: FunctionContext,
    ) -> Self {
        Self {
            wasi_ctx: ctx,
            allocator,
            memory: None,
            logger,
            function_context,
        }
    }

    fn new_wasm(allocator: Allocator, logger: Logger, function_context: FunctionContext) -> Self {
        Self {
            wasi_ctx: WasiCtxBuilder::new().build(),
            allocator,
            memory: None,
            logger,
            function_context,
        }
    }
}

pub trait WasmAllocator: Clone {
    fn allocate<Allocator: WasmAllocator>(
        &self,
        store: impl AsContextMut<Data = WasmState<Allocator>>,
        amount: u32,
    ) -> Result<WasmPtr, WasmError>;
}

#[derive(Clone, Default)]
struct WasmExportedAllocator {
    allocator_fn: Option<Func>,
}

impl WasmExportedAllocator {
    fn new(instance: &Instance, store: impl AsContextMut) -> Self {
        Self {
            allocator_fn: instance.get_func(store, "allocate_wasi_mem"),
        }
    }
}

impl WasmAllocator for WasmExportedAllocator {
    fn allocate<Allocator: WasmAllocator>(
        &self,
        mut store: impl AsContextMut<Data = WasmState<Allocator>>,
        amount: u32,
    ) -> Result<WasmPtr, WasmError> {
        let mut returns = [Val::null()];
        self.allocator_fn
            .ok_or_else(|| {
                WasmError::AllocationFailure(String::from("No allocation function found"))
            })
            .and_then(|f| {
                f.call(
                    store.as_context_mut(),
                    &[(amount as i32).into()],
                    &mut returns,
                )
                .map_err(|e| {
                    WasmError::AllocationFailure(format!("Allocation function trapped: {}", e))
                })
                .and_then(|_| match returns.first() {
                    Some(Val::I32(0)) => Err(WasmError::AllocationFailure(String::from(
                        "WASM allocation failed (OOM?)",
                    ))),
                    Some(Val::I32(ptr)) => Ok(WasmPtr::new(store.as_context(), *ptr as u32)),
                    Some(_) => Err(WasmError::AllocationFailure(String::from(
                        "WASM allocation function returned wrong type",
                    ))),
                    None => Err(WasmError::AllocationFailure(String::from(
                        "WASM allocation function returned too few values",
                    ))),
                })
            })
    }
}

pub struct WasmPtr {
    offset: u32,
    host_ptr: *mut c_void,
}

impl std::fmt::Debug for WasmPtr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmPtr")
            .field("guest_offset", &self.offset)
            .field("host_ptr", &self.host_ptr)
            .finish()
    }
}

impl Default for WasmPtr {
    fn default() -> Self {
        Self {
            offset: 0,
            host_ptr: std::ptr::null_mut(),
        }
    }
}

impl WasmPtr {
    fn new<Allocator: WasmAllocator>(
        store: impl AsContext<Data = WasmState<Allocator>>,
        offset: u32,
    ) -> Self {
        Self {
            offset,
            host_ptr: store
                .as_context()
                .data()
                .memory
                .map(|mem| mem.data_ptr(&store.as_context()))
                .map(|base_ptr| unsafe { base_ptr.add(offset as usize) as *mut c_void })
                .unwrap_or_else(std::ptr::null_mut),
        }
    }

    fn host_ptr(&self) -> *const c_void {
        self.host_ptr
    }

    fn guest_offset(&self) -> u32 {
        self.offset
    }

    fn write<Allocator: WasmAllocator>(
        &mut self,
        mut store: impl AsContextMut<Data = WasmState<Allocator>>,
        data: &[u8],
    ) -> Result<(), WasmError> {
        store
            .as_context_mut()
            .data()
            .memory
            .ok_or(WasmError::NoMemory)
            .and_then(|mem| {
                mem.write(&mut store, self.offset as usize, data)
                    .map_err(Into::into)
            })
    }

    #[allow(dead_code)]
    fn get_ptr<Allocator: WasmAllocator>(
        &self,
        mut store: impl AsContextMut<Data = WasmState<Allocator>>,
    ) -> Result<Self, WasmError> {
        store
            .as_context_mut()
            .data()
            .memory
            .ok_or(WasmError::NoMemory)
            .and_then(|mem| {
                let mut buf = [0u8; 4];
                mem.read(&mut store, self.offset as usize, &mut buf)?;
                Ok(WasmPtr::new(store, u32::from_ne_bytes(buf)))
            })
    }
}

macro_rules! native_type {
    (string) => {
        &str
    };

    ($type:ident) => {
        $type
    };
}

macro_rules! native_return_type {
    (string) => {
        String
    };

    ($type:ident) => {
        $type
    };
}

macro_rules! abi_type {
    (bool) => {
        u8
    };

    (string) => {
        u32
    };

    ($type:ident) => {
        $type
    };
}

macro_rules! to_wasm_type {
    (string, $arg:expr, $caller:expr) => {
        crate::runtime::wasm::WasmString::from(crate::runtime::wasm::WasmPtr::new(
            &$caller,
            $arg as u32,
        ))
    };

    ($type:ident, $arg:expr, $caller:expr) => {
        crate::runtime::wasm::WasmPtr::new(&$caller, $arg as u32)
    };
}

macro_rules! wasm_type_to_native {
    (string, $var:ident) => {
        $var.to_str()
            .map_err(crate::runtime::wasm::WasmError::from)
            .unwrap() // TODO: this
    };
}

macro_rules! native_to_wasm_value {
    (bool, $native:expr, $caller:expr) => {
        [$native as abi_type!(bool)]
    };

    (string, $native:expr, $caller:expr) => {
        crate::runtime::wasm::WasmString::try_from_str($caller, &$native)?
            .guest_offset()
            .to_ne_bytes()
    };

    ($type:ident, $native:expr, $caller:expr) => {
        $native.to_ne_bytes()
    };
}

macro_rules! write_return_ptr {
    ($val:expr, $caller:expr, $retname:ident: $rettype:ident: $offset:expr) => {{
        let r = native_to_wasm_value!($rettype, $val, &mut $caller);
        crate::runtime::wasm::WasmPtr::new(&$caller, $offset as u32).write(&mut $caller, &r)
    }};

    // multi-value returns
    ($val:expr, $caller:expr, $($retname:ident: $rettype:ident: $offset:expr),*) => {
        [
            $(write_return_ptr!($val.$retname, $caller, $retname: $rettype: $offset)),*
        ].into_iter().collect()
    }
}

macro_rules! firm_function {
    ($linker:expr, $symbol:expr, $fun:expr) => {
        $linker
            .func_wrap("firm", stringify!(__$symbol), $fun)
            .map_err(|e| RuntimeError::from(e.to_string()))?;
    };

    // single return
    (@traitfn $name:ident($($arg:ident: $argtype:ident),*) -> $retname:ident: $rettype:ident) => {
        paste::paste! {
            fn [<$name>](&mut self, $($arg: native_type!($argtype)),*) -> Result<native_return_type!($rettype), String>;
        }
    };


    // multiple returns
    (@traitfn $name:ident($($arg:ident: $argtype:ident),*) -> $($retname:ident: $rettype:ident),*) => {
        paste::paste! {
            fn [<$name>](&mut self, $($arg: native_type!($argtype)),*) -> Result<types::[<$name:camel Value>], String>;
        }
    };

    (@wrapperfn $name:ident($($arg:ident: $argtype:ident),*) -> $($ret:ident: $rettype:ident),*) => {
        paste::paste! {
            pub fn [<$name>]<Allocator: crate::runtime::wasm::WasmAllocator>(
                mut caller: wasmtime::Caller<crate::runtime::wasm::WasmState<Allocator>>$(, $arg: i32)*$(, [<$ret _out>]: i32)*) -> i32 {

                $(
                    let [<$arg _wasm>] = to_wasm_type!($argtype, $arg, caller);
                    let $arg = wasm_type_to_native!($argtype, [<$arg _wasm>]);
                )*


                <crate::runtime::wasm::WasmState<Allocator> as super::FirmApi>::[<$name>](caller.data_mut(), $($arg),*)
                    .map_err(crate::runtime::wasm::WasmError::UsageError)
                    .and_then(|native_ret| {
                        write_return_ptr!(native_ret, caller, $($ret: $rettype: [<$ret _out>]),*)
                    }).to_string_ptr(&mut caller)
            }
        }
    };

    // do not create a struct for single-value returns
    (@returnstruct $name:ident, $member:ident: $type:ident) => {};

    (@returnstruct $name:ident, $($member:ident: $type:ident),*) => {
        paste::paste! {
            pub struct [<$name:camel Value>] {
                $(pub $member: native_return_type!($type)),*
            }
        }
    };
}

macro_rules! firm_functions {
    ($($name:ident($($arg:ident: $argtype:ident),*) -> ($($ret:ident: $rettype:ident),*)),*) => {
        paste::paste! {
            mod types {
                $(firm_function!{@returnstruct $name, $($ret: $rettype),*})*
            }

            trait FirmApi {
                $(firm_function!(@traitfn $name($($arg: $argtype),*) -> $($ret: $rettype),*);)*
            }

            mod wrappers {
                use crate::runtime::wasm::WasmResult;
                $(firm_function!(@wrapperfn $name($($arg: $argtype),*) -> $($ret: $rettype),*);)*
            }

            fn set_up_api<Allocator: WasmAllocator + 'static>(
                linker: &mut Linker<WasmState<Allocator>>,
            ) -> Result<&mut Linker<WasmState<Allocator>>, RuntimeError> {
                firm_function!(linker, "__map_attachment", api::map_attachment);
                firm_function!(linker, "__start_host_process", api::start_host_process);
                firm_function!(linker, "__set_error", api::set_error);

                $(firm_function!(linker, $name, wrappers::$name);)*

                Ok(linker)
            }
        }
    };
}
firm_structs! {
    StartHostProcessRequest {}
}
firm_functions! {
    host_os() -> (os: string),
    host_path_exists(path: string) -> (exists: bool),
    start_host_process(request: StartHostProcessRequest) -> (pid: u64, exit_code: i64)
}

#[derive(Debug)]
struct WasmString(WasmPtr);

impl WasmString {
    fn try_from_str<Allocator: WasmAllocator>(
        mut store: impl AsContextMut<Data = WasmState<Allocator>>,
        value: &str,
    ) -> Result<Self, WasmError> {
        let mut ctx = store.as_context_mut();
        let allocator = ctx.data().allocator.clone();

        allocator
            .allocate(&mut ctx, value.len() as u32 + 1)
            .map(|wasm_ptr| unsafe {
                let mem = wasm_ptr.host_ptr() as *mut c_char;
                mem.copy_from_nonoverlapping(value.as_ptr() as *const c_char, value.len());
                mem.add(value.len() + 1).write(b'\0' as c_char);
                Self(wasm_ptr)
            })
    }

    #[allow(dead_code)]
    fn to_str(&self) -> Result<&str, Utf8Error> {
        unsafe { CStr::from_ptr(self.0.host_ptr() as *const c_char) }.to_str()
    }
}

impl From<WasmPtr> for WasmString {
    fn from(ptr: WasmPtr) -> Self {
        Self(ptr)
    }
}

impl Deref for WasmString {
    type Target = WasmPtr;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Runtime for WasmRuntime {
    fn execute(
        &self,
        runtime_parameters: RuntimeParameters,
        _inputs: &ChannelSet<ChannelReader>,
        _outputs: &mut ChannelSet<ChannelWriter>,
        attachments: Vec<Attachment>,
    ) -> Result<Result<(), String>, RuntimeError> {
        let function_logger = self
            .logger
            .new(o!("function" => runtime_parameters.function_name.to_owned()));

        let (sandbox, attachment_sandbox, _cache_sandbox) = (
            Sandbox::new(
                runtime_parameters.function_dir.execution_path(),
                Path::new("sandbox"),
            )?,
            Sandbox::new(
                runtime_parameters.function_dir.execution_path(),
                Path::new("attachments"),
            )?,
            Sandbox::new(
                runtime_parameters.function_dir.cache_path(),
                Path::new("cache"),
            )?,
        );

        info!(
            function_logger,
            "using sandbox directory: {}",
            sandbox.host_path().display()
        );
        info!(
            function_logger,
            "using sandbox attachments directory: {}",
            attachment_sandbox.host_path().display()
        );

        let stdout = Output::new(vec![
            Box::new(
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(sandbox.host_path().join("stdout"))
                    .map_err(|e| format!("Failed to create stdout sandbox file: {}", e))?,
            ),
            Box::new(LineWriter::new(NamedFunctionOutputSink::new(
                "stdout",
                runtime_parameters.output_sink.clone(),
                function_logger.new(o!("output-sink" => "stdout")),
            ))),
        ]);

        let stderr = Output::new(vec![
            Box::new(
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(sandbox.host_path().join("stderr"))
                    .map_err(|e| format!("Failed to create stderr sandbox file: {}", e))?,
            ),
            Box::new(LineWriter::new(NamedFunctionOutputSink::new(
                "stderr",
                runtime_parameters.output_sink,
                function_logger.new(o!("output-sink" => "stderr")),
            ))),
        ]);
        let engine = Engine::default();
        let code = futures::executor::block_on(
            runtime_parameters
                .code
                .map(|code| {
                    info!(
                        function_logger,
                        "Downloading code from \"{}\"",
                        code.url
                            .as_ref()
                            .map(|url| url.url.as_str())
                            .unwrap_or("No Url")
                    );
                    code
                })
                .ok_or_else(|| RuntimeError::MissingCode("wasi".to_owned()))?
                .download_cached(
                    runtime_parameters.function_dir.attachments_path(),
                    &runtime_parameters.auth_service,
                )
                .map_ok(|content| {
                    info!(function_logger, "Done downloading code");
                    content
                }),
        )
        .and_then(|p| {
            std::fs::read(p).map_err(|e| {
                RuntimeError::AttachmentReadError(
                    "code".to_owned(),
                    format!("Failed to read downloaded code: {}", e),
                )
            })
        })?;

        let mut linker = Linker::new(&engine);
        if self.is_wasi {
            wasmtime_wasi::add_to_linker(&mut linker, |s: &mut WasmState<_>| &mut s.wasi_ctx)
                .map_err(|e| e.to_string())?;
        }

        set_up_api(&mut linker)?;

        let module_cache_path = runtime_parameters
            .function_dir
            .cache_path
            .join(&runtime_parameters.function_name);
        Module::from_file(&engine, &module_cache_path)
            .or_else(|_| {
                debug!(
                    function_logger,
                    "failed to load wasi module \"{}\" from cache, compiling...",
                    &runtime_parameters.function_name
                );
                let now = Instant::now();
                Module::new_with_name(&engine, code, &runtime_parameters.function_name).and_then(
                    |module| {
                        debug!(
                            function_logger,
                            "wasi module \"{}\" compiled (took {} ms)",
                            &runtime_parameters.function_name,
                            now.elapsed().as_millis()
                        );
                        module
                            .serialize()
                            .and_then(|bytes| {
                                std::fs::write(&module_cache_path, bytes).map_err(Into::into)
                            })
                            .map(|_| {
                                debug!(
                                    function_logger,
                                    "wasi module \"{}\" cached at {}",
                                    &runtime_parameters.function_name,
                                    module_cache_path.display()
                                );

                                module
                            })
                    },
                )
            })
            .map_err(|e| RuntimeError::from(e.to_string()))
            .and_then(|module| {
                let function_context =
                    FunctionContext::new(stdout.clone(), stderr.clone(), attachments);
                let mut store = Store::new(
                    &engine,
                    if self.is_wasi {
                        WasmState::new_wasi(
                            WasiCtxBuilder::new()
                                .stderr(Box::new(stderr))
                                .stdout(Box::new(stdout))
                                .build(),
                            WasmExportedAllocator::default(),
                            function_logger.new(o!("wasm-flavor" => "wasi")),
                            function_context,
                        )
                    } else {
                        WasmState::new_wasm(
                            WasmExportedAllocator::default(),
                            function_logger.new(o!("wasm-flavor" => "wasm")),
                            function_context,
                        )
                    },
                );

                linker
                    .instantiate(&mut store, &module)
                    .map_err(|e| {
                        RuntimeError::from(format!(
                            "failed to instantiate wasi module \"{}\": {}",
                            &runtime_parameters.function_name, e
                        ))
                    })
                    .map(|instance| (instance, store))
            })
            .and_then(|(instance, mut store)| {
                store.data_mut().allocator = WasmExportedAllocator::new(&instance, &mut store);

                store.data_mut().memory =
                    Some(instance.get_memory(&mut store, "memory").ok_or_else(|| {
                        RuntimeError::from(format!(
                            "wasi module \"{}\" does not export any memory?",
                            &runtime_parameters.function_name
                        ))
                    })?);

                Ok((instance, store))
            })
            .and_then(|(instance, mut store)| {
                instance
                    .get_typed_func::<(), (), _>(&mut store, "")
                    .or_else(|_| instance.get_typed_func::<(), (), _>(&mut store, "_start"))
                    .map_err(|e| {
                        RuntimeError::from(format!(
                            "wasi module \"{}\" does not have an entrypoint: {}",
                            &runtime_parameters.function_name, e
                        ))
                    })
                    .map(|entrypoint| (entrypoint, store))
            })
            // from this point, errors are considered function errors, not runtime errors
            .map(|(entrypoint, mut store)| {
                entrypoint
                    .call(&mut store, ())
                    .map_err(|trap| trap.to_string())
            })
    }
}

#[cfg(test)]
mod tests {
    use crate::{auth::AuthService, executor::FunctionOutputSink, runtime::FunctionDirectory};

    use super::*;
    use firm_types::code_file;

    macro_rules! null_logger {
        () => {{
            slog::Logger::root(slog::Discard, o!())
        }};
    }

    #[tokio::test]
    async fn test_execution() {
        let tmp_fold = tempfile::tempdir().unwrap();
        // hello-world is a WASI executable
        let runtime = WasmRuntime::new(null_logger!(), true);
        let res = runtime.execute(
            RuntimeParameters {
                function_dir: FunctionDirectory::new(
                    tmp_fold.path(),
                    "hello-world",
                    "0.1.0",
                    "checksumma",
                    "abc123",
                )
                .unwrap(),
                function_name: "hello-world".to_owned(),
                entrypoint: None, // use default entrypoint _start
                code: Some(code_file!(include_bytes!("hello.wasm"))),
                arguments: std::collections::HashMap::new(),
                output_sink: FunctionOutputSink::null(),
                auth_service: AuthService::default(),
            },
            &ChannelSet::from(&HashMap::with_capacity(0)).reader(),
            &mut ChannelSet::from(&HashMap::with_capacity(0)),
            vec![],
        );

        assert!(res.is_ok());
        assert!(res.unwrap().is_ok());
    }
}
