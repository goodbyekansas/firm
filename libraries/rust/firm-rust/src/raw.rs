#[link(wasm_import_module = "firm")]
extern "C" {

    pub fn get_attachment_path_len(
        attachment_name_ptr: *const u8,
        attachment_name_len: usize,
        path_len: *mut usize,
    ) -> u32;

    pub fn map_attachment(
        attachment_name_ptr: *const u8,
        attachment_name_len: usize,
        unpack: u8,
        path_ptr: *mut u8,
        path_len: usize,
    ) -> u32;

    pub fn host_path_exists(path: *const u8, path_len: usize, exists: *mut u8) -> u32;

    pub fn get_host_os(os_name: *mut u8, output_os_name_len: *mut u32) -> u32;

    pub fn get_attachment_path_len_from_descriptor(
        attachment_descriptor_ptr: *const u8,
        attachment_descriptor_len: usize,
        path_len: *mut usize,
    ) -> u32;

    pub fn map_attachment_from_descriptor(
        attachment_descriptor_ptr: *const u8,
        attachment_descriptor_len: usize,
        unpack: u8,
        path_ptr: *mut u8,
        path_len: usize,
    ) -> u32;

    pub fn start_host_process(request_ptr: *const u8, len: usize, pid: *mut u64) -> u32;
    pub fn run_host_process(request_ptr: *const u8, len: usize, exit_code: *mut i32) -> u32;
    pub fn get_input_len(key_ptr: *const u8, len: usize, value: *mut u64) -> u32;
    pub fn get_input(
        key_ptr: *const u8,
        key_len: usize,
        value_ptr: *mut u8,
        value_len: usize,
    ) -> u32;
    pub fn set_output(
        key_ptr: *const u8,
        key_len: usize,
        value_ptr: *const u8,
        value_len: usize,
    ) -> u32;
    pub fn set_error(msg_ptr: *const u8, msg_len: usize) -> u32;

    #[cfg(feature = "net")]
    pub fn connect(addr_ptr: *const u8, addr_len: usize, file_descriptor: *mut i32) -> u32;
}
