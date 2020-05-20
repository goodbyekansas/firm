use std::collections::HashMap;

use futures::{self, pin_mut};
use url::Url;
use uuid::Uuid;

use gbk_protocols::{
    functions::{
        functions_registry_server::FunctionsRegistry, ArgumentType, AttachmentStreamUpload,
        Checksums, ExecutionEnvironment, FunctionAttachmentId, FunctionId, FunctionInput,
        FunctionOutput, ListRequest, OrderingDirection, OrderingKey, RegisterAttachmentRequest,
        RegisterRequest,
    },
    tonic,
};

use avery::registry::FunctionsRegistryService;

macro_rules! registry {
    () => {{
        FunctionsRegistryService::new()
    }};
}

macro_rules! register_request {
    ($name: expr) => {{
        let checksums = Some(Checksums {
            sha256: "724a8940e46ffa34e930258f708d890dbb3b3243361dfbc41eefcff124407a29".to_owned(),
        });
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
            checksums,
            execution_environment: Some(ExecutionEnvironment {
                name: "wasm".to_owned(),
                entrypoint: "kanske".to_owned(),
                args: vec![],
            }),
            attachment_ids: vec![],
        })
    }};
}

macro_rules! custom_register_request {
    ($name:expr, $execution_environment:expr) => {{
        tonic::Request::new(RegisterRequest {
            name: $name.to_owned(),
            version: "0.1.0".to_owned(),
            tags: HashMap::with_capacity(0),
            inputs: vec![],
            outputs: vec![],
            code: None,
            checksums: Some(Checksums {
                sha256: "724a8940e46ffa34e930258f708d890dbb3b3243361dfbc41eefcff124407a29"
                    .to_owned(),
            }),
            execution_environment: $execution_environment,
            attachment_ids: vec![],
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
            code: None,
            checksums: Some(Checksums {
                sha256: "724a8940e46ffa34e930258f708d890dbb3b3243361dfbc41eefcff124407a29"
                    .to_owned(),
            }),
            execution_environment: Some(ExecutionEnvironment {
                name: "wasm".to_owned(),
                entrypoint: "kanske".to_owned(),
                args: vec![],
            }),
            attachment_ids: vec![],
        })
    }};
}

macro_rules! register_request_with_tags {
    ($name: expr, $($key:expr => $value:expr),+) => {{

        let mut m = ::std::collections::HashMap::new();
            $(
                m.insert($key.to_owned(), $value.to_owned());
            )+

        tonic::Request::new(RegisterRequest {
            name: $name.to_owned(),
            version: "0.1.0".to_owned(),
            tags: m,
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
            checksums: Some(Checksums {
                sha256: "724a8940e46ffa34e930258f708d890dbb3b3243361dfbc41eefcff124407a29".to_owned(),
            }),
            execution_environment: Some(ExecutionEnvironment {
                name: "wasm".to_owned(),
                args: vec![],
                entrypoint: "kanske".to_owned(),
            }),
            attachment_ids: vec![],
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
        exact_name_match: false,
        version_requirement: None,
        order_direction: OrderingDirection::Descending as i32,
        order_by: OrderingKey::Name as i32,
    })));

    assert!(list_request.is_ok());
    assert_eq!(0, list_request.unwrap().into_inner().functions.len());

    // Test with 3
    futures::executor::block_on(fr.register(register_request!("random-1"))).unwrap();
    futures::executor::block_on(fr.register(register_request!("function-1"))).unwrap();
    futures::executor::block_on(fr.register(register_request!("function-dev-1"))).unwrap();
    futures::executor::block_on(fr.register(register_request!("function-dev-2"))).unwrap();

    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(ListRequest {
        name_filter: "".to_owned(),
        tags_filter: HashMap::new(),
        offset: 0,
        limit: 100,
        exact_name_match: false,
        version_requirement: None,
        order_direction: OrderingDirection::Descending as i32,
        order_by: OrderingKey::Name as i32,
    })));

    assert!(list_request.is_ok());
    assert_eq!(4, list_request.unwrap().into_inner().functions.len());

    // Test filtering by name
    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(ListRequest {
        name_filter: "function".to_owned(),
        tags_filter: HashMap::new(),
        offset: 0,
        limit: 100,
        exact_name_match: false,
        version_requirement: None,
        order_direction: OrderingDirection::Descending as i32,
        order_by: OrderingKey::Name as i32,
    })));

    assert!(list_request.is_ok());
    assert_eq!(3, list_request.unwrap().into_inner().functions.len());

    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(ListRequest {
        name_filter: "dev".to_owned(),
        tags_filter: HashMap::new(),
        offset: 0,
        limit: 100,
        exact_name_match: false,
        version_requirement: None,
        order_direction: OrderingDirection::Descending as i32,
        order_by: OrderingKey::Name as i32,
    })));

    assert!(list_request.is_ok());
    assert_eq!(2, list_request.unwrap().into_inner().functions.len());

    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(ListRequest {
        name_filter: "dev-2".to_owned(),
        tags_filter: HashMap::new(),
        offset: 0,
        limit: 100,
        exact_name_match: false,
        version_requirement: None,
        order_direction: OrderingDirection::Descending as i32,
        order_by: OrderingKey::Name as i32,
    })));
    assert!(list_request.is_ok());
    assert_eq!(1, list_request.unwrap().into_inner().functions.len());
}

#[test]
fn test_list_tag_filtering() {
    let fr = registry!();

    futures::executor::block_on(fr.register(register_request!("random-1"))).unwrap();
    futures::executor::block_on(fr.register(register_request_with_tags!(
        "matrix-1",
        "a" => "neo",
        "b" => "smith"
    )))
    .unwrap();

    // Test filtering without filtering
    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(ListRequest {
        name_filter: "".to_owned(),
        tags_filter: HashMap::new(),
        offset: 0,
        limit: 100,
        exact_name_match: false,
        version_requirement: None,
        order_direction: OrderingDirection::Descending as i32,
        order_by: OrderingKey::Name as i32,
    })));

    assert!(list_request.is_ok());
    assert_eq!(2, list_request.unwrap().into_inner().functions.len());

    // Test filtering with tags
    let mut tags_dict = HashMap::new();
    tags_dict.insert("a".to_owned(), "neo".to_owned());
    tags_dict.insert("b".to_owned(), "smith".to_owned());
    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(ListRequest {
        name_filter: "".to_owned(),
        tags_filter: tags_dict,
        offset: 0,
        limit: 100,
        exact_name_match: false,
        version_requirement: None,
        order_direction: OrderingDirection::Descending as i32,
        order_by: OrderingKey::Name as i32,
    })));

    assert!(list_request.is_ok());
    let functions = list_request.unwrap().into_inner().functions;
    assert_eq!(1, functions.len());
    assert_eq!(
        "matrix-1",
        functions.first().unwrap().function.as_ref().unwrap().name
    );
}

#[test]
fn test_offset_and_limit() {
    let fr = registry!();
    let count: usize = 10;

    for i in 0..count {
        futures::executor::block_on(fr.register(register_request!(&format!("fn-{}", i)))).unwrap();
    }

    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(ListRequest {
        name_filter: "".to_owned(),
        tags_filter: HashMap::new(),
        offset: 0,
        limit: (count * 2) as u32, // Limit above max
        exact_name_match: false,
        version_requirement: None,
        order_direction: OrderingDirection::Descending as i32,
        order_by: OrderingKey::Name as i32,
    })));

    assert!(list_request.is_ok());
    assert_eq!(count, list_request.unwrap().into_inner().functions.len());

    // do not take everything
    let limit: usize = 5;
    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(ListRequest {
        name_filter: "".to_owned(),
        tags_filter: HashMap::new(),
        offset: 0,
        limit: limit as u32,
        exact_name_match: false,
        version_requirement: None,
        order_direction: OrderingDirection::Descending as i32,
        order_by: OrderingKey::Name as i32,
    })));

    assert!(list_request.is_ok());
    assert_eq!(limit, list_request.unwrap().into_inner().functions.len());

    // Take last one
    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(ListRequest {
        name_filter: "".to_owned(),
        tags_filter: HashMap::new(),
        offset: (count - 1) as u32,
        limit: count as u32,
        exact_name_match: false,
        version_requirement: None,
        order_direction: OrderingDirection::Descending as i32,
        order_by: OrderingKey::Name as i32,
    })));

    assert!(list_request.is_ok());
    assert_eq!(1, list_request.unwrap().into_inner().functions.len());
}

#[test]
fn test_sorting() {
    // yer a wizard harry
    let fr = registry!();

    futures::executor::block_on(fr.register(register_request_with_version!("my-name-a", "1.0.0")))
        .unwrap();
    futures::executor::block_on(fr.register(register_request_with_version!("my-name-a", "1.0.1")))
        .unwrap();
    futures::executor::block_on(fr.register(register_request_with_version!("my-name-b", "1.0.2")))
        .unwrap();
    futures::executor::block_on(fr.register(register_request_with_version!("my-name-c", "1.1.0")))
        .unwrap();

    // No filter specified
    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(ListRequest {
        name_filter: "".to_owned(),
        tags_filter: HashMap::new(),
        offset: 0,
        limit: 10,
        exact_name_match: true,
        version_requirement: None,
        order_direction: OrderingDirection::Descending as i32,
        order_by: OrderingKey::Name as i32,
    })));

    assert!(list_request.is_ok());
    let functions = list_request.unwrap().into_inner().functions;
    assert_eq!(4, functions.len());
    assert_eq!(
        "my-name-c",
        functions.first().unwrap().function.as_ref().unwrap().name
    );

    // Descending
    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(ListRequest {
        name_filter: "my-name-a".to_owned(),
        tags_filter: HashMap::new(),
        offset: 0,
        limit: 10,
        exact_name_match: true,
        version_requirement: None,
        order_direction: OrderingDirection::Descending as i32,
        order_by: OrderingKey::Name as i32,
    })));

    assert!(list_request.is_ok());
    let functions = list_request.unwrap().into_inner().functions;
    assert_eq!(2, functions.len());
    assert_eq!(
        "1.0.1-dev",
        functions
            .first()
            .unwrap()
            .function
            .as_ref()
            .unwrap()
            .version
    );

    // Ascending
    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(ListRequest {
        name_filter: "my-name-a".to_owned(),
        tags_filter: HashMap::new(),
        offset: 0,
        limit: 10,
        exact_name_match: true,
        version_requirement: None,
        order_direction: OrderingDirection::Ascending as i32,
        order_by: OrderingKey::Name as i32,
    })));

    assert!(list_request.is_ok());
    let functions = list_request.unwrap().into_inner().functions;
    assert_eq!(2, functions.len());
    assert_eq!(
        "1.0.1-dev",
        functions
            .first()
            .unwrap()
            .function
            .as_ref()
            .unwrap()
            .version
    );

    // testing swedish idioms
    let fr = registry!();
    futures::executor::block_on(fr.register(register_request_with_version!("sune-a", "1.0.0")))
        .unwrap();
    futures::executor::block_on(fr.register(register_request_with_version!("sune-a", "2.1.0")))
        .unwrap();
    futures::executor::block_on(fr.register(register_request_with_version!("sune-b", "2.0.0")))
        .unwrap();
    futures::executor::block_on(fr.register(register_request_with_version!("sune-b", "1.1.0")))
        .unwrap();
    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(ListRequest {
        name_filter: "sune".to_owned(),
        tags_filter: HashMap::new(),
        offset: 0,
        limit: 10,
        exact_name_match: false,
        version_requirement: None,
        order_direction: OrderingDirection::Descending as i32,
        order_by: OrderingKey::Name as i32,
    })));
    assert!(list_request.is_ok());
    let functions = list_request.unwrap().into_inner().functions;
    assert_eq!(4, functions.len());
    let first_function = functions.first().unwrap().function.as_ref().unwrap();
    assert_eq!("sune-b", first_function.name);
    assert_eq!("2.0.0-dev", first_function.version);
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
    let register_result =
        futures::executor::block_on(fr.register(custom_register_request!("create-cake", None)));

    assert!(register_result.is_err());
    assert!(matches!(
        register_result.unwrap_err().code(),
        tonic::Code::InvalidArgument
    ));

    // Testing if we can register a valid function
    let register_result = futures::executor::block_on(fr.register(custom_register_request!(
        "my-name",
        Some(ExecutionEnvironment {
            name: "wassaa".to_owned(),
            entrypoint: "my-entrypoint".to_owned(),
            args: vec![],
        })
    )));
    assert!(register_result.is_ok());
}

#[test]
fn test_register_dev_version() {
    let fr = registry!();
    let register_result = futures::executor::block_on(fr.register(custom_register_request!(
        "my-name",
        Some(ExecutionEnvironment {
            name: "wassaa".to_owned(),
            entrypoint: "my-entrypoint".to_owned(),
            args: vec![],
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
        Some(ExecutionEnvironment {
            name: "wassaa".to_owned(),
            entrypoint: "my-entrypoint".to_owned(),
            args: vec![],
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

#[test]
fn test_attachments() {
    let fr = registry!();
    let code_result = futures::executor::block_on(fr.register_attachment(tonic::Request::new(
        RegisterAttachmentRequest {
            name: String::from("code"),
        },
    )));
    assert!(code_result.is_ok());
    let code_attachment_id = code_result.unwrap().into_inner();

    let attachment1 = futures::executor::block_on(fr.register_attachment(tonic::Request::new(
        RegisterAttachmentRequest {
            name: String::from("attachment1"),
        },
    )));
    assert!(attachment1.is_ok());
    let attachment1_id = attachment1.unwrap().into_inner();

    let attachment2 = futures::executor::block_on(fr.register_attachment(tonic::Request::new(
        RegisterAttachmentRequest {
            name: String::from("attachment2"),
        },
    )));
    assert!(attachment2.is_ok());
    let attachment2_id = attachment2.unwrap().into_inner();

    let attachment2_id_clone = attachment2_id.clone();
    let outbound = async_stream::stream! {
        for _ in 0u32..5u32 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            yield Ok(AttachmentStreamUpload {
                id: Some(attachment2_id_clone.clone()),
                content: "sune".as_bytes().to_vec(),
            });
        }
    };
    pin_mut!(outbound);
    let upload_result = futures::executor::block_on(fr.upload_stream_attachment(
        tonic::Request::new(outbound)
    ));

    assert!(upload_result.is_ok());

    let rr = tonic::Request::new(RegisterRequest {
        name: "name".to_owned(),
        version: "0.1.0".to_owned(),
        tags: HashMap::with_capacity(0),
        inputs: vec![],
        outputs: vec![],
        code: Some(code_attachment_id.clone()),
        checksums: Some(Checksums {
            sha256: "7767e3afca54296110dd596d8de7cd8adc6f89253beb3c69f0fc810df7f8b6d5".to_owned(),
        }),
        execution_environment: Some(ExecutionEnvironment {
            name: "wasm".to_owned(),
            entrypoint: "kanske".to_owned(),
            args: vec![],
        }),
        attachment_ids: vec![attachment1_id, attachment2_id],
    });

    let register_result = futures::executor::block_on(fr.register(rr));

    assert!(register_result.is_ok());

    // confirm results are as expected from registration
    let function_result = futures::executor::block_on(
        fr.get(tonic::Request::new(register_result.unwrap().into_inner())),
    );

    assert!(function_result.is_ok());
    let function = function_result.unwrap().into_inner();
    let code = function.code.unwrap();
    assert_eq!(function.attachments.len(), 2);
    assert_eq!(code.name, "code");
    assert_eq!(code.id, Some(code_attachment_id));

    let code_url = Url::parse(&code.url);
    assert!(code_url.is_ok());
    let code_url = code_url.unwrap();
    assert_eq!(code_url.scheme(), "file");
    assert!(std::path::Path::new(code_url.path()).exists());

    // Ensure content of attachment
    let attach = function.attachments.iter().find(|a| a.name == "attachment2");
    assert!(attach.is_some());

    let file_content = Url::parse(&attach.unwrap().url).ok().and_then(|url| {
        std::fs::read(url.path()).ok()
    }).unwrap();
    assert_eq!(file_content, "sunesunesunesunesune".as_bytes());

    // non-registered attachment
    let rr = tonic::Request::new(RegisterRequest {
        name: "name".to_owned(),
        version: "0.1.0".to_owned(),
        tags: HashMap::with_capacity(0),
        inputs: vec![],
        outputs: vec![],
        code: Some(FunctionAttachmentId {
            id: Uuid::new_v4().to_string(),
        }),
        checksums: Some(Checksums {
            sha256: "7767e3afca54296110dd596d8de7cd8adc6f89253beb3c69f0fc810df7f8b6d5".to_owned(),
        }),
        execution_environment: Some(ExecutionEnvironment {
            name: "wasm".to_owned(),
            entrypoint: "kanske".to_owned(),
            args: vec![],
        }),
        attachment_ids: vec![
            FunctionAttachmentId {
                id: Uuid::new_v4().to_string(),
            },
            FunctionAttachmentId {
                id: String::from("not-a-valid-id"),
            },
        ],
    });

    let register_result = futures::executor::block_on(fr.register(rr));
    assert!(register_result.is_err());
    assert_eq!(
        register_result.unwrap_err().code(),
        tonic::Code::InvalidArgument
    );
}
