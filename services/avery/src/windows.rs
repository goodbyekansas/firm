use std::{
    path::PathBuf,
    pin::Pin,
    task::{Context, Poll},
};

use firm_types::tonic::transport::server::Connected;
use futures::TryFutureExt;
use slog::{info, Logger};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::NamedPipeServerBuilder,
};
use winapi::um::{errhandlingapi::GetLastError, winbase::GetUserNameW};

pub const DEFAULT_RUNTIME_DIR: &str = r#"%PROGRAMDATA%\Avery\runtimes"#;

unsafe fn get_user() -> Option<String> {
    const CAPACITY: usize = 1024;
    let mut size = CAPACITY as u32;
    let mut name: [u16; CAPACITY] = [0; CAPACITY];
    (GetUserNameW(name.as_mut_ptr(), &mut size as *mut u32) != 0)
        .then(|| String::from_utf16_lossy(&name[..(size as usize) - 1]))
}

pub fn user() -> Option<String> {
    unsafe { get_user() }
}

pub fn user_config_path() -> Option<PathBuf> {
    std::env::var("LOCALAPPDATA")
        .ok()
        .map(|p| PathBuf::from(p).join("avery").join("config"))
}

pub fn user_cache_path() -> Option<PathBuf> {
    std::env::var("LOCALAPPDATA")
        .ok()
        .map(|p| PathBuf::from(p).join("avery").join("cache"))
}

pub fn user_data_path() -> Option<PathBuf> {
    std::env::var("LOCALAPPDATA")
        .ok()
        .map(|p| PathBuf::from(p).join("avery"))
}

pub async fn create_listener(
    log: Logger,
) -> Result<
    (
        async_stream::AsyncStream<
            std::result::Result<NamedPipe, std::io::Error>,
            impl futures::Future<Output = ()>,
        >,
        Option<Box<dyn FnOnce()>>,
    ),
    String,
> {
    let pipe_path = format!(
        r#"\\.\pipe\avery-{username}"#,
        username = unsafe {
            get_user()
                .ok_or_else(|| format!("Failed to determine windows user name: {}", GetLastError()))
        }?
    );

    info!(
        log,
        "👨‍⚖️ The Firm is listening for requests on pipe {}", &pipe_path
    );

    Ok((
        {
            let pipe = NamedPipeServerBuilder::new(pipe_path)
                .with_accept_remote(false)
                .build()
                .map_err(|e| format!("Failed to create named pipe: {}", e))?;

            async_stream::stream! {
                while let item = pipe.accept().map_ok(|np| NamedPipe(np)).await {
                    yield item;
                }
            }
        },
        None,
    ))
}

pub async fn shutdown_signal(_log: Logger) {
    tokio::signal::ctrl_c().await.map_or_else(|_| (), |_| ())
}

#[derive(Debug)]
pub struct NamedPipe(pub tokio::net::NamedPipe);

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
