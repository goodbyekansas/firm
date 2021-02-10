use std::{
    collections::HashMap,
    error::Error,
    fs::File,
    path::{Path, PathBuf},
};

use firm_types::{
    functions::AttachmentUrl,
    functions::AuthMethod,
    functions::Checksums,
    functions::Strings,
    functions::{channel::Value as ValueType, Attachment, Channel, Stream as ValueStream},
    stream::{StreamExt, ToChannel},
    wasi::Attachments,
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
    runtime_checksums: Option<Checksums>,
    logger: Logger,
}

impl NestedWasiRuntime {
    fn new(name: &str, executable: &Path, checksums: Option<Checksums>, logger: Logger) -> Self {
        Self {
            wasi_runtime: wasi::WasiRuntime::new(logger.new(o!("runtime" => "wasi"))),
            runtime_executable: executable.to_owned(),
            runtime_name: name.to_owned(),
            runtime_checksums: checksums,
            logger,
        }
    }

    fn with_filesystem<P>(mut self, host_path: P) -> Self
    where
        P: AsRef<Path>,
    {
        self.wasi_runtime = self.wasi_runtime.with_host_dir("runtime-fs", host_path);
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
        let mut runtime_arguments = ValueStream {
            channels: runtime_parameters
                .arguments
                .into_iter()
                .map(|(k, v)| {
                    (
                        k,
                        Channel {
                            value: Some(ValueType::Strings(Strings { values: vec![v] })),
                        },
                    )
                })
                .collect(),
        };

        // not having any code for the function is a valid case used for example to execute
        // external functions (gcp, aws lambdas, etc)
        if let Some(code) = runtime_parameters.code {
            let mut code_buf = Vec::with_capacity(code.encoded_len());
            code.encode(&mut code_buf)?;

            runtime_arguments.set_channel("_code", code_buf.to_channel());

            let checksums = code.checksums.ok_or(RuntimeError::MissingChecksums)?;
            runtime_arguments.set_channel("_sha256", checksums.sha256.to_channel());
        }

        runtime_arguments.set_channel("_entrypoint", runtime_parameters.entrypoint.to_channel());

        // nest arguments and attachments
        let mut arguments_buf: Vec<u8> = Vec::with_capacity(function_arguments.encoded_len());
        function_arguments.encode(&mut arguments_buf)?;
        runtime_arguments.set_channel("_arguments", arguments_buf.to_channel());

        let proto_attachments = Attachments {
            attachments: function_attachments,
        };
        let mut attachments_buf: Vec<u8> = Vec::with_capacity(proto_attachments.encoded_len());
        proto_attachments.encode(&mut attachments_buf)?;
        runtime_arguments.set_channel("_attachments", attachments_buf.to_channel());

        self.wasi_runtime.execute(
            RuntimeParameters {
                function_name: runtime_parameters.function_name.to_owned(),
                output_sink: runtime_parameters.output_sink,
                entrypoint: None,
                code: Some(Attachment {
                    name: format!("{}-code", self.runtime_name),
                    url: Some(AttachmentUrl {
                        url: format!("file://{}", self.runtime_executable.display()),
                        auth_method: AuthMethod::None as i32,
                    }),
                    metadata: HashMap::new(),
                    checksums: self.runtime_checksums.clone(),
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
            },
            runtime_arguments,
            vec![], // files on disk can not have attachments
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
) -> (String, RuntimeWrapper) {
    let checksums = directory_checksums
        .get(&format!("{}.{}", name, ext))
        .map(|c| c.into());

    let log = logger.new(o!(
        "runtime" => name.to_owned(),
        "parent runtime" => "wasi"
    ));

    let name = name.to_owned();
    (
        name.clone(),
        if ext == "wasm" {
            Box::new(move |_cache_dir: &Path| -> Option<Box<dyn Runtime>> {
                Some(Box::new(NestedWasiRuntime::new(
                    &name,
                    &path,
                    checksums.clone(),
                    log.clone(),
                )))
            }) as RuntimeWrapper
        } else {
            Box::new(move |cache_dir: &Path| -> Option<Box<dyn Runtime>> {
                // unpack tar.gz to a cached location
                let destination = cache_dir.join(format!(
                    "{}-{}",
                    &name,
                    checksums
                        .as_ref()
                        .map(|cs| &cs.sha256[..16])
                        .unwrap_or("unknown")
                ));

                if !destination.exists() {
                    if let Err(e) = File::open(&path).and_then(|archive_file| {
                        Archive::new(GzDecoder::new(archive_file)).unpack(&destination)
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
                    &destination.join(format!("{}.wasm", &name)),
                    checksums.clone(),
                    log.clone(),
                );

                // instruct nestedwasiruntime to map "fs"
                if destination.join("fs").exists() {
                    nested_runtime = nested_runtime.with_filesystem(destination.join("fs"));
                }

                Some(Box::new(nested_runtime))
            }) as RuntimeWrapper
        },
    )
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
                            direntry.file_type().ok().and_then(|ft| {
                                if ft.is_file() {
                                    Some(direntry)
                                } else {
                                    None
                                }
                            })
                        })
                        .and_then(|direntry| {
                            let p = direntry.path();
                            p.file_name().and_then(|filename| {
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
                                        Some(create_nested_wasi_runtime(
                                            stem,
                                            e,
                                            p.clone(),
                                            &directory_checksums,
                                            &logger,
                                        ))
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
}

#[cfg(test)]
mod tests {

    use flate2::{write::GzEncoder, Compression};
    use sha2::Digest;
    use tempfile::TempDir;

    use super::*;

    macro_rules! null_logger {
        () => {{
            slog::Logger::root(slog::Discard, slog::o!())
        }};
    }

    macro_rules! with_runtime_dir {
        ($fss:ident, $body:block) => {{
            let td = TempDir::new().unwrap();
            let td_fs = TempDir::new().unwrap();
            std::fs::create_dir_all(td_fs.path().join("fönster/puts")).unwrap();
            std::fs::create_dir_all(td_fs.path().join("fönster/hasp")).unwrap();
            let hello_bytes = include_bytes!("hello.wasm");

            std::fs::write(&td.path().join("bad.wasm"), hello_bytes).unwrap();
            std::fs::write(&td.path().join("missing.wasm"), hello_bytes).unwrap();
            std::fs::write(&td.path().join("good.wasm"), hello_bytes).unwrap();
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

            let good = fss.get("good").unwrap();
            let res = good.execute(RuntimeParameters::new("good"), ValueStream::new(), vec![]);
            assert!(res.is_ok(), "Expected to execute successfully.");
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
            let res = bad.execute(RuntimeParameters::new("bad"), ValueStream::new(), vec![]);
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
                missing.is_some(),
                "Even if we get one with a missing checksum we should get a runtime."
            );
            let missing = missing.unwrap();
            let res = missing.execute(
                RuntimeParameters::new("missing"),
                ValueStream::new(),
                vec![],
            );
            assert!(
                res.is_err(),
                "Missing checksum must result in error during execution."
            );
            assert!(
                matches!(res.unwrap_err(), RuntimeError::MissingChecksums { .. }),
                "Missing checksums error is expected."
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
            assert!(
                bad.unwrap()
                    .execute(RuntimeParameters::new("bad"), ValueStream::new(), vec![])
                    .is_err(),
                "An invalid runtime archive should generate an error when executing"
            );

            let good = fss.get("good_archived");
            assert!(
                good.is_some(),
                "A valid tar.gz should result in a discoverable runtime"
            );
            assert!(
                dbg!(good.unwrap().execute(
                    RuntimeParameters::new("good"),
                    ValueStream::new(),
                    vec![]
                ))
                .is_ok(),
                "A valid runtime should be executable"
            );
        })
    }
}
