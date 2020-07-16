use std::{
    collections::HashMap,
    fs::File,
    process::{Command, Stdio},
};

use prost::Message;
use slog::{info, Logger};
use wasmer_runtime::{memory::Memory, Array, Item, WasmPtr};

use super::error::{WasiError, WasiResult};
use super::sandbox::Sandbox;
use gbk_protocols::functions::StartProcessRequest;

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
    vm_memory: &Memory,
    request: WasmPtr<u8, Array>,
    len: u32,
    pid_out: WasmPtr<u64, Item>,
) -> WasiResult<()> {
    let request: StartProcessRequest = request
        .deref(vm_memory, 0, len)
        .ok_or_else(WasiError::FailedToDerefPointer)
        .and_then(|cells| {
            StartProcessRequest::decode(
                cells
                    .iter()
                    .map(|v| v.get())
                    .collect::<Vec<u8>>()
                    .as_slice(),
            )
            .map_err(WasiError::FailedToDecodeProtobuf)
        })?;

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
            println!("Failed to launch host process: {}", e);
            WasiError::FailedToStartProcess(e)
        })
        .and_then(|c| unsafe {
            pid_out
                .deref_mut(&vm_memory)
                .ok_or_else(WasiError::FailedToDerefPointer)
                .map(|cell| cell.set(c.id() as u64))
        })
}

pub fn run_process(
    logger: &Logger,
    sandboxes: &[Sandbox],
    stdio: StdIOConfig,
    vm_memory: &Memory,
    request: WasmPtr<u8, Array>,
    len: u32,
    exit_code_out: WasmPtr<i32, Item>,
) -> WasiResult<()> {
    let request: StartProcessRequest = request
        .deref(vm_memory, 0, len)
        .ok_or_else(WasiError::FailedToDerefPointer)
        .and_then(|cells| {
            StartProcessRequest::decode(
                cells
                    .iter()
                    .map(|v| v.get())
                    .collect::<Vec<u8>>()
                    .as_slice(),
            )
            .map_err(WasiError::FailedToDecodeProtobuf)
        })?;

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
            println!("Failed to run host process: {}", e);
            WasiError::FailedToStartProcess(e)
        })
        .and_then(|c| unsafe {
            exit_code_out
                .deref_mut(&vm_memory)
                .ok_or_else(WasiError::FailedToDerefPointer)
                .map(|cell| cell.set(c.code().unwrap_or(-1)))
        })
}
