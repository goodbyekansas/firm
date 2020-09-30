use config::{Config, ConfigError, Environment};
use futures::executor::block_on;
use regex::Regex;
use serde::Deserialize;
use thiserror::Error;

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
        let mut c: Configuration = s.try_into()?;
        let secret_resolvers: &[&dyn SecretResolver] = &[&GcpSecretResolver {}];

        c.functions_storage_uri = resolve_secrets(c.functions_storage_uri, secret_resolvers)
            .map_err(|e| ConfigError::Foreign(Box::new(e)))?;
        c.attachment_storage_uri = resolve_secrets(c.attachment_storage_uri, secret_resolvers)
            .map_err(|e| ConfigError::Foreign(Box::new(e)))?;
        Ok(c)
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
) -> Result<String, SecretResolveError> {
    let reg = Regex::new(r"\{\{\s*(?P<type>\w+):(?P<value>\w+)\s*\}\}")
        .expect("Regex was invalid for resolving secrets.");

    reg.captures_iter(content.as_ref())
        .try_fold(content.as_ref().to_owned(), |acc, captures| {
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
                        .map(|resolver| (v, resolver))
                })
                .and_then(|(v, resolver)| resolver.resolve(v.as_str()))
                .map(|real_value| reg.replace(&acc, real_value.as_str()).to_string())
        })
}

trait SecretResolver {
    fn resolve(&self, content: &str) -> Result<String, SecretResolveError>;
    fn prefix(&self) -> &'static str;
}

struct GcpSecretResolver {}

macro_rules! create_resolve_error {
    ($message: expr, $value: ident, $type: ident) => {
        SecretResolveError::FailedToResolveValue {
            value: $value.to_owned(),
            type_: $type.prefix().to_owned(),
            message: String::from($message),
        }
    };
}

impl SecretResolver for GcpSecretResolver {
    fn resolve(&self, content: &str) -> Result<String, SecretResolveError> {
        block_on(
            reqwest::Client::new()
                .get(
                    reqwest::Url::parse(&format!(
                        "https://secretmanager.googleapis.com/v1/{}:access",
                        content
                    ))
                    .map_err(|e| {
                        create_resolve_error!(
                            format!("Failed parse gcp secrets url: {}", e),
                            content,
                            self
                        )
                    })?,
                )
                .send(),
        )
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
            json.get("data")
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

    struct MockResolver {
        secrets: HashMap<String, String>,
    }

    impl SecretResolver for MockResolver {
        fn resolve(&self, content: &str) -> Result<String, SecretResolveError> {
            self.secrets.get(content).map(|s| s.clone()).ok_or_else(|| {
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
    fn test_not_finding_resolver() {
        let res = resolve_secrets("Something{{mock:bune}}", &[]);
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
        );
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            SecretResolveError::FailedToFindResolver(..)
        ));
    }

    #[test]
    fn test_resolver() {
        // Test single
        let mut secrets = HashMap::new();
        secrets.insert(String::from("bune"), String::from("mega-secret"));
        let res = resolve_secrets("Something={{ mock:bune }}", &[&MockResolver { secrets }]);

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
        );

        assert!(res.is_ok());
        let content = res.unwrap();
        assert_eq!(content, "Something=mega-secret:ryck=bad-secret");

        // test same value used twice
        let res = resolve_secrets(
            "Something={{ mock:first }}:ryck={{ mock:second}}&sule={{mock:first  }}",
            &[&MockResolver { secrets }],
        );

        assert!(res.is_ok());
        let content = res.unwrap();
        assert_eq!(
            content,
            "Something=mega-secret:ryck=bad-secret&sule=mega-secret"
        );
    }

    #[test]
    fn test_failing_resolving_value() {
        let res = resolve_secrets("Something={{ mock:bune }}", &[&FailingMockResolver {}]);

        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            SecretResolveError::FailedToResolveValue {..}
        ));
    }
}
