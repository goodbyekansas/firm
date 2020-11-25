use std::path::Path;

use config::{ConfigError, Environment, File, FileFormat};
use serde::Deserialize;

fn default_port() -> u64 {
    1939
}

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default = "default_port")]
    pub port: u64,

    #[serde(default)]
    pub registries: Vec<Registry>,
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
pub struct Registry {
    pub name: String,
    pub url: String,
    pub oauth_scope: Option<String>,
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
    fn defaults() {
        let c = Config::new_with_toml_string("");

        assert!(c.is_ok());
        assert_eq!(c.unwrap().port, default_port());
    }

    #[test]
    fn ports() {
        // Read the port 🛳️
        let c = Config::new_with_toml_string(
            r#"
        port=1337
        "#,
        );
        assert!(c.is_ok());
        assert_eq!(c.unwrap().port, 1337);
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
        oauth_scope="everything"
        "#,
        );
        assert!(c.is_ok());
        assert_eq!(
            c.unwrap().registries,
            vec![
                Registry {
                    name: "registry1".to_owned(),
                    url: "https://over-here".to_owned(),
                    oauth_scope: None
                },
                Registry {
                    name: "registry3".to_owned(),
                    url: "https://on-the-internet.com".to_owned(),
                    oauth_scope: Some("everything".to_owned())
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
        oauth_scope="everything"
        "#,
        );
        assert!(c.is_err());
    }
}
