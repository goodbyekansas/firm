#![deny(warnings)]

use slog::{info, o, Drain, Logger};

use gbk_protocols::{
    functions::functions_registry_server::FunctionsRegistryServer, tonic::transport::Server,
};
use quinn::{config, registry};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();

    let log = Logger::root(drain, o!());

    let config = config::Configuration::new(log.new(o!("component" => "config")))?;

    let addr = format!(
        "0.0.0.0:{}",
        std::env::var("PORT").unwrap_or_else(|_| config.port.to_string())
    )
    .parse()?;
    let svc =
        registry::FunctionRegistryService::new(config, log.new(o!("component" => "registry")))?;

    info!(log, "Quinn initialized and listening on {}", addr);

    Server::builder()
        .add_service(FunctionsRegistryServer::new(svc))
        .serve(addr)
        .await?;

    Ok(())
}
