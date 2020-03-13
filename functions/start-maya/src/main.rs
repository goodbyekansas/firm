#[link(wasm_import_module = "gbk")]
extern "C" {
    fn start_host_process(string_ptr: *const u8, len: usize) -> i64;
}

pub mod gbk {
    use crate::start_host_process as raw_start_host_process;
    pub fn start_host_process(name: &str) -> bool {
        let ri64 = unsafe { raw_start_host_process(name.as_ptr(), name.len()) };
        return ri64 != 0;
    }
}

fn main() {
    println!("Hello! I will start maya from WASI now!");

    if gbk::start_host_process("/usr/autodesk/maya2019/bin/maya") {
        println!("started maya")
    } else {
        println!("failed to start maya because of reasons")
    };
}
