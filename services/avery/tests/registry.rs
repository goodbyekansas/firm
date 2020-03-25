use std::collections::HashMap;

use futures;

use avery::proto::{
    functions_registry_server::FunctionsRegistry, ArgumentType, ExecutionEnvironment, FunctionId,
    FunctionInput, FunctionOutput, GetLatestVersionRequest, ListRequest, ListVersionsRequest,
    Ordering, RegisterRequest,
};
use avery::registry::FunctionsRegistryService;

macro_rules! registry {
    () => {{
        FunctionsRegistryService::new()
    }};
}

macro_rules! register_request {
    ($name: expr) => {{
        tonic::Request::new(RegisterRequest {
            name: $name.to_owned(),
            version: "0.1.0".to_owned(),
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
            version: "0.1.0".to_owned(),
            tags: HashMap::with_capacity(0),
            inputs: vec![],
            outputs: vec![],
            code: vec![],
            entrypoint: $entrypoint.to_owned(),
            execution_environment: $execution_environment,
        })
    }};
}

macro_rules! register_request_with_version {
    ($name:expr, $version:expr) => {{
        tonic::Request::new(RegisterRequest {
            name: $name.to_owned(),
            version: $version.to_owned(),
            tags: HashMap::with_capacity(0),
            inputs: vec![],
            outputs: vec![],
            code: vec![],
            entrypoint: "kanske".to_owned(),
            execution_environment: Some(ExecutionEnvironment {
                name: "wasm".to_owned(),
            }),
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
    futures::executor::block_on(fr.register(register_request!("test-1"))).unwrap();
    futures::executor::block_on(fr.register(register_request!("test-2"))).unwrap();
    futures::executor::block_on(fr.register(register_request!("test-3"))).unwrap();

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
    assert!(matches!(
        get_request.unwrap_err().code(),
        tonic::Code::InvalidArgument
    ));

    // Test with valid UUID but doesn't exist
    let get_request = futures::executor::block_on(fr.get(tonic::Request::new(FunctionId {
        value: "18406c9e-d91a-4226-b0b3-6a02ccaa8b74".to_owned(),
    })));

    assert!(get_request.is_err());
    assert!(matches!(
        get_request.unwrap_err().code(),
        tonic::Code::NotFound
    ));

    // Test actually getting a function
    let f_id = futures::executor::block_on(fr.register(register_request!("func")))
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
    assert!(matches!(
        register_result.unwrap_err().code(),
        tonic::Code::InvalidArgument
    ));

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

#[test]
fn test_list_versions() {
    let fr = registry!();

    futures::executor::block_on(fr.register(register_request_with_version!("my-name", "1.2.3")))
        .unwrap();
    futures::executor::block_on(fr.register(register_request_with_version!("my-name", "2.0.3")))
        .unwrap();
    futures::executor::block_on(fr.register(register_request_with_version!("my-name", "8.1.4")))
        .unwrap();

    // since these are prereleases, they will only match exact version requirements
    let list_result =
        futures::executor::block_on(fr.list_versions(tonic::Request::new(ListVersionsRequest {
            name: "my-name".to_owned(),
            version_requirement: "1.2.3-dev".to_owned(),
            limit: 100,
            offset: 0,
            ordering: Ordering::Descending as i32,
        })));

    assert!(list_result.is_ok());

    let functions = list_result.unwrap().into_inner().functions;
    assert_eq!(1, functions.len());

    let list_result =
        futures::executor::block_on(fr.list_versions(tonic::Request::new(ListVersionsRequest {
            name: "my-name".to_owned(),
            version_requirement: "2.0.3-dev".to_owned(),
            limit: 100,
            offset: 0,
            ordering: Ordering::Descending as i32,
        })));

    assert!(list_result.is_ok());

    let functions = list_result.unwrap().into_inner().functions;
    assert_eq!(1, functions.len());

    let list_result =
        futures::executor::block_on(fr.list_versions(tonic::Request::new(ListVersionsRequest {
            name: "my-name".to_owned(),
            version_requirement: "8.1.4-dev".to_owned(),
            limit: 100,
            offset: 0,
            ordering: Ordering::Descending as i32,
        })));

    assert!(list_result.is_ok());

    let functions = list_result.unwrap().into_inner().functions;
    assert_eq!(1, functions.len());

    let latest_result = futures::executor::block_on(fr.get_latest_version(tonic::Request::new(
        GetLatestVersionRequest {
            name: "my-name".to_owned(),
            version_requirement: "2.0.3-dev".to_owned(),
        },
    )));

    assert!(latest_result.is_ok());

    let latest_result = futures::executor::block_on(fr.get_latest_version(tonic::Request::new(
        GetLatestVersionRequest {
            name: "my-name".to_owned(),
            version_requirement: "2.0.3".to_owned(),
        },
    )));

    assert!(latest_result.is_err());
    assert!(matches!(
        latest_result.unwrap_err().code(),
        tonic::Code::NotFound
    ));
}

#[test]
fn test_register_dev_version() {
    let fr = registry!();
    let register_result = futures::executor::block_on(fr.register(custom_register_request!(
        "my-name",
        "my-entrypoint",
        Some(ExecutionEnvironment {
            name: "wassaa".to_owned(),
        })
    )));

    assert!(register_result.is_ok());

    // make sure that the function registered above ends with "-dev"
    // because the local function registry should always append that
    let get_request = futures::executor::block_on(fr.get(tonic::Request::new(
        register_result.unwrap().into_inner().clone(),
    )));
    let f = get_request.unwrap().into_inner().function.unwrap();
    assert!(f.version.ends_with("-dev"));

    // make sure that registering another function with the same name and version gives us a new
    // id. Deleting functions instead of replacing guarantees that functions are immutable
    let first_id = f.id.unwrap();
    let register_result = futures::executor::block_on(fr.register(custom_register_request!(
        "my-name",
        "my-entrypoint",
        Some(ExecutionEnvironment {
            name: "wassaa".to_owned(),
        })
    )));

    assert!(first_id != register_result.unwrap().into_inner());

    // make sure that the first function we registered is actually deleted
    let get_request = futures::executor::block_on(fr.get(tonic::Request::new(first_id)));
    assert!(get_request.is_err());
    assert!(matches!(
        get_request.unwrap_err().code(),
        tonic::Code::NotFound
    ));
}
