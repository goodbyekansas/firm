use std::{collections::HashMap, process::Command};

use prost::Message;
use slog::{info, Logger};
use wasmer_runtime::{memory::Memory, Array, Item, WasmPtr};

use super::error::{WasiError, WasiResult};
use super::sandbox::Sandbox;
use gbk_protocols::functions::StartProcessRequest;

pub fn get_args_and_envs(
    request: &StartProcessRequest,
    sandbox: &Sandbox,
) -> (Vec<String>, HashMap<String, String>) {
    (
        request.args.iter().map(|arg| sandbox.map(arg)).collect(),
        request
            .environment_variables
            .iter()
            .map(|(k, v)| (k.clone(), sandbox.map(v)))
            .collect(),
    )
}

pub fn start_process(
    logger: &Logger,
    sandbox: &Sandbox,
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

    let (args, env) = get_args_and_envs(&request, sandbox);
    info!(
        logger,
        "Launching host process {}, args: {:#?}", request.command, &args
    );

    Command::new(request.command)
        .args(args)
        .envs(&env)
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
    sandbox: &Sandbox,
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

    let (args, env) = get_args_and_envs(&request, sandbox);
    info!(
        logger,
        "Running host process {} (and waiting for exit), args: {:#?}", request.command, &args
    );

    Command::new(request.command)
        .args(args)
        .envs(&env)
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
