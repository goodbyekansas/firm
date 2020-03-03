// magic to enable tonic prtobufs
pub mod proto {
    tonic::include_proto!("functions");
}

// crate / internal includes
use proto::functions_server::Functions as FunctionsServiceTrait;
use proto::{ListRequest, ListResponse};

// define the FunctionsService struct
pub struct FunctionsService {}

// local methods to operate on a FunctionsService struct
impl FunctionsService {
    pub fn new() -> Self {
        Self {}
    }
}

// implementation of the grpc service trait (interface)
#[tonic::async_trait]
impl FunctionsServiceTrait for FunctionsService {
    async fn list(
        &self,
        request: tonic::Request<ListRequest>,
    ) -> Result<tonic::Response<ListResponse>, tonic::Status> {
        Ok(tonic::Response::new(ListResponse {
            functions: Vec::new(),
        }))
    }
}
