use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use thiserror::Error;
use toml;

use crate::proto::{
    ArgumentType, ExecutionEnvironment as ProtoExecutionEnvironment,
    FunctionInput as ProtoFunctionInput, FunctionOutput as ProtoFunctionOutput, RegisterRequest,
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
    inputs: HashMap<String, FunctionInput>,

    #[serde(default)]
    outputs: HashMap<String, FunctionOutput>,

    #[serde(rename = "execution-environment")]
    execution_environment: ExecutionEnvironment,

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
struct ExecutionEnvironment {
    r#type: String,
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
            tags: fm.tags.clone(),
            inputs: fm
                .inputs
                .iter()
                .map(|(name, input)| ProtoFunctionInput {
                    name: name.clone(),
                    required: input.required,
                    r#type: ArgumentType::from(&input.r#type) as i32,
                    default_value: input.default_value.clone(),
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
            entrypoint: String::new(),
            execution_environment: Some(ProtoExecutionEnvironment {
                name: fm.execution_environment.r#type.clone(),
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
        "#;
        let r = FunctionManifest::parse(write_toml_to_tempfile!(toml));
        assert!(r.is_ok());

        let r = FunctionManifest::parse(Path::new(""));
        assert!(r.is_err());
        assert!(matches!(
            r.unwrap_err(),
            ManifestError::ManifestFileReadError{ .. }
        ));
    }
}
