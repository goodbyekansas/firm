use std::{
    fs::File, io, io::BufReader, io::Read, net::Ipv6Addr, net::SocketAddr, net::SocketAddrV6,
    path::Path, pin::Pin, sync::Arc, task::Context, task::Poll,
};

use futures::{Stream, TryFutureExt};
use rustls::ServerConfig;
use slog::{info, o, warn, Logger};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{server::TlsStream, TlsAcceptor as TlsAcceptorTokio};

pub fn get_tls_config(cert_file: &Path, key_file: &Path) -> Result<ServerConfig, String> {
    File::open(cert_file)
        .map_err(|e| {
            format!(
                "Failed to open certificates file \"{}\": {}",
                cert_file.display(),
                e
            )
        })
        .map(BufReader::new)
        .and_then(|mut reader| {
            let mut data = Vec::new();
            reader.read_to_end(&mut data).map_err(|e| {
                format!(
                    "Failed to read certificates file \"{}\": {}",
                    cert_file.display(),
                    e
                )
            })?;

            let certs = pem::parse_many(data);

            if certs.is_empty() {
                Err(format!(
                    "Certificates file \"{}\" contained zero valid certificates.",
                    cert_file.display()
                ))
            } else {
                Ok(certs
                    .into_iter()
                    .map(|cert| rustls::Certificate(cert.contents))
                    .collect())
            }
        })
        .and_then(|certs| {
            File::open(key_file)
                .map_err(|e| {
                    format!(
                        "Failed to open private key file \"{}\": {}",
                        key_file.display(),
                        e
                    )
                })
                .map(|key_file| (certs, BufReader::new(key_file)))
        })
        .and_then(|(certs, mut key_reader)| {
            let mut data = Vec::new();
            key_reader.read_to_end(&mut data).map_err(|e| {
                format!(
                    "Failed to read private key file \"{}\": {}",
                    key_file.display(),
                    e
                )
            })?;

            pem::parse(data)
                .map_err(|e| {
                    format!(
                        "Failed to parse private key file \"{}\": {}",
                        key_file.display(),
                        e
                    )
                })
                .map(|private_key| (certs, rustls::PrivateKey(private_key.contents)))
        })
        .and_then(|(certs, private_key)| {
            let mut cfg = ServerConfig::new(rustls::NoClientAuth::new());
            cfg.set_single_cert(certs, private_key)
                .map_err(|e| format!("Failed to set server certificates: {}", e))?;
            cfg.set_protocols(&[b"h2".to_vec()]);
            Ok(cfg)
        })
}

pub struct TlsAcceptor<'a> {
    acceptor: Pin<Box<dyn Stream<Item = Result<TlsStream<TcpStream>, io::Error>> + 'a>>,
}

// https://www.freedesktop.org/software/systemd/man/sd_listen_fds.html
#[cfg(unix)]
fn get_systemd_sockets() -> Vec<std::os::unix::io::RawFd> {
    std::env::var("LISTEN_FDS")
        .ok()
        .and_then(|x| x.parse().ok())
        .and_then(|offset| {
            let for_this_pid = match std::env::var("LISTEN_PID").as_ref().map(|x| x.as_str()) {
                Err(std::env::VarError::NotPresent) | Ok("") => true,
                Ok(val) if val.parse().ok() == Some(unsafe { libc::getpid() }) => true,
                _ => false,
            };

            std::env::remove_var("LISTEN_PID");
            std::env::remove_var("LISTEN_FDS");
            for_this_pid.then(|| {
                (0..offset)
                    .map(|offset| 3 + offset as std::os::unix::io::RawFd)
                    .collect()
            })
        })
        .unwrap_or_default()
}

impl<'a> TlsAcceptor<'a> {
    pub async fn new(
        config: ServerConfig,
        port: u16,
        log: Logger,
    ) -> Result<TlsAcceptor<'a>, String> {
        let addr = SocketAddr::V6(SocketAddrV6::new(
            Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 0),
            port,
            0,
            0,
        ));
        let tcp = {
            #[cfg(unix)]
            {
                // TODO: socket activation can give more than one socket to bind to
                // this only supports the first one
                if let Some(sock) = get_systemd_sockets().first() {
                    unsafe {
                        let sl =
                            <std::net::TcpListener as std::os::unix::io::FromRawFd>::from_raw_fd(
                                *sock,
                            );
                        sl.set_nonblocking(true).map(|_| sl)
                    }
                    .and_then(tokio::net::TcpListener::from_std)
                    .map(|l| {
                        info!(log, "Listening for requests on systemd socket {}", sock);
                        l
                    })
                    .map_err(|e| {
                        format!("Failed to create TCP listener from systemd socket: {}", e)
                    })?
                } else {
                    TcpListener::bind(&addr)
                        .await
                        .map(|l| {
                            info!(log, "Listening for requests on port {}", addr.port());
                            l
                        })
                        .map_err(|e| format!("Failed to bind TCP listener: {}", e))?
                }
            }
            #[cfg(windows)]
            {
                TcpListener::bind(&addr)
                    .await
                    .map(|l| {
                        info!(log, "Listening for requests on port {}", addr.port());
                        l
                    })
                    .map_err(|e| format!("Failed to bind TCP listener: {}", e))?
            }
        };
        let acceptor = TlsAcceptorTokio::from(Arc::new(config));

        Ok(Self {
            acceptor: Box::pin(async_stream::stream! {
                loop {
                    match tcp.accept().map_err(|e| (e, log.new(o!()))).and_then(|(socket, peer)| {
                        let peer_addr = peer.to_string();
                        acceptor.accept(socket).map_err(|e| (e, log.new(o!("peer" => peer_addr))))
                    }).await {
                        Ok(conn) => { yield Ok(conn); },
                        Err((e, log)) => {
                            warn!(log, "Client connection error: {}", e);
                            continue;
                        }
                    }
                }
            }),
        })
    }
}

impl hyper::server::accept::Accept for TlsAcceptor<'_> {
    type Conn = TlsStream<TcpStream>;
    type Error = io::Error;

    fn poll_accept(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
    ) -> Poll<Option<Result<Self::Conn, Self::Error>>> {
        Pin::new(&mut self.acceptor).poll_next(cx)
    }
}
