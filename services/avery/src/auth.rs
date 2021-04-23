use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use chrono::{TimeZone, Utc};
use firm_types::{
    auth::authentication_server::Authentication, auth::AcquireTokenParameters,
    auth::Token as ProtoToken, tonic,
};
use futures::TryFutureExt;
use slog::{debug, info, o, Logger};
use tokio::sync::RwLock;

use self::{aliasmap::AliasMap, keystore::KeyStore, oidc::Oidc};
use crate::config::{AuthConfig, IdentityProvider, OidcProvider};

mod aliasmap;
mod internal;
mod keystore;
mod oidc;

type SharedToken = Arc<RwLock<Box<dyn Token>>>;
type TokenCacheMap = Arc<RwLock<HashMap<String, SharedToken>>>;

#[derive(Clone)]
pub struct AuthService {
    providers: Providers,
    token_cache: TokenCache,
    logger: Logger,
    token_generators: TokenGenerators,
    user_identity: String,
    key_store: Arc<Box<dyn KeyStore>>,
}

#[derive(Clone)]
struct TokenCache {
    tokens: TokenCacheMap,
    scope_aliases: Arc<AliasMap>,
}

impl TokenCache {
    fn new(scope_aliases: AliasMap) -> Self {
        Self {
            tokens: Arc::new(RwLock::new(HashMap::new())),
            scope_aliases: Arc::new(scope_aliases),
        }
    }

    async fn insert<'a>(&'a self, token: Box<dyn Token>, scope: &'a str) -> SharedToken {
        let mut cache = self.tokens.write().await;
        let shared_token = Arc::new(RwLock::new(token));

        match self.scope_aliases.get(scope) {
            Some(aliases) => aliases.iter().for_each(|alias| {
                cache.insert(alias.to_owned(), Arc::clone(&shared_token));
            }),
            None => {
                cache.insert(scope.to_owned(), Arc::clone(&shared_token));
            }
        }

        shared_token
    }

    async fn get(&self, key: &str) -> Option<SharedToken> {
        self.tokens.read().await.get(key).cloned()
    }
}

#[derive(Clone)]
struct Providers {
    oidc: Arc<HashMap<String, Oidc>>,
    auth: Arc<HashMap<String, AuthConfig>>,
}

#[derive(Clone)]
struct TokenGenerators {
    self_signed: Arc<internal::TokenGenerator>,
    self_signed_with_file: Arc<HashMap<PathBuf, internal::TokenGenerator>>,
}

#[cfg(unix)]
fn set_keyfolder_permissions(keystore_path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    keystore_path.metadata().and_then(|m| {
        let mut perm = m.permissions();
        perm.set_mode(0o700);
        std::fs::set_permissions(keystore_path, perm)
    })
}

#[cfg(windows)]
#[allow(clippy::unnecessary_wraps)]
fn set_keyfolder_permissions(_: &Path) -> std::io::Result<()> {
    Ok(())
}

#[async_trait::async_trait]
pub trait Token: Send + Sync {
    fn token(&self) -> &str;
    async fn refresh(&mut self) -> Result<&mut dyn Token, String>;
    fn expires_at(&self) -> u64;
    fn exp(&self) -> Option<u64>;
    fn iss(&self) -> Option<&str>;
    fn iat(&self) -> Option<u64>;
    fn jti(&self) -> Option<&str>;
    fn nbf(&self) -> Option<u64>;
    fn sub(&self) -> Option<&str>;
    fn aud(&self) -> Option<&str>;
    fn claim(&self, key: &str) -> Option<&serde_json::Value>;
}

impl AuthService {
    pub async fn new(
        oidc_providers: HashMap<String, OidcProvider>,
        auth_scopes: HashMap<String, AuthConfig>,
        identity_provider: IdentityProvider,
        key_store_config: crate::config::KeyStore,
        logger: Logger,
    ) -> Result<Self, String> {
        let token_cache = TokenCache::new(
            auth_scopes
                .iter()
                .fold(HashMap::new(), |mut m, (k, v)| match v {
                    AuthConfig::Oidc { provider } => {
                        m.entry(provider.to_owned())
                            .or_insert_with(Vec::new)
                            .push(k.to_owned());
                        m
                    }
                    _ => m,
                })
                .into(),
        );
        let providers = Providers {
            oidc: Arc::new(
                oidc_providers
                    .into_iter()
                    .map(|(key, value)| {
                        (
                            key.clone(),
                            Oidc::new(&value, logger.new(o!("scope" => "oidc", "host" => key))),
                        )
                    })
                    .collect::<HashMap<String, Oidc>>(),
            ),

            auth: Arc::new(auth_scopes),
        };

        let audience = Self::get_identity(identity_provider, &providers, &token_cache).await?;

        let (new_key, token_generators) = Self::create_token_generators(
            &audience,
            &providers,
            logger.new(o!("scope" => "token-generator-creation")),
        )?;
        let mut auth_service = Self {
            token_cache,
            providers,
            token_generators,
            user_identity: audience.clone(),
            key_store: Arc::new(Box::new(keystore::NullKeyStore {})),
            logger,
        };
        let key_store = Arc::new(match key_store_config {
            // TODO:
            // The key store needs an auth service object that has everything except the key store
            // and only has a token endpoint. To acquire a token, token cache providers and token generators
            // are needed from the auth service object. One could create an object that contains cache,
            // provides and generators with a acquire_token (only takes string scope as arg) function (not containing any tonic specific code).
            // This object could be sent into the key store.
            crate::config::KeyStore::Simple { url } => Box::new(keystore::SimpleKeyStore::new(
                &url,
                auth_service.clone(),
                auth_service
                    .logger
                    .new(o!("scope" => "key-store", "type" => "simple", "url" => url.clone())),
            )) as Box<dyn KeyStore>,
            crate::config::KeyStore::None => {
                Box::new(keystore::NullKeyStore {}) as Box<dyn KeyStore>
            }
        });
        if let Some(public_key_path) = new_key {
            futures::future::ready(std::fs::read(&public_key_path).map_err(|e| {
                format!(
                    "Failed to read public key file at {}: {}",
                    public_key_path.display(),
                    e
                )
            }))
            .and_then(|key_content| {
                let key_store = Arc::clone(&key_store);
                debug!(auth_service.logger, "Uploading key store data");
                async move {
                    key_store
                        .clone()
                        .set(&audience, key_content.as_ref())
                        .map_err(|e| e.to_string())
                        .await
                }
            })
            .await?;
        }
        auth_service.key_store = key_store;
        Ok(auth_service)
    }

    async fn get_identity(
        identity_provider: IdentityProvider,
        providers: &Providers,
        token_cache: &TokenCache,
    ) -> Result<String, String> {
        match identity_provider {
            IdentityProvider::Oidc { provider } => {
                futures::future::ready(
                    providers
                        .oidc
                        .get(&provider)
                        .ok_or_else(|| format!("Oidc provider \"{}\" not found.", provider)),
                )
                .and_then(|oidc_client| oidc_client.authenticate().map_err(|e| e.to_string()))
                .map_ok(Box::new)
                .and_then(|token| {
                    let providers = providers.clone();
                    let token_cache = token_cache.clone();
                    async move {
                        let email = token
                            .claim("email")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_owned());

                        if let Some(scope) =
                            providers
                                .auth
                                .iter()
                                .find_map(|(scope, config)| match config {
                                    AuthConfig::Oidc { .. } => Some(scope),
                                    _ => None,
                                })
                        {
                            token_cache.insert(token, scope).await;
                        }
                        Ok(email)
                    }
                })
                .await?
            }
            IdentityProvider::Username => crate::system::user(),
            IdentityProvider::UsernameSuffix { suffix } => {
                crate::system::user().map(|name| format!("{}{}", name, suffix))
            }
            IdentityProvider::Override { identity } => Some(identity),
        }
        .ok_or_else(|| "Failed to determine identity".to_owned())
    }

    fn create_token_generators<'a>(
        audience: &'a str,
        providers: &'a Providers,
        logger: Logger,
    ) -> Result<(Option<PathBuf>, TokenGenerators), String> {
        let keystore_path = crate::system::user_data_path()
            .map(|p| p.join("keys"))
            .ok_or_else(|| {
                "Could not determine key store path for saving generated keys".to_owned()
            })?;
        let (new_key, self_signed) = std::fs::create_dir_all(&keystore_path)
            .map_err(|e| {
                format!(
                    "Failed to create keystore directory at \"{}\": {}",
                    &keystore_path.display(),
                    e
                )
            })
            .and_then(|_| {
                set_keyfolder_permissions(&keystore_path)
                    .map_err(|e| format!("Failed to set permissions on keystore directory: {}", e))
            })
            .and_then(|_| {
                let private_key_path = keystore_path.join("id_ecdsa.pem");
                let public_key_path = keystore_path.join("id_ecdsa_pub.pem");
                if private_key_path.exists() {
                    info!(
                        logger,
                        "Using token signing private key from: {}",
                        private_key_path.display()
                    );
                    internal::TokenGeneratorBuilder::new(&audience)
                        .with_ecdsa_private_key_from_file(&private_key_path)
                        .build()
                        .map_err(|e| e.to_string())
                        .map(|token_gen| (None, token_gen))
                } else {
                    internal::TokenGeneratorBuilder::new(&audience)
                        .build()
                        .map_err(|e| e.to_string())
                        .and_then(|tg| {
                            tg.save_keys(&private_key_path, &public_key_path)
                                .map_err(|e| e.to_string())?;
                            info!(
                                logger,
                                "Saved generated token signing keypair in {} and {}",
                                private_key_path.display(),
                                public_key_path.display()
                            );
                            Ok((Some(public_key_path), tg))
                        })
                }
                .map(|(new_key, tg)| (new_key, Arc::new(tg)))
            })?;

        Ok((
            new_key,
            TokenGenerators {
                self_signed,
                self_signed_with_file: Arc::new(
                    providers
                        .auth
                        .iter()
                        .filter_map(|(_, value)| match value {
                            AuthConfig::KeyFile { path } => Some(
                                internal::TokenGeneratorBuilder::new(&audience)
                                    .with_rsa_private_key_from_file(path)
                                    .build()
                                    .map(|token_generator| (path.to_owned(), token_generator)),
                            ),
                            _ => None,
                        })
                        .collect::<Result<HashMap<_, _>, internal::SelfSignedTokenError>>()
                        .map_err(|e| e.to_string())?,
                ),
            },
        ))
    }
}

#[tonic::async_trait]
impl Authentication for AuthService {
    async fn acquire_token(
        &self,
        request: tonic::Request<AcquireTokenParameters>,
    ) -> Result<tonic::Response<ProtoToken>, tonic::Status> {
        let scope = &request.get_ref().scope;
        let auth = match self.token_cache.get(scope).await {
            Some(token) => {
                debug!(
                    self.logger,
                    "Found cached token for scope \"{}\", expires at {}",
                    &request.get_ref().scope,
                    Utc.timestamp(token.read().await.expires_at() as i64, 0)
                        .to_string(),
                );
                token.clone()
            }
            None => {
                self.token_cache
                    .insert(
                        match self.providers.auth.get(scope) {
                            Some(AuthConfig::None) | None => {
                                return Ok(tonic::Response::new(ProtoToken {
                                    token: "".to_owned(),
                                    expires_at: 0,
                                    scope: scope.clone(),
                                }));
                            }
                            Some(AuthConfig::Oidc { provider }) => futures::future::ready(
                                self.providers.oidc.get(provider).ok_or_else(|| {
                                    tonic::Status::internal(format!(
                                        "Oidc provider \"{}\" not found.",
                                        provider
                                    ))
                                }),
                            )
                            .and_then(|oidc_client| {
                                oidc_client
                                    .authenticate()
                                    .map_err(|e| tonic::Status::internal(e.to_string()))
                            })
                            .await
                            .map(Box::new)?,
                            Some(AuthConfig::SelfSigned) => self
                                .token_generators
                                .self_signed
                                .generate(scope)
                                .map_err(|e| tonic::Status::internal(e.to_string()))
                                .map(Box::new)?,
                            Some(AuthConfig::KeyFile { path }) => self
                                .token_generators
                                .self_signed_with_file
                                .get(path)
                                .ok_or_else(|| {
                                    tonic::Status::internal(format!(
                                    "Failed to find self signed generator for keyfile at \"{}\"",
                                    path.display(),
                                ))
                                })
                                .and_then(|generator| {
                                    generator
                                        .generate(scope)
                                        .map_err(|e| tonic::Status::internal(e.to_string()))
                                })
                                .map(Box::new)?,
                        },
                        scope,
                    )
                    .await
            }
        };

        // always do refresh, the refresh methods of the providers
        // are responsible for checking if it is needed
        auth.write().await.refresh().await.map_err(|err| {
            tonic::Status::internal(format!(
                "Failed to refresh token for scope \"{}\": {}",
                &request.get_ref().scope,
                err
            ))
        })?;

        let auth_read = auth.read().await;
        Ok(tonic::Response::new(ProtoToken {
            token: auth_read.token().to_owned(),
            expires_at: auth_read.expires_at(),
            scope: request.get_ref().scope.clone(),
        }))
    }
}
