use std::{os::linux::fs::MetadataExt, path::PathBuf};

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

fn get_process_owner() -> (u32, u32) {
    unsafe { (libc::geteuid(), libc::getegid()) }
}

lazy_static::lazy_static! {
    static ref SOCKET_GROUP_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::new(());
}

async fn set_group(group_id: u32, path: &str) -> Result<(), String> {
    let _ = SOCKET_GROUP_LOCK.lock().await;
    unsafe {
        let (uid, gid) = get_process_owner();

        // Switch to root (0 is root)
        set_privileges(0, 0)?;

        func_ret_neg!(
            // You can give chown a -1 for user or group id to
            // make it not change it. The definition of this
            // function only takes u32 however, hence the u32::MAX.
            libc::chown(path.as_ptr() as *const i8, std::u32::MAX, group_id),
            format!("Failed to set group id for file {}", path)
        )?;

        set_privileges(uid, gid)
    }
}

unsafe fn set_privileges(uid: u32, gid: u32) -> Result<(), String> {
    // The call order here is of the utmost importance, need to set
    // group before user or else 💥
    func_ret_neg!(libc::setegid(gid), "Failed to set group id")?;
    func_ret_neg!(libc::seteuid(uid), "Failed to set user id")?;

    Ok(())
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

        set_privileges(uid.pw_uid, gid.gr_gid)
    }
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
        let path = self.uri.path().to_owned();
        let log = self.log.clone();
        Box::pin(
            futures::future::ready(std::fs::metadata(&path))
                .and_then(|metadata| async move {
                    let (_, egid) = get_process_owner();
                    if metadata.st_gid() != egid && egid != 0 {
                        info!(
                            log,
                            "Changing group ownership of unix socket to gid {}", egid
                        );
                        set_group(egid, &path)
                            .await
                            .map(|_| path)
                            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
                    } else {
                        Ok(path)
                    }
                })
                .and_then(|path| tokio::net::UnixStream::connect(path).map_ok(UnixStream)),
        ) as super::LocalConnectorFuture<Self>
    }
}
