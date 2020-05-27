#![deny(warnings)]

use slog::{info, o, Drain};
use structopt::StructOpt;

use gbk_protocols::{
    functions::{
        functions_registry_server::FunctionsRegistryServer, functions_server::FunctionsServer,
    },
    tonic::transport::Server,
};

use avery::{registry::FunctionsRegistryService, FunctionsService};

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
    let functions_registry_service = FunctionsRegistryService::new(log.new(o!("service" => "registry")));

    let functions_service = FunctionsService::new(
        log.new(o!("service" => "functions")),
        functions_registry_service.clone(),
    );

    info!(
        log,
        "👨‍⚖️ The Firm is listening for requests on port {}", port
    );

    Server::builder()
        .add_service(FunctionsServer::new(functions_service))
        .add_service(FunctionsRegistryServer::new(functions_registry_service))
        .serve_with_shutdown(addr, ctrlc())
        .await?;
    info!(log, "👋 see you soon - no one leaves the Firm");
    Ok(())
}
