use std::{
    collections::HashMap,
    convert::TryFrom,
    path::{Path, PathBuf},
};

use config::{ConfigError, Environment, File, FileFormat};
use serde::Deserialize;
use slog::{info, Logger};

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum AuthConfig {
    Oidc {
        provider: String,
    },
    SelfSigned,
    KeyFile {
        /// Path to the private key used to sign the JWT
        path: PathBuf,

        /// Override iss on JWT
        iss: Option<String>,

        /// Override sub on JWT
        sub: Option<String>,

        /// Override aud on JWT
        aud: Option<String>,

        /// Override kid on JWT
        kid: Option<String>,

        /// Override exp on JWT.
        /// Will expire in exp seconds
        exp: Option<usize>,
    },
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum KeyStore {
    Simple { url: String },
    None,
}

impl Default for KeyStore {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct AllowConfig {
    pub users: Vec<String>,
    // TODO for LDAP and google groups: pub groups: Vec<{ name: String, provider: String }>
}

#[derive(Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum IdentityProvider {
    Oidc { provider: String },
    Username,
    UsernameSuffix { suffix: String },
    Override { name: String, email: String },
}

impl Default for IdentityProvider {
    fn default() -> Self {
        Self::Username
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub struct Auth {
    #[serde(default)]
    pub identity: IdentityProvider,

    #[serde(default)]
    pub scopes: HashMap<String, AuthConfig>,

    #[serde(default)]
    pub key_store: KeyStore,

    #[serde(default)]
    pub allow: AllowConfig,
}

// For backwards compatibility, add things to this struct and
// convert it in the try from. TODO: consider removing the
// hosted_domain for next major release.
#[derive(Debug, Deserialize, PartialEq, Clone)]
#[serde(try_from = "OidcProviderConfig")]
pub struct OidcProvider {
    pub discovery_url: String,
    pub client_id: String,
    pub client_secret: String,

    #[serde(default)]
    pub hosted_domains: Vec<String>,
}
#[derive(Debug, Deserialize, PartialEq, Clone)]
pub struct OidcProviderConfig {
    pub discovery_url: String,
    pub client_id: String,
    pub client_secret: String,

    #[serde(default)]
    pub hosted_domain: Option<String>,
    #[serde(default)]
    pub hosted_domains: Vec<String>,
}

impl TryFrom<OidcProviderConfig> for OidcProvider {
    type Error = &'static str;

    fn try_from(value: OidcProviderConfig) -> Result<Self, Self::Error> {
        Ok(Self {
            discovery_url: value.discovery_url,
            client_id: value.client_id,
            client_secret: value.client_secret,
            hosted_domains: value
                .hosted_domain
                .map(|hd| vec![hd])
                .unwrap_or_default()
                .into_iter()
                .chain(value.hosted_domains)
                .collect(),
        })
    }
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
    pub auth: Auth,
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
                hosted_domains: vec![]
            }),
        );
        assert_eq!(
            conf.oidc_providers.get("undermined"),
            Some(&OidcProvider {
                discovery_url: "ocd.something.external".to_owned(),
                client_id: "456def".to_owned(),
                client_secret: "no".to_owned(),
                hosted_domains: vec![]
            }),
        );
    }

    #[test]
    fn oidc_combine_single_multi_hd() {
        let c = Config::new_with_toml_string(
            r#"
[oidc_providers.overmind]
discovery_url="oidc.something.external"
client_id="123abc"
client_secret="weeeooooo"
hosted_domains=["yes.no", "no.yes"]
[oidc_providers.undermined]
discovery_url="ocd.something.external"
client_id="456def"
client_secret="no"
hosted_domain="sula.com"
[oidc_providers.sidemind]
discovery_url="ocd.something.external"
client_id="456def"
client_secret="no"
hosted_domain="sula.com"
hosted_domains=["💈.no", "🌡️.yes"]
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
                hosted_domains: vec![String::from("yes.no"), String::from("no.yes")]
            }),
        );
        assert_eq!(
            conf.oidc_providers.get("undermined"),
            Some(&OidcProvider {
                discovery_url: "ocd.something.external".to_owned(),
                client_id: "456def".to_owned(),
                client_secret: "no".to_owned(),
                hosted_domains: vec![String::from("sula.com")]
            }),
        );
        assert_eq!(
            conf.oidc_providers.get("sidemind"),
            Some(&OidcProvider {
                discovery_url: "ocd.something.external".to_owned(),
                client_id: "456def".to_owned(),
                client_secret: "no".to_owned(),
                hosted_domains: vec![
                    String::from("sula.com"),
                    String::from("💈.no"),
                    String::from("🌡️.yes")
                ]
            }),
        );
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
        identity={type="oidc", provider="auth-inc"}
        key-store={type="simple", url="https://scones.se"}

        [auth.scopes]
        "over-here"={type="oidc", provider="auth-inc"}
        "on-the-internet.com"={type="self-signed"}
        "not-on-www"={type="key-file", path="/tmp/my/file"}

        [auth.allow]
        users=["Kreti", "Pleti"]
        "#,
        );
        assert!(c.is_ok());
        let c = c.unwrap();
        assert_eq!(c.auth.scopes.len(), 3);
        assert!(
            matches!(c.auth.scopes.get("over-here"), Some(AuthConfig::Oidc{provider}) if provider == "auth-inc")
        );
        assert!(matches!(
            c.auth.scopes.get("on-the-internet.com"),
            Some(AuthConfig::SelfSigned)
        ));
        assert!(
            matches!(c.auth.scopes.get("not-on-www"), Some(AuthConfig::KeyFile{ path, .. }) if path == Path::new("/tmp/my/file"))
        );

        assert_eq!(
            c.auth.identity,
            IdentityProvider::Oidc {
                provider: "auth-inc".to_owned()
            }
        );

        assert_eq!(
            c.auth.key_store,
            KeyStore::Simple {
                url: "https://scones.se".to_owned()
            }
        );

        // Test allowed users
        assert_eq!(
            c.auth.allow.users,
            &["Kreti".to_owned(), "Pleti".to_owned()]
        );

        // Empty
        let c = Config::new_with_toml_string("").unwrap();
        assert_eq!(c.auth.identity, IdentityProvider::Username);
        assert_eq!(c.auth.key_store, KeyStore::None);
    }
}
