use std::path::Path;

use config::{ConfigError, Environment, File};
use serde::Deserialize;

fn default_port() -> u16 {
    1939u16
}

#[derive(Deserialize)]
pub struct Config {
    #[serde(default = "default_port")]
    pub port: u16,
}

const DEFAULT_CFG_FILE_NAME: &str = "lomax.toml";
const ENVIRONMENT_PREFIX: &str = "LOMAX";

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
                let etc_path = Path::new("/etc/lomax").join(DEFAULT_CFG_FILE_NAME);
                if etc_path.exists() {
                    c.merge(File::from(etc_path))?;
                }
            }
            #[cfg(windows)]
            {
                if let Some(path) = std::env::var_os("PROGRAMDATA")
                    .map(|appdata| Path::new(&appdata).join(DEFAULT_CFG_FILE_NAME))
                {
                    if path.exists() {
                        c.merge(File::from(path))?;
                    }
                }
            }
        }

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
