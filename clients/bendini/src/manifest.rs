use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use thiserror::Error;

use firm_types::{
    functions::{ChannelSpec, ChannelType, Checksums as ProtoChecksums, Runtime as ProtoRuntime},
    registry::{AttachmentData, FunctionData},
};

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("Failed to parse function manifest: {0}")]
    ManifestParseError(#[from] toml::de::Error),

    #[error("Manifest file \"{path}\" could not be read: {io_error}")]
    ManifestFileReadError { path: PathBuf, io_error: io::Error },

    #[error("Attachment file \"{path}\" could not be read: {io_error}")]
    AttachmentFileReadError { path: PathBuf, io_error: io::Error },

    #[error("Invalid manifest path: \"{0}\"")]
    InvalidManifestPath(PathBuf),
}

#[derive(Debug, Deserialize)]
enum ArgumentType {
    #[serde(rename = "string")]
    String,

    #[serde(rename = "bool")]
    Bool,

    #[serde(rename = "int")]
    Int,

    #[serde(rename = "float")]
    Float,

    #[serde(rename = "bytes")]
    Bytes,
}

#[derive(Debug, Deserialize)]
pub struct FunctionManifest {
    name: String,

    #[serde(skip)]
    path: PathBuf,

    version: String,

    #[serde(default)]
    inputs: HashMap<String, Input>,

    #[serde(default)]
    outputs: HashMap<String, Output>,

    runtime: Runtime,

    #[serde(default)]
    metadata: HashMap<String, String>,

    #[serde(default)]
    attachments: HashMap<String, Attachment>,

    #[serde(default)]
    code: Option<Attachment>,
}

#[derive(Debug, Deserialize)]
struct Attachment {
    path: String,

    #[serde(default)]
    metadata: HashMap<String, String>,
    checksums: Checksums,
}

#[derive(Debug, Deserialize)]
struct Input {
    r#type: ArgumentType,

    #[serde(default)]
    required: bool,

    #[serde(default)]
    description: String,
}

#[derive(Debug, Deserialize)]
struct Output {
    r#type: ArgumentType,

    #[serde(default)]
    description: String,
}

#[derive(Debug, Deserialize, Clone)]
struct Checksums {
    sha256: String,
}

impl From<&Checksums> for ProtoChecksums {
    fn from(checksum: &Checksums) -> Self {
        Self {
            sha256: checksum.sha256.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct Runtime {
    r#type: String,

    #[serde(default)]
    entrypoint: String,

    #[serde(default)]
    args: HashMap<String, String>,
}

#[derive(Debug, PartialEq)]
pub struct AttachmentInfo {
    pub path: PathBuf,
    pub request: AttachmentData,
}

impl FunctionManifest {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn parse<P: AsRef<Path>>(path: P) -> Result<Self, ManifestError> {
        let fullpath =
            path.as_ref()
                .canonicalize()
                .map_err(|e| ManifestError::ManifestFileReadError {
                    path: path.as_ref().to_path_buf(),
                    io_error: e,
                })?;
        let mut manifest: Self = std::fs::read_to_string(&fullpath)
            .map_err(|e| ManifestError::ManifestFileReadError {
                path: path.as_ref().to_path_buf(),
                io_error: e,
            })
            .and_then(|toml_content| toml::from_str(&toml_content).map_err(|e| e.into()))?;

        manifest.path = fullpath;
        Ok(manifest)
    }

    fn get_attachment_path(&self, attachment: &Attachment) -> Result<PathBuf, ManifestError> {
        let fullpath = self
            .path
            .parent()
            .ok_or_else(|| ManifestError::InvalidManifestPath(self.path.clone()))?
            .join(attachment.path.clone());
        Ok(fullpath
            .canonicalize()
            .map_err(|e| ManifestError::AttachmentFileReadError {
                path: fullpath.clone(),
                io_error: e,
            })?)
    }

    pub fn code(&self) -> Result<Option<AttachmentInfo>, ManifestError> {
        self.code
            .as_ref()
            .map(|code| {
                self.get_attachment_path(&code)
                    .map(|absolute| AttachmentInfo {
                        path: absolute,
                        request: AttachmentData {
                            name: "code".to_owned(),
                            metadata: code.metadata.clone(),
                            checksums: Some(ProtoChecksums::from(&code.checksums)),
                        },
                    })
            })
            .transpose()
    }

    pub fn attachments(&self) -> Result<Vec<AttachmentInfo>, ManifestError> {
        self.attachments
            .iter()
            .map(|(n, a)| {
                self.get_attachment_path(&a).map(|absolute| AttachmentInfo {
                    path: absolute,
                    request: AttachmentData {
                        name: n.clone(),
                        metadata: a.metadata.clone(),
                        checksums: Some(ProtoChecksums::from(&a.checksums)),
                    },
                })
            })
            .collect()
    }
}

impl From<&FunctionManifest> for FunctionData {
    fn from(fm: &FunctionManifest) -> Self {
        FunctionData {
            name: fm.name.clone(),
            version: fm.version.clone(),
            metadata: fm.metadata.clone(),
            required_inputs: fm
                .inputs
                .iter()
                .filter_map(|(name, input)| {
                    if input.required {
                        Some((
                            name.clone(),
                            ChannelSpec {
                                r#type: ChannelType::from(&input.r#type) as i32,
                                description: input.description.to_owned(),
                            },
                        ))
                    } else {
                        None
                    }
                })
                .collect(),
            optional_inputs: fm
                .inputs
                .iter()
                .filter_map(|(name, input)| {
                    if !input.required {
                        Some((
                            name.clone(),
                            ChannelSpec {
                                r#type: ChannelType::from(&input.r#type) as i32,
                                description: input.description.to_owned(),
                            },
                        ))
                    } else {
                        None
                    }
                })
                .collect(),
            outputs: fm
                .outputs
                .iter()
                .map(|(name, output)| {
                    (
                        name.clone(),
                        ChannelSpec {
                            description: output.description.clone(),
                            r#type: ChannelType::from(&output.r#type) as i32,
                        },
                    )
                })
                .collect(),
            code_attachment_id: None,
            runtime: Some(ProtoRuntime {
                name: fm.runtime.r#type.clone(),
                arguments: fm.runtime.args.clone(),
                entrypoint: fm.runtime.entrypoint.clone(),
            }),
            attachment_ids: vec![],
        }
    }
}

// this is here to get compile time checks that the two enum types
// are identical
impl From<ChannelType> for ArgumentType {
    fn from(at: ChannelType) -> Self {
        match at {
            ChannelType::String => ArgumentType::String,
            ChannelType::Bool => ArgumentType::Bool,
            ChannelType::Int => ArgumentType::Int,
            ChannelType::Float => ArgumentType::Float,
            ChannelType::Bytes => ArgumentType::Bytes,
        }
    }
}

impl From<&ArgumentType> for ChannelType {
    fn from(at: &ArgumentType) -> Self {
        match *at {
            ArgumentType::String => ChannelType::String,
            ArgumentType::Bool => ChannelType::Bool,
            ArgumentType::Int => ChannelType::Int,
            ArgumentType::Float => ChannelType::Float,
            ArgumentType::Bytes => ChannelType::Bytes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Write;

    use tempfile::{NamedTempFile, TempDir};

    use firm_types::{attachment_data, runtime};

    macro_rules! write_toml_to_tempfile {
        ($toml: expr) => {{
            let mut f = NamedTempFile::new().unwrap();
            write!(f, "{}", $toml).unwrap();
            f.into_temp_path()
        }};
    }

    #[test]
    fn test_parse() {
        let toml = r#""#;
        let r = FunctionManifest::parse(write_toml_to_tempfile!(toml));
        assert!(r.is_err());
        assert!(matches!(
            r.unwrap_err(),
            ManifestError::ManifestParseError(_)
        ));

        let toml = r#"
        name = "start-blender"
        version = "0.1.0"
        [runtime]
        type = "wasm"
        "#;
        let r = FunctionManifest::parse(write_toml_to_tempfile!(toml));
        assert!(r.is_ok());

        let toml = r#"
        name = "start-blender"
        [inputs]
          [inputs.version]
          type = "string"

        [outputs]
          [outputs.pid]

        [runtime]
        type = "wasm"
        "#;
        let r = FunctionManifest::parse(write_toml_to_tempfile!(toml));
        assert!(r.is_err());
        assert!(matches!(
            r.unwrap_err(),
            ManifestError::ManifestParseError(_)
        ));

        let toml = r#"
        name = "start-blender"

        version = "0.1.0"

        [inputs]
          [inputs.version]
          type = "string"

        [outputs]
          [outputs.pid]
          type="int"

        [runtime]
        type = "wasm"

        [runtime.args]
        sune = "bune"
        "#;
        let r = FunctionManifest::parse(write_toml_to_tempfile!(toml));
        assert!(r.is_ok());
        assert_eq!(r.unwrap().runtime.args["sune"], "bune");

        let r = FunctionManifest::parse(Path::new(""));
        assert!(r.is_err());
        assert!(matches!(
            r.unwrap_err(),
            ManifestError::ManifestFileReadError{ .. }
        ));

        // Test parsing code and attachments
        let toml = r#"
        name = "start-blender"
        version = "0.1.0"
        [runtime]
        type = "wasm"

        [code]
        path = "code/path"
        metadata = { is_code = "true" }
        [code.checksums]
        sha256 = "7767e3afca54296110dd596d8de7cd8adc6f89253beb3c69f0fc810df7f8b6d5"

        [attachments.kalle]
        path = "fabrikam/sune"
        metadata = { someMetadata = "sune", cool = "chorizo korvén" }
        [attachments.kalle.checksums]
        sha256 = "7767e3afca54296110dd596d8de7cd8adc6f89253beb3c69f0fc810df7f8b6d5"

        [attachments.oran]
        path = "fabrikam/security"
        metadata = { surname = "jonsson" }
        [attachments.oran.checksums]
        sha256 = "7767e3afca54296110dd596d8de7cd8adc6f89253beb3c69f0fc810df7f8b6d5"
        "#;

        let r = FunctionManifest::parse(write_toml_to_tempfile!(toml));
        assert!(&r.is_ok());
        let val = r.unwrap();

        assert!(val.code.is_some());
        let code = val.code.unwrap();
        assert_eq!(code.path, "code/path");
        assert_eq!(code.metadata.get("is_code").unwrap(), "true");

        assert_eq!(val.attachments.len(), 2);
        let attachment = val.attachments.get("kalle").unwrap();

        assert_eq!(attachment.path, "fabrikam/sune");
        assert_eq!(attachment.metadata.get("someMetadata").unwrap(), "sune");
        assert_eq!(attachment.metadata.get("cool").unwrap(), "chorizo korvén");

        let attachment = val.attachments.get("oran").unwrap();
        assert_eq!(attachment.path, "fabrikam/security");
        assert_eq!(attachment.metadata.get("surname").unwrap(), "jonsson");
    }

    #[test]
    fn test_register_request_conversion() {
        // Test parsing code and attachments
        let toml = r#"
        name = "super-simple"
        version = "0.1.0"
        [runtime]
        type = "wasm"
        "#;

        let r = FunctionManifest::parse(write_toml_to_tempfile!(toml));
        let rr = FunctionData::from(&r.unwrap());
        assert_eq!(rr.name, "super-simple");
        assert_eq!(rr.version, "0.1.0");

        assert_eq!(rr.runtime, Some(runtime!("wasm")));

        let toml = r#"
        name = "super-simple"
        version = "0.1.0"
        [runtime]
        type = "wasm"
        [inputs.korv]
        description = "falu"
        type = "string"
        required = true
        [inputs.aaa]
        description = "bbb"
        type = "float"
        [outputs.ost]
        description = "chez"
        type = "int"
        "#;

        let r = FunctionManifest::parse(write_toml_to_tempfile!(toml));
        let rr = FunctionData::from(&r.unwrap());
        assert_eq!(
            rr.required_inputs.get("korv"),
            Some(&ChannelSpec {
                r#type: ChannelType::String as i32,
                description: "falu".to_owned(),
            })
        );
        assert_eq!(
            rr.optional_inputs.get("aaa"),
            Some(&ChannelSpec {
                r#type: ChannelType::Float as i32,
                description: "bbb".to_owned(),
            })
        );

        assert_eq!(
            rr.outputs.get("ost"),
            Some(&ChannelSpec {
                r#type: ChannelType::Int as i32,
                description: "chez".to_owned()
            })
        );
    }

    #[test]
    fn test_attachment_conversion() {
        // Test parsing code and attachments
        let tempd = TempDir::new().unwrap();
        let codepath = tempd.path().join("code");
        std::fs::write(&codepath, "").unwrap();
        let fkalle = NamedTempFile::new().unwrap();
        let foran = NamedTempFile::new().unwrap();

        let toml = format!(
            r#"
        name = "start-blender"
        version = "0.1.0"
        [runtime]
        type = "wasm"

        [code]
        path = "code"
        metadata = {{ is_code = "true" }}
        [code.checksums]
        sha256 = "7767e3afca54296110dd596d8de7cd8adc6f89253beb3c69f0fc810df7f8b6d5"

        [attachments.kalle]
        path = "{}"
        metadata = {{ someMetadata = "sune", cool = "chorizo korvén" }}
        [attachments.kalle.checksums]
        sha256 = "7767e3afca54296110dd596d8de7cd8adc6f89253beb3c69f0fc810df7f8b6d5"

        [attachments.oran]
        path = "{}"
        metadata = {{ surname = "jonsson" }}
        [attachments.oran.checksums]
        sha256 = "7767e3afca54296110dd596d8de7cd8adc6f89253beb3c69f0fc810df7f8b6d5"
        "#,
            fkalle.path().display(),
            foran.path().display(),
        );

        let tomlpath = tempd.path().join("manifest.toml");
        std::fs::write(&tomlpath, toml).unwrap();
        let r = FunctionManifest::parse(tomlpath).unwrap();
        let attachments = r.attachments().unwrap();
        assert_eq!(
            r.code().unwrap().unwrap(),
            AttachmentInfo {
                path: codepath.canonicalize().unwrap(),
                request: attachment_data!("code", "7767e3afca54296110dd596d8de7cd8adc6f89253beb3c69f0fc810df7f8b6d5", {"is_code" => "true"})
            }
        );
        assert_eq!(
            attachments
                .iter()
                .find(|a| a.request.name == "kalle")
                .unwrap(),
            &AttachmentInfo {
                path: fkalle.path().canonicalize().unwrap(),
                request: attachment_data!("kalle", "7767e3afca54296110dd596d8de7cd8adc6f89253beb3c69f0fc810df7f8b6d5", {"someMetadata" => "sune", "cool" => "chorizo korvén"})
            }
        );
        assert_eq!(
            attachments
                .iter()
                .find(|a| a.request.name == "oran")
                .unwrap(),
            &AttachmentInfo {
                path: foran.path().canonicalize().unwrap(),
                request: attachment_data!("oran", "7767e3afca54296110dd596d8de7cd8adc6f89253beb3c69f0fc810df7f8b6d5", {"surname" => "jonsson"})
            }
        );
    }
}
