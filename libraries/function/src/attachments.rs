use std::{
    fs::OpenOptions,
    io::{Cursor, Error as StdError, ErrorKind as StdErrorKind, Read, Write},
    marker::PhantomData,
    net::TcpStream,
    os::unix::prelude::AsRawFd,
    path::{Path, PathBuf},
    sync::Arc,
    task::Poll,
};

use firm_protocols::functions::Attachment;
use flate2::read::GzDecoder;
use http::{HeaderMap, HeaderName, HeaderValue, Request, Response, StatusCode, Version};
use libc::pollfd;
use rustls::{ClientConnection, IoState};
use thiserror::Error;
use url::Url;

use crate::io::{PollRead, PollWrite};

// TODO: Return error when we try to write and we didn't write everything.

struct ResponseParserState {
    buffer: Vec<u8>,
    buffer_len: usize,
    buffer_size: usize,
    parsing_headers: bool,
    version: Option<Version>,
    headers: HeaderMap,
    status_code: Option<StatusCode>,
}

impl ResponseParserState {
    fn new() -> Self {
        Self::new_with_capacity(1024)
    }

    fn new_with_capacity(buffer_size: usize) -> Self {
        Self {
            buffer: vec![0u8; buffer_size],
            buffer_len: Default::default(),
            buffer_size,
            parsing_headers: Default::default(),
            version: Default::default(),
            headers: Default::default(),
            status_code: Default::default(),
        }
    }
}

impl Default for ResponseParserState {
    fn default() -> Self {
        Self::new()
    }
}

struct BodyReader<Leftover: Read, Body: PollRead> {
    leftover: Leftover,
    body: Body,
    content_length: Option<u64>,
    read_bytes: u64,
}

impl<Leftover: Read, Body: PollRead> BodyReader<Leftover, Body> {
    fn new(leftover: Leftover, body: Body, content_length: Option<u64>) -> Self {
        Self {
            leftover,
            body,
            content_length,
            read_bytes: 0,
        }
    }
}

impl<Leftover: Read, Body: PollRead> PollRead for BodyReader<Leftover, Body> {
    fn poll_read(&mut self, buf: &mut [u8]) -> Result<Poll<usize>, std::io::Error> {
        if let Some(content_length) = self.content_length {
            if content_length == self.read_bytes {
                return Ok(Poll::Ready(0));
            }
        }

        self.leftover.read(buf).and_then(|read_bytes| {
            self.body
                .poll_read(&mut buf[read_bytes..])
                .map(|rb| match rb {
                    Poll::Ready(rb) => {
                        self.read_bytes += (rb + read_bytes) as u64;
                        Poll::Ready(rb + read_bytes)
                    }
                    Poll::Pending if read_bytes > 0 => {
                        self.read_bytes += read_bytes as u64;
                        Poll::Ready(read_bytes)
                    }
                    _ => Poll::Pending,
                })
        })
    }
}

#[derive(Debug)]
enum GzDecoderState {
    ReadHeader([u8; 10], usize),
    Decode(Box<GzDecoder<Cursor<Vec<u8>>>>, bool),
}

struct GzipBodyReader<Body: PollRead> {
    inner: Body,
    state: Option<GzDecoderState>,
}

impl<Body: PollRead> GzipBodyReader<Body> {
    fn new(inner: Body) -> Self {
        Self { inner, state: None }
    }
}

impl<Body: PollRead> PollRead for GzipBodyReader<Body> {
    fn poll_read(&mut self, buf: &mut [u8]) -> Result<Poll<usize>, std::io::Error> {
        match self.state.take() {
            Some(GzDecoderState::ReadHeader(mut encoded_buf, mut len)) => {
                match self.inner.poll_read(&mut encoded_buf[len..]) {
                    Ok(Poll::Ready(num)) => {
                        len += num;
                        if len >= 10 {
                            let mut buffer = Vec::with_capacity(4096);
                            buffer.extend_from_slice(&encoded_buf);
                            self.state = Some(GzDecoderState::Decode(
                                Box::new(GzDecoder::new(Cursor::new(buffer))),
                                false,
                            ));
                        } else {
                            self.state = Some(GzDecoderState::ReadHeader(encoded_buf, len));
                        }

                        Ok(Poll::Pending)
                    }
                    Ok(Poll::Pending) => {
                        self.state = Some(GzDecoderState::ReadHeader(encoded_buf, len));
                        Ok(Poll::Pending)
                    }
                    Err(e) => Err(e),
                }
            }
            Some(GzDecoderState::Decode(mut decoder, mut encoded_consumed)) => {
                if Cursor::get_ref(GzDecoder::get_ref(&decoder)).len() < 4096 && !encoded_consumed {
                    let len = Cursor::get_ref(GzDecoder::get_ref(&decoder)).len();
                    let data = Cursor::get_mut(GzDecoder::get_mut(&mut decoder));
                    unsafe { data.set_len(data.capacity()) }
                    match self.inner.poll_read(&mut data[len..]) {
                        Ok(Poll::Ready(read)) => {
                            unsafe { data.set_len(len + read) }
                            if read == 0 {
                                encoded_consumed = true;
                            }
                        }
                        Ok(Poll::Pending) => {}
                        Err(e) => return Err(e),
                    };
                }

                match decoder.read(buf) {
                    Ok(read) => {
                        // Minimize memory usage.
                        // We move all unread elements to the front and read to the back.
                        // Just set the length of the vec to "forget" about the read values.
                        // This should prevent the cursor+vec to increase indefinetly in size.
                        let cursor_pos = decoder.get_ref().position() as usize;
                        let data = decoder.get_mut().get_mut();
                        data.rotate_left(cursor_pos);
                        unsafe { data.set_len(cursor_pos) }
                        decoder.get_mut().set_position(0);
                        self.state = Some(GzDecoderState::Decode(decoder, encoded_consumed));

                        if read == 0 && encoded_consumed {
                            Ok(Poll::Ready(0))
                        } else if read == 0 {
                            Ok(Poll::Pending)
                        } else {
                            Ok(Poll::Ready(read))
                        }
                    }
                    Err(e) => Err(e),
                }
            }
            None => {
                self.state = Some(GzDecoderState::ReadHeader([0u8; 10], 0));
                Ok(Poll::Pending)
            }
        }
    }
}

enum BodyStream<Leftover: Read, Body: PollRead> {
    Uncompressed(BodyReader<Leftover, Body>),
    UncompressedChunked(ChunkedDecoder<Leftover, Body>),
    Gzip(GzipBodyReader<BodyReader<Leftover, Body>>),
    GzipChunked(GzipBodyReader<ChunkedDecoder<Leftover, Body>>),
}

impl<Leftover: Read, Body: PollRead> PollRead for BodyStream<Leftover, Body> {
    fn poll_read(&mut self, buf: &mut [u8]) -> Result<Poll<usize>, std::io::Error> {
        match self {
            BodyStream::Uncompressed(b) => b.poll_read(buf),
            BodyStream::Gzip(b) => b.poll_read(buf),
            BodyStream::UncompressedChunked(b) => b.poll_read(buf),
            BodyStream::GzipChunked(b) => b.poll_read(buf),
        }
    }
}

impl<Leftover: Read, Body: PollRead> BodyStream<Leftover, Body> {
    fn new(
        content_type: Option<&HeaderValue>,
        transfer_encoding: Option<&HeaderValue>,
        content_length: Option<&HeaderValue>,
        leftover_bytes: Leftover,
        stream: Body,
    ) -> Self {
        match (
            content_type.map(|v| v.as_bytes()),
            transfer_encoding.map(|v| v.as_bytes()),
        ) {
            (Some(b"gzip"), Some(b"chunked")) => Self::GzipChunked(GzipBodyReader::new(
                ChunkedDecoder::new(leftover_bytes, stream),
            )),
            (Some(b"gzip"), _) => {
                let content_length = content_length.map(|v| {
                    std::str::from_utf8(v.as_bytes())
                        .unwrap()
                        .parse::<u64>()
                        .unwrap()
                });
                Self::Gzip(GzipBodyReader::new(BodyReader::new(
                    leftover_bytes,
                    stream,
                    content_length,
                )))
            }
            (_, Some(b"chunked")) => {
                Self::UncompressedChunked(ChunkedDecoder::new(leftover_bytes, stream))
            }
            _ => {
                let content_length = content_length.map(|v| {
                    std::str::from_utf8(v.as_bytes())
                        .unwrap()
                        .parse::<u64>()
                        .unwrap()
                });

                Self::Uncompressed(BodyReader::new(leftover_bytes, stream, content_length))
            }
        }
    }
}

enum ReadChunkState {
    ReadChunkSize {
        chunk_size_buf: Vec<u8>,
        last_was_cr: bool,
        skip_chunk_extensions: bool,
    },
    ReadChunk {
        chunk_size: u64,
        left_in_chunk: u64,
    },
    SkipLine {
        last_was_cr: bool,
    },
}

struct ChunkedDecoder<Leftover: Read, Body: PollRead> {
    reader: BodyReader<Leftover, Body>,
    read_state: Option<ReadChunkState>,
}

impl<Leftover: Read, Body: PollRead> ChunkedDecoder<Leftover, Body> {
    fn new(leftover: Leftover, stream: Body) -> Self {
        Self {
            reader: BodyReader::new(leftover, stream, None),
            read_state: None,
        }
    }

    fn read_chunked(&mut self, buf: &mut [u8]) -> Result<Poll<usize>, StdError> {
        match self.read_state.take() {
            Some(ReadChunkState::ReadChunkSize {
                mut chunk_size_buf,
                mut last_was_cr,
                mut skip_chunk_extensions,
            }) => {
                let mut mini_buf = [0u8; 1];
                loop {
                    match self.reader.poll_read(&mut mini_buf) {
                        Ok(Poll::Ready(1)) => {
                            skip_chunk_extensions |= mini_buf[0] == b';';

                            if last_was_cr && mini_buf[0] == b'\n' {
                                if chunk_size_buf.last() == Some(&b'\r') {
                                    chunk_size_buf.pop();
                                }

                                let chunk_size = std::str::from_utf8(&chunk_size_buf)
                                    .map_err(|e| StdError::new(StdErrorKind::InvalidData, e))
                                    .and_then(|octet| {
                                        u64::from_str_radix(octet, 16).map_err(|e| {
                                            StdError::new(StdErrorKind::InvalidData, e)
                                        })
                                    })?;

                                self.read_state = Some(ReadChunkState::ReadChunk {
                                    chunk_size,
                                    left_in_chunk: chunk_size,
                                });
                                return Ok(Poll::Pending);
                            }

                            last_was_cr = mini_buf[0] == b'\r';

                            if !skip_chunk_extensions && mini_buf[0] != b' ' && mini_buf[0] != b'\t'
                            {
                                chunk_size_buf.push(mini_buf[0]);
                            }
                        }
                        Ok(r) => {
                            self.read_state = Some(ReadChunkState::ReadChunkSize {
                                chunk_size_buf,
                                last_was_cr,
                                skip_chunk_extensions,
                            });
                            return Ok(r);
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
            Some(ReadChunkState::ReadChunk { chunk_size, .. }) if chunk_size == 0 => {
                Ok(Poll::Ready(0))
            }
            Some(ReadChunkState::ReadChunk {
                chunk_size,
                mut left_in_chunk,
            }) => {
                let len = buf.len();
                match self
                    .reader
                    .poll_read(&mut buf[..len.min(left_in_chunk as usize)])
                {
                    Ok(Poll::Ready(read)) => {
                        left_in_chunk -= read as u64;

                        if left_in_chunk == 0 {
                            self.read_state = Some(ReadChunkState::SkipLine { last_was_cr: false });
                        } else {
                            self.read_state = Some(ReadChunkState::ReadChunk {
                                chunk_size,
                                left_in_chunk,
                            });
                        }
                        Ok(Poll::Ready(read))
                    }
                    Ok(Poll::Pending) => {
                        self.read_state = Some(ReadChunkState::ReadChunk {
                            chunk_size,
                            left_in_chunk,
                        });
                        Ok(Poll::Pending)
                    }
                    x => x,
                }
            }
            Some(ReadChunkState::SkipLine { mut last_was_cr }) => {
                let mut mini_buf = [0u8; 1];
                loop {
                    match self.reader.poll_read(&mut mini_buf) {
                        Ok(Poll::Ready(1)) => {
                            if mini_buf[0] == b'\n' && last_was_cr {
                                self.read_state = Some(ReadChunkState::ReadChunkSize {
                                    chunk_size_buf: Vec::with_capacity(16),
                                    last_was_cr: false,
                                    skip_chunk_extensions: false,
                                });
                                return Ok(Poll::Pending);
                            }
                            last_was_cr = mini_buf[0] == b'\r';
                        }
                        x => return x,
                    }
                }
            }
            None => {
                self.read_state = Some(ReadChunkState::ReadChunkSize {
                    chunk_size_buf: Vec::with_capacity(16),
                    last_was_cr: false,
                    skip_chunk_extensions: false,
                });

                Ok(Poll::Pending)
            }
        }
    }
}

impl<Leftover: Read, Body: PollRead> PollRead for ChunkedDecoder<Leftover, Body> {
    fn poll_read(&mut self, buf: &mut [u8]) -> Result<Poll<usize>, std::io::Error> {
        self.read_chunked(buf)
    }
}

enum AttachmentLoadState<Stream: PollRead + PollWrite> {
    Sending {
        stream: Stream,
        buf: Vec<u8>,
    },
    ReadingHeaders {
        stream: Stream,
        parse_state: ResponseParserState,
    },
    ReadingBody {
        response: Response<()>,
        stream: Box<BodyStream<Cursor<Vec<u8>>, Stream>>,
    },
}

impl<Stream> AttachmentLoadState<Stream>
where
    Stream: PollRead + PollWrite + AsRawFd,
{
    fn new(stream: Stream) -> Self {
        Self::Sending {
            stream,
            buf: vec![],
        }
    }
}

#[cfg(unix)]
struct StreamPoller<S> {
    write_poller: pollfd,
    read_poller: pollfd,
    marker: PhantomData<S>,
}

#[cfg(unix)]
impl<S: AsRawFd> StreamPoller<S> {
    pub fn new_from_stream(stream: &S) -> Self {
        Self {
            write_poller: pollfd {
                fd: stream.as_raw_fd(),
                events: libc::POLLOUT,
                revents: 0,
            },
            read_poller: pollfd {
                fd: stream.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            marker: PhantomData,
        }
    }

    pub fn readable(&mut self) -> Result<bool, StdError> {
        self.read_poller.revents = 0;
        let res = unsafe { libc::poll(&mut self.read_poller as *mut pollfd, 1, 0) };
        match res {
            1 => Ok(self.read_poller.revents & libc::POLLIN != 0),
            0 => Ok(false),
            error_code => Err(StdError::from_raw_os_error(-error_code)),
        }
    }

    pub fn writeable(&mut self) -> Result<bool, StdError> {
        self.write_poller.revents = 0;
        let res = unsafe { libc::poll(&mut self.write_poller as *mut pollfd, 1, 0) };
        match res {
            1 => Ok(self.write_poller.revents & libc::POLLOUT != 0),
            0 => Ok(false),
            error_code => Err(StdError::from_raw_os_error(-error_code)),
        }
    }
}

#[cfg(windows)]
struct StreamPoller {}

#[cfg(windows)]
impl StreamPoller {}

pub struct HttpAttachmentReader {
    url: Url,
    load_state: Option<Box<AttachmentLoadState<Connection<TcpStream>>>>,
}

struct HttpConnection<Stream> {
    stream: Stream,
    poller: StreamPoller<Stream>,
}

impl<Stream: AsRawFd> HttpConnection<Stream> {
    fn new(stream: Stream) -> Self {
        Self {
            poller: StreamPoller::new_from_stream(&stream),
            stream,
        }
    }
}

impl<Stream: Read + Write + AsRawFd> PollRead for HttpConnection<Stream> {
    fn poll_read(&mut self, buf: &mut [u8]) -> Result<Poll<usize>, std::io::Error> {
        if self.poller.readable()? {
            match self.stream.read(buf) {
                Ok(n) => Ok(Poll::Ready(n)),
                Err(e) if e.kind() == StdErrorKind::WouldBlock => Ok(Poll::Pending),
                Err(e) => Err(e),
            }
        } else {
            Ok(Poll::Pending)
        }
    }
}

impl<Stream: Read + Write + AsRawFd> PollWrite for HttpConnection<Stream> {
    fn poll_write(&mut self, buf: &[u8]) -> Result<Poll<()>, std::io::Error> {
        if self.poller.writeable()? {
            match self.stream.write(buf) {
                Ok(_) => Ok(Poll::Ready(())),
                // Non fatal. Write operation should be retried.
                Err(e) if e.kind() == StdErrorKind::Interrupted => Ok(Poll::Pending),
                Err(e) => Err(e),
            }
        } else {
            Ok(Poll::Pending)
        }
    }
}

struct HttpsConnection<Stream> {
    stream: Stream,
    poller: StreamPoller<Stream>,
    connection: ClientConnection,
}

impl<Stream: AsRawFd> HttpsConnection<Stream> {
    fn new(
        stream: Stream,
        dns_name: &str,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let mut root_store = rustls::RootCertStore::empty();
        // TODO: should use the system store
        root_store.add_server_trust_anchors(webpki_roots::TLS_SERVER_ROOTS.0.iter().map(|ta| {
            rustls::OwnedTrustAnchor::from_subject_spki_name_constraints(
                ta.subject,
                ta.spki,
                ta.name_constraints,
            )
        }));

        let config = rustls::ClientConfig::builder()
            .with_safe_defaults()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        let name = dns_name.try_into()?;
        let conn = ClientConnection::new(Arc::new(config), name)?;
        Ok(Self {
            poller: StreamPoller::new_from_stream(&stream),
            stream,
            connection: conn,
        })
    }
}

impl<Stream: Read + Write + AsRawFd> HttpsConnection<Stream> {
    fn process_io(&mut self) -> Result<Option<IoState>, StdError> {
        let state = if self.poller.readable()? && self.connection.wants_read() {
            self.connection
                .read_tls(&mut self.stream)
                .and_then(|_read| {
                    self.connection
                        .process_new_packets()
                        .map(Some)
                        .map_err(|e| StdError::new(StdErrorKind::Other, e))
                })?
        } else {
            None
        };

        if self.poller.writeable()? && self.connection.wants_write() {
            self.connection.write_tls(&mut self.stream)?;
        }

        Ok(state)
    }
}

impl<Stream: Read + Write + AsRawFd> PollRead for HttpsConnection<Stream> {
    fn poll_read(&mut self, buf: &mut [u8]) -> Result<Poll<usize>, std::io::Error> {
        match self.process_io()? {
            Some(io_state)
                if io_state.peer_has_closed() && io_state.plaintext_bytes_to_read() == 0 =>
            {
                Ok(Poll::Ready(0))
            }
            _ => match self.connection.reader().read(buf) {
                Ok(n) => Ok(Poll::Ready(n)),
                Err(e) if e.kind() == StdErrorKind::WouldBlock => Ok(Poll::Pending),
                Err(e) => Err(e),
            },
        }
    }
}

impl<Stream: Write + Read + AsRawFd> PollWrite for HttpsConnection<Stream> {
    fn poll_write(&mut self, buf: &[u8]) -> Result<Poll<()>, std::io::Error> {
        if self.poller.writeable()? {
            let _ = self.connection.writer().write(buf)?;

            self.process_io().map(|_| Poll::Ready(()))
        } else {
            Ok(Poll::Pending)
        }
    }
}

#[allow(dead_code)]
enum Connection<Stream> {
    Http(HttpConnection<Stream>),
    Https(HttpsConnection<Stream>),
}

impl<Stream: Read + Write + AsRawFd> PollRead for Connection<Stream> {
    fn poll_read(&mut self, buf: &mut [u8]) -> Result<Poll<usize>, std::io::Error> {
        match self {
            Connection::Http(h) => h.poll_read(buf),
            Connection::Https(h) => h.poll_read(buf),
        }
    }
}

impl<Stream: Write + Read + AsRawFd> PollWrite for Connection<Stream> {
    fn poll_write(&mut self, buf: &[u8]) -> Result<Poll<()>, std::io::Error> {
        match self {
            Connection::Http(h) => h.poll_write(buf),
            Connection::Https(h) => h.poll_write(buf),
        }
    }
}

impl<Stream: AsRawFd> AsRawFd for Connection<Stream> {
    fn as_raw_fd(&self) -> std::os::unix::prelude::RawFd {
        match self {
            Connection::Http(h) => h.stream.as_raw_fd(),
            Connection::Https(h) => h.stream.as_raw_fd(),
        }
    }
}

impl HttpAttachmentReader {
    pub fn new(url: Url) -> Self {
        Self {
            url,
            load_state: None,
        }
    }

    pub fn response(&self) -> Option<Response<()>> {
        match self.load_state.as_deref() {
            Some(AttachmentLoadState::ReadingBody { response, .. }) => {
                let mut builder = Response::builder();
                builder = builder.version(response.version());
                builder = builder.status(response.status());
                if let Some(headers) = builder.headers_mut() {
                    *headers = response.headers().clone();
                }

                Some(builder.body(()).unwrap_or_default())
            }
            _ => None,
        }
    }

    fn connect(&mut self) -> Result<Connection<TcpStream>, StdError> {
        let host = self.url.host().unwrap_or(url::Host::Domain("localhost"));
        let port = self.url.port_or_known_default().unwrap_or(80);

        let stream = match host {
            url::Host::Domain(host) => TcpStream::connect((host, port)),
            url::Host::Ipv4(ipv4) => TcpStream::connect((ipv4, port)),
            url::Host::Ipv6(ipv6) => TcpStream::connect((ipv6, port)),
        }?;

        stream.set_nonblocking(true)?;

        match self.url.scheme() {
            "https" => Ok(Connection::Https(
                HttpsConnection::new(stream, host.to_string().as_str())
                    .map_err(|e| StdError::new(StdErrorKind::AddrNotAvailable, e))?,
            )),
            _ => Ok(Connection::Http(HttpConnection::new(stream))),
        }
    }

    fn send_request<W: PollWrite>(
        &self,
        mut stream: W,
        buf: &mut Vec<u8>,
    ) -> Result<Poll<()>, StdError> {
        if buf.is_empty() {
            let mut reqbld = Request::get(self.url.to_string()).header("User-Agent", "firm/1.0");

            if let Some(host) = self.url.host_str() {
                reqbld = reqbld.header("Host", host);
            }

            let request = reqbld
                .header("Accept-Encoding", "gzip;q=1.0")
                .header("Accept-Encoding", "*;q=0.8")
                .body(())
                .map_err(|e| StdError::new(StdErrorKind::InvalidData, e))?;

            let request_str = format!(
                "{} {} {:?}\r\n",
                request.method(),
                request
                    .uri()
                    .path_and_query()
                    .map(|p| p.as_str())
                    .unwrap_or("/"),
                request.version(),
            );

            buf.extend(request_str.as_bytes());
            request.headers().iter().for_each(|(k, v)| {
                buf.extend(AsRef::<[u8]>::as_ref(k));
                buf.extend(b": ");
                buf.extend(v.as_bytes());
                buf.extend(b"\r\n");
            });
            buf.extend(b"\r\n");
        }

        stream.poll_write(buf)
    }

    fn parse_headers<R: PollRead>(
        &self,
        mut stream: R,
        parse_state: &mut ResponseParserState,
    ) -> Result<Poll<HeaderContinuation>, StdError> {
        struct HeaderLines<'a> {
            lines: Vec<&'a [u8]>,
            status: HeaderContinuation,
        }

        fn as_header_lines(buf: &[u8]) -> HeaderLines {
            let mut lines = Vec::new();
            let mut start_index = 0;
            let mut cr = false;

            let mut line_empty = true;

            for (i, byte) in buf.iter().enumerate() {
                match *byte {
                    b'\r' => {
                        cr = true;
                    }
                    b'\n' if cr => {
                        if line_empty {
                            return HeaderLines {
                                lines,
                                status: HeaderContinuation::Body(i + 1),
                            };
                        }
                        lines.push(&buf[start_index..i - 1]);
                        start_index = i + 1;
                        cr = false;
                        line_empty = true;
                    }
                    _ => {
                        line_empty = false;
                        cr = false;
                    }
                }
            }

            HeaderLines {
                lines,
                status: HeaderContinuation::Headers(start_index),
            }
        }

        const fn trim_whitespace(buf: &[u8]) -> &[u8] {
            let mut bytes = buf;
            while let [first, rest @ ..] = bytes {
                if first.is_ascii_whitespace() {
                    bytes = rest;
                } else {
                    break;
                }
            }

            while let [rest @ .., last] = bytes {
                if last.is_ascii_whitespace() {
                    bytes = rest;
                } else {
                    break;
                }
            }

            bytes
        }

        if parse_state.buffer[parse_state.buffer_len..].is_empty() {
            parse_state
                .buffer
                .resize(parse_state.buffer.len() + parse_state.buffer_size, 0);
        }

        let read_bytes = match stream.poll_read(&mut parse_state.buffer[parse_state.buffer_len..]) {
            Ok(Poll::Ready(read)) => Ok(read),
            Ok(Poll::Pending) => return Ok(Poll::Pending),
            Err(e) => Err(e),
        }? + parse_state.buffer_len;

        let header_lines = as_header_lines(&parse_state.buffer[0..read_bytes]);

        for line in header_lines.lines.into_iter() {
            if parse_state.parsing_headers {
                // Split on first colon
                if let Some(split) = line
                    .iter()
                    .enumerate()
                    .find(|(_k, v)| **v == b':')
                    .map(|(k, _)| k)
                {
                    let (key_bytes, value_bytes) = line.split_at(split);
                    let key = HeaderName::from_bytes(trim_whitespace(key_bytes))
                        .map_err(|e| StdError::new(StdErrorKind::InvalidData, e))?;
                    let value = HeaderValue::from_bytes(trim_whitespace(&value_bytes[1..]))
                        .map_err(|e| StdError::new(StdErrorKind::InvalidData, e))?;
                    parse_state.headers.append(key, value);
                }
            } else {
                let mut space_splitted = line.split(u8::is_ascii_whitespace); // Split on space

                if let Some(version) = space_splitted.next() {
                    parse_state.version = Some(match version {
                        b"HTTP/0.9" => Version::HTTP_09,
                        b"HTTP/1.0" => Version::HTTP_10,
                        b"HTTP/1.1" => Version::HTTP_11,
                        b"HTTP/2.0" => Version::HTTP_2,
                        b"HTTP/3.0" => Version::HTTP_3,
                        _ => Version::HTTP_11,
                    });
                }

                if let Some(status) = space_splitted
                    .next()
                    .and_then(|status_bytes| StatusCode::from_bytes(status_bytes).ok())
                {
                    parse_state.status_code = Some(status);
                }

                parse_state.parsing_headers = true;
            }
        }

        Ok(match header_lines.status {
            HeaderContinuation::Headers(split) => {
                parse_state.buffer.rotate_left(split);
                parse_state.buffer_len = read_bytes - split;
                Poll::Ready(HeaderContinuation::Headers(split))
            }
            HeaderContinuation::Body(split) => Poll::Ready(HeaderContinuation::Body(split)),
        })
    }

    fn read_body<R: PollRead>(
        &self,
        mut stream: R,
        buf: &mut [u8],
    ) -> Result<Poll<usize>, StdError> {
        stream.poll_read(buf)
    }
}

pub enum AttachmentReader {
    File(FileAttachmentReader),
    Http(HttpAttachmentReader),
}

pub trait AttachmentExt {
    fn create_reader(&self) -> Result<AttachmentReader, AttachmentError>;
}

#[derive(Error, Debug)]
pub enum AttachmentError {
    #[error("Url Error for attachment \"{0}\": {1}")]
    Url(String, String),

    #[error("Unsupported transport \"{transport}\" in url for attachment \"{name}\"")]
    UnsupportedTransport { name: String, transport: String },
}

impl AttachmentExt for Attachment {
    fn create_reader(&self) -> Result<AttachmentReader, AttachmentError> {
        self.url
            .as_ref()
            .ok_or_else(|| {
                AttachmentError::Url(
                    self.name.clone(),
                    String::from("Missing url which is required."),
                )
            })
            .and_then(|url| -> Result<_, _> {
                Url::parse(&url.url)
                    .map_err(|e| AttachmentError::Url(self.name.clone(), e.to_string()))
            })
            .and_then(|url| match url.scheme() {
                "file" => Ok(AttachmentReader::File(FileAttachmentReader::new(
                    url.path(),
                ))),
                "https" | "http" => Ok(AttachmentReader::Http(HttpAttachmentReader::new(url))),
                transport => Err(AttachmentError::UnsupportedTransport {
                    name: self.name.clone(),
                    transport: transport.to_owned(),
                }),
            })
    }
}

enum HeaderContinuation {
    Headers(usize),
    Body(usize),
}

impl PollRead for HttpAttachmentReader {
    fn poll_read(&mut self, buf: &mut [u8]) -> Result<Poll<usize>, StdError> {
        match self.load_state.take().map(|v| *v) {
            None => {
                let stream = self.connect()?;
                self.load_state = Some(Box::new(AttachmentLoadState::new(stream)));
                Ok(Poll::Pending)
            }
            Some(AttachmentLoadState::Sending {
                mut stream,
                mut buf,
            }) => {
                match self.send_request(&mut stream, &mut buf)? {
                    Poll::Ready(_) => {
                        self.load_state = Some(Box::new(AttachmentLoadState::ReadingHeaders {
                            stream,
                            parse_state: ResponseParserState::new(),
                        }));
                    }
                    Poll::Pending => {}
                };
                Ok(Poll::Pending)
            }
            Some(AttachmentLoadState::ReadingHeaders {
                mut parse_state,
                mut stream,
            }) => {
                match self.parse_headers(&mut stream, &mut parse_state)? {
                    Poll::Ready(HeaderContinuation::Headers(_)) => {
                        self.load_state = Some(Box::new(AttachmentLoadState::ReadingHeaders {
                            stream,
                            parse_state,
                        }));
                    }
                    Poll::Ready(HeaderContinuation::Body(split)) => {
                        parse_state.buffer.drain(0..split);

                        let mut response = Response::builder();
                        if let Some(version) = parse_state.version {
                            response = response.version(version);
                        }
                        if let Some(headers) = response.headers_mut() {
                            *headers = parse_state.headers;
                        }
                        if let Some(status) = parse_state.status_code {
                            response = response.status(status);
                        }
                        let response = response.body(()).unwrap_or_default();

                        if response.status().is_redirection() {
                            self.url = response
                                .headers()
                                .get("location")
                                .ok_or_else(|| {
                                    StdError::new(
                                        StdErrorKind::Other,
                                        "Invalid redirect. Missing location header",
                                    )
                                })
                                .and_then(|header| {
                                    header.to_str().map_err(|e| {
                                        StdError::new(
                                            StdErrorKind::Other,
                                            format!("Invalid redirect. Location header is not a valid string: {}", e),
                                        )
                                    })
                                })
                                .and_then(|header| {
                                    Url::parse(header).map_err(|e| {
                                        StdError::new(
                                            StdErrorKind::Other,
                                            format!("Invalid redirect. Failed to parse url: {}", e),
                                        )
                                    })
                                })?;

                            self.load_state = None;
                            return Ok(Poll::Pending);
                        } else if !response.status().is_success() {
                            return Err(StdError::new(
                                StdErrorKind::Other,
                                response.status().to_string(),
                            ));
                        }
                        self.load_state = Some(Box::new(AttachmentLoadState::ReadingBody {
                            stream: Box::new(BodyStream::new(
                                response.headers().get("content-encoding"),
                                response.headers().get("transfer-encoding"),
                                response.headers().get("content-length"),
                                Cursor::new(parse_state.buffer),
                                stream,
                            )),
                            response,
                        }));
                    }
                    _ => {
                        self.load_state = Some(Box::new(AttachmentLoadState::ReadingHeaders {
                            stream,
                            parse_state,
                        }));
                    }
                }

                Ok(Poll::Pending)
            }
            Some(AttachmentLoadState::ReadingBody {
                response,
                mut stream,
            }) => {
                let res = self.read_body(&mut stream, buf);

                self.load_state = Some(Box::new(AttachmentLoadState::ReadingBody {
                    response,
                    stream,
                }));

                res
            }
        }
    }
}

enum FileReadState {
    Reading(std::fs::File),
    Waiting,
}

pub struct FileAttachmentReader {
    path: PathBuf,
    state: FileReadState,
}

impl FileAttachmentReader {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        Self {
            path: path.as_ref().to_owned(),
            state: FileReadState::Waiting,
        }
    }
}

impl PollRead for FileAttachmentReader {
    fn poll_read(&mut self, buf: &mut [u8]) -> Result<Poll<usize>, std::io::Error> {
        match self.state {
            FileReadState::Reading(ref mut f) => f.read(buf).map(Poll::Ready),
            FileReadState::Waiting => {
                self.state =
                    FileReadState::Reading(OpenOptions::new().read(true).open(&self.path)?);

                // next poll_read will start reading the file
                Ok(Poll::Pending)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Cursor, Read, Write},
        net::{TcpListener, TcpStream},
        task::Poll,
    };

    use flate2::{write::GzEncoder, Compression};
    use http::{HeaderValue, StatusCode, Version};

    use crate::io::PollRead;

    use super::{
        BodyStream, ChunkedDecoder, HeaderContinuation, HttpAttachmentReader, ResponseParserState,
        StreamPoller,
    };

    #[test]
    fn test_poller() {
        // note that it might seem wild to use the actual network stack of the OS here,
        // and it is but to know that this will work with all the intricacies of platform
        // specifics, this is needed to make the test useful at all.
        let listener = TcpListener::bind(("localhost", 0)).expect("failed to bind socket");
        listener
            .set_nonblocking(true)
            .expect("Failed to make listener socket non-blocking");
        let stream = TcpStream::connect(
            listener
                .local_addr()
                .expect("failed to get local address of bound socket"),
        )
        .expect("failed to create socket");
        stream
            .set_nonblocking(true)
            .expect("Failed to make socket non-blocking");

        let mut poller = StreamPoller::new_from_stream(&stream);

        let writeable = poller.writeable();
        assert!(
            writeable.is_ok(),
            "Expected to be able to poll stream for writeability"
        );
        assert!(writeable.unwrap(), "Expected TCP stream to be writeable");

        let readable = poller.readable();
        assert!(
            readable.is_ok(),
            "Expected to be able to poll stream for readability"
        );
        assert!(!readable.unwrap(), "Expected stream to _not_ be writeable");

        let (mut server_stream, _) = listener.accept().expect("Failed to accept the connection");
        server_stream
            .write_all(b"hej svejs")
            .expect("Failed to write on server stream");

        let readable = poller.readable();
        assert!(
            readable.is_ok(),
            "Expected to be able to poll stream for readability"
        );
        assert!(
            readable.unwrap(),
            "Expected stream to be writeable after server wrote stuff"
        );
    }

    #[test]
    fn test_download_request() {
        let dl = HttpAttachmentReader::new(
            url::Url::parse("https://company.com/attachments/datta.tar.gz").unwrap(),
        );

        let mut bytes = vec![];
        let mut buff = vec![];
        let res = dl.send_request(Cursor::new(&mut bytes), &mut buff);
        assert!(res.is_ok(), "Expected to be able to generate a request");

        // it does not have to be a valid string, but ours is
        // if that changes, feel free to change this as well
        let request = String::from_utf8(bytes).expect("Expected request to be a valid string");
        let mut lines = request.lines();
        let first_line = lines.next();
        assert!(
            first_line.is_some(),
            "Expected request to have at least one line"
        );
        assert_eq!(
            first_line.unwrap(),
            "GET /attachments/datta.tar.gz HTTP/1.1",
            "Expected first line to be a valid HTTP request"
        );

        // check that the last line is empty
        let last_line = lines.last();
        assert!(last_line.is_some(), "Expected there to be a last line");
        assert!(
            last_line.unwrap().is_empty(),
            "Expected last line to only be a newline to denote an emtpy body"
        );
    }

    #[test]
    fn test_header_parsing() {
        // give too little data first time
        let mut data = b"HTTP/2.0".to_vec();
        let mut cursor = Cursor::new(&mut data);

        let dl = HttpAttachmentReader::new(
            url::Url::parse("https://company.com/attachments/datta.tar.gz").unwrap(),
        );

        let mut parse_state = ResponseParserState::new();
        let res = dl.parse_headers(&mut cursor, &mut parse_state);
        assert!(
            res.is_ok(),
            "Expected to be able to parse partial header content"
        );

        assert!(
            matches!(res.unwrap(), Poll::Ready(HeaderContinuation::Headers(_))),
            "Expected us to not be done with parsing headers"
        );

        assert!(
            !parse_state.parsing_headers,
            "Expected to still be parsing the first line"
        );

        let pos = cursor.position();
        cursor
            .write_all(b" 200 OK\r\n")
            .expect("Expected to be able to write to cursor");
        cursor.set_position(pos);

        let res = dl.parse_headers(&mut cursor, &mut parse_state);
        assert!(res.is_ok(), "Expected to be able to parse header content");
        assert!(
            matches!(res.unwrap(), Poll::Ready(HeaderContinuation::Headers(_))),
            "Expected us to not be done with parsing headers"
        );

        assert!(
            parse_state.parsing_headers,
            "Expected to be ready for parsing headers"
        );

        assert!(
            parse_state.version.is_some(),
            "Expected parse state to contain a version"
        );
        assert_eq!(
            parse_state.version.unwrap(),
            Version::HTTP_2,
            "Expected parse state version to be HTTP 2.0"
        );

        assert!(
            parse_state.status_code.is_some(),
            "Expected parse state to contain a status code"
        );
        assert_eq!(
            parse_state.status_code.unwrap(),
            StatusCode::OK,
            "Expected status code to represent 200 (OK)"
        );

        let pos = cursor.position();
        cursor
            .write_all(b"Content-Type: text/plain\r\n")
            .expect("Expected to be able to write to cursor");
        cursor.set_position(pos);

        let res = dl.parse_headers(&mut cursor, &mut parse_state);
        assert!(res.is_ok(), "Expected to be able to parse header content");
        assert!(
            !parse_state.headers.is_empty(),
            "Expected parse state to contain headers after parsing one"
        );
        let content_type = parse_state.headers.get("content-type");
        assert!(
            content_type.is_some(),
            "Expected content-type header to exist"
        );
        assert_eq!(
            content_type.unwrap(),
            b"text/plain".as_slice(),
            "Expected content-type to be text/plain"
        );

        // test end conditions
        let pos = cursor.position();
        cursor
            .write_all(b"x-gbk-something: orother")
            .expect("Expected to be able to write to cursor");
        cursor.set_position(pos);
        let res = dl.parse_headers(&mut cursor, &mut parse_state);
        assert!(res.is_ok(), "Expected to be able to parse header content");
        assert!(
            matches!(res.unwrap(), Poll::Ready(HeaderContinuation::Headers(_))),
            "Expected to still be in header parsing"
        );

        // now, write the missing newline from the previous header
        let pos = cursor.position();
        cursor
            .write_all(b"\r\n")
            .expect("Expected to be able to write to cursor");
        cursor.set_position(pos);
        let res = dl.parse_headers(&mut cursor, &mut parse_state);
        assert!(res.is_ok(), "Expected to be able to parse header content");
        assert!(
            matches!(res.unwrap(), Poll::Ready(HeaderContinuation::Headers(_))),
            "Expected to still be in header parsing after missing newline was received"
        );

        // now actually end the headers
        cursor
            .write_all(b"\r\n")
            .expect("Expected to be able to write to cursor");
        cursor.set_position(pos);
        let res = dl.parse_headers(&mut cursor, &mut parse_state);
        assert!(res.is_ok(), "Expected to be able to parse header content");
        assert!(
            matches!(res.unwrap(), Poll::Ready(HeaderContinuation::Body(_))),
            "Expected body parsing to be next"
        );
    }

    #[test]
    fn test_body_decoding() {
        let dl = HttpAttachmentReader::new(
            url::Url::parse("https://company.com/attachments/datta.tar.gz").unwrap(),
        );

        let body = b"hejhej hemskt mycket hej";
        let mut body_compressed = vec![];
        GzEncoder::new(&mut body_compressed, Compression::fast())
            .write_all(body)
            .expect("Expect to be able to gzip body");

        let body_stream = BodyStream::new(
            None,
            None,
            None,
            Cursor::new(Vec::with_capacity(0)),
            Cursor::new(&body),
        );
        let mut buf = [0u8; 32];
        let res = dl.read_body(body_stream, &mut buf);
        assert!(res.is_ok(), "Expected to be able to read uncompressed body");
        let read = res.unwrap();
        match read {
            Poll::Ready(read) => {
                assert_eq!(read, body.len(), "Expected to read whole body");
                assert_eq!(body, &buf[0..read], "Expected body to be equal to itself");
            }
            Poll::Pending => panic!("Expected body to be ready."),
        }

        // compressed
        let mut body_stream = BodyStream::new(
            Some(&HeaderValue::from_static("gzip")),
            None,
            None,
            Cursor::new(Vec::with_capacity(0)),
            Cursor::new(&body_compressed),
        );

        fn read_body<T: Read, Y: PollRead>(
            reader: &HttpAttachmentReader,
            stream: &mut BodyStream<T, Y>,
            buf: &mut [u8],
        ) -> Result<usize, std::io::Error> {
            let mut bytes_read = 0;
            loop {
                match reader.read_body(&mut *stream, &mut buf[bytes_read..]) {
                    Ok(Poll::Ready(read)) => {
                        bytes_read += read;
                        if read == 0 {
                            return Ok(bytes_read);
                        }
                    }
                    Ok(Poll::Pending) => {}
                    Err(e) => return Err(e),
                }
            }
        }

        let mut buf = [0u8; 32];
        let res = read_body(&dl, &mut body_stream, &mut buf);

        assert!(res.is_ok(), "Expected to be able to read compressed body");
        let read = res.unwrap();
        assert_eq!(read, body.len(), "Expected to read whole body");
        assert_eq!(body, &buf[0..read], "Expected body to be equal to itself");

        // leftover bytes (uncompressed)
        let body_stream = BodyStream::new(
            None,
            None,
            None,
            Cursor::new(body[..2].to_vec()),
            Cursor::new(&body[2..]),
        );
        let mut buf = [0u8; 32];

        let res = dl.read_body(body_stream, &mut buf);
        assert!(res.is_ok(), "Expected to be able to read uncompressed body");
        let read = res.unwrap();
        match read {
            Poll::Ready(read) => {
                assert_eq!(read, body.len(), "Expected to read whole body");
                assert_eq!(body, &buf[0..read], "Expected body to be equal to itself");
            }
            Poll::Pending => panic!("Body was not ready."),
        }

        // leftover bytes (compressed)
        let mut body_stream = BodyStream::new(
            Some(&HeaderValue::from_static("gzip")),
            None,
            None,
            Cursor::new(body_compressed[..4].to_vec()),
            Cursor::new(&body_compressed[4..]),
        );
        let mut buf = [0u8; 32];
        let res = read_body(&dl, &mut body_stream, &mut buf);
        assert!(res.is_ok(), "Expected to be able to read uncompressed body");
        let read = res.unwrap();
        assert_eq!(read, body.len(), "Expected to read whole body");
        assert_eq!(body, &buf[0..read], "Expected body to be equal to itself");
    }

    #[test]
    fn test_chunked_body_decoding() {
        fn read_all(
            reader: &mut ChunkedDecoder<Cursor<&[u8]>, Cursor<&[u8]>>,
            buf: &mut [u8],
        ) -> Result<usize, std::io::Error> {
            let mut bytes_read = 0;
            let mut done = false;
            while bytes_read < buf.len() && !done {
                match reader.poll_read(&mut buf[bytes_read..]) {
                    Ok(Poll::Ready(read)) => {
                        bytes_read += read;
                        if read == 0 {
                            done = true;
                        }
                    }
                    Err(e) => return Err(e),
                    _ => {}
                }
            }

            Ok(bytes_read)
        }

        let data = b"ed=\"header \"; junk\r\na\r\nbcd\r\n2\r\n22\r\n0";
        let leftover_data = b"6 ;this= is; ignor";
        let mut decoder = ChunkedDecoder::new(
            Cursor::new(leftover_data.as_slice()),
            Cursor::new(data.as_slice()),
        );
        let mut buf = [0u8; 2];
        let res = read_all(&mut decoder, &mut buf);
        assert!(matches!(res, Ok(2)));
        assert_eq!(&buf, b"a\r");
        let mut buf = [0u8; 6];
        let res = read_all(&mut decoder, &mut buf);
        assert!(matches!(res, Ok(6)));
        assert_eq!(&buf, b"\nbcd22");
    }
}
