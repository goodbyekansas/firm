use std::{
    ffi::OsString,
    path::PathBuf,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use crate::run::{self, AveryArgs};
use firm_types::tonic::transport::server::Connected;
use futures::{FutureExt, TryFutureExt};
use lazy_static::lazy_static;
use slog::{error, info, o, Drain, Logger};
use structopt::StructOpt;
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::windows::named_pipe::{NamedPipeServer, ServerOptions},
};

use triggered::{Listener, Trigger};
use winapi::{
    shared::{minwindef::FALSE, ntdef::NULL},
    um::{
        errhandlingapi::GetLastError,
        minwinbase::{LPTR, SECURITY_ATTRIBUTES},
        securitybaseapi::InitializeSecurityDescriptor,
        winbase::{GetUserNameW, LocalAlloc, LocalFree},
        winnt::{
            PSECURITY_DESCRIPTOR, SECURITY_DESCRIPTOR_MIN_LENGTH, SECURITY_DESCRIPTOR_REVISION,
        },
    },
};

use log::Log;
use windows_service::{
    define_windows_service,
    service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult, ServiceStatusHandle},
    service_dispatcher,
};
use winlog::WinLogger;

define_windows_service!(ffi_service_main, service_main);

struct WindowsServiceStopEvent {
    trigger: Trigger,
    listener: Listener,
}

impl WindowsServiceStopEvent {
    pub fn new() -> Self {
        let (trigger, listener) = triggered::trigger();
        Self { trigger, listener }
    }

    pub fn trigger(&self) {
        self.trigger.trigger();
    }

    pub fn get_stop_listener(&self) -> Listener {
        self.listener.clone()
    }
}

struct WinLoggerDrain {
    inner: WinLogger,
}

impl Drain for WinLoggerDrain {
    type Ok = ();
    type Err = Box<dyn std::error::Error>;

    fn log(
        &self,
        record: &slog::Record,
        _values: &slog::OwnedKVList,
    ) -> Result<Self::Ok, Self::Err> {
        self.inner.log(
            &log::RecordBuilder::new()
                .args(*record.msg())
                .file_static(Some(record.file()))
                .module_path_static(Some(record.module()))
                .line(Some(record.line()))
                .level(match record.level() {
                    slog::Level::Critical => log::Level::Error,
                    slog::Level::Error => log::Level::Error,
                    slog::Level::Warning => log::Level::Warn,
                    slog::Level::Info => log::Level::Info,
                    slog::Level::Debug => log::Level::Debug,
                    slog::Level::Trace => log::Level::Trace,
                })
                .build(),
        );

        Ok(())
    }
}

lazy_static! {
    static ref WINDOWS_SERVICE_STOP_EVENT: WindowsServiceStopEvent = WindowsServiceStopEvent::new();
}

fn service_main(_: Vec<OsString>) {
    // We don't own the signature to service main so we have to parse the arguments
    // again, we need args both before and after this point so there's no good way
    // around it.
    let args = run::AveryArgs::from_args();
    let log = Logger::root(
        slog_async::Async::new(
            WinLoggerDrain {
                inner: WinLogger::try_new("Avery")
                    .expect("Failed to create windows event logger for Avery"),
            }
            .ignore_res(),
        )
        .build()
        .fuse(),
        o!(),
    );

    let exit_log = log.new(o!("scope" => "unhandled_error"));

    let started_callback = |status_handle: ServiceStatusHandle| {
        status_handle
            .set_service_status(ServiceStatus {
                service_type: ServiceType::USER_OWN_PROCESS,
                current_state: ServiceState::Running,
                controls_accepted: ServiceControlAccept::STOP,
                exit_code: ServiceExitCode::Win32(0),
                checkpoint: 0,
                wait_hint: Duration::default(),
                process_id: None,
            })
            .map_err(|e| format!("Failed to set service running status: {}", e))
    };

    let event_handler = move |control_event| -> ServiceControlHandlerResult {
        match control_event {
            ServiceControl::Stop => {
                WINDOWS_SERVICE_STOP_EVENT.trigger();
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    service_control_handler::register("avery", event_handler)
        .map_err(|e| {
            error!(exit_log, "Could not register event handler: {}", e);
        })
        .and_then(|status_handle| {
            tokio::runtime::Runtime::new()
                .map_err(|e| {
                    error!(exit_log, "Could not start Tokio runtime: {}", e);
                })
                .map(|rt| (rt, status_handle))
        })
        .map(|(rt, status_handle)| {
            // We always want to return the handle to signal stop so after the log we
            // not interested in the actual result any more
            let _ = rt
                .block_on(run::run(args, || started_callback(status_handle), log))
                .map_err(|e| {
                    error!(exit_log, "Unhandled error: {}. Exiting", e);
                });
            status_handle
        })
        .and_then(|status_handle| {
            status_handle
                .set_service_status(ServiceStatus {
                    service_type: ServiceType::USER_OWN_PROCESS,
                    current_state: ServiceState::Stopped,
                    controls_accepted: ServiceControlAccept::STOP,
                    exit_code: ServiceExitCode::Win32(0),
                    checkpoint: 0,
                    wait_hint: Duration::default(),
                    process_id: None,
                })
                .map_err(|e| error!(exit_log, "Failed to set service stopped status: {}", e))
        })
        .ok();
}

pub async fn shutdown_signal(log: Logger) {
    futures::select! {
        () = tokio::signal::ctrl_c().map_ok_or_else(|_| (), |_| ()).fuse() => { info!(log, "Received Ctrl-C"); },
        () = WINDOWS_SERVICE_STOP_EVENT.get_stop_listener().fuse() => {
            info!(log, "Received STOP from service control manager")
        }
    }
}

pub fn bootstrap(args: AveryArgs) -> Result<(), i32> {
    use std::error::Error;

    match args.service {
        true => {
            let exit_log = run::create_logger().new(o!("scope" => "unhandled_error"));
            service_dispatcher::start("avery", ffi_service_main).map_err(|e| {
                error!(
                    exit_log,
                    "Failed to dispatch service: {}: {}",
                    e,
                    e.source() // The error does not say much without the source so lets try to get it
                        .map(|se| se.to_string())
                        .unwrap_or_else(|| String::from("Unknown error source"))
                );
                1i32
            })
        }
        false => run::run_with_tokio(args),
    }
}

pub fn default_runtime_dir() -> PathBuf {
    PathBuf::from(
        std::env::var_os("PROGRAMDATA")
            .unwrap_or_else(|| std::ffi::OsString::from(r#"C:\ProgramData"#)),
    )
    .join("Firm")
    .join("avery")
    .join("runtimes")
}

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

pub fn global_config_path() -> Option<PathBuf> {
    std::env::var_os("PROGRAMDATA").map(|appdata| PathBuf::from(&appdata).join("Firm"))
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

struct Security {
    descriptor: PSECURITY_DESCRIPTOR,
    attributes: SECURITY_ATTRIBUTES,
    log: Logger,
}

impl Security {
    pub fn try_new(log: Logger) -> Result<Self, String> {
        match unsafe { LocalAlloc(LPTR, SECURITY_DESCRIPTOR_MIN_LENGTH) } {
            v if v == NULL => Err("Failed to allocate security descriptor".to_string()),
            descriptor => {
                (unsafe { InitializeSecurityDescriptor(descriptor, SECURITY_DESCRIPTOR_REVISION) }
                    != 0)
                    .then(|| Self {
                        descriptor,
                        attributes: SECURITY_ATTRIBUTES {
                            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                            lpSecurityDescriptor: descriptor,
                            bInheritHandle: FALSE,
                        },
                        log,
                    })
                    .ok_or_else(|| "Failed to initialize security descriptor".to_string())
            }
        }
    }
}

impl Drop for Security {
    fn drop(&mut self) {
        unsafe {
            if LocalFree(self.descriptor) != NULL {
                error!(
                    self.log,
                    "Failed to free security descriptor memory, Error Code: {}",
                    GetLastError()
                );
            }
        }
    }
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
            let mut security = Security::try_new(log.new(o!("listener" => "security-descriptor")))?;
            let mut server = unsafe {
                ServerOptions::new()
                    .first_pipe_instance(true)
                    .create_with_security_attributes_raw(
                        &pipe_path,
                        &mut security.attributes as winapi::um::minwinbase::PSECURITY_ATTRIBUTES
                            as *mut std::ffi::c_void,
                    )
                    .map_err(|e| format!("Failed to create named pipe: {}", e))
            }?;

            async_stream::stream! {
                while server.connect().await.is_ok() {
                    // Making sure server is always open for connections.
                    // https://docs.rs/tokio/1.12.0/src/tokio/net/windows/named_pipe.rs.html#79
                    let old_server = server;
                    server = unsafe { ServerOptions::new()
                        .create_with_security_attributes_raw(
                            &pipe_path,
                            &mut security.attributes as winapi::um::minwinbase::PSECURITY_ATTRIBUTES
                                as *mut std::ffi::c_void,
                        )}?;
                    yield Ok(NamedPipe(old_server));
                }
            }
        },
        None,
    ))
}

#[derive(Debug)]
pub struct NamedPipe(pub NamedPipeServer);

impl Connected for NamedPipe {
    type ConnectInfo = ();

    fn connect_info(&self) -> Self::ConnectInfo {}
}

impl AsyncRead for NamedPipe {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
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
