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
        let tcp = TcpListener::bind(&addr)
            .await
            .map_err(|e| format!("Failed to bind TCP listener: {}", e))?;
        let acceptor = TlsAcceptorTokio::from(Arc::new(config));

        info!(log, "Listening for requests on port {}", addr.port());

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
