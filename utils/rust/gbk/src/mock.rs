use std::{
    collections::HashMap,
    sync::Mutex,
    thread::{self, ThreadId},
};

use gbk_protocols::functions::{FunctionArgument, ReturnValue, StartProcessRequest};
use lazy_static::lazy_static;
use prost::Message;

pub unsafe fn start_host_process(request_ptr: *const u8, len: usize, pid: *mut u64) -> u32 {
    MockResultRegistry::execute_start_host_process(request_ptr, len, pid)
}

pub unsafe fn run_host_process(request_ptr: *const u8, len: usize, exit_code: *mut i32) -> u32 {
    MockResultRegistry::execute_run_host_process(request_ptr, len, exit_code)
}

pub unsafe fn get_input_len(key_ptr: *const u8, len: usize, value: *mut u64) -> u32 {
    MockResultRegistry::execute_get_input_len(key_ptr, len, value)
}

pub unsafe fn get_input(
    key_ptr: *const u8,
    key_len: usize,
    value_ptr: *mut u8,
    value_len: usize,
) -> u32 {
    MockResultRegistry::execute_get_input(key_ptr, key_len, value_ptr, value_len)
}

pub unsafe fn set_output(value_ptr: *const u8, value_len: usize) -> u32 {
    MockResultRegistry::execute_set_output(value_ptr, value_len)
}

pub unsafe fn set_error(msg_ptr: *const u8, msg_len: usize) -> u32 {
    MockResultRegistry::execute_set_error(msg_ptr, msg_len)
}

#[cfg(feature = "net")]
pub unsafe fn connect(addr_ptr: *const u8, addr_len: usize, file_descriptor: *mut u32) -> u32 {
    MockResultRegistry::execute_connect(addr_ptr, addr_len, file_descriptor)
}

lazy_static! {
    static ref MOCK_RESULT_REGISTRY: Mutex<MockResultRegistry> =
        Mutex::new(MockResultRegistry::new());
}

pub struct MockResultRegistry {
    start_host_process_closure:
        HashMap<ThreadId, Box<dyn Fn(StartProcessRequest) -> Result<u64, u32> + Send>>,
    run_host_process_closure:
        HashMap<ThreadId, Box<dyn Fn(StartProcessRequest) -> Result<i32, u32> + Send>>,
    get_input_len_closure: HashMap<ThreadId, Box<dyn Fn(&str) -> Result<usize, u32> + Send>>,
    get_input_closure: HashMap<ThreadId, Box<dyn Fn(&str) -> Result<FunctionArgument, u32> + Send>>,
    set_output_closure: HashMap<ThreadId, Box<dyn Fn(ReturnValue) -> Result<(), u32> + Send>>,
    set_error_closure: HashMap<ThreadId, Box<dyn Fn(&str) -> Result<(), u32> + Send>>,

    #[cfg(feature = "net")]
    connect_closure: HashMap<ThreadId, Box<dyn Fn(&str) -> Result<u32, u32> + Send>>,
}

impl MockResultRegistry {
    pub fn new() -> Self {
        Self {
            start_host_process_closure: HashMap::new(),
            run_host_process_closure: HashMap::new(),
            get_input_len_closure: HashMap::new(),
            get_input_closure: HashMap::new(),
            set_output_closure: HashMap::new(),
            set_error_closure: HashMap::new(),

            #[cfg(feature = "net")]
            connect_closure: HashMap::new(),
        }
    }

    pub fn set_start_host_process_impl<F>(closure: F)
    where
        F: Fn(StartProcessRequest) -> Result<u64, u32> + 'static + Send,
    {
        MOCK_RESULT_REGISTRY
            .lock()
            .unwrap()
            .start_host_process_closure
            .insert(thread::current().id(), Box::new(closure));
    }

    fn execute_start_host_process(request_ptr: *const u8, len: usize, pid: *mut u64) -> u32 {
        let mut vec = Vec::with_capacity(len);
        unsafe {
            request_ptr.copy_to(vec.as_mut_ptr(), len);
            vec.set_len(len);
        }

        let start_process_request = StartProcessRequest::decode(vec.as_slice()).unwrap();

        MOCK_RESULT_REGISTRY
            .lock()
            .unwrap()
            .start_host_process_closure
            .get(&thread::current().id())
            .map_or_else(
                || 1,
                |c| match c(start_process_request) {
                    Ok(p) => {
                        unsafe {
                            *pid = p;
                        }
                        0
                    }
                    Err(e) => e,
                },
            )
    }

    pub fn set_run_host_process_impl<F>(closure: F)
    where
        F: Fn(StartProcessRequest) -> Result<i32, u32> + 'static + Send,
    {
        MOCK_RESULT_REGISTRY
            .lock()
            .unwrap()
            .run_host_process_closure
            .insert(thread::current().id(), Box::new(closure));
    }

    fn execute_run_host_process(request_ptr: *const u8, len: usize, exit_code: *mut i32) -> u32 {
        let mut vec = Vec::with_capacity(len);
        unsafe {
            request_ptr.copy_to(vec.as_mut_ptr(), len);
            vec.set_len(len);
        }

        let start_process_request = StartProcessRequest::decode(vec.as_slice()).unwrap();

        MOCK_RESULT_REGISTRY
            .lock()
            .unwrap()
            .run_host_process_closure
            .get(&thread::current().id())
            .map_or_else(
                || 1,
                |c| match c(start_process_request) {
                    Ok(ex) => {
                        unsafe {
                            *exit_code = ex;
                        }
                        0
                    }
                    Err(e) => e,
                },
            )
    }

    pub fn set_get_input_len_impl<F>(closure: F)
    where
        F: Fn(&str) -> Result<usize, u32> + 'static + Send,
    {
        MOCK_RESULT_REGISTRY
            .lock()
            .unwrap()
            .get_input_len_closure
            .insert(thread::current().id(), Box::new(closure));
    }

    fn execute_get_input_len(key_ptr: *const u8, len: usize, value: *mut u64) -> u32 {
        let key = unsafe {
            let slice = std::slice::from_raw_parts(key_ptr, len);
            std::str::from_utf8(slice).unwrap()
        };

        MOCK_RESULT_REGISTRY
            .lock()
            .unwrap()
            .get_input_len_closure
            .get(&thread::current().id())
            .map_or_else(
                || 1,
                |c| match c(key) {
                    Ok(l) => {
                        unsafe {
                            *value = l as u64;
                        }
                        0
                    }
                    Err(e) => e,
                },
            )
    }

    pub fn set_get_input_impl<F>(closure: F)
    where
        F: Fn(&str) -> Result<FunctionArgument, u32> + 'static + Send,
    {
        MOCK_RESULT_REGISTRY
            .lock()
            .unwrap()
            .get_input_closure
            .insert(thread::current().id(), Box::new(closure));
    }

    fn execute_get_input(key_ptr: *const u8, len: usize, value: *mut u8, value_len: usize) -> u32 {
        let key = unsafe {
            let slice = std::slice::from_raw_parts(key_ptr, len);
            std::str::from_utf8(slice).unwrap()
        };

        MOCK_RESULT_REGISTRY
            .lock()
            .unwrap()
            .get_input_closure
            .get(&thread::current().id())
            .map_or_else(
                || 1,
                |c| match c(key) {
                    Ok(f) => {
                        let mut buff = Vec::with_capacity(value_len);
                        f.encode(&mut buff).unwrap();
                        unsafe {
                            buff.as_ptr().copy_to(value, value_len);
                        }
                        0
                    }
                    Err(e) => e,
                },
            )
    }

    pub fn set_set_output_impl<F>(closure: F)
    where
        F: Fn(ReturnValue) -> Result<(), u32> + 'static + Send,
    {
        MOCK_RESULT_REGISTRY
            .lock()
            .unwrap()
            .set_output_closure
            .insert(thread::current().id(), Box::new(closure));
    }

    fn execute_set_output(value_ptr: *const u8, value_len: usize) -> u32 {
        let mut vec = Vec::with_capacity(value_len);
        unsafe {
            value_ptr.copy_to(vec.as_mut_ptr(), value_len);
            vec.set_len(value_len);
        }

        let return_value = ReturnValue::decode(vec.as_slice()).unwrap();
        MOCK_RESULT_REGISTRY
            .lock()
            .unwrap()
            .set_output_closure
            .get(&thread::current().id())
            .map_or_else(
                || 1,
                |c| match c(return_value) {
                    Ok(_) => 0,
                    Err(e) => e,
                },
            )
    }

    pub fn set_set_error_impl<F>(closure: F)
    where
        F: Fn(&str) -> Result<(), u32> + 'static + Send,
    {
        MOCK_RESULT_REGISTRY
            .lock()
            .unwrap()
            .set_error_closure
            .insert(thread::current().id(), Box::new(closure));
    }

    fn execute_set_error(msg_ptr: *const u8, msg_len: usize) -> u32 {
        let error_msg = unsafe {
            let slice = std::slice::from_raw_parts(msg_ptr, msg_len);
            std::str::from_utf8(slice).unwrap()
        };

        MOCK_RESULT_REGISTRY
            .lock()
            .unwrap()
            .set_error_closure
            .get(&thread::current().id())
            .map_or_else(
                || 1,
                |c| match c(error_msg) {
                    Ok(_) => 0,
                    Err(e) => e,
                },
            )
    }

    #[cfg(feature = "net")]
    pub fn set_connect_impl<F>(closure: F)
    where
        F: Fn(&str) -> Result<u32, u32> + 'static + Send,
    {
        MOCK_RESULT_REGISTRY
            .lock()
            .unwrap()
            .connect_closure
            .insert(thread::current().id(), Box::new(closure));
    }

    #[cfg(feature = "net")]
    fn execute_connect(addr_ptr: *const u8, addr_len: usize, file_descriptor: *mut u32) -> u32 {
        let address = unsafe {
            let slice = std::slice::from_raw_parts(addr_ptr, addr_len);
            std::str::from_utf8(slice).unwrap()
        };
        MOCK_RESULT_REGISTRY
            .lock()
            .unwrap()
            .connect_closure
            .get(&thread::current().id())
            .map_or_else(
                || 1,
                |c| match c(address) {
                    Ok(fd) => {
                        unsafe {
                            *file_descriptor = fd;
                        }
                        0
                    }
                    Err(e) => e,
                },
            )
    }

    pub fn set_inputs(args: &[FunctionArgument]) {
        let argument_lengths: Vec<(String, usize)> = args
            .iter()
            .map(|a| (a.name.clone(), a.encoded_len()))
            .collect();
        MockResultRegistry::set_get_input_len_impl(move |key| {
            match argument_lengths.iter().find(|(name, _)| name == key) {
                None => Err(1),
                Some((_, len)) => Ok(*len),
            }
        });

        let arguments = args.to_vec();
        MockResultRegistry::set_get_input_impl(move |key| {
            match arguments.iter().find(|i| &i.name == key) {
                None => Err(1),
                Some(a) => Ok(a.clone()),
            }
        });
    }
}