use slog::{error, info, o, Drain, Logger};
use structopt::StructOpt;

use firm_types::{
    functions::{execution_server::ExecutionServer, registry_server::RegistryServer},
    tonic,
    tonic::transport::Server,
};

use avery::{
    config,
    executor::ExecutionService,
    proxy_registry::{ExternalRegistry, ProxyRegistry},
    registry::RegistryService,
    runtime,
};
use futures::{FutureExt, TryFutureExt};
use std::{net::SocketAddr, path::PathBuf};
use tokio::{
    net::UnixListener,
    signal::unix::{signal, SignalKind},
};
use url::Url;

async fn ctrl_c() {
    let _ = tokio::signal::ctrl_c().await;
}

async fn sig_term() {
    match signal(SignalKind::terminate()) {
        Ok(mut stream) => stream.recv().await,
        Err(_) => futures::future::pending::<Option<()>>().await,
    };
}

async fn shutdown_signal(log: Logger) {
    futures::select! {
        () = ctrl_c().fuse() => { info!(log, "Recieved Ctrl-C"); },
        () = sig_term().fuse() => { info!(log, "Recieved SIGTERM"); }
    }
}

#[derive(StructOpt, Debug)]
#[structopt(name = "avery")]
struct AveryArgs {
    #[structopt(short = "c", long = "config", parse(from_os_str), env = "AVERY_CONFIG")]
    config: Option<PathBuf>,
}

async fn create_front_door(
    execution_service: ExecutionService,
    proxy_registry: ProxyRegistry,
    addr: SocketAddr,
    log: Logger,
) -> Result<(), String> {
    Server::builder()
        .add_service(ExecutionServer::new(execution_service))
        .add_service(RegistryServer::new(proxy_registry))
        .serve_with_shutdown(addr, shutdown_signal(log.new(o!("scope" => "shutdown"))))
        .await
        .map_err(|e| e.to_string())
}

async fn create_trap_door(
    local_socket_path: &std::path::Path,
    execution_service: ExecutionService,
    proxy_registry: ProxyRegistry,
    log: Logger,
) -> Result<(), String> {
    let incoming = {
        let uds = UnixListener::bind(local_socket_path).map_err(|e| e.to_string())?;

        async_stream::stream! {
            while let item = uds.accept().map_ok(|(st, _)| unix::UnixStream(st)).await {
                yield item;
            }
        }
    };

    let server = Server::builder()
        .add_service(ExecutionServer::new(execution_service))
        .add_service(RegistryServer::new(proxy_registry))
        .serve_with_incoming_shutdown(
            incoming,
            shutdown_signal(log.new(o!("scope" => "shutdown"))),
        )
        .await
        .map_err(|e| e.to_string());
    std::fs::remove_file(local_socket_path).unwrap_or(());
    server
}

#[cfg(unix)]
mod unix {
    use std::{
        pin::Pin,
        task::{Context, Poll},
    };

    use super::tonic::transport::server::Connected;
    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

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

        fn poll_shutdown(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.0).poll_shutdown(cx)
        }
    }
}
async fn run(log: Logger) -> Result<(), Box<dyn std::error::Error>> {
    let args = AveryArgs::from_args();

    let config = args
        .config
        .map_or_else(config::Config::new, config::Config::new_with_file)?;

    let external_registries = config
        .registries
        .into_iter()
        .map(|reg| {
            Url::parse(&reg.url)
                .map_err(|e| {
                    format!(
                        "Failed to parse url for external registry \"{}\": {}",
                        reg.name, e
                    )
                })
                .and_then(|url| {
                    let reg_name = reg.name.clone();
                    reg.oauth_scope
                        .map(|scope| {
                            std::env::var(format!("AVERY_OAUTH_{}", scope)).map_err(|e| {
                                format!(
                                    "Could not use environment variable \"{}\": {}",
                                    format!("AVERY_OAUTH_{}", scope),
                                    e
                                )
                            })
                        })
                        .transpose()
                        .map(|oauth| {
                            oauth.map_or_else(
                                || ExternalRegistry::new(reg_name.clone(), url.clone()),
                                |oauth| {
                                    ExternalRegistry::new_with_oauth(
                                        reg_name.clone(),
                                        url.clone(),
                                        oauth,
                                    )
                                },
                            )
                        })
                })
        })
        .collect::<Result<Vec<ExternalRegistry>, String>>()?;

    let internal_registry = RegistryService::new(
        config.internal_registry,
        log.new(o!("service" => "internal-registry")),
    );

    let proxy_registry = ProxyRegistry::new(
        external_registries,
        internal_registry,
        config.conflict_resolution,
        log.new(o!("service" => "proxy-registry")),
    )
    .await?;

    let mut runtime_directories = config.runtime_directories.clone();
    #[cfg(windows)]
    runtime_directories.push(PathBuf::from("%PROGRAMDATA%/Avery/runtimes"));
    #[cfg(unix)]
    runtime_directories.push(PathBuf::from("/usr/share/avery/runtimes"));
    let directory_sources = runtime_directories
        .into_iter()
        .filter_map(|d| {
            if d.exists() {
                Some(
                    runtime::filesystem_source::FileSystemSource::new(
                        &d,
                        log.new(o!("source" => "fs")),
                    )
                    .map(|fss| Box::new(fss) as Box<dyn runtime::RuntimeSource>),
                )
            } else {
                None
            }
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut runtime_sources: Vec<Box<dyn runtime::RuntimeSource>> = vec![Box::new(
        runtime::InternalRuntimeSource::new(log.new(o!("source" => "internal"))),
    )];
    runtime_sources.extend(directory_sources.into_iter());
    let execution_service = ExecutionService::new(
        log.new(o!("service" => "execution")),
        Box::new(proxy_registry.clone()),
        runtime_sources,
    );

    let front_door = if config.enable_external_port {
        let port = config.port;
        let addr = format!("[::]:{}", port).parse()?;

        info!(
            log,
            "👨‍⚖️ The Firm is listening for external requests on port {}", port
        );

        create_front_door(
            execution_service.clone(),
            proxy_registry.clone(),
            addr,
            log.new(o!("🚪" => "front")),
        )
        .boxed()
    } else {
        futures::future::ready(Ok(())).boxed()
    };

    // 🚪
    info!(
        log,
        "👨‍⚖️ The Firm is listening for internal requests on {}",
        &config.internal_port_socket_path.display()
    );
    futures::try_join!(
        create_trap_door(
            &config.internal_port_socket_path,
            execution_service,
            proxy_registry,
            log.new(o!("🚪" => "trap")),
        ),
        front_door
    )?;

    info!(log, "👋 see you soon - no one leaves the Firm");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), i32> {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    let log = Logger::root(drain, o!());

    let exit_log = log.new(o!("scope" => "unhandled_error"));
    run(log).await.map_err(|e| {
        error!(exit_log, "Unhandled error: {}. Exiting", e);
        1i32
    })
}
