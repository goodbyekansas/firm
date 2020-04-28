#![deny(warnings)]

// magic to enable tonic prtobufs
pub mod proto {
    tonic::include_proto!("functions");
}

mod executor;
pub mod registry;

use std::{collections::HashMap, sync::Arc};

use futures::future::{join_all, OptionFuture};
use slog::{o, Logger};

// crate / internal includes
use executor::{
    download_code, get_execution_env_inputs, lookup_executor, validate_args, validate_results,
};
use proto::functions_registry_server::FunctionsRegistry;
use proto::functions_server::Functions as FunctionsServiceTrait;
use proto::{
    execute_response::Result as ProtoResult, ExecuteRequest, ExecuteResponse, Function, FunctionId,
    ListRequest, ListResponse,
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
        let functions = join_all(
            self.functions_register
                .list(tonic::Request::new(request.into_inner()))
                .await?
                .into_inner()
                .functions
                .iter()
                .filter_map(|fd| {
                    let ee = fd.execution_environment.clone();
                    fd.function.clone().map(|mut f| async {
                        if let Ok(mut additional_inputs) = get_execution_env_inputs(
                            self.log.new(o!()),
                            &self.functions_register,
                            &ee.map(|ee| ee.name).unwrap_or_default(),
                        )
                        .await
                        .map_err(|e| {
                            tonic::Status::new(
                                tonic::Code::Internal,
                                format!(
                                    "Failed to resolve inputs for execution environment: {}",
                                    e
                                ),
                            )
                        }) {
                            f.inputs.append(&mut additional_inputs);
                        }
                        f
                    })
                }),
        )
        .await;

        Ok(tonic::Response::new(ListResponse { functions }))
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
        let ee = function_descriptor.execution_environment.clone();
        OptionFuture::from(function_descriptor.function.map(|mut f| async {
            if let Ok(mut additional_inputs) = get_execution_env_inputs(
                self.log.new(o!()),
                &self.functions_register,
                &ee.map(|ee| ee.name).unwrap_or_default(),
            )
            .await
            .map_err(|e| {
                tonic::Status::new(
                    tonic::Code::Internal,
                    format!("Failed to resolve inputs for execution environment: {}", e),
                )
            }) {
                f.inputs.append(&mut additional_inputs);
            }
            tonic::Response::new(f)
        }))
        .await
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
        let checksums = function_descriptor.clone().checksums.ok_or_else(|| {
            tonic::Status::new(
                tonic::Code::Internal,
                "Function descriptor did not contain any checksums.",
            )
        })?;

        // lookup executor and run
        let mut tags = HashMap::new();
        tags.insert("type".to_owned(), "execution-environment".to_owned());
        lookup_executor(
            self.log.new(o!()),
            &execution_environment.name,
            &self.functions_register,
        )
        .await
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
                &execution_environment.entrypoint,
                &code,
                &checksums,
                &execution_environment.args,
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
