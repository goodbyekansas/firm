use std::{collections::HashMap, sync::Arc};

use futures;
use slog::o;
use tonic::Request;

use avery::{
    fake_registry::FunctionsRegistryService,
    proto::{
        functions_registry_server::FunctionsRegistry,
        functions_server::Functions as FunctionsTrait, ArgumentType, ExecuteRequest,
        ExecutionEnvironment, FunctionArgument, FunctionId, FunctionInput, FunctionOutput,
        ListRequest, RegisterRequest,
    },
    FunctionsService,
};

macro_rules! null_logger {
    () => {{
        slog::Logger::root(slog::Discard, o!())
    }};
}

macro_rules! functions_service {
    () => {{
        FunctionsService::new(null_logger!(), Arc::new(FunctionsRegistryService::new()))
    }};
}

macro_rules! functions_service_with_functions {
    () => {{
        let functions_registry_service = Arc::new(FunctionsRegistryService::new());
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
        FunctionsService::new(null_logger!(), Arc::clone(&functions_registry_service))
    }};
}

macro_rules! functions_service_with_specified_functions {
    ($fns:expr) => {{
        let functions_registry_service = Arc::new(FunctionsRegistryService::new());
        $fns.iter().for_each(|f| {
            futures::executor::block_on(
                functions_registry_service.register(tonic::Request::new(f.clone())),
            )
            .map_or_else(
                |e| println!("Failed to register function \"{}\". Err: {}", f.name, e),
                |_| (),
            );
        });
        FunctionsService::new(null_logger!(), Arc::clone(&functions_registry_service))
    }};
}

macro_rules! first_function {
    ($service:expr) => {{
        futures::executor::block_on($service.list(Request::new(ListRequest {
            name_filter: String::from(""),
            tags_filter: HashMap::new(),
            offset: 0,
            limit: 100,
        })))
        .unwrap()
        .into_inner()
        .functions
        .first()
        .unwrap()
    }};
}

#[test]
fn test_list() {
    let svc = functions_service!();

    let r = futures::executor::block_on(svc.list(Request::new(ListRequest {
        name_filter: String::from(""),
        tags_filter: HashMap::new(),
        offset: 0,
        limit: 100,
    })));

    assert!(r.is_ok());
    let fns = r.unwrap().into_inner();
    assert_eq!(0, fns.functions.len());

    let svc2 = functions_service_with_functions!();
    let r = futures::executor::block_on(svc2.list(Request::new(ListRequest {
        name_filter: String::from(""),
        tags_filter: HashMap::new(),
        offset: 0,
        limit: 100,
    })));
    assert!(r.is_ok());
    let fns = r.unwrap().into_inner();
    assert_eq!(2, fns.functions.len());
}

#[test]
fn test_get() {
    let svc2 = functions_service_with_functions!();
    let first_function_id = first_function!(svc2).id.clone().unwrap();
    let r = futures::executor::block_on(svc2.get(Request::new(first_function_id.clone())));
    assert!(r.is_ok());
    let f = r.unwrap().into_inner();
    assert_eq!(first_function_id, f.id.unwrap());

    let r = futures::executor::block_on(svc2.get(Request::new(FunctionId {
        value: "ef394e5b-0b32-447d-b483-a34bcb70cbc1".to_string(),
    })));
    assert!(r.is_err());
}

#[test]
fn test_execute() {
    let svc = functions_service_with_specified_functions!(vec![RegisterRequest {
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
    }]);

    let correct_args = vec![
        FunctionArgument {
            name: "say".to_owned(),
            r#type: ArgumentType::String as i32,
            value: "sune".as_bytes().to_vec(),
        },
        FunctionArgument {
            name: "count".to_owned(),
            r#type: ArgumentType::Int as i32,
            value: 3i64.to_le_bytes().to_vec(),
        },
    ];
    let r = futures::executor::block_on(svc.execute(Request::new(ExecuteRequest {
        function: first_function!(svc).id.clone(),
        arguments: correct_args,
    })));
    assert!(r.is_ok());

    let incorrect_args = vec![
        FunctionArgument {
            name: "say".to_owned(),
            r#type: ArgumentType::String as i32,
            value: 3i64.to_le_bytes().to_vec(),
        },
        FunctionArgument {
            name: "count".to_owned(),
            r#type: ArgumentType::Int as i32,
            value: "sune".as_bytes().to_vec(),
        },
    ];

    let r = futures::executor::block_on(svc.execute(Request::new(ExecuteRequest {
        function: first_function!(svc).id.clone(),
        arguments: incorrect_args,
    })));
    assert!(r.is_err());
}
