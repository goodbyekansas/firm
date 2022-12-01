use std::io::Error as IoError;

use thiserror::Error;
use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};

const EVENT_LOG_SOURCES: &str = "SYSTEM\\CurrentControlSet\\Services\\EventLog\\Application";

#[derive(Error, Debug)]
pub enum EventLogError {
    #[error("Failed to add log source \"{0}\" ({1}): {2}")]
    FailedToAddLogSource(String, String, IoError),

    #[error("Failed to add log source \"{0}\": {1}")]
    FailedToRemoveLogSource(String, IoError),
}

impl From<EventLogError> for u32 {
    fn from(event_log_error: EventLogError) -> Self {
        match event_log_error {
            EventLogError::FailedToAddLogSource(_, _, _) => 50,
            EventLogError::FailedToRemoveLogSource(_, _) => 51,
        }
    }
}

pub fn add_log_source(name: &str, exe_path: &str) -> Result<(), EventLogError> {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(EVENT_LOG_SOURCES)
        .and_then(|cur_ver| cur_ver.create_subkey(name))
        .and_then(|(app_key, _)| app_key.set_value("EventMessageFile", &exe_path))
        .map_err(|e| EventLogError::FailedToAddLogSource(name.to_owned(), exe_path.to_owned(), e))
}

pub fn remove_log_source(name: &str) -> Result<(), EventLogError> {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(EVENT_LOG_SOURCES)
        .and_then(|cur_ver| cur_ver.delete_subkey(name))
        .map_err(|e| EventLogError::FailedToRemoveLogSource(name.to_owned(), e))
}
