use std::{
    collections::HashMap,
    ffi::OsString,
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
use prost::Message;
use serde::{Deserialize, Serialize};
use slog::{info, o, Logger};
use thiserror::Error;

use super::{wasi, Runtime, RuntimeError, RuntimeParameters, RuntimeSource};

type RuntimeWrapper = Box<dyn Fn() -> Box<dyn Runtime> + Send + Sync>;
pub struct FileSystemSource {
    runtimes: HashMap<OsString, RuntimeWrapper>,
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
}

impl Runtime for NestedWasiRuntime {
    fn execute(
        &self,
        executor_context: RuntimeParameters,
        arguments: ValueStream,
        attachments: Vec<Attachment>,
    ) -> Result<Result<ValueStream, String>, RuntimeError> {
        let mut executor_function_arguments = ValueStream {
            channels: executor_context
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
        if let Some(code) = executor_context.code {
            let mut code_buf = Vec::with_capacity(code.encoded_len());
            code.encode(&mut code_buf)?;

            executor_function_arguments.set_channel("_code", code_buf.to_channel());

            let checksums = code.checksums.ok_or(RuntimeError::MissingChecksums)?;
            executor_function_arguments.set_channel("_sha256", checksums.sha256.to_channel());
        }

        executor_function_arguments
            .set_channel("_entrypoint", executor_context.entrypoint.to_channel());

        // nest arguments and attachments
        let mut arguments_buf: Vec<u8> = Vec::with_capacity(arguments.encoded_len());
        arguments.encode(&mut arguments_buf)?;
        executor_function_arguments.set_channel("_arguments", arguments_buf.to_channel());

        let proto_attachments = Attachments { attachments };
        let mut attachments_buf: Vec<u8> = Vec::with_capacity(proto_attachments.encoded_len());
        proto_attachments.encode(&mut attachments_buf)?;
        executor_function_arguments.set_channel("_attachments", attachments_buf.to_channel());

        self.wasi_runtime.execute(
            RuntimeParameters {
                function_name: self.runtime_name.clone(),
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
            executor_function_arguments,
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
}

impl From<&TOMLChecksums> for Checksums {
    fn from(toml_checksums: &TOMLChecksums) -> Self {
        Self {
            sha256: toml_checksums.sha256.to_owned(),
        }
    }
}

impl FileSystemSource {
    pub fn new(root: &Path, logger: Logger) -> Result<Self, FileSystemSourceError> {
        info!(logger, "Scanning runtimes in directory {}", root.display());
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
                            p.file_stem().and_then(|filename| {
                                p.extension().and_then(
                                    |ext|
                                    match ext.to_string_lossy().as_ref() {
                                        "wasm" => {
                                            let checksums = directory_checksums.get(
                                                &format!("{}.{}",filename.to_string_lossy(), ext.to_string_lossy())
                                            ).map(|c| c.into());

                                            let log = logger.new(o!(
                                                "runtime" => filename.to_string_lossy().into_owned(),
                                                "parent runtime" => "wasi"
                                            ));

                                            let name = filename.to_string_lossy().into_owned();
                                            let p = p.clone();

                                            Some((
                                                filename.to_owned(),
                                                Box::new(move || -> Box<dyn Runtime> {
                                                    Box::new(NestedWasiRuntime::new(&name, &p, checksums.clone(), log.clone()))
                                                }) as RuntimeWrapper
                                            ))
                                        },
                                        _ => None,
                                    },
                                )
                            })
                        })
                })
                .collect::<HashMap<OsString, RuntimeWrapper>>(),
        })
    }
}

impl RuntimeSource for FileSystemSource {
    fn get(&self, name: &str) -> Option<Box<dyn Runtime>> {
        self.runtimes.get(&OsString::from(name)).map(|rtfm| rtfm())
    }
}

#[cfg(test)]
mod tests {

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
            let hello_bytes = include_bytes!("hello.wasm");

            std::fs::write(&td.path().join("bad.wasm"), hello_bytes).unwrap();
            std::fs::write(&td.path().join("missing.wasm"), hello_bytes).unwrap();
            std::fs::write(&td.path().join("good.wasm"), hello_bytes).unwrap();

            let mut checksums = HashMap::new();
            checksums.insert(
                String::from("bad.wasm"),
                TOMLChecksums {
                    sha256: hex::encode(sha2::Sha256::digest(&[])),
                },
            );

            checksums.insert(
                String::from("good.wasm"),
                TOMLChecksums {
                    sha256: hex::encode(sha2::Sha256::digest(hello_bytes)),
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
}
