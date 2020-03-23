#![deny(warnings)]

use std::{path::Path, sync::Arc};

use futures;
use slog::{error, info, o, Drain};
use slog_async;
use slog_term;
use structopt::StructOpt;
use tonic::transport::Server;

use avery::{
    fake_registry::FunctionsRegistryService,
    manifest::FunctionManifest,
    proto::{
        functions_registry_server::{FunctionsRegistry, FunctionsRegistryServer},
        functions_server::FunctionsServer,
        RegisterRequest,
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
        info!(log, "registering functions from $inputFunctions...");
        match std::env::var("inputFunctions") {
            Ok(fnstring) => {
                fnstring
                    .split(';')
                    .filter_map(|p| {
                        let manifest_path = Path::new(p).join("manifest.toml");

                        let manifest = FunctionManifest::parse(manifest_path)
                            .map_err(|e| error!(log, "\"{}\". Skipping", e))
                            .ok()?;

                        let mut register_request = RegisterRequest::from(&manifest);
                        let code_path = Path::new(p)
                            .join("bin")
                            .join(format!("{}.wasm", manifest.name()));
                        info!(log, "reading code file from: {}", code_path.display());
                        register_request.code = std::fs::read(code_path)
                            .map_err(|e| {
                                error!(
                                    log,
                                    "Failed to read code for function {}: {}. Skipping.",
                                    manifest.name(),
                                    e
                                )
                            })
                            .ok()?
                            .to_vec();
                        Some(register_request)
                    })
                    .for_each(|f| {
                        futures::executor::block_on(
                            functions_registry_service.register(tonic::Request::new(f.clone())),
                        )
                        .map_or_else(
                            |e| {
                                error!(
                                    log,
                                    "Failed to register function \"{}\". Err: {}", f.name, e
                                )
                            },
                            |_| (),
                        );
                    });
            }
            Err(e) => error!(
                log,
                "Tried to add functions from the env var but $inputFunctions was not set. {}", e
            ),
        };

        info!(log, "done registering functions from $inputFunctions");
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
