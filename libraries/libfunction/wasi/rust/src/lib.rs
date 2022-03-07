#[cfg(feature = "host")]
pub mod host;

#[cfg(not(feature = "host"))]
mod host;

use std::{
    borrow::Cow,
    collections::HashMap,
    ffi::{CStr, CString},
    io::{Read, Write},
    marker::PhantomData,
    net::{Ipv4Addr, Ipv6Addr},
    ops::{Deref, DerefMut},
    os::raw::{c_char, c_void},
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::io::FromRawFd;
#[cfg(target_os = "wasi")]
use std::os::wasi::io::FromRawFd;

use thiserror::Error;

use host::{ApiSize, ChannelData, EnvironmentVariable, StartProcessRequest};

/// Error type for all API calls.
#[derive(Error, Debug)]
pub enum ApiError {
    #[error("Blocking call.")]
    WouldBlock,

    #[error("Type error: {0}")]
    TypeError(host::Error),

    #[error("Api error: {0}")]
    Generic(String),
}

impl From<host::Error> for ApiError {
    fn from(err: host::Error) -> Self {
        match err {
            host::Error::WouldBlock => ApiError::WouldBlock,
            host::Error::MismatchedTypes { .. } => ApiError::TypeError(err),
            host::Error::Io(_) => ApiError::Generic(err.to_string()),
            host::Error::Host(e) => ApiError::Generic(e),
            host::Error::AppendToClosedChannel(e) => ApiError::Generic(e),
        }
    }
}

/// Result type returned by all API methods.
type ApiResult<T> = Result<T, ApiError>;

/// Converts ChannelData to native rust types.
pub trait FromChannelData
where
    Self: Sized,
{
    type HostType: host::FromChannelData;

    fn first(channel_data: &ChannelData) -> Self {
        FromChannelData::nth(channel_data, 0)
    }

    fn nth(channel_data: &ChannelData, index: host::ApiSize) -> Self;
}

impl FromChannelData for String {
    type HostType = String;
    fn nth(channel_data: &ChannelData, index: host::ApiSize) -> Self {
        unsafe {
            CStr::from_ptr(*(channel_data.data() as *const *const c_char).offset(index as isize))
                .to_string_lossy()
                .into_owned()
        }
    }
}

macro_rules! impl_from_channel_data {
    ($host_type:ty) => {
        impl FromChannelData for $host_type {
            type HostType = $host_type;
            fn nth(channel_data: &ChannelData, index: host::ApiSize) -> Self {
                (unsafe { *(channel_data.data() as *const $host_type).offset(index as isize) })
            }
        }
    };

    // TODO: Research the ability to do try_from instead of as casting.
    ($host_type:ty, $to_type:ty) => {
        impl FromChannelData for $to_type {
            type HostType = $host_type;
            fn nth(channel_data: &ChannelData, index: host::ApiSize) -> Self {
                (unsafe { *(channel_data.data() as *const $host_type).offset(index as isize) })
                    as $to_type
            }
        }
    };
}

impl_from_channel_data!(i64);
impl_from_channel_data!(f64);
impl_from_channel_data!(bool);
impl_from_channel_data!(u8);

impl_from_channel_data!(i64, i32);
impl_from_channel_data!(i64, i16);
impl_from_channel_data!(i64, u32);
impl_from_channel_data!(i64, u16);
impl_from_channel_data!(f64, f32);

pub trait ToOutputData {
    type Item: Sized + host::FromChannelData + Clone;
    fn to_output_data(&self) -> Cow<'_, [Self::Item]>;
}

/// Get a single input
pub fn input<T: FromChannelData>(key: &str) -> Option<ApiResult<T>> {
    unsafe {
        host::get_channel_data::<T::HostType>(key, 1, true)
            .map(|ref cd| (cd.count > 0).then(|| FromChannelData::first(cd)))
            .map_err(Into::into)
            .transpose()
    }
}

/// Append output.
pub fn append_output<T>(key: &str, data: T) -> ApiResult<()>
where
    T: ToOutputData,
{
    unsafe { host::append_channel_data(key, data.to_output_data().as_ref()).map_err(Into::into) }
}

macro_rules! impl_to_output_data {
    ($type:ty) => {
        impl ToOutputData for Vec<$type> {
            type Item = $type;

            fn to_output_data(&self) -> Cow<'_, [Self::Item]> {
                Cow::Borrowed(self)
            }
        }

        impl ToOutputData for &[$type] {
            type Item = $type;

            fn to_output_data(&self) -> Cow<'_, [Self::Item]> {
                Cow::Borrowed(self)
            }
        }
    };

    ($type:ty, $from:ty) => {
        impl ToOutputData for Vec<$from> {
            type Item = $type;

            fn to_output_data(&self) -> Cow<'_, [Self::Item]> {
                Cow::Owned(self.iter().map(|v| *v as $type).collect::<Vec<$type>>())
            }
        }

        impl ToOutputData for &[$from] {
            type Item = $type;

            fn to_output_data(&self) -> Cow<'_, [Self::Item]> {
                Cow::Owned(self.iter().map(|v| *v as $type).collect::<Vec<$type>>())
            }
        }
    };
}

pub type Int = i64;
pub type Float = f64;
pub type Bool = bool;
pub type Byte = u8;

impl_to_output_data!(String);
impl_to_output_data!(i64);
impl_to_output_data!(f64);
impl_to_output_data!(bool);
impl_to_output_data!(u8);

impl_to_output_data!(i64, i32);
impl_to_output_data!(i64, i16);
impl_to_output_data!(i64, u32);
impl_to_output_data!(i64, u16);
impl_to_output_data!(i64, i8);
impl_to_output_data!(f64, f32);

pub fn close_output(key: &str) -> ApiResult<()> {
    let ckey = CString::new(key).map_err(|e| {
        ApiError::Generic(format!(
            "Invalid output name \"{}\": {}",
            key,
            e.to_string()
        ))
    })?;
    unsafe { host_call!(host::__close_output(ckey.as_ptr())).map_err(Into::into) }
}

/// Iterator over input values
pub struct InputIter<T> {
    channel_name: String,
    data: ChannelData,
    index: host::ApiSize,
    fetch_size: host::ApiSize,
    phantom: PhantomData<T>,
}

impl<T> InputIter<T> {
    fn new(channel_name: &str, fetch_size: host::ApiSize) -> Self {
        Self {
            channel_name: channel_name.to_owned(),
            data: ChannelData::default(),
            index: 0,
            fetch_size,
            phantom: PhantomData,
        }
    }

    fn new_data(&mut self, channel_data: ChannelData) -> &mut Self {
        self.data = channel_data;
        self.index = 0;
        self
    }
}

impl<T: FromChannelData> Iterator for InputIter<T> {
    type Item = ApiResult<T>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.data.count() {
            let i = self.index;
            self.index += 1;
            Some(Ok(FromChannelData::nth(&self.data, i)))
        } else {
            // fetch new data
            unsafe {
                host::get_channel_data::<T::HostType>(&self.channel_name, self.fetch_size, true)
            }
            .map(|channel_data| self.new_data(channel_data))
            .map_err(ApiError::from)
            .and_then(|iter| {
                // if the newly fetched data is zero, the channel is closed and there
                // will be no more data
                (iter.data.count() > 0)
                    .then(|| iter)
                    .and_then(|iter| iter.next())
                    .transpose()
            })
            .transpose()
        }
    }
}

pub fn input_iter<T: FromChannelData>(key: &str, fetch_size: host::ApiSize) -> InputIter<T> {
    // TODO: It's a bit weird that you can create an iterator of
    // Floats that errors later on next() because the input is strings.
    // It currently works like this because it will error on the first host call.
    // We could possibly error earlier by doing a type check here.
    InputIter::new(key, fetch_size)
}

/// Maps an attachment into the guest filesystem
///
/// Returns a PathBuf where the attachment is located.  Path can be
/// both a directory or a file depending on if the mapped attachment
/// was archive and `unpack` was true.
pub fn map_attachment(attachment_name: &str, unpack: bool) -> ApiResult<PathBuf> {
    CString::new(attachment_name)
        .map_err(|e| {
            ApiError::Generic(format!(
                "Invalid attachment name \"{}\": {}",
                attachment_name,
                e.to_string()
            ))
        })
        .and_then(|c_str| {
            let path_out = std::ptr::null_mut();
            unsafe { host_call!(host::__map_attachment(c_str.as_ptr(), unpack, path_out)) }
                .map_err(Into::into)
                .map(|_| path_out)
        })
        .and_then(|path| {
            unsafe { CStr::from_ptr(*path) }.to_str().map_err(|e| {
                ApiError::Generic(format!(
                    "Failed to decode result path when mapping attachment {}: {}",
                    attachment_name,
                    e.to_string()
                ))
            })
        })
        .map(PathBuf::from)
}

/// Checks if a path exists on the hosts file system.
///
/// Returns true or false depending on if the path existed.
pub fn host_path_exists(path: &Path) -> ApiResult<bool> {
    path.to_str()
        .ok_or_else(|| {
            ApiError::Generic(format!(
                "Failed to convert path \"{}\" to a str.",
                path.display()
            ))
        })
        .and_then(|s| {
            CString::new(s).map_err(|e| {
                ApiError::Generic(format!(
                    "Path \"{}\" is not valid as a C-String: {}",
                    path.display(),
                    e.to_string()
                ))
            })
        })
        .and_then(|c_str| {
            let mut exists = false;
            (unsafe { host_call!(host::__host_path_exists(c_str.as_ptr(), &mut exists)) })
                .map_err(Into::into)
                .map(|_| exists)
        })
}

/// Get OS for WASI host
///
/// This will return a string like "windows", "linux" etc.
pub fn get_host_os() -> ApiResult<String> {
    let mut result_os_name: *const c_char = std::ptr::null_mut();
    unsafe {
        host_call!(host::__host_os(&mut result_os_name))
            .map_err(Into::into)
            .map(|_| {
                let s = CStr::from_ptr(result_os_name)
                    .to_string_lossy()
                    .into_owned();
                libc::free((result_os_name) as *mut c_void);
                s
            })
    }
}

#[derive(Debug)]
/// Result of running a host process
pub enum HostProcessResult {
    ExitedProcess { exit_code: i64 },
    RunningProcess { pid: u64 },
}

impl HostProcessResult {
    /// Returns true if the process exited.
    pub fn is_exited(&self) -> bool {
        matches!(self, HostProcessResult::ExitedProcess { .. })
    }

    /// Returns an [`Option`] with the pid if the process was started without wait.
    pub fn pid(&self) -> Option<u64> {
        match self {
            HostProcessResult::RunningProcess { pid } => Some(*pid),
            _ => None,
        }
    }

    /// Returns an [`Option`] with the process exit code if the process has exited.
    pub fn exit_code(&self) -> Option<i64> {
        match self {
            HostProcessResult::ExitedProcess { exit_code } => Some(*exit_code),
            _ => None,
        }
    }
}

/// A process builder for running processes on the host.
pub struct HostProcess {
    command: String,
    wait: bool,
    environment_variables: HashMap<String, String>,
}

impl HostProcess {
    /// Returns a new `HostProcess`
    ///
    /// # Arguments
    ///
    /// * `command` - A string containing the process to run and its arguments.
    ///
    /// # Examples
    /// ```
    /// use firm_function::HostProcess;
    ///
    /// let hp = HostProcess::new("system32.exe --format-c");
    ///
    /// ```
    pub fn new<'a, S: Into<Cow<'a, str>>>(command: S) -> Self {
        Self {
            command: command.into().into_owned(),
            wait: false,
            environment_variables: HashMap::new(),
        }
    }

    /// Starts the `HostProcess`
    ///
    /// # Examples
    /// ```
    /// use firm_function::HostProcess;
    ///
    /// let env_vars :HashMap<String,String> =
    ///     [(String::from("my_var"), String::from("1"))]
    ///     .into_iter()
    ///     .collect();
    /// let result = HostProcess::new("system32.exe --format-c")
    ///     .wait()
    ///     .environment_variables(env_vars)
    ///     .start();
    ///
    /// ```
    pub fn start(&self) -> ApiResult<HostProcessResult> {
        // env_vars need to live longer than the call to
        // host::__start_process for all pointers in the request to be valid.
        let env_vars = self
            .environment_variables
            .iter()
            .map(|(k, v)| {
                CString::new(k.as_str())
                    .map_err(|e| {
                        ApiError::Generic(format!(
                            "Invalid environment variable key \"{}\": {}",
                            k,
                            e.to_string()
                        ))
                    })
                    .and_then(|k| {
                        CString::new(v.as_str())
                            .map_err(|e| {
                                ApiError::Generic(format!(
                                    "Invalid environment variable value \"{}\": {}",
                                    v,
                                    e.to_string()
                                ))
                            })
                            .map(|v| (k, v))
                    })
            })
            .collect::<ApiResult<Vec<_>>>()?;

        CString::new(self.command.clone())
            .map_err(|e| {
                ApiError::Generic(format!(
                    "Invalid command \"{}\": {}",
                    self.command,
                    e.to_string()
                ))
            })
            .and_then(|c_command| {
                let mut pid_out: u64 = 0;
                let mut exit_code_out: i64 = 0;

                unsafe {
                    host_call!(host::__start_host_process(
                        &StartProcessRequest {
                            command: c_command.as_ptr(),
                            env_vars: env_vars
                                .iter()
                                .map(|(k, v)| EnvironmentVariable {
                                    key: k.as_ptr(),
                                    value: v.as_ptr(),
                                })
                                .collect::<Vec<_>>()
                                .as_ptr(),
                            num_env_vars: env_vars.len() as ApiSize,
                            wait: self.wait,
                        },
                        &mut pid_out,
                        &mut exit_code_out
                    ))
                }
                .map_err(Into::into)
                .map(|_| (pid_out, exit_code_out))
            })
            .map(|(pid, exit_code)| match self.wait {
                true => HostProcessResult::ExitedProcess { exit_code },
                false => HostProcessResult::RunningProcess { pid },
            })
    }

    /// Makes the `HostProcess` wait for the process to exit.
    ///
    /// If you wait, the `HostProcess` will return a `HostProcessResult::ExitedProcess`.
    /// If you do not wait (default) it will return a `HostProcessResult::RunningProcess`.
    pub fn wait(&mut self, wait: bool) -> &mut Self {
        self.wait = wait;
        self
    }

    /// Makes the `HostProcess` run the process with the provided environment variables.
    ///
    /// You can call the method multiple times, the `HostProcess` will extend all enviroment variables.
    ///
    /// # Arguments
    ///
    /// * `environment_variables` - A [`HashMap<String,String>`] containing the keys and values for the environment.
    pub fn environment_variables(
        &mut self,
        environment_variables: &HashMap<String, String>,
    ) -> &mut Self {
        self.environment_variables
            .extend(environment_variables.clone());
        self
    }
}

pub fn set_error<S: AsRef<str>>(error_msg: S) -> ApiResult<()> {
    CString::new(error_msg.as_ref())
        .map_err(|e| {
            ApiError::Generic(format!(
                "Invalid error message \"{}\": {}",
                error_msg.as_ref().to_string(),
                e.to_string()
            ))
        })
        .and_then(|err_msg| {
            unsafe { host_call!(host::__set_error(err_msg.as_ptr())) }.map_err(Into::into)
        })
}

pub trait ToWasiConnectAddress {
    fn to_address(&self) -> ApiResult<WasiConnectAddress>;
}

impl ToWasiConnectAddress for (String, u16) {
    fn to_address(&self) -> ApiResult<WasiConnectAddress> {
        WasiConnectAddress::new(self.0.clone(), self.1)
    }
}

impl ToWasiConnectAddress for (&str, u16) {
    fn to_address(&self) -> ApiResult<WasiConnectAddress> {
        WasiConnectAddress::new(self.0.to_owned(), self.1)
    }
}

pub struct WasiConnectAddress {
    address: String,
    port: u16,
}

impl WasiConnectAddress {
    pub fn new(addr: String, port: u16) -> ApiResult<Self> {
        match (addr.parse::<Ipv4Addr>(), addr.parse::<Ipv6Addr>()) {
            (Err(_), Err(_)) => Ok(WasiConnectAddress {
                address: addr,
                port,
            }),
            _ => Err(ApiError::Generic(String::from(
                "Wasi connect only support hostnames (for the ability to do capability checking).",
            ))),
        }
    }
}

impl ToWasiConnectAddress for &str {
    fn to_address(&self) -> ApiResult<WasiConnectAddress> {
        let mut sp = self.splitn(2, ':');
        sp.next()
            .ok_or_else(|| ApiError::Generic(format!("Invalid address: \"{}\"", self)))
            .and_then(|addr| {
                sp.next()
                    .ok_or_else(|| {
                        ApiError::Generic(format!("Address did not contain a port: \"{}\"", self))
                    })
                    .map(|port| (addr, port))
            })
            .and_then(|(addr, port)| {
                port.parse::<u16>()
                    .map_err(|e| ApiError::Generic(format!("Failed to parse port: {}", e)))
                    .and_then(|port| WasiConnectAddress::new(addr.to_owned(), port))
            })
    }
}

impl ToWasiConnectAddress for String {
    fn to_address(&self) -> ApiResult<WasiConnectAddress> {
        <&str>::to_address(&self.as_str())
    }
}

// TODO: Remove abstraction once TcpStream in std gets up to par with what we need
/// Host side TCP stream
pub struct TcpStream {
    inner: std::fs::File,
    peer_addr: WasiConnectAddress,
}

impl Deref for TcpStream {
    type Target = std::fs::File;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for TcpStream {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl TcpStream {
    /// Opens a TCP connection to a remote host.
    ///
    /// `address` is an address of the remote host.
    pub fn connect<A: ToWasiConnectAddress>(address: A) -> ApiResult<Self> {
        address
            .to_address()
            .and_then(|addr| {
                connect(format!("tcp://{}:{}", addr.address, addr.port)).map(|fd| (addr, fd))
            })
            .map(|(addr, fd)| Self {
                inner: unsafe { std::fs::File::from_raw_fd(fd) },
                peer_addr: addr,
            })
    }

    pub fn peer_addr(&self) -> &WasiConnectAddress {
        &self.peer_addr
    }
}

// TODO: Remove abstraction once UdpSocket in std gets up to par with what we need
/// Host-side UDP socket
pub struct UdpSocket {
    inner: std::fs::File,
    peer_addr: WasiConnectAddress,
}

impl Deref for UdpSocket {
    type Target = std::fs::File;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl UdpSocket {
    pub fn connect<A: ToWasiConnectAddress>(address: A) -> ApiResult<Self> {
        address
            .to_address()
            .and_then(|addr| {
                connect(format!("udp://{}:{}", addr.address, addr.port)).map(|fd| (addr, fd))
            })
            .map(|(addr, fd)| Self {
                inner: unsafe { std::fs::File::from_raw_fd(fd) },
                peer_addr: addr,
            })
    }

    pub fn peer_addr(&self) -> &WasiConnectAddress {
        &self.peer_addr
    }

    pub fn send(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner.write(buf)
    }

    pub fn recv(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

fn connect<S: AsRef<str>>(address: S) -> ApiResult<i32> {
    CString::new(address.as_ref())
        .map_err(|e| {
            ApiError::Generic(format!(
                "Invalid error message \"{}\": {}",
                address.as_ref().to_string(),
                e.to_string()
            ))
        })
        .and_then(|addr| {
            let mut file_descriptor_out: i32 = 0;
            unsafe { host_call!(host::__connect(addr.as_ptr(), &mut file_descriptor_out)) }
                .map_err(Into::into)
                .map(|_| file_descriptor_out)
        })
}

#[cfg(test)]
mod tests {
    use crate::{TcpStream, UdpSocket};

    use super::host;
    use paste::paste;
    use std::{
        collections::HashMap,
        ffi::CStr,
        io::Write,
        os::{raw::c_char, wasi::prelude::AsRawFd},
        path::Path,
        sync::Mutex,
    };

    fn create_string_ptr(s: &str) -> *const c_char {
        (unsafe {
            let mem = libc::malloc(s.len() + 1) as *mut u8;
            s.as_ptr().copy_to_nonoverlapping(mem, s.len());
            mem.add(s.len()).write(b'\0');
            mem
        }) as *const c_char
    }

    macro_rules! mock_host_fn {
        ($name:ident => $($signature:tt)*) => {
            mock_host_fn!(@fn $name, $($signature)*);
        };

        (@fn $name:ident, ($($arg:ident: $argty:ty),*) -> $ret:ty) => {
            paste! {
                type [<$name:camel Fn>] =
                Option<Mutex<Box<dyn Fn($($argty),*) -> Result<(),String> + Send + Sync>>>;
                static mut [<$name:snake:upper _IMP>]: [<$name:camel Fn>] = None;
                fn [<set_ $name _impl>]<
                    F: Fn($($argty),*) -> Result<(),String> + Send + Sync + 'static,
                    >(
                    f: F,
                ) {
                    unsafe {
                        [<$name:snake:upper _IMP>] = Some(Mutex::new(Box::new(f)));
                    }
                }

                #[no_mangle]
                unsafe extern "C" fn [<__ $name>]($($arg: $argty),*) -> $ret {
                    [<$name:snake:upper _IMP>]
                    .as_ref()
                        .map(|f| {
                            match f.lock().unwrap()($($arg),*) {
                                Ok(_) => std::ptr::null(),
                                Err(e) => {
                                    create_string_ptr(e.as_str())
                                }
                            }
                        })
                    .unwrap_or_else(|| create_string_ptr(&format!("No implementation set for {}", stringify!($fn_name))))
                }
            }
        };
    }

    mock_host_fn!(map_attachment => (attachment_name: *const c_char, unpack: bool, path_out: *mut *const c_char) -> *const c_char);
    mock_host_fn!(host_os => (os_out: *mut *const c_char) -> *const c_char);
    mock_host_fn!(host_path_exists => (path: *const c_char, exists: *mut bool) -> *const c_char);
    mock_host_fn!(set_error => (msg: *const c_char) -> *const c_char);

    mock_host_fn!(start_host_process => (request: *const host::StartProcessRequest, pid_out: *mut u64, exit_code_out: *mut i64) -> *const c_char);
    mock_host_fn!(input_data => (key: *const c_char, size: host::ApiSize, value_out: *mut host::ChannelData) -> *const c_char);
    mock_host_fn!(channel_type => (key: *const c_char, type_out: *mut u8) -> *const c_char);
    mock_host_fn!(input_available => (key: *const c_char, num_available_out: *mut host::ApiSize, closed_out: *mut bool) -> *const c_char);
    mock_host_fn!(append_output => (key: *const c_char, data: *const host::ChannelData) -> *const c_char);
    mock_host_fn!(close_output => (key: *const c_char) -> *const c_char);
    mock_host_fn!(connect => (key: *const c_char, file_descriptor: *mut i32) -> *const c_char);
    mock_host_fn!(channel_closed => (key: *const c_char, closed_out: *mut bool) -> *const c_char);

    #[test]
    fn test_map_attachment() {
        set_map_attachment_impl(|attachment_name, _, path_out| {
            match unsafe { CStr::from_ptr(attachment_name) }
                .to_string_lossy()
                .as_ref()
            {
                "good.txt" => unsafe {
                    *path_out = create_string_ptr("/root/mega/good.txt");
                    Ok(())
                },
                _ => Err(String::from(
                    "Could not find the \"file\". Yes, i won't tell you which one.",
                )),
            }
        });

        let res = super::map_attachment("good.txt", true);
        assert!(res.is_ok());
        assert!(res.unwrap().ends_with("good.txt"));

        let res = super::map_attachment("nope.elf", true);
        assert!(res.is_err());
        assert!(res
            .unwrap_err()
            .to_string()
            .contains("Yes, i won't tell you which one."));
    }

    #[test]
    fn test_host_path_exists() {
        set_host_path_exists_impl(|path, exists| {
            match unsafe { CStr::from_ptr(path) }.to_string_lossy().as_ref() {
                "good/path" => unsafe {
                    *exists = true;
                    Ok(())
                },
                "bad/path" => unsafe {
                    *exists = false;
                    Ok(())
                },
                _ => Err(String::from("Could not do the thing")),
            }
        });

        let res = super::host_path_exists(Path::new("good/path"));
        assert!(res.is_ok());
        assert!(res.unwrap(), "Expected good/path to \"exist\"");

        let res = super::host_path_exists(Path::new("bad/path"));
        assert!(res.is_ok());
        assert!(!res.unwrap(), "Expected bad/path to not exist");

        let res = super::host_path_exists(Path::new(r#"C:\Windows\System32\drivers\etc\hosts"#));
        assert!(res.is_err(), "Expected \"invalid\" path cause an error");
        assert!(res
            .unwrap_err()
            .to_string()
            .contains("Could not do the thing"));
    }

    #[test]
    fn test_get_host_os() {
        set_host_os_impl(|os_out| unsafe {
            *os_out = create_string_ptr("openbsd");
            Ok(())
        });

        let res = super::get_host_os();
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), "openbsd");

        set_host_os_impl(|_os_out| Err(String::from("Everything is on fire!")));
        let res = super::get_host_os();
        assert!(res.is_err());
        assert!(res
            .unwrap_err()
            .to_string()
            .contains("Everything is on fire!"));
    }

    #[test]
    fn test_set_error() {
        set_set_error_impl(|error_msg| {
            match unsafe { CStr::from_ptr(error_msg) }
                .to_string_lossy()
                .as_ref()
            {
                "bad" => Err(String::from("Failed to set the error.")),
                _ => Ok(()),
            }
        });

        let res = super::set_error("This was mega bad!");
        assert!(res.is_ok());

        let res = super::set_error("bad");
        assert!(res.is_err());
        assert!(res
            .unwrap_err()
            .to_string()
            .contains("Failed to set the error."))
    }

    #[test]
    fn test_host_process() {
        static mut START_HOST_PROCESS_ENV: Option<HashMap<String, String>> = None;

        set_start_host_process_impl(|request, pid_out, exit_out| unsafe {
            let r = &*request;
            let command = CStr::from_ptr(r.command).to_string_lossy();
            let envs = std::slice::from_raw_parts(r.env_vars, r.num_env_vars as usize);

            START_HOST_PROCESS_ENV = Some(
                envs.iter()
                    .map(|env| {
                        (
                            CStr::from_ptr(env.key).to_string_lossy().to_string(),
                            CStr::from_ptr(env.value).to_string_lossy().to_string(),
                        )
                    })
                    .collect(),
            );

            match command.as_ref() {
                "error" => Err(String::from("I error")),
                _ if r.wait => {
                    *exit_out = 1;
                    *pid_out = 64005;
                    Ok(())
                }
                _ => {
                    *pid_out = 64006;
                    Ok(())
                }
            }
        });

        let mut test_env = HashMap::new();
        test_env.insert(String::from("sune"), String::from("håkan"));
        test_env.insert(String::from("bune"), String::from("lune"));

        let res = super::HostProcess::new("rune")
            .wait(true)
            .environment_variables(&test_env)
            .start();

        assert!(res.is_ok());
        let res = res.unwrap();
        assert!(res.is_exited());
        assert_eq!(res.pid(), None);
        assert_eq!(res.exit_code(), Some(1));
        unsafe {
            let env = START_HOST_PROCESS_ENV.as_ref().unwrap();
            assert_eq!(env.len(), 2);
            assert!(env.contains_key("sune"));
            assert!(env.contains_key("bune"));
            assert_eq!(env.get("sune").unwrap(), "håkan");
            assert_eq!(env.get("bune").unwrap(), "lune");
        }

        let res = super::HostProcess::new("rune")
            .environment_variables(&HashMap::new())
            .start();

        assert!(res.is_ok());
        let res = res.unwrap();
        assert!(!res.is_exited());
        assert_eq!(res.pid(), Some(64006));
        unsafe {
            assert_eq!(START_HOST_PROCESS_ENV.as_ref().unwrap().len(), 0);
        }

        let res = super::HostProcess::new("error")
            .environment_variables(&HashMap::new())
            .start();

        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("I error"));
    }

    #[test]
    fn test_single_input() {
        set_input_data_impl(|_key, _size, value_out| unsafe {
            let channel_data = &mut *value_out;
            channel_data.channel_type = host::ChannelType::Integer;
            channel_data.count = 1;
            let ints = libc::malloc(std::mem::size_of::<i64>()) as *mut i64;
            *ints = 1337i64;
            channel_data.array = ints as *const _;
            Ok(())
        });

        set_input_available_impl(|_, num_available_out, closed_out| unsafe {
            *num_available_out = 1;
            *closed_out = false;
            Ok(())
        });

        set_channel_type_impl(|_, type_out| unsafe {
            *type_out = host::ChannelType::Integer as u8;
            Ok(())
        });

        let res = super::input::<super::Int>("maj-inputt");
        assert!(res.is_some());
        let res = res.unwrap();
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), 1337i64, "Expected leet result");

        let res = super::input::<String>("nåt-maj-inputt");
        assert!(res.is_some());
        assert!(
            matches!(res.unwrap().unwrap_err(), super::ApiError::TypeError(_)),
            "Expected to get type error when trying to get input with wrong type"
        );

        // test closed output
        set_input_available_impl(|_, num_available_out, closed_out| unsafe {
            *num_available_out = 0;
            *closed_out = true;
            Ok(())
        });

        set_input_data_impl(|_key, _size, value_out| unsafe {
            let channel_data = &mut *value_out;
            channel_data.channel_type = host::ChannelType::Integer;
            channel_data.count = 0;
            Ok(())
        });

        let res = super::input::<super::Int>("maj-inputt");
        assert!(res.is_none(), "A closed input should not yield a value");
    }

    #[test]
    fn test_input_iter() {
        static mut STRINGS_LEFT: super::ApiSize = 100;
        set_input_data_impl(|_key, size, value_out| unsafe {
            let channel_data = &mut *value_out;
            channel_data.channel_type = host::ChannelType::String;
            channel_data.count = std::cmp::min(size, STRINGS_LEFT);

            if channel_data.count > 0 {
                let strings = libc::malloc(
                    std::mem::size_of::<*const c_char>() * channel_data.count as usize,
                ) as *mut *const c_char;

                let strings_slice =
                    std::slice::from_raw_parts_mut(strings, channel_data.count as usize);
                strings_slice.iter_mut().enumerate().for_each(|(i, s)| {
                    *s = create_string_ptr(&format!(
                        "stringstring-{}",
                        (100 - STRINGS_LEFT) + i as u32
                    ))
                });
                channel_data.array = strings as *const _;
            }

            STRINGS_LEFT = STRINGS_LEFT.saturating_sub(size);
            Ok(())
        });

        set_input_available_impl(|_, num_available_out, closed_out| unsafe {
            *num_available_out = STRINGS_LEFT;
            *closed_out = STRINGS_LEFT == 0;
            Ok(())
        });

        set_channel_type_impl(|_, type_out| unsafe {
            *type_out = host::ChannelType::String as u8;
            Ok(())
        });

        let iterator = super::input_iter::<String>("iterera", 10);

        // test collection of the array, of course it is also possible to iterate over it
        // directly, dealing with the result for each item
        let all_strings = iterator.collect::<super::ApiResult<Vec<_>>>();
        assert!(all_strings.is_ok());

        all_strings
            .unwrap()
            .into_iter()
            .enumerate()
            .for_each(|(i, s)| assert_eq!(s, format!("stringstring-{}", i)));

        // Test that iterator gives type error on first iteration
        let mut iterator = super::input_iter::<super::Float>("ultra-iterator", 10);
        let res = iterator.next();
        assert!(res.is_some());
        let res = res.unwrap();
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), super::ApiError::TypeError(_)));
    }

    #[test]
    fn test_output() {
        static mut APPENDED_DATA: Vec<i64> = Vec::new();
        static mut CLOSED: bool = false;
        set_channel_type_impl(|_, type_out| unsafe {
            *type_out = host::ChannelType::Integer as u8;
            Ok(())
        });

        set_append_output_impl(|_key, data| unsafe {
            let data = &*data;
            APPENDED_DATA =
                std::slice::from_raw_parts(data.array as *const i64, data.count as usize).to_vec();
            Ok(())
        });

        set_close_output_impl(|key| unsafe {
            CLOSED = true;
            match CStr::from_ptr(key).to_string_lossy().as_ref() {
                "error" => Err(String::from("I was error")),
                _ => Ok(()),
            }
        });

        set_channel_closed_impl(|_, closed| unsafe {
            *closed = CLOSED;
            Ok(())
        });

        let res = super::append_output("skeleton key", [5, 56, 76].as_slice());

        assert!(res.is_ok());
        unsafe {
            assert_eq!(APPENDED_DATA.len(), 3);
            assert_eq!(APPENDED_DATA[0], 5);
            assert_eq!(APPENDED_DATA[1], 56);
            assert_eq!(APPENDED_DATA[2], 76);
        }

        // Test appending wrong type
        let res = super::append_output("skeleton key", [5f64, 56f64, 76f64].as_slice());
        assert!(res.is_err());

        // Test appending similar (supported conversion) type
        let res = super::append_output("skeleton key", [5i16, 56i16, 76i16].as_slice());
        assert!(res.is_ok());

        // Test closing output
        unsafe {
            assert!(!CLOSED);
            let res = super::close_output("ivar");
            assert!(res.is_ok());
            assert!(CLOSED);

            let res = super::close_output("error");
            assert!(res.is_err());
            assert!(res.unwrap_err().to_string().contains("I was error"));
        }

        // Test appending to closed output
        let res = super::append_output("skeleton key", [5, 56, 76].as_slice());
        assert!(res.is_err(), "Expected appending to closed yield an error");
    }

    #[test]
    fn test_connect() {
        let file = std::fs::File::create("my_file.elf").unwrap();
        let file_descriptor = file.as_raw_fd();

        static mut LAST_ADDR: String = String::new();
        set_connect_impl(move |addr, fd| unsafe {
            LAST_ADDR = CStr::from_ptr(addr).to_string_lossy().to_string();
            match LAST_ADDR.as_str() {
                "error" => Err(String::from("connection error")),
                _ => {
                    *fd = file_descriptor;
                    Ok(())
                }
            }
        });

        let tcp = TcpStream::connect("mega-rune.com:9999");
        assert!(tcp.is_ok());
        let mut tcp = tcp.unwrap();
        assert!(unsafe { LAST_ADDR.starts_with("tcp://") });
        let write_res = tcp.write(b"TCP was here.");
        assert!(write_res.is_ok());

        let udp = UdpSocket::connect("maj-onäs.blog:654");
        assert!(udp.is_ok());
        let mut udp = udp.unwrap();
        assert!(unsafe { LAST_ADDR.starts_with("udp://") });
        let write_res = udp.send(b"UDP was here.");
        assert!(write_res.is_ok());

        let file_content = std::fs::read_to_string("my_file.elf").unwrap();
        assert!(file_content.contains("TCP was here"));
        assert!(file_content.contains("UDP was here"));

        // Test that it doesn't work with ipv4 and ipv6 addresses.
        let tcp = TcpStream::connect("127.0.0.2:9999");
        assert!(tcp.is_err());

        let udp = UdpSocket::connect("0000:0000:0000:0000:0000:0000:0000:0001:654");
        assert!(udp.is_err());
    }
}
