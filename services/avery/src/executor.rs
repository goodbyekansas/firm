mod wasi;

use std::{
    collections::{HashMap, HashSet},
    fmt::{self, Debug, Display},
    fs, str,
};

use prost::Message;
use semver::VersionReq;
use sha2::{Digest, Sha256};
use slog::{o, Logger};
use thiserror::Error;
use url::Url;

use crate::executor::wasi::WasiExecutor;
use firm_types::{
    execution::{
        channel::Value as ValueType, execution_result::Result as ProtoResult,
        execution_server::Execution as ExecutionServiceTrait, Channel, ExecutionError, ExecutionId,
        ExecutionParameters, ExecutionResult, Stream as ValueStream, Strings,
    },
    functions::{Attachment, AuthMethod, Function},
    registry::{
        registry_server::Registry, Filters, NameFilter, Ordering, OrderingKey, VersionRequirement,
    },
    stream::{StreamExt, ToChannel},
    tonic,
    wasi::Attachments,
};
use uuid::Uuid;
use ExecutorError::AttachmentReadError;

pub struct ExecutionService {
    log: Logger,
    registry: Box<dyn Registry>,
}

impl ExecutionService {
    pub fn new(log: Logger, registry: Box<dyn Registry>) -> Self {
        Self { log, registry }
    }

    /// Lookup an executor for the given `runtime_name`
    ///
    /// If an executor is not supported, an error is returned
    pub async fn lookup_executor_for_runtime(
        &self,
        runtime_name: &str,
    ) -> Result<Box<dyn FunctionExecutor>, ExecutorError> {
        let functions = self.traverse_runtimes(runtime_name).await?;

        // TODO: now we are assuming that the stop condition for the above function
        // was "wasi". This may not be true later
        let executor = Box::new(WasiExecutor::new(
            functions
                .last()
                .map(|(_fd, logger)| logger)
                .unwrap_or(&self.log)
                .new(o!("executor" => "wasi")),
        ));

        Ok(functions
            .into_iter()
            .fold(executor, |prev_executor, (fd, fd_logger)| {
                Box::new(FunctionAdapter::new(prev_executor, fd, fd_logger))
            }))
    }

    async fn traverse_runtimes(
        &self,
        runtime_name: &str,
    ) -> Result<Vec<(Function, Logger)>, ExecutorError> {
        let mut runtime = runtime_name.to_owned();
        let mut functions = vec![];
        let mut ids = HashSet::new();

        loop {
            match runtime.as_str() {
                "wasi" => break,
                rt => {
                    let function = self
                        .get_executor_function_for_runtime(rt, None) // TODO: runtime version requirements
                        .await
                        .ok_or_else(|| ExecutorError::ExecutorNotFound(rt.to_owned()))?;

                    runtime = function
                        .runtime
                        .as_ref()
                        .ok_or_else(|| ExecutorError::MissingRuntime("".to_owned()))?
                        .name
                        .clone();

                    functions.push((
                        function.clone(),
                        functions
                            .last()
                            .map(|(_fd, logger)| logger)
                            .unwrap_or(&self.log)
                            .new(o!("executor" => runtime.clone())),
                    ));

                    if !ids.insert(format!("{}-{}", &function.name, &function.version)) {
                        return Err(ExecutorError::ExecutorDependencyCycle(DependencyCycle {
                            dependencies: functions
                                .iter()
                                .map(|(f, _log)| {
                                    (
                                        f.name.clone(),
                                        f.runtime
                                            .as_ref()
                                            .map(|rt| rt.name.clone())
                                            .unwrap_or_else(|| {
                                                "invalid execution environment".to_owned()
                                            }),
                                    )
                                })
                                .collect(),
                        }));
                    }
                }
            }
        }

        functions.reverse();
        Ok(functions)
    }

    async fn get_executor_function_for_runtime(
        &self,
        runtime: &str,
        version_requirement: Option<VersionReq>,
    ) -> Option<Function> {
        let mut execution_env_metadata = HashMap::new();
        execution_env_metadata.insert("executor-for".to_owned(), runtime.to_owned());

        let result = self
            .registry
            .list(tonic::Request::new(Filters {
                name: None,
                version_requirement: version_requirement.map(|vr| VersionRequirement {
                    expression: vr.to_string(),
                }),
                metadata: execution_env_metadata,
                order: Some(Ordering {
                    key: OrderingKey::NameVersion as i32,
                    reverse: false,
                    offset: 0,
                    limit: 1,
                }),
            }))
            .await
            .ok()?
            .into_inner();

        result.functions.first().cloned()
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
        args.validate(&function.input.clone().unwrap_or_default())
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
        self.lookup_executor_for_runtime(&runtime.name)
            .await
            .map_err(|e| {
                tonic::Status::new(
                    tonic::Code::Internal,
                    format!("Failed to lookup function executor: {}", e),
                )
            })
            .and_then(|executor| {
                let res = executor.execute(
                    ExecutorParameters {
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
                        .validate(&function.output.unwrap_or_default())
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

#[derive(Default, Debug)]
pub struct ExecutorParameters {
    pub function_name: String,
    pub entrypoint: String,
    pub code: Option<Attachment>,
    pub arguments: HashMap<String, String>,
}

pub trait FunctionExecutor: Debug {
    fn execute(
        &self,
        executor_context: ExecutorParameters,
        arguments: ValueStream,
        attachments: Vec<Attachment>,
    ) -> Result<Result<ValueStream, String>, ExecutorError>;
}

pub trait AttachmentDownload {
    fn download(&self) -> Result<Vec<u8>, ExecutorError>;
}

/// Download function attachment from the given URL
///
/// TODO: This is a huge security hole ⛳️ and needs to be managed properly (gpg sign 🔏 things?)
impl AttachmentDownload for Attachment {
    fn download(&self) -> Result<Vec<u8>, ExecutorError> {
        let url = self
            .url
            .as_ref()
            .ok_or_else(|| ExecutorError::InvalidCodeUrl("Attachment missing url.".to_owned()))
            .and_then(|u| {
                Url::parse(&u.url)
                    .map_err(|e| ExecutorError::InvalidCodeUrl(e.to_string()))
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
                    .ok_or(ExecutorError::MissingChecksums)
                    .and_then(|checksums| {
                        let mut hasher = Sha256::new();
                        hasher.update(&content);

                        let checksum = hasher.finalize();

                        if &checksum[..] != hex::decode(checksums.sha256.clone())?.as_slice() {
                            Err(ExecutorError::ChecksumMismatch {
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
            (s, _) => Err(ExecutorError::UnsupportedTransport(s.to_owned())),
        }
    }
}

#[derive(Debug)]
pub struct FunctionAdapter {
    executor: Box<dyn FunctionExecutor>,
    executor_function: Function,
    logger: Logger,
}

/// Adapter for functions to act as executors
impl FunctionAdapter {
    pub fn new(executor: Box<dyn FunctionExecutor>, function: Function, logger: Logger) -> Self {
        Self {
            executor,
            executor_function: function,
            logger,
        }
    }
}

impl FunctionExecutor for FunctionAdapter {
    fn execute(
        &self,
        executor_context: ExecutorParameters,
        arguments: ValueStream,
        attachments: Vec<Attachment>,
    ) -> Result<Result<ValueStream, String>, ExecutorError> {
        let mut executor_function_arguments = ValueStream {
            channels: executor_context
                .arguments
                .into_iter()
                .map(|(k, v)| {
                    (
                        k,
                        Channel {
                            value: Some(ValueType::Strings(Strings { values: vec![v] })),
                        },
                    )
                })
                .collect(),
        };

        // not having any code for the function is a valid case used for example to execute
        // external functions (gcp, aws lambdas, etc)
        if let Some(code) = executor_context.code {
            let mut code_buf = Vec::with_capacity(code.encoded_len());
            code.encode(&mut code_buf)?;

            executor_function_arguments
                .channels
                .insert("_code".to_owned(), code_buf.to_channel());

            let checksums = code.checksums.ok_or(ExecutorError::MissingChecksums)?;
            executor_function_arguments
                .channels
                .insert("_sha256".to_owned(), checksums.sha256.to_channel());
        }

        executor_function_arguments.channels.insert(
            "_entrypoint".to_owned(),
            executor_context.entrypoint.to_channel(),
        );

        // nest arguments and attachments
        let mut arguments_buf: Vec<u8> = Vec::with_capacity(arguments.encoded_len());
        arguments.encode(&mut arguments_buf)?;
        executor_function_arguments
            .channels
            .insert("_arguments".to_owned(), arguments_buf.to_channel());

        let proto_attachments = Attachments { attachments };
        let mut attachments_buf: Vec<u8> = Vec::with_capacity(proto_attachments.encoded_len());
        proto_attachments.encode(&mut attachments_buf)?;
        executor_function_arguments
            .channels
            .insert("_attachments".to_owned(), attachments_buf.to_channel());

        let function_exe_env =
            self.executor_function.runtime.clone().ok_or_else(|| {
                ExecutorError::MissingRuntime(self.executor_function.name.clone())
            })?;

        self.executor.execute(
            ExecutorParameters {
                function_name: self.executor_function.name.clone(),
                entrypoint: function_exe_env.entrypoint,
                code: self.executor_function.code.clone(),
                arguments: function_exe_env.arguments,
            },
            executor_function_arguments,
            self.executor_function.attachments.clone(),
        )
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
            .map(|(fn_name, ee)| write!(f, "{} ({}) ➡️ ", fn_name, ee))
            .collect::<fmt::Result>()?;
        write!(f, "💥")
    }
}

#[derive(Error, Debug)]
pub enum ExecutorError {
    #[error("Unsupported code transport mechanism: \"{0}\"")]
    UnsupportedTransport(String),

    #[error("Cyclic depencency detected for execution environments: \"{0}\"")]
    ExecutorDependencyCycle(DependencyCycle),

    #[error("Invalid code url: {0}")]
    InvalidCodeUrl(String),

    #[error("Failed to read code from {0}: {1}")]
    AttachmentReadError(String, String),

    #[error("Failed to find executor for execution environment \"{0}\"")]
    ExecutorNotFound(String),

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

    use std::{cell::RefCell, collections::HashMap, rc::Rc};

    use crate::registry::RegistryService;
    use firm_types::{attachment, attachment_file, code_file, function_data, stream};
    use firm_types::{functions::Runtime, registry::FunctionData};

    macro_rules! null_logger {
        () => {{
            slog::Logger::root(slog::Discard, o!())
        }};
    }

    macro_rules! registry {
        () => {{
            RegistryService::new(null_logger!())
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
            ExecutorError::AttachmentReadError(..)
        ));

        // invalid url
        let attachment = attachment!("this-is-not-url");
        let r = attachment.download();
        assert!(r.is_err());
        assert!(matches!(r.unwrap_err(), ExecutorError::InvalidCodeUrl(..)));

        // unsupported scheme
        let attachment = attachment!("unsupported://that-scheme.fabrikam.com");
        let r = attachment.download();
        assert!(r.is_err());
        assert!(matches!(
            r.unwrap_err(),
            ExecutorError::UnsupportedTransport(..)
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
        executor_context: RefCell<ExecutorParameters>,
        arguments: RefCell<ValueStream>,
        attachments: RefCell<Vec<Attachment>>,
        downloaded_code: RefCell<Vec<u8>>,
    }

    impl FunctionExecutor for Rc<FakeExecutor> {
        fn execute(
            &self,
            executor_context: ExecutorParameters,
            arguments: ValueStream,
            attachments: Vec<Attachment>,
        ) -> Result<Result<ValueStream, String>, ExecutorError> {
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

    /*#[test]
    // TODO:
    fn test_bad_checksum_for_code() {
    }*/

    #[test]
    fn test_nested_executor() {
        let fake = Rc::new(FakeExecutor::default());

        let mut runtime_args = HashMap::new();
        runtime_args.insert("sune".to_owned(), "bune".to_owned());

        let s = "some data 🖥️";
        let nested = FunctionAdapter::new(
            Box::new(fake.clone()),
            Function {
                runtime: Some(Runtime {
                    name: "Springtid 🏃⌚".to_owned(),
                    entrypoint: "ingångspoäng 💯".to_owned(),
                    arguments: HashMap::new(),
                }),
                code: Some(code_file!(s.as_bytes())),
                attachments: vec![],
                name: "wienerbröööööööö".to_owned(),
                version: "2019.3-5-PR2".to_owned(),
                metadata: HashMap::new(),
                input: None,
                output: None,
                created_at: 0,
            },
            null_logger!(),
        );
        let code = "asd";
        let entry = "entry";
        let attachments = vec![attachment!("fake://")];
        let result = nested.execute(
            ExecutorParameters {
                function_name: "test".to_owned(),
                entrypoint: entry.to_owned(),
                code: Some(code_file!(code.as_bytes())),
                arguments: runtime_args.clone(),
            },
            stream!({"test-arg" => "test-value"}),
            attachments,
        );
        // Test that code got passed
        assert!(result.is_ok());
        assert_eq!(s.as_bytes(), fake.downloaded_code.borrow().as_slice());

        // Test that the argument we send in is passed through
        {
            let arguments = fake.arguments.borrow();

            assert_eq!(arguments.channels.len(), 6);
            let code_attachment =
                Attachment::decode(arguments.get_channel_as_ref::<[u8]>("_code").unwrap()).unwrap();

            assert_eq!(code_attachment.download().unwrap(), code.as_bytes());
            assert_eq!(
                arguments
                    .get_channel_as_ref::<String>("_entrypoint")
                    .unwrap(),
                entry
            );
            assert_eq!(
                arguments.get_channel_as_ref::<String>("_sha256").unwrap(),
                "688787d8ff144c502c7f5cffaafe2cc588d86079f9de88304c26b0cb99ce91c6"
            );

            let inner_arguments: ValueStream =
                ValueStream::decode(arguments.get_channel_as_ref::<[u8]>("_arguments").unwrap())
                    .unwrap();

            assert_eq!(
                inner_arguments
                    .get_channel_as_ref::<String>("test-arg")
                    .unwrap(),
                "test-value"
            );

            // Test that we get the execution environment args we supplied earlier
            let fake_exe_args = &fake.executor_context.borrow().arguments.clone();
            assert_eq!(fake_exe_args.len(), 0);
            assert_eq!(
                arguments.get_channel_as_ref::<String>("sune").unwrap(),
                "bune"
            );
        }

        // Test president! 🕴
        {
            let args = stream!({"_code" => "this-is-not-code"}); // deliberately use a reserved word 👿

            let mut runtime_args = HashMap::new();
            runtime_args.insert("the-arg".to_owned(), "this-is-exec-arg".to_owned());
            let result = nested.execute(
                ExecutorParameters {
                    function_name: "test".to_owned(),
                    entrypoint: "entry".to_owned(),
                    code: Some(code_file!(b"code")),
                    arguments: runtime_args,
                },
                args,
                vec![],
            );

            assert!(result.is_ok());
            let args = &fake.arguments.borrow();
            assert!(args.has_channel("_code"));
            assert!(args.has_channel("_sha256"));
            assert!(args.has_channel("_entrypoint"));

            // make sure that we get the actual code and not the argument named _code
            let code_attachment =
                Attachment::decode(args.get_channel_as_ref::<[u8]>("_code").unwrap()).unwrap();
            assert_eq!(
                String::from_utf8(code_attachment.download().unwrap()).unwrap(),
                "code"
            );
        }

        // Double nested 🎰
        let nested2 = FunctionAdapter::new(
            Box::new(nested),
            Function {
                runtime: Some(Runtime {
                    name: "Avlivningsmiljö 🗡️".to_owned(),
                    entrypoint: "ingångspoäng 💯".to_owned(),
                    arguments: HashMap::new(),
                }),
                code: Some(code_file!(s.as_bytes())),
                attachments: vec![],
                name: "mandelkubb".to_owned(),
                version: "2022.1-5-PR50".to_owned(),
                metadata: HashMap::new(),
                input: None,
                output: None,
                created_at: 0u64,
            },
            null_logger!(),
        );
        {
            let args = stream!({"test-arg" => "test-value"});
            let result = nested2.execute(
                ExecutorParameters {
                    function_name: "test".to_owned(),
                    entrypoint: "entry".to_owned(),
                    code: Some(code_file!(b"vec")),
                    arguments: runtime_args,
                },
                args,
                vec![],
            );

            assert!(result.is_ok());
            let args = &fake.arguments.borrow();
            assert!(args.channels.contains_key("_code"));
            assert!(args.channels.contains_key("_sha256"));
            assert!(args.channels.contains_key("_entrypoint"));
        }
    }

    #[test]
    fn test_lookup_executor() {
        // get wasi executor
        let fr = registry!();
        let execution_service = ExecutionService::new(null_logger!(), Box::new(fr.clone()));
        let res =
            futures::executor::block_on(execution_service.lookup_executor_for_runtime("wasi"));
        assert!(res.is_ok());

        // get non existing executor
        let res =
            futures::executor::block_on(execution_service.lookup_executor_for_runtime("ur-sula!"));
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            ExecutorError::ExecutorNotFound(..)
        ));

        // get function executor
        let mut wasi_executor_metadata = HashMap::new();
        wasi_executor_metadata.insert("executor-for".to_owned(), "oran-malifant".to_owned());

        let mut nested_executor_metadata = HashMap::new();
        nested_executor_metadata.insert("executor-for".to_owned(), "precious-granag".to_owned());

        let mut broken_executor_metadata = HashMap::new();
        broken_executor_metadata.insert(
            "executor-for".to_owned(),
            "broken-chain-executor".to_owned(),
        );

        vec![
            FunctionData {
                runtime: Some(Runtime {
                    name: "wasi".to_owned(),
                    entrypoint: "wasi.kexe".to_owned(),
                    arguments: HashMap::new(),
                }),
                code_attachment_id: None,
                name: "oran-func".to_owned(),
                version: "0.1.1".to_owned(),
                metadata: wasi_executor_metadata,
                input: None,
                output: None,
                attachment_ids: vec![],
            },
            FunctionData {
                runtime: Some(Runtime {
                    name: "oran-malifant".to_owned(),
                    entrypoint: "oran hehurr".to_owned(),
                    arguments: HashMap::new(),
                }),
                code_attachment_id: None,
                name: "precious-granag".to_owned(),
                version: "8.1.5".to_owned(),
                metadata: nested_executor_metadata,
                input: None,
                output: None,
                attachment_ids: vec![],
            },
            FunctionData {
                runtime: Some(Runtime {
                    name: "oran-elefant".to_owned(),
                    entrypoint: "oran hehurr".to_owned(),
                    arguments: HashMap::new(),
                }),
                code_attachment_id: None,
                name: "precious-granag".to_owned(),
                version: "3.2.2".to_owned(),
                metadata: broken_executor_metadata,
                input: None,
                output: None,
                attachment_ids: vec![],
            },
        ]
        .into_iter()
        .for_each(|rr| {
            futures::executor::block_on(fr.register(tonic::Request::new(rr)))
                .map_or_else(|e| panic!(e.to_string()), |_| ())
        });

        let res = futures::executor::block_on(
            execution_service.lookup_executor_for_runtime("oran-malifant"),
        );
        assert!(res.is_ok());

        // Get two smetadatae executor
        let res = futures::executor::block_on(
            execution_service.lookup_executor_for_runtime("precious-granag"),
        );
        assert!(res.is_ok());

        // get function executor missing link
        let res = futures::executor::block_on(
            execution_service.lookup_executor_for_runtime("broken-chain-executor"),
        );
        assert!(res.is_err());

        assert!(matches!(
            res.unwrap_err(),
            ExecutorError::ExecutorNotFound(..)
        ));
    }

    #[test]
    fn test_cyclic_dependency_check() {
        let fr = registry!();
        let execution_service = ExecutionService::new(null_logger!(), Box::new(fr.clone()));
        let res =
            futures::executor::block_on(execution_service.lookup_executor_for_runtime("wasi"));
        assert!(res.is_ok());

        // get non existing executor
        let res =
            futures::executor::block_on(execution_service.lookup_executor_for_runtime("ur-sula!"));
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            ExecutorError::ExecutorNotFound(..)
        ));

        // get function executor
        let mut wasi_executor_metadata = HashMap::new();
        wasi_executor_metadata.insert("executor-for".to_owned(), "aa-exec".to_owned());

        let mut nested_executor_metadata = HashMap::new();
        nested_executor_metadata.insert("executor-for".to_owned(), "bb-exec".to_owned());

        let mut broken_executor_metadata = HashMap::new();
        broken_executor_metadata.insert("executor-for".to_owned(), "cc-exec".to_owned());

        vec![
            function_data!(
                "aa-func",
                "0.1.1",
                Runtime {
                    name: "bb-exec".to_owned(),
                    entrypoint: "wasi.kexe".to_owned(),
                    arguments: HashMap::new(),
                },
                { "executor-for" => "aa-exec" }
            ),
            function_data!(
                "bb-func",
                "0.1.5",
                Runtime {
                    name: "cc-exec".to_owned(),
                    entrypoint: "wasi.kexe".to_owned(),
                    arguments: HashMap::new(),
                },
                { "executor-for" => "bb-exec" }
            ),
            function_data!(
                "cc-func",
                "3.2.2",
                Runtime {
                    name: "bb-exec".to_owned(),
                    entrypoint: "wasi.kexe".to_owned(),
                    arguments: HashMap::new(),
                },
                { "executor-for" => "cc-exec" }
            ),
        ]
        .into_iter()
        .for_each(|rr| {
            futures::executor::block_on(fr.register(tonic::Request::new(rr)))
                .map_or_else(|e| panic!(e.to_string()), |_| ())
        });

        let res =
            futures::executor::block_on(execution_service.lookup_executor_for_runtime("aa-exec"));
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            ExecutorError::ExecutorDependencyCycle(..)
        ));
    }
}
