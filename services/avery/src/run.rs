use std::path::PathBuf;

use firm_types::{
    auth::authentication_server::AuthenticationServer,
    functions::execution_server::ExecutionServer, functions::registry_server::RegistryServer,
    tonic::transport::Server,
};
use slog::{error, info, o, Drain, Logger};
use structopt::StructOpt;
use url::Url;

use crate::{
    auth::AuthService,
    config,
    executor::ExecutionService,
    proxy_registry::{ExternalRegistry, ProxyRegistry},
    registry::RegistryService,
    runtime, system,
};

#[derive(StructOpt, Debug)]
#[structopt(name = "avery")]
pub struct AveryArgs {
    #[structopt(short = "c", long = "config", parse(from_os_str), env = "AVERY_CONFIG")]
    config: Option<PathBuf>,

    #[cfg(windows)]
    #[structopt(short = "s", long = "service")]
    pub service: bool,
}

pub fn create_logger() -> Logger {
    Logger::root(
        slog_async::Async::new(
            slog_term::FullFormat::new(slog_term::TermDecorator::new().build())
                .build()
                .fuse(),
        )
        .build()
        .fuse(),
        o!(),
    )
}

pub async fn run<G>(
    args: AveryArgs,
    started_callback: G,
    log: Logger,
) -> Result<(), Box<dyn std::error::Error>>
where
    G: FnOnce() -> Result<(), String>,
{
    info!(log, "🏎️ Starting Avery...");

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
    runtime_directories.push(system::default_runtime_dir());
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

    let execution_service = ExecutionService::new(
        log.new(o!("service" => "execution")),
        proxy_registry.clone(),
        runtime_sources,
        auth_service.clone(),
        // TODO In the future root_directory must be configurable
        &system::user_cache_path()
            .map(|p| p.join("functions"))
            .ok_or_else(|| "Failed to get user cache path.".to_owned())
            .and_then(|p| {
                std::fs::create_dir_all(&p)
                    .map_err(|e| format!("Failed to create Avery cache directory: {}", e))
                    .map(|_| p)
            })?,
    )?;

    let (incoming, shutdown_cb) =
        system::create_listener(log.new(o!("scope" => "listener"))).await?;
    started_callback().map_err(|e| format!("Failed to signal startup done: {}", e))?;

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
pub async fn run_with_tokio(args: AveryArgs) -> Result<(), i32> {
    let log = create_logger();
    let exit_log = log.new(o!("scope" => "unhandled_error"));

    run(args, || Ok(()), log).await.map_err(|e| {
        error!(exit_log, "Unhandled error: {}. Exiting", e);
        1i32
    })
}
