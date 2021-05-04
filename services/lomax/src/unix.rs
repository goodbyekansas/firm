use futures::{FutureExt, TryFutureExt};
use slog::{info, Logger};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    signal::unix::{signal, SignalKind},
};

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

pub fn get_local_socket(username: &str) -> String {
    format!(
        "unix://localhost/tmp/avery-{username}.sock",
        username = username
    )
}

pub struct UnixStream(tokio::net::UnixStream);

impl AsyncRead for UnixStream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for UnixStream {
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

impl hyper::client::connect::Connection for UnixStream {
    fn connected(&self) -> hyper::client::connect::Connected {
        hyper::client::connect::Connected::new()
    }
}

impl hyper::service::Service<http::Uri> for super::LocalAveryConnector {
    type Response = UnixStream;
    type Error = std::io::Error;
    type Future = super::LocalConnectorFuture<Self>;

    fn poll_ready(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, _req: http::Uri) -> Self::Future {
        Box::pin(tokio::net::UnixStream::connect(self.uri.path().to_owned()).map_ok(UnixStream))
    }
}
