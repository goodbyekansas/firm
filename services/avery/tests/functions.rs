use std::collections::HashMap;

use futures;
use slog::o;

use avery::{registry::FunctionsRegistryService, FunctionsService};
use gbk_protocols::{
    functions::{
        functions_registry_server::FunctionsRegistry,
        functions_server::Functions as FunctionsTrait, ArgumentType, Checksums, ExecuteRequest,
        ExecutionEnvironment, FunctionArgument, FunctionId, FunctionInput, FunctionOutput,
        RegisterAttachmentRequest, RegisterRequest,
    },
    tonic,
};

use gbk_protocols_test_helpers::{
    exec_env, function_input, function_output, list_request, register_request,
};

macro_rules! null_logger {
    () => {{
        slog::Logger::root(slog::Discard, o!())
    }};
}

macro_rules! functions_service {
    () => {{
        FunctionsService::new(
            null_logger!(),
            FunctionsRegistryService::new(null_logger!()),
        )
    }};
}

macro_rules! functions_service_with_functions {
    () => {{
        let functions_registry_service = FunctionsRegistryService::new(null_logger!());
        vec![
            RegisterRequest {
                name: "hello-world".to_owned(),
                version: "0.5.1-beta".to_owned(),
                tags: HashMap::with_capacity(0),
                inputs: Vec::with_capacity(0),
                outputs: Vec::with_capacity(0),
                code: None,
                execution_environment: Some(ExecutionEnvironment {
                    name: "wasm".to_owned(),
                    entrypoint: "det du!".to_owned(),
                    args: vec![],
                }),
                attachment_ids: vec![],
            },
            RegisterRequest {
                name: "say-hello-yourself".to_owned(),
                version: "1.6.8".to_owned(),
                tags: HashMap::with_capacity(0),
                inputs: vec![
                    FunctionInput {
                        name: "say".to_string(),
                        required: true,
                        r#type: ArgumentType::String as i32,
                        default_value: String::new(),
                        from_execution_environment: false,
                    },
                    FunctionInput {
                        name: "count".to_string(),
                        required: false,
                        r#type: ArgumentType::Int as i32,
                        default_value: 1.to_string(),
                        from_execution_environment: false,
                    },
                ],
                outputs: vec![FunctionOutput {
                    name: "output_string".to_string(),
                    r#type: ArgumentType::String as i32,
                }],
                code: None,
                execution_environment: Some(ExecutionEnvironment {
                    name: "wasm".to_owned(),
                    entrypoint: "kanske".to_owned(),
                    args: vec![],
                }),
                attachment_ids: vec![],
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
        FunctionsService::new(null_logger!(), functions_registry_service)
    }};
}

macro_rules! register_code_attachment {
    ($service:expr) => {{
        futures::executor::block_on(
            $service.register_attachment(tonic::Request::new(RegisterAttachmentRequest {
                name: "code".to_owned(),
                metadata: HashMap::new(),
                checksums: Some(Checksums {
                    sha256: "7767e3afca54296110dd596d8de7cd8adc6f89253beb3c69f0fc810df7f8b6d5"
                        .to_owned(),
                }),
            })),
        )
        .unwrap()
        .into_inner()
    }};
}

macro_rules! register_functions {
    ($service:expr, $fns:expr) => {{
        $fns.iter().for_each(|f| {
            futures::executor::block_on($service.register(tonic::Request::new(f.clone())))
                .map_or_else(
                    |e| println!("Failed to register function \"{}\". Err: {}", f.name, e),
                    |_| (),
                );
        });
        FunctionsService::new(null_logger!(), $service)
    }};
}

macro_rules! first_function {
    ($service:expr) => {{
        futures::executor::block_on($service.list(tonic::Request::new(list_request!())))
            .unwrap()
            .into_inner()
            .functions
            .first()
            .unwrap()
    }};
}

macro_rules! registry_service {
    () => {{
        FunctionsRegistryService::new(null_logger!())
    }};
}

#[test]
fn test_list() {
    let svc = functions_service!();

    let r = futures::executor::block_on(svc.list(tonic::Request::new(list_request!())));

    assert!(r.is_ok());
    let fns = r.unwrap().into_inner();
    assert_eq!(0, fns.functions.len());

    let svc2 = functions_service_with_functions!();
    let r = futures::executor::block_on(svc2.list(tonic::Request::new(list_request!())));
    assert!(r.is_ok());
    let fns = r.unwrap().into_inner();
    assert_eq!(2, fns.functions.len());
}

#[test]
fn test_get() {
    let svc2 = functions_service_with_functions!();
    let first_function_id = first_function!(svc2).id.clone().unwrap();
    let r = futures::executor::block_on(svc2.get(tonic::Request::new(first_function_id.clone())));
    assert!(r.is_ok());
    let f = r.unwrap().into_inner();
    assert_eq!(first_function_id, f.id.unwrap());

    let r = futures::executor::block_on(svc2.get(tonic::Request::new(FunctionId {
        value: "ef394e5b-0b32-447d-b483-a34bcb70cbc1".to_string(),
    })));
    assert!(r.is_err());
}

#[test]
fn test_execute() {
    let sr = registry_service!();
    let svc = register_functions!(
        sr,
        vec![register_request!(
            "say-hello-yourself",
            [
                function_input!("say", true, ArgumentType::String),
                function_input!("count", true, ArgumentType::Int)
            ],
            [function_output!("output_string", ArgumentType::String)],
            {},
            register_code_attachment!(sr),
            exec_env!("wasi")
        )]
    );

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
    let r = dbg!(futures::executor::block_on(svc.execute(
        tonic::Request::new(ExecuteRequest {
            function: first_function!(svc).id.clone(),
            arguments: correct_args,
        })
    )));
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

    let r = futures::executor::block_on(svc.execute(tonic::Request::new(ExecuteRequest {
        function: first_function!(svc).id.clone(),
        arguments: incorrect_args,
    })));
    assert!(r.is_err());
}

#[test]
fn test_execution_environment_inputs() {
    let sr = registry_service!();
    let svc = register_functions!(
        sr,
        vec![
            register_request!(
                "kalle-bula-execution-environment",
                [
                    function_input!("say", true, ArgumentType::String),
                    function_input!("count", false, ArgumentType::Int)
                ],
                [function_output!("feff", ArgumentType::String)],
                { "type" => "execution-environment", "execution-environment" => "kalle-bula"},
                register_code_attachment!(sr),
                exec_env!("wasi")
            ),
            register_request!(
                "jockes-dank-method",
                [function_input!("jockes-arg", true, ArgumentType::String)],
                [function_output!("hass_string", ArgumentType::String)],
                {},
                register_code_attachment!(sr),
                exec_env!("kalle-bula")
            )
        ]
    );

    let list_request = futures::executor::block_on(
        svc.list(tonic::Request::new(list_request!("jockes-dank-method"))),
    );

    assert!(list_request.is_ok());
    let res = list_request.unwrap().into_inner();
    let function = res.functions.first().unwrap();
    assert_eq!(1, function.outputs.len());
    assert_eq!("hass_string", &function.outputs.first().unwrap().name);
    assert_eq!(3, dbg!(&function.inputs).len());
    assert_eq!(
        1,
        function
            .inputs
            .iter()
            .filter(|i| !i.from_execution_environment)
            .collect::<Vec<_>>()
            .len()
    );
    assert_eq!(
        2,
        function
            .inputs
            .iter()
            .filter(|i| i.from_execution_environment)
            .collect::<Vec<_>>()
            .len()
    );

    // get should also give us execution environment inputs
    let fn_id = function.id.clone().unwrap().value;
    let res =
        futures::executor::block_on(svc.get(tonic::Request::new(FunctionId { value: fn_id })));
    assert!(res.is_ok());
    let function = res.unwrap().into_inner();
    assert_eq!(1, function.outputs.len());
    assert_eq!("hass_string", &function.outputs.first().unwrap().name);
    assert_eq!(3, function.inputs.len());
    assert_eq!(
        1,
        function
            .inputs
            .iter()
            .filter(|i| !i.from_execution_environment)
            .collect::<Vec<_>>()
            .len()
    );
    assert_eq!(
        2,
        function
            .inputs
            .iter()
            .filter(|i| i.from_execution_environment)
            .collect::<Vec<_>>()
            .len()
    );
}
