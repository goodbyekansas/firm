#![deny(warnings)]

use slog::{info, o, Drain};
use slog_async;
use slog_term;
use tonic::transport::Server;

use avery::{proto::functions_server::FunctionsServer, FunctionsService};

// clean exit on crtl c
async fn ctrlc() {
    match tokio::signal::ctrl_c().await {
        _ => {}
    }
}

// local server main loop
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port: u32 = 1939;
    let addr = format!("[::]:{}", port).parse().unwrap();

    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::CompactFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();

    let log = slog::Logger::root(drain, o!());

    let functions_service = FunctionsService::new(log.new(o!("service" => "functions")));

    info!(
        log,
        "👨‍⚖️ The Firm is listening for requests on port {}", port
    );

    Server::builder()
        .add_service(FunctionsServer::new(functions_service))
        .serve_with_shutdown(addr, ctrlc())
        .await?;
    info!(log, "👋 see you soon - no one leaves the Firm");
    Ok(())
}
