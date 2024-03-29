use slog::{error, info, o, Drain, Logger};

use firm_types::{functions::registry_server::RegistryServer, tonic::transport::Server};
use quinn::{config, registry};
use std::error::Error;

async fn run(log: Logger) -> Result<(), Box<dyn Error>> {
    let config_log = log.new(o!("component" => "config"));

    let config = config::Configuration::new(config_log)
        .await
        .map_err(|ce| format!("Configuration error: {}", ce))?;

    let addr = format!(
        "0.0.0.0:{}",
        std::env::var("PORT").unwrap_or_else(|_| config.port.to_string())
    )
    .parse()?;
    let svc =
        registry::RegistryService::new(config, log.new(o!("component" => "registry"))).await?;

    info!(log, "Quinn initialized and listening on {}", addr);

    Server::builder()
        .add_service(RegistryServer::new(svc))
        .serve(addr)
        .await?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), i32> {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    let log = Logger::root(drain, o!());

    let exit_log = log.new(o!("component" => "exit"));
    run(log).await.map_err(|e| {
        error!(exit_log, "Unhandled error: {}. Exiting", e);
        1i32
    })
}
