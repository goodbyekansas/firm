#![deny(warnings)]

mod executor;
pub mod registry;

use std::collections::HashMap;

use futures::future::{join_all, OptionFuture};
use slog::{o, Logger};

// crate / internal includes
use executor::{
    get_execution_env_inputs, lookup_executor, validate_args, validate_results, ExecutorContext,
    FunctionContextExt,
};

use registry::FunctionsRegistryService;

use gbk_protocols::{
    functions::{
        execute_response::Result as ProtoResult, functions_registry_server::FunctionsRegistry,
        functions_server::Functions as FunctionsServiceTrait, ExecuteRequest, ExecuteResponse,
        Function, FunctionContext, FunctionId, ListRequest, ListResponse,
    },
    tonic,
};

// define the FunctionsService struct
#[derive(Debug)]
pub struct FunctionsService {
    functions_register: FunctionsRegistryService,
    log: Logger,
}

// local methods to operate on a FunctionsService struct
impl FunctionsService {
    pub fn new(log: Logger, functions_register: FunctionsRegistryService) -> Self {
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

        // lookup executor and run
        let mut metadata = HashMap::new();
        metadata.insert("type".to_owned(), "execution-environment".to_owned());
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
            let res = executor.execute(
                ExecutorContext {
                    function_name: function.name.clone(),
                    entrypoint: execution_environment.entrypoint,
                    code: function_descriptor.code.clone(),
                    arguments: execution_environment.args,
                },
                FunctionContext::new(args, function_descriptor.attachments),
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
                    format!("Failed to execute function {}: {}", function.name, e),
                )),
            }
        })
    }
}
