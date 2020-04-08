#![deny(warnings)]

// magic to enable tonic prtobufs
pub mod proto {
    tonic::include_proto!("functions");
}

mod executor;
pub mod registry;

use std::{collections::HashMap, sync::Arc};

use slog::{o, Logger};

// crate / internal includes
use executor::{download_code, lookup_executor, validate_args, validate_results};
use proto::functions_registry_server::FunctionsRegistry;
use proto::functions_server::Functions as FunctionsServiceTrait;
use proto::{
    execute_response::Result as ProtoResult, ExecuteRequest, ExecuteResponse, Function, FunctionId,
    GetLatestVersionRequest, ListRequest, ListResponse, OrderingDirection, OrderingKey,
};

// define the FunctionsService struct
#[derive(Debug)]
pub struct FunctionsService {
    functions_register: Arc<registry::FunctionsRegistryService>,
    log: Logger,
}

// local methods to operate on a FunctionsService struct
impl FunctionsService {
    pub fn new(log: Logger, functions_register: Arc<registry::FunctionsRegistryService>) -> Self {
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

    async fn get_latest_version(
        &self,
        request: tonic::Request<GetLatestVersionRequest>,
    ) -> Result<tonic::Response<Function>, tonic::Status> {
        let function_descriptor = self
            .functions_register
            .get_latest_version(tonic::Request::new(request.into_inner()))
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
        let mut tags = HashMap::new();
        tags.insert("type".to_owned(), "execution_environment".to_owned());
        // TODO: this needs to be done on demand which is slightly more complicated
        // but prob has nice perf benefits
        let available_executor_functions = self
            .functions_register
            .list(tonic::Request::new(ListRequest {
                name_filter: "".to_owned(),
                tags_filter: tags,
                offset: 0,
                limit: 100,
                exact_name_match: false,
                version_requirement: None,
                order_direction: OrderingDirection::Ascending as i32,
                order_by: OrderingKey::Name as i32,
            }))
            .await?;

        lookup_executor(
            self.log.new(o!()),
            &execution_environment.name,
            available_executor_functions
                .into_inner()
                .functions
                .as_slice(),
        )
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
                Ok(ProtoResult::Ok(r)) => validate_results(function.outputs.iter(), &r)
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
                Ok(ProtoResult::Error(e)) => Ok(tonic::Response::new(ExecuteResponse {
                    function: function.id.clone(),
                    result: Some(ProtoResult::Error(e)),
                })),

                Err(e) => Err(tonic::Status::new(
                    tonic::Code::Internal,
                    format!(
                        "Failed to execute function {}: {}",
                        function.name.clone(),
                        e
                    ),
                )),
            }
        })
    }
}
