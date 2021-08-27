use std::{
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
    let (exe_path, data_path) = registry::find_firm(logger.new(o!()));
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
    pass_result!(logger, registry::cancel_pending_deletions(install_path));

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
        .and_then(|_| registry::add_to_path(&install_path).map_err(Into::into))
        .and_then(|_| registry::register_firm(install_path, data_path).map_err(Into::into))
        .and_then(|_| registry::register_uninstaller(&install_path).map_err(Into::into))
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
                        debug!(logger, "Stopping: {:#?}", handle);
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

    pass_result!(logger, windows_events::try_deregister(AVERY));
    pass_result!(logger, windows_events::try_deregister(LOMAX));

    let (exe_path, data_path) = registry::find_firm(logger.new(o!()));
    pass_result!(logger, registry::remove_from_path(&exe_path));
    pass_result!(logger, registry::remove_from_path(&data_path));

    debug!(logger, "Marking folders for deletion");
    registry::mark_folder_for_deletion(&exe_path)
        .iter()
        .for_each(|e| debug!(logger, "{}", e));

    pass_result!(logger, remove_directory(&data_path));
    pass_result!(logger, registry::deregister_firm());
    pass_result!(logger, registry::deregister_uninstaller());
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
                default_path_from_env(&log, "PROGRAMFILES", r#"C:\Program Files"#)
            }),
            &data_path
                .unwrap_or_else(|| default_path_from_env(&log, "PROGRAMDATA", r#"C:\ProgramData"#)),
        ),
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
