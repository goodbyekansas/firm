#![deny(warnings)]

use gbk_protocols::{
    functions::functions_registry_server::FunctionsRegistryServer, tonic::transport::Server,
};
use quinn::{config, registry};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = config::Configuration::new()?;

    let addr = format!(
        "0.0.0.0:{}",
        std::env::var("PORT").unwrap_or_else(|_| config.port.to_string())
    )
    .parse()?;
    let svc = registry::FunctionRegistryService::new(config)?;

    println!("Quinn listening on {}", addr);

    Server::builder()
        .add_service(FunctionsRegistryServer::new(svc))
        .serve(addr)
        .await?;

    Ok(())
}
