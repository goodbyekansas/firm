use std::collections::HashMap;

use futures::{self, pin_mut};
use slog::o;
use url::Url;

use firm_types::{
    registry::{
        registry_server::Registry, AttachmentId, AttachmentStreamUpload, Filters, FunctionId,
        NameFilter, Ordering, OrderingKey,
    },
    tonic,
};

use firm_protocols_test_helpers::{attachment_data, filters, function_data, runtime};

use avery::registry::RegistryService as LocalRegistryService;

macro_rules! null_logger {
    () => {{
        slog::Logger::root(slog::Discard, o!())
    }};
}

macro_rules! registry {
    () => {{
        LocalRegistryService::new(null_logger!())
    }};
}

#[test]
fn test_list_functions() {
    let fr = registry!();

    // Test empty
    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(filters!())));

    assert!(list_request.is_ok());
    assert_eq!(0, list_request.unwrap().into_inner().functions.len());

    // Test with 3
    futures::executor::block_on(
        fr.register(tonic::Request::new(function_data!("random-1", "1.2.3"))),
    )
    .unwrap();
    futures::executor::block_on(
        fr.register(tonic::Request::new(function_data!("function-1", "6.6.6"))),
    )
    .unwrap();
    futures::executor::block_on(fr.register(tonic::Request::new(function_data!(
        "function-dev-1",
        "100.100.100"
    ))))
    .unwrap();
    futures::executor::block_on(fr.register(tonic::Request::new(function_data!(
        "function-dev-2",
        "127.0.1"
    ))))
    .unwrap();

    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(filters!())));

    assert!(list_request.is_ok());
    assert_eq!(4, list_request.unwrap().into_inner().functions.len());

    // Test filtering by name
    let list_request =
        futures::executor::block_on(fr.list(tonic::Request::new(filters!("function"))));

    assert!(list_request.is_ok());
    assert_eq!(3, list_request.unwrap().into_inner().functions.len());

    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(filters!("dev"))));

    assert!(list_request.is_ok());
    assert_eq!(2, list_request.unwrap().into_inner().functions.len());

    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(filters!("dev-2"))));
    assert!(list_request.is_ok());
    assert_eq!(1, list_request.unwrap().into_inner().functions.len());
}

#[test]
fn test_list_metadata_filtering() {
    let fr = registry!();

    futures::executor::block_on(
        fr.register(tonic::Request::new(function_data!("random-1", "5.87.1"))),
    )
    .unwrap();
    futures::executor::block_on(fr.register(tonic::Request::new(function_data!(
        "matrix-1",
        "0.0.1",
        runtime!(),
        {"a" => "neo", "b" => "smith"}
    ))))
    .unwrap();

    // Test filtering without filtering
    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(filters!())));

    assert!(list_request.is_ok());
    assert_eq!(2, list_request.unwrap().into_inner().functions.len());

    // Test filtering with metadata
    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(
        filters!("", {"a" => "neo", "b" => "smith"}),
    )));

    assert!(list_request.is_ok());
    let functions = list_request.unwrap().into_inner().functions;
    assert_eq!(1, functions.len());
    assert_eq!("matrix-1", functions.first().unwrap().name);
}

#[test]
fn test_list_metadata_key_filtering() {
    let fr = registry!();

    futures::executor::block_on(
        fr.register(tonic::Request::new(function_data!("random-1", "5.87.1"))),
    )
    .unwrap();
    futures::executor::block_on(fr.register(tonic::Request::new(function_data!(
        "words",
        "0.1.0",
        runtime!(),
        {"potato" => "foot", "fish" => "green"}
    ))))
    .unwrap();

    // Test filtering without filtering
    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(filters!())));

    assert!(list_request.is_ok());
    assert_eq!(2, list_request.unwrap().into_inner().functions.len());

    // Test filtering with metadata
    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(filters!(
        "",
        {},
        ["potato".to_owned()]
    ))));

    assert!(list_request.is_ok());
    let functions = list_request.unwrap().into_inner().functions;
    assert_eq!(1, functions.len());
    assert_eq!("words", functions.first().unwrap().name);

    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(filters!(
        "",
        {},
        ["fish".to_owned()]
    ))));

    assert!(list_request.is_ok());
    let functions = list_request.unwrap().into_inner().functions;
    assert_eq!(1, functions.len());
    assert_eq!("words", functions.first().unwrap().name);

    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(filters!(
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
        futures::executor::block_on(fr.register(tonic::Request::new(function_data!(
            &format!("fn-{}", i),
            "1.1.1"
        ))))
        .unwrap();
    }

    let list_request =
        futures::executor::block_on(fr.list(tonic::Request::new(filters!("", count * 2, {}))));

    assert!(list_request.is_ok());
    assert_eq!(count, list_request.unwrap().into_inner().functions.len());

    // do not take everything
    let limit: usize = 5;
    let list_request =
        futures::executor::block_on(fr.list(tonic::Request::new(filters!("", limit, {}))));

    assert!(list_request.is_ok());
    assert_eq!(limit, list_request.unwrap().into_inner().functions.len());

    // Take last one
    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(filters!(
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
        fr.register(tonic::Request::new(function_data!("my-name-a", "1.0.0"))),
    )
    .unwrap();
    futures::executor::block_on(
        fr.register(tonic::Request::new(function_data!("my-name-a", "1.0.1"))),
    )
    .unwrap();
    futures::executor::block_on(
        fr.register(tonic::Request::new(function_data!("my-name-b", "1.0.2"))),
    )
    .unwrap();
    futures::executor::block_on(
        fr.register(tonic::Request::new(function_data!("my-name-c", "1.1.0"))),
    )
    .unwrap();

    // No filter specified
    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(filters!())));

    assert!(list_request.is_ok());
    let functions = list_request.unwrap().into_inner().functions;
    assert_eq!(4, functions.len());
    assert_eq!("my-name-a", functions.first().unwrap().name);
    assert_eq!("1.0.1-dev", functions.first().unwrap().version);

    // Reverse version sorting
    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(Filters {
        name: Some(NameFilter {
            pattern: "my-name-a".to_owned(),
            exact_match: true,
        }),
        metadata: HashMap::new(),
        order: Some(Ordering {
            offset: 0,
            limit: 10,
            reverse: true,
            key: OrderingKey::NameVersion as i32,
        }),
        version_requirement: None,
    })));

    assert!(list_request.is_ok());
    let functions = list_request.unwrap().into_inner().functions;
    assert_eq!(2, functions.len());
    assert_eq!("1.0.0-dev", functions.first().unwrap().version);

    // testing swedish idioms
    let fr = registry!();
    futures::executor::block_on(
        fr.register(tonic::Request::new(function_data!("sune-a", "1.0.0"))),
    )
    .unwrap();
    futures::executor::block_on(
        fr.register(tonic::Request::new(function_data!("sune-a", "2.1.0"))),
    )
    .unwrap();
    futures::executor::block_on(
        fr.register(tonic::Request::new(function_data!("sune-b", "2.0.0"))),
    )
    .unwrap();
    futures::executor::block_on(
        fr.register(tonic::Request::new(function_data!("sune-b", "1.1.0"))),
    )
    .unwrap();
    let list_request = futures::executor::block_on(fr.list(tonic::Request::new(Filters {
        name: Some(NameFilter {
            pattern: "sune".to_owned(),
            exact_match: false,
        }),
        metadata: HashMap::new(),
        order: Some(Ordering {
            key: OrderingKey::NameVersion as i32,
            reverse: true,
            offset: 0,
            limit: 10,
        }),
        version_requirement: None,
    })));
    assert!(list_request.is_ok());
    let functions = list_request.unwrap().into_inner().functions;
    assert_eq!(4, functions.len());
    let first_function = functions.first().unwrap();
    assert_eq!("sune-b", first_function.name);
    assert_eq!("1.1.0-dev", first_function.version);
}

#[test]
fn test_get_function() {
    let fr = registry!();

    // Test non existant "id"
    let get_request = futures::executor::block_on(fr.get(tonic::Request::new(FunctionId {
        name: "kall-sune".to_owned(),
        version: "1.2.3".to_owned(),
    })));

    assert!(get_request.is_err());
    assert!(matches!(
        get_request.unwrap_err().code(),
        tonic::Code::NotFound
    ));

    // Test actually getting a function
    let f = futures::executor::block_on(
        fr.register(tonic::Request::new(function_data!("func", "7.7.7"))),
    )
    .unwrap()
    .into_inner();
    let get_request = futures::executor::block_on(fr.get(tonic::Request::new(FunctionId {
        name: f.name.clone(),
        version: f.version.clone(),
    })));
    assert!(get_request.is_ok());

    let function_from_get = get_request.unwrap().into_inner();
    assert_eq!(f.name, function_from_get.name);
    assert_eq!(f.version, function_from_get.version);
}

#[test]
fn test_register_function() {
    // Register a function missing execution environment
    let fr = registry!();
    let register_result = futures::executor::block_on(fr.register(tonic::Request::new(
        function_data!("create-cake", "0.0.1", None),
    )));

    assert!(register_result.is_err());
    assert!(matches!(
        register_result.unwrap_err().code(),
        tonic::Code::InvalidArgument
    ));

    // Testing if we can register a valid function
    let register_result = futures::executor::block_on(fr.register(tonic::Request::new(
        function_data!("my-name", "0.2.111111", runtime!()),
    )));
    assert!(register_result.is_ok());
}

#[test]
fn test_register_dev_version() {
    let fr = registry!();
    let register_result = futures::executor::block_on(fr.register(tonic::Request::new(
        function_data!("my-name", "0.1.2", runtime!()),
    )))
    .unwrap()
    .into_inner();

    // make sure that the function registered above ends with "-dev"
    // because the local function registry should always append that
    let get_request = futures::executor::block_on(fr.get(tonic::Request::new(FunctionId {
        name: register_result.name,
        version: register_result.version,
    })));
    let f = get_request.unwrap().into_inner();
    assert!(f.version.ends_with("-dev"));

    // Register a function with a pre-release and make sure it is preserved
    let fr = registry!();
    let register_result = futures::executor::block_on(fr.register(tonic::Request::new(
        function_data!("my-name", "0.1.2-kanin", runtime!()),
    )))
    .unwrap()
    .into_inner();

    let get_request = futures::executor::block_on(fr.get(tonic::Request::new(FunctionId {
        name: register_result.name,
        version: register_result.version,
    })));
    let f = get_request.unwrap().into_inner();
    assert!(f.version.ends_with("-kanin.dev"));
}

#[test]
fn test_attachments() {
    let fr = registry!();
    let code_result = futures::executor::block_on(
        fr.register_attachment(tonic::Request::new(attachment_data!("code"))),
    );
    assert!(code_result.is_ok());

    let attachment1 = futures::executor::block_on(
        fr.register_attachment(tonic::Request::new(attachment_data!("attachment1"))),
    );
    assert!(attachment1.is_ok());
    let attachment1_handle = attachment1.unwrap().into_inner();

    let attachment2 =
        futures::executor::block_on(fr.register_attachment(tonic::Request::new(attachment_data!(
            "attachment2",
            "21db76ad585e9a0c64e7e2cf2bbae937c3601c263ff9639061349468e9217585" // actual sha256 of sune * 5
        ))));
    assert!(attachment2.is_ok());
    let attachment2_id = attachment2.unwrap().into_inner().id;
    let attachment2_id_clone = attachment2_id.clone();

    let outbound = async_stream::stream! {
        for _ in 0u32..5u32 {
            yield Ok(AttachmentStreamUpload {
                id: attachment2_id_clone.clone(),
                content: b"sune".to_vec(),
            });
        }
    };
    pin_mut!(outbound);
    let upload_result =
        futures::executor::block_on(fr.upload_stream_attachment(tonic::Request::new(outbound)));

    assert!(upload_result.is_ok());

    let rr = tonic::Request::new(function_data!(
        "name",
        "0.1.0",
        runtime!(),
        code_result.unwrap().into_inner().id,
        [attachment1_handle.id.unwrap(), attachment2_id.unwrap()],
        {}
    ));

    let register_result = futures::executor::block_on(fr.register(rr));

    assert!(register_result.is_ok());
    let register_result = register_result.unwrap().into_inner();

    // confirm results are as expected from registration
    let function_result = futures::executor::block_on(fr.get(tonic::Request::new(FunctionId {
        name: register_result.name,
        version: register_result.version,
    })));

    assert!(function_result.is_ok());
    let function = function_result.unwrap().into_inner();
    let code = function.code.unwrap();
    assert_eq!(function.attachments.len(), 2);
    assert_eq!(code.name, "code");

    let code_url = Url::parse(&code.url.unwrap().url);
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

    let file_path = Url::parse(&attach.as_ref().unwrap().url.as_ref().unwrap().url)
        .map(|url| url.path().to_owned())
        .unwrap();
    let file_content = std::fs::read(&file_path).unwrap();
    assert_eq!(file_content, b"sunesunesunesunesune");

    // non-registered attachment
    let rr = tonic::Request::new(function_data!(
        "name",
        "0.1.0",
        runtime!(),
        Some(AttachmentId {
            uuid: uuid::Uuid::new_v4().to_string(),
        }),
        [
            AttachmentId {
                uuid: "not-a-valid-id".to_owned(),
            },
            AttachmentId {
                uuid: "c5b60066-ce2b-4168-b0af-8c0678112ec1".to_owned(),
            }
        ],
        {}
    ));

    let register_result = futures::executor::block_on(fr.register(rr));
    assert!(register_result.is_err());
    assert_eq!(
        register_result.unwrap_err().code(),
        tonic::Code::InvalidArgument
    );

    // test invalid checksum
    let invalid_attachment =
        futures::executor::block_on(fr.register_attachment(tonic::Request::new(attachment_data!(
            "invalid-attachment",
            "31db76ad585e9a0c64e7e2cf2bbae937c3601c263ff9639061349468e9217585" // not actual sha256 of sune * 5
        ))));
    assert!(invalid_attachment.is_ok());
    let invalid_attachment_id = invalid_attachment.unwrap().into_inner();

    let outbound = async_stream::stream! {
        for _ in 0u32..5u32 {
            yield Ok(AttachmentStreamUpload {
                id: invalid_attachment_id.id.clone(),
                content: b"sune".to_vec(),
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
