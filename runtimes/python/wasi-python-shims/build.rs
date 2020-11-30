use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let crate_dir = env::var("CARGO_MANIFEST_DIR")?;

    println!("cargo:rerun-if-changed=cbindgen.toml");
    println!("cargo:rerun-if-changed=wasi_python_shims.h");

    cbindgen::generate(crate_dir)
        .expect("Unable to generate C header")
        .write_to_file("wasi_python_shims.h");

    Ok(())
}
