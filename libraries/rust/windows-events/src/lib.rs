use std::{
    ffi::OsStr,
    fmt,
    io::{self, Error as IoError},
    iter::once,
    os::windows::ffi::OsStrExt,
    ptr,
};

use slog::{Drain, Level, OwnedKVList, Record, KV};
use thiserror::Error;
use winapi::{
    shared::{
        minwindef::BYTE,
        ntdef::{HANDLE, NULL},
    },
    um::{
        winbase::{DeregisterEventSource, RegisterEventSourceW, ReportEventW},
        winnt::{EVENTLOG_ERROR_TYPE, EVENTLOG_INFORMATION_TYPE, EVENTLOG_WARNING_TYPE, PSID},
    },
};
use windows_acl::helper::{current_user, name_to_sid};
use winreg::{enums::*, RegKey};

mod eventmsgs;
use eventmsgs::{MSG_DEBUG, MSG_ERROR, MSG_INFO, MSG_TRACE, MSG_WARNING};

const REG_BASEKEY: &str = "SYSTEM\\CurrentControlSet\\Services\\EventLog\\Application";

#[derive(Error, Debug)]
pub enum EventError {
    #[error("IO Error: {0}")]
    Io(io::Error),

    #[error("Could not determine executable path")]
    ExePathNotFound,

    #[error(r#"Failed to register event source "{0}": {1}"#)]
    RegisterSourceFailed(String, IoError),

    #[error(r#"Failed to deregister event source "{0}": {1}"#)]
    DeregisterSourceFailed(String, IoError),

    #[error(r#"Failed to register logger for event source "{0}": {1}"#)]
    RegisterLoggerSource(String, String),
}

impl From<io::Error> for EventError {
    fn from(err: io::Error) -> EventError {
        EventError::Io(err)
    }
}

impl From<EventError> for String {
    fn from(val: EventError) -> Self {
        val.to_string()
    }
}

pub struct WinLogger {
    handle: HANDLE,
    sid_ptr: PSID,
    #[allow(dead_code)] // Pointers expect this data to be present
    sid_data: Option<Vec<BYTE>>,
}

unsafe impl Send for WinLogger {}
unsafe impl Sync for WinLogger {}

fn win_string(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(once(0)).collect()
}

pub fn try_register_current(name: &str) -> Result<(), EventError> {
    std::env::current_exe()?
        .to_str()
        .ok_or(EventError::ExePathNotFound)
        .and_then(|exe| try_register(name, exe))
}

pub fn try_register(name: &str, exe_path: &str) -> Result<(), EventError> {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(REG_BASEKEY)
        .and_then(|current_version| current_version.create_subkey(name))
        .and_then(|(app_key, _)| app_key.set_value("EventMessageFile", &exe_path))
        .map_err(|e| EventError::RegisterSourceFailed(name.to_string(), e))
}

pub fn try_deregister(name: &str) -> Result<(), EventError> {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(REG_BASEKEY)
        .and_then(
            |current_version| match current_version.delete_subkey(name) {
                Ok(_) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e),
            },
        )
        .map_err(|e| EventError::DeregisterSourceFailed(name.to_string(), e))
}

impl WinLogger {
    pub fn try_new(name: &str) -> Result<WinLogger, EventError> {
        let sid_data = current_user().and_then(|user| name_to_sid(&user, None).ok());
        let sid_ptr = sid_data
            .as_ref()
            .map(|v| v.as_ptr() as PSID)
            .unwrap_or(NULL);
        match unsafe { RegisterEventSourceW(ptr::null_mut(), win_string(name).as_ptr()) } {
            NULL => Err(EventError::RegisterLoggerSource(
                name.to_string(),
                String::from("Failed to get handle for registration"),
            )),
            handle => Ok(WinLogger {
                handle,
                sid_ptr,
                sid_data,
            }),
        }
    }
}

impl Drop for WinLogger {
    fn drop(&mut self) {
        unsafe { DeregisterEventSource(self.handle) };
    }
}

impl Drain for WinLogger {
    type Err = io::Error;
    type Ok = ();

    fn log(&self, info: &Record, logger_values: &OwnedKVList) -> io::Result<()> {
        let (message_type, msg) = match info.level() {
            Level::Critical => (EVENTLOG_ERROR_TYPE, MSG_ERROR),
            Level::Error => (EVENTLOG_ERROR_TYPE, MSG_ERROR),
            Level::Warning => (EVENTLOG_WARNING_TYPE, MSG_WARNING),
            Level::Info => (EVENTLOG_INFORMATION_TYPE, MSG_INFO),
            Level::Debug => (EVENTLOG_INFORMATION_TYPE, MSG_DEBUG),
            Level::Trace => (EVENTLOG_INFORMATION_TYPE, MSG_TRACE),
        };

        let mut ksv = KeyValueParser::new();
        info.kv()
            .serialize(info, &mut ksv)
            .and_then(|_| logger_values.serialize(info, &mut ksv))?;

        let message = win_string(&format!(
            "{} \n{}",
            info.msg(),
            ksv.kv.iter().fold(String::new(), |acc, val| {
                format!(
                    "{} \n\
                             {}: {}",
                    acc, val.0, val.1
                )
            })
        ));
        let mut vec = vec![message.as_ptr()];

        unsafe {
            ReportEventW(
                self.handle,
                message_type,
                0,
                msg,
                self.sid_ptr,
                vec.len() as u16,
                0,
                vec.as_mut_ptr(),
                ptr::null_mut(),
            )
        };

        Ok(())
    }
}

struct KeyValueParser {
    kv: Vec<(String, String)>,
}

impl KeyValueParser {
    pub fn new() -> Self {
        KeyValueParser { kv: Vec::new() }
    }
}

impl slog::Serializer for KeyValueParser {
    fn emit_arguments(&mut self, key: &str, val: &fmt::Arguments) -> slog::Result {
        self.kv.push((key.to_string(), val.to_string()));
        Ok(())
    }
}
