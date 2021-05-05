use std::{
    collections::HashMap,
    collections::HashSet,
    fmt::Debug,
    hash::Hash,
    hash::Hasher,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use chrono::{TimeZone, Utc};
use expiremap::ExpireMap;
use firm_types::{
    auth::authentication_server::Authentication, auth::AcquireTokenParameters,
    auth::Token as ProtoToken, tonic,
};
use futures::{future::OptionFuture, TryFutureExt};
use serde::{Deserialize, Serialize};
use slog::{debug, info, o, warn, Logger};
use tokio::{sync::RwLock, time::Instant};

pub use self::keystore::{KeyStore, KeyStoreError};
use self::{aliasmap::AliasMap, oidc::Oidc};
use crate::config::{AuthConfig, IdentityProvider, OidcProvider};

mod aliasmap;
mod expiremap;
mod internal;
mod keystore;
mod oidc;

#[derive(Debug, Clone, Default)]
struct PendingAccessRequest {
    subject: String,
    expires_at: u64,
    approved: bool,
}

impl From<firm_types::auth::RemoteAccessRequest> for PendingAccessRequest {
    fn from(rar: firm_types::auth::RemoteAccessRequest) -> Self {
        Self {
            subject: rar.subject,
            expires_at: rar.expires_at,
            approved: rar.approved,
        }
    }
}

#[derive(Clone)]
pub struct AuthService {
    logger: Logger,
    token_store: Arc<RwLock<TokenStore>>,
    key_store: Arc<Box<dyn KeyStore>>,
    scope_mappings: Arc<HashMap<String, AuthConfig>>,
    access_list: ExpireMap<String, ()>,
    pending_access_requests: ExpireMap<uuid::Uuid, PendingAccessRequest>,
}

impl Debug for AuthService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Auth service: {:?}", self.token_store)
    }
}

impl Default for AuthService {
    fn default() -> Self {
        Self {
            logger: slog::Logger::root(slog::Discard, slog::o!()),
            token_store: Arc::new(RwLock::new(TokenStore::default())),
            key_store: Arc::new(Box::new(keystore::NullKeyStore {})),
            scope_mappings: Arc::new(HashMap::new()),
            access_list: ExpireMap::default(),
            pending_access_requests: ExpireMap::default(),
        }
    }
}

#[async_trait::async_trait]
pub trait AuthenticationSource: Send + Sync {
    async fn acquire_token(&self, scope: &str) -> Result<String, String>;
}

#[async_trait::async_trait]
impl AuthenticationSource for AuthService {
    async fn acquire_token(&self, scope: &str) -> Result<String, String> {
        self.token_store
            .write()
            .await
            .acquire_token(scope, &self.scope_mappings, &self.logger)
            .await
            .map(|t| t.token().to_owned())
    }
}

#[derive(Debug, Default)]
struct TokenStore {
    token_cache: TokenCache,
    token_providers: TokenProviders,
}

impl TokenStore {
    async fn acquire_token(
        &mut self,
        scope: &str,
        scope_mappings: &HashMap<String, AuthConfig>,
        logger: &Logger,
    ) -> Result<&mut dyn Token, String> {
        match self.token_cache.entry(scope) {
            std::collections::hash_map::Entry::Occupied(mut e) => {
                debug!(
                    logger,
                    "Found cached token for scope \"{}\", expires at {}",
                    scope,
                    Utc.timestamp(e.get_mut().as_mut().expires_at() as i64, 0)
                        .to_string(),
                );
                e.into_mut()
            }
            std::collections::hash_map::Entry::Vacant(e) => {
                debug!(
                    logger,
                    "Found no cached token for scope \"{}\", acquiring new token...", scope
                );
                e.insert(
                    self.token_providers
                        .get_token(scope_mappings, scope)
                        .await?,
                )
            }
        }
        .as_mut()
        // always do refresh, the refresh methods of the providers
        // are responsible for checking if it is needed
        .refresh(&logger)
        .await
        .map_err(|err| format!("Failed to refresh token for scope \"{}\": {}", scope, err))
    }
}

#[derive(Debug)]
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
                    serde_json::to_writer(
                        std::io::BufWriter::new(file),
                        &self
                            .tokens
                            .iter()
                            .filter(|(_, v)| match v {
                                TypedToken::Oidc(_) => true,
                                TypedToken::Internal(_) => false,
                            })
                            .collect::<HashMap<&String, &TypedToken>>(),
                    )
                    .map_err(|e| -> Box<dyn FnOnce(&Logger)> {
                        Box::new(move |logger: &Logger| warn!(logger, "{}", e))
                    })
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

    fn alias(&self, scope: &str) -> Option<String> {
        match self.scope_aliases.get(scope) {
            Some(strings) => strings
                .iter()
                .find(|key| self.tokens.contains_key(*key))
                .map(|s| s.to_owned()),
            None => Some(scope.to_owned()),
        }
    }

    fn get(&mut self, scope: &str) -> Option<&mut TypedToken> {
        self.alias(scope)
            .and_then(move |alias| self.tokens.get_mut(&alias))
    }

    fn entry(&mut self, scope: &str) -> std::collections::hash_map::Entry<String, TypedToken> {
        match self.alias(scope) {
            Some(alias) => self.tokens.entry(alias),
            None => self.tokens.entry(scope.to_owned()),
        }
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

#[derive(Debug, Default)]
struct TokenProviders {
    oidc: HashMap<String, Oidc>,
    self_signed: Option<internal::TokenGenerator>,
    self_signed_with_file: HashMap<PathBuf, internal::TokenGenerator>,
}

impl TokenProviders {
    async fn get_token(
        &self,
        scope_mappings: &HashMap<String, AuthConfig>,
        scope: &str,
    ) -> Result<TypedToken, String> {
        match scope_mappings.get(scope) {
            Some(AuthConfig::Oidc { provider }) => {
                futures::future::ready(self.oidc.get(provider).ok_or_else(|| {
                    format!("Oidc provider \"{}\" not found.", provider)
                }))
                .and_then(|oidc_client| {
                    oidc_client
                        .authenticate()
                        .map_err(|e| e.to_string())
                })
                .await
                .map(TypedToken::Oidc)
            }
            Some(AuthConfig::SelfSigned) | None => self
                .self_signed
                .as_ref()
                .ok_or_else(|| {
                    format!(
                        "Scope mappings specify to use a self-signed token for scope \"{}\" but none has been configured",
                        scope
                    )
                })
                .and_then(|generator| generator.generate(scope).map_err(|e| e.to_string()))
                .map(TypedToken::Internal),
            Some(AuthConfig::KeyFile { path }) => self
                .self_signed_with_file
                .get(path)
                .ok_or_else(|| {
                    format!(
                        "Failed to find self signed generator for keyfile at \"{}\"",
                        path.display(),
                    )
                })
                .and_then(|generator| {
                    generator
                        .generate(scope)
                        .map_err(|e| e.to_string())
                })
                .map(TypedToken::Internal),
        }
    }
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

#[derive(Debug, Serialize, Deserialize)]
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
    pub fn new(keystore: Box<dyn KeyStore>) -> Self {
        Self {
            key_store: Arc::new(keystore),
            ..Default::default()
        }
    }

    pub fn with_access_list(&mut self, access_list: HashSet<String>) -> &mut Self {
        self.access_list = access_list.into_iter().collect();
        self
    }

    pub async fn from_config(
        oidc_providers: HashMap<String, OidcProvider>,
        auth_scopes: HashMap<String, AuthConfig>,
        identity_provider: IdentityProvider,
        key_store_config: crate::config::KeyStore,
        access_config: crate::config::AllowConfig,
        logger: Logger,
    ) -> Result<Self, String> {
        let mut token_cache = TokenCache::new(
            Self::create_alias_map(&auth_scopes, &identity_provider),
            logger.new(o!("scope" => "token-cache")),
        )
        .await;

        let oidc_providers = oidc_providers
            .into_iter()
            .map(|(key, value)| {
                (
                    key.clone(),
                    Oidc::new(&value, logger.new(o!("scope" => "oidc", "host" => key))),
                )
            })
            .collect::<HashMap<String, Oidc>>();

        let audience = Self::get_identity(
            identity_provider,
            &oidc_providers,
            &mut token_cache,
            crate::system::user,
            logger.new(o!("scope" => "get-identity")),
        )
        .await?;

        let self_signed_with_file = auth_scopes
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
            .map_err(|e| e.to_string())?;

        let (new_key, self_signed) = Self::create_self_signed_generator(
            &audience,
            logger.new(o!("scope" => "token-generator-creation")),
        )?;

        let token_store = Arc::new(RwLock::new(TokenStore {
            token_cache,
            token_providers: TokenProviders {
                oidc: oidc_providers,
                self_signed: Some(self_signed),
                self_signed_with_file,
            },
        }));

        let key_store = match key_store_config {
            crate::config::KeyStore::Simple { url } => Box::new(keystore::SimpleKeyStore::new(
                &url,
                token_store.clone(),
                auth_scopes.clone(),
                logger.new(o!("scope" => "key-store", "type" => "simple", "url" => url.clone())),
            )) as Box<dyn KeyStore>,
            crate::config::KeyStore::None => {
                Box::new(keystore::NullKeyStore {}) as Box<dyn KeyStore>
            }
        };

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
                let ks = &key_store;
                let audience = &audience;
                let logger = &logger;
                async move {
                    debug!(logger, "Uploading key store data");
                    ks.set(audience, key_content.as_ref())
                        .map_err(|e| e.to_string())
                        .await
                }
            })
            .await?;
        }

        Ok(Self {
            key_store: Arc::new(key_store),
            token_store,
            scope_mappings: Arc::new(auth_scopes),
            logger,
            access_list: access_config.users.into_iter().collect(),
            pending_access_requests: ExpireMap::default(),
        })
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
        oidc_providers: &'a HashMap<String, Oidc>,
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
                    oidc_providers
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

    fn create_self_signed_generator(
        audience: &str,
        logger: Logger,
    ) -> Result<(Option<PathBuf>, internal::TokenGenerator), String> {
        let keystore_path = crate::system::user_data_path()
            .map(|p| p.join("keys"))
            .ok_or_else(|| {
                "Could not determine key store path for saving generated keys".to_owned()
            })?;

        std::fs::create_dir_all(&keystore_path)
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
                .map(|(new_key, tg)| (new_key, tg))
            })
    }
}

#[tonic::async_trait]
impl Authentication for AuthService {
    async fn acquire_token(
        &self,
        request: tonic::Request<AcquireTokenParameters>,
    ) -> Result<tonic::Response<ProtoToken>, tonic::Status> {
        let scope = &request.get_ref().scope;
        let mut store = self.token_store.write().await;

        let token = store
            .acquire_token(scope, &self.scope_mappings, &self.logger)
            .await
            .map_err(tonic::Status::internal)?;

        Ok(tonic::Response::new(ProtoToken {
            token: token.token().to_owned(),
            expires_at: token.expires_at(),
            scope: request.get_ref().scope.clone(),
        }))
    }

    async fn authenticate(
        &self,
        request: tonic::Request<firm_types::auth::AuthenticationParameters>,
    ) -> Result<tonic::Response<firm_types::auth::AuthenticationResponse>, tonic::Status> {
        {
            let payload = request.into_inner();
            futures::future::ready(
                jsonwebtoken::decode_header(&payload.token)
                    .map_err(|e| {
                        tonic::Status::invalid_argument(format!(
                            "Failed decode token header: {}",
                            e
                        ))
                    })
                    .and_then(|header| {
                        header.kid.ok_or_else(|| {
                            tonic::Status::invalid_argument("Header is missing key id")
                        })
                    }),
            )
            .and_then(|key_id| async move {
                self.key_store
                    .get(&key_id)
                    .map_err(|e| {
                        tonic::Status::invalid_argument(format!(
                            "Failed to get public key for key id \"{}\": {}",
                            &key_id, e
                        ))
                    })
                    .await
            })
            .and_then(|key| {
                let token = payload.token.clone();
                let aud = payload.expected_audience.clone();
                async move {
                    jsonwebtoken::DecodingKey::from_ec_pem(&key)
                        .map_err(|e| {
                            tonic::Status::internal(format!("Failed to parse public key: {}", e))
                        })
                        .and_then(|decoding_key| {
                            let mut aud_hash = HashSet::new();
                            aud_hash.insert(aud);
                            jsonwebtoken::decode::<serde_json::Value>(
                                &token,
                                &decoding_key,
                                &jsonwebtoken::Validation {
                                    validate_exp: true,
                                    aud: Some(aud_hash),
                                    algorithms: vec![jsonwebtoken::Algorithm::ES256],
                                    ..Default::default()
                                },
                            )
                            .map_err(|e| {
                                tonic::Status::invalid_argument(format!(
                                    "JWT Validation failed: {}",
                                    e
                                ))
                            })
                        })
                }
            })
            .and_then(|claims| async move {
                claims
                    .claims
                    .get("sub")
                    .and_then(|sub| sub.as_str().map(|s| s.to_owned()))
                    .and_then(|sub| {
                        claims
                            .claims
                            .get("exp")
                            .and_then(|exp| exp.as_u64())
                            .map(|exp| (sub, exp))
                    })
                    .ok_or_else(|| {
                        tonic::Status::invalid_argument("JWT claims do not contain sub and exp")
                    })
            })
            .and_then(|(sub, exp)| {
                let create_remote_access_request = payload.create_remote_access_request;
                async move {
                    self.access_list
                        .contains(&sub)
                        .await
                        .then(|| None)
                        .or_else(|| {
                            create_remote_access_request.then(|| Some(uuid::Uuid::new_v4()))
                        })
                        .map(|maybe_rid| (maybe_rid, sub, exp))
                        .ok_or_else(|| tonic::Status::permission_denied(""))
                }
            })
            .and_then(|(maybe_request_id, sub, exp)| async move {
                OptionFuture::<_>::from(maybe_request_id.map(|uuid| async move {
                    self.pending_access_requests
                        .insert(
                            uuid,
                            PendingAccessRequest {
                                subject: sub.to_owned(),
                                expires_at: exp,
                                approved: false,
                            },
                            Some(
                                Instant::now()
                                    .checked_add(Duration::from_secs(
                                        exp - chrono::Utc::now().timestamp() as u64,
                                    ))
                                    .ok_or_else(|| {
                                        tonic::Status::internal(
                                            "Failed to calculate \
                                             expiry date for pending access request.",
                                        )
                                    })?,
                            ),
                        )
                        .await;
                    Ok(firm_types::auth::RemoteAccessRequestId {
                        uuid: uuid.to_string(),
                    })
                }))
                .await
                .transpose()
            })
            .await
            .map(|remote_access_request_id| {
                tonic::Response::new(firm_types::auth::AuthenticationResponse {
                    remote_access_request_id,
                })
            })
        }
    }

    async fn list_remote_access_requests(
        &self,
        request: tonic::Request<firm_types::auth::RemoteAccessListParameters>,
    ) -> Result<tonic::Response<firm_types::auth::RemoteAccessRequests>, tonic::Status> {
        let subject_filter = &request.get_ref().subject_filter;
        let include_approved = request.get_ref().include_approved;
        let mut results: Vec<firm_types::auth::RemoteAccessRequest> = self
            .pending_access_requests
            .snapshot()
            .await
            .iter()
            .filter(|(_, v)| {
                (subject_filter.is_empty() || v.subject.contains(subject_filter))
                    && (include_approved || !v.approved)
            })
            .map(|(k, v)| firm_types::auth::RemoteAccessRequest {
                id: Some(firm_types::auth::RemoteAccessRequestId {
                    uuid: k.to_string(),
                }),
                expires_at: v.expires_at,
                subject: v.subject.to_owned(),
                approved: v.approved,
            })
            .collect();
        match request.get_ref().order {
            Some(firm_types::auth::Ordering { key, reverse, .. })
                if key == firm_types::auth::OrderingKey::ExpiresAt as i32 =>
            {
                results.sort_unstable_by(|a, b| {
                    if reverse {
                        b.expires_at.cmp(&a.expires_at)
                    } else {
                        a.expires_at.cmp(&b.expires_at)
                    }
                })
            }
            Some(firm_types::auth::Ordering { reverse, .. }) => results.sort_unstable_by(|a, b| {
                if reverse {
                    b.subject.cmp(&a.subject)
                } else {
                    a.subject.cmp(&b.subject)
                }
            }),
            None => results.sort_unstable_by(|a, b| a.subject.cmp(&b.subject)),
        }

        Ok(tonic::Response::new(
            firm_types::auth::RemoteAccessRequests {
                requests: if let Some(ordering) = request.get_ref().order.as_ref() {
                    results
                        .into_iter()
                        .skip(ordering.offset as usize)
                        .take(if ordering.limit == 0 {
                            100
                        } else {
                            std::cmp::min(ordering.limit, 100)
                        } as usize)
                        .collect()
                } else {
                    results.into_iter().take(100).collect()
                },
            },
        ))
    }

    async fn approve_remote_access_request(
        &self,
        request: tonic::Request<firm_types::auth::RemoteAccessApproval>,
    ) -> Result<tonic::Response<firm_types::auth::RemoteAccessRequest>, tonic::Status> {
        futures::future::ready(
            request
                .get_ref()
                .id
                .as_ref()
                .ok_or_else(|| {
                    tonic::Status::invalid_argument("No Id on remote access approval request")
                })
                .map(|id| &id.uuid)
                .and_then(|uuid| {
                    uuid::Uuid::parse_str(&uuid).map_err(|e| {
                        tonic::Status::invalid_argument(format!("Invalid UUID: {}", e))
                    })
                }),
        )
        .and_then(|uuid| async move {
            futures::future::ready(
                self.pending_access_requests
                    .snapshot_mut()
                    .await
                    .get_mut(&uuid)
                    .ok_or_else(|| {
                        tonic::Status::not_found(format!(
                            "Failed to find pending access request with id: {}",
                            uuid
                        ))
                    }),
            )
            .and_then(|req| async move {
                req.approved = request.get_ref().approved;
                self.access_list
                    .insert(
                        req.subject.to_owned(),
                        (),
                        Some(
                            Instant::now()
                                .checked_add(Duration::from_secs(
                                    req.expires_at - chrono::Utc::now().timestamp() as u64,
                                ))
                                .ok_or_else(|| {
                                    tonic::Status::internal(
                                        "Failed to calculate \
                                             expiry date for pending access request.",
                                    )
                                })?,
                        ),
                    )
                    .await;
                Ok(firm_types::auth::RemoteAccessRequest {
                    id: Some(firm_types::auth::RemoteAccessRequestId {
                        uuid: uuid.to_string(),
                    }),
                    expires_at: req.expires_at,
                    subject: req.subject.to_owned(),
                    approved: req.approved,
                })
            })
            .await
        })
        .await
        .map(tonic::Response::new)
    }

    async fn get_remote_access_request(
        &self,
        request: tonic::Request<firm_types::auth::RemoteAccessRequestId>,
    ) -> Result<tonic::Response<firm_types::auth::RemoteAccessRequest>, tonic::Status> {
        futures::future::ready(
            uuid::Uuid::parse_str(&request.get_ref().uuid)
                .map_err(|e| tonic::Status::invalid_argument(format!("Invalid UUID: {}", e))),
        )
        .and_then(|uuid| async move {
            self.pending_access_requests
                .snapshot()
                .await
                .get(&uuid)
                .map(|req| firm_types::auth::RemoteAccessRequest {
                    id: Some(firm_types::auth::RemoteAccessRequestId {
                        uuid: uuid.to_string(),
                    }),
                    expires_at: req.expires_at,
                    subject: req.subject.to_owned(),
                    approved: req.approved,
                })
                .ok_or_else(|| {
                    tonic::Status::not_found(format!(
                        "Failed to find pending access request with id: {}",
                        uuid
                    ))
                })
        })
        .await
        .map(tonic::Response::new)
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
            &HashMap::new(),
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
            &HashMap::new(),
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
            &HashMap::new(),
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
