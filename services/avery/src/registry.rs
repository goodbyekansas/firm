use std::{
    collections::HashMap,
    fs,
    sync::{Arc, RwLock},
};

use regex::Regex;
use semver::{Version, VersionReq};
use tempfile::NamedTempFile;
use url::Url;
use uuid::Uuid;

use gbk_protocols::{
    functions::{
        functions_registry_server::FunctionsRegistry, Checksums, ExecutionEnvironment,
        Function as ProtoFunction, FunctionDescriptor, FunctionId, FunctionInput, FunctionOutput,
        ListRequest, OrderingDirection, OrderingKey, RegisterRequest, RegistryListResponse,
    },
    tonic,
};

#[derive(Debug, Default, Clone)]
pub struct FunctionsRegistryService {
    functions: Arc<RwLock<HashMap<Uuid, Function>>>,
}

#[derive(Debug)]
struct Function {
    id: Uuid,
    name: String,
    version: Version,
    execution_environment: ExecutionEnvironment,
    inputs: Vec<FunctionInput>,
    outputs: Vec<FunctionOutput>,
    tags: HashMap<String, String>,
    code_url: Option<Url>,
    checksums: Checksums,
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
            execution_environment: Some(f.execution_environment.clone()),
            code_url: f
                .code_url
                .as_ref()
                .map(|u| u.to_string())
                .unwrap_or_default(),
            checksums: Some(f.checksums.clone()),
        }
    }
}

#[tonic::async_trait]
impl FunctionsRegistry for FunctionsRegistryService {
    async fn list(
        &self,
        list_request: tonic::Request<ListRequest>,
    ) -> Result<tonic::Response<RegistryListResponse>, tonic::Status> {
        let reader = self.functions.read().map_err(|e| {
            tonic::Status::new(
                tonic::Code::Internal,
                format!("Failed to get read lock for functions: {}", e),
            )
        })?;

        let payload = list_request.into_inner();
        let required_tags = if payload.tags_filter.is_empty() {
            None
        } else {
            Some(payload.tags_filter.clone())
        };
        let offset: usize = payload.offset as usize;
        let limit: usize = payload.limit as usize;
        let version_req = payload
            .version_requirement
            .clone()
            .map(|vr| {
                VersionReq::parse(&vr.expression).map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::InvalidArgument,
                        format!("Supplied version requirement is invalid: {}", e),
                    )
                })
            })
            .map_or(Ok(None), |v| v.map(Some))?;
        let mut filtered_functions = reader
            .values()
            .filter(|func| {
                func.name.contains(&payload.name_filter)
                    && version_req
                        .as_ref()
                        .map_or(true, |ver_req| ver_req.matches(&func.version))
                    && required_tags.as_ref().map_or(true, |filters| {
                        filters.iter().all(|filter| {
                            func.tags
                                .iter()
                                .any(|(k, v)| filter.0 == k && filter.1 == v)
                        })
                    })
            })
            .collect::<Vec<&Function>>();
        filtered_functions.sort_unstable_by(|a, b| {
            match (
                OrderingKey::from_i32(payload.order_by),
                OrderingDirection::from_i32(payload.order_direction),
            ) {
                (Some(OrderingKey::Name), Some(OrderingDirection::Ascending))
                | (Some(OrderingKey::Name), None)
                | (None, None)
                | (None, Some(OrderingDirection::Ascending)) => match a.name.cmp(&b.name) {
                    std::cmp::Ordering::Equal => b.version.cmp(&a.version),
                    o => o,
                },
                (Some(OrderingKey::Name), Some(OrderingDirection::Descending))
                | (None, Some(OrderingDirection::Descending)) => match b.name.cmp(&a.name) {
                    std::cmp::Ordering::Equal => b.version.cmp(&a.version),
                    o => o,
                },
            }
        });

        Ok(tonic::Response::new(RegistryListResponse {
            functions: filtered_functions
                .iter()
                .skip(offset)
                .take(limit)
                .map(|f| (*f).into())
                .collect(),
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
        let checksums = payload.checksums.ok_or_else(|| {
            tonic::Status::new(
                tonic::Code::InvalidArgument,
                String::from("Checksums is required when registering function"),
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
                execution_environment,
                code_url,
                tags: payload.tags,
                inputs: payload.inputs,
                outputs: payload.outputs,
                checksums,
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
        let regex = Regex::new(r"^[a-z][a-z0-9]{1,}([a-z0-9\-]?[a-z0-9]+)+$|^[a-z][a-z0-9]{2,}$")
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
        assert!(validate_name("1ab").is_err());
        assert!(validate_name("a1b").is_ok());
        assert!(validate_name("ab1").is_ok());
        assert!(validate_name(&vec!['a'; 129].iter().collect::<String>()).is_err());
        assert!(validate_name("abc!").is_err());
        assert!(validate_name("😭").is_err());
    }

    #[test]
    fn test_validate_version() {
        assert!(validate_version("").is_err());
        assert!(validate_version("1.0,3").is_err());
        assert!(validate_version("1.0.5-alpha").is_ok());
    }
}
