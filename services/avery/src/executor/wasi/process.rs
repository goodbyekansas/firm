use std::{
    collections::HashMap,
    fs::File,
    process::{Command, Stdio},
};

use prost::Message;
use slog::{info, warn, Logger};

use super::{
    error::{WasiError, WasiResult},
    sandbox::Sandbox,
    WasmBuffer, WasmItemPtr,
};
use firm_protocols::wasi::StartProcessRequest;

pub struct StdIOConfig {
    pub stdout: Stdio,
    pub stderr: Stdio,
}

impl StdIOConfig {
    pub fn new(stdout: &File, stderr: &File) -> std::io::Result<Self> {
        Ok(StdIOConfig {
            stdout: Stdio::from(stdout.try_clone()?),
            stderr: Stdio::from(stderr.try_clone()?),
        })
    }
}

pub fn get_args_and_envs(
    request: &StartProcessRequest,
    sandboxes: &[Sandbox],
) -> (Vec<String>, HashMap<String, String>) {
    (
        request
            .args
            .iter()
            .map(|arg| {
                sandboxes
                    .iter()
                    .fold(arg.to_owned(), |s, sndbx| sndbx.map(&s))
            })
            .collect(),
        request
            .environment_variables
            .iter()
            .map(|(k, v)| {
                let value = sandboxes
                    .iter()
                    .fold(v.to_owned(), |val, sndbx| sndbx.map(&val));
                (k.clone(), value)
            })
            .collect(),
    )
}

pub fn start_process(
    logger: &Logger,
    sandboxes: &[Sandbox],
    stdio: StdIOConfig,
    request: WasmBuffer,
    pid_out: WasmItemPtr<u64>,
) -> WasiResult<()> {
    let request: StartProcessRequest =
        StartProcessRequest::decode(request.buffer()).map_err(WasiError::FailedToDecodeProtobuf)?;

    let (args, env) = get_args_and_envs(&request, sandboxes);
    info!(
        logger,
        "Launching host process {}, args: {:#?}", request.command, &args
    );

    Command::new(request.command)
        .args(args)
        .envs(&env)
        .stdout(stdio.stdout)
        .stderr(stdio.stderr)
        .spawn()
        .map_err(|e| {
            warn!(logger, "Failed to launch host process: {}", e);
            WasiError::FailedToStartProcess(e)
        })
        .and_then(|c| pid_out.set(c.id() as u64))
}

pub fn run_process(
    logger: &Logger,
    sandboxes: &[Sandbox],
    stdio: StdIOConfig,
    request: WasmBuffer,
    exit_code_out: WasmItemPtr<i32>,
) -> WasiResult<()> {
    let request: StartProcessRequest =
        StartProcessRequest::decode(request.buffer()).map_err(WasiError::FailedToDecodeProtobuf)?;

    let (args, env) = get_args_and_envs(&request, sandboxes);
    info!(
        logger,
        "Running host process {} (and waiting for exit), args: {:#?}", request.command, &args
    );

    Command::new(request.command)
        .args(args)
        .envs(&env)
        .stdout(stdio.stdout)
        .stderr(stdio.stderr)
        .status()
        .map_err(|e| {
            warn!(logger, "Failed to run host process: {}", e);
            WasiError::FailedToStartProcess(e)
        })
        .and_then(|c| exit_code_out.set(c.code().unwrap_or(-1)))
}
