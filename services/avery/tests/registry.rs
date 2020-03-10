use std::collections::HashMap;

use futures;

use avery::fake_registry::FunctionsRegistryService;
use avery::proto::{
    functions_registry_server::FunctionsRegistry, ArgumentType, ExecutionEnvironment, FunctionId,
    FunctionInput, FunctionOutput, ListRequest, RegisterRequest,
};

macro_rules! registry {
    () => {{
        FunctionsRegistryService::new()
    }};
}

macro_rules! register_request {
    () => {{
        tonic::Request::new(RegisterRequest {
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
        })
    }};
}

macro_rules! custom_register_request {
    ($name:expr, $entrypoint:expr, $execution_environment:expr) => {{
        tonic::Request::new(RegisterRequest {
            name: $name.to_owned(),
            tags: HashMap::with_capacity(0),
            inputs: vec![],
            outputs: vec![],
            code: vec![],
            entrypoint: $entrypoint.to_owned(),
            execution_environment: $execution_environment,
        })
    }};
}

#[test]
fn test_list_functions() {
    let fr = registry!();

    // Test empty
    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(ListRequest {
        name_filter: "".to_owned(),
        tags_filter: HashMap::new(),
        offset: 0,
        limit: 100,
    })));

    assert!(list_request.is_ok());
    assert_eq!(0, list_request.unwrap().into_inner().functions.len());

    // Test with 3
    futures::executor::block_on(fr.register(register_request!())).unwrap();
    futures::executor::block_on(fr.register(register_request!())).unwrap();
    futures::executor::block_on(fr.register(register_request!())).unwrap();

    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(ListRequest {
        name_filter: "".to_owned(),
        tags_filter: HashMap::new(),
        offset: 0,
        limit: 100,
    })));

    assert!(list_request.is_ok());
    assert_eq!(3, list_request.unwrap().into_inner().functions.len());

    // TODO: Test filtering
}

#[test]
fn test_get_function() {
    let fr = registry!();

    // Test get with invalid UUID
    let get_request = futures::executor::block_on(fr.get(tonic::Request::new(FunctionId {
        value: "oran malifant".to_owned(),
    })));

    assert!(get_request.is_err());
    assert!(match get_request.unwrap_err().code() {
        tonic::Code::InvalidArgument => true,
        _ => false,
    });

    // Test with valid UUID but doesn't exist
    let get_request = futures::executor::block_on(fr.get(tonic::Request::new(FunctionId {
        value: "18406c9e-d91a-4226-b0b3-6a02ccaa8b74".to_owned(),
    })));

    assert!(get_request.is_err());
    assert!(match get_request.unwrap_err().code() {
        tonic::Code::NotFound => true,
        _ => false,
    });

    // Test actually getting a function
    let f_id = futures::executor::block_on(fr.register(register_request!()))
        .unwrap()
        .into_inner();
    let get_request = futures::executor::block_on(fr.get(tonic::Request::new(f_id.clone())));
    assert!(get_request.is_ok());

    assert_eq!(
        f_id.value,
        get_request
            .unwrap()
            .into_inner()
            .function
            .unwrap()
            .id
            .unwrap()
            .value
    );
}

#[test]
fn test_register_function() {
    // Register a function missing execution environment
    let fr = registry!();
    let register_result = futures::executor::block_on(fr.register(custom_register_request!(
        "create-cake",
        "my-entrypoint",
        None
    )));

    assert!(register_result.is_err());
    assert!(match register_result.unwrap_err().code() {
        tonic::Code::InvalidArgument => true,
        _ => false,
    });

    // Testing if we can register a valid function
    let register_result = futures::executor::block_on(fr.register(custom_register_request!(
        "my-name",
        "my-entrypoint",
        Some(ExecutionEnvironment {
            name: "wassaa".to_owned(),
        })
    )));
    assert!(register_result.is_ok());
}
