mod api;
mod error;
mod function;
mod net;
mod output;
mod process;
mod sandbox;

use std::{
    collections::HashMap, fs::OpenOptions, io::LineWriter, path::Path, path::PathBuf, sync::Arc,
    sync::Mutex,
};

use output::{NamedFunctionOutputSink, Output};
use slog::{info, o, Logger};

use wasmer::{imports, ChainableNamedResolver, Function, ImportObject, Instance, Module, Store};
use wasmer_wasi::WasiState;

use super::{Runtime, RuntimeParameters, StreamExt};
use crate::executor::{AttachmentDownload, RuntimeError};
use api::ApiState;
use error::WasiError;
use firm_types::functions::{Attachment, Stream};
use sandbox::Sandbox;

#[derive(Debug, Clone)]
pub struct WasiRuntime {
    logger: Logger,
    host_dirs: HashMap<String, PathBuf>,
}

impl WasiRuntime {
    pub fn new(logger: Logger) -> Self {
        Self {
            logger,
            host_dirs: HashMap::new(),
        }
    }

    pub fn with_host_dir<P>(mut self, wasi_name: &str, host_path: P) -> Self
    where
        P: AsRef<Path>,
    {
        self.host_dirs
            .insert(wasi_name.to_owned(), host_path.as_ref().to_owned());
        self
    }
}

impl From<String> for RuntimeError {
    fn from(message: String) -> Self {
        Self::RuntimeError {
            name: "wasi-wasmer".to_owned(),
            message,
        }
    }
}

fn setup_api_imports(store: &Store, api_state: ApiState) -> ImportObject {
    imports! {
        "firm" => {
            // Host queries
            "host_path_exists" => Function::new_native_with_env(&store, api_state.clone(), api::host::path_exists),
            "get_host_os" => Function::new_native_with_env(&store, api_state.clone(), api::host::get_os),
            "start_host_process" => Function::new_native_with_env(&store, api_state.clone(), api::host::start_process),
            "run_host_process" => Function::new_native_with_env(&store, api_state.clone(), api::host::run_process),
            "connect" => Function::new_native_with_env(&store, api_state.clone(), api::host::socket_connect),

            // Attachments
            "get_attachment_path_len" => Function::new_native_with_env(&store, api_state.clone(), api::attachments::get_path_len),
            "map_attachment" => Function::new_native_with_env(&store, api_state.clone(), api::attachments::map),
            "get_attachment_path_len_from_descriptor" => Function::new_native_with_env(&store, api_state.clone(), api::attachments::get_path_len_from_descriptor),
            "map_attachment_from_descriptor" => Function::new_native_with_env(&store, api_state.clone(), api::attachments::map_from_descriptor),

            // Connections
            "get_input_len" => Function::new_native_with_env(&store, api_state.clone(), api::connections::get_input_len),
            "get_input" => Function::new_native_with_env(&store, api_state.clone(), api::connections::get_input),
            "set_output" => Function::new_native_with_env(&store, api_state.clone(), api::connections::set_output),
            "set_error" => Function::new_native_with_env(&store, api_state, api::connections::set_error),
        }
    }
}

impl Runtime for WasiRuntime {
    fn execute(
        &self,
        runtime_parameters: RuntimeParameters,
        arguments: Stream,
        attachments: Vec<Attachment>,
    ) -> Result<Result<Stream, String>, RuntimeError> {
        let function_logger = self
            .logger
            .new(o!("function" => runtime_parameters.function_name.to_owned()));

        let sandbox = Sandbox::new(&runtime_parameters.root_dir, Path::new("sandbox"))
            .map_err(|e| e.to_string())?;
        let attachment_sandbox =
            Sandbox::new(&runtime_parameters.root_dir, Path::new("attachments"))
                .map_err(|e| e.to_string())?;

        info!(
            function_logger,
            "using sandbox directory: {}",
            sandbox.host_path().display()
        );
        info!(
            function_logger,
            "using sandbox attachments directory: {}",
            attachment_sandbox.host_path().display()
        );

        let stdout = Output::new(vec![
            Box::new(
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(sandbox.host_path().join("stdout"))
                    .map_err(|e| format!("Failed to create stdout sandbox file: {}", e))?,
            ),
            Box::new(LineWriter::new(NamedFunctionOutputSink::new(
                "stdout",
                runtime_parameters.output_sink.clone(),
                function_logger.new(o!("output-sink" => "stdout")),
            ))),
        ]);

        let stderr = Output::new(vec![
            Box::new(
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(sandbox.host_path().join("stderr"))
                    .map_err(|e| format!("Failed to create stderr sandbox file: {}", e))?,
            ),
            Box::new(LineWriter::new(NamedFunctionOutputSink::new(
                "stderr",
                runtime_parameters.output_sink,
                function_logger.new(o!("output-sink" => "stderr")),
            ))),
        ]);

        let mut wasi_env = WasiState::new(&format!("wasi-{}", runtime_parameters.function_name))
            .stdout(Box::new(stdout.clone()))
            .stderr(Box::new(stderr.clone()))
            .preopen(|p| {
                p.directory(sandbox.host_path())
                    .alias(&sandbox.guest_path().to_string_lossy())
                    .read(true)
                    .write(true)
                    .create(true)
            })
            .and_then(|state| {
                state.preopen(|p| {
                    p.directory(attachment_sandbox.host_path())
                        .alias(&attachment_sandbox.guest_path().to_string_lossy())
                        .read(true)
                        .write(false)
                        .create(false)
                })
            })
            .and_then(|state| {
                self.host_dirs
                    .iter()
                    .try_fold(state, |current_state, (alias, host_path)| {
                        current_state.preopen(|p| {
                            p.directory(host_path)
                                .alias(alias)
                                .read(true)
                                .write(false)
                                .create(false)
                        })
                    })
            })
            .and_then(|state| state.finalize())
            .map_err(|e| format!("Failed to create wasi state: {:?}", e))?;

        let results = Arc::new(Mutex::new(Stream::new()));
        let errors = Arc::new(Mutex::new(Vec::new()));

        let api_state = ApiState {
            arguments: Arc::new(arguments),
            attachments: Arc::new(attachments),
            sandbox,
            attachment_sandbox,
            logger: function_logger.new(o!("scope" => "api")),
            stdout,
            stderr,
            results: results.clone(),
            errors: errors.clone(),
            wasi_env: wasi_env.clone(),
            auth_service: runtime_parameters.auth_service.clone(),
            async_runtime: runtime_parameters.async_runtime.handle().clone(),
        };

        let entrypoint = runtime_parameters
            .entrypoint
            .unwrap_or_else(|| String::from("_start"));

        let store = Store::default();
        let module = Module::new(
            &store,
            runtime_parameters.async_runtime.block_on(
                runtime_parameters
                    .code
                    .ok_or_else(|| RuntimeError::MissingCode("wasi".to_owned()))?
                    .download(&runtime_parameters.auth_service),
            )?,
        )
        .map_err(|e| format!("failed to compile wasm: {}", e))?;

        Instance::new(
            &module,
            &wasi_env
                .import_object(&module)
                .map_err(|e| format!("Failed to generate import object: {}", e))?
                .chain_back(setup_api_imports(&store, api_state)),
        )
        .map_err(|e| format!("failed to instantiate WASI module: {}", e))?
        .exports
        .get_function(&entrypoint)
        .map_err(|e| format!("Failed to resolve entrypoint {}: {}", &entrypoint, e))?
        .call(&[])
        .map_err(|e| format!("Failed to call entrypoint function {}: {}", &entrypoint, e))?;

        let results = Arc::try_unwrap(results)
            .map_err(|e| {
                format!(
            "Failed to get function results. There are still {} references to the results stream.",
            Arc::strong_count(&e)
        )
            })?
            .into_inner()
            .map_err(|e| format!("Failed to acquire lock for results: {}", e))?;

        let errors = Arc::try_unwrap(errors)
            .map_err(|e| {
                format!(
            "Failed to get function errors. There are still {} references to the errors vector.",
            Arc::strong_count(&e)
        )
            })?
            .into_inner()
            .map_err(|e| format!("Failed to acquire lock for errors: {}", e))?;

        Ok(if errors.is_empty() {
            Ok(results)
        } else {
            Err(errors.join("\n"))
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::{auth::AuthService, executor::FunctionOutputSink};

    use super::*;
    use firm_types::{code_file, stream};

    macro_rules! null_logger {
        () => {{
            slog::Logger::root(slog::Discard, o!())
        }};
    }

    #[test]
    fn test_execution() {
        let tmp_fold = tempfile::tempdir().unwrap();
        let executor = WasiRuntime::new(null_logger!());
        let res = executor.execute(
            RuntimeParameters {
                root_dir: tmp_fold.path().to_owned(),
                function_name: "hello-world".to_owned(),
                entrypoint: None, // use default entrypoint _start
                code: Some(code_file!(include_bytes!("hello.wasm"))),
                arguments: std::collections::HashMap::new(),
                output_sink: FunctionOutputSink::null(),
                auth_service: AuthService::default(),
                async_runtime: tokio::runtime::Builder::new_current_thread()
                    .build()
                    .unwrap(),
            },
            stream!(),
            vec![],
        );

        assert!(res.is_ok());
        assert!(res.unwrap().is_ok());
    }
}
