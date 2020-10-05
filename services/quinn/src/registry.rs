use std::convert::TryFrom;

use futures::TryFutureExt;
use gbk_protocols::{functions::functions_registry_server::FunctionsRegistry, tonic};
use slog::{o, Logger};
use tonic::Status;

use crate::{config, storage, storage_conversions::ToFunctionDescriptor};
use uuid::Uuid;

pub struct FunctionRegistryService {
    function_storage: Box<dyn storage::FunctionStorage>,
    attachment_storage: Box<dyn storage::AttachmentStorage>,
}

impl FunctionRegistryService {
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
impl FunctionsRegistry for FunctionRegistryService {
    async fn list(
        &self,
        request: tonic::Request<gbk_protocols::functions::ListRequest>,
    ) -> Result<tonic::Response<gbk_protocols::functions::RegistryListResponse>, tonic::Status>
    {
        self.function_storage
            .list(&storage::Filters::try_from(request.into_inner())?)
            .and_then(|functions| async move {
                futures::future::try_join_all(functions.into_iter().map(|f| async move {
                    f.to_function_descriptor(
                        self.function_storage.as_ref(),
                        self.attachment_storage.as_ref(),
                    )
                    .await
                }))
                .await
            })
            .map_ok(|func_descs| {
                tonic::Response::new(gbk_protocols::functions::RegistryListResponse {
                    functions: func_descs,
                })
            })
            .map_err(|e| e.into())
            .await
    }
    async fn get(
        &self,
        request: tonic::Request<gbk_protocols::functions::FunctionId>,
    ) -> Result<tonic::Response<gbk_protocols::functions::FunctionDescriptor>, tonic::Status> {
        futures::future::ready(Uuid::parse_str(&request.into_inner().value).map_err(|e| {
            Status::new(
                tonic::Code::InvalidArgument,
                format!("Failed to parse id to uuid: {}", e),
            )
        }))
        .and_then(|uuid| async move {
            self.function_storage
                .get(&uuid)
                .map_err(|e| e.into())
                .and_then(|function| async move {
                    function
                        .to_function_descriptor(
                            self.function_storage.as_ref(),
                            self.attachment_storage.as_ref(),
                        )
                        .map_err(|e| e.into())
                        .await
                })
                .await
        })
        .await
        .map(tonic::Response::new)
    }

    async fn register(
        &self,
        request: tonic::Request<gbk_protocols::functions::RegisterRequest>,
    ) -> Result<tonic::Response<gbk_protocols::functions::FunctionId>, tonic::Status> {
        self.function_storage
            .insert(storage::FunctionData::try_from(request.into_inner())?)
            .map_err(|se| se.into())
            .map_ok(|uuid| {
                tonic::Response::new(gbk_protocols::functions::FunctionId {
                    value: uuid.to_string(),
                })
            })
            .await
    }
    async fn register_attachment(
        &self,
        request: tonic::Request<gbk_protocols::functions::RegisterAttachmentRequest>,
    ) -> Result<tonic::Response<gbk_protocols::functions::FunctionAttachmentId>, tonic::Status>
    {
        self.function_storage
            .insert_attachment(storage::FunctionAttachmentData::try_from(
                request.into_inner(),
            )?)
            .map_err(|se| se.into())
            .map_ok(|uuid| {
                tonic::Response::new(gbk_protocols::functions::FunctionAttachmentId {
                    id: uuid.to_string(),
                })
            })
            .await
    }
    async fn upload_streamed_attachment(
        &self,
        _request: tonic::Request<
            tonic::Streaming<gbk_protocols::functions::AttachmentStreamUpload>,
        >,
    ) -> Result<tonic::Response<gbk_protocols::functions::AttachmentUploadResponse>, tonic::Status>
    {
        Err(tonic::Status::new(
            tonic::Code::Unimplemented,
            "The Quinn registry does not support uploading via streaming upload, use URL instead."
                .to_owned(),
        ))
    }
    async fn upload_attachment_url(
        &self,
        request: tonic::Request<gbk_protocols::functions::AttachmentUpload>,
    ) -> Result<tonic::Response<gbk_protocols::functions::AttachmentUploadResponse>, tonic::Status>
    {
        futures::future::ready(
            request
                .into_inner()
                .id
                .ok_or_else(|| {
                    tonic::Status::new(
                        tonic::Code::InvalidArgument,
                        "attachment id is required for obtaining upload url".to_owned(),
                    )
                })
                .and_then(|id| {
                    uuid::Uuid::parse_str(&id.id).map_err(|e| {
                        tonic::Status::new(
                            tonic::Code::InvalidArgument,
                            format!("Invalid uuid: {}: {}", &id.id, e),
                        )
                    })
                }),
        )
        .and_then(|id| async move {
            self.function_storage
                .get_attachment(&id)
                .and_then(|attachment| async move {
                    self.attachment_storage.get_upload_url(&attachment)
                })
                .map_err(|e| e.into())
                .await
        })
        .await
        .map(tonic::Response::new)
    }
}
