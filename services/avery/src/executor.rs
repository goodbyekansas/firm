use std::{collections::HashMap, io::Write, path::Path, path::PathBuf, sync::Arc, task::Poll};

use firm_protocols::{
    functions::{
        execution_server::Execution as ExecutionServiceTrait,
        function_input::Payload as InputPayload, function_output::Result as OutputPayload,
        ChannelPacket, ExecutionError, ExecutionId, FunctionInput, FunctionLogMessage,
        FunctionOutput, FunctionUri, Runtime, RuntimeFilters, RuntimeList,
    },
    tonic::{self, Status},
};
use futures::{FutureExt, StreamExt, TryFutureExt, TryStreamExt};
use rayon::ThreadPool;
use runtime::{FsCache, FsStore, QueuedFunction, Store};
use slog::{error, info, o, warn, Logger};
use tempfile::{Builder, TempDir};
use tokio::sync::{mpsc::Sender, Mutex};
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

use function::{
    io::PollRead,
    stream::{ChannelWriter, RWChannelStream, Stream as ValueStream},
};

pub struct ExecutionService {
    logger: Logger,
    execution_queue: Arc<Mutex<HashMap<Uuid, QueuedFunction<FsCache>>>>, // Death row hehurr
    thread_pool: Arc<ThreadPool>,
    store: FsStore,
    cache_dir: TempDir,
}

impl ExecutionService {
    pub fn new(
        log: Logger,
        runtime_directories: Vec<PathBuf>,
        root_dir: &Path,
    ) -> Result<Self, String> {
        Ok(Self {
            logger: log,
            execution_queue: Arc::new(Mutex::new(HashMap::new())),
            store: FsStore::new(root_dir, &runtime_directories),
            thread_pool: Arc::new(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(num_cpus::get())
                    .thread_name(|tid| format!("function-execution-thread-{}", tid))
                    .build()
                    .map_err(|e| {
                        format!("Failed to create function execution thread pool: {}", e)
                    })?,
            ),
            cache_dir: Builder::new()
                .prefix("avery-runtime-cache")
                .tempdir()
                .map_err(|e| format!("Failed to create cache directory: {}", e))?,
        })
    }

    fn blocking_function_error<T: AsRef<str>>(
        sender: Sender<Result<FunctionOutput, Status>>,
        logger: &Logger,
        msg: T,
    ) {
        let res: Result<FunctionOutput, tonic::Status> = Ok(FunctionOutput {
            result: Some(OutputPayload::Error(ExecutionError {
                msg: msg.as_ref().to_string(),
            })),
        });
        if let Err(e) = sender.blocking_send(res) {
            warn!(logger, "Error sending error to function output: {}", e);
        }
    }

    async fn function_error<T: AsRef<str>>(
        sender: Sender<Result<FunctionOutput, Status>>,
        logger: &Logger,
        msg: T,
    ) {
        let res: Result<FunctionOutput, tonic::Status> = Ok(FunctionOutput {
            result: Some(OutputPayload::Error(ExecutionError {
                msg: msg.as_ref().to_string(),
            })),
        });
        if let Err(e) = sender.send(res).await {
            warn!(logger, "Error sending error to function output: {}", e);
        }
    }
}

#[tonic::async_trait]
impl ExecutionServiceTrait for ExecutionService {
    type RunFunctionStream = ReceiverStream<Result<FunctionOutput, tonic::Status>>;
    type FunctionLogStream = tonic::Streaming<FunctionLogMessage>;

    async fn queue_function(
        &self,
        request: tonic::Request<FunctionUri>,
    ) -> Result<tonic::Response<ExecutionId>, tonic::Status> {
        let payload = request.into_inner();

        futures::future::ready(
            payload.uri.parse::<registry::FunctionUrl>().map_err(|e| {
                tonic::Status::invalid_argument(format!("Invalid function URI: {}", e))
            }),
        )
        .and_then(|function_url| async move {
            registry::resolve(&function_url)
                .map_err(|e| {
                    tonic::Status::internal(format!(
                        "Error resolving function {}: {}",
                        function_url, e
                    ))
                })
                .await
                .and_then(|res| {
                    res.ok_or_else(|| {
                        tonic::Status::not_found(format!(
                            "Function with URI: {} not found",
                            payload.uri
                        ))
                    })
                })
        })
        .and_then(|function| async move {
            self.store
                .execute_function(&function, Some(FsCache::new(self.cache_dir.as_ref())))
                .map_err(|e| tonic::Status::unknown(format!("{}", e)))
        })
        .and_then(|queued_function| {
            self.execution_queue.lock().then(move |mut g| async move {
                let execution_id = Uuid::new_v4();
                let _ = g.insert(execution_id, queued_function);

                Ok(tonic::Response::new(ExecutionId {
                    uuid: execution_id.to_string(),
                }))
            })
        })
        .await
    }

    async fn run_function(
        &self,
        request: tonic::Request<tonic::Streaming<FunctionInput>>,
    ) -> Result<tonic::Response<Self::RunFunctionStream>, tonic::Status> {
        let mut input_packets = request.into_inner();
        let (uuid, strict) = match input_packets.next().await {
            Some(Ok(pkt)) => match pkt.payload {
                Some(InputPayload::Header(header)) => header
                    .id
                    .ok_or_else(|| tonic::Status::invalid_argument("Expected header to contain id"))
                    .and_then(|id| {
                        Uuid::parse_str(&id.uuid)
                            .map_err(|e| {
                                tonic::Status::invalid_argument(format!(
                                    "Failed to parse execution id as uuid: {}.",
                                    e
                                ))
                            })
                            .map(|uuid| (uuid, header.strict))
                    }),
                _ => Err(tonic::Status::invalid_argument(
                    "input stream needs to contain execution id in first packet",
                )),
            },
            Some(Err(e)) => Err(e),
            None => Err(tonic::Status::invalid_argument("empty input stream")),
        }?;

        let queued: QueuedFunction<FsCache> = self
            .execution_queue
            .lock()
            .await
            .get(&uuid)
            .ok_or_else(|| {
                tonic::Status::not_found(format!(
                    "Could not find queued function with execution id: {}",
                    uuid
                ))
            })?
            .clone();

        let inputs = RWChannelStream::new_from_specs(queued.function().inputs.clone());
        let input_writers = inputs.writers();
        let input_writers2 = inputs.writers();
        let outputs = RWChannelStream::new_from_specs(queued.function().outputs.clone());
        let output_readers = outputs.readers();
        let (tx, rx) = tokio::sync::mpsc::channel(128);
        let function_name = queued.function().name.clone();
        info!(self.logger, "executing function {}", uuid);

        self.thread_pool.spawn({
            let tx = tx.clone();
            let logger = self
                .logger
                .new(o!("function" => queued.function().name.clone()));
            move || match queued.run(inputs, outputs) {
                Ok(_) => {}
                Err(re) => {
                    let res: Result<FunctionOutput, tonic::Status> = Ok(FunctionOutput {
                        result: Some(OutputPayload::Error(ExecutionError {
                            msg: re.to_string(),
                        })),
                    });
                    if let Err(e) = tx.blocking_send(res) {
                        error!(logger, "Error sending error to function output: {}", e);
                    }
                }
            }
        });

        tokio::spawn({
            let inputs_tx = tx.clone();
            let inputs_tx2 = tx.clone();
            let function_name = function_name.clone();
            let function_name2 = function_name.clone();
            let inputs_log = self
                .logger
                .new(o!("scope" => "read-inputs", "function" => function_name.clone()));
            let inputs_log2 = self
                .logger
                .new(o!("scope" => "close-inputs", "function" => function_name.clone()));
            input_packets
                .try_for_each(move |packet| {
                    let writers = input_writers.clone();
                    let inputs_log = inputs_log.new(o!());
                    let function_name = function_name.clone();
                    let inputs_tx = inputs_tx.clone();
                    async move {
                        match packet.payload {
                            Some(InputPayload::Data(d)) => {
                                let inputs_log = inputs_log.new(o!("channel" => d.channel.clone()));
                                if let Some(writer) = writers.iter().find(|w| w.channel_id() == d.channel) {
                                    match writer.clone().write(&d.data) {
                                        Ok(_) => {
                                            Ok(())
                                        }
                                        Err(e) => {
                                            let error_message = format!("Got error when trying to write to input \"{}\": {}", d.channel, e);
                                            warn!(inputs_log, "{}", error_message);

                                            if strict {
                                                ExecutionService::function_error(inputs_tx.clone(), &inputs_log, error_message).await;
                                            }

                                            Ok(())
                                        }
                                    }
                                } else {
                                    let error_message = format!("Received input for channel \"{}\" which function \"{}\" does not have.", d.channel, function_name);
                                    warn!(inputs_log, "{}", error_message);

                                    if strict {
                                        ExecutionService::function_error(inputs_tx.clone(), &inputs_log, error_message).await;
                                    }

                                    Ok(())
                                }
                            }
                            Some(InputPayload::Header(_)) => {
                                warn!(inputs_log, "Stray header packet detected inside the input stream");
                                Ok(())
                            },
                            _ => Ok(()),
                        }
                    }
                })
                .and_then(move |_| {
                    let writers = input_writers2.clone();
                    async move {
                        if let Err(e) = writers
                            .into_iter()
                            .try_for_each(|writer| writer.close()) {
                                ExecutionService::function_error(inputs_tx2.clone(), &inputs_log2, format!(
                                    "Failed to close channel for function \"{}\": {}",
                                    function_name2,
                                    e
                                )).await;
                            }
                        Ok(())
                    }
                })
        });

        let output_logger = self
            .logger
            .new(o!("function" => function_name.clone(), "scope" => "write-outputs"));
        self.thread_pool.spawn(move || {
            let mut buf = [0u8; 1024];
            let mut readers = output_readers;
            loop {
                readers = readers
                    .into_iter()
                    .filter_map(
                        |mut output_reader| match output_reader.poll_read(&mut buf) {
                            Ok(Poll::Ready(0)) => None,
                            Ok(Poll::Ready(read)) => {
                                let res: Result<FunctionOutput, tonic::Status> =
                                    Ok(FunctionOutput {
                                        result: Some(OutputPayload::Ok(ChannelPacket {
                                            channel: output_reader.channel_id().to_owned(),
                                            data: {
                                                let mut data = Vec::new();
                                                data.copy_from_slice(&buf[0..read]);
                                                data
                                            },
                                        })),
                                    });

                                if let Err(e) = tx.blocking_send(res) {
                                    warn!(
                                        output_logger,
                                        "Failed to write channel data to channel \"{}\": {}",
                                        output_reader.channel_id().to_owned(),
                                        e
                                    )
                                }

                                Some(output_reader)
                            }
                            Err(e) => {
                                ExecutionService::blocking_function_error(
                                    tx.clone(),
                                    &output_logger,
                                    format!(
                                        "Failed to read function output \
                                             channel {} from function {}: {}",
                                        output_reader.channel_id().to_owned(),
                                        function_name.clone(),
                                        e
                                    ),
                                );
                                Some(output_reader)
                            }
                            _ => Some(output_reader),
                        },
                    )
                    .collect();
            }
        });

        Ok(tonic::Response::new(ReceiverStream::new(rx)))
    }

    async fn function_log(
        &self,
        _request: tonic::Request<ExecutionId>,
    ) -> Result<tonic::Response<Self::FunctionLogStream>, tonic::Status> {
        todo!();
    }

    async fn list_runtimes(
        &self,
        request: tonic::Request<RuntimeFilters>,
    ) -> Result<tonic::Response<RuntimeList>, tonic::Status> {
        let payload = request.into_inner();
        Ok(tonic::Response::new(RuntimeList {
            runtimes: self
                .store
                .list_runtimes()
                .map(|runtimes| {
                    runtimes
                        .into_iter()
                        .filter_map(|p| {
                            let name = p
                                .file_name()
                                .map(|s| s.to_string_lossy().to_string())
                                .unwrap_or_else(|| String::from("invalid-file-name"));

                            name.contains(&payload.name).then(|| Runtime {
                                name,
                                source: p.to_string_lossy().to_string(),
                            })
                        })
                        .collect()
                })
                .map_err(|e| tonic::Status::internal(format!("Failed to list runtimes: {}", e)))?,
        }))
    }
}
