use std::path::{Path, PathBuf};

use regex::Regex;
use slog::{Logger, debug, o};
use thiserror::Error;
use winapi::{
    shared::ntdef::{LPCWSTR, NULL},
    um::winbase::{MoveFileExW, MOVEFILE_DELAY_UNTIL_REBOOT},
};
use winreg::{
    enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE},
    RegKey,
};

const ENVIRONMENT_KEY: &str = r#"SYSTEM\CurrentControlSet\Control\Session Manager\Environment"#;
const PENDING_REMOVAL_KEY: &str = r#"SYSTEM\CurrentControlSet\Control\Session Manager"#;
const FIRM_KEY: &str = r#"SOFTWARE\Firm"#;
const UNINSTALL_KEY: &str = r#"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"#;

#[derive(Error, Debug)]
pub enum RegistryError {
    #[error("Failed to register Firm: {0}")]
    FailedToRegister(Box<dyn std::error::Error>),

    #[error("Failed to deregister Firm: {0}")]
    FailedToDeregister(Box<dyn std::error::Error>),

    #[error("Failed to add Firm to PATH: {0}")]
    FailedToAddToPath(Box<dyn std::error::Error>),

    #[error("Failed to remove Firm from PATH: {0}")]
    FailedToRemoveFromPath(Box<dyn std::error::Error>),

    #[error("Failed to register uninstaller: {0}")]
    FailedToRegisterUninstaller(Box<dyn std::error::Error>),

    #[error("Failed to deregister uninstaller: {0}")]
    FailedToDeregisterUninstaller(Box<dyn std::error::Error>),

    #[error("Failed to mark file for reboot deletion: {0}")]
    FailedToMarkFileForRebootDeletion(String),

    #[error(r#"Failed to mark folder "{0}" for deletion: {1}"#)]
    FailedToMarkDirectoryForRebootDeletion(String, std::io::Error),

    #[error("Failed to cancel file deletion: {0}")]
    FailedToCancelFileDeletion(std::io::Error),
}

impl From<RegistryError> for u32 {
    fn from(registry_error: RegistryError) -> Self {
        match registry_error {
            RegistryError::FailedToRegister(_) => 10,
            RegistryError::FailedToDeregister(_) => 11,
            RegistryError::FailedToAddToPath(_) => 12,
            RegistryError::FailedToRemoveFromPath(_) => 13,
            RegistryError::FailedToRegisterUninstaller(_) => 14,
            RegistryError::FailedToDeregisterUninstaller(_) => 15,
            RegistryError::FailedToMarkFileForRebootDeletion(_) => 16,
            RegistryError::FailedToMarkDirectoryForRebootDeletion(_, _) => 17,
            RegistryError::FailedToCancelFileDeletion(_) => 18,
        }
    }
}

pub fn register_firm(exe_path: &Path, data_path: &Path) -> Result<(), RegistryError> {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .create_subkey(FIRM_KEY)
        .and_then(|(key, _)| {
            key.set_value("InstallPath", &exe_path.to_string_lossy().to_string())
                .and_then(|_| key.set_value("DataPath", &data_path.to_string_lossy().to_string()))
                .and_then(|_| key.set_value("Version", &String::from(std::env!("version"))))
        })
        .map_err(|e| RegistryError::FailedToRegister(e.into()))
}

pub fn deregister_firm() -> Result<(), RegistryError> {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .delete_subkey_all(FIRM_KEY)
        .map_err(|e| RegistryError::FailedToDeregister(e.into()))
}

pub fn add_to_path(location: &Path) -> Result<(), RegistryError> {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(ENVIRONMENT_KEY, KEY_READ | KEY_WRITE)
        .and_then(|key| {
            {
                key.get_value("Path").and_then(|old_path: String| {
                    if old_path.contains(&location.to_string_lossy().to_string()) {
                        Ok(())
                    } else {
                        key.set_value(
                            "Path",
                            &format!("{};{}", old_path, location.to_string_lossy()),
                        )
                    }
                })
            }
        })
        .map_err(|e| RegistryError::FailedToAddToPath(e.into()))
}

pub fn register_uninstaller(exe_path: &Path) -> Result<(), RegistryError> {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(UNINSTALL_KEY)
        .and_then(|key| {
            key.create_subkey_with_flags("Firm", KEY_READ | KEY_WRITE)
                .and_then(|(firm, _)| {
                    firm.set_value("DisplayName", &String::from("Firm"))
                        .and_then(|_| {
                            firm.set_value(
                                "UninstallString",
                                &format!(
                                    "{} uninstall",
                                    exe_path.join("install.exe").to_string_lossy()
                                ),
                            )
                        })
                        .and_then(|_| {
                            firm.set_value(
                                "InstallLocation",
                                &exe_path.to_string_lossy().to_string(),
                            )
                        })
                        .and_then(|_| {
                            firm.set_value("DisplayVersion", &String::from(std::env!("version")))
                        })
                        .and_then(|_| {
                            firm.set_value(
                                "URLInfoAbout",
                                &String::from("https://github.com/goodbyekansas/firm"),
                            )
                        })
                })
        })
        .map_err(|e| RegistryError::FailedToRegisterUninstaller(e.into()))
}

pub fn deregister_uninstaller() -> Result<(), RegistryError> {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(UNINSTALL_KEY, KEY_WRITE)
        .and_then(|key| key.delete_subkey_all("Firm"))
        .map_err(|e| RegistryError::FailedToDeregisterUninstaller(e.into()))
}

pub fn remove_from_path(location: &Path) -> Result<(), RegistryError> {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(ENVIRONMENT_KEY, KEY_READ | KEY_WRITE)
        .and_then(|key| {
            {
                key.get_value("Path").and_then(|old_path: String| {
                    let location_string = location.to_string_lossy();
                    // To cover the cases where firm is last in the path and to not
                    // catch C:\Program Files\Firmware
                    if old_path.ends_with(&location_string.to_string())
                        || old_path.contains(&format!(";{};", location_string))
                    {
                        key.set_value(
                            "Path",
                            &old_path.replace(&format!(";{}", location_string), ""),
                        )
                    } else {
                        Ok(())
                    }
                })
            }
        })
        .map_err(|e| RegistryError::FailedToRemoveFromPath(e.into()))
}

pub fn find_firm(logger: Logger) -> (PathBuf, PathBuf) {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(FIRM_KEY)
        .map(|key| {
            (
                key.get_value::<String, &str>("InstallPath")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| {
                        super::default_path_from_env(&logger, "PROGRAMFILES", r#"C:\Program Files"#)
                    }),
                key.get_value::<String, &str>("DataPath")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| {
                        super::default_path_from_env(&logger.new(o!()), "PROGRAMDATA", r#"C:\ProgramData"#)
                    }),
            )
        })
        .unwrap_or_else(|e| {
            debug!(
                logger,
                "Failed to get install paths from registry, fallback to default: {}", e
            );
            (
                super::default_path_from_env(&logger, "PROGRAMFILES", r#"C:\Program Files"#),
                super::default_path_from_env(&logger, "PROGRAMDATA", r#"C:\ProgramData"#),
            )
        })
}

fn remove_paths_from_string(path: &Path, data: &str) -> String {
    let pattern = Regex::new(&format!(
        r#"(?m)^\\\?\?\\{}.*$"#,
        path.to_string_lossy().into_owned().escape_default()
    ))
    .unwrap();
    pattern.replace_all(data, "").into_owned()
}

pub fn cancel_pending_deletions(path: &Path) -> Result<(), RegistryError> {
    const PENDING_OPERATIONS: &str = "PendingFileRenameOperations";
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(PENDING_REMOVAL_KEY, KEY_READ | KEY_WRITE)
        .and_then(|key| {
            key.get_value::<String, &str>(PENDING_OPERATIONS)
                .and_then(|v| {
                    key.set_value(PENDING_OPERATIONS, &remove_paths_from_string(path, &v))
                })
        })
        .map_err(RegistryError::FailedToCancelFileDeletion)
}

fn mark_file_for_reboot_delete(path: &Path) -> Option<RegistryError> {
    let win_path = crate::service::win_string(&path.to_string_lossy());
    (unsafe {
        MoveFileExW(
            win_path.as_ptr(),
            NULL as LPCWSTR,
            MOVEFILE_DELAY_UNTIL_REBOOT,
        )
    } == 0)
        .then(|| {
            RegistryError::FailedToMarkFileForRebootDeletion(path.to_string_lossy().into_owned())
        })
}

fn recursively_mark_for_delete(path: &Path) -> Vec<RegistryError> {
    std::fs::read_dir(path)
        .map(|entries| {
            entries
                .filter_map(|entry| {
                    if let Ok(p) = entry {
                        if p.path().is_dir() {
                            Some(recursively_mark_for_delete(&p.path()))
                        } else {
                            mark_file_for_reboot_delete(&p.path()).map(|v| vec![v])
                        }
                    } else {
                        None
                    }
                })
                .flatten()
                .collect::<Vec<RegistryError>>()
        })
        .unwrap_or_else(|e| {
            vec![RegistryError::FailedToMarkDirectoryForRebootDeletion(
                path.to_string_lossy().into_owned(),
                e,
            )]
        })
}

pub fn mark_folder_for_deletion(path: &Path) -> Vec<RegistryError> {
    if path.is_dir() {
        recursively_mark_for_delete(&path)
    } else {
        vec![]
    }
    .into_iter()
    .chain(mark_file_for_reboot_delete(path).into_iter())
    .collect()
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_remove_pending_file_operations() {
        let data = r#"\??\C:\Program Files\Firm\avery.exe

\??\C:\Program Files\Firm\bendini.exe

\??\C:\Program Files\Firm\install.exe

\??\C:\Program Files\Firm\lomax.exe

\??\C:\Program Files\Firm

\??\D:\Program Files\Firm
"#;
        let new_data = remove_paths_from_string(Path::new(r#"C:\Program Files\Firm"#), data);
        assert!(new_data.contains(r#"\??\D:\Program Files\Firm"#));
        assert!(!new_data.contains(r#"\??\C:\Program Files\Firm"#));
        assert!(!new_data.contains(r#"\??\C:\Program Files\Firm\bendini.exe"#));

        let data = r#"\??\C:\Windows\Temp\4548b014-cc8b-4de6-b305-1afb8991f0ad.tmp

\??\C:\Program Files\Firm\avery.exe

\??\C:\Program Files\Firm\bendini.exe

\??\C:\Program Diles\Dirm\dendini.dexe

\??\C:\Program Files\Firm\install.exe

\??\C:\Program Files\Firm\lomax.exe

\??\C:\Program Files\Firm

\??\D:\Program Files\Firm
"#;
        let new_data = remove_paths_from_string(Path::new(r#"C:\Program Files\Firm"#), data);
        assert!(new_data.contains(r#"\??\D:\Program Files\Firm"#));
        assert!(new_data.contains(r#"\??\C:\Program Diles\Dirm\dendini.dexe"#));
        assert!(
            new_data.contains(r#"\??\C:\Windows\Temp\4548b014-cc8b-4de6-b305-1afb8991f0ad.tmp"#)
        );
        assert!(!new_data.contains(r#"\??\C:\Program Files\Firm"#));
        assert!(!new_data.contains(r#"\??\C:\Program Files\Firm\bendini.exe"#));
    }
}
