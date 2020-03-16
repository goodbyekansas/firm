pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/functions.rs"));
}

#[link(wasm_import_module = "gbk")]
extern "C" {
    pub fn start_host_process(string_ptr: *const u8, len: usize) -> i64;
    pub fn get_input_len(key_ptr: *const u8, len: usize) -> usize;
    pub fn get_input(
        key_ptr: *const u8,
        key_len: usize,
        value_ptr: *const u8,
        value_len: usize,
    ) -> usize;
}

pub mod gbk {
    pub use crate::proto::FunctionArgument;
    use prost::Message;
    use thiserror::Error;

    mod raw {
        pub use crate::{get_input, get_input_len, start_host_process};
    }

    #[derive(Error, Debug)]
    pub enum Error {
        #[error("Unknown error occurred: {0}")]
        Unknown(String),

        #[error("Failed to start process.")]
        FailedToStartProcess(),

        #[error("Failed to decode: {0}")]
        FailedToDecodeResult(#[from] prost::DecodeError),

        #[error("Failed to get input \"{value_name}\". Requested {requested_byte_count} bytes to be written as result but got {written_byte_count}.")]
        InputWriteFailed {
            value_name: String,
            requested_byte_count: usize,
            written_byte_count: usize,
        },

        #[error("Failed to find input with key \"{0}\"")]
        InputNotFound(String),
    }

    pub fn start_host_process(name: &str) -> Result<(), Error> {
        let ri64 = unsafe { raw::start_host_process(name.as_ptr(), name.len()) };

        if ri64 != 0 {
            Ok(())
        } else {
            Err(Error::FailedToStartProcess())
        }
    }

    pub fn get_input(key: &str) -> Result<FunctionArgument, Error> {
        let size = unsafe { raw::get_input_len(key.as_ptr(), key.len()) };

        if size == 0 {
            return Err(Error::InputNotFound(key.to_owned()));
        }

        let mut value_buffer = Vec::with_capacity(size);
        let written_bytes =
            unsafe { raw::get_input(key.as_ptr(), key.len(), value_buffer.as_mut_ptr(), size) };

        if written_bytes != size {
            Err(Error::InputWriteFailed {
                value_name: key.to_owned(),
                requested_byte_count: size,
                written_byte_count: written_bytes,
            })
        } else {
            FunctionArgument::decode(value_buffer.as_slice()).map_err(|e| e.into())
        }
    }
}

fn main() {
    println!("Hello! I will start maya from WASI now!");

    match gbk::start_host_process("/usr/autodesk/maya2019/bin/maya") {
        Ok(_) => println!("started maya"),
        Err(e) => println!("failed to start maya because of: {}", e),
    };
}
