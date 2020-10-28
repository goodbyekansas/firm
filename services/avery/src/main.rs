use slog::{info, o, Drain};
use structopt::StructOpt;

use firm_protocols::{
    execution::execution_server::ExecutionServer, registry::registry_server::RegistryServer,
    tonic::transport::Server,
};

use avery::{executor::ExecutionService, registry::RegistryService};

// clean exit on crtl c
async fn ctrlc() {
    let _ = tokio::signal::ctrl_c().await;
}

#[derive(StructOpt, Debug)]
#[structopt(name = "avery")]
struct AveryArgs {}

// local server main loop
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::CompactFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();

    let log = slog::Logger::root(drain, o!());

    let _args = AveryArgs::from_args();

    let port: u32 = 1939;
    let addr = format!("[::]:{}", port).parse().unwrap();
    let functions_registry_service = RegistryService::new(log.new(o!("service" => "registry")));

    let execution_service = ExecutionService::new(
        log.new(o!("service" => "functions")),
        Box::new(functions_registry_service.clone()),
    );

    info!(
        log,
        "👨‍⚖️ The Firm is listening for requests on port {}", port
    );

    Server::builder()
        .add_service(ExecutionServer::new(execution_service))
        .add_service(RegistryServer::new(functions_registry_service))
        .serve_with_shutdown(addr, ctrlc())
        .await?;

    info!(log, "👋 see you soon - no one leaves the Firm");
    Ok(())
}
