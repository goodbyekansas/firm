use std::{
    collections::HashMap,
    fs,
    sync::{Arc, RwLock},
};

use regex::Regex;
use semver::{Version, VersionReq};
use tempfile::NamedTempFile;
use tonic;
use url::Url;
use uuid::Uuid;

use crate::proto::functions_registry_server::FunctionsRegistry;
use crate::proto::{
    ExecutionEnvironment, Function as ProtoFunction, FunctionDescriptor, FunctionId, FunctionInput,
    FunctionOutput, GetLatestVersionRequest, ListRequest, ListVersionsRequest,
    Ordering as ProtoOrdering, RegisterRequest, RegistryListResponse,
};

#[derive(Debug, Default)]
pub struct FunctionsRegistryService {
    functions: Arc<RwLock<HashMap<Uuid, Function>>>,
}

#[derive(Debug)]
struct Function {
    id: Uuid,
    name: String,
    version: Version,
    execution_environment: String,
    entrypoint: String,
    inputs: Vec<FunctionInput>,
    outputs: Vec<FunctionOutput>,
    tags: HashMap<String, String>,
    code_url: Option<Url>,
}

impl From<&Function> for FunctionDescriptor {
    fn from(f: &Function) -> Self {
        FunctionDescriptor {
            function: Some(ProtoFunction {
                id: Some(FunctionId {
                    value: f.id.to_string(),
                }),
                name: f.name.clone(),
                version: f.version.to_string(),
                tags: f.tags.clone(),
                inputs: f.inputs.clone(),
                outputs: f.outputs.clone(),
            }),
            entrypoint: f.entrypoint.clone(),
            execution_environment: Some(ExecutionEnvironment {
                name: f.execution_environment.clone(),
            }),
            code_url: f
                .code_url
                .as_ref()
                .map(|u| u.to_string())
                .unwrap_or_default(),
        }
    }
}

impl FunctionsRegistryService {
    fn get_function_versions(
        &self,
        name: &str,
        version_requirement: &VersionReq,
        sort_order: ProtoOrdering,
    ) -> Result<Vec<FunctionDescriptor>, String> {
        let functions = self
            .functions
            .read()
            .map_err(|e| format!("Failed to get read lock for functions: {}", e))?;

        let mut filtered_functions: Vec<_> = functions
            .values()
            .filter(|f| f.name == name && dbg!(version_requirement).matches(&f.version))
            .collect();

        filtered_functions.sort_unstable_by(|a, b| match sort_order {
            ProtoOrdering::Ascending => b.version.cmp(&a.version),
            _ => a.version.cmp(&b.version),
        });

        Ok(filtered_functions.iter().map(|f| (*f).into()).collect())
    }
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
            functions: reader.values().map(|f| f.into()).collect(),
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
            .map(|f| tonic::Response::new(f.into()))
    }

    async fn get_latest_version(
        &self,
        request: tonic::Request<GetLatestVersionRequest>,
    ) -> Result<tonic::Response<FunctionDescriptor>, tonic::Status> {
        let payload = request.into_inner();
        let version_req = VersionReq::parse(&payload.version_requirement).map_err(|e| {
            tonic::Status::new(
                tonic::Code::InvalidArgument,
                format!("Supplied version requirement is invalid: {}", e),
            )
        })?;

        let filtered_functions = self
            .get_function_versions(&payload.name, &version_req, ProtoOrdering::Descending)
            .map_err(|e| tonic::Status::new(tonic::Code::Internal, e))?;

        filtered_functions.first().map(|f| tonic::Response::new(f.clone())).ok_or_else(|| {
            tonic::Status::new(
                tonic::Code::NotFound,
                format!("Found no function with name \"{}\", matching the version requirements \"{}\"", payload.name, payload.version_requirement)
                )
        })
    }

    async fn list_versions(
        &self,
        request: tonic::Request<ListVersionsRequest>,
    ) -> Result<tonic::Response<RegistryListResponse>, tonic::Status> {
        let payload = request.into_inner();
        let version_req = VersionReq::parse(&payload.version_requirement).map_err(|e| {
            tonic::Status::new(
                tonic::Code::InvalidArgument,
                format!("Supplied version requirement is invalid: {}", e),
            )
        })?;

        let ordering = match ProtoOrdering::from_i32(payload.ordering) {
            Some(ProtoOrdering::Ascending) => ProtoOrdering::Ascending,
            _ => ProtoOrdering::Descending,
        };

        let filtered_functions = self
            .get_function_versions(&payload.name, &version_req, ordering)
            .map_err(|e| tonic::Status::new(tonic::Code::Internal, e))?;

        Ok(tonic::Response::new(RegistryListResponse {
            functions: filtered_functions,
        }))
    }

    async fn register(
        &self,
        register_request: tonic::Request<RegisterRequest>,
    ) -> Result<tonic::Response<FunctionId>, tonic::Status> {
        let payload = register_request.into_inner();

        validate_name(&payload.name).map_err(|e| {
            tonic::Status::new(
                tonic::Code::InvalidArgument,
                format!("Invalid function name \"{}\": {}", payload.name, e),
            )
        })?;

        let mut version = validate_version(&payload.version).map_err(|e| {
            tonic::Status::new(
                tonic::Code::InvalidArgument,
                format!("Invalid function version \"{}\": {}", payload.version, e),
            )
        })?;

        let mut functions = self.functions.write().map_err(|e| {
            tonic::Status::new(
                tonic::Code::Internal,
                format!("Failed to get write lock for functions: {}", e),
            )
        })?;

        // this is the local case, always add dev to any function version
        version
            .pre
            .push(semver::Identifier::AlphaNumeric("dev".to_owned()));

        // remove function if name and version matches (after the -dev has been appended)
        functions.retain(|_, v| v.name != payload.name || v.version != version);

        let id = Uuid::new_v4();
        let execution_environment = payload.execution_environment.ok_or_else(|| {
            tonic::Status::new(
                tonic::Code::InvalidArgument,
                String::from("Execution environment is required when registering function"),
            )
        })?;

        // TODO: A better storage mechanism _will_ be needed 🏩
        let code_url = if payload.code.is_empty() {
            None
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

            Some(Url::from_file_path(&path).map_err(|_| {
                tonic::Status::new(
                    tonic::Code::Internal,
                    format!("Failed to generate url for file path {}", path.display()),
                )
            })?)
        };

        functions.insert(
            id.clone(),
            Function {
                id,
                name: payload.name,
                version,
                entrypoint: payload.entrypoint,
                execution_environment: execution_environment.name,
                code_url,
                tags: payload.tags,
                inputs: payload.inputs,
                outputs: payload.outputs,
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

    async fn get_latest_version(
        &self,
        request: tonic::Request<GetLatestVersionRequest>,
    ) -> Result<tonic::Response<FunctionDescriptor>, tonic::Status> {
        (**self).get_latest_version(request).await
    }

    async fn list_versions(
        &self,
        request: tonic::Request<ListVersionsRequest>,
    ) -> Result<tonic::Response<RegistryListResponse>, tonic::Status> {
        (**self).list_versions(request).await
    }

    async fn register(
        &self,
        register_request: tonic::Request<RegisterRequest>,
    ) -> Result<tonic::Response<FunctionId>, tonic::Status> {
        (**self).register(register_request).await
    }
}

fn validate_name(name: &str) -> Result<(), String> {
    const MAX_LEN: usize = 128;
    const MIN_LEN: usize = 3;
    if name.len() > MAX_LEN {
        Err(format!(
            "Function name is too long! Max {} characters",
            MAX_LEN
        ))
    } else if name.len() < MIN_LEN {
        Err(format!(
            "Function name is too short! Min {} characters",
            MIN_LEN
        ))
    } else {
        let regex = Regex::new(r"^[a-z0-9]{2,}([a-z0-9\-]?[a-z0-9]+)+$|^[a-z0-9]{3,}$")
            .map_err(|e| format!("Invalid regex: {}", e))?;
        if regex.is_match(name) {
            Ok(())
        } else {
            Err(String::from("Name contains invalid characters. Only lower case characters, numbers and dashes are allowed"))
        }
    }
}

fn validate_version(version: &str) -> Result<Version, String> {
    if version.is_empty() {
        return Err(String::from(
            "Version cannot be empty when registering function",
        ));
    }

    Version::parse(version).map_err(|e| format!("Invalid semantic version specified: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_validate_name() {
        assert!(validate_name("a").is_err());
        assert!(validate_name("ab").is_err());
        assert!(validate_name("abc").is_ok());
        assert!(validate_name("-ab").is_err());
        assert!(validate_name("ab-").is_err());
        assert!(validate_name("ab-c").is_ok());
        assert!(validate_name("ab-C").is_err());
        assert!(validate_name(&vec!['a'; 129].iter().collect::<String>()).is_err());
        assert!(validate_name("abc!").is_err());
        assert!(validate_name("ab1").is_ok());
        assert!(validate_name("😭").is_err());
    }

    #[test]
    fn test_validate_version() {
        assert!(validate_version("").is_err());
        assert!(validate_version("1.0,3").is_err());
        assert!(validate_version("1.0.5-alpha").is_ok());
    }
}
