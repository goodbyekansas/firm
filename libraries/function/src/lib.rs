pub mod stream;

pub mod io {
    use std::{
        collections::HashMap,
        io::{Cursor, Read, Write},
        task::Poll,
    };

    use firm_protocols::functions::{
        Attachment, ChannelSpec, Checksums, Function, Publisher, RuntimeSpec, Signature,
    };
    use serde::{Deserialize, Serialize};
    use thiserror::Error;

    pub trait PollRead {
        fn poll_read(&mut self, buf: &mut [u8]) -> Result<Poll<usize>, std::io::Error>;
    }

    pub trait PollWrite {
        // TODO: Error if we could not write whole buffer
        fn poll_write(&mut self, buf: &[u8]) -> Result<Poll<()>, std::io::Error>;
    }

    impl<T: PollRead> PollRead for &mut T {
        fn poll_read(&mut self, buf: &mut [u8]) -> Result<Poll<usize>, std::io::Error> {
            PollRead::poll_read(*self, buf)
        }
    }

    impl<T: PollRead> PollRead for Box<T> {
        fn poll_read(&mut self, buf: &mut [u8]) -> Result<Poll<usize>, std::io::Error> {
            PollRead::poll_read(self.as_mut(), buf)
        }
    }

    impl<T: PollWrite> PollWrite for &mut T {
        fn poll_write(&mut self, buf: &[u8]) -> Result<Poll<()>, std::io::Error> {
            PollWrite::poll_write(*self, buf)
        }
    }

    impl<T> PollRead for Cursor<T>
    where
        T: AsRef<[u8]>,
    {
        fn poll_read(&mut self, buf: &mut [u8]) -> Result<Poll<usize>, std::io::Error> {
            match self.read(buf) {
                Ok(n) => Ok(Poll::Ready(n)),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(Poll::Pending),
                Err(e) => Err(e),
            }
        }
    }

    impl PollWrite for Cursor<&mut Vec<u8>> {
        fn poll_write(&mut self, buf: &[u8]) -> Result<Poll<()>, std::io::Error> {
            match self.write(buf) {
                Ok(_) => Ok(Poll::Ready(())),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(Poll::Pending),
                Err(e) => Err(e),
            }
        }
    }

    #[derive(Error, Debug)]
    pub enum FunctionSerializationError {
        #[error("Function manifest parse error: {0}")]
        ParseError(#[from] toml::de::Error),

        #[error("IO error parsing function from manifest: {0}")]
        IoError(#[from] std::io::Error),

        #[error("Function manifest serialization error: {0}")]
        SerializationError(#[from] toml::ser::Error),
    }

    #[derive(Deserialize, Serialize)]
    struct ParsedFunction {
        name: String,
        version: String,
        metadata: HashMap<String, String>,
        inputs: HashMap<String, ParsedChannelSpec>,
        outputs: HashMap<String, ParsedChannelSpec>,
        attachments: Vec<ParsedAttachment>,
        runtime: ParsedRuntimeSpec,
        created_at: u64,
        publisher: Option<ParsedPublisher>,
        signature: Option<Vec<u8>>,
    }

    #[derive(Deserialize, Serialize)]
    struct ParsedChannelSpec {
        description: String,
    }

    #[derive(Deserialize, Serialize)]
    struct ParsedAttachment {
        name: String,
        url: String,
        metadata: HashMap<String, String>,
        checksums: Option<ParsedChecksums>,
        created_at: u64,
        publisher: Option<ParsedPublisher>,
        signature: Option<Vec<u8>>,
    }

    #[derive(Deserialize, Serialize)]
    struct ParsedChecksums {
        sha256: String,
        sha512: String,
    }

    #[derive(Deserialize, Serialize)]
    struct ParsedPublisher {
        name: String,
        email: String,
    }

    #[derive(Deserialize, Serialize)]
    struct ParsedRuntimeSpec {
        name: String,
        arguments: HashMap<String, String>,
    }

    impl From<ParsedFunction> for Function {
        fn from(parsed: ParsedFunction) -> Self {
            Self {
                name: parsed.name,
                version: parsed.version,
                metadata: parsed.metadata,
                inputs: parsed
                    .inputs
                    .into_iter()
                    .map(|(k, v)| (k, v.into()))
                    .collect(),
                outputs: parsed
                    .outputs
                    .into_iter()
                    .map(|(k, v)| (k, v.into()))
                    .collect(),
                attachments: parsed.attachments.into_iter().map(Into::into).collect(),
                runtime: Some(parsed.runtime.into()),
                created_at: parsed.created_at,
                publisher: parsed.publisher.map(Into::into),
                signature: parsed.signature.map(|signature| Signature { signature }),
            }
        }
    }

    impl TryFrom<Function> for ParsedFunction {
        type Error = String;
        fn try_from(f: Function) -> Result<Self, Self::Error> {
            Ok(Self {
                name: f.name.clone(),
                version: f.version,
                metadata: f.metadata,
                inputs: f.inputs.into_iter().map(|(k, v)| (k, v.into())).collect(),
                outputs: f.outputs.into_iter().map(|(k, v)| (k, v.into())).collect(),
                attachments: f.attachments.into_iter().map(Into::into).collect(),
                runtime: f
                    .runtime
                    .ok_or_else(|| format!("Function {} is missing runtime spec", f.name))?
                    .into(),
                created_at: f.created_at,
                publisher: f.publisher.map(Into::into),
                signature: f.signature.map(|sig| sig.signature),
            })
        }
    }

    impl From<ParsedChannelSpec> for ChannelSpec {
        fn from(parsed: ParsedChannelSpec) -> Self {
            Self {
                description: parsed.description,
            }
        }
    }

    impl From<ChannelSpec> for ParsedChannelSpec {
        fn from(cs: ChannelSpec) -> Self {
            Self {
                description: cs.description,
            }
        }
    }

    impl From<ParsedAttachment> for Attachment {
        fn from(parsed: ParsedAttachment) -> Self {
            Self {
                name: parsed.name,
                url: parsed.url,
                metadata: parsed.metadata,
                checksums: parsed.checksums.map(Into::into),
                created_at: parsed.created_at,
                publisher: parsed.publisher.map(Into::into),
                signature: parsed.signature.map(|signature| Signature { signature }),
            }
        }
    }

    impl From<Attachment> for ParsedAttachment {
        fn from(a: Attachment) -> Self {
            Self {
                name: a.name,
                url: a.url,
                metadata: a.metadata,
                checksums: a.checksums.map(Into::into),
                created_at: a.created_at,
                publisher: a.publisher.map(Into::into),
                signature: a.signature.map(|sig| sig.signature),
            }
        }
    }

    impl From<ParsedChecksums> for Checksums {
        fn from(parsed: ParsedChecksums) -> Self {
            Self {
                sha256: parsed.sha256,
                sha512: parsed.sha512,
            }
        }
    }

    impl From<Checksums> for ParsedChecksums {
        fn from(cs: Checksums) -> Self {
            Self {
                sha256: cs.sha256,
                sha512: cs.sha512,
            }
        }
    }

    impl From<ParsedPublisher> for Publisher {
        fn from(parsed: ParsedPublisher) -> Self {
            Self {
                name: parsed.name,
                email: parsed.email,
            }
        }
    }

    impl From<Publisher> for ParsedPublisher {
        fn from(p: Publisher) -> Self {
            Self {
                name: p.name,
                email: p.email,
            }
        }
    }

    impl From<ParsedRuntimeSpec> for RuntimeSpec {
        fn from(parsed: ParsedRuntimeSpec) -> Self {
            Self {
                name: parsed.name,
                arguments: parsed.arguments,
            }
        }
    }

    impl From<RuntimeSpec> for ParsedRuntimeSpec {
        fn from(rs: RuntimeSpec) -> Self {
            Self {
                name: rs.name,
                arguments: rs.arguments,
            }
        }
    }

    pub fn function_from_toml<R: Read>(
        mut reader: R,
    ) -> Result<Function, FunctionSerializationError> {
        let mut s = String::new();
        reader
            .read_to_string(&mut s)
            .map_err(Into::into)
            .and_then(|_| toml::from_str::<ParsedFunction>(&s).map_err(Into::into))
            .map(|parsed| parsed.into())
    }

    pub fn function_to_toml<W: Write>(
        mut writer: W,
        function: Function,
    ) -> Result<(), FunctionSerializationError> {
        toml::to_string_pretty(&ParsedFunction::try_from(function))
            .map_err(Into::into)
            .and_then(|toml_str| writer.write_all(toml_str.as_bytes()).map_err(Into::into))
    }
}

pub mod attachments;
