use slog::{error, info, o, Drain, Logger};
use structopt::StructOpt;

use firm_types::{
    functions::{execution_server::ExecutionServer, registry_server::RegistryServer},
    tonic::transport::Server,
};

use avery::{executor::ExecutionService, registry::RegistryService};
use std::path::PathBuf;

mod config;

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

    let internal_registry = RegistryService::new(log.new(o!("service" => "internal-registry")));

    let execution_service = ExecutionService::new(
        log.new(o!("service" => "functions")),
        Box::new(internal_registry.clone()),
    );

    info!(
        log,
        "👨‍⚖️ The Firm is listening for requests on port {}", port
    );

    Server::builder()
        .add_service(ExecutionServer::new(execution_service))
        .add_service(RegistryServer::new(internal_registry))
        .serve_with_shutdown(addr, ctrlc())
        .await?;

    info!(log, "👋 see you soon - no one leaves the Firm");
    Ok(())
}

// local server main loop
#[tokio::main]
async fn main() -> Result<(), i32> {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::CompactFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    let log = Logger::root(drain, o!());

    let exit_log = log.new(o!("scope" => "unhandled_error"));
    run(log).await.map_err(|e| {
        error!(exit_log, "Unhandled error: {}. Exiting", e);
        1i32
    })
}
