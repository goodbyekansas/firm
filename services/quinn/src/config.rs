use config::{Config, Environment};
use serde::Deserialize;

fn default_port() -> u64 {
    50051
}

fn default_storage_uri() -> String {
    String::from("memory://")
}

fn default_attachment_storage_uri() -> String {
    String::from("gcs://default-bucket")
}

#[derive(Debug, Deserialize)]
pub struct Configuration {
    #[serde(default = "default_storage_uri")]
    pub functions_storage_uri: String,

    #[serde(default = "default_port")]
    pub port: u64,

    #[serde(default = "default_attachment_storage_uri")]
    pub attachment_storage_uri: String,
}

impl Configuration {
    pub fn new() -> Result<Self, config::ConfigError> {
        let mut s = Config::new();
        s.merge(Environment::with_prefix("REGISTRY").separator("__"))?;
        s.try_into()
    }
}
