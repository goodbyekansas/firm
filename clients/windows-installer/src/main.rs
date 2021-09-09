use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
};

use flate2::bufread::GzDecoder;
use slog::{debug, error, info, o, Drain, Logger};
use slog_term::{FullFormat, TermDecorator};
use structopt::StructOpt;
use tar::{Archive, Entry};
use thiserror::Error;
use winapi::um::{
    winnt::DELETE,
    winsvc::{SC_MANAGER_CREATE_SERVICE, SC_MANAGER_ENUMERATE_SERVICE},
};
mod registry;
mod service;

use registry::RegistryEditor;

const AVERY: &str = "Avery";
const LOMAX: &str = "Lomax";

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
    ServiceError(#[from] service::ServiceError),

    #[error(transparent)]
    EventError(#[from] windows_events::EventError),

    #[error(transparent)]
    RegistryError(#[from] registry::RegistryError),
}

impl From<InstallerError> for u32 {
    fn from(installer_error: InstallerError) -> Self {
        match installer_error {
            InstallerError::FailedToCopyFile(_, _) => 1,
            InstallerError::FailedToRemoveFiles(_, _) => 2,
            InstallerError::FailedToFindCurrentExe(_) => 3,
            InstallerError::ArchiveError(_) => 4,
            InstallerError::ServiceError(e) => e.into(),
            InstallerError::EventError(_) => 200,
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
    reg_edit.find_application("Firm").map(|entries| {
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
        .map(|appdata| PathBuf::from(&appdata))
        .unwrap_or_else(|| {
            debug!(
                logger,
                r#"Could not find "{}" in environment, fallback to "{}""#, key, default
            );
            PathBuf::from(default)
        })
        .join("Firm")
}

fn unpack_entry<E>(mut entry: Entry<E>, install_path: &Path) -> Result<(), InstallerError>
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
                    .and_then(|file_name| entry.unpack(install_path.join(file_name)).map(|_| ()))
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

fn unpack_data_entry<E>(mut entry: Entry<E>, data_path: &Path) -> Result<(), InstallerError>
where
    E: io::Read,
{
    std::fs::create_dir_all(&data_path)
        .and_then(|_| entry.unpack_in(data_path).map(|_| ()))
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
) -> Result<(), InstallerError> {
    let archive = include_bytes!("../install-data");
    debug!(logger, "🗜️ Unpacking archive...");
    Archive::new(GzDecoder::new(&archive[..]))
        .entries()
        .map_err(|e| InstallerError::ArchiveError(e.to_string()))
        .and_then(|mut entries| {
            entries.try_for_each(|entry_res| {
                entry_res
                    .map_err(|e| InstallerError::ArchiveError(e.to_string()))
                    .and_then(|entry| match entry.header().entry_type() {
                        tar::EntryType::Directory => Ok(()),
                        tar::EntryType::Regular => {
                            if entry
                                .path()
                                .map(|p| p.starts_with(Path::new(".").join("bin")))
                                .unwrap_or_default()
                            {
                                unpack_entry(entry, &install_path)
                            } else {
                                unpack_data_entry(entry, &data_path)
                            }
                        }
                        _ => Err(InstallerError::ArchiveError(format!(
                            r#"Entry "{}" is of unsupported type "{:#?}" "#,
                            entry.path().unwrap_or_default().display(),
                            entry.header().entry_type()
                        ))),
                    })
            })
        })
        .and_then(|_| std::env::current_exe().map_err(InstallerError::FailedToFindCurrentExe))
        .and_then(|installer| {
            fs::copy(installer, &install_path.join("install.exe"))
                .map_err(|e| InstallerError::FailedToCopyFile(String::from("install.exe"), e))
                .map(|_| ())
        })
}

fn remove_directory(path: &Path) -> Result<(), InstallerError> {
    fs::remove_dir_all(path)
        .or_else(|e| match e.kind() {
            io::ErrorKind::NotFound => Ok(()),
            _ => Err(e),
        })
        .map_err(|e| InstallerError::FailedToRemoveFiles(path.to_path_buf(), e))
}

fn get_config_arg(path: &Path, name: &str) -> String {
    let config_path = path.join(name);
    config_path
        .exists()
        .then(|| format!(r#"--config "{}""#, config_path.to_string_lossy()))
        .unwrap_or_default()
}

fn upgrade(logger: Logger) -> Result<(), InstallerError> {
    info!(logger, "☝️ Upgrading...");
    let reg_edit = registry::RegistryEditor::new();
    let (exe_path, data_path) = find_firm(
        &reg_edit,
        &logger,
        || default_path_from_env(&logger, "PROGRAMFILES", DEFAULT_FIRM_BIN_PATH),
        || default_path_from_env(&logger, "PROGRAMDATA", DEFAULT_FIRM_DATA_PATH),
    );
    uninstall(logger.new(o!("scope" => "uninstall")));

    install(logger.new(o!("scope" => "install")), &exe_path, &data_path)
        .and_then(|_| {
            service::get_service_manager(SC_MANAGER_ENUMERATE_SERVICE)
                .and_then(|handle| service::get_services(&handle, &format!("{}_", AVERY)))
                .map_err(Into::into)
        })
        .and_then(|services| service::start_services(services).map_err(Into::into))
}

fn install(logger: Logger, install_path: &Path, data_path: &Path) -> Result<(), InstallerError> {
    debug!(
        logger,
        r#"Using executable dir: "{}" and data dir: "{}""#,
        install_path.to_string_lossy(),
        data_path.to_string_lossy()
    );
    info!(logger, "💾 Installing...");
    let reg_edit = registry::RegistryEditor::new();
    pass_result!(logger, reg_edit.cancel_pending_deletions(install_path));

    copy_files(&logger, install_path, data_path)
        .and_then(|_| {
            windows_events::try_register(AVERY, &install_path.join("avery.exe").to_string_lossy())
                .map_err(Into::into)
        })
        .and_then(|_| {
            windows_events::try_register(LOMAX, &install_path.join("lomax.exe").to_string_lossy())
                .map_err(Into::into)
        })
        .and_then(|_| {
            debug!(logger, "🏃‍♀️ Starting services.");
            service::get_service_manager(SC_MANAGER_CREATE_SERVICE)
                .and_then(|handle| {
                    service::create_user_service(
                        AVERY,
                        &install_path.join("avery.exe").to_string_lossy(),
                        &handle,
                        &[
                            "--service",
                            get_config_arg(data_path, "avery.toml").as_str(),
                        ],
                    )
                    .map(|_| handle)
                })
                .and_then(|handle| {
                    service::create_system_service(
                        LOMAX,
                        &install_path.join("lomax.exe").to_string_lossy(),
                        &handle,
                        &[
                            "--service",
                            get_config_arg(data_path, "lomax.toml").as_str(),
                        ],
                    )
                })
                .and_then(|lomax| service::start_service(&lomax))
                .map_err(Into::into)
        })
        .and_then(|_| reg_edit.add_to_path(&install_path).map_err(Into::into))
        .and_then(|_| {
            let mut additional_data = HashMap::new();
            additional_data.insert(
                String::from("DataPath"),
                data_path.to_string_lossy().into_owned(),
            );
            additional_data.insert(String::from("Version"), String::from(std::env!("version")));
            reg_edit
                .register_application("Firm", install_path, additional_data)
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
                    "Firm",
                    "Firm",
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
        service::get_service_manager(SC_MANAGER_ENUMERATE_SERVICE | DELETE)
            .and_then(|handle| {
                service::get_services(&handle, format!("{}_", AVERY).as_str())
                    .map(|services| (handle, services))
            })
            .and_then(|(manager_handle, user_services)| {
                debug!(logger, "Stopping user services.");
                user_services
                    .iter()
                    .try_for_each(|handle| {
                        debug!(logger, "Stopping: {}", handle);
                        service::stop_service(handle)
                    })
                    .map(|_| manager_handle)
            })
            .map(|manager_handle| {
                debug!(logger, "Stopping system services.");
                if let Err(error) =
                    service::get_service_handle(LOMAX, &manager_handle).and_then(|lomax| {
                        service::stop_service(&lomax).and_then(|_| service::delete_service(&lomax))
                    })
                {
                    debug!(logger, "Did not delete lomax: {}", error)
                };

                if let Err(error) =
                    service::get_service_handle(AVERY, &manager_handle).and_then(|avery| {
                        service::stop_service(&avery).and_then(|_| service::delete_service(&avery))
                    })
                {
                    debug!(logger, "Did not delete avery: {}", error)
                };
            }),
        "😭 Failed to stop services"
    );
    let reg_edit = registry::RegistryEditor::new();
    pass_result!(logger, windows_events::try_deregister(AVERY));
    pass_result!(logger, windows_events::try_deregister(LOMAX));

    let (exe_path, data_path) = find_firm(
        &reg_edit,
        &logger,
        || default_path_from_env(&logger, "PROGRAMFILES", DEFAULT_FIRM_BIN_PATH),
        || default_path_from_env(&logger, "PROGRAMDATA", DEFAULT_FIRM_DATA_PATH),
    );
    pass_result!(logger, reg_edit.remove_from_path(&exe_path));
    pass_result!(logger, reg_edit.remove_from_path(&data_path));

    debug!(logger, "Marking folders for deletion");
    pass_result!(logger, reg_edit.mark_for_delete(&exe_path));

    pass_result!(logger, remove_directory(&data_path));
    pass_result!(logger, reg_edit.deregister_application("Firm"));
    pass_result!(logger, reg_edit.deregister_uninstaller("Firm"));
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
    use crate::populate_fake_registry;
    use std::collections::HashMap;
    use std::sync::{Arc, RwLock};

    macro_rules! null_logger {
        () => {{
            slog::Logger::root(slog::Discard, slog::o!())
        }};
    }

    #[test]
    fn finding_firm() {
        let registry_keys = Arc::new(RwLock::new(HashMap::new()));
        let root = populate_fake_registry!(registry_keys, {r#"SOFTWARE\Firm"#.to_string() => {"InstallPath" => "🍊", "DataPath" => "🥭", "Version" => "🕳️"}});
        let editor = RegistryEditor::new_with_registry(root, |_| Ok(vec![]));
        let log = null_logger!();
        let (exe, data) = find_firm(&editor, &log, PathBuf::new, PathBuf::new);
        assert_eq!(exe, PathBuf::from("🍊"));
        assert_eq!(data, PathBuf::from("🥭"));

        // Test when we have not put in the data
        let registry_keys = Arc::new(RwLock::new(HashMap::new()));
        let root = populate_fake_registry!(registry_keys, [String::from(r#"SOFTWARE\Firm"#)]);
        let editor = registry::RegistryEditor::new_with_registry(root, |_| Ok(vec![]));
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
