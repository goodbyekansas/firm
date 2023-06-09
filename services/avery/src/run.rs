use std::path::PathBuf;

use firm_protocols::{functions::execution_server::ExecutionServer, tonic::transport::Server};
use slog::{error, info, o, Drain, Logger};
use structopt::StructOpt;

use crate::{config, executor::ExecutionService, system};

#[derive(StructOpt, Debug)]
#[structopt(name = "avery")]
pub struct AveryArgs {
    #[structopt(short = "c", long = "config", parse(from_os_str), env = "AVERY_CONFIG")]
    config: Option<PathBuf>,

    #[cfg(windows)]
    #[structopt(short = "s", long = "service")]
    pub service: bool,
}

pub fn create_logger() -> Logger {
    Logger::root(
        slog_async::Async::new(
            slog_term::FullFormat::new(slog_term::TermDecorator::new().build())
                .build()
                .fuse(),
        )
        .build()
        .fuse(),
        o!(),
    )
}

pub async fn run<G>(
    args: AveryArgs,
    started_callback: G,
    log: Logger,
) -> Result<(), Box<dyn std::error::Error>>
where
    G: FnOnce() -> Result<(), String>,
{
    info!(log, "🏎️ Starting Avery {}...", env!("CARGO_PKG_VERSION"));

    let config = args.config.map_or_else(
        || config::Config::new(log.new(o! { "scope" => "load-config"})),
        |file| config::Config::new_with_file(file, log.new(o! { "scope" => "load-config"})),
    )?;

    let mut runtime_directories = config.runtime_directories.clone();
    runtime_directories.push(system::default_runtime_dir());

    let execution_service = ExecutionService::new(
        log.new(o!("service" => "execution")),
        runtime_directories,
        // TODO In the future root_directory must be configurable
        &system::user_cache_path()
            .map(|p| p.join("functions"))
            .ok_or_else(|| "Failed to get user cache path.".to_owned())
            .and_then(|p| {
                std::fs::create_dir_all(&p)
                    .map_err(|e| format!("Failed to create Avery cache directory: {}", e))
                    .map(|_| p)
            })?,
    )?;

    let (incoming, shutdown_cb) =
        system::create_listener(log.new(o!("scope" => "listener"))).await?;
    started_callback().map_err(|e| format!("Failed to signal startup done: {}", e))?;

    Server::builder()
        .add_service(ExecutionServer::new(execution_service))
        .serve_with_incoming_shutdown(
            incoming,
            system::shutdown_signal(log.new(o!("scope" => "shutdown"))),
        )
        .await
        .map_err(|e| e.to_string())?;

    if let Some(f) = shutdown_cb {
        f();
    }

    info!(log, "👋 see you soon - no one leaves the Firm");
    Ok(())
}

#[tokio::main]
pub async fn run_with_tokio(args: AveryArgs) -> Result<(), i32> {
    let log = create_logger();
    let exit_log = log.new(o!("scope" => "unhandled_error"));

    run(args, || Ok(()), log).await.map_err(|e| {
        error!(exit_log, "Unhandled error: {}. Exiting", e);
        1i32
    })
}
