use std::convert::{TryFrom, TryInto};

use firm_types::{registry::registry_server::Registry, tonic};
use futures::TryFutureExt;
use slog::{o, Logger};

use crate::{config, storage, storage_conversions::FunctionResolver};

pub struct RegistryService {
    function_storage: Box<dyn storage::FunctionStorage>,
    attachment_storage: Box<dyn storage::AttachmentStorage>,
}

impl RegistryService {
    pub async fn new(config: config::Configuration, log: Logger) -> Result<Self, String> {
        Ok(Self {
            function_storage: storage::create_storage(
                &config.functions_storage_uri,
                log.new(o!("storage" => "functions")),
            )
            .await
            .map_err(|e| format!("Failed to create storage backend: {}", e))?,
            attachment_storage: storage::create_attachment_storage(
                &config.attachment_storage_uri,
                log.new(o!("storage" => "attachments")),
            )
            .map_err(|e| format!("Failed to create attachment storage backed! {}", e))?,
        })
    }
}

#[tonic::async_trait]
impl Registry for RegistryService {
    async fn list(
        &self,
        request: tonic::Request<firm_types::registry::Filters>,
    ) -> Result<tonic::Response<firm_types::functions::Functions>, tonic::Status> {
        self.function_storage
            .list(&storage::Filters::try_from(request.into_inner())?)
            .and_then(|functions| async move {
                futures::future::try_join_all(functions.into_iter().map(|f| async move {
                    f.resolve_function(
                        self.function_storage.as_ref(),
                        self.attachment_storage.as_ref(),
                    )
                    .await
                }))
                .await
            })
            .map_ok(|functions| {
                tonic::Response::new(firm_types::functions::Functions { functions })
            })
            .map_err(|e| e.into())
            .await
    }
    async fn get(
        &self,
        request: tonic::Request<firm_types::registry::FunctionId>,
    ) -> Result<tonic::Response<firm_types::functions::Function>, tonic::Status> {
        self.function_storage
            .get(&request.into_inner().try_into()?)
            .map_err(|e| e.into())
            .and_then(|function| async move {
                function
                    .resolve_function(
                        self.function_storage.as_ref(),
                        self.attachment_storage.as_ref(),
                    )
                    .map_err(|e| e.into())
                    .await
            })
            .await
            .map(tonic::Response::new)
    }

    async fn register(
        &self,
        request: tonic::Request<firm_types::registry::FunctionData>,
    ) -> Result<tonic::Response<firm_types::functions::Function>, tonic::Status> {
        self.function_storage
            .insert(storage::Function::try_from(request.into_inner())?)
            .map_err(|se| se.into())
            .and_then(|function| async move {
                function
                    .resolve_function(
                        self.function_storage.as_ref(),
                        self.attachment_storage.as_ref(),
                    )
                    .map_err(|e| e.into())
                    .await
            })
            .await
            .map(tonic::Response::new)
    }
    async fn register_attachment(
        &self,
        request: tonic::Request<firm_types::registry::AttachmentData>,
    ) -> Result<tonic::Response<firm_types::registry::AttachmentHandle>, tonic::Status> {
        self.function_storage
            .insert_attachment(storage::FunctionAttachmentData::try_from(
                request.into_inner(),
            )?)
            .map_err(|se| se.into())
            .await
            .and_then(|attachment| {
                Ok(tonic::Response::new(
                    firm_types::registry::AttachmentHandle {
                        id: Some(firm_types::registry::AttachmentId {
                            uuid: attachment.id.to_string(),
                        }),
                        upload_url: Some(
                            self.attachment_storage
                                .get_upload_url(&attachment)
                                .map_err(tonic::Status::from)?,
                        ),
                    },
                ))
            })
    }
    async fn upload_streamed_attachment(
        &self,
        _request: tonic::Request<tonic::Streaming<firm_types::registry::AttachmentStreamUpload>>,
    ) -> Result<tonic::Response<firm_types::registry::Nothing>, tonic::Status> {
        Err(tonic::Status::new(
            tonic::Code::Unimplemented,
            "The Quinn registry does not support uploading via streaming upload, use URL instead."
                .to_owned(),
        ))
    }
}
