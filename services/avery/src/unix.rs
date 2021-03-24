use std::{
    net::{Ipv6Addr, SocketAddr, SocketAddrV6},
    pin::Pin,
    task::{Context, Poll},
};

use firm_types::{
    functions::{execution_server::ExecutionServer, registry_server::RegistryServer},
    tonic::{
        transport::{server::Connected, Server},
        Request, Status,
    },
};
use futures::{FutureExt, TryFutureExt};
use slog::{info, o, Logger};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::UnixListener,
    signal::unix::{signal, SignalKind},
};
use users::os::unix::UserExt;

use crate::{
    executor::ExecutionService,
    proxy_registry::ProxyRegistry,
    userinfo::{RequestUserInfoExt, UserInfo},
};

pub const INTERNAL_PORT_PATH: &str = "/tmp/avery.sock";
pub const DEFAULT_RUNTIME_DIR: &str = "/usr/share/avery/runtimes";

pub async fn create_trap_door(
    local_socket_path: &std::path::Path,
    execution_service: ExecutionService,
    proxy_registry: ProxyRegistry,
    log: Logger,
) -> Result<(), String> {
    let incoming = {
        let uds = UnixListener::bind(local_socket_path).map_err(|e| e.to_string())?;

        async_stream::stream! {
            while let item = uds.accept().map_ok(|(st, _)| {
                UnixStream {
                    stream: st,
                }
            }).await {
                yield item;
            }
        }
    };

    let server = Server::builder()
        .add_service(ExecutionServer::with_interceptor(
            execution_service,
            inject_user,
        ))
        .add_service(RegistryServer::with_interceptor(
            proxy_registry,
            inject_user,
        ))
        .serve_with_incoming_shutdown(
            incoming,
            shutdown_signal(log.new(o!("scope" => "shutdown"))),
        )
        .await
        .map_err(|e| e.to_string());
    std::fs::remove_file(local_socket_path).unwrap_or(());
    server
}

fn inject_user(req: Request<()>) -> Result<Request<()>, Status> {
    req.remote_addr()
        .ok_or_else(|| Status::unauthenticated("Could not find credentials.".to_owned()))
        .and_then(|val| {
            match val {
                // We have previously injected the ID as an Ipv6 address so we know we always get one.
                // Need to treat Ipv4 as an error case.
                SocketAddr::V6(ipv6) => {
                    let uid: u32 = u128::from(*ipv6.ip()) as u32;
                    let user = users::get_user_by_uid(uid).ok_or_else(|| {
                        Status::unauthenticated(format!("Failed to find user for id {}", uid))
                    })?;
                    req.with_user_info(&UserInfo {
                        // Might drop some chars from the users name but we do not care.
                        username: user.name().to_string_lossy().to_string(),
                        home_dir: user.home_dir().to_owned(),
                    })
                    .map_err(|_| {
                        Status::unauthenticated(
                            "Failed to insert user metadata in request.".to_owned(),
                        )
                    })
                }
                SocketAddr::V4(_) => Err(Status::unauthenticated(
                    "Could not find credentials.".to_owned(),
                )),
            }
        })
}

async fn sig_term() {
    match signal(SignalKind::terminate()) {
        Ok(mut stream) => stream.recv().await,
        Err(_) => futures::future::pending::<Option<()>>().await,
    };
}

pub async fn shutdown_signal(log: Logger) {
    futures::select! {
        () = tokio::signal::ctrl_c().map_ok_or_else(|_| (), |_| ()).fuse() => { info!(log, "Recieved Ctrl-C"); },
        () = sig_term().fuse() => { info!(log, "Recieved SIGTERM"); }
    }
}

#[derive(Debug)]
struct UnixStream {
    stream: tokio::net::UnixStream,
}

// Hack: Using existing data inside the request to insert an user id (cookie).
impl Connected for UnixStream {
    fn remote_addr(&self) -> Option<SocketAddr> {
        self.stream.peer_cred().ok().map(|v| {
            let encoded_cookie: u128 = v.uid() as u128 | ((v.gid() as u128) << 32);
            SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::from(encoded_cookie), 0, 0, 0))
        })
    }
}

impl AsyncRead for UnixStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for UnixStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}
