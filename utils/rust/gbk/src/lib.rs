pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/functions.rs"));
}

use std::collections::{hash_map::RandomState, HashMap};

use prost::Message;
pub use proto::ReturnValue;
use proto::{ArgumentType, FunctionArgument, StartProcessRequest};
use thiserror::Error;

mod raw {
    #[link(wasm_import_module = "gbk")]
    extern "C" {
        pub fn start_host_process(request_ptr: *const u8, len: usize, pid: *mut u64) -> u32;
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
    #[error("Failed to decode: {0}")]
    FailedToDecodeResult(#[from] prost::DecodeError),

    #[error("Failed to encode: {0}")]
    FailedToEncodeReturnValue(#[from] prost::EncodeError),

    #[error("Host error occured. Error code: {0}")]
    HostError(u32),

    #[error("Failed to convert to requested type")]
    ConversionError(),
}

macro_rules! host_call {
    ($call: expr) => {
        unsafe { $call }.to_result()
    };
}

pub fn start_host_process<S: AsRef<str>>(
    name: &str,
    args: &[S],
    environment: &HashMap<String, String, RandomState>,
) -> Result<u64, Error> {
    let request = StartProcessRequest {
        command: name.to_owned(),
        args: args.iter().map(|s| s.as_ref().to_owned()).collect(),
        environment_variables: environment.clone(),
    };

    let mut value = Vec::with_capacity(request.encoded_len());
    request.encode(&mut value)?;

    let mut pid: u64 = 0;
    host_call!(raw::start_host_process(
        value.as_ptr(),
        value.len(),
        &mut pid as *mut u64
    ))
    .map(|_| pid)
}

pub trait FromFunctionArgument: Sized {
    fn from_arg(arg: FunctionArgument) -> Option<Self>;
}

pub trait ToReturnValue: Sized {
    fn to_return_value(&self, name: &str) -> ReturnValue;
}

impl FromFunctionArgument for String {
    fn from_arg(arg: FunctionArgument) -> Option<Self> {
        String::from_utf8(arg.value).ok()
    }
}

impl ToReturnValue for &str {
    fn to_return_value(&self, name: &str) -> ReturnValue {
        ReturnValue {
            name: name.to_owned(),
            value: self.as_bytes().to_vec(),
            r#type: ArgumentType::String as i32,
        }
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

impl FromFunctionArgument for i64 {
    fn from_arg(arg: FunctionArgument) -> Option<Self> {
        Some(i64::from_le_bytes(bytes_as_64_bit_array!(arg.value)))
    }
}

impl ToReturnValue for i64 {
    fn to_return_value(&self, name: &str) -> ReturnValue {
        ReturnValue {
            name: name.to_owned(),
            value: self.to_le_bytes().to_vec(),
            r#type: ArgumentType::Int as i32,
        }
    }
}

impl FromFunctionArgument for f64 {
    fn from_arg(arg: FunctionArgument) -> Option<Self> {
        Some(f64::from_le_bytes(bytes_as_64_bit_array!(arg.value)))
    }
}

impl ToReturnValue for f64 {
    fn to_return_value(&self, name: &str) -> ReturnValue {
        ReturnValue {
            name: name.to_owned(),
            value: self.to_le_bytes().to_vec(),
            r#type: ArgumentType::Float as i32,
        }
    }
}

impl FromFunctionArgument for bool {
    fn from_arg(arg: FunctionArgument) -> Option<Self> {
        arg.value.first().map(|b| *b != 0)
    }
}

impl ToReturnValue for bool {
    fn to_return_value(&self, name: &str) -> ReturnValue {
        ReturnValue {
            name: name.to_owned(),
            value: vec![*self as u8],
            r#type: ArgumentType::Bool as i32,
        }
    }
}

impl FromFunctionArgument for Vec<u8> {
    fn from_arg(arg: FunctionArgument) -> Option<Self> {
        Some(arg.value)
    }
}

impl ToReturnValue for Vec<u8> {
    fn to_return_value(&self, name: &str) -> ReturnValue {
        ReturnValue {
            name: name.to_owned(),
            value: self.to_vec(),
            r#type: ArgumentType::Bytes as i32,
        }
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
    unsafe {
        value_buffer.set_len(size as usize);
    }
    FunctionArgument::decode(value_buffer.as_slice())
        .map_err(|e| e.into())
        .and_then(|a| T::from_arg(a).ok_or_else(Error::ConversionError))
}

pub fn set_output<T: ToReturnValue>(name: &str, value: &T) -> Result<(), Error> {
    let ret_value = value.to_return_value(name);
    let mut value = Vec::with_capacity(ret_value.encoded_len());
    ret_value.encode(&mut value)?;
    host_call!(raw::set_output(value.as_mut_ptr(), value.len()))
}

pub fn set_error(msg: &str) -> Result<(), Error> {
    host_call!(raw::set_error(msg.as_ptr(), msg.len()))
}
