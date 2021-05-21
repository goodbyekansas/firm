use std::{
    convert::TryFrom,
    path::{Path, PathBuf},
};

use config::{ConfigError, Environment, File};
use serde::Deserialize;

fn default_port() -> u16 {
    1939u16
}

fn create_self_signed_certificate_default() -> bool {
    true
}

fn default_user_and_group() -> String {
    String::from("firm")
}

#[derive(Deserialize)]
struct MaybeCertificateLocations {
    #[serde(default)]
    pub certificate_location: Option<PathBuf>,

    #[serde(default)]
    pub certificate_key_location: Option<PathBuf>,
}

#[derive(Deserialize)]
#[serde(try_from = "MaybeCertificateLocations")]
pub struct CertificateLocations {
    pub cert: PathBuf,
    pub key: PathBuf,
}

impl TryFrom<MaybeCertificateLocations> for CertificateLocations {
    type Error = &'static str;

    fn try_from(value: MaybeCertificateLocations) -> Result<Self, Self::Error> {
        Ok(Self {
            cert: value
                .certificate_location
                .or_else(|| crate::system::get_lomax_cfg_dir().map(|p| p.join("cert.pem")))
                .ok_or("Failed to determine lomax config directory.")?,
            key: value
                .certificate_key_location
                .or_else(|| crate::system::get_lomax_cfg_dir().map(|p| p.join("key.pem")))
                .ok_or("Failed to determine lomax config directory.")?,
        })
    }
}

#[derive(Deserialize)]
pub struct Config {
    #[serde(default = "default_port")]
    pub port: u16,

    #[serde(flatten)]
    pub certificate_locations: CertificateLocations,

    #[serde(default = "create_self_signed_certificate_default")]
    pub create_self_signed_certificate: bool,

    #[serde(default = "default_user_and_group")]
    pub user: String,

    #[serde(default = "default_user_and_group")]
    pub group: String,

    #[serde(default)]
    pub certificate_alt_names: Vec<String>,
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
        } else if let Some(etc_path) =
            crate::system::get_lomax_cfg_dir().map(|p| p.join(DEFAULT_CFG_FILE_NAME))
        {
            if etc_path.exists() {
                c.merge(File::from(etc_path))?;
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
