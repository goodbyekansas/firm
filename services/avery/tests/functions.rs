use std::collections::HashMap;

use futures;
use slog::o;
use tonic::Request;

use avery::proto::{functions_server::Functions as FunctionsTrait, ListRequest};
use avery::FunctionsService;

macro_rules! null_logger {
    () => {{
        slog::Logger::root(slog::Discard, o!())
    }};
}

macro_rules! functions_service {
    () => {{
        FunctionsService::new(null_logger!())
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
    assert!(fns.functions.len() != 0);
}
