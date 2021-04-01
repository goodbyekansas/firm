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

use crate::executor::{FunctionOutputSink, RuntimeError};

#[derive(Debug)]
pub struct RuntimeParameters {
    pub root_dir: PathBuf,
    pub function_name: String,
    pub entrypoint: Option<String>,
    pub code: Option<Attachment>,
    pub arguments: HashMap<String, String>,
    pub output_sink: FunctionOutputSink,
}

impl RuntimeParameters {
    pub fn new(function_name: &str, root_dir: &Path) -> Self {
        Self {
            function_name: function_name.to_owned(),
            entrypoint: None,
            code: None,
            arguments: HashMap::new(),
            output_sink: FunctionOutputSink::null(),
            root_dir: root_dir.to_owned(),
        }
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
