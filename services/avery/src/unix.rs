use std::{
    pin::Pin,
    task::{Context, Poll},
};

use firm_types::{
    functions::{execution_server::ExecutionServer, registry_server::RegistryServer},
    tonic::transport::{server::Connected, Server},
};
use futures::{FutureExt, TryFutureExt};
use slog::{info, o, Logger};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::UnixListener,
    signal::unix::{signal, SignalKind},
};

use crate::{executor::ExecutionService, proxy_registry::ProxyRegistry};

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
            while let item = uds.accept().map_ok(|(st, _)| UnixStream(st)).await {
                yield item;
            }
        }
    };

    let server = Server::builder()
        .add_service(ExecutionServer::new(execution_service))
        .add_service(RegistryServer::new(proxy_registry))
        .serve_with_incoming_shutdown(
            incoming,
            shutdown_signal(log.new(o!("scope" => "shutdown"))),
        )
        .await
        .map_err(|e| e.to_string());
    std::fs::remove_file(local_socket_path).unwrap_or(());
    server
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
struct UnixStream(pub tokio::net::UnixStream);

impl Connected for UnixStream {}

impl AsyncRead for UnixStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for UnixStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}
