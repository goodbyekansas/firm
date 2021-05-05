use std::{
    collections::HashMap,
    error::Error,
    fs::File,
    io::Write,
    path::{Path, PathBuf},
};

use firm_types::{
    functions::AttachmentUrl,
    functions::AuthMethod,
    functions::Checksums,
    functions::{Attachment, Stream as ValueStream},
    wasi::RuntimeContext,
};
use flate2::read::GzDecoder;
use prost::Message;
use serde::{Deserialize, Serialize};
use slog::{debug, info, o, warn, Logger};
use tar::Archive;
use thiserror::Error;

use super::{wasi, Runtime, RuntimeError, RuntimeParameters, RuntimeSource};

type RuntimeWrapper = Box<dyn Fn(&Path) -> Option<Box<dyn Runtime>> + Send + Sync>;
pub struct FileSystemSource {
    runtimes: HashMap<String, RuntimeWrapper>,
    cache_dir: tempfile::TempDir,
}

#[derive(Debug)]
struct NestedWasiRuntime {
    wasi_runtime: wasi::WasiRuntime,
    runtime_name: String,
    runtime_executable: PathBuf,
    runtime_checksums: Checksums,
    logger: Logger,
    runtime_context_folder: PathBuf,
}

impl NestedWasiRuntime {
    fn new(
        name: &str,
        executable: &Path,
        runtime_context_folder: &Path,
        checksums: Checksums,
        logger: Logger,
    ) -> Self {
        Self {
            wasi_runtime: wasi::WasiRuntime::new(logger.new(o!())),
            runtime_executable: executable.to_owned(),
            runtime_name: name.to_owned(),
            runtime_checksums: checksums,
            logger,
            runtime_context_folder: runtime_context_folder.to_owned(),
        }
    }

    fn with_filesystem<P, S>(mut self, host_path: P, guest_path: S) -> Self
    where
        P: AsRef<Path>,
        S: AsRef<str>,
    {
        self.wasi_runtime = self
            .wasi_runtime
            .with_host_dir(guest_path.as_ref(), host_path);
        self
    }
}

impl Runtime for NestedWasiRuntime {
    fn execute(
        &self,
        runtime_parameters: RuntimeParameters,
        function_arguments: ValueStream,
        function_attachments: Vec<Attachment>,
    ) -> Result<Result<ValueStream, String>, RuntimeError> {
        let runtime_context = RuntimeContext {
            code: runtime_parameters.code,
            entrypoint: runtime_parameters.entrypoint.unwrap_or_default(),
            arguments: runtime_parameters.arguments,
            name: runtime_parameters.function_name.clone(),
        };

        std::fs::create_dir_all(&self.runtime_context_folder).map_err(|e| {
            RuntimeError::RuntimeError {
                name: "nested-wasi".to_owned(),
                message: format!("Failed to create runtime context folder: {}", e),
            }
        })?;
        let runtime_context_file_hostpath = self.runtime_context_folder.join("context");
        let mut runtime_context_file = std::fs::File::create(runtime_context_file_hostpath)
            .map_err(|e| RuntimeError::RuntimeError {
                name: "nested-wasi".to_owned(),
                message: e.to_string(),
            })?;

        let mut runtime_context_buf: Vec<u8> = Vec::with_capacity(runtime_context.encoded_len());
        runtime_context.encode(&mut runtime_context_buf)?;
        runtime_context_file
            .write_all(&runtime_context_buf)
            .map_err(|e| RuntimeError::RuntimeError {
                name: "nested-wasi".to_owned(),
                message: e.to_string(),
            })?;

        self.wasi_runtime.execute(
            RuntimeParameters {
                function_name: runtime_parameters.function_name.to_owned(),
                output_sink: runtime_parameters.output_sink,
                entrypoint: None,
                code: Some(Attachment {
                    name: format!("{}-runtime-code", self.runtime_name),
                    url: Some(AttachmentUrl {
                        url: format!("file://{}", self.runtime_executable.display()),
                        auth_method: AuthMethod::None as i32,
                    }),
                    metadata: HashMap::new(),
                    checksums: Some(self.runtime_checksums.clone()),
                    created_at: self
                        .runtime_executable
                        .metadata()
                        .and_then(|meta| meta.created())
                        .map_err(|_| ())
                        .and_then(|created| {
                            created
                                .duration_since(std::time::UNIX_EPOCH)
                                .map_err(|_| ())
                        })
                        .map_or(0, |timestamp| timestamp.as_secs()),
                }),
                arguments: HashMap::new(), // files on disk can not have arguments
                function_dir: runtime_parameters.function_dir,
                auth_service: runtime_parameters.auth_service,
                async_runtime: runtime_parameters.async_runtime,
            },
            function_arguments,
            function_attachments,
        )
    }
}

#[derive(Error, Debug)]
pub enum FileSystemSourceError {
    #[error("Missing checksum file for directory: {0}")]
    MissingChecksumFile(PathBuf),

    #[error("IO error: {0}")]
    IOError(#[from] std::io::Error),

    #[error("TOML read error: {0}")]
    TOMLError(#[from] toml::de::Error),
}

#[derive(Deserialize, Serialize)]
struct TOMLChecksums {
    pub sha256: String,

    #[serde(default)]
    pub executable_sha256: Option<String>,
}

impl From<&TOMLChecksums> for Checksums {
    fn from(toml_checksums: &TOMLChecksums) -> Self {
        Self {
            sha256: toml_checksums
                .executable_sha256
                .as_deref()
                .unwrap_or_else(|| toml_checksums.sha256.as_str())
                .to_owned(),
        }
    }
}

fn create_nested_wasi_runtime(
    name: &str,
    ext: &str,
    path: PathBuf,
    directory_checksums: &HashMap<String, TOMLChecksums>,
    logger: &Logger,
) -> Option<(String, RuntimeWrapper)> {
    let log = logger.new(o!(
        "runtime" => name.to_owned(),
        "parent runtime" => "wasi"
    ));

    let checksums: Checksums = directory_checksums
        .get(&format!("{}.{}", name, ext))
        .map(|c| c.into())
        .or_else(|| {
            warn!(log, "Failed to find checksum for \"{}.{}\"", name, ext);
            None
        })?;

    let name = name.to_owned();
    let ext = ext.to_owned();
    Some((
        name.clone(),
        Box::new(move |cache_dir: &Path| -> Option<Box<dyn Runtime>> {
            // unpack tar.gz to a cached location
            let function_dir = cache_dir.join(format!("{}-{}", &name, &checksums.sha256[..16]));
            let context_path = function_dir.join("context");
            Some(Box::new(
                if ext == "wasm" {
                    NestedWasiRuntime::new(
                        &name,
                        &path,
                        &context_path,
                        checksums.clone(),
                        log.clone(),
                    )
                } else {
                    if !function_dir.exists() {
                        if let Err(e) = File::open(&path).and_then(|archive_file| {
                            Archive::new(GzDecoder::new(archive_file)).unpack(&function_dir)
                        }) {
                            warn!(
                                log,
                                "failed to unpack runtime archive at \"{}\": {}, caused by: {}",
                                path.display(),
                                e,
                                e.source()
                                    .map(|err| err.to_string())
                                    .unwrap_or_else(|| String::from("unknown"))
                            );
                            return None;
                        }
                    }

                    let mut nested_runtime = NestedWasiRuntime::new(
                        &name,
                        &function_dir.join(format!("{}.wasm", &name)),
                        &context_path,
                        checksums.clone(),
                        log.clone(),
                    );

                    // instruct nestedwasiruntime to map "fs"
                    if function_dir.join("fs").exists() {
                        nested_runtime =
                            nested_runtime.with_filesystem(function_dir.join("fs"), "runtime-fs");
                    }
                    nested_runtime
                }
                .with_filesystem(context_path, "runtime-context"),
            ))
        }) as RuntimeWrapper,
    ))
}

impl FileSystemSource {
    pub fn new(root: &Path, logger: Logger) -> Result<Self, FileSystemSourceError> {
        info!(logger, "Scanning runtimes in directory {}", root.display());
        let fs_source_logger = logger.new(o!("runtime-dir" => root.display().to_string()));
        let checksum_file = root.join(".checksums.toml");

        if !checksum_file.exists() {
            return Err(FileSystemSourceError::MissingChecksumFile(root.to_owned()));
        }

        let directory_checksums: HashMap<String, TOMLChecksums> =
            toml::from_slice(&std::fs::read(checksum_file)?)?;

        Ok(Self {
            runtimes: root
                .read_dir()?
                .filter_map(|direntry| {
                    direntry
                        .ok()
                        .and_then(|direntry| {
                            direntry
                                .path()
                                .canonicalize()
                                .map_err(|e| {
                                    warn!(
                                        logger,
                                        "Failed to resolve directory entry: {}. Skipping!", e
                                    );
                                })
                                .ok()
                                .and_then(|path| if path.is_dir() { None } else { Some(path) })
                        })
                        .and_then(|path| {
                            path.file_name().and_then(|filename| {
                                let filename = filename.to_string_lossy();
                                let filename = filename.as_ref();
                                let (stem, extension) = {
                                    let mut parts = filename.splitn(2, '.');
                                    (parts.next().unwrap_or("no-filename"), parts.next())
                                };

                                extension.and_then(|ext| match ext {
                                    e @ "tar.gz" | e @ "wasm" => {
                                        debug!(
                                            fs_source_logger,
                                            "found runtime {} from file {}", stem, filename
                                        );
                                        create_nested_wasi_runtime(
                                            stem,
                                            e,
                                            path.clone(),
                                            &directory_checksums,
                                            &logger,
                                        )
                                    }
                                    _ => None,
                                })
                            })
                        })
                })
                .collect::<HashMap<String, RuntimeWrapper>>(),
            cache_dir: tempfile::TempDir::new()?,
        })
    }
}

impl RuntimeSource for FileSystemSource {
    fn get(&self, name: &str) -> Option<Box<dyn Runtime>> {
        self.runtimes
            .get(name)
            .and_then(|rtfm| rtfm(self.cache_dir.path()))
    }

    fn list(&self) -> Vec<String> {
        self.runtimes.keys().cloned().collect()
    }

    fn name(&self) -> &'static str {
        "filesystem"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::ops::Deref;

    use firm_types::stream::StreamExt;
    use flate2::{write::GzEncoder, Compression};
    use sha2::Digest;
    use tempfile::TempDir;

    use crate::runtime::FunctionDirectory;

    struct RuntimeParametersWrapper {
        runtime_parameters: RuntimeParameters,
        _temp_root_dir: tempfile::TempDir,
    }

    impl RuntimeParametersWrapper {
        fn new(runtime_parameters: RuntimeParameters, temp_root_dir: tempfile::TempDir) -> Self {
            Self {
                runtime_parameters,
                _temp_root_dir: temp_root_dir,
            }
        }
    }

    impl Deref for RuntimeParametersWrapper {
        type Target = RuntimeParameters;

        fn deref(&self) -> &Self::Target {
            &self.runtime_parameters
        }
    }

    macro_rules! runtime_parameters {
        ($name:expr) => {{
            let temp_root_dir = tempfile::TempDir::new().unwrap();
            RuntimeParametersWrapper::new(
                RuntimeParameters::new(
                    $name,
                    FunctionDirectory::new(
                        temp_root_dir.path(),
                        $name,
                        "0.1.0",
                        "checksum",
                        "execution-id",
                    )
                    .unwrap(),
                )
                .unwrap(),
                temp_root_dir,
            )
        }};
    }

    macro_rules! null_logger {
        () => {{
            slog::Logger::root(slog::Discard, slog::o!())
        }};
    }

    macro_rules! with_runtime_dir {
        ($fss:ident, $body:block) => {{
            let td = TempDir::new().unwrap();
            let td_fs = TempDir::new().unwrap();

            let symlink_folder = TempDir::new().unwrap();

            std::fs::create_dir_all(td_fs.path().join("fönster/puts")).unwrap();
            std::fs::create_dir_all(td_fs.path().join("fönster/hasp")).unwrap();
            let hello_bytes = include_bytes!("hello.wasm");

            std::fs::write(&td.path().join("bad.wasm"), hello_bytes).unwrap();
            std::fs::write(&td.path().join("missing.wasm"), hello_bytes).unwrap();
            std::fs::write(&td.path().join("good.wasm"), hello_bytes).unwrap();

            let link_path = symlink_folder.path().join("symlink.wasm");
            std::fs::write(&link_path, hello_bytes).unwrap();

            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(&link_path, &td.path().join("symlink.wasm")).unwrap();
            }

            #[cfg(windows)]
            {
                std::os::windows::fs::symlink_file(&link_path, &td.path().join("symlink.wasm"))
                    .unwrap();
            }

            let mut archive_bad = tar::Builder::new(GzEncoder::new(
                File::create(&td.path().join("bad_archived.tar.gz")).unwrap(),
                Compression::default(),
            ));
            archive_bad
                .append_path_with_name(&td.path().join("good.wasm"), "wrong_name.wasm")
                .unwrap();
            archive_bad.into_inner().unwrap();

            let mut archive_good = tar::Builder::new(GzEncoder::new(
                File::create(&td.path().join("good_archived.tar.gz")).unwrap(),
                Compression::default(),
            ));
            archive_good
                .append_path_with_name(&td.path().join("good.wasm"), "good_archived.wasm")
                .unwrap();
            archive_good.append_dir_all("fs", td_fs).unwrap();
            archive_good.into_inner().unwrap();

            let mut checksums = HashMap::new();
            checksums.insert(
                String::from("bad.wasm"),
                TOMLChecksums {
                    sha256: hex::encode(sha2::Sha256::digest(&[])),
                    executable_sha256: None,
                },
            );

            checksums.insert(
                String::from("good.wasm"),
                TOMLChecksums {
                    sha256: hex::encode(sha2::Sha256::digest(hello_bytes)),
                    executable_sha256: None,
                },
            );

            checksums.insert(
                String::from("symlink.wasm"),
                TOMLChecksums {
                    sha256: hex::encode(sha2::Sha256::digest(hello_bytes)),
                    executable_sha256: None,
                },
            );

            checksums.insert(
                String::from("bad_archived.tar.gz"),
                TOMLChecksums {
                    sha256: hex::encode(sha2::Sha256::digest(
                        &std::fs::read(&td.path().join("bad_archived.tar.gz")).unwrap(),
                    )),
                    executable_sha256: Some(hex::encode(sha2::Sha256::digest(hello_bytes))),
                },
            );

            checksums.insert(
                String::from("good_archived.tar.gz"),
                TOMLChecksums {
                    sha256: hex::encode(sha2::Sha256::digest(
                        &std::fs::read(&td.path().join("bad_archived.tar.gz")).unwrap(),
                    )),
                    executable_sha256: Some(hex::encode(sha2::Sha256::digest(hello_bytes))),
                },
            );

            std::fs::write(
                &td.path().join(".checksums.toml"),
                &toml::to_vec(&checksums).unwrap(),
            )
            .unwrap();

            let fss = FileSystemSource::new(td.path(), null_logger!());
            assert!(fss.is_ok(), "creating in a valid dir should give Ok");
            let $fss = fss.unwrap();

            $body
        }};
    }

    #[test]
    fn test_empty_dir() {
        assert!(
            FileSystemSource::new(&PathBuf::from("asdasd"), null_logger!()).is_err(),
            "non-existent dir should give an error"
        );

        let td = TempDir::new().unwrap();
        assert!(
            FileSystemSource::new(td.path(), null_logger!()).is_err(),
            "an empty directory should give an error since a checksum file is required"
        );
    }

    #[test]
    fn test_valid_runtimes() {
        with_runtime_dir!(fss, {
            assert!(
                fss.get("good").is_some(),
                "asking for an existing runtime should give something"
            );
            assert!(
                fss.get("kryptid").is_none(),
                "asking for a non-existing runtime should give none"
            );
            assert!(
                fss.get("symlink").is_some(),
                "asking for an existing symlinked runtime should give something"
            );

            let good = fss.get("good").unwrap();
            let parameters = runtime_parameters!("good");
            let res = good.execute(parameters.runtime_parameters, ValueStream::new(), vec![]);
            assert!(res.is_ok(), "Expected to execute successfully.");

            let symlink = fss.get("symlink").unwrap();
            let parameters = runtime_parameters!("symlink");
            let res = symlink.execute(parameters.runtime_parameters, ValueStream::new(), vec![]);
            assert!(
                res.is_ok(),
                "Expected to execute symlink runtime successfully."
            );
        });
    }

    #[test]
    fn test_invalid_and_missing_checksums() {
        with_runtime_dir!(fss, {
            // Bad
            let bad = fss.get("bad");
            assert!(
                bad.is_some(),
                "Even if we get one with a bad checksum we should get a runtime."
            );
            let bad = bad.unwrap();
            let parameters = runtime_parameters!("bad");
            let res = bad.execute(parameters.runtime_parameters, ValueStream::new(), vec![]);
            assert!(
                res.is_err(),
                "Bad checksum must result in error during execution."
            );

            assert!(
                matches!(res.unwrap_err(), RuntimeError::ChecksumMismatch { .. }),
                "Checksum mismatch error is expected."
            );

            // Missing
            let missing = fss.get("missing");
            assert!(
                missing.is_none(),
                "A missing checksum should skip registering the runtime."
            );
        });
    }

    #[test]
    fn test_archived_runtimes() {
        with_runtime_dir!(fss, {
            let bad = fss.get("bad_archived");
            assert!(
                bad.is_some(),
                "A tar.gz archive where the exe has the wrong name should still result in an discoverable runtime"
            );
            let parameters = runtime_parameters!("bad");
            assert!(
                bad.unwrap()
                    .execute(parameters.runtime_parameters, ValueStream::new(), vec![])
                    .is_err(),
                "An invalid runtime archive should generate an error when executing"
            );

            let good = fss.get("good_archived");
            assert!(
                good.is_some(),
                "A valid tar.gz should result in a discoverable runtime"
            );
            let parameters = runtime_parameters!("good");
            assert!(
                good.unwrap()
                    .execute(parameters.runtime_parameters, ValueStream::new(), vec![])
                    .is_ok(),
                "A valid runtime should be executable"
            );
        })
    }
}
