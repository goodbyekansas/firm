use std::{env, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if env::var("WASI_PYTHON_SHIMS_SKIP_C_BINDGEN").is_ok() {
        return Ok(());
    }

    println!("cargo:rerun-if-changed=cbindgen.toml");
    println!("cargo:rerun-if-changed=src");

    cbindgen::generate(PathBuf::from(env::var("CARGO_MANIFEST_DIR")?))
        .expect("Unable to generate C header")
        .write_to_file(PathBuf::from(env::var("OUT_DIR")?).join("wasi_python_shims.h"));

    Ok(())
}
