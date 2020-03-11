#![deny(warnings)]

use std::collections::HashMap;

use slog::{info, o, Drain};
use slog_async;
use slog_term;
use tonic::transport::Server;
use uuid::Uuid;

use avery::{
    proto::{
        functions_server::FunctionsServer, ArgumentType, Function, FunctionId, FunctionInput,
        FunctionOutput,
    },
    FunctionDescriptor, FunctionExecutionEnvironment, FunctionExecutorEnvironmentDescriptor,
    FunctionsService,
};

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

    let functions = vec![
        FunctionDescriptor {
            id: Uuid::parse_str("0c8c108c-bf61-4735-a86d-2d0f5b53561c")
                .unwrap_or_else(|_| Uuid::new_v4()),
            execution_environment: FunctionExecutionEnvironment {
                name: "rust".to_owned(),
                descriptor: FunctionExecutorEnvironmentDescriptor::Inline(vec![1]),
            },
            function: Function {
                id: Some(FunctionId {
                    value: "0c8c108c-bf61-4735-a86d-2d0f5b53561c".to_string(),
                }),
                name: "hello_world".to_owned(),
                tags: HashMap::with_capacity(0),
                inputs: Vec::with_capacity(0),
                outputs: Vec::with_capacity(0),
            },
        },
        FunctionDescriptor {
            id: Uuid::parse_str("ef394e5b-0b32-447d-b483-a34bcb70cbc0")
                .unwrap_or_else(|_| Uuid::new_v4()),
            execution_environment: FunctionExecutionEnvironment {
                name: "wasm".to_owned(),
                descriptor: FunctionExecutorEnvironmentDescriptor::Inline(vec![1]),
            },
            function: Function {
                id: Some(FunctionId {
                    value: "ef394e5b-0b32-447d-b483-a34bcb70cbc0".to_string(),
                }),
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
            },
        },
    ];

    let functions_service =
        FunctionsService::new(log.new(o!("service" => "functions")), functions.iter());

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
