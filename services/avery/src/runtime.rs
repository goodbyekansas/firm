pub mod wasi;

use std::{collections::HashMap, fmt::Debug};

use firm_types::{
    functions::Strings,
    functions::{
        channel::Value as ValueType, Attachment, Channel, Function, Stream as ValueStream,
    },
    stream::{StreamExt, ToChannel},
    wasi::Attachments,
};
use prost::Message;
use slog::{o, Logger};

use crate::executor::RuntimeError;

#[derive(Default, Debug)]
pub struct RuntimeParameters {
    pub function_name: String,
    pub entrypoint: String,
    pub code: Option<Attachment>,
    pub arguments: HashMap<String, String>,
}

pub trait Runtime: Debug {
    fn execute(
        &self,
        executor_context: RuntimeParameters,
        arguments: ValueStream,
        attachments: Vec<Attachment>,
    ) -> Result<Result<ValueStream, String>, RuntimeError>;
}

pub trait RuntimeSource: Send + Sync {
    fn get(&self, name: &str) -> Option<Box<dyn Runtime>>;
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
}

#[derive(Debug)]
pub struct FunctionAdapter {
    executor: Box<dyn Runtime>,
    executor_function: Function,
    logger: Logger,
}

/// Adapter for functions to act as executors
impl FunctionAdapter {
    pub fn new(executor: Box<dyn Runtime>, function: Function, logger: Logger) -> Self {
        Self {
            executor,
            executor_function: function,
            logger,
        }
    }
}

impl Runtime for FunctionAdapter {
    fn execute(
        &self,
        executor_context: RuntimeParameters,
        arguments: ValueStream,
        attachments: Vec<Attachment>,
    ) -> Result<Result<ValueStream, String>, RuntimeError> {
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

            executor_function_arguments.set_channel("_code", code_buf.to_channel());

            let checksums = code.checksums.ok_or(RuntimeError::MissingChecksums)?;
            executor_function_arguments.set_channel("_sha256", checksums.sha256.to_channel());
        }

        executor_function_arguments
            .set_channel("_entrypoint", executor_context.entrypoint.to_channel());

        // nest arguments and attachments
        let mut arguments_buf: Vec<u8> = Vec::with_capacity(arguments.encoded_len());
        arguments.encode(&mut arguments_buf)?;
        executor_function_arguments.set_channel("_arguments", arguments_buf.to_channel());

        let proto_attachments = Attachments { attachments };
        let mut attachments_buf: Vec<u8> = Vec::with_capacity(proto_attachments.encoded_len());
        proto_attachments.encode(&mut attachments_buf)?;
        executor_function_arguments.set_channel("_attachments", attachments_buf.to_channel());

        let function_exe_env = self
            .executor_function
            .runtime
            .clone()
            .ok_or_else(|| RuntimeError::MissingRuntime(self.executor_function.name.clone()))?;

        self.executor.execute(
            RuntimeParameters {
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
