use gbk_protocols::{
    functions::{functions_registry_server::FunctionsRegistry, AttachmentUpload},
    tonic,
};
use quinn::{config, registry::FunctionRegistryService};

use gbk_protocols_test_helpers::{exec_env, register_attachment_request, register_request};

macro_rules! null_logger {
    () => {{
        slog::Logger::root(slog::Discard, slog::o!())
    }};
}

macro_rules! registry_with_memory_storage {
    () => {{
        let mut config = config::Configuration::new(null_logger!()).unwrap();
        config.functions_storage_uri = "memory://".to_owned();
        futures::executor::block_on(FunctionRegistryService::new(config, null_logger!())).unwrap()
    }};
}

#[test]
fn register() {
    let reg_service = registry_with_memory_storage!();
    let request = tonic::Request::new(register_request!("random-1", "1.2.3"));
    assert!(futures::executor::block_on(reg_service.register(request)).is_ok());
}

#[test]
fn register_duplicate() {
    let reg_service = registry_with_memory_storage!();

    let name = "sune";
    let version = "122.13.155";

    let request = tonic::Request::new(register_request!(name, version));
    futures::executor::block_on(reg_service.register(request)).unwrap();

    let request = tonic::Request::new(register_request!(name, version));
    let r = futures::executor::block_on(reg_service.register(request));

    assert!(r.is_err());
    assert!(matches!(
        r.unwrap_err().code(),
        tonic::Code::InvalidArgument
    ));
}

#[test]
fn register_attachment() {
    let reg_service = registry_with_memory_storage!();
    let request = tonic::Request::new(register_attachment_request!("attackment"));

    let r = futures::executor::block_on(reg_service.register_attachment(request));
    assert!(r.is_ok());
    assert!(uuid::Uuid::parse_str(&r.unwrap().into_inner().id).is_ok());
}

#[test]
fn get_attachment_url() {
    let reg_service = registry_with_memory_storage!();
    let request = tonic::Request::new(register_attachment_request!("attackment"));

    let attachment_id = futures::executor::block_on(reg_service.register_attachment(request))
        .unwrap()
        .into_inner();

    futures::executor::block_on(reg_service.register(tonic::Request::new(
        register_request!("sune", "1.1.1", exec_env!(), None, [&attachment_id.id], {"banan" => "körbanan"})
    ))).unwrap();

    let res = futures::executor::block_on(reg_service.upload_attachment_url(tonic::Request::new(
        AttachmentUpload {
            id: Some(attachment_id),
        },
    )));
    assert!(res.is_ok());

    assert!(url::Url::parse(&res.unwrap().into_inner().url).is_ok());
}

#[cfg(feature = "TODO:PLS-REMOVE")]
#[test]
fn get_url_for_invalid_attachment() {
    let reg_service = registry_with_memory_storage!();
    let request = tonic::Request::new(register_attachment_request!("attackment"));

    let attachment_id = futures::executor::block_on(reg_service.register_attachment(request))
        .unwrap()
        .into_inner();

    let res = futures::executor::block_on(reg_service.upload_attachment_url(tonic::Request::new(
        AttachmentUpload {
            id: Some(attachment_id),
        },
    )));

    // Since the attachment doesn't belong to any function we cannot get the attachment url even though the attachment exists
    assert!(res.is_err());

    let res = futures::executor::block_on(reg_service.upload_attachment_url(tonic::Request::new(
        AttachmentUpload {
            id: Some(FunctionAttachmentId {
                id: "567017b1-4b6a-4549-b9d1-c348f04fb617".to_owned(),
            }),
        },
    )));
    // Get upload url of attachmenht that does not exist.
    assert!(res.is_err());
}
