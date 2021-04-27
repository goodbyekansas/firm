use std::{
    collections::HashMap,
    hash::Hash,
    hash::Hasher,
    path::{Path, PathBuf},
    sync::Arc,
};

use chrono::{TimeZone, Utc};
use firm_types::{
    auth::authentication_server::Authentication, auth::AcquireTokenParameters,
    auth::Token as ProtoToken, tonic,
};
use futures::TryFutureExt;
use serde::{Deserialize, Serialize};
use slog::{debug, info, o, warn, Logger};
use tokio::sync::RwLock;

use self::{aliasmap::AliasMap, keystore::KeyStore, oidc::Oidc};
use crate::config::{AuthConfig, IdentityProvider, OidcProvider};

mod aliasmap;
mod internal;
mod keystore;
mod oidc;

#[derive(Clone)]
pub struct AuthService {
    providers: Providers,
    token_cache: Arc<RwLock<TokenCache>>,
    logger: Logger,
    token_generators: TokenGenerators,
    user_identity: String,
    key_store: Arc<Box<dyn KeyStore>>,
}

struct TokenCache {
    tokens: HashMap<String, TypedToken>,
    scope_aliases: AliasMap,
    logger: Logger,
    save_cache: bool,
}

impl Drop for TokenCache {
    fn drop(&mut self) {
        if self.save_cache {
            if let Err(e) = Self::token_cache_path()
                .and_then(|path| {
                    std::fs::OpenOptions::new()
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .open(path)
                        .map_err(|e| -> Box<dyn FnOnce(&Logger)> {
                            Box::new(move |logger: &Logger| warn!(logger, "{}", e))
                        })
                })
                .and_then(|file| {
                    serde_json::to_writer(std::io::BufWriter::new(file), &self.tokens).map_err(
                        |e| -> Box<dyn FnOnce(&Logger)> {
                            Box::new(move |logger: &Logger| warn!(logger, "{}", e))
                        },
                    )
                })
            {
                e(&self.logger);
            }
        }
    }
}

impl TokenCache {
    fn token_cache_path() -> Result<PathBuf, Box<dyn FnOnce(&Logger)>> {
        crate::system::user_data_path()
            .map(|p| p.join("token-cache.json"))
            .ok_or_else(|| -> Box<dyn FnOnce(&Logger)> {
                Box::new(|logger: &Logger| warn!(logger, "Could not determine token cache path"))
            })
    }

    async fn new(scope_aliases: AliasMap, logger: Logger) -> Self {
        Self {
            tokens: match Self::token_cache_path()
                .and_then(|path| {
                    path.exists().then(|| path.clone()).ok_or_else(
                        || -> Box<dyn FnOnce(&Logger)> {
                            Box::new(move |logger: &Logger| {
                                debug!(
                                    logger,
                                    "Token cache file does not exist: {}",
                                    path.display()
                                )
                            })
                        },
                    )
                })
                .and_then(|path| {
                    std::fs::File::open(path).map_err(|e| -> Box<dyn FnOnce(&Logger)> {
                        Box::new(move |logger: &Logger| warn!(logger, "{}", e))
                    })
                })
                .and_then(|file| -> Result<HashMap<String, TypedToken>, _> {
                    serde_json::from_reader(std::io::BufReader::new(file)).map_err(
                        |e| -> Box<dyn FnOnce(&Logger)> {
                            Box::new(move |logger: &Logger| warn!(logger, "{}", e))
                        },
                    )
                }) {
                Ok(tokens) => tokens,
                Err(e) => {
                    e(&logger);
                    HashMap::new()
                }
            },
            scope_aliases,
            logger,
            save_cache: true,
        }
    }

    fn insert(&mut self, scope: &str, token: TypedToken) -> &mut TypedToken {
        match self.tokens.entry(scope.to_owned()) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                entry.insert(token);
                entry.into_mut()
            }
            std::collections::hash_map::Entry::Vacant(entry) => entry.insert(token),
        }
    }

    fn get(&mut self, key: &str) -> Option<&mut TypedToken> {
        let empty = vec![key.to_owned()];
        self.tokens.get_mut(
            match self.scope_aliases.get(key) {
                Some(strings) => strings,
                None => empty.as_slice(),
            }
            .iter()
            .find(|key| self.tokens.contains_key(*key))?,
        )
    }

    fn get_as_token(&mut self, key: &str) -> Option<&mut dyn Token> {
        self.get(key).map(|v| v.as_mut())
    }
}

impl Default for TokenCache {
    fn default() -> Self {
        Self {
            tokens: HashMap::new(),
            scope_aliases: AliasMap::from(HashMap::new()),
            logger: slog::Logger::root(slog::Discard, slog::o!()),
            save_cache: false,
        }
    }
}

#[derive(Clone, Default)]
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

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum TypedToken {
    Oidc(oidc::OidcToken),

    #[serde(skip)]
    Internal(internal::JwtToken),
}

impl<'a> AsMut<dyn Token + 'a> for TypedToken {
    fn as_mut(&mut self) -> &mut (dyn Token + 'a) {
        match self {
            TypedToken::Oidc(token) => token,
            TypedToken::Internal(token) => token,
        }
    }
}

#[async_trait::async_trait]
pub trait Token: Send + Sync {
    fn token(&self) -> &str;
    async fn refresh(&mut self, logger: &Logger) -> Result<&mut dyn Token, String>;
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

trait ScopeKey {
    fn scope_key(&self) -> String;
}

impl ScopeKey for IdentityProvider {
    fn scope_key(&self) -> String {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.hash(&mut hasher);
        let hash = hasher.finish();
        format!(
            "{}-identity-provider-{:x}",
            match self {
                IdentityProvider::Oidc { .. } => "oidc",
                IdentityProvider::Username => "username",
                IdentityProvider::UsernameSuffix { .. } => "username-suffix",
                IdentityProvider::Override { .. } => "override",
            },
            hash
        )
    }
}

impl AuthService {
    pub async fn new(
        oidc_providers: HashMap<String, OidcProvider>,
        auth_scopes: HashMap<String, AuthConfig>,
        identity_provider: IdentityProvider,
        key_store_config: crate::config::KeyStore,
        logger: Logger,
    ) -> Result<Self, String> {
        let mut token_cache = TokenCache::new(
            Self::create_alias_map(&auth_scopes, &identity_provider),
            logger.new(o!("scope" => "token-cache")),
        )
        .await;

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

        let audience = Self::get_identity(
            identity_provider,
            &providers,
            &mut token_cache,
            crate::system::user,
            logger.new(o!("scope" => "get-identity")),
        )
        .await?;

        let (new_key, token_generators) = Self::create_token_generators(
            &audience,
            &providers,
            logger.new(o!("scope" => "token-generator-creation")),
        )?;

        let mut auth_service = Self {
            token_cache: Arc::new(RwLock::new(token_cache)),
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

        // upload the newly generated internal key
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

    fn create_alias_map(
        auth_scopes: &HashMap<String, AuthConfig>,
        identity_provider: &IdentityProvider,
    ) -> AliasMap {
        let provider_scope_key = identity_provider.scope_key();
        auth_scopes
            .iter()
            .fold(
                match identity_provider {
                    IdentityProvider::Oidc { provider } => {
                        let mut m = HashMap::new();
                        m.insert(provider.to_owned(), vec![provider_scope_key]);
                        m
                    }
                    _ => HashMap::new(),
                },
                |mut m, (k, v)| match v {
                    AuthConfig::Oidc { provider } => {
                        m.entry(provider.to_owned())
                            .or_insert_with(Vec::new)
                            .push(k.to_owned());
                        m
                    }
                    _ => m,
                },
            )
            .into()
    }

    async fn get_identity<'a, F>(
        identity_provider: IdentityProvider,
        providers: &'a Providers,
        token_cache: &'a mut TokenCache,
        username_provider: F,
        logger: Logger,
    ) -> Result<String, String>
    where
        F: FnOnce() -> Option<String>,
    {
        let provider_scope_key = identity_provider.scope_key();
        match identity_provider {
            IdentityProvider::Oidc { provider } => {
                let logger = logger.new(o!(
                    "provider-type" => "oidc",
                    "provider-name" => provider.clone(),
                    "scope-key" => provider_scope_key.clone()
                ));
                futures::future::ready(
                    providers
                        .oidc
                        .get(&provider)
                        .ok_or_else(|| format!("OIDC provider \"{}\" not found", provider)),
                )
                .and_then(|oidc_client| async move {
                    match token_cache.get_as_token(&provider_scope_key) {
                        // We're done. Got a cached token.
                        Some(token) => {
                            debug!(logger, "Found cached token");
                            Ok(token)
                        }

                        // We currently do not have a token cached and need to create one
                        None => {
                            debug!(logger, "Obtaining token");
                            oidc_client
                                .authenticate()
                                .map_err(|e| e.to_string())
                                .map_ok(move |token| {
                                    token_cache
                                        .insert(&provider_scope_key, TypedToken::Oidc(token))
                                        .as_mut()
                                })
                                .await
                        }
                    }
                    .map(|token| {
                        token
                            .claim("email")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_owned())
                    })
                })
                .await?
            }
            IdentityProvider::Username => username_provider(),
            IdentityProvider::UsernameSuffix { suffix } => {
                username_provider().map(|name| format!("{}{}", name, suffix))
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
        let mut token_cache = self.token_cache.write().await;
        let auth = match token_cache.get_as_token(scope) {
            Some(token) => {
                debug!(
                    self.logger,
                    "Found cached token for scope \"{}\", expires at {}",
                    scope,
                    Utc.timestamp(token.expires_at() as i64, 0).to_string(),
                );
                token
            }
            None => token_cache
                .insert(
                    scope,
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
                        .map(TypedToken::Oidc)?,
                        Some(AuthConfig::SelfSigned) => self
                            .token_generators
                            .self_signed
                            .generate(scope)
                            .map_err(|e| tonic::Status::internal(e.to_string()))
                            .map(TypedToken::Internal)?,
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
                            .map(TypedToken::Internal)?,
                    },
                )
                .as_mut(),
        };

        // always do refresh, the refresh methods of the providers
        // are responsible for checking if it is needed
        auth.refresh(&self.logger).await.map_err(|err| {
            tonic::Status::internal(format!(
                "Failed to refresh token for scope \"{}\": {}",
                scope, err
            ))
        })?;

        Ok(tonic::Response::new(ProtoToken {
            token: auth.token().to_owned(),
            expires_at: auth.expires_at(),
            scope: request.get_ref().scope.clone(),
        }))
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    macro_rules! null_logger {
        () => {{
            slog::Logger::root(slog::Discard, slog::o!())
        }};
    }

    #[test]
    fn alias_map_creation() {
        let mut auth_scopes = HashMap::new();
        auth_scopes.insert(
            "a".to_owned(),
            AuthConfig::Oidc {
                provider: "auth".to_owned(),
            },
        );

        let map = AuthService::create_alias_map(
            &auth_scopes,
            &(IdentityProvider::Oidc {
                provider: "auth".to_owned(),
            }),
        );
        assert_eq!(
            map.get("a").unwrap().len(),
            2,
            "Alias map must contain both scopes when an OIDC provider is used both in a scope and as identity provider"
        );

        let map = AuthService::create_alias_map(
            &auth_scopes,
            &IdentityProvider::Override {
                identity: "i-am-identity@company.se".to_owned(),
            },
        );

        assert_eq!(
            map.get("a").unwrap().len(),
            1,
            "Alias map must only contain itself when identity provider does not overlap with scopes"
        );
    }

    #[tokio::test]
    async fn get_identity() {
        let id = AuthService::get_identity(
            IdentityProvider::Override {
                identity: "user@company.com".to_owned(),
            },
            &Providers::default(),
            &mut TokenCache::default(),
            || None,
            null_logger!(),
        )
        .await;

        assert!(id.is_ok());
        assert_eq!(
            id.unwrap(),
            "user@company.com",
            "Overridden user identity must come back unmodified"
        );

        let id = AuthService::get_identity(
            IdentityProvider::UsernameSuffix {
                suffix: "@company.com".to_owned(),
            },
            &Providers::default(),
            &mut TokenCache::default(),
            || Some("user".to_owned()),
            null_logger!(),
        )
        .await;

        assert!(id.is_ok());
        assert_eq!(
            id.unwrap(),
            "user@company.com",
            "Username suffix must be added to the username from the system"
        );

        let id = AuthService::get_identity(
            IdentityProvider::Username,
            &Providers::default(),
            &mut TokenCache::default(),
            || Some("username".to_owned()),
            null_logger!(),
        )
        .await;

        assert!(id.is_ok());
        assert_eq!(
            id.unwrap(),
            "username",
            "Username must be the un-altered username from the system"
        );

        // we do not test OIDC here because it either needs to access an OIDC service or
        // it needs to have an already cached key that is very inconvenient to create manually
    }
}
