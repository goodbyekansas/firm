use slog::{error, info, o, Drain, Logger};
use structopt::StructOpt;

use firm_types::{
    functions::{execution_server::ExecutionServer, registry_server::RegistryServer},
    tonic::transport::Server,
};

use avery::{
    config,
    executor::ExecutionService,
    proxy_registry::{ExternalRegistry, ProxyRegistry},
    registry::RegistryService,
};
use std::path::PathBuf;
use url::Url;

// clean exit on crtl c
async fn ctrlc() {
    let _ = tokio::signal::ctrl_c().await;
}

#[derive(StructOpt, Debug)]
#[structopt(name = "avery")]
struct AveryArgs {
    #[structopt(short = "c", long = "config", parse(from_os_str), env = "AVERY_CONFIG")]
    config: Option<PathBuf>,
}

async fn run(log: Logger) -> Result<(), Box<dyn std::error::Error>> {
    let args = AveryArgs::from_args();

    let config = args
        .config
        .map_or_else(config::Config::new, config::Config::new_with_file)?;
    let port = config.port;
    let addr = format!("[::]:{}", port).parse().unwrap();

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
                .and_then(|url| {
                    let reg_name = reg.name.clone();
                    reg.oauth_scope
                        .map(|scope| {
                            std::env::var(format!("AVERY_OAUTH_{}", scope)).map_err(|e| {
                                format!(
                                    "Could not use environment variable \"{}\": {}",
                                    format!("AVERY_OAUTH_{}", scope),
                                    e
                                )
                            })
                        })
                        .transpose()
                        .map(|oauth| {
                            oauth.map_or_else(
                                || ExternalRegistry::new(reg_name.clone(), url.clone()),
                                |oauth| {
                                    ExternalRegistry::new_with_oauth(
                                        reg_name.clone(),
                                        url.clone(),
                                        oauth,
                                    )
                                },
                            )
                        })
                })
        })
        .collect::<Result<Vec<ExternalRegistry>, String>>()?;

    let internal_registry = RegistryService::new(
        config.internal_registry,
        log.new(o!("service" => "internal-registry")),
    );

    let proxy_registry = ProxyRegistry::new(
        external_registries,
        internal_registry,
        config.conflict_resolution,
        log.new(o!("service" => "proxy-registry")),
    )
    .await?;

    let execution_service = ExecutionService::new(
        log.new(o!("service" => "functions")),
        Box::new(proxy_registry.clone()),
    );

    info!(
        log,
        "👨‍⚖️ The Firm is listening for requests on port {}", port
    );

    Server::builder()
        .add_service(ExecutionServer::new(execution_service))
        .add_service(RegistryServer::new(proxy_registry))
        .serve_with_shutdown(addr, ctrlc())
        .await?;

    info!(log, "👋 see you soon - no one leaves the Firm");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), i32> {
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
