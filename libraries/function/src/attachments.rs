use std::{
    fs::OpenOptions,
    io::{Cursor, Error as StdError, ErrorKind as StdErrorKind, Read, Write},
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
    task::Poll,
};

use firm_protocols::functions::Attachment;
use flate2::read::GzDecoder;
use futures::{io::BufReader, ready, AsyncBufRead, AsyncRead, AsyncWrite};
use http::{HeaderMap, HeaderName, HeaderValue, Request, Response, StatusCode, Version};
use rustls::{ClientConnection, IoState};
use thiserror::Error;
use url::Url;

#[derive(Debug)]
struct BodyReader<Body: AsyncRead> {
    body: Body,
    content_length: Option<u64>,
    read_bytes: u64,
}

impl<Body: AsyncRead> BodyReader<Body> {
    fn new(body: Body, content_length: Option<u64>) -> Self {
        Self {
            body,
            content_length,
            read_bytes: 0,
        }
    }
}

impl<Body: AsyncRead + Unpin> AsyncRead for BodyReader<Body> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        if let Some(content_length) = self.content_length {
            if content_length == self.read_bytes {
                return Poll::Ready(Ok(0));
            }
        }

        let rb = ready!(Pin::new(&mut self.body).poll_read(cx, buf))?;
        self.read_bytes += rb as u64;
        Poll::Ready(Ok(rb))
    }
}

#[derive(Debug)]
enum GzDecoderState {
    ReadHeader([u8; 10], usize),
    Decode(Box<GzDecoder<Cursor<Vec<u8>>>>, bool),
}

#[derive(Debug)]
struct GzipBodyReader<Body: AsyncRead> {
    inner: Body,
    state: Option<GzDecoderState>,
}

impl<Body: AsyncRead> GzipBodyReader<Body> {
    fn new(inner: Body) -> Self {
        Self { inner, state: None }
    }

    const BUFFER_CAPACITY: usize = 4096;
}

impl<Body: AsyncRead + Unpin> AsyncRead for GzipBodyReader<Body> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.state.take() {
            Some(GzDecoderState::ReadHeader(mut encoded_buf, mut len)) => {
                match Pin::new(&mut self.inner).poll_read(cx, &mut encoded_buf[len..]) {
                    Poll::Ready(Ok(num)) => {
                        len += num;
                        if len == 10 {
                            let mut buffer = Vec::with_capacity(Self::BUFFER_CAPACITY);
                            buffer.extend_from_slice(&encoded_buf);
                            self.state = Some(GzDecoderState::Decode(
                                Box::new(GzDecoder::new(Cursor::new(buffer))),
                                false,
                            ));
                        } else {
                            self.state = Some(GzDecoderState::ReadHeader(encoded_buf, len));
                        }

                        cx.waker().clone().wake();
                        Poll::Pending
                    }
                    Poll::Pending => {
                        self.state = Some(GzDecoderState::ReadHeader(encoded_buf, len));
                        Poll::Pending
                    }
                    x => x,
                }
            }
            Some(GzDecoderState::Decode(mut decoder, mut encoded_consumed)) => {
                // Maximize buffer space.
                // We move all unread elements to the front and read to the back.
                // Just set the length of the vec to "forget" about the read values.
                let cursor = decoder.get_mut();
                let current_pos = cursor.position() as usize;
                cursor.set_position(0);

                let data = cursor.get_mut();
                data.rotate_left(current_pos);
                unsafe { data.set_len(data.len() - current_pos) }

                let len = data.len();
                if len < data.capacity() {
                    unsafe { data.set_len(data.capacity()) }
                    match Pin::new(&mut self.inner).poll_read(cx, &mut data[len..]) {
                        Poll::Ready(Ok(read)) => {
                            unsafe { data.set_len(len + read) }
                            if read == 0 {
                                encoded_consumed = true;
                            }
                        }
                        Poll::Pending => {}
                        Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                    };
                }

                let res = match decoder.read(buf) {
                    Ok(read) => match read {
                        0 if encoded_consumed || buf.is_empty() => Poll::Ready(Ok(0)),
                        0 => {
                            cx.waker().clone().wake();
                            Poll::Pending
                        }
                        read => Poll::Ready(Ok(read)),
                    },
                    Err(e) => Poll::Ready(Err(e)),
                };

                self.state = Some(GzDecoderState::Decode(decoder, encoded_consumed));
                res
            }
            None => {
                self.state = Some(GzDecoderState::ReadHeader([0u8; 10], 0));
                cx.waker().clone().wake();
                Poll::Pending
            }
        }
    }
}

#[derive(Debug)]
enum BodyStream<Body: AsyncRead + Unpin> {
    Uncompressed(BodyReader<Body>),
    UncompressedChunked(ChunkedDecoder<Body>),
    Gzip(GzipBodyReader<BodyReader<Body>>),
    GzipChunked(GzipBodyReader<ChunkedDecoder<Body>>),
}

impl<Body: AsyncRead + Unpin> AsyncRead for BodyStream<Body> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            BodyStream::Uncompressed(ref mut b) => Pin::new(b).poll_read(cx, buf),
            BodyStream::Gzip(ref mut b) => Pin::new(b).poll_read(cx, buf),
            BodyStream::UncompressedChunked(ref mut b) => Pin::new(b).poll_read(cx, buf),
            BodyStream::GzipChunked(ref mut b) => Pin::new(b).poll_read(cx, buf),
        }
    }
}

impl<Body: AsyncRead + Unpin> BodyStream<Body> {
    fn new(
        content_type: Option<&HeaderValue>,
        transfer_encoding: Option<&HeaderValue>,
        content_length: Option<&HeaderValue>,
        stream: Body,
    ) -> Self {
        match (
            content_type.map(|v| v.as_bytes()),
            transfer_encoding.map(|v| v.as_bytes()),
        ) {
            (Some(b"gzip"), Some(b"chunked")) => {
                Self::GzipChunked(GzipBodyReader::new(ChunkedDecoder::new(stream)))
            }
            (Some(b"gzip"), _) => {
                let content_length = content_length.map(|v| {
                    std::str::from_utf8(v.as_bytes())
                        .unwrap()
                        .parse::<u64>()
                        .unwrap()
                });
                Self::Gzip(GzipBodyReader::new(BodyReader::new(stream, content_length)))
            }
            (_, Some(b"chunked")) => Self::UncompressedChunked(ChunkedDecoder::new(stream)),
            _ => {
                let content_length = content_length.map(|v| {
                    std::str::from_utf8(v.as_bytes())
                        .unwrap()
                        .parse::<u64>()
                        .unwrap()
                });

                Self::Uncompressed(BodyReader::new(stream, content_length))
            }
        }
    }
}

#[derive(Debug)]
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

#[derive(Debug)]
struct ChunkedDecoder<Body: AsyncRead> {
    reader: BodyReader<Body>,
    read_state: Option<ReadChunkState>,
}

impl<Body: AsyncRead + Unpin> ChunkedDecoder<Body> {
    fn new(stream: Body) -> Self {
        Self {
            reader: BodyReader::new(stream, None),
            read_state: None,
        }
    }

    fn read_chunked(
        &mut self,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<Result<usize, StdError>> {
        match self.read_state.take() {
            Some(ReadChunkState::ReadChunkSize {
                mut chunk_size_buf,
                mut last_was_cr,
                mut skip_chunk_extensions,
            }) => {
                let mut mini_buf = [0u8; 1];
                loop {
                    match Pin::new(&mut self.reader).poll_read(cx, &mut mini_buf) {
                        Poll::Ready(Ok(1)) => {
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

                                cx.waker().clone().wake();
                                return Poll::Pending;
                            }

                            last_was_cr = mini_buf[0] == b'\r';

                            if !skip_chunk_extensions && mini_buf[0] != b' ' && mini_buf[0] != b'\t'
                            {
                                chunk_size_buf.push(mini_buf[0]);
                            }
                        }
                        Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                        _ => {
                            self.read_state = Some(ReadChunkState::ReadChunkSize {
                                chunk_size_buf,
                                last_was_cr,
                                skip_chunk_extensions,
                            });
                            return Poll::Pending;
                        }
                    }
                }
            }
            Some(ReadChunkState::ReadChunk { chunk_size, .. }) if chunk_size == 0 => {
                Poll::Ready(Ok(0))
            }
            Some(ReadChunkState::ReadChunk {
                chunk_size,
                mut left_in_chunk,
            }) => {
                let len = buf.len();
                match Pin::new(&mut self.reader)
                    .poll_read(cx, &mut buf[..len.min(left_in_chunk as usize)])
                {
                    Poll::Ready(Ok(read)) => {
                        left_in_chunk -= read as u64;

                        if left_in_chunk == 0 {
                            self.read_state = Some(ReadChunkState::SkipLine { last_was_cr: false });
                        } else {
                            self.read_state = Some(ReadChunkState::ReadChunk {
                                chunk_size,
                                left_in_chunk,
                            });
                        }
                        Poll::Ready(Ok(read))
                    }
                    Poll::Pending => {
                        self.read_state = Some(ReadChunkState::ReadChunk {
                            chunk_size,
                            left_in_chunk,
                        });
                        Poll::Pending
                    }
                    x => x,
                }
            }
            Some(ReadChunkState::SkipLine { mut last_was_cr }) => {
                let mut mini_buf = [0u8; 1];
                loop {
                    match Pin::new(&mut self.reader).poll_read(cx, &mut mini_buf) {
                        Poll::Ready(Ok(1)) => {
                            if mini_buf[0] == b'\n' && last_was_cr {
                                self.read_state = Some(ReadChunkState::ReadChunkSize {
                                    chunk_size_buf: Vec::with_capacity(16),
                                    last_was_cr: false,
                                    skip_chunk_extensions: false,
                                });

                                cx.waker().clone().wake();
                                return Poll::Pending;
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

                cx.waker().clone().wake();
                Poll::Pending
            }
        }
    }
}

impl<Body: AsyncRead + Unpin> AsyncRead for ChunkedDecoder<Body> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        self.get_mut().read_chunked(cx, buf)
    }
}

#[derive(Debug)]
enum AttachmentLoadState<Stream: AsyncRead + AsyncBufRead + AsyncWrite + Unpin> {
    Sending {
        stream: Stream,
        buf: Vec<u8>,
    },
    ReadingHeaders {
        headers: Headers,
        stream: Stream,
    },
    ReadingBody {
        response: Response<()>,
        stream: BodyStream<Stream>,
    },
}

impl<Stream> AttachmentLoadState<Stream>
where
    Stream: AsyncRead + AsyncWrite + AsyncBufRead + Unpin,
{
    fn new(stream: Stream) -> Self {
        Self::Sending {
            stream,
            buf: vec![],
        }
    }
}

#[cfg(feature = "tokio")]
#[derive(Debug)]
struct TokioTcpStream(tokio::net::TcpStream);

#[cfg(feature = "tokio")]
type TcpStream = TokioTcpStream;

#[cfg(feature = "tokio")]
impl AsyncRead for TokioTcpStream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        let mut rb = tokio::io::ReadBuf::new(buf);
        match tokio::io::AsyncRead::poll_read(Pin::new(&mut self.0), cx, &mut rb) {
            Poll::Ready(Ok(_)) => Poll::Ready(Ok(rb.filled().len())),
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(feature = "tokio")]
impl AsyncWrite for TokioTcpStream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        tokio::io::AsyncWrite::poll_write(Pin::new(&mut self.0), cx, buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        tokio::io::AsyncWrite::poll_flush(Pin::new(&mut self.0), cx)
    }

    fn poll_close(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        tokio::io::AsyncWrite::poll_shutdown(Pin::new(&mut self.0), cx)
    }
}

pub struct HttpAttachmentReader {
    url: Url,
    load_state: Option<Box<AttachmentLoadState<BufReader<Connection<TcpStream>>>>>,
}

#[derive(Debug)]
struct HttpConnection<Stream> {
    stream: Stream,
}

impl<Stream> HttpConnection<Stream> {
    fn new(stream: Stream) -> Self {
        Self { stream }
    }
}

impl<Stream: AsyncRead + Unpin> AsyncRead for HttpConnection<Stream> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl<Stream: AsyncWrite + Unpin> AsyncWrite for HttpConnection<Stream> {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_close(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_close(cx)
    }
}

#[derive(Debug)]
struct HttpsConnection<Stream> {
    stream: Stream,
    connection: ClientConnection,
}

impl<Stream> HttpsConnection<Stream> {
    fn new(
        stream: Stream,
        dns_name: &str,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let mut root_store = rustls::RootCertStore::empty();
        // TODO: should use the system store
        root_store.add_trust_anchors(webpki_roots::TLS_SERVER_ROOTS.0.iter().map(|ta| {
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
            stream,
            connection: conn,
        })
    }
}

struct IOWrapper<'a, 'b, Stream>(&'a mut Stream, &'a mut std::task::Context<'b>);

impl<Stream: AsyncRead + Unpin> Read for IOWrapper<'_, '_, Stream> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match Pin::new(&mut self.0).poll_read(&mut self.1, buf) {
            Poll::Ready(r) => r,
            Poll::Pending => Err(StdError::from(StdErrorKind::WouldBlock)),
        }
    }
}

impl<Stream: AsyncWrite + Unpin> Write for IOWrapper<'_, '_, Stream> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match Pin::new(&mut self.0).poll_write(&mut self.1, buf) {
            Poll::Ready(r) => r,
            Poll::Pending => Err(StdError::from(StdErrorKind::WouldBlock)),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match Pin::new(&mut self.0).poll_flush(&mut self.1) {
            Poll::Ready(r) => r,
            Poll::Pending => Err(StdError::from(StdErrorKind::WouldBlock)),
        }
    }
}

impl<Stream: AsyncRead + AsyncWrite + Unpin> HttpsConnection<Stream> {
    fn process_io(&mut self, cx: &mut std::task::Context<'_>) -> Result<IoState, StdError> {
        let mut read_res = None;
        if self.connection.wants_read() {
            read_res = Some(
                self.connection
                    .read_tls(&mut IOWrapper(&mut self.stream, cx)),
            );
        }

        let state = self
            .connection
            .process_new_packets()
            .map_err(|e| StdError::new(StdErrorKind::Other, e))
            .map(|state| state)?;

        let mut write_res = None;
        if self.connection.wants_write() {
            write_res = Some(
                self.connection
                    .write_tls(&mut IOWrapper(&mut self.stream, cx)),
            );
        }

        // combine the result from write and read this is to make sure that a write can
        // happen even if the read blocks which is necessary for TLS handshaking.
        match (read_res, write_res) {
            (None, None) => Ok(state),
            (None, Some(w)) => w.map(|_| state),
            (Some(r), None) => r.map(|_| state),
            (Some(r), Some(w)) => r.map(|_| state).and_then(|state| w.map(|_| state)),
        }
    }

    fn wake_maybe(&self, cx: &mut std::task::Context<'_>) {
        if self.connection.wants_read() || self.connection.wants_write() {
            cx.waker().wake_by_ref();
        }
    }
}

impl<Stream: AsyncRead + AsyncWrite + Unpin> AsyncRead for HttpsConnection<Stream> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.process_io(cx) {
            Ok(_) => match self.connection.reader().read(buf) {
                Ok(n) => Poll::Ready(Ok(n)),
                Err(ref err) if err.kind() == StdErrorKind::WouldBlock => {
                    self.wake_maybe(cx);
                    Poll::Pending
                }
                Err(e) => Poll::Ready(Err(e)),
            },
            Err(e) if e.kind() == StdErrorKind::WouldBlock => Poll::Pending,
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

impl<Stream: AsyncWrite + AsyncRead + Unpin> AsyncWrite for HttpsConnection<Stream> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let written = ready!(match self.connection.writer().write(buf) {
            Err(e) if e.kind() == StdErrorKind::WouldBlock => {
                self.wake_maybe(cx);
                Poll::Pending
            }
            r => Poll::Ready(r),
        })?;

        match self.process_io(cx) {
            Err(e) if e.kind() == StdErrorKind::WouldBlock => Poll::Pending,
            r => Poll::Ready(r.map(|_| written)),
        }
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.connection.writer().flush() {
            Err(e) if e.kind() == StdErrorKind::WouldBlock => {
                cx.waker().clone().wake();
                Poll::Pending
            }
            r => Poll::Ready(r),
        }
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.process_io(cx) {
            Ok(io_state)
                if io_state.peer_has_closed() && io_state.plaintext_bytes_to_read() == 0 =>
            {
                Poll::Ready(Ok(()))
            }
            r => Poll::Ready(r.map(|_| ())),
        }
    }
}

#[derive(Debug)]
enum Connection<Stream> {
    Http(HttpConnection<Stream>),
    Https(HttpsConnection<Stream>),
}

impl<Stream: AsyncRead + AsyncWrite + Unpin> AsyncRead for Connection<Stream> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            Connection::Http(h) => Pin::new(h).poll_read(cx, buf),
            Connection::Https(h) => Pin::new(h).poll_read(cx, buf),
        }
    }
}

impl<Stream: AsyncWrite + AsyncRead + Unpin> AsyncWrite for Connection<Stream> {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            Connection::Http(h) => Pin::new(h).poll_write(cx, buf),
            Connection::Https(h) => Pin::new(h).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Connection::Http(h) => Pin::new(h).poll_flush(cx),
            Connection::Https(h) => Pin::new(h).poll_flush(cx),
        }
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Connection::Http(h) => Pin::new(h).poll_close(cx),
            Connection::Https(h) => Pin::new(h).poll_close(cx),
        }
    }
}

#[derive(Debug)]
struct Headers {
    headers: HeaderMap,
    version: Version,
    status: StatusCode,
    parsing_headers: bool,
    linebuf: Vec<u8>,
}

impl Headers {
    fn new() -> Self {
        Self {
            headers: HeaderMap::new(),
            version: Version::HTTP_11,
            status: StatusCode::OK,
            parsing_headers: false,
            linebuf: Vec::with_capacity(256),
        }
    }
}

impl Default for Headers {
    fn default() -> Self {
        Self::new()
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

    #[cfg(feature = "tokio")]
    fn connect(&mut self) -> Result<BufReader<Connection<TcpStream>>, StdError> {
        let host = self.url.host().unwrap_or(url::Host::Domain("localhost"));
        let port = self.url.port_or_known_default().unwrap_or(80);

        let stream = match host {
            url::Host::Domain(host) => std::net::TcpStream::connect((host, port)),
            url::Host::Ipv4(ipv4) => std::net::TcpStream::connect((ipv4, port)),
            url::Host::Ipv6(ipv6) => std::net::TcpStream::connect((ipv6, port)),
        }?;

        stream.set_nonblocking(true)?;

        let stream = tokio::net::TcpStream::from_std(stream)?;

        match self.url.scheme() {
            "https" => Ok(BufReader::new(Connection::Https(
                HttpsConnection::new(TokioTcpStream(stream), host.to_string().as_str())
                    .map_err(|e| StdError::new(StdErrorKind::AddrNotAvailable, e))?,
            ))),
            _ => Ok(BufReader::new(Connection::Http(HttpConnection::new(
                TokioTcpStream(stream),
            )))),
        }
    }

    fn send_request<W: AsyncWrite + Unpin>(
        &self,
        mut stream: W,
        cx: &mut std::task::Context<'_>,
        buf: &mut Vec<u8>,
    ) -> Poll<Result<(), StdError>> {
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

        AsyncWrite::poll_write(Pin::new(&mut stream), cx, buf).map_ok(|_| ())
    }

    fn parse_headers<R: AsyncRead + Unpin>(
        mut stream: R,
        cx: &mut std::task::Context<'_>,
        headers: &mut Headers,
    ) -> Poll<Result<(), StdError>> {
        // TODO: this could be a stream impl
        // not sure if it gives anything though
        fn read_line<R: AsyncRead + Unpin>(
            mut reader: R,
            cx: &mut std::task::Context<'_>,
            line: &mut Vec<u8>,
        ) -> Poll<Result<Option<String>, StdError>> {
            let mut cr = false;

            loop {
                let mut byte = [0u8; 1];
                let nread = ready!(Pin::new(&mut reader).poll_read(cx, &mut byte))?;

                // stream is closed?
                if nread == 0 {
                    return Poll::Ready(Err(StdError::new(
                        StdErrorKind::UnexpectedEof,
                        "EOF while reading header lines",
                    )));
                }

                match byte[0] {
                    b'\r' => {
                        cr = true;
                    }
                    b'\n' if cr => {
                        if line.is_empty() {
                            return Poll::Ready(Ok(None));
                        } else {
                            let s = String::from_utf8_lossy(&line).into_owned();
                            line.clear();
                            return Poll::Ready(Ok(Some(s)));
                        }
                    }
                    b => {
                        cr = false;
                        line.push(b);
                    }
                }
            }
        }

        if !headers.parsing_headers {
            if let Some(status_line) = ready!(read_line(&mut stream, cx, &mut headers.linebuf))? {
                let mut space_splitted = status_line.split_whitespace();

                if let Some(version) = space_splitted.next() {
                    headers.version = match version {
                        "HTTP/0.9" => Version::HTTP_09,
                        "HTTP/1.0" => Version::HTTP_10,
                        "HTTP/1.1" => Version::HTTP_11,
                        "HTTP/2.0" => Version::HTTP_2,
                        "HTTP/3.0" => Version::HTTP_3,
                        _ => Version::HTTP_11,
                    };
                }

                if let Some(status) = space_splitted
                    .next()
                    .and_then(|s| StatusCode::try_from(s).ok())
                {
                    headers.status = status;
                }

                headers.parsing_headers = true;
            }
        }

        // parse headers
        while let Some(line) = ready!(read_line(&mut stream, cx, &mut headers.linebuf))? {
            // Split on first colon
            let mut colon_split = line.splitn(2, ':');
            if let (Some(key), Some(value)) = (colon_split.next(), colon_split.next()) {
                headers.headers.append(
                    HeaderName::try_from(key.trim())
                        .map_err(|e| StdError::new(StdErrorKind::InvalidData, e))?,
                    HeaderValue::try_from(value.trim_start_matches(':').trim())
                        .map_err(|e| StdError::new(StdErrorKind::InvalidData, e))?,
                );
            }
        }

        Poll::Ready(Ok(()))
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
        Url::parse(&self.url)
            .map_err(|e| AttachmentError::Url(self.name.clone(), e.to_string()))
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

impl AsyncRead for HttpAttachmentReader {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.load_state.take().map(|v| *v) {
            None => {
                let stream = self.connect()?;
                self.load_state = Some(Box::new(AttachmentLoadState::new(stream)));
                cx.waker().clone().wake();
                Poll::Pending
            }
            Some(AttachmentLoadState::Sending {
                mut stream,
                mut buf,
            }) => {
                match self.send_request(&mut stream, cx, &mut buf)? {
                    Poll::Ready(_) => {
                        self.load_state = Some(Box::new(AttachmentLoadState::ReadingHeaders {
                            stream,
                            headers: Headers::default(),
                        }));
                        cx.waker().clone().wake();
                    }
                    Poll::Pending => {}
                };

                Poll::Pending
            }
            Some(AttachmentLoadState::ReadingHeaders {
                mut headers,
                mut stream,
            }) => {
                match Self::parse_headers(&mut stream, cx, &mut headers)? {
                    Poll::Ready(_) => {
                        let mut response = Response::builder()
                            .version(headers.version)
                            .status(headers.status);
                        if let Some(h) = response.headers_mut() {
                            *h = headers.headers;
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
                                            format!("Invalid redirect. Failed to parse url \"{}\": {}", header, e),
                                        )
                                    })
                                })?;

                            self.load_state = None;
                            cx.waker().wake_by_ref();
                            return Poll::Pending;
                        } else if !response.status().is_success() {
                            return Poll::Ready(Err(StdError::new(
                                StdErrorKind::Other,
                                response.status().to_string(),
                            )));
                        }
                        self.load_state = Some(Box::new(AttachmentLoadState::ReadingBody {
                            stream: BodyStream::new(
                                response.headers().get("content-encoding"),
                                response.headers().get("transfer-encoding"),
                                response.headers().get("content-length"),
                                stream,
                            ),
                            response,
                        }));
                        cx.waker().clone().wake()
                    }
                    Poll::Pending => {
                        self.load_state = Some(Box::new(AttachmentLoadState::ReadingHeaders {
                            stream,
                            headers,
                        }));
                    }
                }

                Poll::Pending
            }
            Some(AttachmentLoadState::ReadingBody {
                response,
                mut stream,
            }) => {
                let res = Pin::new(&mut stream).poll_read(cx, buf);

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

impl AsyncRead for FileAttachmentReader {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.state {
            FileReadState::Reading(ref mut f) => Poll::Ready(f.read(buf)),
            FileReadState::Waiting => {
                self.state =
                    FileReadState::Reading(OpenOptions::new().read(true).open(&self.path)?);

                // next poll_read will start reading the file
                cx.waker().clone().wake();
                Poll::Pending
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Cursor, Read, Write},
        pin::Pin,
        task::Poll,
    };

    use flate2::{write::GzEncoder, Compression};
    use futures::AsyncRead;
    use http::{HeaderValue, StatusCode, Version};

    use crate::attachments::{BodyStream, ChunkedDecoder};

    use super::{Headers, HttpAttachmentReader};

    struct FakeSocket {
        buffer: Cursor<Vec<u8>>,
        closed: bool,
    }

    impl FakeSocket {
        fn new(initial_buffer: Vec<u8>) -> Self {
            Self {
                buffer: Cursor::new(initial_buffer),
                closed: false,
            }
        }

        fn new_static(buffer: Vec<u8>) -> Self {
            Self {
                buffer: Cursor::new(buffer),
                closed: true,
            }
        }

        #[allow(dead_code)]
        fn close(&mut self) {
            self.closed = true;
        }
    }

    impl AsyncRead for FakeSocket {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            buf: &mut [u8],
        ) -> Poll<std::io::Result<usize>> {
            if self.buffer.position() == self.buffer.get_ref().len() as u64 {
                if self.closed {
                    Poll::Ready(Ok(0))
                } else {
                    Poll::Pending
                }
            } else {
                Poll::Ready(self.buffer.read(buf))
            }
        }
    }

    impl Read for FakeSocket {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.buffer.read(buf)
        }
    }

    impl Write for FakeSocket {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let p = self.buffer.position();
            let r = self.buffer.write(buf);

            // reset since this a read-socket
            self.buffer.set_position(p);
            r
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.buffer.flush()
        }
    }

    macro_rules! fake_context {
        () => {{
            let waker = futures::task::noop_waker_ref();
            std::task::Context::from_waker(waker)
        }};
    }

    #[test]
    fn test_header_parsing() {
        // give too little data first time
        let mut sock = FakeSocket::new(b"HTTP/2.0".to_vec());

        let mut headers = Headers::default();
        let res =
            HttpAttachmentReader::parse_headers(&mut sock, &mut fake_context!(), &mut headers);
        assert!(
            res.is_pending(),
            "Expected us to not be done with parsing headers"
        );

        assert!(
            !headers.parsing_headers,
            "Expected to still be parsing the first line"
        );

        sock.write_all(b" 200 OK\r\n")
            .expect("Expected to be able to write to fake socket");

        let res =
            HttpAttachmentReader::parse_headers(&mut sock, &mut fake_context!(), &mut headers);
        assert!(
            res.is_pending(),
            "Expected us to not be done with parsing headers"
        );

        assert!(
            headers.parsing_headers,
            "Expected to be ready for parsing headers"
        );

        assert_eq!(
            headers.version,
            Version::HTTP_2,
            "Expected parse state version to be HTTP 2.0"
        );

        assert_eq!(
            headers.status,
            StatusCode::OK,
            "Expected status code to represent 200 (OK)"
        );

        sock.write_all(b"Content-Type: text/plain\r\n")
            .expect("Expected to be able to write to fake socket");

        let res =
            HttpAttachmentReader::parse_headers(&mut sock, &mut fake_context!(), &mut headers);
        assert!(res.is_pending(), "Expected to still be in header parsing after single header (but no empty line) has been added");
        assert!(
            !headers.headers.is_empty(),
            "Expected headers to contain headers after parsing one"
        );
        let content_type = headers.headers.get("content-type");
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
        sock.write_all(b"x-gbk-something: orother")
            .expect("Expected to be able to write to fake socket");
        let res =
            HttpAttachmentReader::parse_headers(&mut sock, &mut fake_context!(), &mut headers);
        assert!(res.is_pending(), "Expected to still be in header parsing");

        // now, write the missing newline from the previous header
        sock.write_all(b"\r\n")
            .expect("Expected to be able to write to fake socket");
        let res =
            HttpAttachmentReader::parse_headers(&mut sock, &mut fake_context!(), &mut headers);
        assert!(
            res.is_pending(),
            "Expected to still be in header parsing after missing newline was received"
        );

        // now actually end the headers
        sock.write_all(b"\r\n")
            .expect("Expected to be able to write to fake socket");
        let res =
            HttpAttachmentReader::parse_headers(&mut sock, &mut fake_context!(), &mut headers);
        assert!(res.is_ready(), "Expected body parsing to be next");
    }

    #[test]
    fn test_body_decoding() {
        let body = b"hejhej hemskt mycket hej";
        let mut body_compressed = vec![];
        GzEncoder::new(&mut body_compressed, Compression::fast())
            .write_all(body)
            .expect("Expect to be able to gzip body");

        let mut body_stream =
            BodyStream::new(None, None, None, FakeSocket::new_static(body.to_vec()));
        let mut buf = [0u8; 32];
        let res = Pin::new(&mut body_stream).poll_read(&mut fake_context!(), &mut buf);
        match res {
            Poll::Ready(Ok(read)) => {
                assert_eq!(read, body.len(), "Expected to read whole body");
                assert_eq!(body, &buf[0..read], "Expected body to be equal to itself");
            }
            Poll::Ready(err) => panic!("Expected body read to not error with {:#?}", err),
            Poll::Pending => panic!("Expected body to be ready."),
        }

        // compressed
        let mut body_stream = BodyStream::new(
            Some(&HeaderValue::from_static("gzip")),
            None,
            None,
            FakeSocket::new_static(body_compressed),
        );

        fn read_body<Y: AsyncRead + Unpin>(
            stream: &mut BodyStream<Y>,
            buf: &mut [u8],
        ) -> Result<usize, std::io::Error> {
            let mut bytes_read = 0;
            loop {
                match Pin::new(&mut *stream).poll_read(&mut fake_context!(), &mut buf[bytes_read..])
                {
                    Poll::Ready(Ok(read)) => {
                        bytes_read += read;
                        if read == 0 {
                            return Ok(bytes_read);
                        }
                    }
                    Poll::Pending => {}
                    Poll::Ready(Err(e)) => return Err(e),
                }
            }
        }

        let mut buf = [0u8; 32];
        let res = read_body(&mut body_stream, &mut buf);

        assert!(res.is_ok(), "Expected to be able to read compressed body");
        let read = res.unwrap();
        assert_eq!(read, body.len(), "Expected to read whole body");
        assert_eq!(body, &buf[0..read], "Expected body to be equal to itself");
    }

    #[test]
    fn test_chunked_body_decoding() {
        fn read_all(
            reader: &mut ChunkedDecoder<FakeSocket>,
            buf: &mut [u8],
        ) -> Result<usize, std::io::Error> {
            let mut bytes_read = 0;
            let mut done = false;
            while bytes_read < buf.len() && !done {
                match Pin::new(&mut *reader).poll_read(&mut fake_context!(), &mut buf[bytes_read..])
                {
                    Poll::Ready(Ok(read)) => {
                        bytes_read += read;
                        if read == 0 {
                            done = true;
                        }
                    }
                    Poll::Ready(Err(e)) => return Err(e),
                    _ => {}
                }
            }

            Ok(bytes_read)
        }

        let data = b"6 ;this= is; ignored=\"header \"; junk\r\na\r\nbcd\r\n2\r\n22\r\n0";
        let mut decoder = ChunkedDecoder::new(FakeSocket::new_static(data.to_vec()));
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
