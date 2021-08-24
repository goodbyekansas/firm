use std::{io, os::windows::ffi::OsStrExt, ptr, sync::atomic::Ordering::Relaxed};

use slog::{Drain, Level, OwnedKVList, Record, KV};
use winapi::{
    shared::{minwindef::BYTE, ntdef::NULL},
    um::{
        errhandlingapi::GetLastError,
        winbase::ReportEventW,
        winnt::{EVENTLOG_ERROR_TYPE, EVENTLOG_INFORMATION_TYPE, EVENTLOG_WARNING_TYPE, PSID},
    },
};
use windows_acl::helper::{current_user, name_to_sid};

struct WinRecord {
    event_type: u16,
    event_id: u32,
    message: Vec<u16>,
}

impl WinRecord {
    fn from_record(info: &Record, level: Level, logger_values: &OwnedKVList) -> Option<Self> {
        (info.level() <= level)
            .then(|| match info.level() {
                // These hex valuese depend on the winlog crate version 0.2.6.
                // The winlog crate generates a bunch of enums from the windows message and resource compiler
                // These enums are not public and relies on the behaviour of the message and resource compiler
                // together with the resource files in the winlog repository. Depending on this crate also
                // ensures we get these message resources compiled into our executable which is a requirement
                // for windows to parse events in the event logger correctly (this is why windows locks a bunch
                // of executables if you are in the event viewer)
                // TODO: Generate our own resources for our executable.
                Level::Critical => (0xC0000001, EVENTLOG_ERROR_TYPE),
                Level::Error => (0xC0000001, EVENTLOG_ERROR_TYPE),
                Level::Warning => (0x80000002, EVENTLOG_WARNING_TYPE),
                Level::Info => (0x40000003, EVENTLOG_INFORMATION_TYPE),
                Level::Debug => (0x40000004, EVENTLOG_INFORMATION_TYPE),
                Level::Trace => (0x40000005, EVENTLOG_INFORMATION_TYPE),
            })
            .map(|(event_id, event_type)| {
                let mut ksv = KeyValueParser::new();
                info.kv().serialize(info, &mut ksv).unwrap();
                logger_values.serialize(info, &mut ksv).unwrap();

                WinRecord {
                    event_type,
                    event_id,
                    message: win_string(
                        (format!(
                            "{} \n{}",
                            info.msg(),
                            ksv.kv.iter().fold(String::new(), |acc, val| {
                                format!(
                                    "{} \n\
                             {}: {}",
                                    acc, val.0, val.1
                                )
                            })
                        ))
                        .as_str(),
                    ),
                }
            })
    }
}

fn win_string(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

pub struct WinEvent {
    source_handle: std::sync::atomic::AtomicPtr<winapi::ctypes::c_void>,
    sid: Option<Vec<BYTE>>,
    level: slog::Level,
}

impl WinEvent {
    pub fn new(name: &str) -> Self {
        Self {
            source_handle: std::sync::atomic::AtomicPtr::new(unsafe {
                winapi::um::winbase::RegisterEventSourceW(ptr::null(), win_string(name).as_ptr())
            }),
            sid: current_user().and_then(|user| name_to_sid(&user, None).ok()),
            level: slog::Level::Trace,
        }
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
    fn emit_arguments(&mut self, key: &str, val: &std::fmt::Arguments) -> slog::Result {
        self.kv.push((key.to_string(), val.to_string()));
        Ok(())
    }
}

unsafe impl Send for WinEvent {}

impl Drain for WinEvent {
    type Err = io::Error;
    type Ok = ();

    fn log(&self, info: &Record, logger_values: &OwnedKVList) -> io::Result<()> {
        WinRecord::from_record(info, self.level, logger_values)
            .and_then(|win_record| {
                let mut message = vec![win_record.message.as_ptr()];
                unsafe {
                    (ReportEventW(
                        self.source_handle.load(Relaxed),
                        win_record.event_type,
                        0,
                        win_record.event_id,
                        self.sid
                            .to_owned()
                            .map(|mut s| s.as_mut_ptr() as PSID)
                            .unwrap_or(NULL),
                        message.len() as u16,
                        0,
                        message.as_mut_ptr(),
                        ptr::null_mut(),
                    ) == 0)
                        .then(|| GetLastError())
                }
            })
            .map_or_else(
                || Ok(()),
                |error| {
                    Err(io::Error::new(
                        io::ErrorKind::Other,
                        format!("Failed to report event to Event Log: {}", error),
                    ))
                },
            )
    }
}
