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
use proto::{
    ArgumentType, ExecuteRequest, ExecuteResponse, Function, FunctionId, FunctionInput,
    FunctionOutput, ListRequest, ListResponse,
};

#[derive(Debug, Default)]
pub struct FunctionDescriptor {
    execution_environment: String,
    code: Vec<u8>,
    function: Function,
}

// define the FunctionsService struct
#[derive(Debug)]
pub struct FunctionsService {
    functions: HashMap<Uuid, FunctionDescriptor>,
    log: Logger,
}

// local methods to operate on a FunctionsService struct
impl FunctionsService {
    pub fn new(log: Logger) -> Self {
        let mut functions = HashMap::new();
        let id = Uuid::parse_str("0c8c108c-bf61-4735-a86d-2d0f5b53561c")
            .unwrap_or_else(|_| Uuid::new_v4());
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

        let id = Uuid::parse_str("ef394e5b-0b32-447d-b483-a34bcb70cbc0")
            .unwrap_or_else(|_| Uuid::new_v4());
        functions.insert(
            id,
            FunctionDescriptor {
                execution_environment: "maya".to_owned(),
                code: Vec::new(),
                function: Function {
                    id: Some(FunctionId {
                        value: id.to_string(),
                    }),
                    name: "say_hello_yourself".to_owned(),
                    tags: HashMap::with_capacity(0),
                    inputs: vec![
                        FunctionInput {
                            name: "say".to_string(),
                            required: true,
                            r#type: ArgumentType::String as i32,
                            default_value: String::new(),
                        },
                        FunctionInput {
                            name: "count".to_string(),
                            required: false,
                            r#type: ArgumentType::Int as i32,
                            default_value: 1.to_string(),
                        },
                    ],
                    outputs: vec![FunctionOutput {
                        name: "output_string".to_string(),
                        r#type: ArgumentType::String as i32,
                    }],
                },
            },
        );

        Self { functions, log }
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
        validate_args(function.function.inputs.iter(), &payload.arguments).map_err(|e| {
            tonic::Status::new(
                tonic::Code::InvalidArgument,
                format!("Invalid function arguments: {}", e),
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
