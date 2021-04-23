use firm_types::{
    auth::{authentication_server::Authentication, AcquireTokenParameters},
    tonic,
};
use futures::TryFutureExt;
use serde::Deserialize;
use slog::{debug, Logger};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum KeyStoreError {
    #[error("Error: {0}")]
    Error(String),

    #[error("Remote error: {0}")]
    RemoteError(String),

    #[error("Failed to authenticate key store request: {0}")]
    AuthenticationError(String),
}

#[async_trait::async_trait]
pub trait KeyStore: Send + Sync {
    async fn get(&self, id: &str) -> Result<Vec<u8>, KeyStoreError>;
    async fn set(&self, id: &str, key_data: &[u8]) -> Result<(), KeyStoreError>;
}

#[derive(Deserialize)]
struct PublicKey {
    public_key: String,
}

#[derive(Clone)]
pub struct SimpleKeyStore {
    url: String,
    authentication: super::AuthService,
    logger: Logger,
}

impl SimpleKeyStore {
    pub fn new(url: &str, authentication: super::AuthService, logger: Logger) -> Self {
        Self {
            url: url.to_owned(),
            authentication,
            logger,
        }
    }
}

#[async_trait::async_trait]
impl KeyStore for SimpleKeyStore {
    async fn get(&self, id: &str) -> Result<Vec<u8>, KeyStoreError> {
        futures::future::ready(
            url::Url::parse(&self.url)
                .and_then(|url| url.join(id))
                .map_err(|e| KeyStoreError::Error(format!("Failed to parse url: {}", e))),
        )
        .and_then(|url| {
            futures::future::ready(
                url.host()
                    .ok_or_else(|| {
                        KeyStoreError::AuthenticationError(format!(
                            "Url \"{}\" is missing hostname.",
                            url.to_string(),
                        ))
                    })
                    .map(|host| host.to_string()),
            )
            .and_then(|scope| {
                debug!(
                    self.logger,
                    "Acquiring token for scope \"{}\" when uploading key", &scope
                );

                self.authentication
                    .acquire_token(tonic::Request::new(AcquireTokenParameters {
                        scope: scope.clone(),
                    }))
                    .map_err(move |e| {
                        KeyStoreError::AuthenticationError(format!(
                            "Failed to acquire token for scope {} when uploading key: {}",
                            scope, e
                        ))
                    })
                    .map_ok(|token| (url, token.into_inner().token))
            })
        })
        .and_then(|(url, token)| {
            reqwest::Client::new()
                .get(url.to_string())
                .bearer_auth(token)
                .send()
                .map_err(|e| {
                    KeyStoreError::RemoteError(format!("Failed to get key store response: {}", e))
                })
                .and_then(|response: reqwest::Response| async move {
                    response.error_for_status().map_err(|e| {
                        KeyStoreError::RemoteError(format!("Received response with error: {}", e))
                    })
                })
                .and_then(|response: reqwest::Response| {
                    response.json::<PublicKey>().map_err(|e| {
                        KeyStoreError::Error(format!("Failed to parse response: {}", e))
                    })
                })
                .map_ok(|json| json.public_key.as_bytes().to_vec())
        })
        .await
    }

    async fn set(&self, _id: &str, key_data: &[u8]) -> Result<(), KeyStoreError> {
        futures::future::ready(
            url::Url::parse(&self.url)
                .map_err(|e| KeyStoreError::Error(format!("Failed to parse url: {}", e))),
        )
        .and_then(|url| {
            futures::future::ready(
                url.host()
                    .ok_or_else(|| {
                        KeyStoreError::AuthenticationError(format!(
                            "Url \"{}\" is missing hostname.",
                            url.to_string(),
                        ))
                    })
                    .map(|host| host.to_string()),
            )
            .and_then(|scope| {
                debug!(
                    self.logger,
                    "Acquiring token for scope \"{}\" when uploading key", &scope
                );

                self.authentication
                    .acquire_token(tonic::Request::new(AcquireTokenParameters {
                        scope: scope.clone(),
                    }))
                    .map_err(move |e| {
                        KeyStoreError::AuthenticationError(format!(
                            "Failed to acquire token for scope {} when uploading key: {}",
                            scope, e
                        ))
                    })
                    .map_ok(|token| (url, token.into_inner().token))
            })
        })
        .and_then(|(url, token)| {
            futures::future::ready(String::from_utf8(key_data.to_vec()))
                .map_err(|e| KeyStoreError::Error(format!("Non utf-8 characters in key: {}", e)))
                .and_then(move |key_string| {
                    reqwest::Client::new()
                        .patch(url.to_string())
                        .json(&serde_json::json!({"patch": [{
                            "op": "add",
                            "path": "/public_key",
                            "value": key_string,
                        }] }))
                        .bearer_auth(token)
                        .send()
                        .map_err(|e| {
                            KeyStoreError::RemoteError(format!(
                                "Failed to get key store response: {}",
                                e
                            ))
                        })
                        .and_then(|response: reqwest::Response| async move {
                            response.error_for_status().map_err(|e| {
                                KeyStoreError::RemoteError(format!(
                                    "Received response with error: {}",
                                    e
                                ))
                            })
                        })
                        .map_ok(|_| ())
                })
        })
        .await
    }
}

pub struct NullKeyStore {}

#[async_trait::async_trait]
impl KeyStore for NullKeyStore {
    async fn get(&self, _id: &str) -> Result<Vec<u8>, KeyStoreError> {
        Ok(vec![])
    }

    async fn set(&self, _id: &str, _key_data: &[u8]) -> Result<(), KeyStoreError> {
        Ok(())
    }
}
