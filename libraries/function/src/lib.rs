pub mod stream;

pub mod io {
    use std::task::Poll;

    pub trait PollRead {
        fn poll_read(&mut self, buf: &mut [u8]) -> Result<Poll<usize>, std::io::Error>;
    }

    pub trait PollWrite {
        fn poll_write(&mut self, buf: &[u8]) -> Result<Poll<()>, std::io::Error>;
    }
}

pub mod attachments {
    use std::{
        fs::OpenOptions,
        io::{Error as StdError, ErrorKind as StdErrorKind, Read, Write},
        net::TcpStream,
        os::unix::prelude::AsRawFd,
        path::{Path, PathBuf},
        task::Poll,
    };

    use firm_protocols::functions::Attachment;
    use http::{HeaderMap, HeaderName, HeaderValue, Request, Response, StatusCode, Version};
    use libc::pollfd;
    use thiserror::Error;
    use url::Url;

    use crate::io::PollRead;

    #[derive(Default)]
    struct ResponseParserState {
        parsing_headers: bool,
        version: Option<Version>,
        headers: HeaderMap,
        status_code: Option<StatusCode>,
    }

    enum AttachmentLoadState {
        Sending {
            stream: TcpStream,
            poller: StreamPoller,
        },
        ReadingHeaders {
            stream: TcpStream,
            poller: StreamPoller,
            buffer: Vec<u8>,
            buffer_len: usize,
            parse_state: ResponseParserState,
        },
        ReadingBody {
            response: Response<()>,
            leftover_body: Vec<u8>,
            stream: TcpStream,
            poller: StreamPoller,
        },
    }

    #[cfg(unix)]
    pub struct StreamPoller {
        write_poller: pollfd,
        read_poller: pollfd,
    }

    #[cfg(unix)]
    impl StreamPoller {
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

    #[cfg(unix)]
    impl From<&TcpStream> for StreamPoller {
        fn from(stream: &TcpStream) -> Self {
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
            }
        }
    }

    #[cfg(windows)]
    impl StreamPoller {}

    pub struct HttpAttachmentReader {
        url: Url,
        load_state: Option<Box<AttachmentLoadState>>,
    }

    impl HttpAttachmentReader {
        pub fn new(url: Url) -> Self {
            Self {
                url,
                load_state: None,
            }
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

    struct HeaderLines<'a> {
        lines: Vec<&'a [u8]>,
        status: HeaderContinuation,
    }

    fn as_header_lines(buf: &[u8]) -> HeaderLines {
        let mut lines = Vec::new();
        let mut start_index = 0;
        let mut state = false;
        let mut last_cr_lf = false;

        for (i, byte) in buf.iter().enumerate() {
            if !state && *byte == 0xd {
                // CR
                state = true;
            } else if state && *byte == 0xa {
                //LF
                if last_cr_lf {
                    return HeaderLines {
                        lines,
                        status: HeaderContinuation::Body(i + 1),
                    };
                }

                lines.push(&buf[start_index..i - 1]);
                start_index = i + 1;
                state = false;
                last_cr_lf = true;
            } else {
                last_cr_lf = false;
            }
        }

        HeaderLines {
            lines,
            status: HeaderContinuation::Headers(start_index),
        }
    }

    impl PollRead for HttpAttachmentReader {
        fn poll_read(&mut self, buf: &mut [u8]) -> Result<Poll<usize>, StdError> {
            match self.load_state.take().map(|v| *v) {
                None => {
                    let host = self
                        .url
                        .host()
                        .ok_or_else(|| StdError::from(StdErrorKind::InvalidInput))?;

                    let port = match (self.url.scheme(), self.url.port()) {
                        (_, Some(port)) => port,
                        ("https", _) => 443,
                        _ => 80,
                    };

                    let stream = match host {
                        url::Host::Domain(host) => TcpStream::connect((host, port)),
                        url::Host::Ipv4(ipv4) => TcpStream::connect((ipv4, port)),
                        url::Host::Ipv6(ipv6) => TcpStream::connect((ipv6, port)),
                    }?;
                    stream.set_nonblocking(true)?;
                    self.load_state = Some(Box::new(AttachmentLoadState::Sending {
                        poller: (&stream).into(),
                        stream,
                    }));
                    Ok(Poll::Pending)
                }
                Some(AttachmentLoadState::Sending {
                    mut stream,
                    mut poller,
                }) => {
                    if !poller.writeable()? {
                        self.load_state =
                            Some(Box::new(AttachmentLoadState::Sending { stream, poller }));
                        return Ok(Poll::Pending);
                    }
                    let request = Request::get(self.url.to_string())
                        .header("User-Agent", "firm/1.0")
                        .header("Host", self.url.host_str().unwrap())
                        .body(())
                        .map_err(|e| StdError::new(StdErrorKind::InvalidData, e))?;
                    let request_str = format!(
                        "{} {} {:?}
",
                        request.method(),
                        request
                            .uri()
                            .path_and_query()
                            .map(|p| p.as_str())
                            .unwrap_or("/"),
                        request.version(),
                    );
                    let mut buf = request_str.into_bytes();
                    request.headers().iter().for_each(|(k, v)| {
                        buf.extend(AsRef::<[u8]>::as_ref(k));
                        buf.extend(b": ");
                        buf.extend(v.as_bytes());
                        buf.extend(b"\r\n");
                    });
                    buf.extend(b"\r\n");
                    stream.write_all(&buf)?;
                    stream.flush()?;

                    self.load_state = Some(Box::new(AttachmentLoadState::ReadingHeaders {
                        stream,
                        poller,
                        buffer: vec![0u8; 1024],
                        buffer_len: 0,
                        parse_state: ResponseParserState::default(),
                    }));
                    Ok(Poll::Pending)
                }
                Some(AttachmentLoadState::ReadingHeaders {
                    mut stream,
                    mut poller,
                    mut buffer,
                    buffer_len,
                    mut parse_state,
                }) => {
                    if !poller.readable()? {
                        self.load_state = Some(Box::new(AttachmentLoadState::ReadingHeaders {
                            stream,
                            poller,
                            buffer,
                            buffer_len,
                            parse_state,
                        }));
                        return Ok(Poll::Pending);
                    }

                    let read_bytes = stream.read(&mut buffer[buffer_len..])?;
                    let header_lines = as_header_lines(&buffer[0..read_bytes + buffer_len]);

                    for line in header_lines.lines.into_iter() {
                        if parse_state.parsing_headers {
                            let mut colon_splitted = line.split(|v| *v == 0x3A); // Split on colon
                            let header_name =
                                HeaderName::from_bytes(colon_splitted.next().unwrap()).unwrap();
                            let header_value =
                                HeaderValue::from_bytes(colon_splitted.next().unwrap()).unwrap();
                            parse_state.headers.append(header_name, header_value);
                        } else {
                            let mut space_splitted = line.split(|b| *b == 0x20); // Split on space
                            let version = space_splitted.next().unwrap();
                            let status =
                                StatusCode::from_bytes(space_splitted.next().unwrap()).unwrap();
                            let _description = space_splitted.next().unwrap();

                            parse_state.status_code = Some(status);
                            parse_state.version = Some(match version {
                                b"HTTP/0.9" => Version::HTTP_09,
                                b"HTTP/1.0" => Version::HTTP_10,
                                b"HTTP/1.1" => Version::HTTP_11,
                                b"HTTP/2.0" => Version::HTTP_2,
                                b"HTTP/3.0" => Version::HTTP_3,
                                _ => Version::HTTP_11,
                            });
                            parse_state.parsing_headers = true;
                        }
                    }

                    match header_lines.status {
                        HeaderContinuation::Headers(split) => {
                            buffer.rotate_left(split);

                            self.load_state = Some(Box::new(AttachmentLoadState::ReadingHeaders {
                                stream,
                                poller,
                                buffer_len: read_bytes - split,
                                buffer,
                                parse_state,
                            }));
                        }
                        HeaderContinuation::Body(split) => {
                            let mut response =
                                Response::builder().version(parse_state.version.unwrap());
                            if let Some(headers) = response.headers_mut() {
                                *headers = parse_state.headers;
                            }
                            response = response.status(parse_state.status_code.unwrap());
                            let a = buffer.drain(0..split);
                            drop(a);
                            self.load_state = Some(Box::new(AttachmentLoadState::ReadingBody {
                                response: response.body(()).unwrap(),
                                leftover_body: buffer,
                                stream,
                                poller,
                            }));
                        }
                    }

                    Ok(Poll::Pending)
                }
                Some(AttachmentLoadState::ReadingBody {
                    response,
                    mut leftover_body,
                    mut stream,
                    mut poller,
                }) => {
                    if !response.status().is_success() {
                        return Err(StdError::from(StdErrorKind::AddrInUse));
                    }

                    let mut bytes_read = 0;
                    if !leftover_body.is_empty() {
                        let min_len = buf.len().min(leftover_body.len());
                        buf[0..min_len].copy_from_slice(leftover_body.drain(0..min_len).as_slice());
                        bytes_read = min_len;
                    }

                    if buf.len() == bytes_read {
                        self.load_state = Some(Box::new(AttachmentLoadState::ReadingBody {
                            response,
                            leftover_body,
                            stream,
                            poller,
                        }));
                        return Ok(Poll::Ready(bytes_read));
                    }

                    if !poller.readable()? {
                        self.load_state = Some(Box::new(AttachmentLoadState::ReadingBody {
                            response,
                            leftover_body,
                            stream,
                            poller,
                        }));
                        if bytes_read > 0 {
                            return Ok(Poll::Ready(bytes_read));
                        } else {
                            return Ok(Poll::Pending);
                        }
                    }
                    let written_bytes = stream.read(&mut buf[bytes_read..])?;
                    bytes_read += written_bytes;

                    self.load_state = Some(Box::new(AttachmentLoadState::ReadingBody {
                        response,
                        leftover_body,
                        stream,
                        poller,
                    }));
                    Ok(Poll::Ready(bytes_read))
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
}
