use std::{
    any::Any,
    fmt::Debug,
    io::{self, Write},
    sync::Arc,
};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use slog::{warn, Logger};
use wasi_common::{
    file::WasiFile,
    file::{FdFlags, FileType},
    Error as WasmtimeError, ErrorExt,
};

use crate::executor::FunctionOutputSink;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Output {
    #[serde(skip)]
    sinks: Arc<Mutex<Vec<Box<dyn OutputSink>>>>,
}

pub trait OutputSink: Write + Send + Sync + Debug {}

impl Output {
    pub fn new(sinks: Vec<Box<dyn OutputSink>>) -> Self {
        Self {
            sinks: Arc::new(Mutex::new(sinks)),
        }
    }
}

#[async_trait::async_trait]
impl WasiFile for Output {
    fn as_any(&self) -> &dyn Any {
        self
    }

    async fn get_filetype(&mut self) -> Result<FileType, WasmtimeError> {
        Ok(FileType::CharacterDevice)
    }

    fn isatty(&mut self) -> bool {
        false
    }

    async fn sock_accept(&mut self, _fdflags: FdFlags) -> Result<Box<dyn WasiFile>, WasmtimeError> {
        Err(WasmtimeError::badf())
    }

    async fn datasync(&mut self) -> Result<(), WasmtimeError> {
        self.sync().await
    }

    async fn sync(&mut self) -> Result<(), WasmtimeError> {
        Write::flush(&mut self).map_err(Into::into)
    }

    async fn write_vectored<'a>(
        &mut self,
        bufs: &[std::io::IoSlice<'a>],
    ) -> Result<u64, WasmtimeError> {
        Write::write_vectored(&mut self, bufs)
            .map_err(Into::into)
            .map(|bytes| bytes as u64)
    }

    async fn writable(&self) -> Result<(), WasmtimeError> {
        Ok(())
    }
}

#[derive(Debug)]
pub struct NamedFunctionOutputSink {
    name: String,
    sink: FunctionOutputSink,
    logger: Logger,
}

impl NamedFunctionOutputSink {
    pub fn new(name: &str, sink: FunctionOutputSink, logger: Logger) -> Self {
        Self {
            name: name.to_owned(),
            sink,
            logger,
        }
    }
}

impl<T> OutputSink for T where T: Write + Sync + Send + Debug {}

impl Write for NamedFunctionOutputSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.sink.send(
            self.name.clone(),
            String::from_utf8(buf.to_vec()).unwrap_or_else(|_| {
                warn!(
                    self.logger,
                    "Failed to convert bytes to utf-8 when trying to write to sink."
                );
                String::new()
            }),
        );
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Write for Output {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.sinks
            .lock()
            .iter_mut()
            .try_for_each(|s| s.write(buf).map(|_| ()))
            .map(|_| buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.sinks
            .lock()
            .iter_mut()
            .try_for_each(|s| s.flush())
    }
}
