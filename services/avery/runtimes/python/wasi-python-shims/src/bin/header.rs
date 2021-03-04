use std::{env, path::PathBuf};

pub fn main() {
    let out_file = PathBuf::from(env!("OUT_DIR")).join("wasi_python_shims.h");
    println!("{}", out_file.display());
}
