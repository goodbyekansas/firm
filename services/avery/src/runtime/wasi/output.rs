use serde::{Deserialize, Serialize};
use slog::{warn, Logger};
use std::{
    fmt::Debug,
    io::{self, Read, Seek, Write},
    sync::Arc,
    sync::Mutex,
};
use wasmer_wasi::{WasiFile, WasiFsError};

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

#[typetag::serde]
impl WasiFile for Output {
    fn last_accessed(&self) -> wasmer_wasi::types::__wasi_timestamp_t {
        0
    }

    fn last_modified(&self) -> wasmer_wasi::types::__wasi_timestamp_t {
        0
    }

    fn created_time(&self) -> wasmer_wasi::types::__wasi_timestamp_t {
        0
    }

    fn size(&self) -> u64 {
        0
    }

    fn set_len(
        &mut self,
        _new_size: wasmer_wasi::types::__wasi_filesize_t,
    ) -> Result<(), WasiFsError> {
        Err(WasiFsError::NotAFile)
    }

    fn unlink(&mut self) -> Result<(), WasiFsError> {
        Ok(())
    }

    fn bytes_available(&self) -> Result<usize, WasiFsError> {
        Ok(0)
    }
}

impl Seek for Output {
    fn seek(&mut self, _pos: io::SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(io::ErrorKind::Other, "can not seek output"))
    }
}

impl Read for Output {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::Other,
            "can not read from output",
        ))
    }
}

impl Write for Output {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.sinks
            .lock()
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::Other,
                    "Failed to aquire write lock for output sinks.",
                )
            })?
            .iter_mut()
            .try_for_each(|s| s.write(buf).map(|_| ()))
            .map(|_| buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.sinks
            .lock()
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::Other,
                    "Failed to aquire flush lock for output sinks.",
                )
            })?
            .iter_mut()
            .try_for_each(|s| s.flush())
    }
}
