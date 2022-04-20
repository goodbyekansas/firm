use std::{
    collections::HashMap,
    ffi::CStr,
    marker::PhantomData,
    ops::Deref,
    os::raw::c_char,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use wasmtime::{Engine, Func, Linker, Memory, MemoryType, Store};

pub type AbiSizeType = i64;

/// Super simple bump allocator for the tests
#[derive(Clone)]
pub struct SimpleAllocator {
    offset: Arc<AtomicUsize>,
    capacity_pages: usize,
}

impl SimpleAllocator {
    pub fn new(capacity_pages: usize) -> Self {
        Self {
            offset: Arc::new(AtomicUsize::new(0)),
            capacity_pages,
        }
    }

    pub fn allocate(&self, amount: usize) -> WasmPtr<()> {
        let offset = self.offset.fetch_add(amount, Ordering::SeqCst);
        if offset >= self.capacity_pages * 1024 * 64 {
            WasmPtr::new(0)
        } else {
            WasmPtr::new(offset)
        }
    }

    pub fn allocate_ptr(&self) -> WasmPtr<()> {
        self.allocate(std::mem::size_of::<AbiSizeType>())
    }
}

/// Helper struct to manage a simple WASI runtime setup for tests
pub struct WasmTestContext<T> {
    pub allocator: Arc<SimpleAllocator>,
    pub store: Store<T>,
    pub mem: Memory,
    pub linker: Linker<T>,
    pub functions: HashMap<&'static str, Func>,
    _engine: Engine,
    pub mem_base: *mut u8,
}

impl<T> WasmTestContext<T> {
    /// Create a new WASI test setup
    ///
    /// `num_pages` controls how many pages of WASI memory is allocated for the "guest"
    pub fn new(num_pages: usize, data: T) -> Self {
        let engine = Engine::default();

        let mut store = Store::new(&engine, data);

        let mem = Memory::new(&mut store, MemoryType::new(num_pages as u32, None))
            .expect("failed to create memory");

        let allocator = Arc::new(SimpleAllocator::new(num_pages));
        let allocator2 = allocator.clone();
        let mut functions = HashMap::new();
        functions.insert(
            "allocate_wasm_mem",
            Func::wrap(&mut store, move |amount: AbiSizeType| {
                allocator2.allocate(amount as usize).guest_offset() as AbiSizeType
            }),
        );

        Self {
            allocator,
            mem_base: mem.data_ptr(&mut store),
            store,
            mem,
            linker: wasmtime::Linker::<T>::new(&engine),
            functions: functions.clone(),
            _engine: engine,
        }
    }

    /// Set mock versions of get_function and get_memory for the WASM code to use
    pub fn setup_mock_functions(
        &mut self,
        get_function: &mut Option<Box<dyn Fn(&str) -> Option<wasmtime::Func>>>,
        get_memory: &mut Option<Box<dyn Fn(&str) -> Option<wasmtime::Memory>>>,
    ) {
        // Note that this is not strictly correct but works since
        // the only thing the generated code wants is the allocator function we have
        // added above. The get_function will not be able to return any functions
        // added later to the linker
        let functions = self.functions.clone();
        let mem = self.mem;
        *get_function = Some(Box::new(move |name| functions.get(name).cloned()));
        *get_memory = Some(Box::new(move |_name| Some(mem)));
    }

    pub fn get_function(&mut self, module: &str, function: &str) -> Option<wasmtime::Func> {
        self.linker
            .get(&mut self.store, module, &format!("__{}", function))
            .and_then(|e| e.into_func())
    }

    /// Call a WASM function in this context and return a Rust result
    pub fn call_function(
        &mut self,
        func: wasmtime::Func,
        args: &[wasmtime::Val],
    ) -> Result<(), String> {
        let mut result: [wasmtime::Val; 1] = [AbiSizeType::from(0).into(); 1];
        func.call(&mut self.store, args, &mut result)
            .map_err(|e| format!("Failed to call WASM function: {}", e))
            .and_then(|_| {
                <Result<(), String> as FromWasmTimeValues>::from_values(&result, self.mem_base)
            })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WasmPtr<T, const HAS_OFFSET: bool = false> {
    offset: usize,
    base: *mut u8,
    _phantom: std::marker::PhantomData<T>,
}

impl<T, const U: bool> WasmPtr<T, U> {
    pub fn new(offset: usize) -> Self {
        Self {
            offset,
            base: std::ptr::null_mut(),
            _phantom: PhantomData,
        }
    }

    pub fn guest_offset(&self) -> usize {
        self.offset
    }

    pub fn to_host<S>(&self, base: *mut u8) -> WasmPtr<S, true> {
        WasmPtr::<S, true>::new_with_offset(self.offset, base)
    }
}

impl<T> WasmPtr<T, true> {
    pub fn new_with_offset(offset: usize, base: *mut u8) -> Self {
        Self {
            offset,
            base,
            _phantom: PhantomData,
        }
    }

    pub fn host_ptr(&self) -> *const T {
        unsafe { self.base.add(self.guest_offset()) as *const T }
    }

    pub fn host_ptr_mut(&self) -> *mut T {
        unsafe { self.base.add(self.guest_offset()) as *mut T }
    }
}

impl<T> Deref for WasmPtr<T, true> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.host_ptr() }
    }
}

impl<T, const U: bool> From<WasmPtr<T, U>> for wasmtime::Val {
    fn from(ptr: WasmPtr<T, U>) -> Self {
        (ptr.guest_offset() as AbiSizeType).into()
    }
}

impl From<WasmString> for wasmtime::Val {
    fn from(string: WasmString) -> Self {
        ((*string).guest_offset() as AbiSizeType).into()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WasmString(WasmPtr<u8, true>);

impl WasmString {
    pub fn new(ptr: WasmPtr<u8, true>) -> Self {
        Self(ptr)
    }

    pub fn new_indirect(ptr: WasmPtr<(), false>, mem_base: *mut u8) -> Self {
        Self::new(WasmPtr::new_with_offset(
            *ptr.to_host::<AbiSizeType>(mem_base) as usize,
            mem_base,
        ))
    }

    pub fn from_str(mem_base: *mut u8, allocator: &SimpleAllocator, value: &str) -> Self {
        let wasm_offset = allocator.allocate(value.len() as usize + 1);
        unsafe {
            let wasm_ptr = wasm_offset.to_host::<u8>(mem_base);
            wasm_ptr
                .host_ptr_mut()
                .copy_from_nonoverlapping(value.as_ptr(), value.len());
            wasm_ptr.host_ptr_mut().add(value.len() + 1).write(b'\0');
            Self(wasm_ptr)
        }
    }

    pub fn to_str(self) -> Result<&'static str, String> {
        unsafe { CStr::from_ptr(self.0.host_ptr() as *const c_char) }
            .to_str()
            .map_err(|e| e.to_string())
    }
}

impl From<WasmPtr<u8, true>> for WasmString {
    fn from(ptr: WasmPtr<u8, true>) -> Self {
        Self(ptr)
    }
}

impl Deref for WasmString {
    type Target = WasmPtr<u8, true>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub trait FromWasmTimeValues {
    fn from_values(values: &[wasmtime::Val], mem_base: *mut u8) -> Self;
}

impl FromWasmTimeValues for Result<(), String> {
    fn from_values(values: &[wasmtime::Val], mem_base: *mut u8) -> Self {
        values
            .first()
            .ok_or_else(|| String::from("Return result did not contain any value"))
            .and_then(|v| {
                v.i64()
                    .ok_or_else(|| String::from("The returned value was not of type i64."))
            })
            .and_then(|err_ptr| {
                match err_ptr {
                    0 => {
                        // Everything went ok!
                        Ok(())
                    }
                    -1 => {
                        // Something bad happened and we couldn't allocate memory for the error message.
                        Err(String::from(
                            "Unexpected unknown error occured. Probably out of memory.",
                        ))
                    }
                    error_ptr => Err(
                        // An error occured that we can handle.
                        match WasmString::new(WasmPtr::new_with_offset(
                            error_ptr as usize,
                            mem_base,
                        ))
                        .to_str()
                        {
                            Ok(v) => v.to_owned(),
                            Err(e) => format!("UTF-8 error: {}", e),
                        },
                    ),
                }
            })
    }
}
