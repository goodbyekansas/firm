use std::path::PathBuf;

use firm_types::{
    auth::authentication_server::AuthenticationServer,
    functions::execution_server::ExecutionServer, functions::registry_server::RegistryServer,
    tonic::transport::Server,
};
use slog::{error, info, o, Drain, Logger};
use structopt::StructOpt;
use url::Url;

use avery::{
    auth::AuthService,
    config,
    executor::ExecutionService,
    proxy_registry::{ExternalRegistry, ProxyRegistry},
    registry::RegistryService,
    runtime, system,
};

#[derive(StructOpt, Debug)]
#[structopt(name = "avery")]
struct AveryArgs {
    #[structopt(short = "c", long = "config", parse(from_os_str), env = "AVERY_CONFIG")]
    config: Option<PathBuf>,
}

async fn run(log: Logger) -> Result<(), Box<dyn std::error::Error>> {
    info!(log, "🏎️ Starting Avery...");
    let args = AveryArgs::from_args();

    let config = args
        .config
        .map_or_else(config::Config::new, config::Config::new_with_file)?;

    let external_registries = config
        .registries
        .into_iter()
        .map(|reg| {
            Url::parse(&reg.url)
                .map_err(|e| {
                    format!(
                        "Failed to parse url for external registry \"{}\": {}",
                        reg.name, e
                    )
                })
                .map(|url| ExternalRegistry::new(reg.name, url))
        })
        .collect::<Result<Vec<ExternalRegistry>, String>>()?;

    let internal_registry = RegistryService::new(
        config.internal_registry,
        log.new(o!("service" => "internal-registry")),
    )?;

    let auth_service = AuthService::from_config(
        config.oidc_providers,
        config.auth.scopes,
        config.auth.identity,
        config.auth.key_store,
        config.auth.allow,
        log.new(o!("service" => "auth")),
    )
    .await?;

    let proxy_registry = ProxyRegistry::new(
        external_registries,
        internal_registry,
        config.conflict_resolution,
        auth_service.clone(),
        log.new(o!("service" => "proxy-registry")),
    )
    .await?;

    let mut runtime_directories = config.runtime_directories.clone();
    runtime_directories.push(PathBuf::from(system::DEFAULT_RUNTIME_DIR));
    let directory_sources = runtime_directories
        .into_iter()
        .filter_map(|d| {
            if d.exists() {
                Some(
                    runtime::filesystem_source::FileSystemSource::new(
                        &d,
                        log.new(o!("source" => "fs")),
                    )
                    .map(|fss| Box::new(fss) as Box<dyn runtime::RuntimeSource>),
                )
            } else {
                None
            }
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut runtime_sources: Vec<Box<dyn runtime::RuntimeSource>> = vec![Box::new(
        runtime::InternalRuntimeSource::new(log.new(o!("source" => "internal"))),
    )];
    runtime_sources.extend(directory_sources.into_iter());
    // TODO Creating a temp directory. In the future this must be configurable.
    let temp_root_directory = tempfile::Builder::new()
        .prefix("avery-functions-")
        .tempdir()?;
    let execution_service = ExecutionService::new(
        log.new(o!("service" => "execution")),
        Box::new(proxy_registry.clone()),
        runtime_sources,
        temp_root_directory.path(),
    );

    let (incoming, shutdown_cb) =
        system::create_listener(log.new(o!("scope" => "listener"))).await?;

    Server::builder()
        .add_service(ExecutionServer::new(execution_service))
        .add_service(RegistryServer::new(proxy_registry))
        .add_service(AuthenticationServer::new(auth_service))
        .serve_with_incoming_shutdown(
            incoming,
            system::shutdown_signal(log.new(o!("scope" => "shutdown"))),
        )
        .await
        .map_err(|e| e.to_string())?;

    if let Some(f) = shutdown_cb {
        f();
    }

    info!(log, "👋 see you soon - no one leaves the Firm");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), i32> {
    // TODO: Check if we run as root and exit if that's the case.
    // We cannot be allowed to run as root. Must be run as a user.

    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    let log = Logger::root(drain, o!());

    let exit_log = log.new(o!("scope" => "unhandled_error"));
    run(log).await.map_err(|e| {
        error!(exit_log, "Unhandled error: {}. Exiting", e);
        1i32
    })
}
