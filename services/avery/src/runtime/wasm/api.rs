use std::{
    collections::HashMap,
    io::{BufRead, Read, Write},
    path::Path,
    process::{Child, Stdio},
};

use slog::{o, warn, Logger};
use wasmtime::Caller;

use super::{output::Output, FirmApi, WasmAllocator, WasmPtr, WasmResult, WasmState, WasmString};

pub fn map_attachment<Allocator: WasmAllocator>(
    _caller: Caller<WasmState<Allocator>>,
    _attachment_name: i32,
    _unpack: i32,
    _path_out: i32,
) -> i32 {
    0
}

impl<Allocator: WasmAllocator> FirmApi for WasmState<Allocator> {
    fn host_path_exists(&mut self, path: &str) -> std::result::Result<bool, String> {
        Ok(Path::new(&path).exists())
    }

    fn host_os(&mut self) -> std::result::Result<String, String> {
        Ok(std::env::consts::OS.to_owned())
    }

    fn start_host_process(
        &mut self,
        _cmd: &str,
    ) -> Result<super::types::StartHostProcessValue, String> {
        todo!()
    }
}

/*pub fn host_path_exists<Allocator: WasmAllocator>(
    mut caller: Caller<WasmState<Allocator>>,
    path: i32,
    exists: i32,
) -> i32 {
    WasmString::from(WasmPtr::new(&mut caller, path as u32))
        .to_str()
        .map_err(Into::into)
        .and_then(|path| {
            let state = caller.data_mut();
            state.host_path_exists(path).map_err(WasmError::UsageError)
        })
        .and_then(|inner_res| {
            WasmPtr::new(&caller, exists as u32).write(&mut caller, &[inner_res as u8])
        })
        .to_string_ptr(&mut caller)
}*/

/*pub fn host_os<Allocator: WasmAllocator>(
    mut caller: Caller<WasmState<Allocator>>,
    host_os_out: i32,
) -> i32 {
    WasmString::try_from_str(&mut caller, std::env::consts::OS)
        .and_then(|os| WasmPtr::new(&caller, host_os_out as u32).set_ptr(&mut caller, &*os))
        .to_string_ptr(&mut caller)
}*/

#[repr(C)]
struct StartProcessRequest {
    command: i32,
    env_vars: i32,
    num_env_vars: u32,
    wait: bool,
}

#[repr(C)]
struct EnvironmentVariable {
    pub key: i32,
    pub value: i32,
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
            s.clear();
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

pub fn start_host_process<Allocator: WasmAllocator>(
    mut caller: Caller<WasmState<Allocator>>,
    request: i32,
    pid_out: i32,
    exit_code_out: i32,
) -> i32 {
    let request: &StartProcessRequest = unsafe {
        &*(WasmPtr::new(&mut caller, request as u32).host_ptr() as *const StartProcessRequest)
    };

    (0..request.num_env_vars)
        .map(|i| {
            let offset = std::mem::size_of::<EnvironmentVariable>() as u32 * i;
            let elem = (unsafe {
                &*(WasmPtr::new(&mut caller, request.env_vars as u32 + offset).host_ptr()
                    as *const EnvironmentVariable)
            }) as &EnvironmentVariable;

            WasmString::from(WasmPtr::new(&mut caller, elem.key as u32))
                .to_str()
                .map_err(Into::into)
                .map(String::from)
                .and_then(|key| {
                    WasmString::from(WasmPtr::new(&mut caller, elem.value as u32))
                        .to_str()
                        .map_err(Into::into)
                        .map(|value_str| (key, String::from(value_str)))
                })
        })
        .collect::<Result<HashMap<String, String>, super::WasmError>>()
        .and_then(|env_vars| {
            WasmString::from(WasmPtr::new(&mut caller, request.command as u32))
                .to_str()
                .map_err(Into::into)
                .map(|cmd| (env_vars, String::from(cmd)))
        })
        .and_then(|(env_vars, cmd)| {
            let mut args = cmd.split(' ');
            args.next()
                .ok_or_else(|| super::WasmError::UsageError(String::from("Empty command")))
                .map(std::process::Command::new)
                .map(|mut cmd_builder| {
                    cmd_builder.args(args);
                    (env_vars, cmd_builder)
                })
        })
        .map(|(env_vars, mut cmd)| {
            cmd.envs(env_vars);
            cmd
        })
        .and_then(|mut cmd| {
            cmd.stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| super::WasmError::ProcessError(e.to_string()))
        })
        .and_then(|mut child| {
            setup_readers(
                &mut child,
                caller.data().function_context.stdout.clone(),
                caller.data().function_context.stderr.clone(),
                &caller.data().logger.new(o!("start_process" => "TODO")),
            );
            let mut pid_out = WasmPtr::new(&mut caller, pid_out as u32);

            pid_out
                .write(&mut caller, &(child.id() as u64).to_ne_bytes())
                .map(|_| child)
        })
        .and_then(|mut child| {
            let mut exit_code_out = WasmPtr::new(&mut caller, exit_code_out as u32);
            if request.wait {
                child
                    .wait()
                    .map_err(|e| super::WasmError::ProcessError(e.to_string()))
                    .and_then(|c| {
                        exit_code_out
                            .write(&mut caller, &(c.code().unwrap_or(-1) as i64).to_ne_bytes())
                    })
            } else {
                Ok(())
            }
        })
        .map(|_| ())
        .to_string_ptr(&mut caller)
}

pub fn set_error<Allocator: WasmAllocator>(
    mut caller: Caller<WasmState<Allocator>>,
    error_message_ptr: i32,
) -> i32 {
    WasmString::from(WasmPtr::new(&mut caller, error_message_ptr as u32))
        .to_str()
        .map_err(Into::into)
        .map(|error_message| {
            caller.data_mut().function_context.error = Some(String::from(error_message));
        })
        .to_string_ptr(&mut caller)
}

#[allow(dead_code)]
#[cfg(test)]
mod tests {
    use crate::runtime::wasm::{FunctionContext, WasmString};

    use super::{
        super::{output::Output, WasmError},
        WasmAllocator, WasmPtr, WasmState,
    };

    use std::sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    };

    use wasmtime::{AsContextMut, Engine, Func, Memory, MemoryType, Store, Val};

    macro_rules! null_logger {
        () => {{
            slog::Logger::root(slog::Discard, slog::o!())
        }};
    }

    /// Super simple bump allocator for the tests
    #[derive(Clone)]
    struct SimpleAllocator {
        offset: Arc<AtomicU32>,
        capacity_pages: u32,
    }

    impl SimpleAllocator {
        fn new(capacity_pages: u32) -> Self {
            Self {
                offset: Arc::new(AtomicU32::new(0)),
                capacity_pages,
            }
        }
    }

    impl WasmAllocator for SimpleAllocator {
        /// Allocate `amount` bytes of memory
        ///
        /// Returns the offset for the memory to use, starting at 0 for the first
        /// allocation

        fn allocate<Allocator: WasmAllocator>(
            &self,
            store: impl AsContextMut<Data = WasmState<Allocator>>,
            amount: u32,
        ) -> Result<WasmPtr, WasmError> {
            let offset = self.offset.fetch_add(amount, Ordering::SeqCst);
            if offset >= self.capacity_pages * 1024 * 64 {
                Err(WasmError::AllocationFailure(String::from(
                    "Simple allocator: out of memory",
                )))
            } else {
                Ok(WasmPtr::new(store, offset))
            }
        }
    }

    /// Helper struct to manage a simple WASI runtime setup for tests
    struct WasmTestContext {
        allocator: SimpleAllocator,
        store: Store<WasmState<SimpleAllocator>>,
    }

    impl WasmTestContext {
        /// Create a new WASI test setup
        ///
        /// `num_pages` controls how many pages of WASI memory is allocated for the "guest"
        fn new(num_pages: u32) -> Self {
            let engine = Engine::default();
            let allocator = SimpleAllocator::new(num_pages);
            let stdmjölk = Output::new(vec![]);
            let function_context = FunctionContext::new(stdmjölk.clone(), stdmjölk, vec![]);

            let mut store = Store::new(
                &engine,
                WasmState::new_wasm(allocator.clone(), null_logger!(), function_context),
            );

            let mem = Memory::new(&mut store, MemoryType::new(num_pages, None))
                .expect("failed to create memory");
            store.data_mut().memory = Some(mem);
            Self { allocator, store }
        }
    }

    fn get_error_message<Allocator: WasmAllocator>(
        mut store: impl AsContextMut<Data = WasmState<Allocator>>,
        error_ptr: i32,
    ) -> String {
        if error_ptr as u32 == 0xFFFFFFFF {
            return String::new();
        }

        let error_msg_ptr = WasmString::from(WasmPtr::new(&mut store, error_ptr as u32));
        String::from(error_msg_ptr.to_str().unwrap())
    }

    fn get_invalid_utf8_string<Allocator: WasmAllocator>(
        mut store: impl AsContextMut<Data = WasmState<Allocator>>,
    ) -> Result<WasmString, WasmError> {
        // first u8 determines the widht of the char.
        // Above 244 counts as invalid utf-8 width.

        let mut ctx = store.as_context_mut();
        let allocator = ctx.data().allocator.clone();
        let invalid_string_data: [u8; 4] = [255, 15, 15, 56];
        let string_len = invalid_string_data.len() as u32 / 4 + 1;

        allocator
            .allocate(&mut ctx, string_len)
            .map(|wasm_ptr| unsafe {
                let mem = wasm_ptr.host_ptr() as *mut u8;
                mem.copy_from_nonoverlapping(
                    invalid_string_data.as_ptr() as *const u8,
                    invalid_string_data.len(),
                );
                mem.add(string_len as usize).write(b'\0');
                WasmString(wasm_ptr)
            })
    }

    /*#[test]
    fn test_host_path_exists() {
        let mut ctx = WasmTestContext::new(5);
        let mut results: [Val; 1] = [Val::null(); 1];
        // Dir structure
        // /temp_dir/
        // /temp_dir/sune.txt
        let tmp_dir =
            tempfile::tempdir().expect("Expected to be able to create temporary directory.");
        std::fs::File::create(tmp_dir.path().join("sune.txt")).expect("Create sune.txt");

        let path = WasmString::try_from_str(
            &mut ctx.store,
            &tmp_dir.path().join("rune.txt").to_string_lossy(),
        )
        .expect("Expected to be able to create a WasmString for a path.");

        let bool_ptr = ctx
            .allocator
            .allocate(&mut ctx.store, std::mem::size_of::<u8>() as u32)
            .expect("Failed to allocate memory for out pointer");

        // Test a path that does not exist
        let call_res = Func::wrap(&mut ctx.store, super::host_path_exists).call(
            &mut ctx.store,
            &[
                Val::I32((*path).guest_offset() as i32),
                Val::I32(bool_ptr.guest_offset() as i32),
            ],
            &mut results,
        );

        assert!(
            call_res.is_ok(),
            "Expected to be able to call host function"
        );

        assert_eq!(
            results[0].unwrap_i32(),
            0,
            "Expected host function to return nullptr (no error message) on success"
        );

        let file_exists = bool_ptr.get_ptr(&mut ctx.store).unwrap().host_ptr() as u8 != 0;
        assert!(
            !file_exists,
            "Expected the path \"/tmp_dir/rune.txt\" to not exist."
        );

        // Test a path that does exist
        let path = WasmString::try_from_str(
            &mut ctx.store,
            &tmp_dir.path().join("sune.txt").to_string_lossy(),
        )
        .expect("Expected to be able to create a WasmString for a path.");

        Func::wrap(&mut ctx.store, super::host_path_exists)
            .call(
                &mut ctx.store,
                &[
                    Val::I32((*path).guest_offset() as i32),
                    Val::I32(bool_ptr.guest_offset() as i32),
                ],
                &mut results,
            )
            .expect("Tried to call host_path_exists with existing path.");

        let file_exists = bool_ptr.get_ptr(&mut ctx.store).unwrap().host_ptr() as u8 != 0;
        assert!(
            file_exists,
            "Expected the path \"/tmp_dir/sune.txt\" to exist."
        );

        // Test on directory
        let path = WasmString::try_from_str(&mut ctx.store, &tmp_dir.path().to_string_lossy())
            .expect("Expected to be able to create a WasmString for a path.");

        Func::wrap(&mut ctx.store, super::host_path_exists)
            .call(
                &mut ctx.store,
                &[
                    Val::I32((*path).guest_offset() as i32),
                    Val::I32(bool_ptr.guest_offset() as i32),
                ],
                &mut results,
            )
            .expect("Tried to call host_path_exists with existing path.");

        let file_exists = bool_ptr.get_ptr(&mut ctx.store).unwrap().host_ptr() as u8 != 0;
        assert!(file_exists, "Expected the path \"/tmp_dir\" to exist.");

        // Test with completely invalid data in path
        let invalid_string = get_invalid_utf8_string(&mut ctx.store).unwrap();

        let call_res = Func::wrap(&mut ctx.store, super::host_path_exists).call(
            &mut ctx.store,
            &[
                Val::I32((*invalid_string).guest_offset() as i32),
                Val::I32(bool_ptr.guest_offset() as i32),
            ],
            &mut results,
        );
        assert!(call_res.is_ok(), "Expected host_path_exists to not error");
        let raw_error_msg = results[0].unwrap_i32();
        let error_msg = get_error_message(&mut ctx.store, raw_error_msg);
        assert_ne!(
            raw_error_msg, 0,
            "Expected host function to not return nullptr (error message) on failure"
        );

        assert!(error_msg.contains("Utf-8 error:"))
    }*/

    /*#[test]
    fn test_host_os() {
        let mut ctx = WasmTestContext::new(5);
        let mut results: [Val; 1] = [Val::null(); 1];
        let string_ptr = ctx
            .allocator
            .allocate(&mut ctx.store, std::mem::size_of::<i32>() as u32)
            .expect("Failed to allocate memory for out pointer");
        let call_res = Func::wrap(&mut ctx.store, super::host_os).call(
            &mut ctx.store,
            &[Val::I32(string_ptr.guest_offset() as i32)],
            &mut results,
        );

        assert!(
            call_res.is_ok(),
            "Expected to be able to call host function"
        );
        assert_eq!(
            results[0].unwrap_i32(),
            0,
            "Expected host function to return nullptr (no error message) on success"
        );

        assert_eq!(
            WasmString::from(
                string_ptr
                    .get_ptr(&mut ctx.store)
                    .expect("Failed to read resulting string ptr")
            )
            .to_str(),
            Ok(std::env::consts::OS),
            "Expected returned string to represent the host OS"
        );

        // Test error case, no memory
        let mut ctx = WasmTestContext::new(1);
        let string_ptr = ctx
            .allocator
            .allocate(&mut ctx.store, std::mem::size_of::<i32>() as u32)
            .expect("Failed to allocate memory for out pointer");

        ctx.allocator
            .allocate(
                &mut ctx.store,
                (1024 * 64 - std::mem::size_of::<i32>()) as u32,
            )
            .expect("Expected to be able to eat almost an entire page");

        let call_res = Func::wrap(&mut ctx.store, super::host_os).call(
            &mut ctx.store,
            &[Val::I32(string_ptr.guest_offset() as i32)],
            &mut results,
        );

        assert!(
            call_res.is_ok(),
            "Expected to be able to call host function"
        );
        let raw_error_ptr = results[0].unwrap_i32();
        let error_msg = get_error_message(&mut ctx.store, raw_error_ptr);

        assert_ne!(
            raw_error_ptr, 0,
            "Expected host function to return an error message"
        );

        // We ran out of memory and thus get an empty string
        assert_eq!(error_msg, "");
    }*/

    #[test]
    fn test_set_error() {
        let mut ctx = WasmTestContext::new(5);
        let mut results: [Val; 1] = [Val::null(); 1];

        let error_msg = WasmString::try_from_str(&mut ctx.store, "I errored very hard.")
            .expect("Expected to be able to create a WasmString for an error message.");

        // Test setting the error
        let call_res = Func::wrap(&mut ctx.store, super::set_error).call(
            &mut ctx.store,
            &[Val::I32((*error_msg).guest_offset() as i32)],
            &mut results,
        );

        assert!(
            call_res.is_ok(),
            "Expected to be able to call host function"
        );

        assert_eq!(
            results[0].unwrap_i32(),
            0,
            "Expected host function to return nullptr (no error message) on success"
        );

        // Setting error causes error.
        let invalid_string = get_invalid_utf8_string(&mut ctx.store).unwrap();
        let call_res = Func::wrap(&mut ctx.store, super::set_error).call(
            &mut ctx.store,
            &[Val::I32((*invalid_string).guest_offset() as i32)],
            &mut results,
        );

        assert!(call_res.is_ok(), "Expected set_error to not error");

        let raw_error_ptr = results[0].unwrap_i32();
        let error_msg = get_error_message(&mut ctx.store, raw_error_ptr);

        assert_ne!(
            raw_error_ptr, 0,
            "Expected host function to not return nullptr (error message) on failure"
        );

        assert!(error_msg.contains("Utf-8 error:"))
    }
}
