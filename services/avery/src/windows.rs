use std::{
    pin::Pin,
    task::{Context, Poll},
};

use firm_types::{
    functions::{execution_server::ExecutionServer, registry_server::RegistryServer},
    tonic::transport::{server::Connected, Server},
};
use futures::TryFutureExt;
use slog::{o, Logger};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::NamedPipeServerBuilder,
};

use crate::{executor::ExecutionService, proxy_registry::ProxyRegistry};

pub const INTERNAL_PORT_PATH: &str = r#"\\.\pipe\avery"#;
pub const DEFAULT_RUNTIME_DIR: &str = r#"%PROGRAMDATA%\Avery\runtimes"#;

pub async fn create_trap_door(
    local_socket_path: &std::path::Path,
    execution_service: ExecutionService,
    proxy_registry: ProxyRegistry,
    log: Logger,
) -> Result<(), String> {
    let incoming = {
        let pipe = NamedPipeServerBuilder::new(local_socket_path)
            .with_accept_remote(false)
            .build()
            .map_err(|e| format!("Failed to create named pipe: {}", e))?;

        async_stream::stream! {
            while let item = pipe.accept().map_ok(|np| NamedPipe(np)).await {
                yield item;
            }
        }
    };

    Server::builder()
        .add_service(ExecutionServer::new(execution_service))
        .add_service(RegistryServer::new(proxy_registry))
        .serve_with_incoming_shutdown(
            incoming,
            shutdown_signal(log.new(o!("scope" => "shutdown"))),
        )
        .await
        .map_err(|e| e.to_string())
}

pub async fn shutdown_signal(_log: Logger) {
    tokio::signal::ctrl_c().await.map_or_else(|_| (), |_| ())
}

#[derive(Debug)]
struct NamedPipe(pub tokio::net::NamedPipe);

impl Connected for NamedPipe {}

impl AsyncRead for NamedPipe {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for NamedPipe {
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
