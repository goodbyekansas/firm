use std::{
    collections::HashMap,
    fmt::{self, Debug, Display},
    fs, str,
};

use sha2::{Digest, Sha256};
use slog::{debug, Logger};
use thiserror::Error;
use url::Url;

use crate::runtime::{Runtime, RuntimeParameters, RuntimeSource};
use firm_types::{
    functions::{
        execution_result::Result as ProtoResult,
        execution_server::Execution as ExecutionServiceTrait, registry_server::Registry,
        Attachment, AuthMethod, ExecutionError, ExecutionId, ExecutionParameters, ExecutionResult,
        Filters, NameFilter, Ordering, OrderingKey, VersionRequirement,
    },
    stream::StreamExt,
    tonic,
};
use uuid::Uuid;
use RuntimeError::AttachmentReadError;

pub struct ExecutionService {
    logger: Logger,
    registry: Box<dyn Registry>,
    runtime_sources: Vec<Box<dyn RuntimeSource>>,
}

impl ExecutionService {
    pub fn new(
        log: Logger,
        registry: Box<dyn Registry>,
        runtime_sources: Vec<Box<dyn RuntimeSource>>,
    ) -> Self {
        Self {
            logger: log,
            registry,
            runtime_sources,
        }
    }

    /// Lookup a runtime for the given `runtime_name`
    ///
    /// If an runtime is not supported, an error is returned
    pub async fn lookup_runtime(
        &self,
        runtime_name: &str,
    ) -> Result<Box<dyn Runtime>, RuntimeError> {
        debug!(self.logger, "Looking up runtime {}", runtime_name);
        self.runtime_sources
            .iter()
            .find_map(|e| e.get(runtime_name))
            .ok_or_else(|| RuntimeError::RuntimeNotFound(runtime_name.to_owned()))
    }
}

#[tonic::async_trait]
impl ExecutionServiceTrait for ExecutionService {
    async fn execute_function(
        &self,
        request: tonic::Request<ExecutionParameters>,
    ) -> Result<tonic::Response<ExecutionResult>, tonic::Status> {
        // TODO: We do not have a cookie for this yet. Should be a cookie for looking up logs of the execution etc.
        let execution_id = ExecutionId {
            uuid: Uuid::new_v4().to_string(),
        };
        // lookup function
        let payload = request.into_inner();
        let function = self
            .registry
            .list(tonic::Request::new(Filters {
                name: Some(NameFilter {
                    pattern: payload.name.clone(),
                    exact_match: true,
                }),
                version_requirement: Some(VersionRequirement {
                    expression: payload.version_requirement.clone(),
                }),
                metadata: HashMap::new(),
                order: Some(Ordering {
                    key: OrderingKey::NameVersion as i32,
                    reverse: false,
                    offset: 0,
                    limit: 1,
                }),
            }))
            .await?
            .into_inner()
            .functions
            .first()
            .cloned()
            .ok_or_else(|| {
                tonic::Status::not_found(format!(
                    "Could not find function \"{}\" with version requirement: \"{}\"",
                    &payload.name, &payload.version_requirement
                ))
            })?;

        // validate args
        let args = payload.arguments.unwrap_or_default();
        args.validate(&function.required_inputs, Some(&function.optional_inputs))
            .map_err(|e| {
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

        let runtime = function.runtime.clone().ok_or_else(|| {
            tonic::Status::new(
                tonic::Code::Internal,
                "Function descriptor did not contain any runtime specification.",
            )
        })?;

        let function_name = function.name.clone();

        // lookup executor and run
        self.lookup_runtime(&runtime.name)
            .await
            .map_err(|e| {
                tonic::Status::new(
                    tonic::Code::Internal,
                    format!("Failed to lookup function runtime: {}", e),
                )
            })
            .and_then(|executor| {
                let res = executor.execute(
                    RuntimeParameters {
                        function_name: function_name.clone(),
                        entrypoint: runtime.entrypoint,
                        code: function.code.clone(),
                        arguments: runtime.arguments,
                    },
                    args,
                    function.attachments.clone(),
                );
                match res {
                    Ok(Ok(r)) => r
                        .validate(&function.outputs, None)
                        .map(|_| {
                            tonic::Response::new(ExecutionResult {
                                execution_id: Some(execution_id),
                                result: Some(ProtoResult::Ok(r)),
                            })
                        })
                        .map_err(|e| {
                            tonic::Status::new(
                                tonic::Code::InvalidArgument,
                                format!(
                                    "Function \"{}\" generated invalid result: {}",
                                    &function_name,
                                    e.iter()
                                        .map(|ae| format!("{}", ae))
                                        .collect::<Vec<String>>()
                                        .join(", ")
                                ),
                            )
                        }),
                    Ok(Err(e)) => Ok(tonic::Response::new(ExecutionResult {
                        execution_id: Some(execution_id),
                        result: Some(ProtoResult::Error(ExecutionError { msg: e })),
                    })),

                    Err(e) => Err(tonic::Status::new(
                        tonic::Code::Internal,
                        format!("Failed to execute function {}: {}", &function_name, e),
                    )),
                }
            })
    }
}

pub trait AttachmentDownload {
    fn download(&self) -> Result<Vec<u8>, RuntimeError>;
}

/// Download function attachment from the given URL
///
/// TODO: This is a huge security hole ⛳️ and needs to be managed properly (gpg sign 🔏 things?)
impl AttachmentDownload for Attachment {
    fn download(&self) -> Result<Vec<u8>, RuntimeError> {
        let url = self
            .url
            .as_ref()
            .ok_or_else(|| RuntimeError::InvalidCodeUrl("Attachment missing url.".to_owned()))
            .and_then(|u| {
                Url::parse(&u.url)
                    .map_err(|e| RuntimeError::InvalidCodeUrl(e.to_string()))
                    .map(|b| (b, AuthMethod::from_i32(u.auth_method)))
            })?;

        match (url.0.scheme(), url.1) {
            ("file", _) => {
                let content = fs::read(url.0.path())
                    .map_err(|e| AttachmentReadError(url.0.to_string(), e.to_string()))?;

                // TODO: this should be generalized when we
                // have other transports (like http(s))
                // validate integrity
                self.checksums
                    .as_ref()
                    .ok_or(RuntimeError::MissingChecksums)
                    .and_then(|checksums| {
                        let mut hasher = Sha256::new();
                        hasher.update(&content);

                        let checksum = hasher.finalize();

                        if &checksum[..] != hex::decode(checksums.sha256.clone())?.as_slice() {
                            Err(RuntimeError::ChecksumMismatch {
                                attachment_name: self.name.clone(),
                                wanted: checksums.sha256.clone(),
                                got: hex::encode(checksum),
                            })
                        } else {
                            Ok(())
                        }
                    })?;

                Ok(content)
            }
            // ("https", Some(auth)) => {}, // TODO: Support Oauth methods for http
            (s, _) => Err(RuntimeError::UnsupportedTransport(s.to_owned())),
        }
    }
}

#[derive(Debug)]
pub struct DependencyCycle {
    dependencies: Vec<(String, String)>,
}

impl Display for DependencyCycle {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.dependencies
            .iter()
            .try_for_each(|(fn_name, ee)| write!(f, "{} ({}) ➡️ ", fn_name, ee))?;
        write!(f, "💥")
    }
}

// TODO: Split this up in a way that makes sense - don't be insane
#[derive(Error, Debug)]
pub enum RuntimeError {
    #[error("Unsupported code transport mechanism: \"{0}\"")]
    UnsupportedTransport(String),

    #[error("Cyclic depencency detected for runtimes: \"{0}\"")]
    RuntimeDependencyCycle(DependencyCycle),

    #[error("Invalid code url: {0}")]
    InvalidCodeUrl(String),

    #[error("Failed to read code from {0}: {1}")]
    AttachmentReadError(String, String),

    #[error("Failed to find runtime \"{0}\"")]
    RuntimeNotFound(String),

    #[error("Function \"{0}\" did not have a runtime specified.")]
    MissingRuntime(String),

    #[error("Function is missing checksums.")]
    MissingChecksums,

    #[error("Function is missing id.")]
    FunctionMissingId,

    #[error("Failed to encode proto data: {0}")]
    EncodeError(#[from] prost::EncodeError),

    #[error("Code is missing even though it is required for the \"{0}\" executor.")]
    MissingCode(String),

    #[error(
        "Checksum mismatch for attachment \"{attachment_name}\". Wanted: {wanted}, got: {got}"
    )]
    ChecksumMismatch {
        attachment_name: String,
        wanted: String,
        got: String,
    },

    #[error("Failed to decode checksum to bytes: {0}")]
    FailedToDecodeChecksum(#[from] hex::FromHexError),
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::{cell::RefCell, rc::Rc};

    use firm_types::{attachment, attachment_file, functions::Stream as ValueStream, stream};

    use crate::{config::InternalRegistryConfig, registry::RegistryService, runtime};

    macro_rules! null_logger {
        () => {{
            slog::Logger::root(slog::Discard, slog::o!())
        }};
    }

    macro_rules! registry {
        () => {{
            RegistryService::new(InternalRegistryConfig::default(), null_logger!())
        }};
    }

    #[test]
    fn test_download() {
        // non-existent file
        let attachment = attachment!("file://this-file-does-not-exist");
        let r = attachment.download();
        assert!(r.is_err());
        assert!(matches!(
            r.unwrap_err(),
            RuntimeError::AttachmentReadError(..)
        ));

        // invalid url
        let attachment = attachment!("this-is-not-url");
        let r = attachment.download();
        assert!(r.is_err());
        assert!(matches!(r.unwrap_err(), RuntimeError::InvalidCodeUrl(..)));

        // unsupported scheme
        let attachment = attachment!("unsupported://that-scheme.fabrikam.com");
        let r = attachment.download();
        assert!(r.is_err());
        assert!(matches!(
            r.unwrap_err(),
            RuntimeError::UnsupportedTransport(..)
        ));

        // actual file
        let s = "some data 🖥️";
        let attachment = attachment_file!(s.as_bytes(), "somename");
        let r = attachment.download();
        assert!(r.is_ok());
        assert_eq!(s.as_bytes(), r.unwrap().as_slice());
    }

    #[derive(Default, Debug)]
    pub struct FakeExecutor {
        executor_context: RefCell<RuntimeParameters>,
        arguments: RefCell<ValueStream>,
        attachments: RefCell<Vec<Attachment>>,
        downloaded_code: RefCell<Vec<u8>>,
    }

    impl Runtime for Rc<FakeExecutor> {
        fn execute(
            &self,
            executor_context: RuntimeParameters,
            arguments: ValueStream,
            attachments: Vec<Attachment>,
        ) -> Result<Result<ValueStream, String>, RuntimeError> {
            *self.downloaded_code.borrow_mut() = executor_context
                .code
                .as_ref()
                .map(|c| c.download().unwrap())
                .unwrap_or_default();
            *self.executor_context.borrow_mut() = executor_context;
            *self.arguments.borrow_mut() = arguments;
            *self.attachments.borrow_mut() = attachments;
            Ok(Ok(stream!()))
        }
    }

    #[test]
    fn test_lookup_runtime() {
        // get wasi executor
        let fr = registry!();
        let execution_service = ExecutionService::new(
            null_logger!(),
            Box::new(fr),
            vec![Box::new(runtime::InternalRuntimeSource::new(
                null_logger!(),
            ))],
        );

        let res = futures::executor::block_on(execution_service.lookup_runtime("wasi"));
        assert!(res.is_ok());

        // get non existing executor
        let res = futures::executor::block_on(execution_service.lookup_runtime("ur-sula!"));
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            RuntimeError::RuntimeNotFound(..)
        ));
    }
}
