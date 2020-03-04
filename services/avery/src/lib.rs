#![deny(warnings)]

// magic to enable tonic prtobufs
pub mod proto {
    tonic::include_proto!("functions");
}
mod executor;

use std::collections::HashMap;

use uuid::Uuid;

// crate / internal includes
use crate::executor::lookup_executor;
use proto::functions_server::Functions as FunctionsServiceTrait;
use proto::{ExecuteRequest, ExecuteResponse, Function, FunctionId, ListRequest, ListResponse};

#[derive(Debug, Default)]
pub struct FunctionDescriptor {
    execution_environment: String,
    code: Vec<u8>,
    function: Function,
}

// define the FunctionsService struct
#[derive(Debug, Default)]
pub struct FunctionsService {
    functions: HashMap<Uuid, FunctionDescriptor>,
}

// local methods to operate on a FunctionsService struct
impl FunctionsService {
    pub fn new() -> Self {
        let mut functions = HashMap::new();
        let id = Uuid::new_v4();
        functions.insert(
            id,
            FunctionDescriptor {
                execution_environment: "maya".to_owned(),
                code: Vec::new(),
                function: Function {
                    id: Some(FunctionId {
                        value: id.to_string(),
                    }),
                    name: "hello_world".to_owned(),
                    tags: HashMap::with_capacity(0),
                    inputs: Vec::with_capacity(0),
                    outputs: Vec::with_capacity(0),
                },
            },
        );
        Self { functions }
    }
}

// implementation of the grpc service trait (interface)
#[tonic::async_trait]
impl FunctionsServiceTrait for FunctionsService {
    async fn list(
        &self,
        _request: tonic::Request<ListRequest>,
    ) -> Result<tonic::Response<ListResponse>, tonic::Status> {
        Ok(tonic::Response::new(ListResponse {
            functions: self
                .functions
                .values()
                .map(|fd| fd.function.clone())
                .collect(),
        }))
    }

    async fn execute(
        &self,
        request: tonic::Request<ExecuteRequest>,
    ) -> Result<tonic::Response<ExecuteResponse>, tonic::Status> {
        request
            .into_inner()
            .function
            .ok_or_else(|| String::from("function id is required to execute a function"))
            .and_then(|fun_id_str| {
                Uuid::parse_str(&fun_id_str.value)
                    .map_err(|e| format!("failed to parse UUID from function id: {}", e))
            })
            .and_then(|fun_id| {
                self.functions
                    .get(&fun_id)
                    .ok_or_else(|| format!("failed to find function with id {}", fun_id))
            })
            .and_then(
                |f| match (lookup_executor(f.execution_environment.as_str()), f) {
                    (Ok(ex), f) => Ok((ex, f)),
                    (Err(e), _) => Err(e),
                },
            )
            .and_then(|(executor, f)| {
                Ok(tonic::Response::new(ExecuteResponse {
                    function: f.function.id.clone(),
                    result: Some(executor.execute(&f.code)),
                }))
            })
            .map_err(|e| {
                tonic::Status::new(
                    tonic::Code::Internal,
                    format!("Failed to execute function: {}", e),
                )
            })
    }
}
