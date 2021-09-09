use std::{
    io::Error as IoError,
    {collections::HashMap, path::Path},
};

use regex::Regex;
use std::path::PathBuf;
use thiserror::Error;
use winapi::um::winreg::REGSAM;
use winreg::{
    enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE},
    types::FromRegValue,
    RegKey,
};

const ENVIRONMENT_KEY: &str = r#"SYSTEM\CurrentControlSet\Control\Session Manager\Environment"#;
const PENDING_REMOVAL_KEY: &str = r#"SYSTEM\CurrentControlSet\Control\Session Manager"#;
const UNINSTALL_KEY: &str = r#"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"#;
const PENDING_OPERATIONS: &str = "PendingFileRenameOperations";

#[derive(Error, Debug)]
pub enum RegistryError {
    #[error("Failed to register Firm: {0}")]
    FailedToRegister(IoError),

    #[error("Failed to deregister Firm: {0}")]
    FailedToDeregister(IoError),

    #[error("Failed to add Firm to PATH: {0}")]
    FailedToAddToPath(IoError),

    #[error("Failed to remove Firm from PATH: {0}")]
    FailedToRemoveFromPath(IoError),

    #[error("Failed to register uninstaller: {0}")]
    FailedToRegisterUninstaller(IoError),

    #[error("Failed to deregister uninstaller: {0}")]
    FailedToDeregisterUninstaller(IoError),

    #[error(r#"Failed to mark folder "{0}" for deletion: {1}"#)]
    FailedToMarkDirectoryForRebootDeletion(String, IoError),

    #[error("Failed to cancel file deletion: {0}")]
    FailedToCancelFileDeletion(String),

    #[error("Registry key error: {0}")]
    RegistryKeyError(IoError),

    #[error("Failed to register application \"{0}\": {1}")]
    FailedToRegisterApplication(String, IoError),

    #[error("Failed to deregister application \"{0}\": {1}")]
    FailedToDeregisterApplication(String, IoError),
}

pub trait RegistryKey {
    fn create_subkey(&self, path: &str) -> Result<Box<dyn RegistryKey>, IoError>;
    fn set_value(&self, name: &str, value: &str) -> Result<(), IoError>;
    fn open_subkey(&self, name: &str) -> Result<Box<dyn RegistryKey>, IoError>;
    fn delete_subkey_all(&self, name: &str) -> Result<(), IoError>;
    fn open_subkey_with_flags(
        &self,
        name: &str,
        flags: REGSAM,
    ) -> Result<Box<dyn RegistryKey>, IoError>;
    fn get_value(&self, name: &str) -> Result<String, IoError>;
    fn create_subkey_with_flags(
        &self,
        name: &str,
        flags: REGSAM,
    ) -> Result<Box<dyn RegistryKey>, IoError>;
    fn enum_values(&self) -> Result<HashMap<String, String>, IoError>;
}

impl RegistryKey for RegKey {
    fn create_subkey(&self, path: &str) -> Result<Box<dyn RegistryKey>, IoError> {
        self.create_subkey(path)
            .map(|(key, _)| Box::new(key) as Box<dyn RegistryKey>)
    }

    fn set_value(&self, name: &str, value: &str) -> Result<(), IoError> {
        self.set_value(name, &value)
    }

    fn open_subkey(&self, name: &str) -> Result<Box<dyn RegistryKey>, IoError> {
        self.open_subkey(name)
            .map(|key| Box::new(key) as Box<dyn RegistryKey>)
    }

    fn delete_subkey_all(&self, name: &str) -> Result<(), IoError> {
        self.delete_subkey_all(name)
    }

    fn open_subkey_with_flags(
        &self,
        name: &str,
        flags: REGSAM,
    ) -> Result<Box<dyn RegistryKey>, IoError> {
        self.open_subkey_with_flags(name, flags)
            .map(|key| Box::new(key) as Box<dyn RegistryKey>)
    }

    fn get_value(&self, name: &str) -> Result<String, IoError> {
        self.get_value(name)
    }

    fn create_subkey_with_flags(
        &self,
        name: &str,
        flags: REGSAM,
    ) -> Result<Box<dyn RegistryKey>, IoError> {
        self.create_subkey_with_flags(name, flags)
            .map(|(key, _)| Box::new(key) as Box<dyn RegistryKey>)
    }

    fn enum_values(&self) -> Result<HashMap<String, String>, IoError> {
        self.enum_values()
            .map(|rv| rv.and_then(|(n, v)| String::from_reg_value(&v).map(|v| (n, v))))
            .collect::<Result<HashMap<String, String>, IoError>>()
    }
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
            RegistryError::FailedToMarkDirectoryForRebootDeletion(_, _) => 16,
            RegistryError::FailedToCancelFileDeletion(_) => 17,
            RegistryError::RegistryKeyError(_) => 18,
            RegistryError::FailedToRegisterApplication(_, _) => 19,
            RegistryError::FailedToDeregisterApplication(_, _) => 20,
        }
    }
}

pub struct RegistryEditor<'a> {
    registry: Box<dyn RegistryKey + 'a>,
    get_folder_paths: Box<dyn Fn(&Path) -> Result<Vec<PathBuf>, IoError> + 'a>,
}

fn get_folder_paths(path: &Path) -> Result<Vec<PathBuf>, IoError> {
    std::fs::read_dir(path).and_then(|entries| {
        entries
            .map(|entry| {
                entry.and_then(|de| {
                    if de.path().is_dir() {
                        get_folder_paths(&de.path())
                    } else {
                        Ok(vec![de.path()])
                    }
                })
            })
            .collect::<Result<Vec<Vec<_>>, IoError>>()
            .map(|r| {
                r.into_iter()
                    .flatten()
                    .chain(vec![PathBuf::from(path)])
                    .collect()
            })
    })
}

impl<'a> RegistryEditor<'a> {
    pub fn new() -> Self {
        Self {
            registry: Box::new(RegKey::predef(HKEY_LOCAL_MACHINE)) as Box<dyn RegistryKey>,
            get_folder_paths: Box::new(get_folder_paths),
        }
    }

    #[cfg(test)]
    pub fn new_with_registry<T, F>(registry: T, lambda: F) -> Self
    where
        T: RegistryKey + 'a,
        F: Fn(&Path) -> Result<Vec<PathBuf>, IoError> + 'a,
    {
        Self {
            registry: Box::new(registry),
            get_folder_paths: Box::new(lambda),
        }
    }

    pub fn root(&self) -> &dyn RegistryKey {
        self.registry.as_ref()
    }

    pub fn register_application(
        &self,
        name: &str,
        exe_path: &Path,
        additional_data: HashMap<String, String>,
    ) -> Result<(), RegistryError> {
        self.registry
            .open_subkey("SOFTWARE")
            .and_then(|key| key.create_subkey(name))
            .and_then(|key| {
                key.set_value("InstallPath", &exe_path.to_string_lossy().into_owned())
                    .and_then(|_| {
                        additional_data
                            .iter()
                            .try_for_each(|(k, value)| key.set_value(k, value))
                    })
            })
            .map_err(|e| RegistryError::FailedToRegisterApplication(name.to_owned(), e))
    }

    pub fn deregister_application(&self, name: &str) -> Result<(), RegistryError> {
        self.registry
            .open_subkey_with_flags("SOFTWARE", KEY_WRITE)
            .and_then(|key| key.delete_subkey_all(name))
            .map_err(|e| RegistryError::FailedToDeregisterApplication(name.to_owned(), e))
    }

    pub fn find_application(&self, name: &str) -> Result<HashMap<String, String>, RegistryError> {
        self.registry
            .open_subkey("SOFTWARE")
            .and_then(|key| key.open_subkey(name))
            .and_then(|key| key.enum_values())
            .map_err(RegistryError::RegistryKeyError)
    }

    pub fn register_uninstaller(
        &self,
        name: &str,
        display_name: &str,
        uninstall_string: &str,
        extra_values: &HashMap<String, String>,
    ) -> Result<(), RegistryError> {
        self.registry
            .open_subkey(UNINSTALL_KEY)
            .and_then(|key| {
                key.create_subkey_with_flags(name, KEY_READ | KEY_WRITE)
                    .and_then(|uninstaller| {
                        uninstaller
                            .set_value("DisplayName", display_name)
                            .and_then(|_| {
                                uninstaller.set_value("UninstallString", uninstall_string)
                            })
                            .and_then(|_| {
                                extra_values
                                    .iter()
                                    .try_for_each(|(key, value)| uninstaller.set_value(key, value))
                            })
                    })
            })
            .map_err(RegistryError::FailedToRegisterUninstaller)
    }

    pub fn deregister_uninstaller(&self, name: &str) -> Result<(), RegistryError> {
        self.registry
            .open_subkey_with_flags(UNINSTALL_KEY, KEY_WRITE)
            .and_then(|key| key.delete_subkey_all(name))
            .map_err(RegistryError::FailedToDeregisterUninstaller)
    }

    pub fn add_to_path(&self, location: &Path) -> Result<(), RegistryError> {
        self.registry
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
            .map_err(RegistryError::FailedToAddToPath)
    }

    pub fn remove_from_path(&self, location: &Path) -> Result<(), RegistryError> {
        self.registry
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
            .map_err(RegistryError::FailedToRemoveFromPath)
    }

    pub fn cancel_pending_deletions(&self, path: &Path) -> Result<(), RegistryError> {
        Regex::new(&format!(
            r#"^\\\?\?\\{}.*"#,
            path.to_string_lossy().into_owned().escape_default()
        ))
        .map_err(|e| {
            RegistryError::FailedToCancelFileDeletion(format!(
                "Failed to create regex: {}",
                e.to_string()
            ))
        })
        .and_then(|regex| {
            self.get_pending_deletions().map(|paths| {
                paths
                    .into_iter()
                    .filter(|path| !regex.is_match(path))
                    .collect::<Vec<String>>()
            })
        })
        .and_then(|new_deletions| self.set_pending_deletions(&new_deletions))
    }

    fn get_pending_deletions(&self) -> Result<Vec<String>, RegistryError> {
        // There is an expected format to this string.
        // Path, two new lines, another path two new lines etc.

        // Keyboard cowboys are using the registry which means we need to make sure
        // that the format is expected since anyone can edit this string.

        // We've noticed that even when using the winapi to add paths the format
        // is ruined due to others presumably editing and inserting bad things.

        self.registry
            .open_subkey_with_flags(PENDING_REMOVAL_KEY, KEY_READ | KEY_WRITE)
            .map_err(|e| RegistryError::FailedToCancelFileDeletion(format!("{}", e)))
            .and_then(|key| {
                key.get_value(PENDING_OPERATIONS)
                    .map_err(|e| RegistryError::FailedToCancelFileDeletion(format!("{}", e)))
            })
            .map(|remove_entry| {
                remove_entry
                    .lines()
                    .filter_map(|entry| (!entry.is_empty()).then(|| entry.to_owned()))
                    .collect()
            })
    }

    fn set_pending_deletions(&self, pending_deletions: &[String]) -> Result<(), RegistryError> {
        self.registry
            .open_subkey_with_flags(PENDING_REMOVAL_KEY, KEY_READ | KEY_WRITE)
            .map_err(|e| RegistryError::FailedToCancelFileDeletion(format!("{}", e)))
            .and_then(|key| {
                key.set_value(
                    PENDING_OPERATIONS,
                    &pending_deletions.iter().fold(String::new(), |acc, entry| {
                        format!("{}{}\n\n", acc, entry.to_owned())
                    }),
                )
                .map_err(|e| RegistryError::FailedToCancelFileDeletion(format!("{}", e)))
            })
    }

    pub fn mark_for_delete(&self, path: &Path) -> Result<(), RegistryError> {
        (self.get_folder_paths)(path)
            .map_err(|e| {
                RegistryError::FailedToMarkDirectoryForRebootDeletion(
                    format!(
                        "Failed to get files and folders for \"{}\"",
                        path.to_string_lossy().into_owned()
                    ),
                    e,
                )
            })
            .and_then(|paths| self.mark_paths_for_delete(&paths))
    }

    fn mark_paths_for_delete(&self, paths: &[PathBuf]) -> Result<(), RegistryError> {
        // TODO: Add transaction layer around this.
        self.get_pending_deletions()
            .and_then(|mut deletion_content| {
                deletion_content.extend(
                    paths
                        .iter()
                        .map(|p| format!(r#"\??\{}"#, p.to_string_lossy().into_owned())),
                );
                self.set_pending_deletions(deletion_content.as_slice())
            })
    }
}

impl<'a> Default for RegistryEditor<'a> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
pub mod test {
    use super::*;
    use std::{
        collections::HashMap,
        path::PathBuf,
        sync::{Arc, RwLock},
    };

    #[macro_export]
    macro_rules! populate_fake_registry {
        () => {{
            let registry_keys = ::std::sync::Arc::new(::std::sync::RwLock::new(HashMap::new()));
            let base = r#"Computer\LOCAL_MACHINE"#;
            crate::registry::test::MemoryKey::from(&registry_keys, base)
        }};

        ([$($key:expr),*]) => {{
            let registry_keys = ::std::sync::Arc::new(::std::sync::RwLock::new(HashMap::new()));
            let root = populate_fake_registry!(registry_keys, [$($key),*]);
            (registry_keys, root)
        }};

        ($registry_keys: expr, [$($key:expr),*]) => {{
            let base = r#"Computer\LOCAL_MACHINE"#;
            let mut registry = $registry_keys.write().unwrap();
            vec![$(($key),)*].into_iter().for_each(|k| {
                    k.split(r#"\"#).fold(String::from(base), |old_s, s| {
                        let new_path = format!(r#"{}\{}"#, old_s, s);
                        registry.insert(new_path.clone(), crate::registry::test::MemoryEntry::new());
                        new_path
                    });
                registry.insert(
                format!(r#"{}\{}"#, base, k),
                crate::registry::test::MemoryEntry::new(),
            );});
            crate::registry::test::MemoryKey::from(&$registry_keys, base)
        }};

        ($registry_keys: expr, {$($path:expr => {$($key:expr => $value:expr),*}),*}) => {{
            let base = r#"Computer\LOCAL_MACHINE"#;
            let mut registry = $registry_keys.write().unwrap();
            $(
                let mut values = ::std::collections::HashMap::new();
                $path.split(r#"\"#).fold(String::from(base), |old_s, s| {
                    let new_path = format!(r#"{}\{}"#, old_s, s);
                    registry.insert(new_path.clone(), crate::registry::test::MemoryEntry::new());
                    new_path
                });
                $(
                    values.insert(String::from($key), String::from($value));
                )*
                registry.insert(
                    format!(r#"{}\{}"#, base, $path),
                    crate::registry::test::MemoryEntry::from(values),
                );
            )*

            crate::registry::test::MemoryKey::from(&$registry_keys, base)
        }};
    }

    pub struct MemoryKey {
        registry_keys: Arc<RwLock<HashMap<String, MemoryEntry>>>,
        path: String,
    }

    impl MemoryKey {
        pub fn from(registry_keys: &Arc<RwLock<HashMap<String, MemoryEntry>>>, path: &str) -> Self {
            Self {
                registry_keys: Arc::clone(registry_keys),
                path: path.to_owned(),
            }
        }
    }

    #[derive(Clone, Debug)]
    pub struct MemoryEntry {
        values: HashMap<String, String>,
    }

    impl MemoryEntry {
        pub fn new() -> Self {
            Self {
                values: HashMap::new(),
            }
        }

        pub fn from(values: HashMap<String, String>) -> Self {
            Self { values }
        }
    }

    impl RegistryKey for MemoryKey {
        fn create_subkey(&self, path: &str) -> Result<Box<dyn RegistryKey>, IoError> {
            self.registry_keys
                .write()
                .unwrap()
                .entry(format!(r#"{}\{}"#, self.path, path))
                .or_insert_with(MemoryEntry::new);
            Ok(Box::new(MemoryKey {
                registry_keys: Arc::clone(&self.registry_keys),
                path: format!(r#"{}\{}"#, self.path, path),
            }) as Box<dyn RegistryKey>)
        }

        fn set_value(&self, name: &str, value: &str) -> Result<(), IoError> {
            self.registry_keys
                .write()
                .unwrap()
                .entry(self.path.clone())
                .or_insert_with(MemoryEntry::new)
                .values
                .insert(name.to_string(), value.to_string());
            Ok(())
        }

        fn open_subkey(&self, name: &str) -> Result<Box<dyn RegistryKey>, IoError> {
            let full_path = format!(r#"{}\{}"#, self.path, name);
            self.registry_keys
                .read()
                .unwrap()
                .get(&full_path)
                .map(|_| {
                    Box::new(MemoryKey {
                        path: full_path,
                        registry_keys: Arc::clone(&self.registry_keys),
                    }) as Box<dyn RegistryKey>
                })
                .ok_or_else(|| IoError::from_raw_os_error(2))
        }

        fn delete_subkey_all(&self, name: &str) -> Result<(), IoError> {
            self.registry_keys
                .write()
                .unwrap()
                .remove(&(format!(r#"{}\{}"#, self.path, name)))
                .map(|_| ())
                .ok_or_else(|| IoError::from_raw_os_error(2))
        }

        fn open_subkey_with_flags(
            &self,
            name: &str,
            _flags: REGSAM,
        ) -> Result<Box<dyn RegistryKey>, IoError> {
            self.open_subkey(name)
        }

        fn get_value(&self, name: &str) -> Result<String, IoError> {
            self.registry_keys
                .read()
                .unwrap()
                .get(&self.path)
                .and_then(|entry| entry.values.get(name))
                .map(|value| value.to_string())
                .ok_or_else(|| IoError::from_raw_os_error(2))
        }

        fn create_subkey_with_flags(
            &self,
            name: &str,
            _flags: REGSAM,
        ) -> Result<Box<dyn RegistryKey>, IoError> {
            self.create_subkey(name)
        }

        fn enum_values(&self) -> Result<HashMap<String, String>, IoError> {
            self.registry_keys
                .read()
                .unwrap()
                .get(&self.path)
                .ok_or_else(|| IoError::from_raw_os_error(2))
                .map(|entry| entry.values.clone())
        }
    }

    fn get_test_folder_paths(_path: &Path) -> Result<Vec<PathBuf>, IoError> {
        Ok(vec![
            PathBuf::from(r#"B:\hus\matbord.mp3"#),
            PathBuf::from(r#"B:\hus\bullar.wav"#),
            PathBuf::from(r#"B:\hus\garage\verktyg.zip"#),
            PathBuf::from(r#"B:\hus\garage\bilar.txt"#),
            PathBuf::from(r#"B:\hus\garage"#),
            PathBuf::from(r#"B:\hus"#),
        ])
    }

    #[test]
    fn get_deletions_formatting() {
        let pending_file_deletions = r#"bune


rune

kune
lune"#;
        let registry_keys = Arc::new(RwLock::new(HashMap::new()));
        let root = populate_fake_registry!(
            registry_keys,
            {
                PENDING_REMOVAL_KEY.to_string() => {PENDING_OPERATIONS.to_string() => pending_file_deletions}
            }
        );
        let editor = RegistryEditor::new_with_registry(root, get_test_folder_paths);
        let res = editor.get_pending_deletions().unwrap();
        assert_eq!(res, vec!["bune", "rune", "kune", "lune"]);

        let pending_file_deletions = r#"

bune


rune






kune
lune





"#;

        editor
            .root()
            .open_subkey(PENDING_REMOVAL_KEY)
            .unwrap()
            .set_value(PENDING_OPERATIONS, pending_file_deletions)
            .unwrap();

        let res = editor.get_pending_deletions().unwrap();

        assert_eq!(res, vec!["bune", "rune", "kune", "lune"]);
    }

    #[test]
    fn mark_deletion() {
        // Some keyboard cowboy edited this value in the registry
        let pending_file_deletions = r#"B:\zoo\apa
B:\zoo\apa\bananer.txt

B:\garage\fin_bil

"#;

        let registry_keys = Arc::new(RwLock::new(HashMap::new()));
        let root = populate_fake_registry!(
            registry_keys,
            {
                PENDING_REMOVAL_KEY.to_string() => {PENDING_OPERATIONS.to_string() => pending_file_deletions}
            }
        );

        let editor = RegistryEditor::new_with_registry(root, |_| {
            Ok(vec![
                PathBuf::from(r#"sune:\runes_hus\inkopslista.txt"#),
                PathBuf::from(r#"sune:\runes_hus\recept.txt"#),
                PathBuf::from(r#"sune:\runes_hus\garage\verktyg.zip"#),
                PathBuf::from(r#"sune:\runes_hus\garage"#),
                PathBuf::from(r#"sune:\runes_hus"#),
            ])
        });
        let res = editor.mark_for_delete(&PathBuf::from(r#"sune:\runes_hus"#));
        assert!(res.is_ok());

        let pending_deletion_value = editor
            .root()
            .open_subkey(PENDING_REMOVAL_KEY)
            .unwrap()
            .get_value(PENDING_OPERATIONS)
            .unwrap();

        assert_eq!(
            pending_deletion_value,
            r#"B:\zoo\apa

B:\zoo\apa\bananer.txt

B:\garage\fin_bil

\??\sune:\runes_hus\inkopslista.txt

\??\sune:\runes_hus\recept.txt

\??\sune:\runes_hus\garage\verktyg.zip

\??\sune:\runes_hus\garage

\??\sune:\runes_hus

"#
        );
    }

    #[test]
    fn cancel_deletions() {
        let pending_file_deletions = r#"\??\D:\bune

\??\B:\birm\bomax.exe
\??\B:\birm\birm.exe

\??\B:\rune
\??\B:\birm\binstaller.exe


\??\B:\kune

\??\A:\lune

\??\B:\birm\dlls\dynamically_linked_zip.zip
\??\B:\birm\dlls
\??\B:\birm

"#;

        let registry_keys = Arc::new(RwLock::new(HashMap::new()));
        let root = populate_fake_registry!(
            registry_keys,
            {
                PENDING_REMOVAL_KEY.to_string() => {PENDING_OPERATIONS.to_string() => pending_file_deletions}
            }
        );
        let editor = RegistryEditor::new_with_registry(root, get_test_folder_paths);

        editor
            .root()
            .open_subkey(PENDING_REMOVAL_KEY)
            .unwrap()
            .set_value(PENDING_OPERATIONS, pending_file_deletions)
            .unwrap();

        let res = editor.cancel_pending_deletions(&PathBuf::from(r#"B:\birm"#));
        assert!(res.is_ok());

        let res = editor.get_pending_deletions().unwrap();

        assert_eq!(
            res,
            vec![
                r#"\??\D:\bune"#,
                r#"\??\B:\rune"#,
                r#"\??\B:\kune"#,
                r#"\??\A:\lune"#
            ]
        );
    }

    #[test]
    fn uninstaller_registry() {
        let (_registry_keys, root) = populate_fake_registry!([UNINSTALL_KEY.to_string()]);

        let editor = RegistryEditor::new_with_registry(root, get_test_folder_paths);
        let mut extra = HashMap::new();
        extra.insert(
            String::from("URLInfoAbout"),
            String::from("https://webpage.a"),
        );
        extra.insert(String::from("DisplayVersion"), String::from("Version 2"));
        extra.insert(
            String::from("InstallLocation"),
            String::from(r#"E:\aaaaa\bbbbb\c"#),
        );
        let res = editor.register_uninstaller(
            "Girm",
            "Hirm",
            r#"E:\aaaaa\bbbbb\c --deinstallify"#,
            &extra,
        );
        assert!(res.is_ok(), "Uninstaller should be registrable");
        assert!(
            editor.root().open_subkey(&UNINSTALL_KEY).is_ok(),
            "We expect there to be something at UNINSTALL_KEY"
        );

        let uninstall_location = format!(r#"{}\{}"#, UNINSTALL_KEY, "Girm");
        let uninstall_entry = editor.root().open_subkey(&uninstall_location);
        assert!(uninstall_entry.is_ok());

        let uninstall_entry = uninstall_entry.unwrap();
        assert_eq!(
            uninstall_entry.get_value("UninstallString").unwrap(),
            String::from(r#"E:\aaaaa\bbbbb\c --deinstallify"#)
        );
        assert_eq!(
            uninstall_entry.get_value("DisplayVersion").unwrap(),
            String::from("Version 2")
        );
        assert_eq!(
            uninstall_entry.get_value("DisplayName").unwrap(),
            String::from(r#"Hirm"#)
        );
        assert_eq!(
            uninstall_entry.get_value("URLInfoAbout").unwrap(),
            String::from(r#"https://webpage.a"#)
        );
        assert_eq!(
            uninstall_entry.get_value("InstallLocation").unwrap(),
            String::from(r#"E:\aaaaa\bbbbb\c"#)
        );

        editor
            .register_uninstaller("Pirm", "Nirm", r#"E:\aaaaa\bbbbb\c --deinstallify"#, &extra)
            .unwrap();
        let res = editor.deregister_uninstaller("Girm");
        assert!(
            res.is_ok(),
            "We want to be able to deregister the uninstaller"
        );
        let entry = editor.root().open_subkey(&uninstall_location);
        let entry2 = editor
            .root()
            .open_subkey(&format!(r#"{}\{}"#, UNINSTALL_KEY, "Pirm"));
        assert!(
            entry.is_err(),
            "There should be nothing in the uninstall path for Girm"
        );
        assert!(entry2.is_ok(), "Pirm entry should still be in here");
    }

    #[test]
    fn add_remove_from_path() {
        let registry_keys = Arc::new(RwLock::new(HashMap::new()));
        let orig_path = format!(
            "{};{}",
            PathBuf::from("this")
                .join("is")
                .join("a")
                .join("path")
                .to_string_lossy(),
            PathBuf::from("some")
                .join("other")
                .join("random")
                .join("thing")
                .to_string_lossy()
        );
        let root = populate_fake_registry!(registry_keys, {ENVIRONMENT_KEY.to_string() => {"Path" => orig_path.clone()}});
        let editor = RegistryEditor::new_with_registry(root, get_test_folder_paths);
        let path = PathBuf::from("yellow").join("brick").join("toad");
        let res = editor.add_to_path(&path);
        assert!(res.is_ok(), "Adding a path to PATH should work");

        let keep_path = PathBuf::from("bellow").join("duck").join("foad");
        editor.add_to_path(&keep_path).unwrap();

        assert!(editor
            .root()
            .open_subkey(ENVIRONMENT_KEY)
            .unwrap()
            .get_value("Path")
            .unwrap()
            .contains(&path.to_string_lossy().to_string()));
        assert!(editor
            .root()
            .open_subkey(ENVIRONMENT_KEY)
            .unwrap()
            .get_value("Path")
            .unwrap()
            .contains(&orig_path));
        let res = editor.remove_from_path(&path);
        assert!(res.is_ok(), "We should be able to remove from path");
        let path_value = editor
            .root()
            .open_subkey(ENVIRONMENT_KEY)
            .unwrap()
            .get_value("Path")
            .unwrap();
        assert!(
            !path_value.contains(&path.to_string_lossy().into_owned()),
            "The path we removed should be.... removed"
        );
        assert!(path_value.contains(&keep_path.to_string_lossy().into_owned()))
    }

    #[test]
    fn cancel_pending_deletions() {
        // Minimal case
        let pending_deletions = r#"\??\C:\Program Files\Firm\avery.exe

\??\C:\Program Files\Firm\lomax.exe

\??\C:\Program Files\Firm\bendini.exe

\??\C:\Program Files\Firm\install.exe

\??\C:\Program Files\Firm

"#;
        let registry_keys = Arc::new(RwLock::new(HashMap::new()));
        let root = populate_fake_registry!(registry_keys, {PENDING_REMOVAL_KEY.to_string() => {PENDING_OPERATIONS => pending_deletions}});
        let editor = RegistryEditor::new_with_registry(root, get_test_folder_paths);
        let res = editor
            .cancel_pending_deletions(&PathBuf::from(r#"C:\"#).join("Program Files").join("Firm"));
        assert!(
            res.is_ok(),
            "Cancel pending deletions from the path should just work"
        );
        let remaining_operations = editor
            .root()
            .open_subkey(PENDING_REMOVAL_KEY)
            .unwrap()
            .get_value(PENDING_OPERATIONS)
            .unwrap();
        assert_eq!(
            remaining_operations, "",
            "When only our operations were in there the result should be empty"
        );

        //When other things needs to burn too
        let pending_deletions = r#"\??\C:\Users\mega-rune\AppData\Local\Temp\~nsuA.tmp\Un_A.exe

\??\C:\Users\mega-rune\AppData\Local\Temp\~nsuA.tmp

\??\C:\Users\mega-rune\AppData\Local\Temp\~nsuA.tmp\Un_B.exe

\??\C:\Users\mega-rune\AppData\Local\Temp\~nsuA.tmp

\??\C:\Users\mega-rune\AppData\Local\Temp\nslD0EA.tmp\nsProcess.dll

\??\C:\Users\mega-rune\AppData\Local\Temp\nslD0EA.tmp\

\??\C:\Users\mega-rune\AppData\Local\Temp\nsb8D72.tmp\nsProcess.dll

\??\C:\Users\mega-rune\AppData\Local\Temp\nsb8D72.tmp\

\??\C:\Program Files\Firm\avery.exe

"#;
        let expected_remaining = r#"\??\C:\Users\mega-rune\AppData\Local\Temp\~nsuA.tmp\Un_A.exe

\??\C:\Users\mega-rune\AppData\Local\Temp\~nsuA.tmp

\??\C:\Users\mega-rune\AppData\Local\Temp\~nsuA.tmp\Un_B.exe

\??\C:\Users\mega-rune\AppData\Local\Temp\~nsuA.tmp

\??\C:\Users\mega-rune\AppData\Local\Temp\nslD0EA.tmp\nsProcess.dll

\??\C:\Users\mega-rune\AppData\Local\Temp\nslD0EA.tmp\

\??\C:\Users\mega-rune\AppData\Local\Temp\nsb8D72.tmp\nsProcess.dll

\??\C:\Users\mega-rune\AppData\Local\Temp\nsb8D72.tmp\

"#;
        let root = populate_fake_registry!(registry_keys, {PENDING_REMOVAL_KEY.to_string() => {PENDING_OPERATIONS => pending_deletions}});
        let editor = RegistryEditor::new_with_registry(root, get_test_folder_paths);
        let res = editor
            .cancel_pending_deletions(&PathBuf::from(r#"C:\"#).join("Program Files").join("Firm"));
        assert!(
            res.is_ok(),
            "Cancel pending deletions from the path should just work"
        );
        let remaining_operations = editor
            .root()
            .open_subkey(PENDING_REMOVAL_KEY)
            .unwrap()
            .get_value(PENDING_OPERATIONS)
            .unwrap();
        assert_eq!(remaining_operations, expected_remaining);

        // With all things and duplicates and things in between
        let pending_deletions = r#"\??\B:\Brogram Biles\Birm\avery.exe

\??\C:\Users\mega-rune\AppData\Local\Temp\~nsuA.tmp\Un_A.exe

\??\C:\Users\mega-rune\AppData\Local\Temp\~nsuA.tmp

\??\C:\Users\mega-rune\AppData\Local\Temp\~nsuA.tmp\Un_B.exe

\??\C:\Users\mega-rune\AppData\Local\Temp\~nsuA.tmp

\??\C:\Users\mega-rune\AppData\Local\Temp\nslD0EA.tmp\nsProcess.dll

\??\B:\Brogram Biles\Birm

\??\C:\Users\mega-rune\AppData\Local\Temp\nslD0EA.tmp\

\??\C:\Users\mega-rune\AppData\Local\Temp\nsb8D72.tmp\nsProcess.dll

\??\B:\Brogram Biles\Birm\bendini.exe

\??\C:\Users\mega-rune\AppData\Local\Temp\nsb8D72.tmp\

\??\B:\Brogram Biles\Birm\lomax.exe

\??\B:\Brogram Biles\Birm\

\??\B:\Brogram Biles\Birm

"#;
        let root = populate_fake_registry!(registry_keys, {PENDING_REMOVAL_KEY.to_string() => {PENDING_OPERATIONS => pending_deletions}});
        let editor = RegistryEditor::new_with_registry(root, get_test_folder_paths);
        let res = editor
            .cancel_pending_deletions(&PathBuf::from(r#"B:\"#).join("Brogram Biles").join("Birm"));
        assert!(
            res.is_ok(),
            "Cancel pending deletions from another path should just work"
        );
        let remaining_operations = editor
            .root()
            .open_subkey(PENDING_REMOVAL_KEY)
            .unwrap()
            .get_value(PENDING_OPERATIONS)
            .unwrap();
        assert_eq!(remaining_operations, expected_remaining);

        // And once more with no pending operations
        let root = populate_fake_registry!(registry_keys, {PENDING_REMOVAL_KEY.to_string() => {PENDING_OPERATIONS => ""}});
        let editor = RegistryEditor::new_with_registry(root, get_test_folder_paths);
        let res = editor
            .cancel_pending_deletions(&PathBuf::from(r#"C:\"#).join("Program Files").join("Firm"));
        assert!(res.is_ok(), "Cancel no pending deletions should just work");
        let remaining_operations = editor
            .root()
            .open_subkey(PENDING_REMOVAL_KEY)
            .unwrap()
            .get_value(PENDING_OPERATIONS)
            .unwrap();
        assert_eq!(remaining_operations, "");
    }

    #[test]
    fn application_registration() {
        let (_, root) = populate_fake_registry!(["SOFTWARE"]);
        let editor = RegistryEditor::new_with_registry(root, get_test_folder_paths);
        let mut moar = HashMap::new();
        moar.insert(
            String::from("HeadMistressName"),
            String::from("Miss Eulalie Butts"),
        );
        let res = editor.register_application("Qirm", &PathBuf::from("vintergatan"), moar);
        assert!(res.is_ok(), "We should be able to add a new app");
        assert_eq!(
            editor
                .root()
                .open_subkey(r#"SOFTWARE\Qirm"#)
                .unwrap()
                .get_value("HeadMistressName")
                .unwrap(),
            "Miss Eulalie Butts"
        );
        let res = editor.find_application("Qirm");
        assert!(res.is_ok(), "We should be able to find our new application");
        let res = res.unwrap();
        assert_eq!(res.get("InstallPath").unwrap(), "vintergatan");
        assert_eq!(res.get("HeadMistressName").unwrap(), "Miss Eulalie Butts");
        let res = editor.deregister_application("Qirm");
        assert!(
            res.is_ok(),
            "We should be able to deregister the application we just registered"
        );
        assert!(
            editor.find_application("Qirm").is_err(),
            "Now we should NOT be able to find it!"
        );
    }
}
