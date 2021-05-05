use std::{
    collections::HashMap,
    fmt::{self, Debug, Display},
    fs,
    path::Path,
    path::PathBuf,
    str,
    sync::Arc,
    sync::Mutex,
};

use firm_types::{
    functions::{
        execution_result::Result as ProtoResult,
        execution_server::Execution as ExecutionServiceTrait, registry_server::Registry,
        Attachment, AuthMethod, ExecutionError, ExecutionId, ExecutionParameters, ExecutionResult,
        Filters, Function, FunctionOutputChunk, NameFilter, Ordering, OrderingKey,
        Runtime as ProtoRuntime, RuntimeFilters, RuntimeList, Stream as ValueStream,
        VersionRequirement,
    },
    stream::StreamExt,
    tonic,
};
use futures::{channel::mpsc::Receiver, channel::mpsc::Sender, TryFutureExt};
use sha2::{Digest, Sha256};
use slog::{debug, Logger};
use thiserror::Error;
use url::Url;
use uuid::Uuid;

use crate::{
    auth::AuthService,
    auth::AuthenticationSource,
    runtime::{Runtime, RuntimeParameters, RuntimeSource},
};

#[derive(Debug, Clone)]
pub struct FunctionOutputSink {
    inner: Option<Sender<Result<FunctionOutputChunk, tonic::Status>>>,
}

impl FunctionOutputSink {
    pub fn null() -> Self {
        Self { inner: None }
    }

    pub fn close(&mut self) {
        if let Some(sender) = self.inner.as_mut() {
            sender.close_channel()
        }
    }

    pub fn send(&mut self, channel: String, content: String) {
        self.inner.as_mut().map(|sender| {
            sender.try_send(Ok(FunctionOutputChunk {
                channel,
                output: content,
            }))
        });
    }
}

impl From<Sender<Result<FunctionOutputChunk, tonic::Status>>> for FunctionOutputSink {
    fn from(sender: Sender<Result<FunctionOutputChunk, tonic::Status>>) -> Self {
        Self {
            inner: Some(sender),
        }
    }
}

#[derive(Debug)]
pub struct QueuedFunction {
    function: Function,
    arguments: ValueStream,
    output_receiver: Option<Receiver<Result<FunctionOutputChunk, tonic::Status>>>,
    output_sender: Sender<Result<FunctionOutputChunk, tonic::Status>>,
}

#[derive(Clone)]
pub struct ExecutionService {
    logger: Logger,
    registry: Arc<Box<dyn Registry>>,
    runtime_sources: Arc<Vec<Box<dyn RuntimeSource>>>,
    execution_queue: Arc<Mutex<HashMap<Uuid, QueuedFunction>>>, // Death row hehurr
    root_dir: PathBuf,
    auth_service: AuthService,
}

impl ExecutionService {
    pub fn new(
        log: Logger,
        registry: Box<dyn Registry>,
        runtime_sources: Vec<Box<dyn RuntimeSource>>,
        auth_service: AuthService,
        root_dir: &Path,
    ) -> Self {
        Self {
            logger: log,
            registry: Arc::new(registry),
            runtime_sources: Arc::new(runtime_sources),
            execution_queue: Arc::new(Mutex::new(HashMap::new())),
            root_dir: root_dir.to_owned(),
            auth_service,
        }
    }

    /// Lookup a runtime for the given `runtime_name`
    ///
    /// If a runtime is not supported, an error is returned
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
    async fn queue_function(
        &self,
        request: tonic::Request<ExecutionParameters>,
    ) -> Result<tonic::Response<ExecutionId>, tonic::Status> {
        let execution_id = Uuid::new_v4();
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

        // TODO: args should be sent to run function instead.
        // Problem is that it's nice to get args validated as early as possible.
        // Another problem is that we do not want to save the state of the arguments
        // in memory since it could potentially be a lot of memory.
        // There are two solutions.
        // 1. Only send keys to queue_function and validate the keys (user could change this in run function later which is bad)
        // 2. Only send to run_function and validate there. Bad part is getting late validation of args.
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

        // allocate an output message queue
        let (sender, receiver) = futures::channel::mpsc::channel(1024);

        self.execution_queue
            .lock()
            .map_err(|_| tonic::Status::internal("Failed to lock execution queue."))?
            .insert(
                execution_id,
                QueuedFunction {
                    function,
                    arguments: args,
                    output_receiver: Some(receiver),
                    output_sender: sender,
                },
            );

        Ok(tonic::Response::new(ExecutionId {
            uuid: execution_id.to_string(),
        }))
    }

    async fn run_function(
        &self,
        request: tonic::Request<ExecutionId>,
    ) -> Result<tonic::Response<ExecutionResult>, tonic::Status> {
        let id = request.into_inner();
        let uuid = Uuid::parse_str(&id.uuid).map_err(|e| {
            tonic::Status::invalid_argument(format!("Failed to parse execution id as uuid: {}.", e))
        })?;

        let queued_function = self
            .execution_queue
            .lock()
            .map_err(|_| tonic::Status::internal("Failed to lock execution queue."))?
            .remove(&uuid)
            .ok_or_else(|| {
                tonic::Status::not_found(format!(
                    "Failed to find queued execution with id \"{}\"",
                    uuid.to_string()
                ))
            })?;

        let runtime_spec = queued_function.function.runtime.clone().ok_or_else(|| {
            tonic::Status::internal(
                "Function descriptor did not contain any runtime specification.",
            )
        })?;

        // TODO: Can combine this with runtime.execute as a long chain once
        // runtime.execute is async.
        let runtime = self.lookup_runtime(&runtime_spec.name).await.map_err(|e| {
            tonic::Status::new(
                tonic::Code::Internal,
                format!("Failed to lookup function runtime: {}", e),
            )
        })?;

        // TODO: Use Rayon to spawn runtime.execute on a thread pool, see: https://ryhl.io/blog/async-what-is-blocking/
        let output_spec = queued_function.function.outputs.clone();
        let function_name = queued_function.function.name.clone();
        let function_name2 = function_name.clone();
        let auth_service = self.auth_service.clone();

        let function_dir = self.root_dir.join(format!(
            "{name}-{version}{checksum}",
            name = &function_name,
            version = &queued_function.function.version,
            checksum = &queued_function
                .function
                .code
                .as_ref()
                .and_then(|cs| cs.checksums.as_ref())
                .map(|cs| format!("-{}", &cs.sha256[..16]))
                .unwrap_or_default()
        ));

        if !function_dir.exists() {
            std::fs::create_dir(&function_dir).map_err(|ioe| {
                tonic::Status::internal(format!("Failed to create function directory: {}", ioe))
            })?;
        }

        let execution_dir = function_dir.join(&id.uuid);
        std::fs::create_dir(&execution_dir).map_err(|ioe| {
            tonic::Status::internal(format!(
                "Failed to create function execution directory: {}",
                ioe
            ))
        })?;

        let async_runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|e| {
                tonic::Status::internal(format!(
                    "Failed to create async runtime for function execution: {}",
                    e
                ))
            })?;

        let execution_res = tokio::task::spawn_blocking(move || {
            runtime.execute(
                RuntimeParameters {
                    function_name: function_name2,
                    entrypoint: if runtime_spec.entrypoint.is_empty() {
                        None
                    } else {
                        Some(runtime_spec.entrypoint)
                    },
                    code: queued_function.function.code.clone(),
                    arguments: runtime_spec.arguments,
                    output_sink: queued_function.output_sender.into(),
                    root_dir: execution_dir,
                    auth_service,
                    async_runtime,
                },
                queued_function.arguments,
                queued_function.function.attachments.clone(),
            )
        })
        .await
        .map_err(|e| tonic::Status::internal(format!("Failed to wait for execution: {}", e)))?;

        match execution_res {
            Ok(Ok(r)) => r
                .validate(&output_spec, None)
                .map(|_| {
                    tonic::Response::new(ExecutionResult {
                        execution_id: Some(id),
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
                execution_id: Some(id),
                result: Some(ProtoResult::Error(ExecutionError { msg: e })),
            })),

            Err(e) => Err(tonic::Status::new(
                tonic::Code::Internal,
                format!("Failed to execute function {}: {}", &function_name, e),
            )),
        }
    }

    async fn function_output(
        &self,
        request: tonic::Request<ExecutionId>,
    ) -> Result<tonic::Response<Self::FunctionOutputStream>, tonic::Status> {
        let id = request.into_inner();
        let uuid = Uuid::parse_str(&id.uuid).map_err(|e| {
            tonic::Status::invalid_argument(format!("Failed to parse execution id as uuid: {}.", e))
        })?;

        self.execution_queue
            .lock()
            .map_err(|_| tonic::Status::internal("Failed to lock execution queue."))?
            .get_mut(&uuid)
            .ok_or_else(|| {
                tonic::Status::not_found(format!(
                    "Failed to find queued execution with id \"{}\"",
                    uuid.to_string()
                ))
            })?
            .output_receiver
            .take()
            .ok_or_else(|| {
                tonic::Status::internal(format!(
                    "No output channel set (or it has already been used) for execution id \"{}\"",
                    id.uuid
                ))
            })
            .map(tonic::Response::new)
    }

    type FunctionOutputStream = Receiver<Result<FunctionOutputChunk, tonic::Status>>;

    async fn list_runtimes(
        &self,
        request: tonic::Request<RuntimeFilters>,
    ) -> Result<tonic::Response<RuntimeList>, tonic::Status> {
        let payload = request.into_inner();
        Ok(tonic::Response::new(RuntimeList {
            runtimes: self
                .runtime_sources
                .iter()
                .flat_map(|runtime_src| {
                    let src_name = runtime_src.name().to_owned();
                    let filter_name = payload.name.clone();
                    runtime_src
                        .list()
                        .into_iter()
                        .filter_map(move |runtime_name| {
                            if runtime_name.contains(&filter_name) {
                                Some(ProtoRuntime {
                                    name: runtime_name,
                                    source: src_name.clone(),
                                })
                            } else {
                                None
                            }
                        })
                })
                .collect(),
        }))
    }
}

#[async_trait::async_trait]
pub trait AttachmentDownload {
    async fn download(&self, auth: &dyn AuthenticationSource) -> Result<Vec<u8>, RuntimeError>;
}

/// Download function attachment from the given URL
#[async_trait::async_trait]
impl AttachmentDownload for Attachment {
    async fn download(&self, auth: &dyn AuthenticationSource) -> Result<Vec<u8>, RuntimeError> {
        let attachment_url = self
            .url
            .as_ref()
            .ok_or_else(|| RuntimeError::InvalidCodeUrl("Attachment missing url.".to_owned()))?;

        let (url, auth_method) = Url::parse(&attachment_url.url)
            .map_err(|e| RuntimeError::InvalidCodeUrl(e.to_string()))
            .map(|b| (b, AuthMethod::from_i32(attachment_url.auth_method)))?;

        let content = match (url.scheme(), auth_method) {
            ("file", _) => {
                let content =
                    // The Url parser looses information on file paths. Therefore just take
                    // The original and skip "file://"
                    fs::read(&attachment_url.url[7..]).map_err(|e| {
                        RuntimeError::AttachmentReadError(attachment_url.url.to_owned(), e.to_string())
                    })?;

                Ok(content)
            }
            ("https", Some(auth_method)) | ("http", Some(auth_method)) => {
                let request = reqwest::Client::new().get(url.clone());

                match auth_method {
                    AuthMethod::None => Ok(request),
                    AuthMethod::Oauth2 => {
                        futures::future::ready(url.host_str().ok_or_else(|| {
                            RuntimeError::AttachmentDownloadError(
                                url.to_string(),
                                "Attachment URL has no host which is needed to acquire credentials"
                                    .to_owned(),
                            )
                        }))
                        .and_then(|scope| {
                            auth.acquire_token(scope)
                                .map_err(|e| {
                                    RuntimeError::AttachmentDownloadError(
                                        url.to_string(),
                                        format!(
                                            "Failed to acquire credentials for attachment: {}",
                                            e
                                        ),
                                    )
                                })
                                .map_ok(|token| request.bearer_auth(token))
                        })
                        .await
                    }
                    // TODO: Maybe not have support for thiz
                    AuthMethod::Basic => Err(RuntimeError::UnsupportedTransport(
                        "Basic auth is not supported when downloading attachments".to_owned(),
                    )),
                }?
                .send()
                .map_err(|e| {
                    RuntimeError::AttachmentDownloadError(
                        url.to_string(),
                        format!("Transport error: {}", e),
                    )
                })
                .and_then(|resp| {
                    futures::future::ready(resp.error_for_status().map_err(|e| {
                        RuntimeError::AttachmentDownloadError(
                            url.to_string(),
                            format!("HTTP error: {}", e),
                        )
                    }))
                })
                .and_then(|resp| {
                    resp.bytes().map_err(|e| {
                        RuntimeError::AttachmentReadError(url.to_string(), e.to_string())
                    })
                })
                .await
                .map(|bytes| bytes.to_vec())
            }
            (s, _) => Err(RuntimeError::UnsupportedTransport(s.to_owned())),
        }?;

        self.checksums
            .as_ref()
            .ok_or_else(|| RuntimeError::MissingChecksums(self.name.clone()))
            .and_then(|checksums| {
                let checksum = Sha256::digest(&content);
                if &checksum[..] != hex::decode(checksums.sha256.clone())?.as_slice() {
                    Err(RuntimeError::ChecksumMismatch {
                        attachment_name: self.name.clone(),
                        wanted: checksums.sha256.clone(),
                        got: hex::encode(checksum),
                    })
                } else {
                    Ok(content)
                }
            })
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

    #[error("Failed to donwload attachment \"{0}\": {1}")]
    AttachmentDownloadError(String, String),

    #[error("Failed to read code from {0}: {1}")]
    AttachmentReadError(String, String),

    #[error("Failed to find runtime \"{0}\"")]
    RuntimeNotFound(String),

    #[error("Function \"{0}\" did not have a runtime specified.")]
    MissingRuntime(String),

    #[error("Attachment \"{0}\" is missing checksums.")]
    MissingChecksums(String),

    #[error("Function is missing id.")]
    FunctionMissingId,

    #[error("Failed to encode proto data: {0}")]
    EncodeError(#[from] prost::EncodeError),

    #[error("Code is missing even though it is required for the \"{0}\" runtime.")]
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

    // TODO Maybe not the best place to have this error since it will pop up for executors
    #[error("Failed to queue output chunk: {0}")]
    FailedToQueueOutputChunk(String),

    #[error("Runtime {name} error: {message}")]
    RuntimeError { name: String, message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    use firm_types::{attachment, attachment_file};

    use crate::{config::InternalRegistryConfig, registry::RegistryService, runtime};

    macro_rules! null_logger {
        () => {{
            slog::Logger::root(slog::Discard, slog::o!())
        }};
    }

    macro_rules! registry {
        () => {{
            RegistryService::new(InternalRegistryConfig::default(), null_logger!()).unwrap()
        }};
    }

    #[tokio::test]
    async fn test_download() {
        // non-existent file
        let attachment = attachment!("file:///this-file-does-not-exist");
        let r = attachment.download(&AuthService::default()).await;
        assert!(r.is_err());
        assert!(matches!(
            r.unwrap_err(),
            RuntimeError::AttachmentReadError(..)
        ));

        // invalid url
        let attachment = attachment!("this-is-not-url");
        let r = attachment.download(&AuthService::default()).await;
        assert!(r.is_err());
        assert!(matches!(r.unwrap_err(), RuntimeError::InvalidCodeUrl(..)));

        // unsupported scheme
        let attachment = attachment!("unsupported://that-scheme.fabrikam.com");
        let r = attachment.download(&AuthService::default()).await;
        assert!(r.is_err());
        assert!(matches!(
            r.unwrap_err(),
            RuntimeError::UnsupportedTransport(..)
        ));

        // actual file
        let s = "some data 🖥️";
        let attachment = attachment_file!(s.as_bytes(), "somename");
        let r = attachment.download(&AuthService::default()).await;
        assert!(r.is_ok());
        assert_eq!(s.as_bytes(), r.unwrap().as_slice());

        // HTTP
        let data = b"datadatadatdatadatadadt";
        let checksum = format!("{:x}", sha2::Sha256::digest(data));
        let attachment = attachment!(mockito::server_url(), "name", &checksum);
        let mock = mockito::mock("GET", "/").with_body(data).create();
        let r = attachment.download(&AuthService::default()).await;
        assert!(r.is_ok());
        assert_eq!(
            r.unwrap(),
            data,
            "The downloaded data should be the initial data"
        );
        assert!(mock.matched(), "The server must have been called");

        // HTTP with bad checksum
        let data = "qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq";
        let mock = mockito::mock("GET", "/").with_body(data).create();
        let r = attachment.download(&AuthService::default()).await;
        assert!(mock.matched());
        assert!(
            matches!(r, Err(RuntimeError::ChecksumMismatch { .. })),
            "Mismatching checksum must return an error"
        );
    }

    #[test]
    fn test_lookup_runtime() {
        // get wasi executor
        let root_dir = tempfile::TempDir::new().unwrap();
        let fr = registry!();
        let execution_service = ExecutionService::new(
            null_logger!(),
            Box::new(fr),
            vec![Box::new(runtime::InternalRuntimeSource::new(
                null_logger!(),
            ))],
            AuthService::default(),
            root_dir.path(),
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

    #[test]
    fn list_runtimes() {
        // get the runtimes
        let root_dir = tempfile::TempDir::new().unwrap();
        let fr = registry!();
        let execution_service = ExecutionService::new(
            null_logger!(),
            Box::new(fr),
            vec![Box::new(runtime::InternalRuntimeSource::new(
                null_logger!(),
            ))],
            AuthService::default(),
            root_dir.path(),
        );

        let res = futures::executor::block_on(execution_service.list_runtimes(
            tonic::Request::new(RuntimeFilters {
                name: String::new(),
            }),
        ));
        assert!(
            res.is_ok(),
            "Expected to be able to list runtimes without a filter"
        );
        let res = res.unwrap().into_inner();
        assert_eq!(
            &res.runtimes,
            &[ProtoRuntime {
                name: "wasi".to_owned(),
                source: "internal".to_owned()
            }]
        );

        // with a filter this time
        let res = futures::executor::block_on(execution_service.list_runtimes(
            tonic::Request::new(RuntimeFilters {
                name: String::from("asi"),
            }),
        ));
        assert!(
            res.is_ok(),
            "Expected to be able to list runtimes with a filter"
        );
        let res = res.unwrap().into_inner();
        assert_eq!(
            &res.runtimes,
            &[ProtoRuntime {
                name: "wasi".to_owned(),
                source: "internal".to_owned()
            }]
        );

        // with bad filter
        let res = futures::executor::block_on(execution_service.list_runtimes(
            tonic::Request::new(RuntimeFilters {
                name: String::from("wasabi"),
            }),
        ));
        assert!(
            res.is_ok(),
            "Expected to be able to list runtimes with a filter"
        );
        let res = res.unwrap().into_inner();
        assert!(res.runtimes.is_empty(), "Expected no matches for wasabi");
    }
}
