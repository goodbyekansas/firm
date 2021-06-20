use std::path::PathBuf;

use futures::TryFutureExt;
use tokio::io::{AsyncRead, AsyncWrite};

pub const DEFAULT_SOCKET_URL: &str = r#"windows://./pipe/avery-{username}"#;

pub async fn shutdown_signal(_log: slog::Logger) {
    tokio::signal::ctrl_c().await.map_or_else(|_| (), |_| ())
}

pub fn get_lomax_cfg_dir() -> Option<PathBuf> {
    std::env::var_os("PROGRAMDATA").map(|appdata| PathBuf::from(&appdata).join("lomax"))
}

pub fn drop_privileges(_: &str, _: &str) -> Result<(), String> {
    Ok(())
}

pub struct NamedPipe(tokio::net::NamedPipe);

impl AsyncRead for NamedPipe {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for NamedPipe {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

impl hyper::client::connect::Connection for NamedPipe {
    fn connected(&self) -> hyper::client::connect::Connected {
        hyper::client::connect::Connected::new()
    }
}

impl hyper::service::Service<http::Uri> for super::LocalAveryConnector {
    type Response = NamedPipe;
    type Error = std::io::Error;
    type Future = super::LocalConnectorFuture<Self>;

    fn poll_ready(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, _req: http::Uri) -> Self::Future {
        Box::pin(
            tokio::net::NamedPipe::connect(format!(
                r#"\\{}{}"#,
                self.uri.host().unwrap_or("."),
                self.uri.path().replace("/", "\\")
            ))
            .map_ok(NamedPipe),
        )
    }
}
