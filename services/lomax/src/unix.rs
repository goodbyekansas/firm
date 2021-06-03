use std::{os::unix::io::FromRawFd, path::PathBuf};

use futures::{FutureExt, TryFutureExt};
use slog::{info, Logger};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    signal::unix::{signal, SignalKind},
};

pub const DEFAULT_SOCKET_URL: &str = "unix://localhost/tmp/avery-{username}.sock";

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

// https://www.freedesktop.org/software/systemd/man/sd_listen_fds.html
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
        // TODO: socket activation can give more than one socket to bind to
        // this only supports the first one
        if let Some(sock) = get_systemd_sockets().first() {
            Box::pin(
                futures::future::ready(tokio::net::UnixStream::from_std(unsafe {
                    std::os::unix::net::UnixStream::from_raw_fd(*sock)
                }))
                .map_ok(UnixStream),
            ) as super::LocalConnectorFuture<Self>
        } else {
            Box::pin(tokio::net::UnixStream::connect(self.uri.path().to_owned()).map_ok(UnixStream))
                as super::LocalConnectorFuture<Self>
        }
    }
}
