use std::{
    os::unix::io::FromRawFd,
    path::PathBuf,
    pin::Pin,
    task::{Context, Poll},
};

use firm_types::tonic::transport::server::Connected;
use futures::{FutureExt, TryFutureExt};
use slog::{info, Logger};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::UnixListener,
    signal::unix::{signal, SignalKind},
};
use users::get_current_username;

pub fn default_runtime_dir() -> PathBuf {
    PathBuf::from("/usr/share/avery/runtimes")
}

pub fn user() -> Option<String> {
    get_current_username().map(|x| x.to_string_lossy().to_string())
}

pub fn global_config_path() -> Option<PathBuf> {
    Some(PathBuf::from("/etc/avery"))
}

pub fn user_config_path() -> Option<PathBuf> {
    match std::env::var("XDG_CONFIG_HOME").ok() {
        Some(p) => Some(PathBuf::from(p)),
        None => std::env::var("HOME")
            .ok()
            .map(|p| PathBuf::from(p).join(".config")),
    }
    .map(|p| p.join("avery"))
}

pub fn user_cache_path() -> Option<PathBuf> {
    match std::env::var("XDG_CACHE_HOME").ok() {
        Some(p) => Some(PathBuf::from(p)),
        None => std::env::var("HOME")
            .ok()
            .map(|p| PathBuf::from(p).join(".cache")),
    }
    .map(|p| p.join("avery"))
}

pub fn user_data_path() -> Option<PathBuf> {
    match std::env::var("XDG_DATA_HOME").ok() {
        Some(p) => Some(PathBuf::from(p)),
        None => std::env::var("HOME")
            .ok()
            .map(|p| PathBuf::from(p).join(".local").join("share")),
    }
    .map(|p| p.join("avery"))
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

pub async fn create_listener(
    log: Logger,
) -> Result<
    (
        async_stream::AsyncStream<
            std::result::Result<UnixStream, std::io::Error>,
            impl futures::Future<Output = ()>,
        >,
        Option<Box<dyn FnOnce()>>,
    ),
    String,
> {
    // TODO: socket activation can give more than one socket to bind to
    // this only supports the first one
    let (uds, cleanup) = if let Some(sock) = get_systemd_sockets().first() {
        info!(
            log,
            "🧦 The Firm is listening for requests on socket fd (started with socket activation) {}", &sock
        );

        unsafe {
            let sl = std::os::unix::net::UnixListener::from_raw_fd(*sock);
            sl.set_nonblocking(true).map(|_| sl)
        }
        .and_then(UnixListener::from_std)
        .map_err(|e| format!("Failed to convert Unix listener from std: {}", e))
        .map(|uds| (uds, None))
    } else {
        let socket_path = format!(
            "/tmp/avery-{username}.sock",
            username = get_current_username()
                .ok_or_else(|| "Failed to determine current unix user name.".to_owned())?
                .to_string_lossy()
        );

        info!(
            log,
            "👨⚖️ The Firm is listening for requests on socket {}", &socket_path
        );

        UnixListener::bind(&socket_path)
            .map_err(|e| e.to_string())
            .map(|uds| {
                (
                    uds,
                    Some(Box::new(|| std::fs::remove_file(socket_path).unwrap_or(()))
                        as Box<dyn FnOnce()>),
                )
            })
    }?;

    Ok((
        async_stream::stream! {
            while let item = uds.accept().map_ok(|(st, _)| UnixStream(st)).await {
                yield item;
            }
        },
        cleanup,
    ))
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
pub struct UnixStream(pub tokio::net::UnixStream);

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
