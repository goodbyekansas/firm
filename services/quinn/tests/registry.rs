use ::config::File as ConfigFile;

use firm_types::{
    functions::{registry_server::Registry, Filters, FunctionId, Ordering},
    tonic,
};
use quinn::{config, registry::RegistryService, storage::OrderingKey};

use firm_types::{attachment_data, filters, function_data, runtime_spec};
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
            ConfigFile::from_str(
                r#"
attachment_storage_uri = "https://attachment-issues.net/"
functions_storage_uri = "memory://"
"#,
                ::config::FileFormat::Toml,
            ),
        ))
        .unwrap();
        futures::executor::block_on(RegistryService::new(config, null_logger!())).unwrap()
    }};
}

#[test]
fn register() {
    let registry = registry_with_memory_storage!();
    let request = tonic::Request::new(function_data!("random-1", "1.2.3"));
    assert!(futures::executor::block_on(registry.register(request)).is_ok());
}

#[test]
fn register_duplicate() {
    let registry = registry_with_memory_storage!();

    let name = "sune";
    let version = "122.13.155";

    let request = tonic::Request::new(function_data!(name, version));
    futures::executor::block_on(registry.register(request)).unwrap();

    let request = tonic::Request::new(function_data!(name, version));
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
    let request = tonic::Request::new(attachment_data!("attackment"));

    let r = futures::executor::block_on(registry.register_attachment(request));
    assert!(r.is_ok());
    assert!(uuid::Uuid::parse_str(&r.unwrap().into_inner().id.unwrap().uuid).is_ok());
}

#[test]
fn get_function() {
    let registry = registry_with_memory_storage!();
    let function_name = "brandon-1".to_owned();
    let request = tonic::Request::new(function_data!(&function_name, "3.2.3"));
    let registered_function = futures::executor::block_on(registry.register(request))
        .unwrap()
        .into_inner();

    let registered_function_id = FunctionId {
        name: registered_function.name.clone(),
        version: registered_function.version.clone(),
    };
    assert_eq!(registered_function.name, function_name);

    let get_request = futures::executor::block_on(
        registry.get(tonic::Request::new(registered_function_id.clone())),
    );
    assert!(get_request.is_ok());
    let fun = get_request.unwrap().into_inner();
    assert_eq!(
        FunctionId {
            name: fun.name.clone(),
            version: fun.version
        },
        registered_function_id
    );
    assert_eq!(fun.name, function_name);
}

#[test]
fn list_function() {
    let registry = registry_with_memory_storage!();
    let request = tonic::Request::new(function_data!("tyler-1", "3.2.4"));
    let registered_function = futures::executor::block_on(registry.register(request))
        .unwrap()
        .into_inner();

    let list_response = futures::executor::block_on(registry.list(tonic::Request::new(filters!())));
    assert!(list_response.is_ok());
    let fun = list_response.unwrap().into_inner().functions.pop().unwrap();
    assert_eq!(
        FunctionId {
            name: fun.name,
            version: fun.version
        },
        FunctionId {
            name: registered_function.name,
            version: registered_function.version,
        }
    );
}

// Filtering
#[test]
fn list_functions() {
    let registry = registry_with_memory_storage!();

    // Test empty
    let list_request = futures::executor::block_on(registry.list(tonic::Request::new(filters!())));

    assert!(list_request.is_ok());
    assert_eq!(0, list_request.unwrap().into_inner().functions.len());

    // Test with 3
    futures::executor::block_on(
        registry.register(tonic::Request::new(function_data!("random-1", "1.2.3"))),
    )
    .unwrap();
    futures::executor::block_on(
        registry.register(tonic::Request::new(function_data!("function-1", "6.6.6"))),
    )
    .unwrap();
    futures::executor::block_on(registry.register(tonic::Request::new(function_data!(
        "function-dev-1",
        "100.100.100"
    ))))
    .unwrap();
    futures::executor::block_on(registry.register(tonic::Request::new(function_data!(
        "function-dev-2",
        "127.0.1"
    ))))
    .unwrap();

    let list_request = futures::executor::block_on(registry.list(tonic::Request::new(filters!())));

    assert!(list_request.is_ok());
    assert_eq!(4, list_request.unwrap().into_inner().functions.len());

    // Test filtering by name
    let list_request =
        futures::executor::block_on(registry.list(tonic::Request::new(filters!("function"))));

    assert!(list_request.is_ok());
    assert_eq!(3, list_request.unwrap().into_inner().functions.len());

    let list_request =
        futures::executor::block_on(registry.list(tonic::Request::new(filters!("dev"))));

    assert!(list_request.is_ok());
    assert_eq!(2, list_request.unwrap().into_inner().functions.len());

    let list_request =
        futures::executor::block_on(registry.list(tonic::Request::new(filters!("dev-2"))));
    assert!(list_request.is_ok());
    assert_eq!(1, list_request.unwrap().into_inner().functions.len());
}

#[test]
fn list_metadata_filtering() {
    let registry = registry_with_memory_storage!();

    futures::executor::block_on(
        registry.register(tonic::Request::new(function_data!("random-1", "5.87.1"))),
    )
    .unwrap();
    futures::executor::block_on(registry.register(tonic::Request::new(function_data!(
        "matrix-1",
        "0.0.1",
        runtime_spec!(),
        {"a" => "neo", "b" => "smith"}
    ))))
    .unwrap();

    // Test filtering without filtering
    let list_request = futures::executor::block_on(registry.list(tonic::Request::new(filters!())));

    assert!(list_request.is_ok());
    assert_eq!(2, list_request.unwrap().into_inner().functions.len());

    // Test filtering with metadata
    let list_request = futures::executor::block_on(registry.list(tonic::Request::new(
        filters!("", {"a" => "neo", "b" => "smith"}),
    )));

    assert!(list_request.is_ok());
    let functions = list_request.unwrap().into_inner().functions;
    assert_eq!(1, functions.len());
    assert_eq!("matrix-1", functions.first().unwrap().name);
}

#[test]
fn list_metadata_key_filtering() {
    let registry = registry_with_memory_storage!();

    futures::executor::block_on(
        registry.register(tonic::Request::new(function_data!("random-1", "5.87.1"))),
    )
    .unwrap();
    futures::executor::block_on(registry.register(tonic::Request::new(function_data!(
        "words",
        "0.1.0",
        runtime_spec!(),
        {"potato" => "foot", "fish" => "green"}
    ))))
    .unwrap();

    // Test filtering without filtering
    let list_request = futures::executor::block_on(registry.list(tonic::Request::new(filters!())));

    assert!(list_request.is_ok());
    assert_eq!(2, list_request.unwrap().into_inner().functions.len());

    // Test filtering with metadata
    let list_request = futures::executor::block_on(registry.list(tonic::Request::new(filters!(
        "",
        {},
        ["potato".to_owned()]
    ))));

    assert!(list_request.is_ok());
    let functions = list_request.unwrap().into_inner().functions;
    assert_eq!(1, functions.len());
    assert_eq!("words", functions.first().unwrap().name);

    let list_request = futures::executor::block_on(registry.list(tonic::Request::new(filters!(
        "",
        {},
        ["fish".to_owned()]
    ))));

    assert!(list_request.is_ok());
    let functions = list_request.unwrap().into_inner().functions;
    assert_eq!(1, functions.len());
    assert_eq!("words", functions.first().unwrap().name);

    let list_request = futures::executor::block_on(registry.list(tonic::Request::new(filters!(
        "",
        {},
        ["foot".to_owned()]
    ))));

    assert!(list_request.is_ok());
    let functions = list_request.unwrap().into_inner().functions;
    assert_eq!(0, functions.len());
}

#[test]
fn offset_and_limit() {
    let registry = registry_with_memory_storage!();
    let count: usize = 10;

    for i in 0..count {
        futures::executor::block_on(registry.register(tonic::Request::new(function_data!(
            &format!("fn-{}", i),
            "1.1.1"
        ))))
        .unwrap();
    }

    let list_request =
        futures::executor::block_on(
            registry.list(tonic::Request::new(filters!("", count * 2, {}))),
        );

    assert!(list_request.is_ok());
    assert_eq!(count, list_request.unwrap().into_inner().functions.len());

    // do not take everything
    let limit: usize = 5;
    let list_request =
        futures::executor::block_on(registry.list(tonic::Request::new(filters!("", limit, {}))));

    assert!(list_request.is_ok());
    assert_eq!(limit, list_request.unwrap().into_inner().functions.len());

    // Take last one
    let list_request = futures::executor::block_on(registry.list(tonic::Request::new(filters!(
        "",
        count,
        count - 1,
        {}
    ))));

    assert!(list_request.is_ok());
    assert_eq!(1, list_request.unwrap().into_inner().functions.len());
}

#[test]
fn sorting() {
    // yer a wizard harry
    let registry = registry_with_memory_storage!();
    // ($name:expr, $version:expr, $runtime_spec:expr, $code:expr, [$($attach:expr),*], {$($key:expr => $value:expr),*}, $email:expr)
    futures::executor::block_on(registry.register(tonic::Request::new(function_data!(
        "my-name-a",
        "1.0.0",
        runtime_spec!(),
        None,
        [],
        {},
        "legs.mcrunfast@people.com"
    ))))
    .unwrap();
    futures::executor::block_on(registry.register(tonic::Request::new(function_data!(
        "my-name-a",
        "1.0.1",
        runtime_spec!(),
        None,
        [],
        {},
        "legs.mcrunfast@people.com"
    ))))
    .unwrap();
    futures::executor::block_on(registry.register(tonic::Request::new(function_data!(
        "my-name-a",
        "1.0.2",
        runtime_spec!(),
        None,
        [],
        {},
        "slab.bulkhhead@people.com"
    ))))
    .unwrap();
    futures::executor::block_on(registry.register(tonic::Request::new(function_data!(
        "my-name-b",
        "1.0.2",
        runtime_spec!(),
        None,
        [],
        {},
        "legs.mcrunfast@people.com"
    ))))
    .unwrap();
    futures::executor::block_on(registry.register(tonic::Request::new(function_data!(
        "my-name-c",
        "1.1.0",
        runtime_spec!(),
        None,
        [],
        {},
        "slab.bulkhhead@people.com"
    ))))
    .unwrap();

    // No filter specified
    let list_request = futures::executor::block_on(registry.list(tonic::Request::new(filters!())));

    assert!(list_request.is_ok());
    let functions = list_request.unwrap().into_inner().functions;
    assert_eq!(
        3,
        functions.len(),
        "Expected to get 3/5 functions without filter since versions are grouped"
    );
    assert_eq!("my-name-a", functions.first().unwrap().name);
    assert_eq!(
        "1.0.2",
        functions.first().unwrap().version,
        "Expected 1.0.2 because that is the latest version by any publisher"
    );

    // reverse
    let list_request =
        futures::executor::block_on(registry.list_versions(tonic::Request::new(Filters {
            name: String::from("my-name-a"),
            metadata: HashMap::new(),
            order: Some(Ordering {
                offset: 0,
                limit: 10,
                reverse: true,
                key: OrderingKey::NameVersion as i32,
            }),

            version_requirement: None,
            publisher_email: String::from("legs.mcrunfast@people.com"),
        })));

    assert!(list_request.is_ok());
    let functions = list_request.unwrap().into_inner().functions;
    assert_eq!(
        2,
        functions.len(),
        "Legs McRunfast should have 2 functions in the registry called my-name-a"
    );
    assert_eq!(
        "1.0.0",
        functions.first().unwrap().version,
        "With reversed 1.0.0 is expected since it's Legs' first version"
    );

    // not reverse
    let list_request =
        futures::executor::block_on(registry.list_versions(tonic::Request::new(Filters {
            name: String::from("my-name-a"),
            metadata: HashMap::new(),
            order: Some(Ordering {
                offset: 0,
                limit: 10,
                reverse: false,
                key: OrderingKey::NameVersion as i32,
            }),

            version_requirement: None,
            publisher_email: String::from("legs.mcrunfast@people.com"),
        })));

    assert!(list_request.is_ok());
    let functions = list_request.unwrap().into_inner().functions;
    assert_eq!(
        2,
        functions.len(),
        "Legs McRunfast should have 2 functions in the registry called my-name-a"
    );
    assert_eq!(
        "1.0.1",
        functions.first().unwrap().version,
        "Without reversed 1.0.1 is expected since it's Legs' latest version"
    );

    // testing partial name (list) match and reverse
    let registry = registry_with_memory_storage!();
    futures::executor::block_on(registry.register(tonic::Request::new(function_data!(
        "sune-a",
        "1.0.0",
        runtime_spec!(),
        None,
        [],
        {},
        "smash.limpjaw@employee.com"
    ))))
    .unwrap();
    futures::executor::block_on(registry.register(tonic::Request::new(function_data!(
        "sune-a",
        "2.1.0",
        runtime_spec!(),
        None,
        [],
        {},
        "smash.limpjaw@employee.com"
    ))))
    .unwrap();
    futures::executor::block_on(registry.register(tonic::Request::new(function_data!(
        "sune-b",
        "1.1.0",
        runtime_spec!(),
        None,
        [],
        {},
        "smash.limpjaw@employee.com"
    ))))
    .unwrap();
    futures::executor::block_on(registry.register(tonic::Request::new(function_data!(
        "sune-b",
        "1.0.0",
        runtime_spec!(),
        None,
        [],
        {},
        "smash.limpjaw@employee.com"
    ))))
    .unwrap();
    futures::executor::block_on(registry.register(tonic::Request::new(function_data!(
        "sune-c",
        "7.0.0",
        runtime_spec!(),
        None,
        [],
        {},
        "brick.hardmeat@employee.com"
    ))))
    .unwrap();

    let list_request = futures::executor::block_on(registry.list(tonic::Request::new(Filters {
        name: String::from("sune"),
        metadata: HashMap::new(),
        order: Some(Ordering {
            offset: 0,
            limit: 10,
            reverse: true,
            key: OrderingKey::NameVersion as i32,
        }),

        version_requirement: None,
        publisher_email: String::from("smash.limpjaw@employee.com"),
    })));
    assert!(list_request.is_ok());
    let functions = list_request.unwrap().into_inner().functions;
    assert_eq!(
        2,
        functions.len(),
        "Expected 2/3 functions because Brick Hardmeat published the latest sune-c"
    );
    let first_function = functions.first().unwrap();
    assert_eq!(
        "sune-b", first_function.name,
        "Reversed sorting of functions should but sune-b at the front"
    );
    assert_eq!(
        "1.1.0", first_function.version,
        "Reversed sorting of functions should put 1.1.0 first"
    );
}
