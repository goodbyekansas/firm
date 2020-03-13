use std::{
    collections::HashMap,
    fs,
    sync::{Arc, RwLock},
};

use tempfile::NamedTempFile;
use tonic;
use uuid::Uuid;

use crate::proto::functions_registry_server::FunctionsRegistry;
use crate::proto::{
    Function, FunctionDescriptor, FunctionId, ListRequest, RegisterRequest, RegistryListResponse,
};

#[derive(Debug, Default)]
pub struct FunctionsRegistryService {
    functions: Arc<RwLock<HashMap<Uuid, FunctionDescriptor>>>,
}

#[tonic::async_trait]
impl FunctionsRegistry for FunctionsRegistryService {
    async fn list(
        &self,
        _list_request: tonic::Request<ListRequest>,
    ) -> Result<tonic::Response<RegistryListResponse>, tonic::Status> {
        let reader = self.functions.read().map_err(|e| {
            tonic::Status::new(
                tonic::Code::Internal,
                format!("Failed to get read lock for functions: {}", e),
            )
        })?;

        Ok(tonic::Response::new(RegistryListResponse {
            functions: reader.values().cloned().collect(),
        }))
    }

    async fn get(
        &self,
        function_id_request: tonic::Request<FunctionId>,
    ) -> Result<tonic::Response<FunctionDescriptor>, tonic::Status> {
        let fn_id = function_id_request.into_inner();

        let reader = self.functions.read().map_err(|e| {
            tonic::Status::new(
                tonic::Code::Internal,
                format!("Failed to get read lock for functions: {}", e),
            )
        })?;

        Uuid::parse_str(&fn_id.value)
            .map_err(|e| {
                tonic::Status::new(
                    tonic::Code::InvalidArgument,
                    format!("failed to parse UUID from function id: {}", e),
                )
            })
            .and_then(|fun_uuid| {
                reader.get(&fun_uuid).ok_or_else(|| {
                    tonic::Status::new(
                        tonic::Code::NotFound,
                        format!("failed to find function with id: {}", fun_uuid),
                    )
                })
            })
            .map(|fd| tonic::Response::new(fd.clone()))
    }

    async fn register(
        &self,
        register_request: tonic::Request<RegisterRequest>,
    ) -> Result<tonic::Response<FunctionId>, tonic::Status> {
        // TODO: We should have checks for having valid names. Just to make sure we can make nice urls and such
        let id = Uuid::new_v4();
        let payload = register_request.into_inner();
        let ee = payload.execution_environment.ok_or_else(|| {
            tonic::Status::new(
                tonic::Code::InvalidArgument,
                String::from("Execution environment is required when registering function"),
            )
        })?;

        let mut writer = self.functions.write().map_err(|e| {
            tonic::Status::new(
                tonic::Code::Internal,
                format!("Failed to get write lock for functions: {}", e),
            )
        })?;

        // TODO: A better storage mechanism _will_ be needed 🏩
        let code_url = if payload.code.is_empty() {
            String::new()
        } else {
            let saved_code = NamedTempFile::new().map_err(|e| {
                tonic::Status::new(
                    tonic::Code::Internal,
                    format!("Failed to create temp file to save code in 😿: {}", e),
                )
            })?;

            fs::write(saved_code.path(), payload.code).map_err(|e| {
                tonic::Status::new(
                    tonic::Code::Internal,
                    format!(
                        "Failed to save the code in {} 🐼: {}",
                        saved_code.path().display(),
                        e
                    ),
                )
            })?;

            let (_, path) = saved_code.keep().map_err(|e| {
                tonic::Status::new(
                    tonic::Code::Internal,
                    format!("Failed to persist temp file with code: {}", e),
                )
            })?;
            format!("file://{}", path.display())
        };

        writer.insert(
            id.clone(),
            FunctionDescriptor {
                execution_environment: Some(ee),
                entrypoint: payload.entrypoint,
                code_url,
                function: Some(Function {
                    id: Some(FunctionId {
                        value: id.to_string(),
                    }),
                    name: payload.name,
                    tags: payload.tags,
                    inputs: payload.inputs,
                    outputs: payload.outputs,
                }),
            },
        );

        Ok(tonic::Response::new(FunctionId {
            value: id.to_string(),
        }))
    }
}

impl FunctionsRegistryService {
    pub fn new() -> Self {
        Self {
            functions: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[tonic::async_trait]
impl FunctionsRegistry for Arc<FunctionsRegistryService> {
    async fn list(
        &self,
        request: tonic::Request<ListRequest>,
    ) -> Result<tonic::Response<RegistryListResponse>, tonic::Status> {
        (**self).list(request).await
    }

    async fn get(
        &self,
        request: tonic::Request<FunctionId>,
    ) -> Result<tonic::Response<FunctionDescriptor>, tonic::Status> {
        (**self).get(request).await
    }

    async fn register(
        &self,
        register_request: tonic::Request<RegisterRequest>,
    ) -> Result<tonic::Response<FunctionId>, tonic::Status> {
        (**self).register(register_request).await
    }
}
