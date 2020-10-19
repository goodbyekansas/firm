use gbk_protocols::{
    functions::{
        functions_registry_server::FunctionsRegistry, AttachmentUpload, FunctionAttachmentId,
        ListRequest, OrderingDirection,
    },
    tonic,
};
use quinn::{config, registry::FunctionRegistryService, storage::OrderingKey};

use gbk_protocols_test_helpers::{
    exec_env, list_request, register_attachment_request, register_request,
};
use std::collections::HashMap;

macro_rules! null_logger {
    () => {{
        slog::Logger::root(slog::Discard, slog::o!())
    }};
}

macro_rules! registry_with_memory_storage {
    () => {{
        let config = futures::executor::block_on(config::Configuration::new_with_init(
            null_logger!(),
            |s| {
                s.set("functions_storage_uri", "memory://".to_owned())?;
                s.set(
                    "attachment_storage_uri",
                    "https://attachment-issues.net/".to_owned(),
                )
            },
        ))
        .unwrap();
        futures::executor::block_on(FunctionRegistryService::new(config, null_logger!())).unwrap()
    }};
}

#[test]
fn register() {
    let registry = registry_with_memory_storage!();
    let request = tonic::Request::new(register_request!("random-1", "1.2.3"));
    assert!(futures::executor::block_on(registry.register(request)).is_ok());
}

#[test]
fn register_duplicate() {
    let registry = registry_with_memory_storage!();

    let name = "sune";
    let version = "122.13.155";

    let request = tonic::Request::new(register_request!(name, version));
    futures::executor::block_on(registry.register(request)).unwrap();

    let request = tonic::Request::new(register_request!(name, version));
    let r = futures::executor::block_on(registry.register(request));

    assert!(r.is_err());
    assert!(matches!(
        r.unwrap_err().code(),
        tonic::Code::InvalidArgument
    ));
}

#[test]
fn register_attachment() {
    let registry = registry_with_memory_storage!();
    let request = tonic::Request::new(register_attachment_request!("attackment"));

    let r = futures::executor::block_on(registry.register_attachment(request));
    assert!(r.is_ok());
    assert!(uuid::Uuid::parse_str(&r.unwrap().into_inner().id).is_ok());
}

#[test]
fn get_attachment_url() {
    let registry = registry_with_memory_storage!();
    let request = tonic::Request::new(register_attachment_request!("attackment"));

    let attachment_id = futures::executor::block_on(registry.register_attachment(request))
        .unwrap()
        .into_inner();

    futures::executor::block_on(registry.register(tonic::Request::new(
        register_request!("sune", "1.1.1", exec_env!(), None, [&attachment_id.id], {"banan" => "körbanan"})
    ))).unwrap();

    let res = futures::executor::block_on(registry.upload_attachment_url(tonic::Request::new(
        AttachmentUpload {
            id: Some(attachment_id),
        },
    )));
    assert!(res.is_ok());

    assert!(url::Url::parse(&res.unwrap().into_inner().url).is_ok());
}

#[test]
fn get_function() {
    let registry = registry_with_memory_storage!();
    let function_name = "brandon-1".to_owned();
    let request = tonic::Request::new(register_request!(&function_name, "3.2.3"));
    let id = futures::executor::block_on(registry.register(request))
        .unwrap()
        .into_inner();

    let get_request = futures::executor::block_on(registry.get(tonic::Request::new(id.clone())));
    assert!(get_request.is_ok());
    let fun = get_request.unwrap().into_inner().function.unwrap();
    assert_eq!(fun.id, Some(id));
    assert_eq!(fun.name, function_name);
}

#[test]
fn get_url_for_invalid_attachment() {
    let registry = registry_with_memory_storage!();
    let res = futures::executor::block_on(registry.upload_attachment_url(tonic::Request::new(
        AttachmentUpload {
            id: Some(FunctionAttachmentId {
                id: "567017b1-4b6a-4549-b9d1-c348f04fb617".to_owned(),
            }),
        },
    )));

    // Get upload url of attachment that does not exist.
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().code(), tonic::Code::NotFound);
}

#[test]
fn list_function() {
    let registry = registry_with_memory_storage!();
    let function_name = "tyler-1".to_owned();
    let request = tonic::Request::new(register_request!(&function_name, "3.2.4"));
    let id = futures::executor::block_on(registry.register(request))
        .unwrap()
        .into_inner();

    let list_response =
        futures::executor::block_on(registry.list(tonic::Request::new(list_request!())));
    assert!(list_response.is_ok());
    let fun = list_response.unwrap().into_inner().functions.pop().unwrap();
    let function = fun.function.as_ref().unwrap();
    assert_eq!(function.id, Some(id));
    assert_eq!(function.name, function_name);
}

// Filtering
#[test]
fn list_functions() {
    let registry = registry_with_memory_storage!();

    // Test empty
    let list_request =
        futures::executor::block_on(registry.list(tonic::Request::new(list_request!())));

    assert!(list_request.is_ok());
    assert_eq!(0, list_request.unwrap().into_inner().functions.len());

    // Test with 3
    futures::executor::block_on(
        registry.register(tonic::Request::new(register_request!("random-1", "1.2.3"))),
    )
    .unwrap();
    futures::executor::block_on(registry.register(tonic::Request::new(register_request!(
        "function-1",
        "6.6.6"
    ))))
    .unwrap();
    futures::executor::block_on(registry.register(tonic::Request::new(register_request!(
        "function-dev-1",
        "100.100.100"
    ))))
    .unwrap();
    futures::executor::block_on(registry.register(tonic::Request::new(register_request!(
        "function-dev-2",
        "127.0.1"
    ))))
    .unwrap();

    let list_request =
        futures::executor::block_on(registry.list(tonic::Request::new(list_request!())));

    assert!(list_request.is_ok());
    assert_eq!(4, list_request.unwrap().into_inner().functions.len());

    // Test filtering by name
    let list_request =
        futures::executor::block_on(registry.list(tonic::Request::new(list_request!("function"))));

    assert!(list_request.is_ok());
    assert_eq!(3, list_request.unwrap().into_inner().functions.len());

    let list_request =
        futures::executor::block_on(registry.list(tonic::Request::new(list_request!("dev"))));

    assert!(list_request.is_ok());
    assert_eq!(2, list_request.unwrap().into_inner().functions.len());

    let list_request =
        futures::executor::block_on(registry.list(tonic::Request::new(list_request!("dev-2"))));
    assert!(list_request.is_ok());
    assert_eq!(1, list_request.unwrap().into_inner().functions.len());
}

#[test]
fn list_metadata_filtering() {
    let registry = registry_with_memory_storage!();

    futures::executor::block_on(
        registry.register(tonic::Request::new(register_request!("random-1", "5.87.1"))),
    )
    .unwrap();
    futures::executor::block_on(registry.register(tonic::Request::new(register_request!(
        "matrix-1",
        "0.0.1",
        exec_env!(),
        {"a" => "neo", "b" => "smith"}
    ))))
    .unwrap();

    // Test filtering without filtering
    let list_request =
        futures::executor::block_on(registry.list(tonic::Request::new(list_request!())));

    assert!(list_request.is_ok());
    assert_eq!(2, list_request.unwrap().into_inner().functions.len());

    // Test filtering with metadata
    let list_request = futures::executor::block_on(registry.list(tonic::Request::new(
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
fn list_metadata_key_filtering() {
    let registry = registry_with_memory_storage!();

    futures::executor::block_on(
        registry.register(tonic::Request::new(register_request!("random-1", "5.87.1"))),
    )
    .unwrap();
    futures::executor::block_on(registry.register(tonic::Request::new(register_request!(
        "words",
        "0.1.0",
        exec_env!(),
        {"potato" => "foot", "fish" => "green"}
    ))))
    .unwrap();

    // Test filtering without filtering
    let list_request =
        futures::executor::block_on(registry.list(tonic::Request::new(list_request!())));

    assert!(list_request.is_ok());
    assert_eq!(2, list_request.unwrap().into_inner().functions.len());

    // Test filtering with metadata
    let list_request = futures::executor::block_on(registry.list(tonic::Request::new(
        list_request!("", {}, ["potato".to_owned()]),
    )));

    assert!(list_request.is_ok());
    let functions = list_request.unwrap().into_inner().functions;
    assert_eq!(1, functions.len());
    assert_eq!(
        "words",
        functions.first().unwrap().function.as_ref().unwrap().name
    );

    let list_request = futures::executor::block_on(registry.list(tonic::Request::new(
        list_request!("", {}, ["fish".to_owned()]),
    )));

    assert!(list_request.is_ok());
    let functions = list_request.unwrap().into_inner().functions;
    assert_eq!(1, functions.len());
    assert_eq!(
        "words",
        functions.first().unwrap().function.as_ref().unwrap().name
    );

    let list_request = futures::executor::block_on(registry.list(tonic::Request::new(
        list_request!("", {}, ["foot".to_owned()]),
    )));

    assert!(list_request.is_ok());
    let functions = list_request.unwrap().into_inner().functions;
    assert_eq!(0, functions.len());
}

#[test]
fn offset_and_limit() {
    let registry = registry_with_memory_storage!();
    let count: usize = 10;

    for i in 0..count {
        futures::executor::block_on(registry.register(tonic::Request::new(register_request!(
            &format!("fn-{}", i),
            "1.1.1"
        ))))
        .unwrap();
    }

    let list_request = futures::executor::block_on(
        registry.list(tonic::Request::new(list_request!("", count * 2, {}))),
    );

    assert!(list_request.is_ok());
    assert_eq!(count, list_request.unwrap().into_inner().functions.len());

    // do not take everything
    let limit: usize = 5;
    let list_request = futures::executor::block_on(
        registry.list(tonic::Request::new(list_request!("", limit, {}))),
    );

    assert!(list_request.is_ok());
    assert_eq!(limit, list_request.unwrap().into_inner().functions.len());

    // Take last one
    let list_request = futures::executor::block_on(
        registry.list(tonic::Request::new(list_request!("", count, count - 1, {}))),
    );

    assert!(list_request.is_ok());
    assert_eq!(1, list_request.unwrap().into_inner().functions.len());
}

#[test]
fn sorting() {
    // yer a wizard harry
    let registry = registry_with_memory_storage!();

    futures::executor::block_on(
        registry.register(tonic::Request::new(register_request!("my-name-a", "1.0.0"))),
    )
    .unwrap();
    futures::executor::block_on(
        registry.register(tonic::Request::new(register_request!("my-name-a", "1.0.1"))),
    )
    .unwrap();
    futures::executor::block_on(
        registry.register(tonic::Request::new(register_request!("my-name-b", "1.0.2"))),
    )
    .unwrap();
    futures::executor::block_on(
        registry.register(tonic::Request::new(register_request!("my-name-c", "1.1.0"))),
    )
    .unwrap();

    // No filter specified
    let list_request =
        futures::executor::block_on(registry.list(tonic::Request::new(list_request!())));

    assert!(list_request.is_ok());
    let functions = list_request.unwrap().into_inner().functions;
    assert_eq!(4, functions.len());
    assert_eq!(
        "my-name-c",
        functions.first().unwrap().function.as_ref().unwrap().name
    );

    // Descending
    let list_request =
        futures::executor::block_on(registry.list(tonic::Request::new(ListRequest {
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
        "1.0.1",
        functions
            .first()
            .unwrap()
            .function
            .as_ref()
            .unwrap()
            .version
    );

    // Ascending
    let list_request =
        futures::executor::block_on(registry.list(tonic::Request::new(ListRequest {
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
        "1.0.1",
        functions
            .first()
            .unwrap()
            .function
            .as_ref()
            .unwrap()
            .version
    );

    // testing swedish idioms
    let registry = registry_with_memory_storage!();
    futures::executor::block_on(
        registry.register(tonic::Request::new(register_request!("sune-a", "1.0.0"))),
    )
    .unwrap();
    futures::executor::block_on(
        registry.register(tonic::Request::new(register_request!("sune-a", "2.1.0"))),
    )
    .unwrap();
    futures::executor::block_on(
        registry.register(tonic::Request::new(register_request!("sune-b", "2.0.0"))),
    )
    .unwrap();
    futures::executor::block_on(
        registry.register(tonic::Request::new(register_request!("sune-b", "1.1.0"))),
    )
    .unwrap();
    let list_request =
        futures::executor::block_on(registry.list(tonic::Request::new(ListRequest {
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
    assert_eq!("2.0.0", first_function.version);
}
