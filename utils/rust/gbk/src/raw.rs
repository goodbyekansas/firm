#[link(wasm_import_module = "gbk")]
extern "C" {
    pub fn start_host_process(request_ptr: *const u8, len: usize, pid: *mut u64) -> u32;
    pub fn run_host_process(request_ptr: *const u8, len: usize, exit_code: *mut i32) -> u32;
    pub fn get_input_len(key_ptr: *const u8, len: usize, value: *mut u64) -> u32;
    pub fn get_input(
        key_ptr: *const u8,
        key_len: usize,
        value_ptr: *mut u8,
        value_len: usize,
    ) -> u32;
    pub fn set_output(value_ptr: *const u8, value_len: usize) -> u32;
    pub fn set_error(msg_ptr: *const u8, msg_len: usize) -> u32;

    #[cfg(feature = "net")]
    pub fn connect(addr_ptr: *const u8, addr_len: usize, file_descriptor: *mut u32) -> u32;
}
