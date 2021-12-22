use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
};

use flate2::bufread::GzDecoder;
use slog::{debug, error, info, o, Drain, Logger};
use slog_term::{FullFormat, TermDecorator};
use structopt::StructOpt;
use tar::{Archive, Entry, Unpacked};
use thiserror::Error;

use windows_install::{
    registry::{RegistryEditor, RegistryError},
    service::ServiceError,
    service_manager::ServiceManager,
    winreg::{enums::HKEY_LOCAL_MACHINE, RegKey},
};

const AVERY: &str = "Avery";
const LOMAX: &str = "Lomax";
const APPLICATION_NAME: &str = "Firm";

#[derive(Error, Debug)]
pub enum InstallerError {
    #[error(r#"Failed to copy file "{0}": {1}"#)]
    FailedToCopyFile(String, io::Error),

    #[error(r#"Failed to remove files from "{0}": {1}"#)]
    FailedToRemoveFiles(PathBuf, io::Error),

    #[error("Failed to find this executable path: {0}")]
    FailedToFindCurrentExe(io::Error),

    #[error("Archive error: {0}")]
    ArchiveError(String),

    #[error(transparent)]
    ServiceError(#[from] ServiceError),

    #[error(transparent)]
    RegistryError(#[from] RegistryError),
}

impl From<InstallerError> for u32 {
    fn from(installer_error: InstallerError) -> Self {
        match installer_error {
            InstallerError::FailedToCopyFile(_, _) => 1,
            InstallerError::FailedToRemoveFiles(_, _) => 2,
            InstallerError::FailedToFindCurrentExe(_) => 3,
            InstallerError::ArchiveError(_) => 4,
            InstallerError::ServiceError(e) => e.into(),
            InstallerError::RegistryError(e) => e.into(),
        }
    }
}

#[derive(StructOpt, Debug)]
enum InstallOperation {
    /// Installer for Firm, the functional pipeline
    /// consisting of the services Avery and Lomax and
    /// the command line interface Bendini.
    Install {
        #[structopt(long, short = "p")]
        install_path: Option<PathBuf>,

        #[structopt(long, short)]
        data_path: Option<PathBuf>,
    },
    Uninstall,
    Upgrade,
}

#[derive(StructOpt, Debug)]
#[structopt(name = "Firm installer")]
struct InstallerArguments {
    #[structopt(subcommand)]
    operation: InstallOperation,

    #[structopt(long, short)]
    verbose: bool,
}

macro_rules! pass_result {
    ($logger:expr, $code:expr) => {{
        if let Err(e) = $code {
            debug!($logger, "{}", e)
        }
    }};

    ($logger:expr, $code:expr, $error_message:expr) => {{
        if let Err(e) = $code {
            debug!($logger, "{}: {}", $error_message, e)
        }
    }};
}

const DEFAULT_FIRM_BIN_PATH: &str = r#"C:\Program Files\Firm"#;
const DEFAULT_FIRM_DATA_PATH: &str = r#"C:\ProgramData\Firm"#;

pub fn find_firm<F: Fn() -> PathBuf, G: Fn() -> PathBuf>(
    reg_edit: &RegistryEditor,
    logger: &Logger,
    default_program_files: F,
    default_program_data: G,
) -> (PathBuf, PathBuf) {
    reg_edit.find_application(APPLICATION_NAME).map(|entries| {
        (
            entries.get("InstallPath")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    debug!(logger, "Failed to find install path for Firm in registry. Getting default.");
                    default_program_files()
                }),
            entries.get("DataPath")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    debug!(logger, "Failed to find data path for Firm in registry. Getting default.");
                    default_program_data()
                }),
        )
    }).unwrap_or_else(|e| {
        debug!(
            logger,
            "Failed to find firm application. Falling back to use default data and install paths: {}", e);
        (
            default_program_files(),
            default_program_data()
        )
    })
}

fn default_path_from_env(logger: &Logger, key: &str, default: &str) -> PathBuf {
    std::env::var_os(key)
        .map(|appdata| PathBuf::from(&appdata).join(APPLICATION_NAME))
        .unwrap_or_else(|| {
            debug!(
                logger,
                r#"Could not find "{}" in environment, fallback to "{}""#, key, default
            );
            PathBuf::from(default)
        })
}

fn unpack_entry<E>(mut entry: Entry<E>, install_path: &Path) -> Result<PathBuf, InstallerError>
where
    E: io::Read,
{
    std::fs::create_dir_all(&install_path)
        .and_then(|_| {
            entry.path().map(|p| p.to_path_buf()).and_then(|path| {
                path.file_name()
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            format!(
                                r#"File "{}" is missing name"#,
                                entry.path().unwrap_or_default().display()
                            ),
                        )
                    })
                    .and_then(|file_name| {
                        let file_path = install_path.join(file_name);
                        entry
                            .unpack(file_path.clone())
                            .and_then(|unpack| match unpack {
                                Unpacked::File(_) => Ok(file_path),
                                _ => Err(io::Error::new(
                                    io::ErrorKind::Other,
                                    format!(
                                        r#"Entry "{}" is not a file."#,
                                        entry.path().unwrap_or_default().display()
                                    ),
                                )),
                            })
                    })
            })
        })
        .map_err(|e| {
            InstallerError::FailedToCopyFile(
                entry
                    .path()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
                e,
            )
        })
}

fn unpack_data_entry<E>(mut entry: Entry<E>, data_path: &Path) -> Result<PathBuf, InstallerError>
where
    E: io::Read,
{
    std::fs::create_dir_all(&data_path)
        .and_then(|_| {
            entry
                .unpack_in(data_path)
                .and_then(|_| entry.path().map(|p| data_path.join(p.to_path_buf())))
        })
        .map_err(|e| {
            InstallerError::FailedToCopyFile(
                entry
                    .path()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
                e,
            )
        })
}

fn copy_files(
    logger: &Logger,
    install_path: &Path,
    data_path: &Path,
) -> Vec<Result<PathBuf, InstallerError>> {
    let archive = include_bytes!("../install-data");
    debug!(logger, "🗜️ Unpacking archive...");

    match Archive::new(GzDecoder::new(&archive[..])).entries() {
        Err(e) => {
            vec![Err(InstallerError::ArchiveError(e.to_string()))]
        }
        Ok(entries) => entries
            .filter_map(|entry_res| match entry_res {
                Ok(entry) => match entry.header().entry_type() {
                    tar::EntryType::Directory => None,
                    tar::EntryType::Regular => Some(
                        if entry
                            .path()
                            .map(|p| p.starts_with(Path::new(".").join("bin")))
                            .unwrap_or_default()
                        {
                            unpack_entry(entry, install_path)
                        } else {
                            unpack_data_entry(entry, data_path)
                        },
                    ),
                    _ => Some(Err(InstallerError::ArchiveError(format!(
                        r#"Entry "{}" is of unsupported type "{:#?}" "#,
                        entry.path().unwrap_or_default().display(),
                        entry.header().entry_type()
                    )))),
                },
                Err(e) => Some(Err(InstallerError::ArchiveError(e.to_string()))),
            })
            .chain(std::iter::once(
                std::env::current_exe()
                    .map_err(InstallerError::FailedToFindCurrentExe)
                    .and_then(|installer_source| {
                        let installer_destination = install_path.join("install.exe");
                        fs::copy(installer_source, &installer_destination)
                            .map_err(|e| {
                                InstallerError::FailedToCopyFile(String::from("install.exe"), e)
                            })
                            .map(|_| installer_destination)
                    }),
            ))
            .collect::<Vec<Result<PathBuf, InstallerError>>>(),
    }
}

fn get_config_arg(path: &Path, name: &str) -> String {
    let config_path = path.join(name);
    config_path
        .exists()
        .then(|| format!(r#"--config "{}""#, config_path.to_string_lossy()))
        .unwrap_or_default()
}

fn get_service_manager() -> Result<ServiceManager, InstallerError> {
    ServiceManager::try_new().map_err(Into::into)
}

fn upgrade(logger: Logger) -> Result<(), InstallerError> {
    info!(logger, "☝️ Upgrading...");
    let reg_edit = RegistryEditor::new();
    let (exe_path, data_path) = find_firm(
        &reg_edit,
        &logger,
        || default_path_from_env(&logger, "PROGRAMFILES", DEFAULT_FIRM_BIN_PATH),
        || default_path_from_env(&logger, "PROGRAMDATA", DEFAULT_FIRM_DATA_PATH),
    );

    uninstall(logger.new(o!("scope" => "uninstall")));

    install(logger.new(o!("scope" => "install")), &exe_path, &data_path)
        .and_then(|_| get_service_manager())
        .and_then(|service_manager| {
            let service_filter = format!("{}_", AVERY);
            debug!(logger, "Starting services: \"{}\"", service_filter);
            service_manager
                .start_services(&service_filter)
                .map_err(Into::into)
        })
}

const REG_BASEKEY: &str = "SYSTEM\\CurrentControlSet\\Services\\EventLog\\Application";
fn try_register_log_source(name: &str, exe_path: &str) -> Result<(), InstallerError> {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(REG_BASEKEY)
        .and_then(|cur_ver| cur_ver.create_subkey(name))
        .and_then(|(app_key, _)| app_key.set_value("EventMessageFile", &exe_path))
        .map_err(|e| RegistryError::FailedToRegisterApplication(name.to_owned(), e).into())
}

fn try_deregister_log_source(name: &str) -> Result<(), InstallerError> {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(REG_BASEKEY)
        .and_then(|cur_ver| cur_ver.delete_subkey(name))
        .map_err(|e| RegistryError::FailedToDeregisterApplication(name.to_owned(), e).into())
}

fn install(logger: Logger, install_path: &Path, data_path: &Path) -> Result<(), InstallerError> {
    debug!(
        logger,
        r#"Using executable dir: "{}" and data dir: "{}""#,
        install_path.to_string_lossy(),
        data_path.to_string_lossy()
    );
    info!(logger, "💾 Installing...");
    let reg_edit = RegistryEditor::new();
    pass_result!(logger, reg_edit.cancel_pending_deletions(install_path));

    copy_files(&logger, install_path, data_path)
        .into_iter()
        .map(|file_result| match file_result {
            Ok(file) => reg_edit
                .register_install_file(APPLICATION_NAME, &file)
                .map_err(|e| e.into()),
            Err(e) => {
                error!(logger, "Failed to copy file: {}", e);
                Err(e)
            }
        })
        // We know this looks weird (two collects). We have the problem
        // where we need to go through ALL values to ensure
        // that all files copied gets pushed to the key in the registry.
        // If we just did a single collect it would stop at the first error
        // and possibly skip files that we copied.
        .collect::<Vec<Result<(), InstallerError>>>()
        .into_iter()
        .collect::<Result<(), InstallerError>>()
        .and_then(|_| {
            try_register_log_source(AVERY, &install_path.join("avery.exe").to_string_lossy())
                .map_err(Into::into)
        })
        .and_then(|_| {
            try_register_log_source(LOMAX, &install_path.join("lomax.exe").to_string_lossy())
                .map_err(Into::into)
        })
        .and_then(|_| get_service_manager())
        .and_then(|service_manager| {
            debug!(logger, "🏃‍♀️ Starting services.");
            service_manager
                .create_user_service(
                    AVERY,
                    &install_path.join("avery.exe").to_string_lossy(),
                    &[
                        "--service",
                        get_config_arg(data_path, "avery.toml").as_str(),
                    ],
                )
                .and_then(|_| {
                    service_manager.create_system_service(
                        LOMAX,
                        &install_path.join("lomax.exe").to_string_lossy(),
                        &[
                            "--service",
                            get_config_arg(data_path, "lomax.toml").as_str(),
                        ],
                    )
                })
                .and_then(|lomax_service| lomax_service.start())
                .map_err(Into::into)
        })
        .and_then(|_| reg_edit.add_to_path(install_path).map_err(Into::into))
        .and_then(|_| {
            let mut additional_data = HashMap::new();
            additional_data.insert(
                String::from("DataPath"),
                data_path.to_string_lossy().into_owned(),
            );
            additional_data.insert(String::from("Version"), String::from(std::env!("version")));
            reg_edit
                .register_application(APPLICATION_NAME, install_path, additional_data)
                .map_err(Into::into)
        })
        .and_then(|_| {
            let mut extra_info = HashMap::new();
            extra_info.insert(
                String::from("InstallLocation"),
                install_path.to_string_lossy().into_owned(),
            );
            extra_info.insert(
                String::from("DisplayVersion"),
                std::env!("version").to_owned(),
            );
            extra_info.insert(
                String::from("URLInfoAbout"),
                String::from("https://github.com/goodbyekansas/firm"),
            );
            reg_edit
                .register_uninstaller(
                    APPLICATION_NAME,
                    APPLICATION_NAME,
                    format!(
                        r#"{}\install.exe uninstall"#,
                        &install_path.to_string_lossy()
                    )
                    .as_str(),
                    &extra_info,
                )
                .map_err(Into::into)
        })
        .map_err(|e| {
            error!(logger, "🧹 Install failed, cleaning up...");
            uninstall(logger.new(o!("scope" => "cleanup")));
            e
        })
}

fn uninstall(logger: Logger) {
    // uninstall does a best effort and removes as much as possible
    info!(logger, "🪓 Uninstalling...");

    pass_result!(
        logger,
        get_service_manager().map(|service_manager| {
            let service_filter = format!("{}_", AVERY);
            debug!(logger, "Stopping user services \"{}\"", service_filter);
            pass_result!(
                logger,
                service_manager.stop_services(&service_filter),
                "😭 Failed to stop user services"
            );

            debug!(logger, "Stopping system services.");
            if let Err(error) = service_manager
                .get_service(LOMAX)
                .and_then(|lomax| lomax.stop())
                .and_then(|lomax| lomax.delete())
            {
                debug!(logger, "Did not delete lomax: {}", error)
            };

            if let Err(error) = service_manager
                .get_service(AVERY)
                .and_then(|avery| avery.stop())
                .and_then(|avery| avery.delete())
            {
                debug!(logger, "Did not delete avery: {}", error)
            };
        })
    );

    let reg_edit = RegistryEditor::new();
    pass_result!(logger, try_deregister_log_source(AVERY));
    pass_result!(logger, try_deregister_log_source(LOMAX));

    let (exe_path, _) = find_firm(
        &reg_edit,
        &logger,
        || default_path_from_env(&logger, "PROGRAMFILES", DEFAULT_FIRM_BIN_PATH),
        || default_path_from_env(&logger, "PROGRAMDATA", DEFAULT_FIRM_DATA_PATH),
    );
    pass_result!(logger, reg_edit.remove_from_path(&exe_path));

    debug!(logger, "Deleting files...");
    pass_result!(
        logger,
        match reg_edit.get_install_files(APPLICATION_NAME) {
            Ok(files) => {
                files.iter().try_for_each(|file| {
                    std::fs::remove_file(file).or_else(|e| {
                        debug!(
                            logger,
                            "Could not remove installed file \"{}\": {}",
                            file.display(),
                            e
                        );
                        debug!(
                            logger,
                            "Marking \"{}\" for deletion in the registry.",
                            file.display()
                        );
                        reg_edit.mark_paths_for_delete(&[file.clone()])
                    })
                })
            }
            Err(e) => {
                debug!(
                    logger,
                    "Could not find any previously installed files to registry: {}", e
                );
                debug!(logger, "Falling back to only removing executables.");

                reg_edit.mark_for_delete(&exe_path)
            }
        }
    );

    pass_result!(logger, reg_edit.deregister_application(APPLICATION_NAME));
    pass_result!(logger, reg_edit.deregister_uninstaller(APPLICATION_NAME));
}

fn main() -> Result<(), u32> {
    let args = InstallerArguments::from_args();
    let log = Logger::root(
        slog::LevelFilter::new(
            slog_async::Async::new(FullFormat::new(TermDecorator::new().build()).build().fuse())
                .build()
                .fuse(),
            if args.verbose {
                slog::Level::Debug
            } else {
                slog::Level::Info
            },
        )
        .fuse(),
        o!(),
    );

    match args.operation {
        InstallOperation::Install {
            install_path,
            data_path,
        } => install(
            log.new(o!("scope" => "install")),
            &install_path.unwrap_or_else(|| {
                default_path_from_env(&log, "PROGRAMFILES", DEFAULT_FIRM_BIN_PATH)
            }),
            &data_path.unwrap_or_else(|| {
                default_path_from_env(&log, "PROGRAMDATA", DEFAULT_FIRM_DATA_PATH)
            }),
        )
        .map(|_| info!(log, "Avery user service will start on next log in.")),
        InstallOperation::Upgrade => upgrade(log.new(o!("scope" => "upgrade"))),
        InstallOperation::Uninstall => {
            uninstall(log.new(o!("scope" => "uninstall")));
            Ok(())
        }
    }
    .map_err(|e| {
        error!(log, "{}", e);
        e.into()
    })
    .map(|_| info!(log, "💪 Done!"))
}

#[cfg(test)]
mod test {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, RwLock};
    use windows_install::{
        populate_fake_registry, registry::mock::new_registry_editor_with_registry,
    };

    macro_rules! null_logger {
        () => {{
            slog::Logger::root(slog::Discard, slog::o!())
        }};
    }

    #[test]
    fn finding_firm() {
        let registry_keys = Arc::new(RwLock::new(HashMap::new()));
        let root = populate_fake_registry!(registry_keys, {r#"SOFTWARE\Firm"#.to_string() => {"InstallPath" => "🍊", "DataPath" => "🥭", "Version" => "🕳️"}});
        let editor = new_registry_editor_with_registry(root, |_| Ok(vec![]));
        let log = null_logger!();
        let (exe, data) = find_firm(&editor, &log, PathBuf::new, PathBuf::new);
        assert_eq!(exe, PathBuf::from("🍊"));
        assert_eq!(data, PathBuf::from("🥭"));

        // Test when we have not put in the data
        let registry_keys = Arc::new(RwLock::new(HashMap::new()));
        let root = populate_fake_registry!(registry_keys, [String::from(r#"SOFTWARE\Firm"#)]);
        let editor = new_registry_editor_with_registry(root, |_| Ok(vec![]));
        let (exe, data) = find_firm(
            &editor,
            &log,
            || PathBuf::from("🔮"),
            || PathBuf::from("🪕"),
        );
        assert_eq!(exe, PathBuf::from("🔮"));
        assert_eq!(data, PathBuf::from("🪕"));
    }
}
