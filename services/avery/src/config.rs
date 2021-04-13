use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use config::{ConfigError, Environment, File, FileFormat};
use serde::Deserialize;

#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Auth {
    None,
    Oidc { provider: String },
    SelfSigned,
    KeyFile { path: PathBuf },
}

#[derive(Debug, Deserialize, PartialEq, Clone)]
pub struct OidcProvider {
    pub discovery_url: String,
    pub client_id: String,
    pub client_secret: String,

    #[serde(default)]
    pub hosted_domain: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub registries: Vec<Registry>,

    #[serde(default)]
    pub conflict_resolution: ConflictResolutionMethod,

    #[serde(default)]
    pub internal_registry: InternalRegistryConfig,

    #[serde(default)]
    pub runtime_directories: Vec<PathBuf>,

    #[serde(default)]
    pub oidc_providers: HashMap<String, OidcProvider>,

    #[serde(default)]
    pub auth: HashMap<String, Auth>,
}

fn default_version_suffix() -> String {
    String::from("dev")
}

#[derive(Debug, Deserialize, Clone)]
pub struct InternalRegistryConfig {
    #[serde(default = "default_version_suffix")]
    pub version_suffix: String,
}

impl Default for InternalRegistryConfig {
    fn default() -> Self {
        Self {
            version_suffix: default_version_suffix(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub enum ConflictResolutionMethod {
    Error,
    UsePriority,
}

impl Default for ConflictResolutionMethod {
    fn default() -> Self {
        ConflictResolutionMethod::UsePriority
    }
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
pub struct Registry {
    pub name: String,
    pub url: String,
}

const DEFAULT_CFG_FILE_NAME: &str = "avery.toml";
const ENVIRONMENT_PREFIX: &str = "AVERY";

impl Config {
    pub fn new() -> Result<Self, ConfigError> {
        let mut c = config::Config::new();

        // try some default config file locations
        let current_folder_cfg = Path::new(DEFAULT_CFG_FILE_NAME);
        if current_folder_cfg.exists() {
            c.merge(File::from(current_folder_cfg))?;
        } else {
            #[cfg(unix)]
            {
                let etc_path = Path::new("/etc/avery").join(DEFAULT_CFG_FILE_NAME);
                if etc_path.exists() {
                    c.merge(File::from(etc_path))?;
                }
            }
        }

        c.merge(Environment::with_prefix(ENVIRONMENT_PREFIX))?;

        c.try_into()
    }

    #[allow(dead_code)]
    pub fn new_with_toml_string<S: AsRef<str>>(cfg: S) -> Result<Self, ConfigError> {
        let mut c = config::Config::new();
        c.merge(File::from_str(cfg.as_ref(), FileFormat::Toml))?;
        c.merge(Environment::with_prefix(ENVIRONMENT_PREFIX))?;

        c.try_into()
    }

    pub fn new_with_file<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let mut c = config::Config::new();
        c.merge(File::from(path.as_ref()))?;
        c.merge(Environment::with_prefix(ENVIRONMENT_PREFIX))?;

        c.try_into()
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn empty() {
        let c = Config::new_with_toml_string("");
        assert!(c.is_ok());
    }

    #[test]
    fn oidc_providers() {
        let c = Config::new_with_toml_string(
            r#"
[oidc_providers.overmind]
discovery_url="oidc.something.external"
client_id="123abc"
client_secret="weeeooooo"
[oidc_providers.undermined]
discovery_url="ocd.something.external"
client_id="456def"
client_secret="no"
"#,
        );
        assert!(c.is_ok());

        let conf = c.unwrap();
        assert_eq!(
            conf.oidc_providers.get("overmind"),
            Some(&OidcProvider {
                discovery_url: "oidc.something.external".to_owned(),
                client_id: "123abc".to_owned(),
                client_secret: "weeeooooo".to_owned(),
                hosted_domain: None
            }),
        );
        assert_eq!(
            conf.oidc_providers.get("undermined"),
            Some(&OidcProvider {
                discovery_url: "ocd.something.external".to_owned(),
                client_id: "456def".to_owned(),
                client_secret: "no".to_owned(),
                hosted_domain: None
            }),
        );
    }

    #[test]
    fn registries() {
        // Test registries in config, make sure oath_scope is optional
        let c = Config::new_with_toml_string(
            r#"
        [[registries]]
        name="registry1"
        url="https://over-here"

        [[registries]]
        name="registry3"
        url="https://on-the-internet.com"
        "#,
        );
        assert!(c.is_ok());
        assert_eq!(
            c.unwrap().registries,
            vec![
                Registry {
                    name: "registry1".to_owned(),
                    url: "https://over-here".to_owned(),
                },
                Registry {
                    name: "registry3".to_owned(),
                    url: "https://on-the-internet.com".to_owned(),
                }
            ]
        );
        // Non optional things should cause error if missing
        let c = Config::new_with_toml_string(
            r#"
        [[registries]]
        name="registry-without-url"

        [[registries]]
        name="registry3"
        url="https://on-the-internet.com"
        "#,
        );
        assert!(c.is_err());
    }

    #[test]
    fn auth() {
        let c = Config::new_with_toml_string(
            r#"
        [[registries]]
        name="registry1"
        url="https://over-here"

        [[registries]]
        name="registry3"
        url="https://on-the-internet.com"

        [auth]
        "over-here"={type="oidc", provider="auth-inc"}
        "on-the-internet.com"={type="self-signed"}
        "not-on-www"={type="key-file", path="/tmp/my/file"}
        "#,
        );
        assert!(c.is_ok());
        let c = c.unwrap();
        assert_eq!(c.auth.len(), 3);
        assert!(
            matches!(c.auth.get("over-here"), Some(Auth::Oidc{provider}) if provider == "auth-inc")
        );
        assert!(matches!(
            c.auth.get("on-the-internet.com"),
            Some(Auth::SelfSigned)
        ));
        assert!(
            matches!(c.auth.get("not-on-www"), Some(Auth::KeyFile{path}) if path == Path::new("/tmp/my/file"))
        );
    }
}
