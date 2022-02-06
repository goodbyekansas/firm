use std::{
    fs::File, io, io::BufReader, io::Read, net::Ipv6Addr, net::SocketAddr, net::SocketAddrV6,
    path::Path, pin::Pin, sync::Arc, task::Context, task::Poll,
};

use futures::{Stream, TryFutureExt};
use rustls::ServerConfig;
use slog::{info, o, warn, Logger};
use tokio::net::TcpStream;
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
                    tokio::net::TcpListener::bind(&addr)
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
                use std::os::windows::io::AsRawSocket;
                use winapi::um::winsock2::setsockopt;

                // This enables windows named pipes to listen to
                // both Ipv4 and Ipv6.
                tokio::net::TcpSocket::new_v6()
                    .map_err(|e| format!("Failed to create TCP socket: {}", e))
                    .and_then(|socket| {
                        unsafe {
                            setsockopt(
                                socket.as_raw_socket() as usize,
                                winapi::shared::ws2def::IPPROTO_IPV6 as i32,
                                winapi::shared::ws2ipdef::IPV6_V6ONLY,
                                (&0u32 as *const u32).cast::<i8>(),
                                4,
                            )
                        };
                        socket
                            .bind(addr)
                            .map_err(|e| format!("Failed to bind TPC socket: {}", e))?;
                        socket
                            .listen(1024)
                            .map(|l| {
                                info!(log, "Listening for requests on port {}", addr.port());
                                l
                            })
                            .map_err(|e| format!("Failed listen on TCP socket: {}", e))
                    })?
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

/// Get the current config version
///
/// Note that the version is a hash and therefore any comparisons except for equality does
/// not make sense. If the version file does not exists, the version returned is 5381.
pub fn get_certificate_version(config: &crate::config::Config) -> Result<u32, String> {
    let mut version_file = config.certificate_locations.key.clone();
    version_file.set_extension("version");
    if !version_file.exists() {
        return Ok(5381u32); // see hash used below
    }
    std::fs::read_to_string(&version_file)
        .map_err(|e| format!("Failed to read certificate version file: {}", e))
        .and_then(|s| {
            s.parse()
                .map_err(|e| format!("Failed to parse certificate version: {}", e))
        })
}

/// Create a version number for the certificate based on config
///
/// Note that this version number is not monotonically increasing
/// but rather a hash of select fields of the config
pub fn create_cert_version(config: &crate::config::Config) -> u32 {
    config
        .certificate_alt_names
        .join(" ")
        .chars()
        // djb2 hash: http://www.cse.yorku.ca/~oz/hash.html
        .fold(5381u32, |hash, c| {
            ((hash << 5).wrapping_add(hash)).wrapping_add(c as u32)
        })
}

/// Create a self-signed certificate
pub fn create_certificate(
    config: &crate::config::Config,
    log: Logger,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(
        config
            .certificate_locations
            .key
            .parent()
            .ok_or("Failed to get certificate key parent directory")?,
    )
    .map_err(|e| format!("Failed to create certificate key directory: {}", e))?;

    std::fs::create_dir_all(
        config
            .certificate_locations
            .cert
            .parent()
            .ok_or("Failed to get certificate parent directory")?,
    )
    .map_err(|e| format!("Failed to create certificate directory: {}", e))?;

    info!(log, "Generating self signed certificate.");

    // determine alt names for the certificate and support {hostname} replacement for the
    // given alt names
    let mut alt_names = config.certificate_alt_names.clone();
    alt_names.extend(["{hostname}".to_string(), "localhost".to_string()]);
    let hostname = hostname::get()
        .map_err(|e| format!("Failed to get host name: {}", e))?
        .to_string_lossy()
        .to_string();

    rcgen::generate_simple_self_signed(
        alt_names
            .into_iter()
            .map(|an| an.replace("{hostname}", &hostname))
            .collect::<Vec<_>>(),
    )
    .map_err(|e| format!("Failed to generate self signed certificate: {}", e))
    .and_then(|cert| {
        cert.serialize_pem()
            .map(|pem_cert| (pem_cert, cert.serialize_private_key_pem()))
            .map_err(|e| format!("Failed to serialize certificate: {}", e))
    })
    .and_then(|(cert, key)| {
        std::fs::write(&config.certificate_locations.key, key)
            .map_err(|e| format!("Failed to write certificate key: {}", e))
            .map(|_| cert)
    })
    .and_then(|cert| {
        let mut version_file = config.certificate_locations.key.clone();
        version_file.set_extension("version");
        std::fs::write(&version_file, format!("{}", create_cert_version(config)))
            .map_err(|e| format!("Failed to write certificate key version: {}", e))
            .map(|_| cert)
    })
    .and_then(|cert| {
        std::fs::write(&config.certificate_locations.cert, cert)
            .map_err(|e| format!("Failed to write certificate: {}", e))
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::tls::{create_certificate, get_certificate_version};

    use super::create_cert_version;

    macro_rules! null_logger {
        () => {{
            slog::Logger::root(slog::Discard, slog::o!())
        }};
    }

    #[test]
    fn certificate_version() {
        let mut c = crate::config::Config {
            certificate_alt_names: vec![String::from("{hostname}.fabriken.se")],
            ..Default::default()
        };

        let version = create_cert_version(&c);
        c.certificate_alt_names = vec![String::from("{hostname}.fabrikan.se")];

        assert_ne!(
            version,
            create_cert_version(&c),
            "Expected different certificate \
             alt name settings to produce different \
             certificate versions"
        );
    }

    #[test]
    fn create_self_signed_cert() {
        let cert_dir =
            tempfile::tempdir().expect("Failed to create temp directory for holding certificates");

        let mut c = crate::config::Config {
            certificate_locations: crate::config::CertificateLocations {
                cert: cert_dir.path().join("cert.pem"),
                key: cert_dir.path().join("cert.key"),
            },
            ..Default::default()
        };

        let expected_version = create_cert_version(&c);
        assert!(
            create_certificate(&c, null_logger!()).is_ok(),
            "Expected to be able to create a certificate with default settings"
        );
        assert!(
            c.certificate_locations.key.exists(),
            "Expected certificate key to exist after creation"
        );
        assert!(
            c.certificate_locations.cert.exists(),
            "Expected certificate to exist after creation"
        );
        let mut version_file = c.certificate_locations.key.clone();
        version_file.set_extension("version");
        assert!(
            version_file.exists(),
            "Expected certificate key version to exist after creation"
        );
        assert_eq!(
            get_certificate_version(&c).expect("Failed to obtain certificate version"),
            expected_version,
            "Expected certificate version to match after creation"
        );

        // test setting alt name
        c.certificate_alt_names = vec![String::from("{hostname}.fabriken.se")];
        let expected_version = create_cert_version(&c);
        assert!(
            create_certificate(&c, null_logger!()).is_ok(),
            "Expected to be able to create a certificate with alt name set"
        );
        assert!(
            c.certificate_locations.key.exists(),
            "Expected certificate key to exist after creation with alt name set"
        );
        assert!(
            c.certificate_locations.cert.exists(),
            "Expected certificate to exist after creation with alt name set"
        );
        let mut version_file = c.certificate_locations.key.clone();
        version_file.set_extension("version");
        assert!(
            version_file.exists(),
            "Expected certificate key version to exist after creation with alt name set"
        );

        assert_eq!(
            get_certificate_version(&c).expect("Failed to obtain certificate version"),
            expected_version,
            "Expected certificate version to match after creation"
        );
    }
}
