use std::collections::HashMap;

use futures;
use slog::o;
use tonic::Request;
use uuid::Uuid;

use avery::proto::{
    functions_server::Functions as FunctionsTrait, ArgumentType, ExecuteRequest, Function,
    FunctionArgument, FunctionId, FunctionInput, FunctionOutput, ListRequest,
};
use avery::{FunctionDescriptor, FunctionsService};

macro_rules! null_logger {
    () => {{
        slog::Logger::root(slog::Discard, o!())
    }};
}

macro_rules! functions_service {
    () => {{
        FunctionsService::new(null_logger!(), Vec::new().iter())
    }};
}

macro_rules! functions_service_with_functions {
    () => {{
        FunctionsService::new(null_logger!(), fake_functions!().iter())
    }};
}

macro_rules! fake_functions {
    () => {{
        vec![FunctionDescriptor {
            id: Uuid::parse_str("ef394e5b-0b32-447d-b483-a34bcb70cbc0")
                .unwrap_or_else(|_| Uuid::new_v4()),
            execution_environment: "maya".to_owned(),
            code: Vec::new(),
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
        }]
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
    assert_eq!(1, fns.functions.len());
}

#[test]
fn test_get() {
    let svc2 = functions_service_with_functions!();
    let first_function_id = fake_functions!()
        .first()
        .unwrap()
        .function
        .id
        .clone()
        .unwrap();
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
    let svc = functions_service_with_functions!();

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
        function: fake_functions!().first().unwrap().function.id.clone(),
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
        function: fake_functions!().first().unwrap().function.id.clone(),
        arguments: incorrect_args,
    })));
    assert!(r.is_err());
}
