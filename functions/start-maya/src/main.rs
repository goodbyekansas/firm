pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/functions.rs"));
}

#[link(wasm_import_module = "gbk")]
extern "C" {
    pub fn start_host_process(string_ptr: *const u8, len: usize, pid: *mut u64) -> u32;
    pub fn get_input_len(key_ptr: *const u8, len: usize, value: *mut u64) -> u32;
    pub fn get_input(
        key_ptr: *const u8,
        key_len: usize,
        value_ptr: *const u8,
        value_len: usize,
    ) -> u32;
    pub fn set_output(value_ptr: *const u8, value_len: usize) -> u32;
    pub fn set_error(msg_ptr: *const u8, msg_len: usize) -> u32;
}

pub mod gbk {
    pub use crate::proto::{ArgumentType, FunctionArgument, ReturnValue};
    use prost::Message;
    use thiserror::Error;

    mod raw {
        pub use crate::{get_input, get_input_len, set_error, set_output, start_host_process};
    }

    trait ToResult: Copy {
        fn to_result(self) -> Result<(), Error>;
    }

    impl ToResult for u32 {
        fn to_result(self) -> Result<(), Error> {
            match self {
                0 => Ok(()),
                ec => Err(Error::HostError(ec)),
            }
        }
    }

    #[derive(Error, Debug)]
    pub enum Error {
        #[error("Unknown error occurred: {0}")]
        Unknown(String),

        #[error("Failed to start process.")]
        FailedToStartProcess(),

        #[error("Failed to decode: {0}")]
        FailedToDecodeResult(#[from] prost::DecodeError),

        #[error("Failed to encode: {0}")]
        FailedToEncodeReturnValue(#[from] prost::EncodeError),

        #[error("Host error occured. Error code: {0}")]
        HostError(u32),

        #[error("Failed to convert to requested type")]
        ConversionError(),
    }

    pub fn start_host_process(name: &str) -> Result<u64, Error> {
        let mut pid: u64 = 0;
        unsafe { raw::start_host_process(name.as_ptr(), name.len(), &mut pid as *mut u64) }
            .to_result()
            .map(|_| pid)
    }

    pub trait FromFunctionArgument: Sized {
        fn from_arg(arg: FunctionArgument) -> Option<Self>;
    }

    impl FromFunctionArgument for String {
        fn from_arg(arg: FunctionArgument) -> Option<Self> {
            String::from_utf8(arg.value).ok()
        }
    }

    macro_rules! bytes_as_64_bit_array {
        ($bytes: expr) => {{
            [
                $bytes.get(0).cloned().unwrap_or_default(),
                $bytes.get(1).cloned().unwrap_or_default(),
                $bytes.get(2).cloned().unwrap_or_default(),
                $bytes.get(3).cloned().unwrap_or_default(),
                $bytes.get(4).cloned().unwrap_or_default(),
                $bytes.get(5).cloned().unwrap_or_default(),
                $bytes.get(6).cloned().unwrap_or_default(),
                $bytes.get(7).cloned().unwrap_or_default(),
            ]
        }};
    }

    macro_rules! host_call {
        ($call: expr) => {
            unsafe { $call }.to_result()
        };
    }

    impl FromFunctionArgument for i64 {
        fn from_arg(arg: FunctionArgument) -> Option<Self> {
            Some(i64::from_le_bytes(bytes_as_64_bit_array!(arg.value)))
        }
    }

    impl FromFunctionArgument for f64 {
        fn from_arg(arg: FunctionArgument) -> Option<Self> {
            Some(f64::from_le_bytes(bytes_as_64_bit_array!(arg.value)))
        }
    }

    impl FromFunctionArgument for bool {
        fn from_arg(arg: FunctionArgument) -> Option<Self> {
            arg.value.first().map(|b| *b != 0)
        }
    }

    pub fn get_input<T: FromFunctionArgument>(key: &str) -> Result<T, Error> {
        let mut size: u64 = 0;
        host_call!(raw::get_input_len(
            key.as_ptr(),
            key.len(),
            &mut size as *mut u64
        ))?;

        let mut value_buffer = Vec::with_capacity(size as usize);
        host_call!(raw::get_input(
            key.as_ptr(),
            key.len(),
            value_buffer.as_mut_ptr(),
            size as usize,
        ))?;

        FunctionArgument::decode(value_buffer.as_slice())
            .map_err(|e| e.into())
            .and_then(|a| T::from_arg(a).ok_or_else(Error::ConversionError))
    }

    pub fn set_output(ret_value: &ReturnValue) -> Result<(), Error> {
        let mut value = Vec::with_capacity(ret_value.encoded_len());
        ret_value.encode(&mut value)?;
        host_call!(raw::set_output(value.as_mut_ptr(), value.len()))
    }

    pub fn set_error(msg: &str) -> Result<(), Error> {
        host_call!(raw::set_error(msg.as_ptr(), msg.len()))
    }
}

fn main() {
    println!("Hello! I will start maya from WASI now!");

    let maya_version = gbk::get_input("version")
        .ok()
        .unwrap_or_else(|| "2019".to_owned());

    match gbk::start_host_process(&format!("/usr/autodesk/maya{}/bin/maya", maya_version)) {
        Ok(pid) => {
            println!("started maya");
            gbk::set_output(&gbk::ReturnValue {
                name: "pid".to_owned(),
                r#type: gbk::ArgumentType::Int as i32,
                value: pid.to_le_bytes().to_vec(),
            })
            .map_or_else(|e| println!("Failed to set output: {}", e), |_| ()); // 🕍🥿🕍 🎾
        }
        Err(e) => {
            gbk::set_error(&format!("Failed to start maya 🛕 because of: {}", e))
                .map_or_else(|e| println!("failed to set error: {}", e), |_| ());
        }
    };
}
