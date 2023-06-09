#![cfg(unix)]

use std::{
    collections::HashMap,
    ffi::OsString,
    fmt::Display,
    fs::OpenOptions,
    io::{Read, Write},
    os::unix::prelude::RawFd,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    str::FromStr,
};

use firm_protocols::functions::Function;
use function::{
    attachments::{AttachmentExt, AttachmentReader},
    io::{PollRead, PollWrite},
    stream::{ChannelReader, ChannelWriter, Stream},
};
use serde::Serialize;

mod error;
mod io_event_queue;

use error::RuntimeError;
use io_event_queue::{IoEventQueue, IoId, IoReader, IoWriter};
use thiserror::Error;

#[derive(Debug, Clone, Copy)]
pub struct ExecutionId(uuid::Uuid);

impl Display for ExecutionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl ExecutionId {
    fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl FromStr for ExecutionId {
    type Err = RuntimeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(uuid::Uuid::parse_str(s).map_err(|e| {
            RuntimeError::FailedToParseFromStringError {
                what: "execution id",
                content: String::from(s),
                error: e.to_string(),
            }
        })?))
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub struct FunctionId {
    name: String,
    version: String,
}

impl From<&Function> for FunctionId {
    fn from(f: &Function) -> Self {
        Self {
            name: f.name.to_owned(),
            version: f.version.to_owned(),
        }
    }
}

impl FromStr for FunctionId {
    type Err = RuntimeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split('-');
        Ok(Self {
            name: parts.next().map(|s| s.to_owned()).ok_or_else(|| {
                RuntimeError::FailedToParseFromString {
                    what: "name for function id",
                    content: String::from(s),
                }
            })?,
            version: parts.next().map(|s| s.to_owned()).ok_or_else(|| {
                RuntimeError::FailedToParseFromString {
                    what: "version for function id",
                    content: String::from(s),
                }
            })?,
        })
    }
}

impl Display for FunctionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.name, self.version)
    }
}

impl<T> IoReader for T
where
    T: PollRead,
{
    fn poll_read(&mut self, buf: &mut [u8]) -> Result<std::task::Poll<usize>, std::io::Error> {
        PollRead::poll_read(self, buf)
    }
}

impl<T> IoWriter for T
where
    T: PollWrite + ChannelWriter,
{
    fn poll_write(self: &mut T, buf: &[u8]) -> Result<std::task::Poll<()>, std::io::Error> {
        PollWrite::poll_write(self, buf)
    }

    fn close(self: &mut T) -> Result<(), String> {
        ChannelWriter::close(self).map_err(|e| e.to_string())
    }
}

#[derive(Serialize)]
struct FunctionContext {
    name: String,
    inputs: HashMap<String, u64>,
    outputs: HashMap<String, u64>,
    attachments: HashMap<String, u64>,

    submission_fd: RawFd,
    completion_fd: RawFd,
    event_queue_size: u32,
}

#[derive(Error, Debug)]
pub enum CacheError {
    #[error("Cache IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Unknown cache error: {0}")]
    Unknown(String),
}

pub trait Cache {
    type Entry: CacheEntry;

    fn entry(&self, key: &str) -> Self::Entry;
}

pub trait CacheEntry {
    type Reader: IoReader;
    type Writer: Write;
    fn read(&self) -> Result<Option<Self::Reader>, CacheError>;
    fn write(&self) -> Result<Self::Writer, CacheError>;
}

#[derive(Clone)]
pub struct FsCache {
    root: PathBuf,
}

impl FsCache {
    pub fn new<P: AsRef<Path>>(root: P) -> Self {
        Self {
            root: root.as_ref().to_owned(),
        }
    }
}

impl Cache for FsCache {
    type Entry = FsCacheEntry;

    fn entry(&self, key: &str) -> Self::Entry {
        Self::Entry {
            path: self.root.join(key),
        }
    }
}

pub struct FsCacheEntry {
    path: PathBuf,
}

pub struct StdFile {
    file: std::fs::File,
}

impl IoReader for StdFile {
    fn poll_read(&mut self, buf: &mut [u8]) -> Result<std::task::Poll<usize>, std::io::Error> {
        self.file.read(buf).map(std::task::Poll::Ready)
    }
}

impl Write for StdFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.file.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

impl CacheEntry for FsCacheEntry {
    type Reader = StdFile;
    type Writer = StdFile;

    fn read(&self) -> Result<Option<Self::Reader>, CacheError> {
        self.path
            .exists()
            .then(|| {
                OpenOptions::new()
                    .read(true)
                    .open(&self.path)
                    .map(|f| StdFile { file: f })
            })
            .transpose()
            .map_err(Into::into)
    }

    fn write(&self) -> Result<Self::Writer, CacheError> {
        OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&self.path)
            .map(|f| StdFile { file: f })
            .map_err(Into::into)
    }
}

enum CacheState<R: IoReader, W: Write, S: IoReader> {
    Cached(R),
    NotCached { source: S, sink: W },
}

struct CachingReader<W: Write, R: IoReader, C: CacheEntry, S: IoReader> {
    cache_entry: C,
    state: CacheState<R, W, S>,
}

impl<C, S> CachingReader<C::Writer, C::Reader, C, S>
where
    S: IoReader,
    C: CacheEntry,
{
    pub fn new(inner: S, cache_entry: C) -> Result<Self, (CacheError, S)> {
        let sink = match cache_entry.write() {
            Ok(s) => s,
            Err(e) => return Err((e, inner)),
        };

        Ok(Self {
            state: match cache_entry.read() {
                Ok(Some(reader)) => CacheState::Cached(reader),
                Ok(None) => CacheState::NotCached {
                    sink,
                    source: inner,
                },
                Err(e) => {
                    return Err((e, inner));
                }
            },

            cache_entry,
        })
    }
}

impl<C, S> IoReader for CachingReader<C::Writer, C::Reader, C, S>
where
    S: IoReader,
    C: CacheEntry,
{
    fn poll_read(&mut self, buf: &mut [u8]) -> Result<std::task::Poll<usize>, std::io::Error> {
        match self.state {
            CacheState::Cached(ref mut r) => r.poll_read(buf),
            CacheState::NotCached {
                ref mut source,
                ref mut sink,
            } => {
                use std::{
                    io::{Error, ErrorKind},
                    task::Poll,
                };

                let mut b = [0u8; 1024];
                match source.poll_read(&mut b) {
                    Ok(Poll::Ready(0)) => {
                        // we have read everything from the underlying source,
                        // start returning data from the cache entry
                        self.state = CacheState::Cached(
                            self.cache_entry
                                .read()
                                .map(|maybe_reader| {
                                    maybe_reader.ok_or_else(|| Error::from(ErrorKind::Other))
                                })
                                .map_err(|e| Error::new(ErrorKind::Other, e))
                                .and_then(|inner| inner)?,
                        );

                        // next call to poll read will read from the cache entry
                        Ok(Poll::Pending)
                    }
                    Ok(Poll::Ready(sz)) => sink.write_all(&b[0..sz]).map(|_| Poll::Pending),
                    Ok(Poll::Pending) => Ok(Poll::Pending),
                    Err(e) => Err(e),
                }
            }
        }
    }
}

pub struct QueuedFunction<C: Cache> {
    id: ExecutionId,
    function: Function,
    runtime: RuntimeSpec,
    cache: Option<C>,
}

impl<C: Cache + Clone> Clone for QueuedFunction<C> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            function: self.function.clone(),
            runtime: self.runtime.clone(),
            cache: self.cache.clone(),
        }
    }
}

#[derive(Clone)]
pub struct RuntimeSpec {
    path: PathBuf,
    env: HashMap<String, String>,
}

impl<C> QueuedFunction<C>
where
    C: Cache,
{
    fn new(function: Function, runtime: RuntimeSpec, cache: Option<C>) -> Self {
        Self {
            id: ExecutionId::new(),
            function,
            runtime,
            cache,
        }
    }

    pub fn id(&self) -> &ExecutionId {
        &self.id
    }

    pub fn function(&self) -> &Function {
        &self.function
    }

    pub fn run(
        &self,
        input: impl for<'x> Stream<'x>,
        output: impl for<'x> Stream<'x>,
    ) -> Result<(), RuntimeError> {
        fn register_attachment_reader<'cache, 'reader, R, C>(
            cache: Option<&'cache C>,
            event_queue: &mut IoEventQueue<'reader>,
            inner: R,
            attachment_name: &str,
            id: IoId,
        ) where
            R: IoReader + 'reader,
            C: Cache,
            <C as Cache>::Entry: 'reader,
        {
            match cache {
                Some(cache) => match CachingReader::new(inner, cache.entry(attachment_name)) {
                    Ok(caching_reader) => event_queue.register_reader(id, caching_reader),
                    Err((_, reader)) => event_queue.register_reader(id, reader),
                },
                None => event_queue.register_reader(id, inner),
            }
        }

        let mut event_queue: IoEventQueue = IoEventQueue::try_new(32)?;

        let ctx = FunctionContext {
            name: self.function.name.to_owned(),
            inputs: input
                .readers()
                .into_iter()
                .map(|input| {
                    let id = IoId::generate_read();
                    let channel_id = input.channel_id().to_owned();
                    event_queue.register_reader(id, input);
                    (channel_id, id.raw())
                })
                .collect(),

            outputs: output
                .writers()
                .into_iter()
                .map(|output| {
                    let id = IoId::generate_write();
                    let channel_id = output.channel_id().to_owned();
                    event_queue.register_writer(id, output);
                    (channel_id, id.raw())
                })
                .collect(),

            attachments: self
                .function
                .attachments
                .iter()
                .map(|a| {
                    let id = IoId::generate_read();
                    match a.create_reader().map_err(RuntimeError::from)? {
                        AttachmentReader::File(reader) => register_attachment_reader(
                            self.cache.as_ref(),
                            &mut event_queue,
                            reader,
                            &a.name,
                            id,
                        ),
                        AttachmentReader::Http(reader) => {
                            register_attachment_reader(
                                self.cache.as_ref(),
                                &mut event_queue,
                                reader,
                                &a.name,
                                id,
                            );
                        }
                    };
                    Ok((a.name.clone(), id.raw()))
                })
                .collect::<Result<_, RuntimeError>>()?,

            submission_fd: event_queue.submission_fd(),
            completion_fd: event_queue.completion_fd(),
            event_queue_size: 32,
        };
        let mut runtime_process = Command::new(&self.runtime.path)
            .envs(&self.runtime.env)
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| RuntimeError::FailedToCreateRuntimeProcess(e.to_string()))?;

        let stdin = runtime_process.stdin.take().ok_or_else(|| {
            RuntimeError::FailedToCreateRuntimeProcess(String::from("Runtime process had no stdin"))
        })?;
        serde_json::to_writer(&stdin, &ctx)
            .map_err(|e| RuntimeError::FailedToCreateRuntimeProcess(e.to_string()))?;
        drop(stdin);

        loop {
            match runtime_process.try_wait() {
                Ok(Some(_ec)) => {
                    // process exited normally
                    break;
                }
                Ok(None) => {
                    while let Ok(done) = event_queue.update() {
                        if done {
                            break;
                        }
                    }
                }
                Err(_) => {
                    // wtf
                    break;
                }
            }
        }

        Ok(())
    }
}

struct FunctionDirectory {
    root: PathBuf,
    executions: PathBuf,
    cache: PathBuf,
}

impl FunctionDirectory {
    pub fn new<P: AsRef<Path>>(root: P, function_id: &FunctionId) -> Self {
        let root = root.as_ref().join(PathBuf::from(format!(
            "{}-{}",
            function_id.name, function_id.version
        )));
        Self {
            executions: root.join("executions"),
            cache: root.join("cache"),
            root,
        }
    }
    pub fn create(&self) -> Result<(), RuntimeError> {
        std::fs::create_dir_all(&self.root)
            .map_err(|e| RuntimeError::FailedToCreateFunctionDir(self.root.clone(), e))?;

        std::fs::create_dir_all(&self.executions)
            .map_err(|e| RuntimeError::FailedToCreateExecutionsDir(self.root.clone(), e))?;
        std::fs::create_dir_all(&self.cache)
            .map_err(|e| RuntimeError::FailedToCreateCacheDir(self.root.clone(), e))?;

        Ok(())
    }

    pub fn create_execution(&self, execution_id: &ExecutionId) -> Result<(), RuntimeError> {
        std::fs::create_dir_all(&self.executions.join(execution_id.to_string())).map_err(|e| {
            RuntimeError::FailedToCreateExecutionDir(
                self.executions.clone(),
                execution_id.to_string(),
                e,
            )
        })
    }
}

pub trait Store {
    fn function_executions(
        &self,
        function_id: &FunctionId,
    ) -> Result<Vec<ExecutionId>, RuntimeError>;

    fn functions(&self) -> Result<Vec<FunctionId>, RuntimeError>;

    fn execute_function<C: Cache>(
        &self,
        function: &Function,
        cache: Option<C>,
    ) -> Result<QueuedFunction<C>, RuntimeError>;

    fn list_runtimes(&self) -> Result<Vec<PathBuf>, RuntimeError>;
}

#[derive(Clone)]
pub struct FsStore {
    root_path: PathBuf,
    runtime_search_path: Vec<PathBuf>,
}

impl FsStore {
    pub fn new<P1: AsRef<Path>, P2: AsRef<Path>>(root: P1, runtime_search_path: &[P2]) -> Self {
        FsStore {
            root_path: root.as_ref().to_owned(),
            runtime_search_path: runtime_search_path
                .iter()
                .map(|p| p.as_ref().to_owned())
                .collect(),
        }
    }

    fn resolve_runtime(&self, function: &Function) -> Result<RuntimeSpec, RuntimeError> {
        let runtime_spec = function.runtime.as_ref().ok_or_else(|| {
            RuntimeError::RuntimeSpecMissing(FunctionId::from(function).to_string())
        })?;

        let mut env = HashMap::new();
        env.extend(
            runtime_spec
                .arguments
                .iter()
                .map(|(k, v)| (k.to_uppercase(), v.clone())),
        );

        self.runtime_search_path
            .iter()
            .find_map(|path| {
                path.read_dir().ok().and_then(|mut dir_iter| {
                    dir_iter.find_map(|de| {
                        de.ok().and_then(|d| {
                            (d.file_name() == OsString::from(&runtime_spec.name)).then(|| d.path())
                        })
                    })
                })
            })
            .ok_or_else(|| {
                RuntimeError::FailedToFindRuntime(
                    runtime_spec.name.clone(),
                    self.runtime_search_path
                        .iter()
                        .map(|p| p.to_string_lossy())
                        .collect::<Vec<_>>()
                        .join(":"),
                )
            })
            .map(|path| RuntimeSpec { path, env })
    }
}

impl Store for FsStore {
    fn function_executions(
        &self,
        function_id: &FunctionId,
    ) -> Result<Vec<ExecutionId>, RuntimeError> {
        FunctionDirectory::new(&self.root_path, function_id)
            .root
            .read_dir()
            .map_err(|e| {
                RuntimeError::FailedToReadStoreDirectory(self.root_path.clone(), e.to_string())
            })
            .and_then(|paths| {
                paths
                    .into_iter()
                    .filter_map(|de| {
                        de.ok().map(|entry| {
                            ExecutionId::from_str(&entry.file_name().to_string_lossy())
                        })
                    })
                    .collect::<Result<Vec<_>, RuntimeError>>()
            })
    }

    fn execute_function<C: Cache>(
        &self,
        function: &Function,
        cache: Option<C>,
    ) -> Result<QueuedFunction<C>, RuntimeError> {
        std::fs::create_dir_all(&self.root_path)
            .map_err(|e| RuntimeError::FailedToCreateStoreDir(self.root_path.clone(), e))
            .map(|_| FunctionDirectory::new(&self.root_path, &FunctionId::from(function)))
            .and_then(|function_dir| {
                function_dir.create()?;
                let queued =
                    QueuedFunction::new(function.clone(), self.resolve_runtime(function)?, cache);
                function_dir.create_execution(queued.id())?;
                Ok(queued)
            })
    }

    fn functions(&self) -> Result<Vec<FunctionId>, RuntimeError> {
        self.root_path
            .read_dir()
            .map_err(|e| {
                RuntimeError::FailedToReadStoreDirectory(self.root_path.clone(), e.to_string())
            })
            .and_then(|paths| {
                paths
                    .into_iter()
                    .filter_map(|dir| {
                        dir.ok()
                            .map(|de| de.file_name().to_string_lossy().parse::<FunctionId>())
                    })
                    .collect::<Result<Vec<_>, RuntimeError>>()
            })
    }

    fn list_runtimes(&self) -> Result<Vec<PathBuf>, RuntimeError> {
        self.runtime_search_path
            .iter()
            .filter_map(|path| {
                path.read_dir().ok().map(|entries| {
                    entries.filter_map(|e| {
                        e.and_then(|x| x.file_type().map(|ft| (ft, x.path())))
                            .map(|(ft, path)| (ft.is_file() || ft.is_symlink()).then(|| path))
                            .map_err(|ioe| RuntimeError::FailedToReadRuntimeDir(path.clone(), ioe))
                            .transpose()
                    })
                })
            })
            .flatten()
            .collect()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::fs::File;

    #[test]
    fn fs_store() {
        let root = tempfile::tempdir().unwrap();
        let runtimes_path = tempfile::tempdir().unwrap();
        let runtimes_path2 = tempfile::tempdir().unwrap();
        let store = FsStore::new(root.path(), &[runtimes_path.path(), runtimes_path2.path()]);
        let res = store.functions().unwrap();
        assert_eq!(res, vec![]);

        let runtimes = store.list_runtimes().unwrap();
        assert_eq!(runtimes, Vec::<&Path>::with_capacity(0));
        File::create(runtimes_path.path().join("mega.exe")).unwrap();
        File::create(runtimes_path.path().join("snek.py")).unwrap();
        File::create(runtimes_path.path().join("sune.jpeg")).unwrap();
        std::fs::create_dir(runtimes_path.path().join("det-ballar-ur")).unwrap();
        File::create(
            runtimes_path
                .path()
                .join("det-ballar-ur")
                .join("ignored-file"),
        )
        .unwrap();

        File::create(runtimes_path2.path().join("other-runtime")).unwrap();

        let runtimes = store.list_runtimes().unwrap();
        assert_eq!(runtimes.len(), 4);
        assert!(runtimes
            .iter()
            .any(|r| r == &runtimes_path.path().join("mega.exe")));
        assert!(runtimes
            .iter()
            .any(|r| r == &runtimes_path.path().join("snek.py")));
        assert!(runtimes
            .iter()
            .any(|r| r == &runtimes_path.path().join("sune.jpeg")));
        assert!(runtimes
            .iter()
            .any(|r| r == &runtimes_path2.path().join("other-runtime")));

        let runtimes_path2 = tempfile::tempdir().unwrap();
        let runtimes_path3 = runtimes_path2.path().join("tjotahejti");
        let store = FsStore::new(root.path(), &[runtimes_path.path(), &runtimes_path3]);

        let res = store.list_runtimes();
        assert!(res.is_ok());
        let runtimes = res.unwrap();
        assert_eq!(runtimes.len(), 3);
    }
}
