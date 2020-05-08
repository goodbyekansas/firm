pub unsafe fn start_host_process(_request_ptr: *const u8, _len: usize, _pid: *mut u64) -> u32 {
    1
}

pub unsafe fn run_host_process(_request_ptr: *const u8, _len: usize, _exit_code: *mut i32) -> u32 {
    1
}

pub unsafe fn get_input_len(_key_ptr: *const u8, _len: usize, _value: *mut u64) -> u32 {
    1
}

pub unsafe fn get_input(
    _key_ptr: *const u8,
    _key_len: usize,
    _value_ptr: *const u8,
    _value_len: usize,
) -> u32 {
    1
}

pub unsafe fn set_output(_value_ptr: *const u8, _value_len: usize) -> u32 {
    1
}

pub unsafe fn set_error(_msg_ptr: *const u8, _msg_len: usize) -> u32 {
    1
}

#[cfg(feature = "net")]
pub unsafe fn connect(_addr_ptr: *const u8, _addr_len: usize, _file_descriptor: *mut u32) -> u32 {
    1
}
