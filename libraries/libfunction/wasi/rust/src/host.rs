/// Raw wasi host functionality.
use std::os::raw::{c_char, c_void};

use thiserror::Error;

pub type ApiSize = u32;

/// API representation of the data type in a Channel
#[derive(Clone, PartialEq, Debug)]
#[repr(u8)]
pub enum ChannelType {
    Null = 0,
    String = 1,
    Integer = 2,
    Float = 3,
    Boolean = 4,
    Byte = 5,
}

impl std::fmt::Display for ChannelType {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                ChannelType::String => "string",
                ChannelType::Integer => "integer",
                ChannelType::Float => "float",
                ChannelType::Boolean => "boolean",
                ChannelType::Byte => "byte",
                ChannelType::Null => "null",
            },
        )
    }
}

impl TryFrom<u8> for ChannelType {
    type Error = std::io::Error;
    fn try_from(discriminant: u8) -> Result<Self, Self::Error> {
        match discriminant {
            0 => Ok(ChannelType::Null),
            1 => Ok(ChannelType::String),
            2 => Ok(ChannelType::Integer),
            3 => Ok(ChannelType::Float),
            4 => Ok(ChannelType::Boolean),
            5 => Ok(ChannelType::Byte),
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                format!("Unknown or unsupported discriminator \"{}\"", discriminant),
            )),
        }
    }
}

/// API representation of data in a Channel
///
/// Note that this carries type information, although the array is untyped. This is to be
/// compatible with the host API so it can be used for all types alike.
#[repr(C)]
#[derive(Debug)]
pub struct ChannelData {
    pub(crate) channel_type: ChannelType,
    pub(crate) count: ApiSize,
    pub(crate) array: *const c_void,
}

impl ChannelData {
    pub fn new(channel_type: ChannelType, count: ApiSize, array: *const c_void) -> Self {
        Self {
            channel_type,
            count,
            array,
        }
    }

    pub fn data(&self) -> *const c_void {
        self.array
    }

    pub fn count(&self) -> ApiSize {
        self.count
    }

    pub fn leak_array(&mut self) -> &mut Self {
        self.array = std::ptr::null();
        self
    }
}

impl Default for ChannelData {
    fn default() -> Self {
        Self {
            channel_type: ChannelType::Null,
            count: Default::default(),
            array: std::ptr::null(),
        }
    }
}

impl Drop for ChannelData {
    fn drop(&mut self) {
        if !self.array.is_null() {
            unsafe { libc::free(self.array as *mut c_void) }
        }
    }
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("Blocking call.")]
    WouldBlock,

    #[error("Expected channel \"{channel_name}\" to be of type \"{wanted_type}\" but it is \"{channel_type}\".")]
    MismatchedTypes {
        channel_name: String,
        channel_type: String,
        wanted_type: String,
    },

    #[error("IO Error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Error calling host API: {0}")]
    Host(String),

    #[error("Tried to append output to closed channel \"{0}\".")]
    AppendToClosedChannel(String),
}

pub trait FromChannelData {
    fn type_matches(channel_type: &ChannelType) -> bool;
    fn channel_type() -> ChannelType;
}

macro_rules! impl_from_channel_data {
    ($rust_type:ty, $($channel_type:tt)*) => {
        impl FromChannelData for $rust_type {
            fn type_matches(channel_type: &ChannelType) -> bool {
                matches!(channel_type, $($channel_type)*)
            }

            fn channel_type() -> ChannelType {
                $($channel_type)*
            }
        }
    };
}

impl_from_channel_data!(String, ChannelType::String);
impl_from_channel_data!(*const c_char, ChannelType::String);

impl_from_channel_data!(i64, ChannelType::Integer);
impl_from_channel_data!(f64, ChannelType::Float);
impl_from_channel_data!(bool, ChannelType::Boolean);
impl_from_channel_data!(u8, ChannelType::Byte);

/// A key-value pair representing an environment variable
#[derive(Debug)]
#[repr(C)]
pub struct EnvironmentVariable {
    pub key: *const c_char,
    pub value: *const c_char,
}

/// A request tot start a process
#[derive(Debug)]
#[repr(C)]
pub struct StartProcessRequest {
    pub(crate) command: *const c_char,
    pub(crate) env_vars: *const EnvironmentVariable,
    pub(crate) num_env_vars: ApiSize,
    pub(crate) wait: bool,
}

#[cfg(feature = "host")]
impl StartProcessRequest {
    /// Create a new request for starting a new host process
    ///
    /// `command` is the command line to use (including arguments) as a C string (with
    /// null terminator)
    /// `env_vars` is a C array with key-value pairs (see [`EnvironmentVariable`]) whose
    /// length is indicated by `num_env_vars`
    /// `wait` If true, a call made with this request will not return until the process
    /// has exited. In this case, both the PID and exit code will be set with proper
    /// values. If this is false, however, a call made with the request will return as
    /// soon as the process has a PID, and will set that PID. The exit code will remain
    /// unset in that case.
    pub fn new(
        command: *const c_char,
        env_vars: *const EnvironmentVariable,
        num_env_vars: u32,
        wait: bool,
    ) -> Self {
        Self {
            command,
            env_vars,
            num_env_vars,
            wait,
        }
    }
}

/// Wraps calls to the host and gives rust `Results` back.
#[macro_export]
macro_rules! host_call {
    ($call:expr) => {{
        let err = $call;

        if !err.is_null() {
            let error_message = ::std::ffi::CStr::from_ptr(err)
                .to_string_lossy()
                .to_string();
            ::libc::free(err as *mut ::std::os::raw::c_void);
            Err($crate::host::Error::Host(error_message))
        } else {
            Ok(())
        }
    }};
}

/// Get channel data from `channel` while checking that it matches type `T`
///
/// `read_count` tells how many entries should be read
/// `blocking` indicates whether the function is allowed to block if there is less data
/// than `read_count`. Note that if the channel is closed, this function might return less
/// data than `read_count` without blocking (there is no point in waiting for data that
/// will never arrive).
/// # Safety
/// This calls a WASI host side function, dealing with raw memory and is therefore unsafe
pub unsafe fn get_channel_data<T: FromChannelData>(
    channel: &str,
    read_count: ApiSize,
    blocking: bool,
) -> Result<ChannelData, Error> {
    let mut num_available = ApiSize::default();
    let mut channel_closed = false;
    host_call!(__input_available(
        channel.as_ptr() as *const c_char,
        &mut num_available,
        &mut channel_closed,
    ))?;

    if !blocking && !channel_closed && num_available < read_count {
        return Err(Error::WouldBlock);
    }

    let mut type_out = 0u8;
    host_call!(__channel_type(
        channel.as_ptr() as *const c_char,
        &mut type_out as *mut u8
    ))?;

    let channel_type = ChannelType::try_from(type_out)?;
    if !T::type_matches(&channel_type) {
        return Err(Error::MismatchedTypes {
            channel_name: channel.to_owned(),
            channel_type: channel_type.to_string(),
            wanted_type: std::any::type_name::<T>().to_string(),
        });
    }

    let mut channel_data = ChannelData::default();
    host_call!(__input_data(
        channel.as_ptr() as *const c_char,
        read_count,
        &mut channel_data as *mut ChannelData,
    ))?;

    Ok(channel_data)
}

/// Append data of type `T` to the channel with name `channel`
///
/// # Safety
/// This function calls a WASI host side function, dealing with raw memory and is
/// therefore unsafe
pub unsafe fn append_channel_data<T: FromChannelData>(
    channel: &str,
    data: &[T],
) -> Result<(), Error> {
    let mut type_out = 0u8;
    host_call!(__channel_type(
        channel.as_ptr() as *const c_char,
        &mut type_out as *mut u8
    ))?;

    let requested_channel_type = T::channel_type();
    let mut closed = false;
    host_call!(__channel_closed(
        channel.as_ptr() as *const c_char,
        &mut closed
    ))?;

    (!closed)
        .then(|| ())
        .ok_or_else(|| Error::AppendToClosedChannel(String::from(channel)))
        .and_then(|_| ChannelType::try_from(type_out).map_err(Into::into))
        .and_then(|channel_type| {
            if channel_type != requested_channel_type {
                Err(Error::MismatchedTypes {
                    channel_name: channel.to_owned(),
                    channel_type: channel_type.to_string(),
                    wanted_type: requested_channel_type.to_string(),
                })
            } else {
                Ok(channel_type)
            }
        })
        .and_then(|channel_type| {
            let mut channel_data = ChannelData::new(
                channel_type,
                data.len() as ApiSize,
                data.as_ptr() as *const c_void,
            );
            let res = host_call!(__append_output(
                channel.as_ptr() as *const c_char,
                &channel_data as *const _,
            ));
            channel_data.leak_array();
            res
        })
}

#[cfg_attr(not(any(test, host_test)), link(wasm_import_module = "firm"))]
extern "C" {
    pub fn __input_data(
        key: *const c_char,
        size: ApiSize,
        value_out: *mut ChannelData,
    ) -> *const c_char;

    pub fn __channel_type(key: *const c_char, type_out: *mut u8) -> *const c_char;

    pub fn __channel_closed(key: *const c_char, closed_out: *mut bool) -> *const c_char;

    pub fn __input_available(
        key: *const c_char,
        num_available_out: *mut ApiSize,
        closed_out: *mut bool,
    ) -> *const c_char;

    pub fn __append_output(key: *const c_char, values: *const ChannelData) -> *const c_char;

    pub fn __close_output(key_ptr: *const c_char) -> *const c_char;

    pub fn __map_attachment(
        attachment_name: *const c_char,
        unpack: bool,
        path_out: *mut *const c_char,
    ) -> *const c_char;

    pub fn __host_path_exists(path: *const c_char, exists: *mut bool) -> *const c_char;

    pub fn __host_os(os_name: *mut *const c_char) -> *const c_char;

    pub fn __start_host_process(
        request: *const StartProcessRequest,
        pid_out: *mut u64,
        exit_code_out: *mut i64,
    ) -> *const c_char;

    pub fn __set_error(msg: *const c_char) -> *const c_char;

    pub fn __connect(addr: *const c_char, file_descriptor_out: *mut i32) -> *const c_char;
}
