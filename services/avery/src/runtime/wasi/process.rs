use std::{
    collections::HashMap,
    io::BufRead,
    io::Read,
    io::Write,
    process::Child,
    process::{Command, Stdio},
};

use prost::Message;
use slog::{info, o, warn, Logger};

use super::{
    api::{WasmBuffer, WasmItemPtr},
    error::{WasiError, WasiResult},
    output::Output,
    sandbox::Sandbox,
};
use firm_types::wasi::StartProcessRequest;

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

fn read_output<T>(mut output: Output, source: Option<T>, logger: Logger)
where
    T: Read,
{
    if let Some(src) = source {
        let mut reader = std::io::BufReader::new(src);
        let mut s = String::with_capacity(128);
        while let Ok(nb) = reader.read_line(&mut s) {
            if nb == 0 {
                break;
            }
            output.write(s.as_bytes()).map_or_else(
                |_| warn!(logger, "Failed to write \"{}\" to output.", s),
                |_| (),
            );
        }
    }
}

fn setup_readers(c: &mut Child, out: Output, err: Output, logger: &Logger) {
    let (stdout, stderr) = (c.stdout.take(), c.stderr.take());
    let stdout_logger = logger.new(o!("reader" => "stdout"));
    let stderr_logger = logger.new(o!("reader" => "stderr"));
    std::thread::spawn(|| read_output(out, stdout, stdout_logger));
    std::thread::spawn(|| read_output(err, stderr, stderr_logger));
}

pub fn start_process(
    logger: &Logger,
    sandboxes: &[Sandbox],
    stdout: &Output,
    stderr: &Output,
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

    Command::new(request.command.clone())
        .args(args)
        .envs(&env)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            warn!(logger, "Failed to launch host process: {}", e);
            WasiError::FailedToStartProcess(e)
        })
        .and_then(|mut c| {
            setup_readers(
                &mut c,
                stdout.clone(),
                stderr.clone(),
                &logger.new(o!("start_process" => request.command)),
            );
            pid_out.set(c.id() as u64)
        })
}

pub fn run_process(
    logger: &Logger,
    sandboxes: &[Sandbox],
    stdout: &Output,
    stderr: &Output,
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
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            warn!(logger, "Failed to run host process: {}", e);
            WasiError::FailedToStartProcess(e)
        })
        .and_then(|mut c| {
            setup_readers(&mut c, stdout.clone(), stderr.clone(), logger);
            c.wait().map_err(|e| {
                warn!(
                    logger,
                    "Failed to run host process (Failed to wait for it to exit): {}", e
                );
                WasiError::FailedToStartProcess(e)
            })
        })
        .and_then(|c| exit_code_out.set(c.code().unwrap_or(-1)))
}
