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
        ArgumentType, ExecutionEnvironment, FunctionInput, FunctionOutput, RegisterRequest,
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
    // function executor servicen address
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
        vec![
            RegisterRequest {
                name: "hello_world".to_owned(),
                tags: HashMap::with_capacity(0),
                inputs: Vec::with_capacity(0),
                outputs: Vec::with_capacity(0),
                code: vec![],
                entrypoint: "det du!".to_owned(),
                execution_environment: Some(ExecutionEnvironment {
                    name: "wasm".to_owned(),
                }),
            },
            RegisterRequest {
                name: "say_hello_yourself".to_owned(),
                tags: HashMap::with_capacity(0),
                inputs: vec![
                    FunctionInput {
                        name: "say".to_string(),
                        required: true,
                        r#type: ArgumentType::String as i32,
                        default_value: String::new(),
                    },
                    FunctionInput {
                        name: "count".to_string(),
                        required: false,
                        r#type: ArgumentType::Int as i32,
                        default_value: 1.to_string(),
                    },
                ],
                outputs: vec![FunctionOutput {
                    name: "output_string".to_string(),
                    r#type: ArgumentType::String as i32,
                }],
                code: vec![],
                entrypoint: "kanske".to_owned(),
                execution_environment: Some(ExecutionEnvironment {
                    name: "wasm".to_owned(),
                }),
            },
        ]
        .iter()
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
