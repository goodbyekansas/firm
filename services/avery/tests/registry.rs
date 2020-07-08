use std::collections::HashMap;

use futures::{self, pin_mut};
use slog::o;
use url::Url;
use uuid::Uuid;

use gbk_protocols::{
    functions::{
        functions_registry_server::FunctionsRegistry, AttachmentStreamUpload, FunctionAttachmentId,
        FunctionId, ListRequest, OrderingDirection, OrderingKey,
    },
    tonic,
};

use gbk_protocols_test_helpers::{
    exec_env, list_request, register_attachment_request, register_request,
};

use avery::registry::FunctionsRegistryService;

macro_rules! null_logger {
    () => {{
        slog::Logger::root(slog::Discard, o!())
    }};
}

macro_rules! registry {
    () => {{
        FunctionsRegistryService::new(null_logger!())
    }};
}

#[test]
fn test_list_functions() {
    let fr = registry!();

    // Test empty
    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(list_request!())));

    assert!(list_request.is_ok());
    assert_eq!(0, list_request.unwrap().into_inner().functions.len());

    // Test with 3
    futures::executor::block_on(
        fr.register(tonic::Request::new(register_request!("random-1", "1.2.3"))),
    )
    .unwrap();
    futures::executor::block_on(fr.register(tonic::Request::new(register_request!(
        "function-1",
        "6.6.6"
    ))))
    .unwrap();
    futures::executor::block_on(fr.register(tonic::Request::new(register_request!(
        "function-dev-1",
        "100.100.100"
    ))))
    .unwrap();
    futures::executor::block_on(fr.register(tonic::Request::new(register_request!(
        "function-dev-2",
        "127.0.1"
    ))))
    .unwrap();

    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(list_request!())));

    assert!(list_request.is_ok());
    assert_eq!(4, list_request.unwrap().into_inner().functions.len());

    // Test filtering by name
    let list_request =
        futures::executor::block_on(fr.list(tonic::Request::new(list_request!("function"))));

    assert!(list_request.is_ok());
    assert_eq!(3, list_request.unwrap().into_inner().functions.len());

    let list_request =
        futures::executor::block_on(fr.list(tonic::Request::new(list_request!("dev"))));

    assert!(list_request.is_ok());
    assert_eq!(2, list_request.unwrap().into_inner().functions.len());

    let list_request =
        futures::executor::block_on(fr.list(tonic::Request::new(list_request!("dev-2"))));
    assert!(list_request.is_ok());
    assert_eq!(1, list_request.unwrap().into_inner().functions.len());
}

#[test]
fn test_list_metadata_filtering() {
    let fr = registry!();

    futures::executor::block_on(
        fr.register(tonic::Request::new(register_request!("random-1", "5.87.1"))),
    )
    .unwrap();
    futures::executor::block_on(fr.register(tonic::Request::new(register_request!(
        "matrix-1",
        "0.0.1",
        exec_env!(),
        {"a" => "neo", "b" => "smith"}
    ))))
    .unwrap();

    // Test filtering without filtering
    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(list_request!())));

    assert!(list_request.is_ok());
    assert_eq!(2, list_request.unwrap().into_inner().functions.len());

    // Test filtering with metadata
    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(
        list_request!("", {"a" => "neo", "b" => "smith"}),
    )));

    assert!(list_request.is_ok());
    let functions = list_request.unwrap().into_inner().functions;
    assert_eq!(1, functions.len());
    assert_eq!(
        "matrix-1",
        functions.first().unwrap().function.as_ref().unwrap().name
    );
}

#[test]
fn test_list_metadata_key_filtering() {
    let fr = registry!();

    futures::executor::block_on(
        fr.register(tonic::Request::new(register_request!("random-1", "5.87.1"))),
    )
    .unwrap();
    futures::executor::block_on(fr.register(tonic::Request::new(register_request!(
        "words",
        "0.1.0",
        exec_env!(),
        {"potato" => "foot", "fish" => "green"}
    ))))
    .unwrap();

    // Test filtering without filtering
    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(list_request!())));

    assert!(list_request.is_ok());
    assert_eq!(2, list_request.unwrap().into_inner().functions.len());

    // Test filtering with metadata
    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(list_request!(
        "",
        {},
        [format!("potato")]
    ))));

    assert!(list_request.is_ok());
    let functions = list_request.unwrap().into_inner().functions;
    assert_eq!(1, functions.len());
    assert_eq!(
        "words",
        functions.first().unwrap().function.as_ref().unwrap().name
    );

    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(list_request!(
        "",
        {},
        ["fish".to_owned()]
    ))));

    assert!(list_request.is_ok());
    let functions = list_request.unwrap().into_inner().functions;
    assert_eq!(1, functions.len());
    assert_eq!(
        "words",
        functions.first().unwrap().function.as_ref().unwrap().name
    );

    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(list_request!(
        "",
        {},
        ["foot".to_owned()]
    ))));

    assert!(list_request.is_ok());
    let functions = list_request.unwrap().into_inner().functions;
    assert_eq!(0, functions.len());
}

#[test]
fn test_offset_and_limit() {
    let fr = registry!();
    let count: usize = 10;

    for i in 0..count {
        futures::executor::block_on(fr.register(tonic::Request::new(register_request!(
            &format!("fn-{}", i),
            "1.1.1"
        ))))
        .unwrap();
    }

    let list_request =
        futures::executor::block_on(fr.list(tonic::Request::new(list_request!("", count * 2, {}))));

    assert!(list_request.is_ok());
    assert_eq!(count, list_request.unwrap().into_inner().functions.len());

    // do not take everything
    let limit: usize = 5;
    let list_request =
        futures::executor::block_on(fr.list(tonic::Request::new(list_request!("", limit, {}))));

    assert!(list_request.is_ok());
    assert_eq!(limit, list_request.unwrap().into_inner().functions.len());

    // Take last one
    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(list_request!(
        "",
        count,
        count - 1,
        {}
    ))));

    assert!(list_request.is_ok());
    assert_eq!(1, list_request.unwrap().into_inner().functions.len());
}

#[test]
fn test_sorting() {
    // yer a wizard harry
    let fr = registry!();

    futures::executor::block_on(
        fr.register(tonic::Request::new(register_request!("my-name-a", "1.0.0"))),
    )
    .unwrap();
    futures::executor::block_on(
        fr.register(tonic::Request::new(register_request!("my-name-a", "1.0.1"))),
    )
    .unwrap();
    futures::executor::block_on(
        fr.register(tonic::Request::new(register_request!("my-name-b", "1.0.2"))),
    )
    .unwrap();
    futures::executor::block_on(
        fr.register(tonic::Request::new(register_request!("my-name-c", "1.1.0"))),
    )
    .unwrap();

    // No filter specified
    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(list_request!())));

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
        metadata_filter: HashMap::new(),
        metadata_key_filter: vec![],
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
        metadata_filter: HashMap::new(),
        metadata_key_filter: vec![],
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
    futures::executor::block_on(
        fr.register(tonic::Request::new(register_request!("sune-a", "1.0.0"))),
    )
    .unwrap();
    futures::executor::block_on(
        fr.register(tonic::Request::new(register_request!("sune-a", "2.1.0"))),
    )
    .unwrap();
    futures::executor::block_on(
        fr.register(tonic::Request::new(register_request!("sune-b", "2.0.0"))),
    )
    .unwrap();
    futures::executor::block_on(
        fr.register(tonic::Request::new(register_request!("sune-b", "1.1.0"))),
    )
    .unwrap();
    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(ListRequest {
        name_filter: "sune".to_owned(),
        metadata_filter: HashMap::new(),
        metadata_key_filter: vec![],
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
    let f_id = futures::executor::block_on(
        fr.register(tonic::Request::new(register_request!("func", "7.7.7"))),
    )
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
    let register_result = futures::executor::block_on(fr.register(tonic::Request::new(
        register_request!("create-cake", "0.0.1", None),
    )));

    assert!(register_result.is_err());
    assert!(matches!(
        register_result.unwrap_err().code(),
        tonic::Code::InvalidArgument
    ));

    // Testing if we can register a valid function
    let register_result = futures::executor::block_on(fr.register(tonic::Request::new(
        register_request!("my-name", "0.2.111111", exec_env!()),
    )));
    assert!(register_result.is_ok());
}

#[test]
fn test_register_dev_version() {
    let fr = registry!();
    let register_result = futures::executor::block_on(fr.register(tonic::Request::new(
        register_request!("my-name", "0.1.2", exec_env!()),
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
    let register_result = futures::executor::block_on(fr.register(tonic::Request::new(
        register_request!("my-name", "0.1.2", exec_env!()),
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
    let code_result = futures::executor::block_on(
        fr.register_attachment(tonic::Request::new(register_attachment_request!("code"))),
    );
    assert!(code_result.is_ok());
    let code_attachment_id = code_result.unwrap().into_inner();

    let attachment1 = futures::executor::block_on(fr.register_attachment(tonic::Request::new(
        register_attachment_request!("attachment1"),
    )));
    assert!(attachment1.is_ok());
    let attachment1_id = attachment1.unwrap().into_inner();

    let attachment2 = futures::executor::block_on(fr.register_attachment(tonic::Request::new(
        register_attachment_request!(
            "attachment2",
            "21db76ad585e9a0c64e7e2cf2bbae937c3601c263ff9639061349468e9217585" // actual sha256 of sune * 5
        ),
    )));
    assert!(attachment2.is_ok());
    let attachment2_id = attachment2.unwrap().into_inner();

    let attachment2_id_clone = attachment2_id.clone();
    let outbound = async_stream::stream! {
        for _ in 0u32..5u32 {
            yield Ok(AttachmentStreamUpload {
                id: Some(attachment2_id_clone.clone()),
                content: "sune".as_bytes().to_vec(),
            });
        }
    };
    pin_mut!(outbound);
    let upload_result =
        futures::executor::block_on(fr.upload_stream_attachment(tonic::Request::new(outbound)));

    assert!(upload_result.is_ok());
    let rr = tonic::Request::new(register_request!(
        "name",
        "0.1.0",
        exec_env!(),
        Some(code_attachment_id.clone()),
        [attachment1_id.id, attachment2_id.id],
        {}
    ));

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
    let attach = function
        .attachments
        .iter()
        .find(|a| a.name == "attachment2");
    assert!(attach.is_some());

    let file_path = Url::parse(&attach.unwrap().url)
        .map(|url| url.path().to_owned())
        .unwrap();
    let file_content = std::fs::read(&file_path).unwrap();
    assert_eq!(file_content, "sunesunesunesunesune".as_bytes());

    // non-registered attachment
    let rr = tonic::Request::new(register_request!(
        "name",
        "0.1.0",
        exec_env!(),
        Some(FunctionAttachmentId {
            id: Uuid::new_v4().to_string(),
        }),
        ["not-a-valid-id", "c5b60066-ce2b-4168-b0af-8c0678112ec1"],
        {}
    ));

    let register_result = futures::executor::block_on(fr.register(rr));
    assert!(register_result.is_err());
    assert_eq!(
        register_result.unwrap_err().code(),
        tonic::Code::InvalidArgument
    );

    // test invalid checksum
    let invalid_attachment = futures::executor::block_on(fr.register_attachment(
        tonic::Request::new(register_attachment_request!(
            "invalid-attachment",
            "31db76ad585e9a0c64e7e2cf2bbae937c3601c263ff9639061349468e9217585" // not actual sha256 of sune * 5
        )),
    ));
    assert!(invalid_attachment.is_ok());
    let invalid_attachment_id = invalid_attachment.unwrap().into_inner();

    let invalid_attachment_id_clone = invalid_attachment_id.clone();
    let outbound = async_stream::stream! {
        for _ in 0u32..5u32 {
            yield Ok(AttachmentStreamUpload {
                id: Some(invalid_attachment_id_clone.clone()),
                content: "sune".as_bytes().to_vec(),
            });
        }
    };
    pin_mut!(outbound);
    let upload_result =
        futures::executor::block_on(fr.upload_stream_attachment(tonic::Request::new(outbound)));
    assert!(upload_result.is_err());
    assert!(matches!(
        upload_result.unwrap_err().code(),
        tonic::Code::InvalidArgument
    ));

    // test cleanup of registry
    assert!(std::path::Path::new(&file_path).exists());
    std::mem::drop(fr);
    assert!(!std::path::Path::new(&file_path).exists());
}
