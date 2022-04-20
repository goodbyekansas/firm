use std::{
    cell::RefCell,
    ffi::CStr,
    os::raw::{c_char, c_void},
};

use firm_function::{
    host::{ApiSize, ChannelData, ChannelType, Error as HostError, StartProcessRequest},
    host_call,
};
use thiserror::Error;

macro_rules! c_str_to_str {
    ($value:expr) => {{
        let string = CStr::from_ptr($value).to_str().map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Failed to convert C string to &str: {}", e),
            )
        });

        if let Err(e) = string {
            return ApiResult::err(LastError::Generic(e.to_string()));
        }

        string.unwrap()
    }};
}

#[derive(Error, Debug)]
enum LastError {
    #[error("{0}")]
    Generic(String),

    #[error("Call would have blocked the calling thread.")]
    BlockingCall,

    #[error("Input \"{0}\" contains no more data.")]
    EndOfInput(String),
}

impl From<HostError> for LastError {
    fn from(error: HostError) -> Self {
        match error {
            HostError::WouldBlock => LastError::BlockingCall,
            e => LastError::Generic(e.to_string()),
        }
    }
}

impl LastError {
    fn with_context(self, context: &str) -> Self {
        match self {
            LastError::Generic(s) => LastError::Generic(format!("{}: {}", context, s)),
            _ => self,
        }
    }
}

macro_rules! impl_get_single {
    ($function_name:ident, $rust_type:ty, $c_type:ty, $channel_type:expr) => {
        /// # Safety
        /// It isn't
        #[no_mangle]
        unsafe extern "C" fn $function_name(
            key: *const c_char,
            blocking: bool,
            result: *mut $c_type,
        ) -> ApiResult {
            let key = c_str_to_str!(key);
            match firm_function::host::get_channel_data::<$rust_type>(key, 1, blocking) {
                Ok(channel_data) => {
                    if channel_data.count() > 0 {
                        *result = *(channel_data.data() as *mut _);
                        ApiResult::ok()
                    } else {
                        ApiResult::err(LastError::EndOfInput(key.to_owned()))
                    }
                }
                Err(e) => {
                    let e = LastError::from(e).with_context(&format!(
                        "Failed to get {} input for \"{}\"",
                        $channel_type.to_string(),
                        key
                    ));
                    ApiResult::err(e)
                }
            }
        }
    };
    ($function_name:ident, $rust_type:ty, $channel_type:expr) => {
        impl_get_single!($function_name, $rust_type, $rust_type, $channel_type);
    };
}

impl_get_single!(ff_next_string, String, *mut c_char, ChannelType::String);
impl_get_single!(ff_next_int, i64, ChannelType::Integer);
impl_get_single!(ff_next_float, f64, ChannelType::Float);
impl_get_single!(ff_next_bool, bool, ChannelType::Boolean);
impl_get_single!(ff_next_byte, u8, ChannelType::Byte);

macro_rules! impl_get_multiple {
    ($function_name:ident, $rust_type:ty, $c_type:ty, $channel_type:expr) => {
        /// # Safety
        /// It isn't
        #[no_mangle]
        unsafe extern "C" fn $function_name(
            key: *const c_char,
            blocking: bool,
            size: ApiSize,
            result: *mut *mut $c_type,
            size_out: &mut ApiSize,
        ) -> ApiResult {
            let key = c_str_to_str!(key);
            match firm_function::host::get_channel_data::<$rust_type>(key, size, blocking) {
                Ok(mut channel_data) => {
                    if channel_data.count() > 0 {
                        *result = (channel_data.data() as *mut _);
                        *size_out = channel_data.count();
                        channel_data.leak_array();
                        ApiResult::ok()
                    } else {
                        ApiResult::err(LastError::EndOfInput(key.to_owned()))
                    }
                }
                Err(e) => {
                    let e = LastError::from(e).with_context(&format!(
                        "Failed to get {} inputs for \"{}\"",
                        $channel_type.to_string(),
                        key
                    ));
                    ApiResult::err(e)
                }
            }
        }
    };
    ($function_name:ident, $rust_type:ty, $channel_type:expr) => {
        impl_get_multiple!($function_name, $rust_type, $rust_type, $channel_type);
    };
}

impl_get_multiple!(ff_strings, String, *mut c_char, ChannelType::String);
impl_get_multiple!(ff_ints, i64, ChannelType::Integer);
impl_get_multiple!(ff_floats, f64, ChannelType::Float);
impl_get_multiple!(ff_bools, bool, ChannelType::Boolean);
impl_get_multiple!(ff_bytes, u8, ChannelType::Byte);

macro_rules! impl_get_iterator {
    ($iterator_type_name:ident, $function_name:ident, $data_type:ty) => {
        impl_get_iterator!($iterator_type_name, $function_name, $data_type, $data_type);
    };
    ($iterator_type_name:ident, $function_name:ident, $data_type:ty, $rust_type:ty) => {
        #[repr(C)]
        struct $iterator_type_name {
            channel_name: String,
            fetch_size: u32,
            blocking: bool,
            data: ChannelData,
            index: usize,
        }

        impl $iterator_type_name {
            fn new(channel_name: String, fetch_size: u32, blocking: bool) -> Self {
                Self {
                    channel_name,
                    fetch_size,
                    blocking,
                    data: ChannelData::default(),
                    index: 0,
                }
            }

            fn new_data(&mut self, data: ChannelData) {
                self.index = 0;
                self.data = data;
            }
        }

        impl Iterator for $iterator_type_name {
            type Item = Result<*const $data_type, LastError>;

            fn next(&mut self) -> Option<Self::Item> {
                (self.index < self.data.count() as usize)
                    .then(|| {
                        let index = self.index as isize;
                        self.index += 1;
                        Ok(unsafe {
                            // Need to cast early so the offset step corresponds to the type of pointer.
                            (self.data.data() as *const $data_type).offset(index as isize)
                                as *const $data_type
                        })
                    })
                    .or_else(|| {
                        unsafe {
                            firm_function::host::get_channel_data::<$rust_type>(
                                &self.channel_name,
                                self.fetch_size,
                                self.blocking,
                            )
                        }
                        .map_err(LastError::from)
                        .and_then(|v| {
                            self.new_data(v);

                            // if we got zero back, we need to stop iteration to not
                            // recurse forever
                            (self.data.count() > 0)
                                .then(|| self.next())
                                .flatten()
                                .transpose()
                        })
                        .transpose()
                    })
            }
        }

        /// # Safety
        /// lol
        #[no_mangle]
        unsafe extern "C" fn $function_name(
            key: *const c_char,
            fetch_size: u32,
            blocking: bool,
            result: *mut *mut $iterator_type_name,
        ) -> ApiResult {
            let channel_name = c_str_to_str!(key).to_owned();
            *result = Box::into_raw(Box::new($iterator_type_name::new(
                channel_name,
                fetch_size,
                blocking,
            )));
            ApiResult::ok()
        }
    };
}

impl_get_iterator!(StringIterator, ff_string_iterator, *const c_char, String);
impl_get_iterator!(IntIterator, ff_int_iterator, i64);
impl_get_iterator!(FloatIterator, ff_float_iterator, f64);
impl_get_iterator!(BoolIterator, ff_bool_iterator, bool);
impl_get_iterator!(ByteIterator, ff_byte_iterator, u8);

macro_rules! impl_iterator_next {
    ($iterator_type:ty, $function_name:ident, $rust_type:ty) => {
        impl_iterator_next!($iterator_type, $function_name, $rust_type, $rust_type);
    };

    ($iterator_type:ty, $function_name:ident, $rust_type:ty, $c_type:ty) => {
        /// # Safety
        /// use seatbelt?
        #[no_mangle]
        unsafe extern "C" fn $function_name(
            iterator: &mut $iterator_type,
            result: *mut $c_type,
        ) -> ApiResult {
            match iterator.next().map(|res| {
                res.map(|value_ptr| {
                    *result = *(value_ptr as *mut _);
                })
            }) {
                // More input data
                Some(Ok(_)) => ApiResult::ok(),
                // Error when trying to get more input data
                Some(Err(e)) => ApiResult::err(e),
                // No more input data, but everything went fine
                None => ApiResult::err(LastError::EndOfInput(iterator.channel_name.clone())),
            }
        }
    };
}

impl_iterator_next!(StringIterator, ff_iterator_next_string, String, *mut c_char);
impl_iterator_next!(IntIterator, ff_iterator_next_int, i64);
impl_iterator_next!(FloatIterator, ff_iterator_next_float, f64);
impl_iterator_next!(BoolIterator, ff_iterator_next_bool, bool);
impl_iterator_next!(ByteIterator, ff_iterator_next_byte, u8);

macro_rules! impl_iterator_collect {
    ($iterator_type:ty, $function_name:ident, $rust_type:ty) => {
        /// # Safety
        /// No clippy, it is not
        #[no_mangle]
        unsafe extern "C" fn $function_name(
            iter: &mut $iterator_type,
            result: *mut *mut $rust_type,
            num_out: *mut ApiSize,
        ) -> ApiResult {
            match iter
                .map(|result| result.map(|i| *i as $rust_type))
                .collect::<Result<Vec<$rust_type>, LastError>>()
            {
                Ok(mut v) => {
                    v.shrink_to_fit();
                    let static_ref: &'static mut [$rust_type] = v.leak();
                    *result = static_ref.as_mut_ptr();
                    *num_out = static_ref.len() as ApiSize;
                    ApiResult::ok()
                }
                Err(e) => ApiResult::err(e),
            }
        }
    };
}

impl_iterator_collect!(StringIterator, ff_iterator_collect_strings, *mut c_char);
impl_iterator_collect!(IntIterator, ff_iterator_collect_ints, i64);
impl_iterator_collect!(FloatIterator, ff_iterator_collect_floats, f64);
impl_iterator_collect!(BoolIterator, ff_iterator_collect_bools, bool);
impl_iterator_collect!(ByteIterator, ff_iterator_collect_bytes, u8);

macro_rules! impl_close_iterator {
    ($iterator_type:ty, $function_name:ident) => {
        /// # Safety
        /// It isn't
        #[no_mangle]
        unsafe extern "C" fn $function_name(iterator: *mut $iterator_type) {
            let _ = Box::from_raw(iterator);
        }
    };
}

impl_close_iterator!(StringIterator, ff_close_string_iterator);
impl_close_iterator!(IntIterator, ff_close_int_iterator);
impl_close_iterator!(FloatIterator, ff_close_float_iterator);
impl_close_iterator!(BoolIterator, ff_close_bool_iterator);
impl_close_iterator!(ByteIterator, ff_close_byte_iterator);

macro_rules! impl_append_data {
    ($rust_type:ty, $function_name:ident) => {
        /// # Safety
        /// It isn't
        #[no_mangle]
        unsafe extern "C" fn $function_name(
            key: *const c_char,
            data: *const $rust_type,
            count: ApiSize,
        ) -> ApiResult {
            let data_slice = std::slice::from_raw_parts::<$rust_type>(data, count as usize);

            match firm_function::host::append_channel_data(c_str_to_str!(key), data_slice) {
                Ok(_) => ApiResult::ok(),
                Err(e) => ApiResult::err(LastError::Generic(e.to_string())),
            }
        }
    };
}

impl_append_data!(*const c_char, ff_append_string_output);
impl_append_data!(i64, ff_append_int_output);
impl_append_data!(f64, ff_append_float_output);
impl_append_data!(bool, ff_append_bool_output);
impl_append_data!(u8, ff_append_byte_output);

/// # Safety
/// It isn't
#[no_mangle]
unsafe extern "C" fn ff_close_output(key: *const c_char) -> ApiResult {
    let channel_name = c_str_to_str!(key).to_owned();
    host_call!(firm_function::host::__close_output(key))
        .map_err(|e| {
            LastError::from(e)
                .with_context(&format!("Failed to close output \"{}\"", &channel_name))
        })
        .into()
}

/// # Safety
/// It isn't
#[no_mangle]
unsafe extern "C" fn ff_map_attachment(
    attachment_name: *const c_char,
    unpack: bool,
    path_out: *mut *const c_char,
) -> ApiResult {
    host_call!(firm_function::host::__map_attachment(
        attachment_name,
        unpack,
        path_out,
    ))
    .map_err(|e| {
        LastError::from(e).with_context(&format!(
            "Failed to map attachment \"{}\"",
            CStr::from_ptr(attachment_name)
                .to_str()
                .unwrap_or("invalid utf-8 string")
        ))
    })
    .into()
}

/// # Safety
/// It isn't
#[no_mangle]
unsafe extern "C" fn ff_host_path_exists(path: *const c_char, exists: *mut bool) -> ApiResult {
    host_call!(firm_function::host::__host_path_exists(path, exists))
        .map_err(|e| LastError::from(e).with_context("Failed to check if host path exists"))
        .into()
}

/// # Safety
/// It isn't
#[no_mangle]
unsafe extern "C" fn ff_start_host_process(
    request: *const StartProcessRequest,
    pid_out: *mut u64,
    exit_code_out: *mut i64,
) -> ApiResult {
    host_call!(firm_function::host::__start_host_process(
        request,
        pid_out,
        exit_code_out,
    ))
    .map_err(|e| LastError::from(e).with_context("Failed to start host process"))
    .into()
}

/// # Safety
/// Do not try this at home
#[no_mangle]
unsafe extern "C" fn ff_get_host_os(os_out: *mut *const c_char) -> ApiResult {
    host_call!(firm_function::host::__host_os(os_out))
        .map_err(|e| LastError::from(e).with_context("Failed to get host OS"))
        .into()
}

/// # Safety
/// Do not try this at home
#[no_mangle]
unsafe extern "C" fn ff_set_function_error(msg: *const c_char) -> ApiResult {
    host_call!(firm_function::host::__set_error(msg))
        .map_err(|e| LastError::from(e).with_context("Failed to set error message"))
        .into()
}

#[no_mangle]
unsafe extern "C" fn ff_connect(address: *const c_char, socket_out: *mut i32) -> ApiResult {
    host_call!(firm_function::host::__connect(address, socket_out))
        .map_err(|e| LastError::from(e).with_context("Failed to connect"))
        .into()
}

#[repr(u8)]
enum ApiResultKind {
    Ok = 0,
    Blocked,
    EndOfInput,
    Error,
}

thread_local! {
    pub static LAST_ERROR: RefCell<Vec<u8>> = RefCell::new(Vec::with_capacity(0));
}

#[repr(C)]
struct ApiResult {
    kind: ApiResultKind,
    error_msg: *const c_char,
}

impl ApiResult {
    fn ok() -> Self {
        Self {
            kind: ApiResultKind::Ok,
            error_msg: std::ptr::null(),
        }
    }

    fn err(e: LastError) -> Self {
        let error_msg = LAST_ERROR.with(|msg| {
            let mut v = e.to_string().into_bytes();
            v.push(b'\0');
            *msg.borrow_mut() = v;
            msg.borrow().as_ptr() as *const c_char
        });
        Self {
            kind: match e {
                LastError::Generic(_) => ApiResultKind::Error,
                LastError::BlockingCall => ApiResultKind::Blocked,
                LastError::EndOfInput(_) => ApiResultKind::EndOfInput,
            },
            error_msg,
        }
    }

    fn is_ok(&self) -> bool {
        matches!(self.kind, ApiResultKind::Ok)
    }

    fn is_err(&self) -> bool {
        !self.is_ok()
    }
}

impl From<LastError> for ApiResult {
    fn from(e: LastError) -> Self {
        ApiResult::err(e)
    }
}

impl From<Result<(), LastError>> for ApiResult {
    fn from(r: Result<(), LastError>) -> Self {
        match r {
            Ok(_) => Self::ok(),
            Err(e) => Self::err(e),
        }
    }
}

/// # Safety
/// Do not try this at home
#[no_mangle]
unsafe extern "C" fn ff_result_is_ok(result: &ApiResult) -> bool {
    result.is_ok()
}

/// # Safety
/// Do not try this at home
#[no_mangle]
unsafe extern "C" fn ff_result_is_err(result: &ApiResult) -> bool {
    result.is_err()
}

/// # Safety
/// Do not try this at home
#[no_mangle]
unsafe extern "C" fn ff_result_would_block(result: &ApiResult) -> bool {
    matches!(result.kind, ApiResultKind::Blocked)
}

/// # Safety
/// Do not try this at home
#[no_mangle]
unsafe extern "C" fn ff_result_is_end_of_input(result: &ApiResult) -> bool {
    matches!(result.kind, ApiResultKind::EndOfInput)
}
