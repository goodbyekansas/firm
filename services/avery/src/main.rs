
// magic to enable tonic prtobufs
pub mod proto {
    tonic::include_proto!("functions");
}

// standard includes

// 3rd party includes
use tonic::transport::Server; //, Request, Response, Status};

// crate / internal includes
use proto::functions_service_server::FunctionsService as FunctionsServiceProto;
use proto::functions_service_server::FunctionsServiceServer;
use proto::{ListRequest, ListResponse};


// define the FunctionsService struct
pub struct FunctionsService {}

// local methods to operate on a FunctionsService struct
impl FunctionsService {
    fn new() -> Self {
        Self {}
    }
}

// implementation of the grpc service trait (interface)
#[tonic::async_trait]
impl FunctionsServiceProto for FunctionsService {
    async fn list(
        &self,
        request: tonic::Request<ListRequest>,
    ) -> Result<tonic::Response<ListResponse>, tonic::Status> {
        Ok(tonic::Response::new(ListResponse{
            functions: Vec::new()
        }))
    }
}

// clean exit on crtl c
async fn ctrlc() {
    match tokio::signal::ctrl_c().await {
        _ => {}
    }
}

// local server main loop
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port: u32 = 8080;
    let addr = format!("0.0.0.0:{}", port).parse().unwrap();

    let function_service = FunctionsService::new();

    println!("The Firm is listening for requests on port {}", port);

    Server::builder()
        .add_service(FunctionsServiceServer::new(function_service))
        .serve_with_shutdown(addr, ctrlc())
        .await?;
    println!("👋 see you soon - no one leaves the Firm");
    Ok(())
}
