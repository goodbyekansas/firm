use std::{
    collections::HashMap,
    sync::Mutex,
    thread::{self, ThreadId},
};

use firm_types::{
    functions::{Attachment, Channel, Stream},
    prost::Message,
    stream::StreamExt,
    wasi::StartProcessRequest,
};
use lazy_static::lazy_static;

/// Get attachment path len
///
/// # Safety
/// This is a mock implementation and while it uses
/// unsafe functions it does nothing technically unsafe
pub unsafe fn get_attachment_path_len(
    attachment_name_ptr: *const u8,
    attachment_name_len: usize,
    path_len: *mut usize,
) -> u32 {
    MockResultRegistry::execute_get_attachment_path_len(
        attachment_name_ptr,
        attachment_name_len,
        path_len,
    )
}

/// Map attachment
///
/// # Safety
/// This is a mock implementation and while it uses
/// unsafe functions it does nothing technically unsafe
pub unsafe fn map_attachment(
    attachment_name_ptr: *const u8,
    attachment_name_len: usize,
    unpack: u8,
    path_ptr: *mut u8,
    path_len: usize,
) -> u32 {
    MockResultRegistry::execute_map_attachment(
        attachment_name_ptr,
        attachment_name_len,
        unpack,
        path_ptr,
        path_len,
    )
}

/// Get attachment path len from descriptor
///
/// # Safety
/// This is a mock implementation and while it uses
/// unsafe functions it does nothing technically unsafe
pub unsafe fn get_attachment_path_len_from_descriptor(
    attachment_descriptor_ptr: *const u8,
    attachment_descriptor_len: usize,
    path_len: *mut usize,
) -> u32 {
    MockResultRegistry::execute_get_attachment_path_len_from_descriptor(
        attachment_descriptor_ptr,
        attachment_descriptor_len,
        path_len,
    )
}

/// Map attachment from descriptor
///
/// # Safety
/// This is a mock implementation and while it uses
/// unsafe functions it does nothing technically unsafe
pub unsafe fn map_attachment_from_descriptor(
    attachment_descriptor_ptr: *const u8,
    attachment_descriptor_len: usize,
    unpack: u8,
    path_ptr: *mut u8,
    path_len: usize,
) -> u32 {
    MockResultRegistry::execute_map_attachment_from_descriptor(
        attachment_descriptor_ptr,
        attachment_descriptor_len,
        unpack,
        path_ptr,
        path_len,
    )
}

/// Host path exists
///
/// # Safety
/// This is a mock implementation and while it uses
/// unsafe functions it does nothing technically unsafe
pub unsafe fn host_path_exists(path: *const u8, path_len: usize, exists: *mut u8) -> u32 {
    MockResultRegistry::execute_host_path_exists(path, path_len, exists)
}

/// Get host os
///
/// # Safety
/// This is a mock implementation and while it uses
/// unsafe functions it does nothing technically unsafe
pub unsafe fn get_host_os(os_name: *mut u8, os_name_length: *mut u32) -> u32 {
    MockResultRegistry::execute_get_host_os(os_name, os_name_length)
}

/// Start host process
///
/// # Safety
/// This is a mock implementation and while it uses
/// unsafe functions it does nothing technically unsafe
pub unsafe fn start_host_process(request_ptr: *const u8, len: usize, pid: *mut u64) -> u32 {
    MockResultRegistry::execute_start_host_process(request_ptr, len, pid)
}

/// Run host process
///
/// # Safety
/// This is a mock implementation and while it uses
/// unsafe functions it does nothing technically unsafe
pub unsafe fn run_host_process(request_ptr: *const u8, len: usize, exit_code: *mut i32) -> u32 {
    MockResultRegistry::execute_run_host_process(request_ptr, len, exit_code)
}

/// Get length of an input
///
/// # Safety
/// This is a mock implementation and while it uses
/// unsafe functions it does nothing technically unsafe
pub unsafe fn get_input_len(key_ptr: *const u8, len: usize, value: *mut u64) -> u32 {
    MockResultRegistry::execute_get_input_len(key_ptr, len, value)
}

/// Get a function input
///
/// # Safety
/// This is a mock implementation and while it uses
/// unsafe functions it does nothing technically unsafe
pub unsafe fn get_input(
    key_ptr: *const u8,
    key_len: usize,
    value_ptr: *mut u8,
    value_len: usize,
) -> u32 {
    MockResultRegistry::execute_get_input(key_ptr, key_len, value_ptr, value_len)
}

/// Set a function output
///
/// # Safety
/// This is a mock implementation and while it uses
/// unsafe functions it does nothing technically unsafe
pub unsafe fn set_output(
    key_ptr: *const u8,
    key_len: usize,
    value_ptr: *const u8,
    value_len: usize,
) -> u32 {
    MockResultRegistry::execute_set_output(key_ptr, key_len, value_ptr, value_len)
}

/// Set an error for this function
///
/// # Safety
/// This is a mock implementation and while it uses
/// unsafe functions it does nothing technically unsafe
pub unsafe fn set_error(msg_ptr: *const u8, msg_len: usize) -> u32 {
    MockResultRegistry::execute_set_error(msg_ptr, msg_len)
}

#[cfg(feature = "net")]
/// Connect to a remote endpoint
///
/// # Safety
/// This is a mock implementation and while it uses
/// unsafe functions it does nothing technically unsafe
pub unsafe fn connect(addr_ptr: *const u8, addr_len: usize, file_descriptor: *mut i32) -> u32 {
    MockResultRegistry::execute_connect(addr_ptr, addr_len, file_descriptor)
}

lazy_static! {
    static ref MOCK_RESULT_REGISTRY: Mutex<MockResultRegistry> =
        Mutex::new(MockResultRegistry::default());
}

type MockCallbacks<T> = HashMap<ThreadId, Box<T>>;

#[derive(Default)]
pub struct MockResultRegistry {
    get_attachment_path_len_closure: MockCallbacks<dyn Fn(&str) -> Result<usize, u32> + Send>,
    map_attachment_closure: MockCallbacks<dyn Fn(&str, bool) -> Result<String, u32> + Send>,
    get_attachment_path_len_from_descriptor_closure:
        MockCallbacks<dyn Fn(&Attachment) -> Result<usize, u32> + Send>,
    map_attachment_from_descriptor_closure:
        MockCallbacks<dyn Fn(&Attachment, bool) -> Result<String, u32> + Send>,
    host_path_exists_closure: MockCallbacks<dyn Fn(&str) -> Result<bool, u32> + Send>,
    get_host_os_closure: MockCallbacks<dyn Fn() -> Result<String, u32> + Send>,
    start_host_process_closure:
        MockCallbacks<dyn Fn(StartProcessRequest) -> Result<u64, u32> + Send>,
    run_host_process_closure: MockCallbacks<dyn Fn(StartProcessRequest) -> Result<i32, u32> + Send>,
    get_input_len_closure: MockCallbacks<dyn Fn(&str) -> Result<usize, u32> + Send>,
    get_input_closure: MockCallbacks<dyn Fn(&str) -> Result<Channel, u32> + Send>,
    set_output_closure: MockCallbacks<dyn Fn(&str, Channel) -> Result<(), u32> + Send>,
    set_error_closure: MockCallbacks<dyn Fn(&str) -> Result<(), u32> + Send>,

    #[cfg(feature = "net")]
    connect_closure: MockCallbacks<dyn Fn(&str) -> Result<i32, u32> + Send>,
}

impl MockResultRegistry {
    pub fn set_get_attachment_path_len_impl<F>(closure: F)
    where
        F: Fn(&str) -> Result<usize, u32> + 'static + Send,
    {
        MOCK_RESULT_REGISTRY
            .lock()
            .unwrap()
            .get_attachment_path_len_closure
            .insert(thread::current().id(), Box::new(closure));
    }

    fn execute_get_attachment_path_len(
        attachment_name_ptr: *const u8,
        attachment_name_len: usize,
        path_len: *mut usize,
    ) -> u32 {
        let attachment_name = unsafe {
            let slice = std::slice::from_raw_parts(attachment_name_ptr, attachment_name_len);
            std::str::from_utf8(slice).unwrap()
        };
        MOCK_RESULT_REGISTRY
            .lock()
            .unwrap()
            .get_attachment_path_len_closure
            .get(&thread::current().id())
            .map_or_else(
                || 1,
                |c| match c(attachment_name) {
                    Ok(ex) => {
                        unsafe {
                            *path_len = ex;
                        }
                        0
                    }
                    Err(e) => e,
                },
            )
    }

    pub fn set_map_attachment_impl<F>(closure: F)
    where
        F: Fn(&str, bool) -> Result<String, u32> + 'static + Send,
    {
        MOCK_RESULT_REGISTRY
            .lock()
            .unwrap()
            .map_attachment_closure
            .insert(thread::current().id(), Box::new(closure));
    }

    fn execute_map_attachment(
        attachment_name_ptr: *const u8,
        attachment_name_len: usize,
        unpack: u8,
        path_ptr: *mut u8,
        path_buffer_len: usize,
    ) -> u32 {
        let attachment_name = unsafe {
            let slice = std::slice::from_raw_parts(attachment_name_ptr, attachment_name_len);
            std::str::from_utf8(slice).unwrap()
        };

        MOCK_RESULT_REGISTRY
            .lock()
            .unwrap()
            .map_attachment_closure
            .get(&thread::current().id())
            .map_or_else(
                || 1,
                |c| match c(attachment_name, unpack != 0) {
                    Ok(att_path) => {
                        let buff =
                            unsafe { std::slice::from_raw_parts_mut(path_ptr, path_buffer_len) };
                        buff.clone_from_slice(att_path.as_bytes());
                        0
                    }
                    Err(e) => e,
                },
            )
    }

    pub fn set_get_attachment_path_len_from_descriptor_impl<F>(closure: F)
    where
        F: Fn(&Attachment) -> Result<usize, u32> + 'static + Send,
    {
        MOCK_RESULT_REGISTRY
            .lock()
            .unwrap()
            .get_attachment_path_len_from_descriptor_closure
            .insert(thread::current().id(), Box::new(closure));
    }

    fn execute_get_attachment_path_len_from_descriptor(
        attachment_descriptor_ptr: *const u8,
        attachment_descriptor_len: usize,
        path_len: *mut usize,
    ) -> u32 {
        let attachment = unsafe {
            Attachment::decode(std::slice::from_raw_parts(
                attachment_descriptor_ptr,
                attachment_descriptor_len,
            ))
            .unwrap()
        };

        MOCK_RESULT_REGISTRY
            .lock()
            .unwrap()
            .get_attachment_path_len_from_descriptor_closure
            .get(&thread::current().id())
            .map_or_else(
                || 1,
                |c| match c(&attachment) {
                    Ok(ex) => {
                        unsafe {
                            *path_len = ex;
                        }
                        0
                    }
                    Err(e) => e,
                },
            )
    }

    pub fn set_map_attachment_from_descriptor_impl<F>(closure: F)
    where
        F: Fn(&Attachment, bool) -> Result<String, u32> + 'static + Send,
    {
        MOCK_RESULT_REGISTRY
            .lock()
            .unwrap()
            .map_attachment_from_descriptor_closure
            .insert(thread::current().id(), Box::new(closure));
    }

    fn execute_map_attachment_from_descriptor(
        attachment_descriptor_ptr: *const u8,
        attachment_descriptor_len: usize,
        unpack: u8,
        path_ptr: *mut u8,
        path_buffer_len: usize,
    ) -> u32 {
        let attachment = unsafe {
            Attachment::decode(std::slice::from_raw_parts(
                attachment_descriptor_ptr,
                attachment_descriptor_len,
            ))
            .unwrap()
        };

        MOCK_RESULT_REGISTRY
            .lock()
            .unwrap()
            .map_attachment_from_descriptor_closure
            .get(&thread::current().id())
            .map_or_else(
                || 1,
                |c| match c(&attachment, unpack != 0) {
                    Ok(att_path) => {
                        let buff =
                            unsafe { std::slice::from_raw_parts_mut(path_ptr, path_buffer_len) };
                        buff.clone_from_slice(att_path.as_bytes());
                        0
                    }
                    Err(e) => e,
                },
            )
    }

    pub fn set_host_path_exists_impl<F>(closure: F)
    where
        F: Fn(&str) -> Result<bool, u32> + Send + 'static,
    {
        MOCK_RESULT_REGISTRY
            .lock()
            .unwrap()
            .host_path_exists_closure
            .insert(thread::current().id(), Box::new(closure));
    }

    fn execute_host_path_exists(path_ptr: *const u8, path_len: usize, exists: *mut u8) -> u32 {
        let path = unsafe {
            let slice = std::slice::from_raw_parts(path_ptr, path_len);
            std::str::from_utf8(slice).unwrap()
        };

        MOCK_RESULT_REGISTRY
            .lock()
            .unwrap()
            .host_path_exists_closure
            .get(&thread::current().id())
            .map_or_else(
                || 1,
                |c| match c(path) {
                    Ok(e) => {
                        unsafe {
                            *exists = e as u8;
                        }
                        0
                    }
                    Err(e) => e,
                },
            )
    }

    pub fn set_get_host_os_impl<F>(closure: F)
    where
        F: Fn() -> Result<String, u32> + Send + 'static,
    {
        MOCK_RESULT_REGISTRY
            .lock()
            .unwrap()
            .get_host_os_closure
            .insert(thread::current().id(), Box::new(closure));
    }

    fn execute_get_host_os(os_name: *mut u8, os_name_length: *mut u32) -> u32 {
        MOCK_RESULT_REGISTRY
            .lock()
            .unwrap()
            .get_host_os_closure
            .get(&thread::current().id())
            .map_or_else(
                || 1,
                |c| match c() {
                    Ok(os) => {
                        let buff = unsafe {
                            std::slice::from_raw_parts_mut(os_name, std::cmp::min(os.len(), 128))
                        };
                        buff.clone_from_slice(os.as_bytes());
                        unsafe {
                            *os_name_length = os.len() as u32;
                        }
                        0
                    }
                    Err(e) => e,
                },
            )
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
        F: Fn(&str) -> Result<Channel, u32> + 'static + Send,
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
        F: Fn(&str, Channel) -> Result<(), u32> + 'static + Send,
    {
        MOCK_RESULT_REGISTRY
            .lock()
            .unwrap()
            .set_output_closure
            .insert(thread::current().id(), Box::new(closure));
    }

    fn execute_set_output(
        key_ptr: *const u8,
        key_len: usize,
        value_ptr: *const u8,
        value_len: usize,
    ) -> u32 {
        let key = unsafe {
            let slice = std::slice::from_raw_parts(key_ptr, key_len);
            std::str::from_utf8(slice).unwrap()
        };

        let mut vec = Vec::with_capacity(value_len);
        unsafe {
            value_ptr.copy_to(vec.as_mut_ptr(), value_len);
            vec.set_len(value_len);
        }

        let return_value = Channel::decode(vec.as_slice()).unwrap();
        MOCK_RESULT_REGISTRY
            .lock()
            .unwrap()
            .set_output_closure
            .get(&thread::current().id())
            .map_or_else(
                || 1,
                |c| match c(key, return_value) {
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
        F: Fn(&str) -> Result<i32, u32> + 'static + Send,
    {
        MOCK_RESULT_REGISTRY
            .lock()
            .unwrap()
            .connect_closure
            .insert(thread::current().id(), Box::new(closure));
    }

    #[cfg(feature = "net")]
    fn execute_connect(addr_ptr: *const u8, addr_len: usize, file_descriptor: *mut i32) -> u32 {
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

    pub fn set_input_stream(stream: Stream) {
        let channel_lengths: HashMap<String, usize> = stream
            .channels
            .iter()
            .map(|(name, channel)| (name.clone(), channel.encoded_len()))
            .collect();
        MockResultRegistry::set_get_input_len_impl(move |key| match channel_lengths.get(key) {
            None => Err(1),
            Some(len) => Ok(*len),
        });

        MockResultRegistry::set_get_input_impl(move |key| match stream.get_channel(key) {
            None => Err(1),
            Some(channel) => Ok(channel.clone()),
        });
    }
}
