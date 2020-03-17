#![deny(warnings)]

use std::{collections::HashMap, sync::Arc};

use futures;
use slog::{info, o, Drain};
use slog_async;
use slog_term;
use structopt::StructOpt;
use tonic::transport::Server;

use avery::{
    fake_registry::FunctionsRegistryService,
    proto::{
        functions_registry_server::{FunctionsRegistry, FunctionsRegistryServer},
        functions_server::FunctionsServer,
        ExecutionEnvironment, RegisterRequest,
    },
    FunctionsService,
};

// clean exit on crtl c
async fn ctrlc() {
    match tokio::signal::ctrl_c().await {
        _ => {}
    }
}

#[derive(StructOpt, Debug)]
#[structopt(name = "avery")]
struct AveryArgs {
    /// function executor service address
    #[structopt(short, long)]
    skip_register_test_functions: bool,
}

// local server main loop
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::CompactFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();

    let log = slog::Logger::root(drain, o!());

    let args = AveryArgs::from_args();

    let port: u32 = 1939;
    let addr = format!("[::]:{}", port).parse().unwrap();
    let functions_registry_service = Arc::new(FunctionsRegistryService::new());
    if !args.skip_register_test_functions {
        // TODO: this is very temp but gives a nice workflow atm
        match std::option_env!("inputFunctions") {
            Some(fnstring) => {
                fnstring
                    .split(';')
                    .map(|p| RegisterRequest {
                        name: std::path::Path::new(p)
                            .file_stem()
                            .unwrap_or_else(|| std::ffi::OsStr::new("unknown-file-name"))
                            .to_string_lossy()
                            .to_string(),
                        tags: HashMap::with_capacity(0),
                        inputs: Vec::with_capacity(0),
                        outputs: Vec::with_capacity(0),
                        code: std::fs::read(p).unwrap_or_else(|_| vec![]).to_vec(),
                        entrypoint: String::new(),
                        execution_environment: Some(ExecutionEnvironment {
                            name: "wasm".to_owned(),
                        }),
                    })
                    .for_each(|f| {
                        futures::executor::block_on(
                            functions_registry_service.register(tonic::Request::new(f.clone())),
                        )
                        .map_or_else(
                            |e| println!("Failed to register function \"{}\". Err: {}", f.name, e),
                            |_| (),
                        );
                    });
            }
            None => {
                println!("Tried to add functions from the env var but $inputFunctions was not set")
            }
        };
    }

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
        .add_service(FunctionsRegistryServer::new(Arc::clone(
            &functions_registry_service,
        )))
        .serve_with_shutdown(addr, ctrlc())
        .await?;
    info!(log, "👋 see you soon - no one leaves the Firm");
    Ok(())
}
