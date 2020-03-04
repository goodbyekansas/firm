#![deny(warnings)]

use tonic::transport::Server; //, Request, Response, Status};

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

    let functions_service = FunctionsService::new();

    println!("👨‍⚖️ The Firm is listening for requests on port {}", port);

    Server::builder()
        .add_service(FunctionsServer::new(functions_service))
        .serve_with_shutdown(addr, ctrlc())
        .await?;
    println!("👋 see you soon - no one leaves the Firm");
    Ok(())
}
