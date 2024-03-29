use config::{Config, ConfigError, Environment};
use futures::executor::block_on;
use regex::Regex;
use serde::Deserialize;
use slog::{info, o, Logger};
use thiserror::Error;

fn default_port() -> u64 {
    50051
}

fn default_storage_uri() -> String {
    String::from("memory://")
}

#[derive(Debug, Deserialize)]
pub struct Configuration {
    #[serde(default = "default_storage_uri")]
    pub functions_storage_uri: String,

    #[serde(default = "default_port")]
    pub port: u64,

    pub attachment_storage_uri: String,
}

impl Configuration {
    pub async fn new(log: Logger) -> Result<Self, ConfigError> {
        Self::new_with_init(log, Config::default()).await
    }

    pub async fn new_with_init<T>(log: Logger, init: T) -> Result<Self, ConfigError>
    where
        T: config::Source + Send + Sync + 'static,
    {
        let mut config: Self = Config::builder()
            .add_source(init)
            .add_source(Environment::with_prefix("REGISTRY").try_parsing(true))
            .build()?
            .try_deserialize()?;

        let secret_resolvers: &[&dyn SecretResolver] = &[&GcpSecretResolver::new(
            log.new(o!("scope" => "secret-resolver", "type" => "gcp")),
        )];

        config.functions_storage_uri = resolve_secrets(
            &config.functions_storage_uri,
            secret_resolvers,
            true,
            log.new(o!("scope" => "resolve-secret", "field" => "functions_storage_uri", "uri" => config.functions_storage_uri.clone())),
        )
        .map_err(|e| ConfigError::Foreign(Box::new(e)))?;

        config.attachment_storage_uri = resolve_secrets(
            &config.attachment_storage_uri,
            secret_resolvers,
            true,
            log.new(o!("scope" => "resolve-secret", "field" => "attachment_storage_uri", "uri" => config.attachment_storage_uri.clone())),
        )
        .map_err(|e| ConfigError::Foreign(Box::new(e)))?;

        Ok(config)
    }
}

#[derive(Error, Debug)]
pub enum SecretResolveError {
    #[error("Secret resolve error: {0}")]
    Generic(String),

    #[error("Failed to find resolver of type \"{0}\"")]
    FailedToFindResolver(String),

    #[error("Failed to resolve secret value \"{value}\" with type \"{type_}\": {message}")]
    FailedToResolveValue {
        value: String,
        type_: String,
        message: String,
    },
}

fn resolve_secrets<S: AsRef<str>>(
    content: S,
    resolvers: &[&dyn SecretResolver],
    is_uri: bool,
    log: Logger,
) -> Result<String, SecretResolveError> {
    // woho I'm in the regex \&/ #Solaire
    let reg = Regex::new(r"\{\{\s*(?P<type>\w+):(?P<value>[\w\.\-\?\&/_=]+)\s*\}\}")
        .expect("Regex was invalid for resolving secrets.");

    let res = reg.captures_iter(content.as_ref()).try_fold(
        content.as_ref().to_owned(),
        |acc, captures| {
            captures
                .name("type")
                .and_then(|t| captures.name("value").map(|v| (t, v)))
                .ok_or_else(|| {
                    SecretResolveError::Generic(
                        "Failed to get type or value from match group.".to_owned(),
                    )
                })
                .and_then(|(t, v)| {
                    resolvers
                        .iter()
                        .find(|resolver| resolver.prefix() == t.as_str())
                        .ok_or_else(|| {
                            SecretResolveError::FailedToFindResolver(t.as_str().to_owned())
                        })
                        .map(|resolver| {
                            info!(
                                log,
                                "found resolver for secret {} with type {}",
                                v.as_str(),
                                t.as_str()
                            );
                            (v, resolver)
                        })
                })
                .and_then(|(v, resolver)| resolver.resolve(v.as_str()))
                .map(|real_value| {
                    info!(log, "successfully resolved secret");
                    let real_value: String = if is_uri {
                        url::form_urlencoded::byte_serialize(real_value.as_bytes()).collect()
                    } else {
                        real_value
                    };
                    reg.replace(&acc, real_value.as_str()).to_string()
                })
        },
    );

    res
}

macro_rules! create_resolve_error {
    ($message: expr, $value: expr, $type: ident) => {
        SecretResolveError::FailedToResolveValue {
            value: $value.to_owned(),
            type_: $type.prefix().to_owned(),
            message: String::from($message),
        }
    };
}

trait SecretResolver {
    fn resolve(&self, content: &str) -> Result<String, SecretResolveError>;
    fn prefix(&self) -> &'static str;
}

struct GcpSecretResolver {
    log: Logger,
}

impl GcpSecretResolver {
    fn new(log: Logger) -> Self {
        Self { log }
    }

    fn get_access_token(&self) -> Result<String, SecretResolveError> {
        let url = "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token?scopes=https://www.googleapis.com/auth/cloud-platform";
        block_on({
                info!(self.log, "Fetching auth token from url: {}", url);
                reqwest::Client::builder()
                    .connect_timeout(std::time::Duration::from_secs(10))
                    .build()
                    .map_err(|e| {
                        create_resolve_error!(
                            format!("Failed to create client to get gcp auth token: {}", e),
                            String::new(),
                            self
                        )
                    })?
                    .get(reqwest::Url::parse(url).map_err(|e| {
                        create_resolve_error!(
                            format!("Failed parse gcp auth token url: {}", e),
                            String::new(),
                            self
                        )
                    })?)
                    .header("Metadata-Flavor", "Google")
                    .timeout(std::time::Duration::from_secs(10))
                    .send()
            })
            .and_then(|response| response.error_for_status())
            .map_err(|e| {
                create_resolve_error!(
                    format!("Failed to obtain auth token from google apis: {}", e),
                    String::new(),
                    self
                )
            })
            .and_then(|response| {
                block_on(response.json::<serde_json::Value>()).map_err(|e| {
                    create_resolve_error!(
                        format!(
                            "Failed to parse auth json payload from google apis: {}",
                            e
                        ),
                        String::new(),
                        self
                    )
                })
            })
            .and_then(|json| {
                json.get("access_token")
                    .ok_or_else(|| {
                        create_resolve_error!(
                            "Failed to get data field from google apis auth request",
                            String::new(),
                            self
                        )
                    })
                    .map(|j| j.clone())
            })
            .and_then(|data| {
                data.as_str()
                    .ok_or_else(|| {
                        create_resolve_error!(
                            "Failed to get access_token field from google apis auth token request to string",
                            String::new(),
                            self
                        )
                    })
                    .map(|d| d.to_owned())
            })
    }
}

impl SecretResolver for GcpSecretResolver {
    fn resolve(&self, content: &str) -> Result<String, SecretResolveError> {
        let url = &format!("https://secretmanager.googleapis.com/v1/{}:access", content);
        let access_token = self.get_access_token()?;
        block_on({
            info!(self.log, "Fetching secret from url: {}", url);
            reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .build()
                .map_err(|e| {
                    create_resolve_error!(
                        format!("Failed to create client to get gcp secret: {}", e),
                        content,
                        self
                    )
                })?
                .get(reqwest::Url::parse(url).map_err(|e| {
                    create_resolve_error!(
                        format!("Failed parse gcp secrets url: {}", e),
                        content,
                        self
                    )
                })?)
                .header("Authorization", format!("Bearer {}", access_token)) // 🐻
                .timeout(std::time::Duration::from_secs(10))
                .send()
        })
        .and_then(|response| response.error_for_status())
        .map_err(|e| {
            create_resolve_error!(
                format!("Failed to obtain secret from google apis: {}", e),
                content,
                self
            )
        })
        .and_then(|response| {
            block_on(response.json::<serde_json::Value>()).map_err(|e| {
                create_resolve_error!(
                    format!(
                        "Failed to parse secret json payload from google apis: {}",
                        e
                    ),
                    content,
                    self
                )
            })
        })
        .and_then(|json| {
            json.get("payload")
                .ok_or_else(|| {
                    create_resolve_error!(
                        "Failed to get payload field from google apis secret request",
                        content,
                        self
                    )
                })?
                .get("data")
                .ok_or_else(|| {
                    create_resolve_error!(
                        "Failed to get data field from google apis secret request",
                        content,
                        self
                    )
                })
                .map(|j| j.clone())
        })
        .and_then(|data| {
            data.as_str()
                .ok_or_else(|| {
                    create_resolve_error!(
                        "Failed to get data field from google apis secret request to string",
                        content,
                        self
                    )
                })
                .map(|d| d.to_owned())
        })
        .and_then(|data| {
            base64::decode(data).map_err(|e| {
                create_resolve_error!(
                    format!(
                        "Failed to base64 decode secret from google apis secret request: {}",
                        e
                    ),
                    content,
                    self
                )
            })
        })
        .and_then(|base64_content| {
            String::from_utf8(base64_content).map_err(|e| {
                create_resolve_error!(
                    format!("Failed to parse base64 content secret as string: {}", e),
                    content,
                    self
                )
            })
        })
    }

    fn prefix(&self) -> &'static str {
        "gcp"
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    macro_rules! null_logger {
        () => {{
            slog::Logger::root(slog::Discard, slog::o!())
        }};
    }

    struct MockResolver {
        secrets: HashMap<String, String>,
    }

    impl SecretResolver for MockResolver {
        fn resolve(&self, content: &str) -> Result<String, SecretResolveError> {
            self.secrets.get(content).cloned().ok_or_else(|| {
                SecretResolveError::FailedToResolveValue {
                    value: String::from(content),
                    type_: self.prefix().to_owned(),
                    message: format!("Failed to find key {} in secrets hashmap", content),
                }
            })
        }

        fn prefix(&self) -> &'static str {
            "mock"
        }
    }

    struct FailingMockResolver {}
    impl SecretResolver for FailingMockResolver {
        fn resolve(&self, content: &str) -> Result<String, SecretResolveError> {
            Err(SecretResolveError::FailedToResolveValue {
                value: content.to_owned(),
                type_: self.prefix().to_owned(),
                message: "I explodded".to_owned(),
            })
        }

        fn prefix(&self) -> &'static str {
            "mock"
        }
    }

    #[test]
    fn not_finding_resolver() {
        let res = resolve_secrets("Something{{mock:bune}}", &[], false, null_logger!());
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            SecretResolveError::FailedToFindResolver(..)
        ));

        // Test when having something in the resolver list
        let res = resolve_secrets(
            "Something{{ rune:bune }}",
            &[&MockResolver {
                secrets: HashMap::new(),
            }],
            false,
            null_logger!(),
        );
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            SecretResolveError::FailedToFindResolver(..)
        ));
    }

    #[test]
    fn resolve_secrets_uri() {
        let mut secrets = HashMap::new();
        secrets.insert(
            String::from("i_has_uri_escape_stuff"),
            String::from("__@__LOL!__%__:__"),
        );
        let res = resolve_secrets(
            "KalleSula{{mock:i_has_uri_escape_stuff}}",
            &[&MockResolver {
                secrets: secrets.clone(),
            }],
            true,
            null_logger!(),
        )
        .unwrap();
        assert_eq!(res, "KalleSula__%40__LOL%21__%25__%3A__");

        let res = resolve_secrets(
            "KalleSula{{mock:i_has_uri_escape_stuff}}",
            &[&MockResolver { secrets }],
            false,
            null_logger!(),
        )
        .unwrap();
        assert_eq!(res, "KalleSula__@__LOL!__%__:__");
    }

    #[test]
    fn resolver() {
        // Test single
        let mut secrets = HashMap::new();
        secrets.insert(String::from("bune"), String::from("mega-secret"));
        let res = resolve_secrets(
            "Something={{ mock:bune }}",
            &[&MockResolver { secrets }],
            false,
            null_logger!(),
        );

        assert!(res.is_ok());
        let content = res.unwrap();
        assert_eq!(content, "Something=mega-secret");

        // Test multiple
        let mut secrets = HashMap::new();
        secrets.insert(String::from("first"), String::from("mega-secret"));
        secrets.insert(String::from("second"), String::from("bad-secret"));
        let res = resolve_secrets(
            "Something={{ mock:first }}:ryck={{mock:second}}",
            &[&MockResolver {
                secrets: secrets.clone(),
            }],
            false,
            null_logger!(),
        );

        assert!(res.is_ok());
        let content = res.unwrap();
        assert_eq!(content, "Something=mega-secret:ryck=bad-secret");

        // newlines in string
        let res = resolve_secrets(
            r#"Something={{
              mock:first
            }}:ryck={{mock:second}}"#,
            &[&MockResolver {
                secrets: secrets.clone(),
            }],
            false,
            null_logger!(),
        );

        assert!(res.is_ok());
        let content = res.unwrap();
        assert_eq!(content, "Something=mega-secret:ryck=bad-secret");

        // test same value used twice
        let res = resolve_secrets(
            "Something={{ mock:first }}:ryck={{ mock:second}}&sule={{mock:first  }}",
            &[&MockResolver {
                secrets: secrets.clone(),
            }],
            false,
            null_logger!(),
        );

        assert!(res.is_ok());
        let content = res.unwrap();
        assert_eq!(
            content,
            "Something=mega-secret:ryck=bad-secret&sule=mega-secret"
        );

        secrets.insert(
            String::from("test/some-chars/1234_check.this"),
            String::from("cardboard"),
        );
        let res = resolve_secrets(
            "Something={{ mock:test/some-chars/1234_check.this }}:ryck={{ mock:second}}&sule={{mock:first  }}",
            &[&MockResolver { secrets }],
            false,
            null_logger!(),
        );
        let content = res.unwrap();
        assert_eq!(
            content,
            "Something=cardboard:ryck=bad-secret&sule=mega-secret"
        );
    }

    #[test]
    fn failing_resolving_value() {
        let res = resolve_secrets(
            "Something={{ mock:bune }}",
            &[&FailingMockResolver {}],
            false,
            null_logger!(),
        );

        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            SecretResolveError::FailedToResolveValue { .. }
        ));
    }
}
