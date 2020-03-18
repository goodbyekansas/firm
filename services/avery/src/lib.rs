#![deny(warnings)]

// magic to enable tonic prtobufs
pub mod proto {
    tonic::include_proto!("functions");
}

mod executor;
pub mod fake_registry;
pub mod manifest;

use std::sync::Arc;

use slog::Logger;

// crate / internal includes
use executor::{download_code, lookup_executor, validate_args, validate_results};
use proto::functions_registry_server::FunctionsRegistry;
use proto::functions_server::Functions as FunctionsServiceTrait;
use proto::{
    execute_response::Result as ProtoResult, ExecuteRequest, ExecuteResponse, Function, FunctionId,
    ListRequest, ListResponse,
};

// define the FunctionsService struct
#[derive(Debug)]
pub struct FunctionsService {
    functions_register: Arc<fake_registry::FunctionsRegistryService>,
    log: Logger,
}

// local methods to operate on a FunctionsService struct
impl FunctionsService {
    pub fn new(
        log: Logger,
        functions_register: Arc<fake_registry::FunctionsRegistryService>,
    ) -> Self {
        Self {
            functions_register,
            log,
        }
    }
}

// implementation of the grpc service trait (interface)
#[tonic::async_trait]
impl FunctionsServiceTrait for FunctionsService {
    async fn list(
        &self,
        request: tonic::Request<ListRequest>,
    ) -> Result<tonic::Response<ListResponse>, tonic::Status> {
        let payload = self
            .functions_register
            .list(tonic::Request::new(request.into_inner()))
            .await?
            .into_inner();
        Ok(tonic::Response::new(ListResponse {
            functions: payload
                .functions
                .iter()
                .filter_map(|fd| fd.function.clone())
                .collect(),
        }))
    }

    async fn get(
        &self,
        request: tonic::Request<FunctionId>,
    ) -> Result<tonic::Response<Function>, tonic::Status> {
        let function_descriptor = self
            .functions_register
            .get(tonic::Request::new(request.into_inner()))
            .await?
            .into_inner();
        function_descriptor
            .function
            .map(tonic::Response::new)
            .ok_or_else(|| {
                tonic::Status::new(
                    tonic::Code::Internal,
                    "Function descriptor did not contain any function.",
                )
            })
    }

    async fn execute(
        &self,
        request: tonic::Request<ExecuteRequest>,
    ) -> Result<tonic::Response<ExecuteResponse>, tonic::Status> {
        // lookup function
        let payload = request.into_inner();
        let args = payload.arguments;
        let function_descriptor = payload
            .function
            .ok_or_else(|| String::from("function id is required to execute a function"))
            .map(|fun_id| self.functions_register.get(tonic::Request::new(fun_id)))
            .map_err(|e| tonic::Status::new(tonic::Code::InvalidArgument, e))?
            .await?
            .into_inner();

        let function = function_descriptor.clone().function.ok_or_else(|| {
            tonic::Status::new(
                tonic::Code::Internal,
                "Function descriptor did not contain any function.",
            )
        })?;

        // validate args
        validate_args(function.inputs.iter(), &args).map_err(|e| {
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

        let execution_environment = function_descriptor
            .clone()
            .execution_environment
            .ok_or_else(|| {
                tonic::Status::new(
                    tonic::Code::Internal,
                    "Function descriptor did not contain any execution environment.",
                )
            })?;

        // lookup executor and run
        lookup_executor(&execution_environment.name)
            .map_err(|e| {
                tonic::Status::new(
                    tonic::Code::Internal,
                    format!("Failed to lookup function executor: {}", e),
                )
            })
            .and_then(|executor| {
                // not having any code for the function is a valid case used for example to execute
                // external functions (gcp, aws lambdas, etc)
                let code = if function_descriptor.code_url.is_empty() {
                    Ok(vec![])
                } else {
                    download_code(&function_descriptor.code_url).map_err(|e| {
                        tonic::Status::new(
                            tonic::Code::Internal,
                            format!(
                                "Failed to download code 🖨️ for function \"{}\" from {}: {}",
                                function.name.clone(),
                                &function_descriptor.code_url,
                                e
                            ),
                        )
                    })
                }?;
                let res = executor.execute(
                    &function.name,
                    &function_descriptor.entrypoint,
                    &code,
                    &args,
                );
                match res {
                    ProtoResult::Ok(r) => validate_results(function.outputs.iter(), &r)
                        .map(|_| {
                            tonic::Response::new(ExecuteResponse {
                                function: function.id.clone(),
                                result: Some(ProtoResult::Ok(r)),
                            })
                        })
                        .map_err(|e| {
                            tonic::Status::new(
                                tonic::Code::InvalidArgument,
                                format!(
                                    "Function \"{}\" generated invalid result: {}",
                                    function.name.clone(),
                                    e.iter()
                                        .map(|ae| format!("{}", ae))
                                        .collect::<Vec<String>>()
                                        .join(", ")
                                ),
                            )
                        }),
                    ProtoResult::Error(e) => Ok(tonic::Response::new(ExecuteResponse {
                        function: function.id.clone(),
                        result: Some(ProtoResult::Error(e)),
                    })),
                }
            })
    }
}
