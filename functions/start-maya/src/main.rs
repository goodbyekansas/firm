#[link(wasm_import_module = "gbk")]
extern "C" {
    fn start_process(string_ptr: *const u8, len: usize) -> i64;
}

pub mod gbk {
    use crate::start_process;
    pub fn start_dcc(name: &str) -> bool {
        let ri64 = unsafe { start_process(name.as_ptr(), name.len()) };
        return ri64 != 0;
    }
}

fn main() {
    println!("Hello! I will start maya from WASI now!");

    if gbk::start_dcc("maya") {
        println!("started maya")
    } else {
        println!("failed to start maya because of reasons")
    };
}
