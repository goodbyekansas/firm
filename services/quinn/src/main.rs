#![deny(warnings)]

use tonic::{transport::Server, Status};

use gbk_protocols::{
    functions::functions_registry_server::{FunctionsRegistry, FunctionsRegistryServer},
    tonic,
};

#[derive(Default)]
pub struct FunctionsRegistryService {}

#[tonic::async_trait]
impl FunctionsRegistry for FunctionsRegistryService {
    async fn list(
        &self,
        _request: tonic::Request<gbk_protocols::functions::ListRequest>,
    ) -> Result<tonic::Response<gbk_protocols::functions::RegistryListResponse>, tonic::Status>
    {
        Err(Status::new(tonic::Code::Unimplemented, "TODO"))
    }
    async fn get(
        &self,
        _request: tonic::Request<gbk_protocols::functions::FunctionId>,
    ) -> Result<tonic::Response<gbk_protocols::functions::FunctionDescriptor>, tonic::Status> {
        Err(Status::new(tonic::Code::Unimplemented, "TODO"))
    }
    async fn register(
        &self,
        _request: tonic::Request<gbk_protocols::functions::RegisterRequest>,
    ) -> Result<tonic::Response<gbk_protocols::functions::FunctionId>, tonic::Status> {
        Err(Status::new(tonic::Code::Unimplemented, "TODO"))
    }
    async fn register_attachment(
        &self,
        _request: tonic::Request<gbk_protocols::functions::RegisterAttachmentRequest>,
    ) -> Result<tonic::Response<gbk_protocols::functions::FunctionAttachmentId>, tonic::Status>
    {
        Err(Status::new(tonic::Code::Unimplemented, "TODO"))
    }
    async fn upload_streamed_attachment(
        &self,
        _request: tonic::Request<
            tonic::Streaming<gbk_protocols::functions::AttachmentStreamUpload>,
        >,
    ) -> Result<tonic::Response<gbk_protocols::functions::AttachmentUploadResponse>, tonic::Status>
    {
        Err(Status::new(tonic::Code::Unimplemented, "TODO"))
    }
    async fn upload_attachment_url(
        &self,
        _request: tonic::Request<gbk_protocols::functions::AttachmentUpload>,
    ) -> Result<tonic::Response<gbk_protocols::functions::AttachmentUploadResponse>, tonic::Status>
    {
        Err(Status::new(tonic::Code::Unimplemented, "TODO"))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "[::1]:50051".parse().unwrap();
    let svc = FunctionsRegistryService::default();

    println!("Quinn listening on {}", addr);

    Server::builder()
        .add_service(FunctionsRegistryServer::new(svc))
        .serve(addr)
        .await?;

    Ok(())
}
