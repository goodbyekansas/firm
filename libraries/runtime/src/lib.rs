use std::{
    collections::HashMap,
    fmt::Display,
    io::{Read, Write},
    os::unix::prelude::RawFd,
    path::PathBuf,
    process::{Command, Stdio},
    str::FromStr,
};

use firm_protocols::functions::{Attachment, Function};
use serde::Serialize;

mod io_event_queue;

use io_event_queue::{IoEventQueue, IoId, IoReader, IoReaderFactory, IoWriter, IoWriterFactory};

#[derive(Clone)]
pub struct Store {
    root_path: PathBuf,
}

pub struct ExecutionId(uuid::Uuid);

impl FromStr for ExecutionId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(uuid::Uuid::parse_str(s).map_err(|e| e.to_string())?))
    }
}

#[derive(Debug)]
pub struct StoreError {}

// ---------------------- TODO: MOVE THESE to libfunctionio ----------
#[derive(Clone)]
struct FunctionChannel {}

impl FunctionChannel {
    pub fn readable(&self) -> bool {
        todo!()
    }

    pub fn writeable(&self) -> bool {
        todo!()
    }
}

impl Read for FunctionChannel {
    fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
        todo!()
    }
}

impl Write for FunctionChannel {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        todo!()
    }

    fn flush(&mut self) -> std::io::Result<()> {
        todo!()
    }
}

// TODO: move
pub struct FunctionStream {}

impl FunctionStream {
    fn get_channel(&self, _channel: &str) -> FunctionChannel {
        todo!()
    }
}

// ----------------------------------------------------------------------

struct AttachmentReader {}

impl Read for AttachmentReader {
    fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
        todo!()
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub struct FunctionId {
    name: String,
    version: String,
}

impl From<&Function> for FunctionId {
    fn from(f: &Function) -> Self {
        Self {
            name: f.name.to_owned(),
            version: f.version.to_owned(),
        }
    }
}

impl FromStr for FunctionId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split('-');
        Ok(Self {
            name: parts
                .next()
                .map(|s| s.to_owned())
                .ok_or_else(|| String::from("NONAME"))?,
            version: parts
                .next()
                .map(|s| s.to_owned())
                .ok_or_else(|| String::from("NOVERSION"))?,
        })
    }
}

impl Display for FunctionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.name, self.version)
    }
}

struct FunctionStreamReaderFactory {
    channel: FunctionChannel,
}

impl IoReaderFactory for FunctionStreamReaderFactory {
    fn create(&self) -> Box<dyn IoReader> {
        Box::new(self.channel.clone())
    }
}

impl IoReader for FunctionChannel {
    fn readable(&self) -> bool {
        self.readable()
    }
}

impl IoWriter for FunctionChannel {
    fn writeable(&self) -> bool {
        self.writeable()
    }
}

struct FunctionStreamWriterFactory {
    channel: FunctionChannel,
}

impl IoWriterFactory for FunctionStreamWriterFactory {
    fn create(&self) -> Box<dyn IoWriter> {
        Box::new(self.channel.clone())
    }
}

struct AttachmentReaderFactory {
    function_id: FunctionId,
    attachment_name: String,
    store: Store,
}

impl IoReaderFactory for AttachmentReaderFactory {
    fn create(&self) -> Box<dyn io_event_queue::IoReader> {
        Box::new(
            self.store
                .attachment_reader(&self.function_id, &self.attachment_name)
                .unwrap(),
        )
    }
}

impl IoReader for AttachmentReader {
    fn readable(&self) -> bool {
        true
    }
}

#[derive(Serialize)]
struct FunctionContext {
    name: String,
    inputs: HashMap<String, u64>,
    outputs: HashMap<String, u64>,
    attachments: HashMap<String, u64>,

    submission_fd: RawFd,
    completion_fd: RawFd,
    event_queue_size: u32,
}

impl Store {
    pub fn function_executions(
        &self,
        function_id: &FunctionId,
    ) -> Result<Vec<ExecutionId>, StoreError> {
        self.root_path
            .join(function_id.to_string())
            .read_dir()
            .unwrap()
            .into_iter()
            .filter_map(|de| {
                de.ok()
                    .map(|entry| ExecutionId::from_str(&entry.file_name().to_string_lossy()))
            })
            .collect::<Result<Vec<_>, String>>()
            .map_err(|_| StoreError {})
    }

    pub fn execute_function(
        &self,
        function: &Function,
        inputs: FunctionStream,
        outputs: FunctionStream,
    ) -> Result<(), StoreError> {
        let mut event_queue = IoEventQueue::new(32);
        let ctx = FunctionContext {
            name: function.name.to_owned(),
            inputs: function
                .required_inputs
                .iter()
                .map(|(name, _)| {
                    let id = IoId::generate_read();
                    event_queue.register_reader(
                        id,
                        Box::new(FunctionStreamReaderFactory {
                            channel: inputs.get_channel(name),
                        }),
                    );
                    (name.to_owned(), id.raw())
                })
                .collect(),
            outputs: function
                .outputs
                .iter()
                .map(|(name, _)| {
                    let id = IoId::generate_write();
                    event_queue.register_writer(
                        id,
                        Box::new(FunctionStreamWriterFactory {
                            channel: outputs.get_channel(name),
                        }),
                    );
                    (name.to_owned(), id.raw())
                })
                .collect(),
            attachments: function
                .attachments
                .iter()
                .map(|attachment| {
                    let id = IoId::generate_read();
                    event_queue.register_reader(
                        id,
                        Box::new(AttachmentReaderFactory {
                            function_id: FunctionId::from(function),
                            attachment_name: attachment.name.clone(),
                            store: self.clone(),
                        }),
                    );
                    (attachment.name.clone(), id.raw())
                })
                .collect(),

            submission_fd: event_queue.submission_fd(),
            completion_fd: event_queue.completion_fd(),
            event_queue_size: 32,
        };
        let mut runtime_process = Command::new("/path/to/a/runtime.runtime")
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();

        let stdin = runtime_process.stdin.take().unwrap();
        serde_json::to_writer(&stdin, &ctx).unwrap();
        drop(stdin);

        loop {
            match runtime_process.try_wait() {
                Ok(Some(_ec)) => {
                    // process exited normally
                    break;
                }
                Ok(None) => while event_queue.update() {},
                Err(_) => {
                    // wtf
                    break;
                }
            }
        }

        Ok(())
    }

    pub fn functions(&self) -> Result<Vec<FunctionId>, StoreError> {
        self.root_path
            .read_dir()
            .unwrap()
            .into_iter()
            .filter_map(|dir| {
                dir.ok()
                    .map(|de| de.file_name().to_string_lossy().parse::<FunctionId>())
            })
            .collect::<Result<Vec<_>, String>>()
            .map_err(|_| StoreError {})
    }

    pub fn attachment(
        &self,
        _function_id: &FunctionId,
        _attachment_name: &str,
    ) -> Result<Attachment, StoreError> {
        todo!()
    }

    fn attachment_reader(
        &self,
        _function: &FunctionId,
        _attachment_name: &str,
    ) -> Result<AttachmentReader, StoreError> {
        todo!()
    }

    pub fn attachments<'a>(&self, function: &'a Function) -> Result<&'a [Attachment], StoreError> {
        Ok(&function.attachments)
    }
}
