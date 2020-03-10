#![deny(warnings)]

// magic to enable tonic prtobufs
pub mod proto {
    tonic::include_proto!("functions");
}
mod executor;

use std::collections::HashMap;

use slog::Logger;
use uuid::Uuid;

// crate / internal includes
use crate::executor::{lookup_executor, validate_args};
use proto::functions_server::Functions as FunctionsServiceTrait;
use proto::{ExecuteRequest, ExecuteResponse, Function, FunctionId, ListRequest, ListResponse};

#[derive(Debug, Default, Clone)]
pub struct FunctionDescriptor {
    pub execution_environment: String,
    pub code: Vec<u8>,
    pub id: Uuid,
    pub function: Function,
}

// define the FunctionsService struct
#[derive(Debug)]
pub struct FunctionsService {
    functions: HashMap<Uuid, FunctionDescriptor>,
    log: Logger,
}

// local methods to operate on a FunctionsService struct
impl FunctionsService {
    pub fn new<'a, I>(log: Logger, functions: I) -> Self
    where
        I: IntoIterator<Item = &'a FunctionDescriptor>,
    {
        Self {
            functions: functions.into_iter().map(|f| (f.id, f.clone())).collect(),
            log,
        }
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

    async fn get(
        &self,
        request: tonic::Request<FunctionId>,
    ) -> Result<tonic::Response<Function>, tonic::Status> {
        let fn_id = request.into_inner();
        Uuid::parse_str(&fn_id.value)
            .map_err(|e| {
                tonic::Status::new(
                    tonic::Code::InvalidArgument,
                    format!("failed to parse UUID from function id: {}", e),
                )
            })
            .and_then(|fun_uuid| {
                self.functions.get(&fun_uuid).ok_or_else(|| {
                    tonic::Status::new(
                        tonic::Code::NotFound,
                        format!("failed to find function with id: {}", fun_uuid),
                    )
                })
            })
            .map(|fd| tonic::Response::new(fd.function.clone()))
    }

    async fn execute(
        &self,
        request: tonic::Request<ExecuteRequest>,
    ) -> Result<tonic::Response<ExecuteResponse>, tonic::Status> {
        // lookup function
        let payload = request.into_inner();
        let function = payload
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
            .map_err(|e| tonic::Status::new(tonic::Code::InvalidArgument, e))?;

        // validate args
        validate_args(function.function.inputs.iter(), payload.arguments).map_err(|e| {
            tonic::Status::new(
                tonic::Code::InvalidArgument,
                format!(
                    "Invalid function arguments: {}",
                    e.iter()
                        .map(|ae| format!("{}", ae))
                        .collect::<Vec<String>>()
                        .join(", ")
                ),
            )
        })?;

        // lookup executor and run
        lookup_executor(function.execution_environment.as_str())
            .and_then(|executor| {
                Ok(tonic::Response::new(ExecuteResponse {
                    function: function.function.id.clone(),
                    result: Some(executor.execute(&function.code)),
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
