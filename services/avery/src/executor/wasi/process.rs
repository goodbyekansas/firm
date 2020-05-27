use std::{collections::HashMap, process::Command};

use prost::Message;
use slog::{info, Logger};
use wasmer_runtime::{memory::Memory, Array, Item, WasmPtr};

use super::error::{WasiError, WasiResult};
use super::sandbox::Sandbox;
use crate::executor::AttachmentDownload;
use gbk_protocols::functions::{FunctionAttachment, StartProcessRequest};

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

pub fn map_attachment(
    attachments: &[FunctionAttachment],
    sandbox: &Sandbox,
    vm_memory: &Memory,
    attachment_name: WasmPtr<u8, Array>,
    attachment_name_len: u32,
) -> WasiResult<()> {
    let attachment_name = attachment_name
        .get_utf8_string(vm_memory, attachment_name_len)
        .ok_or_else(|| WasiError::FailedToReadStringPointer("attachment_name".to_owned()))?;

    attachments
        .iter()
        .find(|a| a.name == attachment_name)
        .ok_or_else(|| WasiError::FailedToFindAttachment(attachment_name.to_owned()))
        .and_then(|a| {
            a.download().map_err(|e| {
                WasiError::FailedToMapAttachment(attachment_name.to_owned(), Box::new(e))
            })
        })
        .and_then(|data| {
            // TODO: Map attachment differently depending on metadata.
            // We need to support mapping folders as well.
            std::fs::write(sandbox.path().join(attachment_name), data).map_err(|e| {
                WasiError::FailedToMapAttachment(attachment_name.to_owned(), Box::new(e))
            })
        })
}

pub fn start_process(
    logger: &Logger,
    sandboxes: &[Sandbox],
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
