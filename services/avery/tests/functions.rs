use futures;
use tonic::Request;

use avery::proto::{functions_server::Functions as FunctionsTrait, ListRequest};
use avery::FunctionsService;

macro_rules! functions_service {
    () => {{
        FunctionsService::new()
    }};
}

#[test]
fn test_list_empty() {
    let svc = functions_service!();

    let r = futures::executor::block_on(svc.list(Request::new(ListRequest {})));
    assert!(r.is_ok());
    let fns = r.unwrap().into_inner();
    assert_eq!(fns.functions.len(), 0);
}
