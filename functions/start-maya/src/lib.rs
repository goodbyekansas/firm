
// Export a function named "hello_wasm". This can be called
// from the embedder!
#[no_mangle]
pub extern "C" fn main() {
    // Call the function we just imported and pass in
    // the offset of our string and its length as parameters.
    println!("Hello! I R rust wasmer!");
}

