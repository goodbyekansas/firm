use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use thiserror::Error;

use crate::proto::{
    ArgumentType, Checksums as ProtoChecksums, ExecutionEnvironment as ProtoExecutionEnvironment,
    FunctionInput as ProtoFunctionInput, FunctionOutput as ProtoFunctionOutput, RegisterRequest, FunctionArgument
};

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("Failed to parse function manifest: {0}")]
    ManifestParseError(#[from] toml::de::Error),

    #[error("Manifest file \"{path}\" could not be read: {io_error}")]
    ManifestFileReadError { path: PathBuf, io_error: io::Error },
}

#[derive(Debug, Deserialize)]
enum FunctionArgumentType {
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

    #[serde(default)]
    version: String,

    #[serde(default)]
    inputs: HashMap<String, FunctionInput>,

    #[serde(default)]
    outputs: HashMap<String, FunctionOutput>,

    #[serde(rename = "execution-environment")]
    execution_environment: ExecutionEnvironment,

    checksums: Checksums,

    #[serde(default)]
    tags: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct FunctionInput {
    r#type: FunctionArgumentType,

    #[serde(default)]
    required: bool,

    #[serde(default, rename = "default")]
    default_value: String,
}

#[derive(Debug, Deserialize)]
struct FunctionOutput {
    r#type: FunctionArgumentType,
}

#[derive(Debug, Deserialize)]
struct Checksums {
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct ExecutionEnvironment {
    r#type: String,

    #[serde(default)]
    entrypoint: String,

    #[serde(default)]
    args: HashMap<String, String>,
}

impl FunctionManifest {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn parse<P: AsRef<Path>>(path: P) -> Result<Self, ManifestError> {
        std::fs::read_to_string(path.as_ref())
            .map_err(|e| ManifestError::ManifestFileReadError {
                path: path.as_ref().to_path_buf(),
                io_error: e,
            })
            .and_then(|toml_content| toml::from_str(&toml_content).map_err(|e| e.into()))
    }
}

impl From<&FunctionManifest> for RegisterRequest {
    fn from(fm: &FunctionManifest) -> Self {
        RegisterRequest {
            name: fm.name.clone(),
            checksums: Some(ProtoChecksums {
                sha256: fm.checksums.sha256.clone(),
            }),
            version: fm.version.clone(),
            tags: fm.tags.clone(),
            inputs: fm
                .inputs
                .iter()
                .map(|(name, input)| ProtoFunctionInput {
                    name: name.clone(),
                    required: input.required,
                    r#type: ArgumentType::from(&input.r#type) as i32,
                    default_value: input.default_value.clone(),
                    from_execution_environment: false, // This is weird. This should only be set from avery
                })
                .collect(),
            outputs: fm
                .outputs
                .iter()
                .map(|(name, output)| ProtoFunctionOutput {
                    name: name.clone(),
                    r#type: ArgumentType::from(&output.r#type) as i32,
                })
                .collect(),
            code: vec![],
            execution_environment: Some(ProtoExecutionEnvironment {
                name: fm.execution_environment.r#type.clone(),
                args: fm.execution_environment.args.iter()
                    .map(|(k, v)| FunctionArgument {
                        name: k.to_owned(),
                        r#type: ArgumentType::String as i32,
                        value: v.as_bytes().to_vec(),
                    })
                    .collect(),
                entrypoint: fm.execution_environment.entrypoint.clone(),
            }),
        }
    }
}

// this is here to get compile time checks that the two enum types
// are identical
impl From<ArgumentType> for FunctionArgumentType {
    fn from(at: ArgumentType) -> Self {
        match at {
            ArgumentType::String => FunctionArgumentType::String,
            ArgumentType::Bool => FunctionArgumentType::Bool,
            ArgumentType::Int => FunctionArgumentType::Int,
            ArgumentType::Float => FunctionArgumentType::Float,
            ArgumentType::Bytes => FunctionArgumentType::Bytes,
        }
    }
}

impl From<&FunctionArgumentType> for ArgumentType {
    fn from(at: &FunctionArgumentType) -> Self {
        match *at {
            FunctionArgumentType::String => ArgumentType::String,
            FunctionArgumentType::Bool => ArgumentType::Bool,
            FunctionArgumentType::Int => ArgumentType::Int,
            FunctionArgumentType::Float => ArgumentType::Float,
            FunctionArgumentType::Bytes => ArgumentType::Bytes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Write;

    use tempfile::NamedTempFile;

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
        [execution-environment]
        type = "wasm"

        [checksums]
        sha256 = "724a8940e46ffa34e930258f708d890dbb3b3243361dfbc41eefcff124407a29"
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

        [execution-environment]
        type = "wasm"

        [checksums]
        sha256 = "724a8940e46ffa34e930258f708d890dbb3b3243361dfbc41eefcff124407a29"
        "#;
        let r = FunctionManifest::parse(write_toml_to_tempfile!(toml));
        assert!(r.is_err());
        assert!(matches!(
            r.unwrap_err(),
            ManifestError::ManifestParseError(_)
        ));

        let toml = r#"
        name = "start-blender"

        [inputs]
          [inputs.version]
          type = "string"

        [outputs]
          [outputs.pid]
          type="int"

        [execution-environment]
        type = "wasm"

        [execution-environment.args]
        sune = "bune"

        [checksums]
        sha256 = "724a8940e46ffa34e930258f708d890dbb3b3243361dfbc41eefcff124407a29"
        "#;
        let r = FunctionManifest::parse(write_toml_to_tempfile!(toml));
        assert!(r.is_ok());
        assert_eq!(r.unwrap().execution_environment.args["sune"], "bune");

        let r = FunctionManifest::parse(Path::new(""));
        assert!(r.is_err());
        assert!(matches!(
            r.unwrap_err(),
            ManifestError::ManifestFileReadError{ .. }
        ));
    }
}