use std::{ffi::OsString, path::PathBuf, time::Duration};

use futures::{FutureExt, TryFutureExt};
use log::Log;
use slog::{error, info, o, Drain, Logger};
use structopt::StructOpt;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::windows::named_pipe::{ClientOptions, NamedPipeClient},
};
use triggered::{Listener, Trigger};
use winapi::shared::winerror;
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

use crate::run::{self, LomaxArgs};

pub const DEFAULT_SOCKET_URL: &str = r#"windows://./pipe/avery-{username}"#;

pub fn get_lomax_cfg_dir() -> Option<PathBuf> {
    std::env::var_os("PROGRAMDATA").map(|appdata| PathBuf::from(&appdata).join("Firm"))
}

pub fn drop_privileges(_: &str, _: &str) -> Result<(), String> {
    Ok(())
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

pub struct NamedPipe(pub NamedPipeClient);

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

impl hyper::service::Service<http::Uri> for crate::run::LocalAveryConnector {
    type Response = NamedPipe;
    type Error = std::io::Error;
    type Future = crate::run::LocalConnectorFuture<Self>;

    fn poll_ready(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, _req: http::Uri) -> Self::Future {
        // TODO: It would be nicer to do this work in poll_ready.
        // It would reflect the correct behaviour that way.
        // Right now we skip it because the code becomes a bit
        // more tricky. You could for example add a private
        // member in LocalAveryConnector that saves the client
        // in poll_ready and later returns it in this function.
        let uri = format!(
            r#"\\{}{}"#,
            self.uri.host().unwrap_or("."),
            self.uri.path().replace("/", "\\")
        );

        Box::pin(async move {
            let client = loop {
                match ClientOptions::new().open(&uri) {
                    Ok(client) => break client,
                    Err(e) if e.raw_os_error() == Some(winerror::ERROR_PIPE_BUSY as i32) => (),
                    Err(e) => return Err(e),
                }

                tokio::time::sleep(Duration::from_millis(50)).await;
            };

            Ok(NamedPipe(client))
        })
    }
}

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

lazy_static::lazy_static! {
    static ref WINDOWS_SERVICE_STOP_EVENT: WindowsServiceStopEvent = WindowsServiceStopEvent::new();
}

fn service_main(_: Vec<OsString>) {
    // We don't own the signature to service main so we have to parse the arguments
    // again, we need args both before and after this point so there's no good way
    // around it.
    let args = run::LomaxArgs::from_args();
    let log = Logger::root(
        slog_async::Async::new(
            WinLoggerDrain {
                inner: WinLogger::try_new("Lomax")
                    .expect("Failed to create windows event logger for Lomax"),
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

    service_control_handler::register("lomax", event_handler)
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

pub fn bootstrap(args: LomaxArgs) -> Result<(), i32> {
    use std::error::Error;

    match args.service {
        true => {
            let exit_log = run::create_logger().new(o!("scope" => "unhandled_error"));
            service_dispatcher::start("lomax", ffi_service_main).map_err(|e| {
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
