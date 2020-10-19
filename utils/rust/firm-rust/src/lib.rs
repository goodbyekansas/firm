#![deny(warnings)]
#![cfg_attr(all(target_os = "wasi", feature = "net"), feature(wasi_ext))]

#[cfg(not(target_os = "wasi"))]
compile_error!("WASI function helper lib only supports running in WASI");

use std::{
    borrow::Borrow,
    collections::{hash_map::RandomState, HashMap},
    path::PathBuf,
};

use prost::Message;
use thiserror::Error;

pub use function_protocols::execution::OutputValue;
use function_protocols::{
    execution::InputValue,
    functions::{Attachment, Type},
    wasi::StartProcessRequest,
};

#[cfg(all(not(test), not(feature = "mock")))]
mod raw;

#[cfg(any(test, feature = "mock"))]
pub mod mock;

#[cfg(any(test, feature = "mock"))]
use mock as raw;

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
    FailedToEncodeOutputValue(#[from] prost::EncodeError),

    #[error("Host error occured. Error code: {0}")]
    HostError(u32),

    #[error("Failed to convert to requested type")]
    ConversionError(),

    #[error("Failed to find input \"{0}\"")]
    FailedToFindInput(String),

    #[error("Failed to find attachment \"{0}\"")]
    FailedToFindAttachment(String),
}

macro_rules! host_call {
    ($call: expr) => {
        unsafe { $call }.to_result()
    };
}

fn _map_attachment<S: AsRef<str> + std::fmt::Display>(
    attachment_name: S,
    unpack: bool,
) -> Result<PathBuf, Error> {
    let mut attachment_path_bytes_len: usize = 0;
    host_call!(raw::get_attachment_path_len(
        attachment_name.as_ref().as_ptr(),
        attachment_name.as_ref().as_bytes().len(),
        &mut attachment_path_bytes_len as *mut usize
    ))?;

    let mut attachment_path_buffer = Vec::with_capacity(attachment_path_bytes_len as usize);
    host_call!(raw::map_attachment(
        attachment_name.as_ref().as_ptr(),
        attachment_name.as_ref().as_bytes().len(),
        unpack as u8,
        attachment_path_buffer.as_mut_ptr(),
        attachment_path_bytes_len as usize,
    ))?;
    unsafe { attachment_path_buffer.set_len(attachment_path_bytes_len as usize) };

    Ok(PathBuf::from(
        String::from_utf8(attachment_path_buffer).map_err(|_| Error::ConversionError())?,
    ))
}

/// Map an attachment that the WASI host knows about, given by `attachment_name`.
///
/// Since it is known by the host, the provided `attachment_name` is enough
/// to resolve it.
pub fn map_attachment<S: AsRef<str> + std::fmt::Display>(
    attachment_name: S,
) -> Result<PathBuf, Error> {
    _map_attachment(attachment_name, false)
}

/// Map an attachment that the WASI host knows about, given by `attachment_name` and unpack it.
///
/// Since it is known by the host, the provided `attachment_name` is enough
/// to resolve it.
pub fn map_attachment_and_unpack<S: AsRef<str> + std::fmt::Display>(
    attachment_name: S,
) -> Result<PathBuf, Error> {
    _map_attachment(attachment_name, true)
}

fn _map_attachment_from_descriptor(
    attachment_descriptor: &Attachment,
    unpack: bool,
) -> Result<PathBuf, Error> {
    let mut attachment_path_bytes_len: usize = 0;

    let mut value = Vec::with_capacity(attachment_descriptor.encoded_len());
    attachment_descriptor.encode(&mut value)?;
    host_call!(raw::get_attachment_path_len_from_descriptor(
        value.as_ptr(),
        value.len(),
        &mut attachment_path_bytes_len as *mut usize
    ))?;

    let mut attachment_path_buffer = Vec::with_capacity(attachment_path_bytes_len as usize);
    host_call!(raw::map_attachment_from_descriptor(
        value.as_ptr(),
        value.len(),
        unpack as u8,
        attachment_path_buffer.as_mut_ptr(),
        attachment_path_bytes_len as usize,
    ))?;
    unsafe { attachment_path_buffer.set_len(attachment_path_bytes_len as usize) };

    Ok(PathBuf::from(
        String::from_utf8(attachment_path_buffer).map_err(|_| Error::ConversionError())?,
    ))
}

/// Map an attachment from a descriptor that the WASI host does not know about.
pub fn map_attachment_from_descriptor(
    attachment_descriptor: &Attachment,
) -> Result<PathBuf, Error> {
    _map_attachment_from_descriptor(attachment_descriptor, false)
}

/// Map an attachment from a descriptor that the WASI host does not know about and unpack it.
pub fn map_attachment_from_descriptor_and_unpack(
    attachment_descriptor: &Attachment,
) -> Result<PathBuf, Error> {
    _map_attachment_from_descriptor(attachment_descriptor, true)
}

/// Checks if a path on the hosts file system exists (outside the wasi file system)
pub fn host_path_exists<S: AsRef<str>>(path: S) -> Result<bool, Error> {
    let mut exists: u8 = 0;

    host_call!(raw::host_path_exists(
        path.as_ref().as_ptr(),
        path.as_ref().len(),
        &mut exists as *mut u8
    ))
    .map(|_| exists != 0)
}

/// Get the name of the OS of the host
pub fn get_host_os() -> Result<String, Error> {
    let mut name = String::with_capacity(128);
    let mut host_os_name_length: u32 = 0;
    host_call!(raw::get_host_os(
        name.as_mut_ptr(),
        &mut host_os_name_length as *mut u32
    ))
    .map(|_| {
        unsafe {
            name.as_mut_vec().set_len(host_os_name_length as usize);
        }

        name
    })
}

/// Start a process on the host
///
/// `name` is the executable to run, `args` the command line arguments to it
/// and `environment` is a mapping of environment vars to use
/// for the launched process. This method returns the pid but does
/// not wait for exit of the process.
pub fn start_host_process<S1: AsRef<str>, S2: AsRef<str>>(
    name: S1,
    args: &[S2],
    environment: &HashMap<String, String, RandomState>,
) -> Result<u64, Error> {
    let request = StartProcessRequest {
        command: name.as_ref().to_owned(),
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

/// Run a process on the host
///
/// `name` is the executable to run, `args` the command line arguments to it
/// and `environment` is a mapping of environment vars to use
/// for the launched process. This method waits for the process
/// to exit and therefore returns the exit code.
pub fn run_host_process<S1: AsRef<str>, S2: AsRef<str>>(
    name: S1,
    args: &[S2],
    environment: &HashMap<String, String, RandomState>,
) -> Result<i32, Error> {
    let request = StartProcessRequest {
        command: name.as_ref().to_owned(),
        args: args.iter().map(|s| s.as_ref().to_owned()).collect(),
        environment_variables: environment.clone(),
    };

    let mut value = Vec::with_capacity(request.encoded_len());
    request.encode(&mut value)?;

    let mut exit_code: i32 = 0;
    host_call!(raw::run_host_process(
        value.as_ptr(),
        value.len(),
        &mut exit_code as *mut i32
    ))
    .map(|_| exit_code)
}

pub trait FromInputValue: Sized {
    fn from_arg(arg: &InputValue) -> Option<Self>;
}

pub trait ToOutputValue: Sized {
    fn to_return_value(&self, name: &str) -> OutputValue;
}

impl FromInputValue for String {
    fn from_arg(arg: &InputValue) -> Option<Self> {
        match Type::from_i32(arg.r#type) {
            Some(Type::String) => String::from_utf8(arg.value.clone()).ok(),
            _ => None,
        }
    }
}

impl<'a, T> ToOutputValue for &'a T
where
    T: ToOutputValue + 'a,
{
    fn to_return_value(&self, name: &str) -> OutputValue {
        T::to_return_value(self, name)
    }
}

impl ToOutputValue for &str {
    fn to_return_value(&self, name: &str) -> OutputValue {
        OutputValue {
            name: name.to_owned(),
            value: self.as_bytes().to_vec(),
            r#type: Type::String as i32,
        }
    }
}

impl ToOutputValue for String {
    fn to_return_value(&self, name: &str) -> OutputValue {
        OutputValue {
            name: name.to_owned(),
            value: self.as_bytes().to_vec(),
            r#type: Type::String as i32,
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

impl FromInputValue for i64 {
    fn from_arg(arg: &InputValue) -> Option<Self> {
        match Type::from_i32(arg.r#type) {
            Some(Type::Int) => {
                if arg.value.len() == 8 {
                    Some(i64::from_le_bytes(bytes_as_64_bit_array!(arg.value)))
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

impl ToOutputValue for i64 {
    fn to_return_value(&self, name: &str) -> OutputValue {
        OutputValue {
            name: name.to_owned(),
            value: self.to_le_bytes().to_vec(),
            r#type: Type::Int as i32,
        }
    }
}

impl FromInputValue for f64 {
    fn from_arg(arg: &InputValue) -> Option<Self> {
        match Type::from_i32(arg.r#type) {
            Some(Type::Float) => {
                if arg.value.len() == 8 {
                    Some(f64::from_le_bytes(bytes_as_64_bit_array!(arg.value)))
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

impl ToOutputValue for f64 {
    fn to_return_value(&self, name: &str) -> OutputValue {
        OutputValue {
            name: name.to_owned(),
            value: self.to_le_bytes().to_vec(),
            r#type: Type::Float as i32,
        }
    }
}

impl FromInputValue for bool {
    fn from_arg(arg: &InputValue) -> Option<Self> {
        match Type::from_i32(arg.r#type) {
            Some(Type::Bool) => arg.value.first().map(|b| *b != 0),
            _ => None,
        }
    }
}

impl ToOutputValue for bool {
    fn to_return_value(&self, name: &str) -> OutputValue {
        OutputValue {
            name: name.to_owned(),
            value: vec![*self as u8],
            r#type: Type::Bool as i32,
        }
    }
}

impl FromInputValue for Vec<u8> {
    fn from_arg(arg: &InputValue) -> Option<Self> {
        match Type::from_i32(arg.r#type) {
            Some(Type::Bytes) => Some(arg.value.clone()),
            _ => None,
        }
    }
}

impl ToOutputValue for Vec<u8> {
    fn to_return_value(&self, name: &str) -> OutputValue {
        OutputValue {
            name: name.to_owned(),
            value: self.to_vec(),
            r#type: Type::Bytes as i32,
        }
    }
}

pub fn get_input<S: AsRef<str>, T: FromInputValue>(key: S) -> Result<T, Error> {
    let mut size: u64 = 0;
    host_call!(raw::get_input_len(
        key.as_ref().as_ptr(),
        key.as_ref().as_bytes().len(),
        &mut size as *mut u64
    ))?;

    let mut value_buffer = Vec::with_capacity(size as usize);
    host_call!(raw::get_input(
        key.as_ref().as_ptr(),
        key.as_ref().as_bytes().len(),
        value_buffer.as_mut_ptr(),
        size as usize,
    ))?;
    unsafe {
        value_buffer.set_len(size as usize);
    }
    InputValue::decode(value_buffer.as_slice())
        .map_err(|e| e.into())
        .and_then(|a| T::from_arg(&a).ok_or_else(Error::ConversionError))
}

pub fn set_output<S: AsRef<str>, T: ToOutputValue>(name: S, value: T) -> Result<(), Error> {
    let ret_value = value.borrow().to_return_value(name.as_ref());
    set_output_with_return_value(&ret_value)
}

pub fn set_output_with_return_value(ret_value: &OutputValue) -> Result<(), Error> {
    let mut value = Vec::with_capacity(ret_value.encoded_len());
    ret_value.encode(&mut value)?;
    host_call!(raw::set_output(value.as_mut_ptr(), value.len()))
}

pub fn set_error<S: AsRef<str>>(msg: S) -> Result<(), Error> {
    host_call!(raw::set_error(
        msg.as_ref().as_ptr(),
        msg.as_ref().as_bytes().len()
    ))
}

pub mod executor {
    use std::path::PathBuf;

    use function_protocols::{
        execution::InputValue,
        functions::Attachment,
        wasi::{Attachments, InputValues},
    };
    use prost::Message;

    use crate::{get_input, map_attachment_from_descriptor, Error, FromInputValue};

    pub trait AttachmentDownload {
        fn download(&self) -> Result<PathBuf, Error>;
    }

    impl AttachmentDownload for Attachment {
        fn download(&self) -> Result<PathBuf, Error> {
            map_attachment_from_descriptor(self)
        }
    }

    /// Special function inputs for
    /// functions that are executors
    #[derive(Debug)]
    pub struct ExecutorArgs {
        code: Attachment,
        sha256: String,
        entrypoint: String,
        arguments: Vec<InputValue>,
        attachments: Vec<Attachment>,
    }

    impl ExecutorArgs {
        /// Create execution environment args from the wasi host
        pub fn from_wasi_host() -> Result<Self, Error> {
            Ok(Self {
                code: get_input("_code").and_then(|a: Vec<u8>| {
                    Attachment::decode(a.as_slice()).map_err(|e| e.into())
                })?,
                sha256: get_input("_sha256")?,
                entrypoint: get_input("_entrypoint")?,
                arguments: get_input("_arguments").and_then(|a: Vec<u8>| {
                    InputValues::decode(a.as_slice())
                        .map_err(|e| e.into())
                        .map(|v| v.values)
                })?,
                attachments: get_input("_attachments").and_then(|a: Vec<u8>| {
                    Attachments::decode(a.as_slice())
                        .map_err(|e| e.into())
                        .map(|a| a.attachments)
                })?,
            })
        }

        /// Get the code that the execution environment is expected to execute
        pub fn code(&self) -> &Attachment {
            &self.code
        }

        /// Get the sha256 for the code that the execution environment is expected to execute
        pub fn sha256(&self) -> &str {
            &self.sha256
        }

        /// Get the entrypoint that the execution environment is expected to use
        pub fn entrypoint(&self) -> &str {
            &self.entrypoint
        }

        /// Get an argument designated by `key` for the
        /// function that the execution environment is
        /// expected to execute
        pub fn argument<S: AsRef<str>, T: FromInputValue>(&self, key: S) -> Result<T, Error> {
            self.get_argument_descriptor(key)
                .and_then(|a| T::from_arg(a).ok_or_else(Error::ConversionError))
        }

        /// Get an argument descriptor designated by `key` for the function that
        /// the execution environment is expected to execute
        pub fn get_argument_descriptor<S: AsRef<str>>(&self, key: S) -> Result<&InputValue, Error> {
            self.arguments
                .iter()
                .find(|a| a.name == key.as_ref())
                .ok_or_else(|| Error::FailedToFindInput(key.as_ref().to_owned()))
        }

        pub fn get_attachment_descriptor<S: AsRef<str>>(
            &self,
            name: S,
        ) -> Result<&Attachment, Error> {
            self.attachments
                .iter()
                .find(|a| a.name == name.as_ref())
                .ok_or_else(|| Error::FailedToFindAttachment(name.as_ref().to_owned()))
        }
    }
}

#[cfg(feature = "net")]
pub mod net {

    use std::{
        fs::File,
        io::{self, Read, Write},
        os::wasi::io::FromRawFd,
    };

    use super::{raw, ToResult};

    #[derive(Debug)]
    pub struct TcpConnection {
        file: File,
    }

    impl Read for TcpConnection {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.file.read(buf)
        }
    }

    impl Write for TcpConnection {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.file.write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.file.flush()
        }
    }

    pub fn connect<S: AsRef<str>>(address: S) -> Result<TcpConnection, super::Error> {
        let mut file_descriptor: u32 = 0;
        host_call!(raw::connect(
            address.as_ref().as_ptr(),
            address.as_ref().as_bytes().len(),
            &mut file_descriptor as *mut u32
        ))?;

        Ok(TcpConnection {
            file: unsafe { File::from_raw_fd(file_descriptor) },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use executor::AttachmentDownload;
    use function_protocols::{
        functions::{Attachment, AttachmentUrl, AuthMethod},
        wasi::InputValues,
    };
    use mock::MockResultRegistry;

    #[test]
    fn test_map_attachment() {
        let attachment_name = "attachment_0";
        let attachment_path = PathBuf::from("attachments").join(attachment_name);
        let attachment_len = attachment_path.to_string_lossy().as_bytes().len();

        MockResultRegistry::set_get_attachment_path_len_impl(move |att| {
            assert_eq!(attachment_name, att);
            Ok(attachment_len)
        });

        let attachment_path2 = attachment_path.clone();
        MockResultRegistry::set_map_attachment_impl(move |att, _| {
            assert_eq!(attachment_name, att);
            Ok(attachment_path2.clone().to_string_lossy().to_string())
        });

        let res = map_attachment(attachment_name);
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), attachment_path);

        MockResultRegistry::set_map_attachment_impl(|_, _| Err(11));
        let res = map_attachment(attachment_name);
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), Error::HostError(11)));

        MockResultRegistry::set_get_attachment_path_len_impl(|_| Err(10));
        let res = map_attachment(attachment_name);
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), Error::HostError(10)));
    }

    #[test]
    fn test_host_path_exists() {
        let datapata = "/sune/super-sune/mega-sune";
        MockResultRegistry::set_host_path_exists_impl(move |path| {
            assert_eq!(path, datapata);
            Ok(true)
        });

        assert!(host_path_exists(datapata).unwrap());

        MockResultRegistry::set_host_path_exists_impl(|_| Ok(false));

        assert!(!host_path_exists(datapata).unwrap());

        MockResultRegistry::set_host_path_exists_impl(|_| Err(123456));

        let res = host_path_exists(datapata);
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), Error::HostError(123456)));
    }

    #[test]
    fn test_get_host_os() {
        MockResultRegistry::set_get_host_os_impl(|| Ok(String::from("windows")));
        assert!(get_host_os().is_ok());
        assert_eq!(get_host_os().unwrap(), "windows");
        MockResultRegistry::set_get_host_os_impl(|| Err(666));
        assert!(get_host_os().is_err());
        assert!(matches!(get_host_os().unwrap_err(), Error::HostError(666)));
    }

    #[test]
    fn test_start_host_process() {
        let mut env = HashMap::new();
        env.insert("ur".to_owned(), "sula".to_owned());

        MockResultRegistry::set_start_host_process_impl(|req| {
            assert_eq!(req.command, "Sune".to_owned());
            assert_eq!(req.args, ["bune", "rune"]);
            assert!(req.environment_variables.contains_key("ur"));
            assert_eq!(req.environment_variables["ur"], "sula".to_owned());
            Ok(1337)
        });

        let res = start_host_process("Sune", &["bune", "rune"], &env);
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), 1337);

        // test failing
        MockResultRegistry::set_start_host_process_impl(|_req| -> Result<u64, u32> { Err(1) });

        let res = start_host_process("Sune", &["bune", "rune"], &env);
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), Error::HostError(_)));
    }

    #[test]
    fn test_run_host_process() {
        let mut env = HashMap::new();
        env.insert("ur".to_owned(), "sula".to_owned());

        MockResultRegistry::set_run_host_process_impl(|req| {
            assert_eq!(req.command, "Sune".to_owned());
            assert_eq!(req.args, ["bune", "rune"]);
            assert!(req.environment_variables.contains_key("ur"));
            assert_eq!(req.environment_variables["ur"], "sula".to_owned());

            Ok(25i32)
        });

        let res = run_host_process("Sune", &["bune", "rune"], &env);
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), 25);

        // test failing
        MockResultRegistry::set_run_host_process_impl(|_req| Err(1));

        let res = run_host_process("Sune", &["bune", "rune"], &env);
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), Error::HostError(_)));
    }

    #[test]
    fn test_get_input() {
        let fa = InputValue {
            name: "namn".to_owned(),
            r#type: Type::String as i32,
            value: "🏌️‍♂️".as_bytes().to_vec(),
        };
        let falen = fa.encoded_len();

        MockResultRegistry::set_get_input_len_impl(move |_key| Ok(falen));

        let cloned_fa = fa.clone();
        MockResultRegistry::set_get_input_impl(move |_key| Ok(cloned_fa.clone()));

        let res: Result<String, _> = get_input("kallebulasularula");
        assert!(res.is_ok());
        assert_eq!(res.unwrap().as_bytes(), fa.value.as_slice());

        // Asking for wrong type
        let res: Result<i64, _> = get_input("cool-grunka");
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), Error::ConversionError()));

        let res: Result<bool, _> = get_input("cool-grunka");
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), Error::ConversionError()));

        // Fail on length (how!?)
        MockResultRegistry::set_get_input_len_impl(move |_key| Err(1));

        let res: Result<String, _> = get_input("ful-grunka");
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), Error::HostError(_)));
    }

    #[test]
    fn test_set_output() {
        // String
        let name = "sugar";
        let value = "kalle";
        MockResultRegistry::set_set_output_impl(move |res| {
            assert_eq!(res.name, name);
            assert_eq!(std::str::from_utf8(&res.value).unwrap(), value);
            assert_eq!(Type::from_i32(res.r#type).unwrap(), Type::String);
            Ok(())
        });

        let res = set_output(name, value);
        assert!(res.is_ok());

        // int
        let name = "sugar int";
        let value = 50i64;
        MockResultRegistry::set_set_output_impl(move |res| {
            assert_eq!(res.name, name);
            assert_eq!(res.value, value.to_le_bytes());
            assert_eq!(Type::from_i32(res.r#type).unwrap(), Type::Int);
            Ok(())
        });

        let res = set_output(name, value);
        assert!(res.is_ok());

        // byte array
        let name = "sugar bytes";
        let value = vec![15, 74, 23, 65, 53];
        let cloned_value = value.clone();

        MockResultRegistry::set_set_output_impl(move |res| {
            assert_eq!(res.name, name);
            assert_eq!(res.value, value);
            assert_eq!(Type::from_i32(res.r#type).unwrap(), Type::Bytes);
            Ok(())
        });

        let res = set_output(name, cloned_value);
        assert!(res.is_ok());

        // float
        let name = "sugar floats";
        let value = 0.65;

        MockResultRegistry::set_set_output_impl(move |res| {
            assert_eq!(res.name, name);
            assert_eq!(res.value.len(), 8);
            assert!(
                (f64::from_le_bytes(bytes_as_64_bit_array!(res.value)) - value).abs()
                    < f64::EPSILON
            );
            assert_eq!(Type::from_i32(res.r#type).unwrap(), Type::Float);
            Ok(())
        });

        let res = set_output(name, value);
        assert!(res.is_ok());

        // bool
        let name = "sugar bool";
        let value = true;

        MockResultRegistry::set_set_output_impl(move |res| {
            assert_eq!(res.name, name);
            assert_eq!(res.value.len(), 1);
            assert_eq!(res.value[0], value as u8);
            assert_eq!(Type::from_i32(res.r#type).unwrap(), Type::Bool);
            Ok(())
        });

        let res = set_output(name, value);
        assert!(res.is_ok());

        // Set bad output
        let name = "sugar bool";
        let value = true;

        MockResultRegistry::set_set_output_impl(move |_res| Err(1));

        let res = set_output(name, value);
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), Error::HostError(_)));
    }

    #[test]
    pub fn test_set_error() {
        let message = "mah error";
        MockResultRegistry::set_set_error_impl(move |msg| {
            assert_eq!(msg, message);
            Ok(())
        });

        let res = set_error(message);
        assert!(res.is_ok());

        // test bad error
        MockResultRegistry::set_set_error_impl(|_msg| Err(1));

        let res = set_error(message);
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), Error::HostError(_)));
    }

    #[cfg(feature = "net")]
    mod net_tests {
        use super::*;

        use std::{io::Write, os::wasi::io::IntoRawFd};

        #[test]
        fn test_connect() {
            let address = "fabrikam.com:123";

            MockResultRegistry::set_connect_impl(move |in_addr| {
                assert_eq!(in_addr, address);
                Ok(std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .open("sune.txt")
                    .unwrap()
                    .into_raw_fd())
            });

            let tcp_message = "Cool \"network connection\" bro";
            {
                let tcp_connection = net::connect(address);
                assert!(tcp_connection.is_ok());
                tcp_connection
                    .unwrap()
                    .write_all(tcp_message.as_bytes())
                    .unwrap();
            }
            assert_eq!(std::fs::read_to_string("sune.txt").unwrap(), tcp_message);

            // error connection
            MockResultRegistry::set_connect_impl(move |_in_addr| Err(1));

            let tcp_connection = net::connect(address);
            assert!(tcp_connection.is_err());
            assert!(matches!(tcp_connection.unwrap_err(), Error::HostError(_)));
        }
    }

    #[test]
    fn test_exec_env() {
        // fake code attachment
        std::fs::write("sune.txt", "code lol").unwrap();
        MockResultRegistry::set_get_attachment_path_len_from_descriptor_impl(|_| Ok(8));
        MockResultRegistry::set_map_attachment_from_descriptor_impl(|_, _| {
            Ok("sune.txt".to_owned())
        });

        let arguments = InputValues {
            values: vec![
                InputValue {
                    name: "sune".to_owned(),
                    r#type: Type::Bool as i32,
                    value: vec![0u8],
                },
                InputValue {
                    name: "rune".to_owned(),
                    r#type: Type::String as i32,
                    value: "datta!".as_bytes().to_vec(),
                },
            ],
        };

        let mut buff = Vec::with_capacity(arguments.encoded_len());
        arguments.encode(&mut buff).unwrap();

        let code_attachment = Attachment {
            checksums: None,
            metadata: HashMap::new(),
            name: "code".to_owned(),
            url: Some(AttachmentUrl {
                url: "fake:///".to_owned(),
                auth_method: AuthMethod::None as i32,
            }),
            created_at: 0,
        };

        let mut code_buff = Vec::with_capacity(code_attachment.encoded_len());
        code_attachment.encode(&mut code_buff).unwrap();

        MockResultRegistry::set_inputs(&[
            InputValue {
                name: "_code".to_owned(),
                r#type: Type::Bytes as i32,
                value: code_buff,
            },
            InputValue {
                name: "_sha256".to_owned(),
                r#type: Type::String as i32,
                value: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                    .as_bytes()
                    .to_vec(),
            },
            InputValue {
                name: "_entrypoint".to_owned(),
                r#type: Type::String as i32,
                value: "windows.exe".as_bytes().to_vec(),
            },
            InputValue {
                name: "_arguments".to_owned(),
                r#type: Type::Bytes as i32,
                value: buff,
            },
            InputValue {
                name: "_attachments".to_owned(),
                r#type: Type::Bytes as i32,
                value: vec![],
            },
        ]);

        let eargs = executor::ExecutorArgs::from_wasi_host();
        assert!(eargs.is_ok());

        let eargs = eargs.unwrap();
        assert_eq!(
            std::fs::read(eargs.code().download().unwrap()).unwrap(),
            "code lol".as_bytes()
        );
        assert_eq!(
            eargs.sha256(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(eargs.entrypoint(), "windows.exe");

        assert_eq!(eargs.argument::<&str, bool>("sune").unwrap(), false);
        assert_eq!(
            eargs.argument::<&str, String>("rune").unwrap(),
            "datta!".to_owned()
        );
    }

    #[test]
    fn test_map_attachment_from_descriptor() {
        let random_attachment = Attachment {
            checksums: None,
            metadata: HashMap::new(),
            name: "foot".to_owned(),
            url: Some(AttachmentUrl {
                url: "fake:///".to_owned(),
                auth_method: AuthMethod::None as i32,
            }),
            created_at: 0,
        };
        let random_attachment2 = random_attachment.clone();

        MockResultRegistry::set_map_attachment_from_descriptor_impl(move |att, _| {
            let attachment_path = PathBuf::from("attachments").join(&att.name);
            assert_eq!(att.checksums, random_attachment.checksums);
            assert_eq!(att.metadata, random_attachment.metadata);
            assert_eq!(att.name, random_attachment.name);
            assert_eq!(att.url, random_attachment.url);
            Ok(attachment_path.to_string_lossy().to_string())
        });
        MockResultRegistry::set_get_attachment_path_len_from_descriptor_impl(|att| {
            Ok(PathBuf::from("attachments")
                .join(&att.name)
                .to_string_lossy()
                .as_bytes()
                .len())
        });

        assert_eq!(
            random_attachment2.download().unwrap(),
            PathBuf::from("attachments/foot")
        );
    }
}
