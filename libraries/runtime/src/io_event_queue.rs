#![cfg(unix)]

use std::{
    cell::RefCell,
    collections::HashMap,
    io::{Read, Write},
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::prelude::{AsRawFd, RawFd};

use mio::{Events, Interest, Poll, Token};
use thiserror::Error;

#[cfg(unix)]
use mio::unix::pipe::{self, Receiver, Sender};

use crate::error::RuntimeError;

static CURRENT_IO_WRITE_ID: AtomicU64 = AtomicU64::new(1);
static CURRENT_IO_READ_ID: AtomicU64 = AtomicU64::new(2);

#[derive(PartialEq, Eq, Hash, Copy, Clone, Default, Debug)]
pub struct IoId {
    id: u64,
}

#[repr(u64)]
#[derive(Clone, Copy, Debug)]
pub enum IoOperationError {
    Unknown = 0,
    InvalidIoId,
    InvalidDiscriminator,

    WouldBlock = 100,
    NotFound,
    PermissionDenied,
    ConnectionRefused,
    ConnectionReset,
    ConnectionAborted,
    NotConnected,
    AddrInUse,
    AddrNotAvailable,
    BrokenPipe,
    AlreadyExists,
    InvalidInput,
    InvalidData,
    TimedOut,
    WriteZero,
    Interrupted,
    Unsupported,
    UnexpectedEof,
    OutOfMemory,
    Other,
}

impl From<std::io::Error> for IoOperationError {
    fn from(e: std::io::Error) -> Self {
        match e.kind() {
            std::io::ErrorKind::WouldBlock => Self::WouldBlock,
            std::io::ErrorKind::NotFound => Self::NotFound,
            std::io::ErrorKind::PermissionDenied => Self::PermissionDenied,
            std::io::ErrorKind::ConnectionRefused => Self::ConnectionRefused,
            std::io::ErrorKind::ConnectionReset => Self::ConnectionReset,
            std::io::ErrorKind::ConnectionAborted => Self::ConnectionAborted,
            std::io::ErrorKind::NotConnected => Self::NotConnected,
            std::io::ErrorKind::AddrInUse => Self::AddrInUse,
            std::io::ErrorKind::AddrNotAvailable => Self::AddrNotAvailable,
            std::io::ErrorKind::BrokenPipe => Self::BrokenPipe,
            std::io::ErrorKind::AlreadyExists => Self::AlreadyExists,
            std::io::ErrorKind::InvalidInput => Self::InvalidInput,
            std::io::ErrorKind::InvalidData => Self::InvalidData,
            std::io::ErrorKind::TimedOut => Self::TimedOut,
            std::io::ErrorKind::WriteZero => Self::WriteZero,
            std::io::ErrorKind::Interrupted => Self::Interrupted,
            std::io::ErrorKind::Unsupported => Self::Unsupported,
            std::io::ErrorKind::UnexpectedEof => Self::UnexpectedEof,
            std::io::ErrorKind::OutOfMemory => Self::OutOfMemory,
            std::io::ErrorKind::Other => Self::Other,
            _ => Self::Unknown,
        }
    }
}

impl From<ParserErrorKind> for IoOperationError {
    fn from(kind: ParserErrorKind) -> Self {
        match kind {
            ParserErrorKind::Io(io_error) => IoOperationError::from(io_error),
            ParserErrorKind::InvalidDiscriminator(_) => IoOperationError::InvalidDiscriminator,
        }
    }
}

impl IoId {
    pub fn new(id: u64) -> Self {
        Self { id }
    }

    pub fn generate_write() -> Self {
        Self {
            id: CURRENT_IO_WRITE_ID.fetch_add(2, Ordering::SeqCst),
        }
    }

    pub fn generate_read() -> Self {
        Self {
            id: CURRENT_IO_READ_ID.fetch_add(2, Ordering::SeqCst),
        }
    }

    pub fn raw(&self) -> u64 {
        self.id
    }
}

pub trait IoReader {
    fn poll_read(&mut self, buf: &mut [u8]) -> Result<std::task::Poll<usize>, std::io::Error>;
}

pub trait IoWriter {
    fn poll_write(&mut self, buf: &[u8]) -> Result<std::task::Poll<()>, std::io::Error>;
    fn close(&mut self) -> Result<(), String>;
}

impl From<WriteOperation<'_>> for IoCompletionEvent {
    fn from(req: WriteOperation) -> Self {
        Self {
            userdata: req.userdata,
            payload: match req.result {
                Ok(_) => IoCompletionEventPayload::WriteComplete,
                Err(ec) => IoCompletionEventPayload::WriteFailed(ec),
            },
            write_progress: CompletionEventWriteProgress::default(),
            bytes_written: 0,
        }
    }
}

enum IoOperation<'a> {
    Read(ReadOperation<'a>),
    Write(WriteOperation<'a>),
}

impl From<IoOperation<'_>> for IoCompletionEvent {
    fn from(iop: IoOperation) -> Self {
        match iop {
            IoOperation::Read(r) => r.into(),
            IoOperation::Write(w) => w.into(),
        }
    }
}

impl<'a> IoOperation<'a> {
    fn from_request(
        req: IoRequest,
        reader_factories: &HashMap<IoId, Rc<RefCell<dyn IoReader + 'a>>>,
        writer_factories: &HashMap<IoId, Rc<RefCell<dyn IoWriter + 'a>>>,
    ) -> Self {
        match req.payload {
            IoRequestPayload::Read(sz) => match reader_factories.get(&req.id) {
                Some(reader_factory) => Self::Read(ReadOperation::new(
                    req.userdata,
                    sz as usize,
                    reader_factory.clone(),
                )),
                None => Self::Read(ReadOperation::new_failed(
                    req.userdata,
                    IoOperationError::InvalidIoId,
                )),
            },
            IoRequestPayload::Write(data) => match writer_factories.get(&req.id) {
                Some(writer_factory) => Self::Write(WriteOperation::new(
                    req.userdata,
                    data,
                    writer_factory.clone(),
                )),
                None => Self::Write(WriteOperation::new_failed(
                    req.userdata,
                    IoOperationError::InvalidIoId,
                )),
            },
        }
    }
}

struct ReadOperation<'a> {
    userdata: u64,
    bytes_requested: usize,
    bytes_read: usize,
    eof: bool,
    reader: Rc<RefCell<dyn IoReader + 'a>>,
    result: Result<Vec<u8>, IoOperationError>,
}

struct NopIo {}

impl IoReader for NopIo {
    fn poll_read(&mut self, _buf: &mut [u8]) -> Result<std::task::Poll<usize>, std::io::Error> {
        Ok(std::task::Poll::Ready(0))
    }
}

impl IoWriter for NopIo {
    fn poll_write(&mut self, _buf: &[u8]) -> Result<std::task::Poll<()>, std::io::Error> {
        Ok(std::task::Poll::Ready(()))
    }

    fn close(&mut self) -> Result<(), String> {
        Ok(())
    }
}

impl<'a> ReadOperation<'a> {
    const BUFFER_SIZE: usize = 4096;

    pub fn new(userdata: u64, requested: usize, reader: Rc<RefCell<dyn IoReader + 'a>>) -> Self {
        Self {
            userdata,
            bytes_requested: requested,
            bytes_read: 0,
            eof: false,
            result: Ok(vec![0; requested]),
            reader,
        }
    }

    pub fn new_failed(userdata: u64, error_code: IoOperationError) -> Self {
        Self {
            userdata,
            bytes_requested: 0,
            bytes_read: 0,
            eof: true,
            reader: Rc::new(RefCell::new(NopIo {})),
            result: Err(error_code),
        }
    }

    pub fn update(&mut self) -> bool {
        if self.bytes_read == self.bytes_requested || self.eof {
            return true;
        }

        match self.result.as_mut() {
            Ok(buffer) => {
                let start = self.bytes_read;
                let end = std::cmp::min(self.bytes_read + Self::BUFFER_SIZE, self.bytes_requested);
                match self.reader.borrow_mut().poll_read(&mut buffer[start..end]) {
                    Ok(std::task::Poll::Pending) => false,
                    Ok(std::task::Poll::Ready(sz)) => {
                        self.bytes_read += sz;
                        if sz == 0 {
                            self.eof = true;
                        }
                        self.bytes_read == self.bytes_requested || self.eof
                    }
                    Err(e) => {
                        self.result = Err(e.into());
                        true
                    }
                }
            }
            Err(_) => true,
        }
    }
}

impl From<ReadOperation<'_>> for IoCompletionEvent {
    fn from(req: ReadOperation) -> Self {
        Self {
            bytes_written: 0,
            userdata: req.userdata,
            payload: match req.result {
                Ok(buf) => IoCompletionEventPayload::ReadComplete(buf),
                Err(ec) => IoCompletionEventPayload::ReadFailed(ec),
            },
            write_progress: CompletionEventWriteProgress::default(),
        }
    }
}

struct WriteOperation<'a> {
    userdata: u64,
    buffer: Vec<u8>,
    written: usize,
    result: Result<(), IoOperationError>,
    writer: Rc<RefCell<dyn IoWriter + 'a>>,
}

impl<'a> WriteOperation<'a> {
    const BUFFER_SIZE: usize = 4096;
    pub fn new(userdata: u64, buffer: Vec<u8>, writer: Rc<RefCell<dyn IoWriter + 'a>>) -> Self {
        Self {
            userdata,
            buffer,
            written: 0,
            result: Ok(()),
            writer,
        }
    }

    pub fn new_failed(userdata: u64, error_code: IoOperationError) -> Self {
        Self {
            userdata,
            buffer: Vec::with_capacity(0),
            written: 0,
            result: Err(error_code),
            writer: Rc::new(RefCell::new(NopIo {})),
        }
    }

    pub fn update(&mut self) -> bool {
        if self.written == self.buffer.len() {
            return true;
        }

        match self.result {
            Ok(_) => {
                let start = self.written;
                let end = std::cmp::min(start + Self::BUFFER_SIZE, self.buffer.len());
                match self
                    .writer
                    .borrow_mut()
                    .poll_write(&self.buffer[start..end])
                {
                    Ok(_) => {
                        self.written += end - start;
                    }
                    Err(e) => {
                        self.result = Err(e.into());
                    }
                }

                false
            }
            Err(_) => true,
        }
    }
}

pub struct IoEventQueue<'a> {
    rx: Receiver,

    tx: Sender,

    poll: Poll,

    client_tx: Sender,
    client_rx: Receiver,

    iops: Vec<Option<(bool, IoOperation<'a>)>>,

    readers: HashMap<IoId, Rc<RefCell<dyn IoReader + 'a>>>,
    writers: HashMap<IoId, Rc<RefCell<dyn IoWriter + 'a>>>,

    parser_state: Option<ParserState>,
    writer_state: Option<IoCompletionEvent>,
}

impl<'a> IoEventQueue<'a> {
    const PIPE_RECV: Token = Token(0);
    const PIPE_SEND: Token = Token(1);

    pub fn try_new(size: usize) -> Result<Self, RuntimeError> {
        pipe::new()
            .map_err(|e| RuntimeError::FailedToCreateEventQueue(e.to_string()))
            .and_then(|sub| {
                pipe::new()
                    .map_err(|e| RuntimeError::FailedToCreateEventQueue(e.to_string()))
                    .map(|comp| (sub, comp))
            })
            .and_then(|((sub_tx, sub_rx), (comp_tx, comp_rx))| {
                sub_tx
                    .set_nonblocking(false)
                    .and_then(|_| comp_rx.set_nonblocking(false))
                    .map_err(|e| RuntimeError::FailedToCreateEventQueue(e.to_string()))
                    .map(|_| ((sub_tx, sub_rx), (comp_tx, comp_rx)))
            })
            .and_then(|((sub_tx, mut sub_rx), comp)| {
                Poll::new()
                    .map_err(|e| RuntimeError::FailedToCreateEventQueue(e.to_string()))
                    .and_then(|poll| {
                        poll.registry()
                            .register(&mut sub_rx, Self::PIPE_RECV, Interest::READABLE)
                            .map_err(|e| RuntimeError::FailedToCreateEventQueue(e.to_string()))
                            .map(|_| poll)
                    })
                    .map(|poll| (poll, (sub_tx, sub_rx), comp))
            })
            .map(|(poll, (sub_tx, sub_rx), (comp_tx, comp_rx))| Self {
                rx: sub_rx,
                tx: comp_tx,
                poll,
                client_tx: sub_tx,
                client_rx: comp_rx,
                iops: std::iter::repeat_with(|| None)
                    .take(size)
                    .collect::<Vec<_>>(),
                readers: HashMap::new(),
                writers: HashMap::new(),
                parser_state: None,
                writer_state: None,
            })
    }

    pub fn submission_fd(&self) -> RawFd {
        self.client_tx.as_raw_fd()
    }

    pub fn completion_fd(&self) -> RawFd {
        self.client_rx.as_raw_fd()
    }

    pub fn register_reader(&mut self, id: IoId, reader: impl IoReader + 'a) {
        self.readers.insert(id, Rc::new(RefCell::new(reader)));
    }

    pub fn register_writer(&mut self, id: IoId, writer: impl IoWriter + 'a) {
        self.writers.insert(id, Rc::new(RefCell::new(writer)));
    }

    pub fn update(&mut self) -> Result<bool, RuntimeError> {
        let mut has_completed = false;
        let mut has_pending = false;
        self.iops.iter_mut().for_each(|req| {
            if let Some(iop) = req {
                match iop.1 {
                    IoOperation::Read(ref mut r) => {
                        (*iop).0 = r.update();
                    }
                    IoOperation::Write(ref mut w) => {
                        (*iop).0 = w.update();
                    }
                }
                has_completed |= (*iop).0;
                has_pending |= !(*iop).0;
            }
        });

        let mut write_registered = false;
        // Register interests
        if has_completed || self.writer_state.is_some() {
            write_registered = true;
            self.poll
                .registry()
                .register(&mut self.tx, Self::PIPE_SEND, Interest::WRITABLE)
                .map_err(|e| RuntimeError::FailedToRegisterInterests(e.to_string()))?;
        }

        // poll pipe
        let timeout = (has_pending || has_completed).then(|| Duration::from_millis(60));
        let mut events = Events::with_capacity(8);
        self.poll
            .poll(&mut events, timeout)
            .map_err(|e| RuntimeError::FailedToPollQueue(e.to_string()))?;

        for v in events.iter() {
            match v.token() {
                IoEventQueue::PIPE_RECV if v.is_read_closed() => return Ok(false),
                IoEventQueue::PIPE_RECV => {
                    while let Some(iops_slot) = self.iops.iter_mut().find(|f| f.is_none()) {
                        match IoRequest::try_parse(self.parser_state.take(), &mut self.rx) {
                            Ok(state) => match state.progress {
                                ParserProgress::Complete(io_request) => {
                                    *iops_slot = Some((
                                        false,
                                        IoOperation::from_request(
                                            io_request,
                                            &self.readers,
                                            &self.writers,
                                        ),
                                    ));
                                }
                                _ => {
                                    self.parser_state = Some(state);
                                    break;
                                }
                            },
                            Err(parser_error) => {
                                let operation = match parser_error.state.progress {
                                    ParserProgress::Id => None,
                                    ParserProgress::Userdata => None,
                                    ParserProgress::Discriminator => {
                                        Some(IoOperation::Read(ReadOperation::new_failed(
                                            parser_error.state.userdata,
                                            parser_error.kind.into(),
                                        )))
                                    }
                                    ParserProgress::Size
                                    | ParserProgress::Payload(_)
                                    | ParserProgress::Complete(_) => {
                                        match parser_error.state.discriminator {
                                            1 => Some(IoOperation::Write(
                                                WriteOperation::new_failed(
                                                    parser_error.state.userdata,
                                                    parser_error.kind.into(),
                                                ),
                                            )),
                                            _ => {
                                                Some(IoOperation::Read(ReadOperation::new_failed(
                                                    parser_error.state.userdata,
                                                    parser_error.kind.into(),
                                                )))
                                            }
                                        }
                                    }
                                };

                                *iops_slot = operation.map(|op| (false, op));
                            }
                        }
                    }
                }
                IoEventQueue::PIPE_SEND if v.is_write_closed() => return Ok(false),
                IoEventQueue::PIPE_SEND => {
                    if let Some(mut prev_completion) = self.writer_state.take() {
                        prev_completion.write_to(&self.tx);
                        if !prev_completion.write_completed() {
                            self.writer_state = Some(prev_completion);
                            continue;
                        }
                    }

                    for v in self.iops.iter_mut() {
                        if let Some(iop) = v.take() {
                            if iop.0 {
                                let mut completion = IoCompletionEvent::from(iop.1);
                                completion.write_to(&self.tx);
                                if !completion.write_completed() {
                                    self.writer_state = Some(completion);
                                    break;
                                }
                            } else {
                                *v = Some(iop);
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        if write_registered {
            self.poll
                .registry()
                .deregister(&mut self.tx)
                .map_err(|e| RuntimeError::FailedToDeregisterQueue(e.to_string()))?;
        }

        Ok(true)
    }
}

#[derive(Debug)]
pub enum IoCompletionEventPayload {
    ReadComplete(Vec<u8>),
    ReadFailed(IoOperationError),
    WriteComplete,
    WriteFailed(IoOperationError),
}

#[derive(Debug)]
pub struct ParserState {
    buf: [u8; 8],
    ioid: IoId,
    buflen: usize,
    userdata: u64,
    size: usize,
    discriminator: u8,
    progress: ParserProgress,
}

impl Default for ParserState {
    fn default() -> Self {
        Self {
            ioid: IoId::default(),
            buf: [0; 8],
            buflen: 0,
            userdata: 0,
            discriminator: 0,
            progress: ParserProgress::Id,
            size: 0,
        }
    }
}

// TODO: Is it sane to ignore all errors and turn them into nones?
impl ParserState {
    fn read_u64<R: Read>(&mut self, reader: &mut R) -> Result<Option<u64>, ParserErrorKind> {
        match reader.read(&mut self.buf[self.buflen..]) {
            Ok(sz) => {
                self.buflen += sz;
                if self.buflen < 8 {
                    return Ok(None);
                }

                self.buflen = 0;
                Ok(Some(u64::from_le_bytes(self.buf)))
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn read_u8<R: Read>(&mut self, reader: &mut R) -> Result<Option<u8>, ParserErrorKind> {
        let mut b = [0u8; 1];
        match reader.read(&mut b) {
            Ok(sz) => {
                if sz < 1 {
                    Ok(None)
                } else {
                    Ok(Some(b[0]))
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn read_ioid<R: Read>(&mut self, reader: &mut R) -> Result<bool, ParserErrorKind> {
        self.read_u64(reader).map(|v| match v {
            Some(val) => {
                self.ioid = IoId::new(val);
                self.progress = ParserProgress::Userdata;
                true
            }
            None => false,
        })
    }

    fn read_userdata<R: Read>(&mut self, reader: &mut R) -> Result<bool, ParserErrorKind> {
        self.read_u64(reader).map(|v| match v {
            Some(val) => {
                self.userdata = val;
                self.progress = ParserProgress::Discriminator;
                true
            }
            None => false,
        })
    }

    fn read_discriminator<R: Read>(&mut self, reader: &mut R) -> Result<bool, ParserErrorKind> {
        self.read_u8(reader).and_then(|v| match v {
            Some(val) => match val {
                0..=1 => {
                    self.discriminator = val;
                    self.progress = ParserProgress::Size;
                    Ok(true)
                }
                x => Err(ParserErrorKind::InvalidDiscriminator(x)),
            },
            None => Ok(false),
        })
    }

    fn read_size<R: Read>(&mut self, reader: &mut R) -> Result<bool, ParserErrorKind> {
        self.read_u64(reader).and_then(|v| match v {
            Some(val) => {
                self.size = val as usize;
                match self.discriminator {
                    0 => {
                        self.progress = ParserProgress::Complete(IoRequest {
                            id: self.ioid,
                            userdata: self.userdata,
                            payload: IoRequestPayload::Read(self.size as u64),
                        });
                        Ok(true)
                    }

                    1 => {
                        self.progress = ParserProgress::Payload(vec![0; self.size]);
                        Ok(true)
                    }

                    x => Err(ParserErrorKind::InvalidDiscriminator(x)),
                }
            }
            None => Ok(false),
        })
    }

    fn read_payload<R: Read>(mut self, reader: &mut R) -> (Self, bool) {
        match self.progress {
            ParserProgress::Payload(mut payload) => {
                let pos = payload.capacity() - self.size;
                let sz = reader.read(&mut payload[pos..]).unwrap_or(0);
                self.size -= sz;
                if self.size == 0 {
                    self.progress = ParserProgress::Complete(IoRequest {
                        id: self.ioid,
                        userdata: self.userdata,
                        payload: IoRequestPayload::Write(payload),
                    });
                    (self, true)
                } else {
                    self.progress = ParserProgress::Payload(payload);
                    (self, false)
                }
            }
            _ => (self, false),
        }
    }

    fn is_complete(&self) -> bool {
        matches!(self.progress, ParserProgress::Complete(_))
    }
}

#[derive(Debug)]
pub enum ParserProgress {
    Id,
    Userdata,
    Discriminator,
    Size,
    Payload(Vec<u8>),
    Complete(IoRequest),
}

#[derive(Debug)]
pub enum IoRequestPayload {
    Read(u64),
    Write(Vec<u8>),
}

#[derive(Debug)]
pub struct IoRequest {
    id: IoId,
    userdata: u64,
    payload: IoRequestPayload,
}

#[derive(Error, Debug)]
pub enum ParserErrorKind {
    #[error("Io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid discriminator: {0}")]
    InvalidDiscriminator(u8),
}

#[derive(Error, Debug)]
struct ParserError {
    state: ParserState,
    kind: ParserErrorKind,
}

impl ParserError {
    pub fn new(state: ParserState, kind: ParserErrorKind) -> Self {
        Self { state, kind }
    }
}

impl std::fmt::Display for ParserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.kind.fmt(f)
    }
}

impl IoRequest {
    fn try_parse<R: Read>(
        state: Option<ParserState>,
        mut reader: R,
    ) -> Result<ParserState, ParserError> {
        let mut state = state.unwrap_or_default();

        while !state.is_complete() {
            match state.progress {
                ParserProgress::Id => match state.read_ioid(&mut reader) {
                    Ok(done) => {
                        if !done {
                            return Ok(state);
                        }
                    }
                    Err(e) => return Err(ParserError::new(state, e)),
                },
                ParserProgress::Userdata => match state.read_userdata(&mut reader) {
                    Ok(done) => {
                        if !done {
                            return Ok(state);
                        }
                    }
                    Err(e) => return Err(ParserError::new(state, e)),
                },
                ParserProgress::Discriminator => match state.read_discriminator(&mut reader) {
                    Ok(done) => {
                        if !done {
                            return Ok(state);
                        }
                    }
                    Err(e) => return Err(ParserError::new(state, e)),
                },
                ParserProgress::Size => match state.read_size(&mut reader) {
                    Ok(done) => {
                        if !done {
                            return Ok(state);
                        }
                    }
                    Err(e) => return Err(ParserError::new(state, e)),
                },
                ParserProgress::Payload(_) => {
                    let result;
                    (state, result) = state.read_payload(&mut reader);
                    if !result {
                        return Ok(state);
                    }
                }
                ParserProgress::Complete(_) => return Ok(state),
            }
        }

        Ok(state)
    }
}

pub struct IoCompletionEvent {
    userdata: u64,
    payload: IoCompletionEventPayload,
    bytes_written: usize,
    write_progress: CompletionEventWriteProgress,
}

enum CompletionEventWriteProgress {
    Userdata,
    Discriminator,
    Size(usize),
    ErrorCode(IoOperationError),
    Payload,
    Completed,
}

impl Default for CompletionEventWriteProgress {
    fn default() -> Self {
        Self::Userdata
    }
}

impl IoCompletionEvent {
    fn write_to<W: Write>(&mut self, mut writer: W) {
        while !self.write_completed() {
            match self.write_progress {
                CompletionEventWriteProgress::Userdata => {
                    let bytes = self.userdata.to_le_bytes();
                    let written = writer.write(&bytes[self.bytes_written..]).unwrap_or(0);
                    self.bytes_written += written;
                    if self.bytes_written < 8 {
                        return;
                    }

                    self.bytes_written = 0;
                    self.write_progress = CompletionEventWriteProgress::Discriminator;
                }
                CompletionEventWriteProgress::Discriminator => {
                    let byte = [match self.payload {
                        IoCompletionEventPayload::ReadComplete(_) => 0,
                        IoCompletionEventPayload::ReadFailed(_) => 1,
                        IoCompletionEventPayload::WriteComplete => 2,
                        IoCompletionEventPayload::WriteFailed(_) => 3,
                    }; 1];
                    let written = writer.write(&byte).unwrap_or(0);

                    if written < 1 {
                        return;
                    }

                    self.write_progress = match &self.payload {
                        IoCompletionEventPayload::ReadComplete(buf) => {
                            CompletionEventWriteProgress::Size(buf.len())
                        }
                        IoCompletionEventPayload::ReadFailed(ec) => {
                            CompletionEventWriteProgress::ErrorCode(*ec)
                        }
                        IoCompletionEventPayload::WriteComplete => {
                            CompletionEventWriteProgress::Completed
                        }
                        IoCompletionEventPayload::WriteFailed(ec) => {
                            CompletionEventWriteProgress::ErrorCode(*ec)
                        }
                    };
                }
                CompletionEventWriteProgress::Size(sz) => {
                    let bytes = (sz as u64).to_le_bytes();
                    let written = writer.write(&bytes[self.bytes_written..]).unwrap_or(0);
                    self.bytes_written += written;
                    if self.bytes_written < 8 {
                        return;
                    }

                    self.bytes_written = 0;
                    self.write_progress = CompletionEventWriteProgress::Payload
                }
                CompletionEventWriteProgress::Payload => match &self.payload {
                    IoCompletionEventPayload::ReadComplete(buf) => {
                        if self.bytes_written < buf.len() {
                            let written = writer.write(&buf[self.bytes_written..]).unwrap_or(0);
                            self.bytes_written += written;
                        }

                        if self.bytes_written == buf.len() {
                            self.bytes_written = 0;
                            self.write_progress = CompletionEventWriteProgress::Completed;
                        } else {
                            return;
                        }
                    }
                    // It is mean to panic in a lib. However this can
                    // only occur if we implemented something wrong
                    // internally in the library.
                    _ => panic!(
                        "Unexpected payload {:?}. Expected it to be ReadComplete",
                        self.payload
                    ),
                },
                CompletionEventWriteProgress::ErrorCode(ec) => {
                    let bytes = (ec as u64).to_le_bytes();
                    let written = writer.write(&bytes[self.bytes_written..]).unwrap_or(0);
                    self.bytes_written += written;
                    if self.bytes_written < 8 {
                        return;
                    }

                    self.bytes_written = 0;
                    self.write_progress = CompletionEventWriteProgress::Completed;
                }
                CompletionEventWriteProgress::Completed => {}
            }
        }
    }

    fn write_completed(&self) -> bool {
        matches!(self.write_progress, CompletionEventWriteProgress::Completed)
    }
}

#[cfg(test)]
mod test {
    use std::{
        fs::File,
        io::Cursor,
        // TODO: How would this work on windodo?
        os::unix::io::FromRawFd,
    };

    use function::stream::{rwstream::RWChannelStream, Channel, Stream};

    use super::*;

    #[test]
    fn test_parse_io_read_request() {
        let mut cursor: Cursor<Vec<u8>> = Cursor::new(Vec::new());
        cursor.write_all(&6u64.to_le_bytes()).unwrap(); // id
        cursor.write_all(&6u64.to_le_bytes()).unwrap(); // userdata
        cursor.set_position(0);
        let state = IoRequest::try_parse(None, &mut cursor).unwrap();
        let old_pos = cursor.position();
        assert!(matches!(state.progress, ParserProgress::Discriminator));
        cursor.write_all(&0u8.to_le_bytes()).unwrap(); // discriminator (0 read, 1 write)
        cursor.write_all(&5u64.to_le_bytes()).unwrap(); // size
        cursor.set_position(old_pos);
        let state = IoRequest::try_parse(Some(state), &mut cursor).unwrap();
        assert!(matches!(state.progress, ParserProgress::Complete(_)));
    }

    #[test]
    fn test_parse_io_write_request() {
        let mut cursor: Cursor<Vec<u8>> = Cursor::new(Vec::new());
        cursor.write_all(&6u64.to_le_bytes()).unwrap(); // id
        cursor.write_all(&6u64.to_le_bytes()).unwrap(); // userdata
        cursor.set_position(0);
        let state = IoRequest::try_parse(None, &mut cursor).unwrap();
        let old_pos = cursor.position();
        assert!(matches!(state.progress, ParserProgress::Discriminator));
        cursor.write_all(&1u8.to_le_bytes()).unwrap(); // discriminator (0 read, 1 write)
        cursor.write_all(&8u64.to_le_bytes()).unwrap(); // size
        cursor.write_all(&10u64.to_le_bytes()).unwrap(); // payload
        cursor.set_position(old_pos);
        let state = IoRequest::try_parse(Some(state), &mut cursor).unwrap();
        assert!(matches!(state.progress, ParserProgress::Complete(_)));

        match state.progress {
            ParserProgress::Complete(request) => {
                assert!(match request.payload {
                    IoRequestPayload::Read(_) => false,
                    IoRequestPayload::Write(buf) => {
                        u64::from_le_bytes(buf.try_into().unwrap()) == 10
                    }
                });
            }
            _ => panic!("Expected progress to be complete."),
        }
    }

    #[test]
    fn test_parse_io_half_request() {
        let buf = 6u64.to_le_bytes();
        let mut cursor: Cursor<Vec<u8>> = Cursor::new(Vec::new());
        cursor.write_all(&buf[0..4]).unwrap(); // write half id
        cursor.set_position(0);
        let state = IoRequest::try_parse(None, &mut cursor).unwrap();
        assert!(matches!(state.progress, ParserProgress::Id));
        cursor.write_all(&buf[4..8]).unwrap(); // write second half id
        cursor.set_position(4);
        let state = IoRequest::try_parse(Some(state), &mut cursor).unwrap();
        assert!(matches!(state.progress, ParserProgress::Userdata));
        assert_eq!(state.ioid.id, 6);
    }

    #[test]
    fn test_io_completion_event_read_complete() {
        let mut event = IoCompletionEvent {
            userdata: 5,
            payload: IoCompletionEventPayload::ReadComplete(vec![6u8; 2]),
            bytes_written: 0,
            write_progress: CompletionEventWriteProgress::Userdata,
        };

        let mut buf = [0u8; 8];
        event.write_to(Cursor::new(buf.as_mut_slice()));
        assert!(matches!(
            event.write_progress,
            CompletionEventWriteProgress::Discriminator
        ));
        assert_eq!(5, u64::from_le_bytes(buf));

        let mut buf = [0u8; 5];
        event.write_to(Cursor::new(buf.as_mut_slice()));
        assert!(matches!(
            event.write_progress,
            CompletionEventWriteProgress::Size(_)
        ));
        assert_eq!(4, event.bytes_written);
        let mut mega_buf = [0u8; 4];
        event.write_to(Cursor::new(mega_buf.as_mut_slice()));
        assert_eq!(0, event.bytes_written);
        assert!(matches!(
            event.write_progress,
            CompletionEventWriteProgress::Payload
        ));
        let combined_buf = [&buf[1..], &mega_buf].concat();
        assert_eq!(2, u64::from_le_bytes(combined_buf.try_into().unwrap()));

        let mut buf = [0u8; 1];
        event.write_to(Cursor::new(buf.as_mut_slice()));
        assert_eq!(1, event.bytes_written);
        assert!(matches!(
            event.write_progress,
            CompletionEventWriteProgress::Payload
        ));
        assert_eq!(6, buf[0]);

        let mut buf = [254u8; 1];
        event.write_to(Cursor::new(buf.as_mut_slice()));
        assert_eq!(0, event.bytes_written);
        assert!(matches!(
            event.write_progress,
            CompletionEventWriteProgress::Completed
        ));
        assert_eq!(6, buf[0]);
    }

    #[test]
    fn test_io_completion_event_write_complete() {
        let mut event = IoCompletionEvent {
            userdata: 20,
            payload: IoCompletionEventPayload::WriteComplete,
            bytes_written: 0,
            write_progress: CompletionEventWriteProgress::Userdata,
        };
        let mut buf = [0u8; 9];
        event.write_to(Cursor::new(buf.as_mut_slice()));
        assert!(matches!(
            event.write_progress,
            CompletionEventWriteProgress::Completed
        ));
        assert_eq!(buf[8], 2);
    }

    #[test]
    fn test_io_completion_event_read_or_write_failed() {
        let mut event = IoCompletionEvent {
            userdata: 20,
            payload: IoCompletionEventPayload::ReadFailed(IoOperationError::InvalidIoId),
            bytes_written: 0,
            write_progress: CompletionEventWriteProgress::Userdata,
        };
        let mut buf = [0u8; 17];
        event.write_to(Cursor::new(buf.as_mut_slice()));
        assert!(matches!(
            event.write_progress,
            CompletionEventWriteProgress::Completed
        ));
        assert_eq!(u64::from_le_bytes(buf[9..].try_into().unwrap()), 1);
        assert_eq!(buf[8], 1);

        let mut event = IoCompletionEvent {
            userdata: 20,
            payload: IoCompletionEventPayload::WriteFailed(IoOperationError::Unknown),
            bytes_written: 0,
            write_progress: CompletionEventWriteProgress::Userdata,
        };
        let mut buf = [0u8; 17];
        event.write_to(Cursor::new(buf.as_mut_slice()));
        assert!(matches!(
            event.write_progress,
            CompletionEventWriteProgress::Completed
        ));
        assert_eq!(u64::from_le_bytes(buf[9..].try_into().unwrap()), 0);
        assert_eq!(buf[8], 3);
    }

    pub enum TestData {
        ReadComplete {
            userdata: u64,
            discriminator: u8,
            size: u64,
            payload: Vec<u8>,
        },
        ReadFailed {
            userdata: u64,
            discriminator: u8,
            error: u64,
        },
        WriteComplete {
            userdata: u64,
            discriminator: u8,
        },
        WriteFailed {
            userdata: u64,
            discriminator: u8,
            error: u64,
        },
    }

    fn parse_completion(file: &mut File) -> TestData {
        let mut buf_u64 = [0u8; 8];
        let mut buf_u8 = [0u8; 1];
        file.read_exact(&mut buf_u64).unwrap();
        let userdata = u64::from_le_bytes(buf_u64);
        file.read_exact(&mut buf_u8).unwrap();
        let discriminator = buf_u8[0];

        match discriminator {
            0 => {
                file.read_exact(&mut buf_u64).unwrap();
                let size = u64::from_le_bytes(buf_u64);
                let mut buf: Vec<u8> = vec![0; size as usize];
                file.read_exact(&mut buf).unwrap();
                TestData::ReadComplete {
                    userdata,
                    discriminator,
                    size,
                    payload: buf,
                }
            }
            1 => {
                file.read_exact(&mut buf_u64).unwrap();
                let error = u64::from_le_bytes(buf_u64);
                TestData::ReadFailed {
                    userdata,
                    discriminator,
                    error,
                }
            }
            2 => TestData::WriteComplete {
                userdata,
                discriminator,
            },
            3 => {
                file.read_exact(&mut buf_u64).unwrap();
                let error = u64::from_le_bytes(buf_u64);
                TestData::WriteFailed {
                    userdata,
                    discriminator,
                    error,
                }
            }
            _ => panic!("This should never happen"),
        }
    }

    #[test]
    fn io_event_queue() {
        let mut queue = IoEventQueue::try_new(10).unwrap();

        let output_stream = RWChannelStream::new(vec![
            Channel::new("out_vigg", "bird"),
            Channel::new("out_storspov", "bird"),
        ]);
        let vigg_writer_id = IoId::generate_write();
        let storspov_writer_id = IoId::generate_write();
        let out_vigg = output_stream.write_channel("out_vigg").unwrap();
        let out_storspov = output_stream.write_channel("out_storspov").unwrap();
        queue.register_writer(vigg_writer_id, out_vigg);
        queue.register_writer(storspov_writer_id, out_storspov);

        let input_stream = RWChannelStream::new(vec![
            Channel::new("in_sarv", "fish"),
            Channel::new("in_nors", "fish"),
        ]);
        // Fill channel with 10 bytes.
        let mut sarv_writer = input_stream.write_channel("in_sarv").unwrap();
        let to_read = 10;
        let buf = vec![1u8; to_read];
        let _ = sarv_writer.poll_write(&buf).unwrap();

        let sarv_reader_id = IoId::generate_read();
        let nors_reader_id = IoId::generate_read();
        let in_sarv = input_stream.read_channel("in_sarv").unwrap();
        let in_nors = input_stream.read_channel("in_nors").unwrap();
        queue.register_reader(sarv_reader_id, in_sarv);
        queue.register_reader(nors_reader_id, in_nors);

        // Read request
        let mut payload = vec![];
        payload.extend_from_slice(&sarv_reader_id.id.to_le_bytes()); // ID
        payload.extend_from_slice(&5u64.to_le_bytes()); // userdata
        payload.extend_from_slice(&0u8.to_le_bytes()); // discriminator(0 is read, 1 is write)
        payload.extend_from_slice(&to_read.to_le_bytes()); // size

        let mut f_write = unsafe { File::from_raw_fd(queue.submission_fd()) };
        f_write.write_all(&payload).unwrap();

        queue.update().unwrap(); // First, parse IO operation
        queue.update().unwrap(); // Second, handle IO operation

        let mut f_read = unsafe { File::from_raw_fd(queue.completion_fd()) };
        let completion = parse_completion(&mut f_read);
        matches!(completion, TestData::ReadComplete { .. });

        // Dropping the file descriptors is a way to tell that the
        // runtime is done. Update should retrn false in that case.
        drop(f_write);
        drop(f_read);
        assert!(matches!(queue.update(), Ok(false)));
    }
}
