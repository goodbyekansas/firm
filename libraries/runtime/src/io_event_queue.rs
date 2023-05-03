use std::{
    collections::HashMap,
    io::{Read, Write},
    os::unix::prelude::{AsRawFd, RawFd},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use mio::{
    unix::pipe::{self, Receiver, Sender},
    Events, Interest, Poll, Token,
};

static CURRENT_IO_WRITE_ID: AtomicU64 = AtomicU64::new(1);
static CURRENT_IO_READ_ID: AtomicU64 = AtomicU64::new(2);

#[derive(PartialEq, Eq, Hash, Copy, Clone, Default)]
pub struct IoId {
    id: u64,
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

pub trait IoReaderFactory {
    fn create(&self) -> Box<dyn IoReader>;
}

pub trait IoReader: Read {
    fn readable(&self) -> bool;
    fn eof(&self) -> bool;
}

pub trait IoWriterFactory {
    fn create(&self) -> Box<dyn IoWriter>;
}

pub trait IoWriter: Write {
    fn writeable(&self) -> bool;
}

impl IoReaderFactory for Box<dyn IoReaderFactory> {
    fn create(&self) -> Box<dyn IoReader> {
        (*self).as_ref().create()
    }
}

impl IoReaderFactory for &Box<dyn IoReaderFactory> {
    fn create(&self) -> Box<dyn IoReader> {
        IoReaderFactory::create(*self)
    }
}

impl IoWriterFactory for Box<dyn IoWriterFactory> {
    fn create(&self) -> Box<dyn IoWriter> {
        (*self).as_ref().create()
    }
}

impl IoWriterFactory for &Box<dyn IoWriterFactory> {
    fn create(&self) -> Box<dyn IoWriter> {
        IoWriterFactory::create(*self)
    }
}

impl ReadOperation {
    const BUFFER_SIZE: usize = 4096;

    pub fn new<F: IoReaderFactory>(userdata: u64, requested: usize, factory: F) -> Self {
        Self {
            userdata,
            bytes_requested: requested,
            bytes_read: 0,
            eof: false,
            result: Ok(vec![0; requested]),
            reader: factory.create(),
        }
    }

    pub fn update(&mut self) -> bool {
        if self.bytes_read == self.bytes_requested || self.eof {
            return true;
        }

        if !self.reader.readable() {
            return false;
        }

        match self.result.as_mut() {
            Ok(buffer) => {
                let start = self.bytes_read;
                let end = std::cmp::min(self.bytes_read + Self::BUFFER_SIZE, self.bytes_requested);
                match self.reader.read(&mut buffer[start..end]) {
                    Ok(sz) => {
                        self.bytes_read += sz;
                        if sz < (end - start) {
                            self.eof = true;
                        }
                    }
                    Err(_) => {
                        // go from ok -> err
                        self.result = Err(42);
                    }
                };

                false
            }
            Err(_) => true,
        }
    }
}

impl From<ReadOperation> for IoCompletionEvent {
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

impl From<WriteOperation> for IoCompletionEvent {
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

enum IoOperation {
    Read(ReadOperation),
    Write(WriteOperation),
}

impl From<IoOperation> for IoCompletionEvent {
    fn from(iop: IoOperation) -> Self {
        match iop {
            IoOperation::Read(r) => r.into(),
            IoOperation::Write(w) => w.into(),
        }
    }
}

impl IoOperation {
    fn from_request(
        req: IoRequest,
        reader_factories: &HashMap<IoId, Box<dyn IoReaderFactory>>,
        writer_factories: &HashMap<IoId, Box<dyn IoWriterFactory>>,
    ) -> Self {
        match req.payload {
            IoRequestPayload::Read(sz) => Self::Read(ReadOperation::new(
                req.userdata,
                sz as usize,
                reader_factories.get(&req.id).unwrap(),
            )),
            IoRequestPayload::Write(data) => Self::Write(WriteOperation::new(
                req.userdata,
                data,
                writer_factories.get(&req.id).unwrap(),
            )),
        }
    }
}

struct ReadOperation {
    userdata: u64,
    bytes_requested: usize,
    bytes_read: usize,
    eof: bool,
    reader: Box<dyn IoReader>,
    result: Result<Vec<u8>, u64>,
}

struct WriteOperation {
    userdata: u64,
    buffer: Vec<u8>,
    written: usize,
    result: Result<(), u64>,
    writer: Box<dyn IoWriter>,
}

impl WriteOperation {
    const BUFFER_SIZE: usize = 4096;
    pub fn new<F: IoWriterFactory>(userdata: u64, buffer: Vec<u8>, factory: F) -> Self {
        Self {
            userdata,
            buffer,
            written: 0,
            result: Ok(()),
            writer: factory.create(),
        }
    }

    pub fn update(&mut self) -> bool {
        if self.written == self.buffer.len() {
            return true;
        }

        if !self.writer.writeable() {
            return false;
        }

        match self.result {
            Ok(_) => {
                let start = self.written;
                let end = std::cmp::min(start + Self::BUFFER_SIZE, self.buffer.len());
                match self.writer.write(&self.buffer[start..end]) {
                    Ok(sz) => {
                        self.written += sz;
                    }
                    Err(_) => {
                        self.result = Err(42);
                    }
                }

                false
            }
            Err(_) => true,
        }
    }
}

pub struct IoEventQueue {
    rx: Receiver,
    tx: Sender,

    poll: Poll,

    client_tx: Sender,
    client_rx: Receiver,

    iops: Vec<Option<(bool, IoOperation)>>,

    readers: HashMap<IoId, Box<dyn IoReaderFactory>>,
    writers: HashMap<IoId, Box<dyn IoWriterFactory>>,

    parser_state: Option<ParserState>,
    writer_state: Option<IoCompletionEvent>,
}

impl IoEventQueue {
    const PIPE_RECV: Token = Token(0);
    const PIPE_SEND: Token = Token(1);

    pub fn new(size: usize) -> Self {
        let (sub_tx, mut sub_rx) = pipe::new().unwrap();
        let (comp_tx, comp_rx) = pipe::new().unwrap();

        let poll = Poll::new().unwrap();
        poll.registry()
            .register(&mut sub_rx, Self::PIPE_RECV, Interest::READABLE)
            .unwrap();
        Self {
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
        }
    }

    pub fn submission_fd(&self) -> RawFd {
        self.client_tx.as_raw_fd()
    }

    pub fn completion_fd(&self) -> RawFd {
        self.client_rx.as_raw_fd()
    }

    pub fn register_reader(&mut self, id: IoId, reader: Box<dyn IoReaderFactory>) {
        self.readers.insert(id, reader);
    }

    pub fn register_writer(&mut self, id: IoId, writer: Box<dyn IoWriterFactory>) {
        self.writers.insert(id, writer);
    }

    pub fn update(&mut self) -> bool {
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
                .unwrap();
        }

        // poll pipe
        let timeout = (has_pending || has_completed).then(|| Duration::from_millis(60));
        let mut events = Events::with_capacity(8);
        self.poll.poll(&mut events, timeout).unwrap();

        for v in events.iter() {
            match v.token() {
                IoEventQueue::PIPE_RECV if v.is_read_closed() => return false,
                IoEventQueue::PIPE_RECV => {
                    while let Some(iops_slot) = self.iops.iter_mut().find(|f| f.is_none()) {
                        let state = IoRequest::parse(self.parser_state.take(), &mut self.rx);
                        match state.progress {
                            ParserProgress::Complete => {
                                let req = state.into_io_request();
                                *iops_slot = Some((
                                    false,
                                    IoOperation::from_request(req, &self.readers, &self.writers),
                                ))
                            }
                            _ => {
                                self.parser_state = Some(state);
                                break;
                            }
                        }
                    }
                }
                IoEventQueue::PIPE_SEND if v.is_write_closed() => return false,
                IoEventQueue::PIPE_SEND => {
                    if let Some(mut prev_completion) = self.writer_state.take() {
                        prev_completion.write_to(&self.tx).unwrap();
                        if !prev_completion.write_completed() {
                            self.writer_state = Some(prev_completion);
                            continue;
                        }
                    }
                    for v in self.iops.iter_mut() {
                        if let Some(iop) = v.take() {
                            if iop.0 {
                                let mut completion = IoCompletionEvent::from(iop.1);
                                completion.write_to(&self.tx).unwrap();
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
            self.poll.registry().deregister(&mut self.tx).unwrap();
        }

        true
    }
}

pub enum IoCompletionEventPayload {
    ReadComplete(Vec<u8>),
    ReadFailed(u64),
    WriteComplete,
    WriteFailed(u64),
}

pub struct ParserState {
    buf: [u8; 8],
    ioid: IoId,
    buflen: usize,
    userdata: u64,
    size: usize,
    discriminator: u8,
    payload: Vec<u8>,
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
            payload: Vec::with_capacity(0),
            size: 0,
        }
    }
}

impl ParserState {
    fn read_u64<R: Read>(&mut self, reader: &mut R) -> Option<u64> {
        let sz = reader.read(&mut self.buf[self.buflen..]).unwrap();
        self.buflen += sz;
        if self.buflen < 8 {
            return None;
        }

        self.buflen = 0;
        Some(u64::from_le_bytes(self.buf))
    }

    fn read_u8<R: Read>(&mut self, reader: &mut R) -> Option<u8> {
        let mut b = [0u8; 1];
        let sz = reader.read(&mut b).unwrap();
        if sz < 1 {
            return None;
        }

        Some(b[0])
    }

    fn read_ioid<R: Read>(&mut self, reader: &mut R) -> bool {
        match self.read_u64(reader) {
            Some(val) => {
                self.ioid = IoId::new(val);
                self.progress = ParserProgress::Userdata;
                true
            }
            None => false,
        }
    }

    fn read_userdata<R: Read>(&mut self, reader: &mut R) -> bool {
        match self.read_u64(reader) {
            Some(val) => {
                self.userdata = val;
                self.progress = ParserProgress::Discriminator;
                true
            }
            None => false,
        }
    }

    fn read_discriminator<R: Read>(&mut self, reader: &mut R) -> bool {
        match self.read_u8(reader) {
            Some(val) => {
                self.discriminator = val;
                self.progress = ParserProgress::Size;

                true
            }
            None => false,
        }
    }

    fn read_size<R: Read>(&mut self, reader: &mut R) -> bool {
        match self.read_u64(reader) {
            Some(val) => {
                self.size = val as usize;
                match self.discriminator {
                    0 => {
                        self.progress = ParserProgress::Complete;
                    }

                    1 => {
                        self.payload = vec![0; self.size];
                        self.progress = ParserProgress::Payload;
                    }

                    x => panic!("Invalid discriminator {}", x),
                }

                true
            }
            None => false,
        }
    }

    fn read_payload<R: Read>(&mut self, reader: &mut R) -> bool {
        let pos = self.payload.capacity() - self.size;
        let sz = reader.read(&mut self.payload[pos..]).unwrap();
        self.size -= sz;
        if self.size == 0 {
            self.progress = ParserProgress::Complete;
            true
        } else {
            false
        }
    }

    fn is_complete(&self) -> bool {
        matches!(self.progress, ParserProgress::Complete)
    }

    fn into_io_request(self) -> IoRequest {
        match self.progress {
            ParserProgress::Complete => IoRequest {
                id: self.ioid,
                userdata: self.userdata,
                payload: match self.discriminator {
                    0 => IoRequestPayload::Read(self.size as u64),
                    1 => IoRequestPayload::Write(self.payload),
                    x => panic!("Invalid discriminator {}", x),
                },
            },
            _ => panic!("TODO: this is wrong"),
        }
    }
}

pub enum ParserProgress {
    Id,
    Userdata,
    Discriminator,
    Size,
    Payload,
    Complete,
}

pub enum IoRequestPayload {
    Read(u64),
    Write(Vec<u8>),
}

pub struct IoRequest {
    id: IoId,
    userdata: u64,
    payload: IoRequestPayload,
}

impl IoRequest {
    fn parse<R: Read>(state: Option<ParserState>, mut reader: R) -> ParserState {
        let mut state = state.unwrap_or_default();

        while !state.is_complete() {
            match state.progress {
                ParserProgress::Id => {
                    if !state.read_ioid(&mut reader) {
                        return state;
                    }
                }
                ParserProgress::Userdata => {
                    if !state.read_userdata(&mut reader) {
                        return state;
                    }
                }
                ParserProgress::Discriminator => {
                    if !state.read_discriminator(&mut reader) {
                        return state;
                    }
                }
                ParserProgress::Size => {
                    if !state.read_size(&mut reader) {
                        return state;
                    }
                }
                ParserProgress::Payload => {
                    if !state.read_payload(&mut reader) {
                        return state;
                    }
                }
                ParserProgress::Complete => return state,
            }
        }

        state
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
    ErrorCode(u64),
    Payload,
    Completed,
}

impl Default for CompletionEventWriteProgress {
    fn default() -> Self {
        Self::Userdata
    }
}

impl IoCompletionEvent {
    fn write_to<W: Write>(&mut self, mut writer: W) -> Result<(), String> {
        while !self.write_completed() {
            match self.write_progress {
                CompletionEventWriteProgress::Userdata => {
                    let bytes = self.userdata.to_le_bytes();
                    let written = writer.write(&bytes[self.bytes_written..]).unwrap();
                    self.bytes_written += written;
                    if written < 8 {
                        return Ok(());
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
                    let written = writer.write(&byte).unwrap();

                    if written < 1 {
                        return Ok(());
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
                    let written = writer.write(&bytes[self.bytes_written..]).unwrap();
                    self.bytes_written += written;
                    if written < 8 {
                        return Ok(());
                    }

                    self.bytes_written = 0;
                    self.write_progress = CompletionEventWriteProgress::Payload
                }
                CompletionEventWriteProgress::Payload => match &self.payload {
                    IoCompletionEventPayload::ReadComplete(buf) => {
                        if self.bytes_written < buf.len() {
                            let written = writer.write(&buf[self.bytes_written..]).unwrap();
                            self.bytes_written += written;
                        }

                        if self.bytes_written == buf.len() {
                            self.bytes_written = 0;
                            self.write_progress = CompletionEventWriteProgress::Completed;
                        } else {
                            return Ok(());
                        }
                    }
                    _ => panic!("This should never happen"),
                },
                CompletionEventWriteProgress::ErrorCode(ec) => {
                    let bytes = ec.to_le_bytes();
                    let written = writer.write(&bytes[self.bytes_written..]).unwrap();
                    self.bytes_written += written;
                    if written < 8 {
                        return Ok(());
                    }

                    self.bytes_written = 0;
                    self.write_progress = CompletionEventWriteProgress::Completed;
                }
                CompletionEventWriteProgress::Completed => {}
            }
        }

        Ok(())
    }

    fn write_completed(&self) -> bool {
        matches!(self.write_progress, CompletionEventWriteProgress::Completed)
    }
}
