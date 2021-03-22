use std::{
    ffi::OsString,
    net::{Ipv6Addr, SocketAddr, SocketAddrV6},
    os::windows::{ffi::OsStringExt, io::AsRawHandle, raw::HANDLE},
    path::PathBuf,
    pin::Pin,
    ptr,
    task::{Context, Poll},
};

use firm_types::{
    functions::{execution_server::ExecutionServer, registry_server::RegistryServer},
    tonic::{
        transport::{server::Connected, Server},
        Request, Status,
    },
};
use futures::TryFutureExt;
use slog::{o, Logger};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::NamedPipeServerBuilder,
};
use winapi::{
    shared::winerror::S_OK,
    um::{
        combaseapi::CoTaskMemFree,
        knownfolders::FOLDERID_Profile,
        namedpipeapi::ImpersonateNamedPipeClient,
        securitybaseapi::RevertToSelf,
        shlobj::SHGetKnownFolderPath,
        winbase::{lstrlenW, GetUserNameW},
        winnt::PWSTR,
    },
};

use crate::{executor::ExecutionService, proxy_registry::ProxyRegistry, userinfo::apply_user_info};

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
        .map_err(|e| e.to_string())
}

pub async fn shutdown_signal(_log: Logger) {
    tokio::signal::ctrl_c().await.map_or_else(|_| (), |_| ())
}

unsafe fn get_home_folder() -> Option<PathBuf> {
    let mut path_ptr: PWSTR = ptr::null_mut();

    let home_folder = (SHGetKnownFolderPath(&FOLDERID_Profile, 0, ptr::null_mut(), &mut path_ptr)
        == S_OK)
        .then(|| {
            let ostr: OsString = OsStringExt::from_wide(::std::slice::from_raw_parts(
                path_ptr,
                lstrlenW(path_ptr) as usize,
            ));
            PathBuf::from(ostr)
        });

    CoTaskMemFree(path_ptr as *mut winapi::ctypes::c_void);
    home_folder
}

unsafe fn get_user() -> Option<String> {
    const CAPACITY: usize = 1024;
    let mut size = CAPACITY as u32;
    let mut name: [u16; CAPACITY] = [0; CAPACITY];
    (GetUserNameW(name.as_mut_ptr(), &mut size as *mut u32) != 0)
        .then(|| String::from_utf16_lossy(&name[..(size as usize) - 1]))
}

fn inject_user(mut req: Request<()>) -> Result<Request<()>, Status> {
    req.remote_addr()
        .ok_or_else(|| Status::unauthenticated("Could not find credentials.".to_owned()))
        .and_then(|val| {
            match val {
                // We have previously injected the ID as an Ipv6 address so we know we always get one.
                // Need to treat Ipv4 as an error case.
                SocketAddr::V6(ipv6) => unsafe {
                    (ImpersonateNamedPipeClient(u128::from(*ipv6.ip()) as HANDLE) != 0)
                        .then(|| ())
                        .and_then(|_| {
                            let res = get_user().and_then(|username| {
                                get_home_folder().map(|homefolder| (username, homefolder))
                            });

                            if RevertToSelf() == 0 {
                                panic!("Failed to undo user impersonation in named pipe. Process has to be restarted.");
                            }

                            res
                        })
                        .ok_or_else(|| {
                            Status::unauthenticated(
                                "Failed to determine named pipe user.".to_owned(),
                            )
                        })
                        .and_then(|(user_name, home_folder)| {
                            apply_user_info(user_name, &home_folder, req.metadata_mut()).map_err(
                                |_| {
                                    Status::unauthenticated(
                                        "Failed to insert user metadata in request.".to_owned(),
                                    )
                                },
                            )?;
                            Ok(req)
                        })
                },
                SocketAddr::V4(_) => Err(Status::unauthenticated(
                    "Could not find credentials.".to_owned(),
                )),
            }
        })
}

#[derive(Debug)]
struct NamedPipe(pub tokio::net::NamedPipe);

impl Connected for NamedPipe {
    fn remote_addr(&self) -> Option<SocketAddr> {
        Some(SocketAddr::V6(SocketAddrV6::new(
            Ipv6Addr::from(self.0.as_raw_handle() as u128),
            0,
            0,
            0,
        )))
    }
}

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
