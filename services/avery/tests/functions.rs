use slog::o;

use avery::{executor::ExecutionService, registry::RegistryService};

use firm_protocols::{
    execution::{execution_server::Execution, ExecutionParameters, InputValue},
    functions::Type,
    registry::registry_server::Registry,
    tonic,
};

use firm_protocols_test_helpers::{
    attachment_data, filters, function_data, input, output, runtime,
};

macro_rules! null_logger {
    () => {{
        slog::Logger::root(slog::Discard, o!())
    }};
}

macro_rules! register_code_attachment {
    ($service:expr) => {{
        futures::executor::block_on(
            $service.register_attachment(tonic::Request::new(attachment_data!("code"))),
        )
        .unwrap()
        .into_inner()
    }};
}

macro_rules! register_functions {
    ($service:expr, $fns:expr) => {{
        $fns.into_iter().for_each(|f| {
            futures::executor::block_on($service.register(tonic::Request::new(f.clone())))
                .map_or_else(
                    |e| println!("Failed to register function \"{}\". Err: {}", f.name, e),
                    |_| (),
                );
        });
        ExecutionService::new(null_logger!(), Box::new($service.clone()))
    }};
}

macro_rules! first_function {
    ($service:expr) => {{
        futures::executor::block_on($service.list(tonic::Request::new(filters!())))
            .unwrap()
            .into_inner()
            .functions
            .first()
            .unwrap()
    }};
}

macro_rules! registry_service {
    () => {{
        RegistryService::new(null_logger!())
    }};
}

#[test]
fn test_execute() {
    let sr = registry_service!();
    let svc = register_functions!(
        sr,
        vec![function_data!(
            "say-hello-yourself",
            [
                input!("say", true, Type::String),
                input!("count", true, Type::Int)
            ],
            [output!("output_string", Type::String)],
            {},
            register_code_attachment!(sr).id,
            runtime!("wasi")
        )]
    );

    let correct_args = vec![
        InputValue {
            name: "say".to_owned(),
            r#type: Type::String as i32,
            value: b"sune".to_vec(),
        },
        InputValue {
            name: "count".to_owned(),
            r#type: Type::Int as i32,
            value: 3i64.to_le_bytes().to_vec(),
        },
    ];
    let r = futures::executor::block_on(svc.execute(tonic::Request::new(ExecutionParameters {
        function: Some(first_function!(sr).clone()),
        arguments: correct_args,
    })));
    assert!(r.is_ok());

    let incorrect_args = vec![
        InputValue {
            name: "say".to_owned(),
            r#type: Type::String as i32,
            value: 3i64.to_le_bytes().to_vec(),
        },
        InputValue {
            name: "count".to_owned(),
            r#type: Type::Int as i32,
            value: b"sune".to_vec(),
        },
    ];

    let r = futures::executor::block_on(svc.execute(tonic::Request::new(ExecutionParameters {
        function: Some(first_function!(sr).clone()),
        arguments: incorrect_args,
    })));
    assert!(r.is_err());
}
