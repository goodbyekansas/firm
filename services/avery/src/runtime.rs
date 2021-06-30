pub mod filesystem_source;
pub mod wasi;

use std::{
    collections::HashMap,
    fmt::Debug,
    path::{Path, PathBuf},
};

use firm_types::{
    functions::{Attachment, Stream as ValueStream},
    stream::StreamExt,
};
use slog::{o, Logger};
use tokio::runtime::Runtime as TokioRuntime;

use crate::{
    auth::AuthService,
    executor::{FunctionOutputSink, RuntimeError},
};

#[derive(Debug)]
pub struct RuntimeParameters {
    pub function_dir: FunctionDirectory,
    pub function_name: String,
    pub entrypoint: Option<String>,
    pub code: Option<Attachment>,
    pub arguments: HashMap<String, String>,
    pub output_sink: FunctionOutputSink,
    pub auth_service: AuthService,
    pub async_runtime: TokioRuntime,
}

#[derive(Debug, Clone)]
pub struct FunctionDirectory {
    root_path: PathBuf,
    attachments_path: PathBuf,
    cache_path: PathBuf,
    execution_path: PathBuf,
}

impl FunctionDirectory {
    pub fn new(
        root: &Path,
        function_name: &str,
        function_version: &str,
        checksum: &str,
        execution_id: &str,
    ) -> std::io::Result<Self> {
        let root_path = root.join(format!(
            "{name}-{version}-{checksum}",
            name = function_name,
            version = function_version,
            checksum = checksum
        ));
        let attachments_path = root_path.join("attachments");
        let cache_path = root_path.join("cache");
        let execution_path = root_path.join(execution_id);

        std::fs::create_dir_all(&attachments_path)?;
        std::fs::create_dir_all(&cache_path)?;
        std::fs::create_dir_all(&execution_path)?;

        Ok(Self {
            root_path,
            attachments_path,
            cache_path,
            execution_path,
        })
    }

    pub fn attachments_path(&self) -> &Path {
        &self.attachments_path
    }

    pub fn execution_path(&self) -> &Path {
        &self.execution_path
    }

    pub fn cache_path(&self) -> &Path {
        &self.cache_path
    }
}

impl RuntimeParameters {
    pub fn new(function_name: &str, execution_dir: FunctionDirectory) -> Result<Self, String> {
        Ok(Self {
            function_name: function_name.to_owned(),
            entrypoint: None,
            code: None,
            arguments: HashMap::new(),
            output_sink: FunctionOutputSink::null(),
            function_dir: execution_dir,
            auth_service: AuthService::default(),
            async_runtime: tokio::runtime::Builder::new_current_thread()
                .build()
                .map_err(|e| e.to_string())?,
        })
    }

    pub fn entrypoint(mut self, entrypoint: &str) -> Self {
        self.entrypoint = Some(entrypoint.to_owned());
        self
    }

    pub fn code(mut self, code: Attachment) -> Self {
        self.code = Some(code);
        self
    }

    pub fn arguments(mut self, arguments: HashMap<String, String>) -> Self {
        self.arguments = arguments;
        self
    }

    pub fn output_sink(mut self, output_sink: FunctionOutputSink) -> Self {
        self.output_sink = output_sink;
        self
    }

    pub fn auth_service(mut self, auth_service: AuthService) -> Self {
        self.auth_service = auth_service;
        self
    }
}

pub trait Runtime: Debug + Send {
    fn execute(
        &self,
        runtime_parameters: RuntimeParameters,
        arguments: ValueStream,
        attachments: Vec<Attachment>,
    ) -> Result<Result<ValueStream, String>, RuntimeError>;
}

pub trait RuntimeSource: Send + Sync {
    fn get(&self, name: &str) -> Option<Box<dyn Runtime>>;
    fn list(&self) -> Vec<String>;
    fn name(&self) -> &'static str;
}

#[derive(Debug)]
pub struct InternalRuntimeSource {
    logger: Logger,
}

impl InternalRuntimeSource {
    pub fn new(logger: Logger) -> Self {
        Self { logger }
    }
}

impl RuntimeSource for InternalRuntimeSource {
    fn get(&self, name: &str) -> Option<Box<dyn Runtime>> {
        match name {
            "wasi" => Some(Box::new(wasi::WasiRuntime::new(
                self.logger.new(o!("runtime" => "wasi")),
            ))),
            _ => None,
        }
    }

    fn list(&self) -> Vec<String> {
        vec!["wasi".to_owned()]
    }

    fn name(&self) -> &'static str {
        "internal"
    }
}
