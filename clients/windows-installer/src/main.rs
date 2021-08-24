use std::path::Path;

use flate2::bufread::GzDecoder;
use structopt::StructOpt;
use tar::{Archive, Entry};
use winapi::um::{
    winnt::DELETE,
    winsvc::{SC_MANAGER_CREATE_SERVICE, SC_MANAGER_ENUMERATE_SERVICE},
};

pub mod service;

#[derive(StructOpt, Debug)]
enum InstallOperation {
    Install {
        #[structopt(long, short = "p", default_value = r#"C:\Program Files\Firm"#)]
        // TODO get default from registry
        install_path: std::path::PathBuf,
        #[structopt(long, short, default_value = r#"C:\ProgramData\Firm"#)]
        // TODO get default from c api?
        data_path: std::path::PathBuf,
    },
    Uninstall,
    Upgrade {},
}

#[derive(StructOpt, Debug)]
#[structopt(name = "Avery service installer")]
struct InstallerArguments {
    #[structopt(subcommand)]
    operation: InstallOperation,
}

fn unpack_entry<E>(mut entry: Entry<E>, install_path: &Path) -> Result<(), std::io::Error>
where
    E: std::io::Read,
{
    std::fs::create_dir_all(&install_path).and_then(|_| {
        entry.path().map(|p| p.to_path_buf()).and_then(|path| {
            path.file_name()
                .ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!(
                            r#"File "{}" is missing name"#,
                            entry.path().unwrap_or_default().display()
                        ),
                    )
                })
                // TODO: Install files into known paths.
                // data -> ProgramData
                // programs -> Program Files etc.
                .and_then(|file_name| entry.unpack(install_path.join(file_name)).map(|_| ()))
        })
    })
}

fn copy_files(install_path: &Path, data_path: &Path) -> Result<(), String> {
    let archive = include_bytes!("../install-data");
    Archive::new(GzDecoder::new(&archive[..]))
        .entries()
        .map_err(|e| format!("Failed to get data to install: {}", e))
        .and_then(|mut entries| {
            entries.try_for_each(|entry_res| {
                entry_res
                    .and_then(|entry| match entry.header().entry_type() {
                        tar::EntryType::Directory => Ok(()),
                        tar::EntryType::Regular => {
                            let install_path = if entry
                                .path()
                                .map(|p| p.starts_with(Path::new(".").join("bin")))
                                .unwrap_or_default()
                            {
                                install_path
                            } else {
                                data_path
                            };
                            unpack_entry(entry, &install_path)
                        }
                        _ => Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            format!(
                                r#"Entry "{}" is of unsupported type "{:#?}" "#,
                                entry.path().unwrap_or_default().display(),
                                entry.header().entry_type()
                            ),
                        )),
                    })
                    .map_err(|e| format!("Failed to copy file: {}", e))
            })
        })
        // TODO return the config paths for avery and lomax so they can be started
        // with --config pointing to the installed place
}

fn upgrade(name: &str) -> Result<(), u32> {
    // TODO upgrade lomax and bendini also
    service::get_service_manager(SC_MANAGER_ENUMERATE_SERVICE)
        .and_then(|handle| service::get_user_services(&handle, &format!("{}_", name)))
        .and_then(|services| {
            println!("Stopping services.");
            services
                .iter()
                .try_for_each(service::stop_service)
                .map(|_| services)
        })
        .and_then(|services| {
            println!("Installing files.");
            // TODO find out where firm was installed somehow
            copy_files(
                Path::new(r#"C:\Program Files\Firm"#),
                Path::new(r#"C:\Program Files\Firm"#),
            )?; // TODO we should try to restart the services we stopped instead of early exit

            service::register_windows_event("Avery", r#"C:\Program Files\Firm\avery.exe"#)?;
            Ok(services)
        })
        .and_then(|services| {
            println!("Starting services.");
            services.iter().try_for_each(service::start_service)
        })
        .map_err(|e| {
            println!("Failed to restart services: {}", e);
            73
        })
}

fn install(name: &str, install_path: &Path, data_path: &Path, args: &[&str]) -> Result<(), u32> {
    // TODO: Should we do this and deregister with the API directly?
    //TODO winlog::register("Lomax");
    println!("Installing files.");
    copy_files(install_path, data_path)
        .map_err(|e| {
            println!("Error installing files: {}", e);
            21
        })
        .and_then(|_| {
            println!("Starting services.");
            service::get_service_manager(SC_MANAGER_CREATE_SERVICE)
                // TODO create a managed service account for lomax
                // TODO create a system service for lomax that runs with the lomax user
                .and_then(|handle| {
                    service::create_user_service(
                        name,
                        &install_path.join("avery.exe").to_string_lossy(),
                        &handle,
                        args,
                    )
                })
                .map_err(|e| {
                    println!(r#"Error installing service "{}": {}"#, name, e);
                    3
                })
        })
        .map(|_| ())
    // TODO add bendini to PATH
    // TODO add to add/remove program
}

fn uninstall(name: &str) -> Result<(), u32> {
    //TODO winlog::deregister("Lomax");
    service::get_service_manager(DELETE)
        .and_then(|handle| service::get_service_handle(name, &handle))
        .and_then(|service_handle| {
            // TODO: If we fail to stop the service we don't delete it?
            // Some more robustness could be nice.
            service::stop_service(&service_handle)
                .and_then(|_| service::delete_service(&service_handle))
        })
        .and_then(|_| service::deregister_windows_event("Avery"))
        .map_err(|e| {
            println!(r#"Could not uninstall service "{}": {}"#, name, e);
            4
        })
    // TODO remove files
    // TODO remove from add/remove program
    // TODO remove bendini from PATH
    // TODO remove lomax service user
}

fn main() -> Result<(), u32> {
    let args = InstallerArguments::from_args();
    match args.operation {
        InstallOperation::Install {
            install_path,
            data_path,
        } => install("Avery", &install_path, &data_path, &["--service"]),
        InstallOperation::Upgrade { .. } => upgrade("Avery"),
        InstallOperation::Uninstall => uninstall("Avery"),
    }
}
