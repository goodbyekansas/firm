use std::sync::Arc;

use futures::TryFutureExt;
use serde::Deserialize;
use slog::{debug, Logger};
use thiserror::Error;
use tokio::sync::RwLock;

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
pub trait KeyStore: Send + Sync + std::fmt::Debug {
    async fn get(&self, id: &str) -> Result<Vec<u8>, KeyStoreError>;
    async fn set(&self, id: &str, key_data: &[u8]) -> Result<(), KeyStoreError>;
}

#[derive(Deserialize)]
struct PublicKey {
    public_key: String,
}

pub struct SimpleKeyStore {
    url: String,
    token_source: Arc<RwLock<super::TokenStore>>,
    logger: Logger,
}

impl std::fmt::Debug for SimpleKeyStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimpleKeyStore")
            .field("url", &self.url)
            .finish()
    }
}

impl SimpleKeyStore {
    pub(super) fn new(
        url: &str,
        token_source: Arc<RwLock<super::TokenStore>>,
        logger: Logger,
    ) -> Self {
        Self {
            url: url.to_owned(),
            token_source,
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
            .and_then(|scope| async {
                debug!(
                    self.logger,
                    "Acquiring token for scope \"{}\" when uploading key", &scope
                );

                self.token_source
                    .write()
                    .await
                    .acquire_token(&scope, &self.logger)
                    .await
                    .map_err(move |e| {
                        KeyStoreError::AuthenticationError(format!(
                            "Failed to acquire token for scope \"{}\" when uploading key: {}",
                            scope, e
                        ))
                    })
                    .map(|token| (url, token.as_mut().token().to_owned()))
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
                .map_err(|e| KeyStoreError::Error(format!("Failed to parse url: {}", e)))
                .and_then(|url| {
                    url.host()
                        .ok_or_else(|| {
                            KeyStoreError::AuthenticationError(format!(
                                "Url \"{}\" is missing hostname.",
                                url.to_string(),
                            ))
                        })
                        .map(|host| (url.clone(), host.to_string()))
                }),
        )
        .and_then(|(url, scope)| async {
            debug!(
                self.logger,
                "Acquiring token for scope \"{}\" when uploading key", &scope
            );

            self.token_source
                .write()
                .await
                .acquire_token(&scope, &self.logger)
                .await
                .map_err(move |e| {
                    KeyStoreError::AuthenticationError(format!(
                        "Failed to acquire token for scope {} when uploading key: {}",
                        scope, e
                    ))
                })
                .map(|token| (url, token.as_mut().token().to_owned()))
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

#[derive(Debug)]
pub struct NullKeyStore {}

#[async_trait::async_trait]
impl KeyStore for NullKeyStore {
    async fn get(&self, _id: &str) -> Result<Vec<u8>, KeyStoreError> {
        Err(KeyStoreError::Error(
            "Failed to find public key (which is expected since this is a null key store)!"
                .to_owned(),
        ))
    }

    async fn set(&self, _id: &str, _key_data: &[u8]) -> Result<(), KeyStoreError> {
        Ok(())
    }
}
