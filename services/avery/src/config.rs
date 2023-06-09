use std::path::{Path, PathBuf};

use config::{ConfigError, Environment, File, FileFormat};
use serde::Deserialize;
use slog::{info, Logger};

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub registries: Vec<Registry>,

    #[serde(default)]
    pub conflict_resolution: ConflictResolutionMethod,

    #[serde(default)]
    pub runtime_directories: Vec<PathBuf>,
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
    pub fn new(logger: Logger) -> Result<Self, ConfigError> {
        let current_folder_cfg = Path::new(DEFAULT_CFG_FILE_NAME);
        current_folder_cfg
            .exists()
            .then(|| {
                // try some default config file locations
                info!(
                    logger,
                    "📒 Reading configuration file: \"./{}\"",
                    current_folder_cfg.display()
                );
                config::Config::builder().add_source(File::from(current_folder_cfg))
            })
            .or_else(|| {
                // first, a global config file
                let builder = crate::system::global_config_path()
                    .and_then(|global_cfg_path| {
                        let path = global_cfg_path.join(DEFAULT_CFG_FILE_NAME);
                        path.exists().then(|| {
                            info!(
                                logger,
                                "📒 Reading configuration file: \"{}\"",
                                path.display()
                            );
                            config::Config::builder().add_source(File::from(path))
                        })
                    })
                    .unwrap_or_else(config::Config::builder);

                // overridden by a local one
                crate::system::user_config_path().and_then(|user_cfg_path| {
                    let path = user_cfg_path.join(DEFAULT_CFG_FILE_NAME);
                    path.exists().then(|| {
                        info!(
                            logger,
                            "📒 Reading user configuration overrides. File: \"{}\"",
                            path.display()
                        );
                        builder.add_source(File::from(path))
                    })
                })
            })
            .unwrap_or_else(config::Config::builder)
            .add_source(Environment::with_prefix(ENVIRONMENT_PREFIX))
            .build()?
            .try_deserialize()
    }

    #[allow(dead_code)]
    pub fn new_with_toml_string<S: AsRef<str>>(cfg: S) -> Result<Self, ConfigError> {
        config::Config::builder()
            .add_source(File::from_str(cfg.as_ref(), FileFormat::Toml))
            .add_source(Environment::with_prefix(ENVIRONMENT_PREFIX))
            .build()?
            .try_deserialize()
    }

    pub fn new_with_file<P: AsRef<Path>>(path: P, logger: Logger) -> Result<Self, ConfigError> {
        info!(
            logger,
            "📒 Reading configuration from file: \"{}\"",
            path.as_ref().display(),
        );

        config::Config::builder()
            .add_source(File::from(path.as_ref()))
            .add_source(Environment::with_prefix(ENVIRONMENT_PREFIX))
            .build()?
            .try_deserialize()
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
    fn registries() {
        // Test registries in config, make sure oauth_scope is optional
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
}
