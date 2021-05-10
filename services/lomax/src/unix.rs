use std::path::PathBuf;

use futures::{FutureExt, TryFutureExt};
use slog::{info, Logger};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    signal::unix::{signal, SignalKind},
};

pub fn get_lomax_cfg_dir() -> Option<PathBuf> {
    Some(PathBuf::from("/etc/lomax"))
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

pub fn get_local_socket(username: &str) -> String {
    format!(
        "unix://localhost/tmp/avery-{username}.sock",
        username = username
    )
}

macro_rules! func_ret_null {
    ($code:expr, $error_message:expr) => {{
        let res = $code;
        (!res.is_null())
            .then(|| res)
            .ok_or_else(|| String::from($error_message))
    }};
}

macro_rules! func_ret_neg {
    ($code:expr, $error_message:expr) => {{
        let res = $code;
        (res >= 0)
            .then(|| ())
            .ok_or_else(|| String::from($error_message))
    }};
}

pub fn drop_privileges(username: &str, groupname: &str) -> Result<(), String> {
    unsafe {
        let uid = *func_ret_null!(
            libc::getpwnam(
                std::ffi::CString::new(username)
                    .map_err(|_| "Failed to create C-string for username")?
                    .as_ptr(),
            ),
            format!(
                "Failed to determine unix user id for username \"{}\"",
                username
            )
        )?;

        let gid = *func_ret_null!(
            libc::getgrnam(
                std::ffi::CString::new(groupname)
                    .map_err(|_| "Failed to create C-string for group name")?
                    .as_ptr(),
            ),
            format!(
                "Failed to determine unix group id for groupname \"{}\"",
                groupname
            )
        )?;

        // The call order here is of the utmost importance.
        func_ret_neg!(libc::setgid(gid.gr_gid), "Failed to set group id")?;
        func_ret_neg!(libc::setuid(uid.pw_uid), "Failed to set user id")?;
    }

    Ok(())
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
