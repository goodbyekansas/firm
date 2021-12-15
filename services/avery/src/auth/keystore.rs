use std::{collections::HashMap, sync::Arc};

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
struct UserDocument {
    // public_key exists for backwards compatibility when we were naive
    // enough to think the people to computer data relationship would me
    // one to one.
    public_key: Option<String>,
    public_keys: Option<HashMap<String, String>>,
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

struct KeyId {
    user_id: String,
    key_id: Option<String>,
}

impl KeyId {
    fn new(s: &str) -> Self {
        s.split_once(':')
            .map(|(userid, keyid)| Self {
                user_id: userid.to_owned(),
                key_id: Some(keyid.to_owned()),
            })
            .unwrap_or_else(|| Self {
                user_id: s.to_owned(),
                key_id: None,
            })
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

    async fn get_user_document(&self, key_id: &KeyId) -> Result<UserDocument, KeyStoreError> {
        futures::future::ready(
            url::Url::parse(&self.url)
                .and_then(|url| url.join(&key_id.user_id))
                .map_err(|e| KeyStoreError::Error(format!("Failed to parse url: {}", e)))
                .and_then(|url| {
                    url.host()
                        .ok_or_else(|| {
                            KeyStoreError::AuthenticationError(format!(
                                "Url \"{}\" is missing hostname.",
                                url.to_string(),
                            ))
                        })
                        .map(|host| (host.to_string(), url.clone()))
                }),
        )
        .and_then(|(scope, url)| async {
            debug!(
                self.logger,
                "Acquiring token for scope \"{}\" \
                 when downloading key",
                &scope
            );

            self.token_source
                .write()
                .await
                .acquire_token(&scope, &self.logger)
                .await
                .map_err(move |e| {
                    KeyStoreError::AuthenticationError(format!(
                        "Failed to acquire token for scope \"{}\" \
                         when downloading key: {}",
                        scope, e
                    ))
                })
                .map(|token| (url, token.as_mut().token().to_owned()))
        })
        .and_then(|(url, token)| async move {
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
                    response.json::<UserDocument>().map_err(|e| {
                        KeyStoreError::Error(format!("Failed to parse response: {}", e))
                    })
                })
                .await
        })
        .await
    }
}

#[async_trait::async_trait]
impl KeyStore for SimpleKeyStore {
    async fn get(&self, id: &str) -> Result<Vec<u8>, KeyStoreError> {
        let key_id = KeyId::new(id);
        self.get_user_document(&key_id)
            .await
            .and_then(|json| match &key_id.key_id {
                Some(key_id) => json
                    .public_keys
                    .ok_or_else(|| {
                        KeyStoreError::Error(String::from("Failed to find key in keystore."))
                    })
                    .and_then(move |keys| {
                        keys.get(key_id)
                            .ok_or_else(|| {
                                KeyStoreError::Error(String::from(
                                    "Failed to find the specified key for the user.",
                                ))
                            })
                            .map(|public_key| public_key.as_bytes().to_vec())
                    }),
                None => json
                    .public_key
                    .ok_or_else(|| {
                        KeyStoreError::Error(String::from(
                            "Key id only has a user id \
                                 but user has no default public \
                                 key (`public_key`)",
                        ))
                    })
                    .map(|pk| pk.as_bytes().to_vec()),
            })
    }

    async fn set(&self, id: &str, key_data: &[u8]) -> Result<(), KeyStoreError> {
        let key_id = KeyId::new(id).key_id.ok_or_else(|| {
            KeyStoreError::Error(String::from("Key id is required for uploading keys"))
        })?;
        let key_id2 = KeyId::new(id);

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
            self.get_user_document(&key_id2)
                .map_ok(|user_document| (url, token, user_document))
        })
        .and_then(|(url, token, user_document)| {
            futures::future::ready(String::from_utf8(key_data.to_vec()))
                .map_err(|e| KeyStoreError::Error(format!("Non utf-8 characters in key: {}", e)))
                .and_then(move |key_string| {
                    let patch = if user_document.public_keys.is_some() {
                        serde_json::json!({"patch": [{
                            "op": "add",
                            "path": format!("/public_keys/{id}", id=key_id),
                            "value": key_string,
                        }]})
                    } else {
                        serde_json::json!({"patch": [
                            {
                                "op": "add",
                                "path": "/public_keys",
                                "value": {},
                            },
                            {
                                "op": "add",
                                "path": format!("/public_keys/{id}", id=key_id),
                                "value": key_string,
                            }
                        ]})
                    };

                    reqwest::Client::new()
                        .patch(url.to_string())
                        .json(&patch)
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
