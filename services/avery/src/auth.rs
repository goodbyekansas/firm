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

use self::oidc::Oidc;
use crate::config::{Auth as AuthConfig, OidcProvider};

mod internal;
mod oidc;

#[derive(Clone)]
pub struct AuthService {
    oidc_providers: Arc<HashMap<String, Oidc>>,
    auth_providers: Arc<HashMap<String, AuthConfig>>,
    tokens: Arc<RwLock<HashMap<String, Box<dyn Token>>>>,
    self_signed: Arc<internal::TokenGenerator>,
    self_signed_with_file: Arc<HashMap<PathBuf, internal::TokenGenerator>>,
    logger: Logger,
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
}

impl AuthService {
    pub fn new(
        oidc_providers: HashMap<String, OidcProvider>,
        auth_providers: HashMap<String, AuthConfig>,
        logger: Logger,
    ) -> Result<Self, String> {
        let self_signed_with_file = Arc::new(
            auth_providers
                .iter()
                .filter_map(|(_, value)| match value {
                    AuthConfig::KeyFile { path } => Some(
                        internal::TokenGeneratorBuilder::new()
                            .with_rsa_private_key_from_file(path)
                            .build()
                            .map(|token_generator| (path.to_owned(), token_generator)),
                    ),
                    _ => None,
                })
                .collect::<Result<HashMap<_, _>, internal::SelfSignedTokenError>>()
                .map_err(|e| e.to_string())?,
        );
        let keystore_path = crate::system::user_data_path()
            .map(|p| p.join("keys"))
            .ok_or_else(|| "Could determine key store path for saving generated keys".to_owned())?;
        let self_signed = std::fs::create_dir_all(&keystore_path)
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
                    internal::TokenGeneratorBuilder::new()
                        .with_ecdsa_private_key_from_file(&private_key_path)
                        .build()
                        .map_err(|e| e.to_string())
                } else {
                    internal::TokenGeneratorBuilder::new()
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
                            Ok(tg)
                        })
                }
                .map(Arc::new)
            })?;

        Ok(Self {
            tokens: Arc::new(RwLock::new(HashMap::new())),
            oidc_providers: Arc::new(
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
            auth_providers: Arc::new(auth_providers),
            self_signed,
            self_signed_with_file,
            logger,
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
        let mut cache = self.tokens.write().await;
        let auth = match cache.entry(scope.clone()) {
            std::collections::hash_map::Entry::Occupied(mut e) => {
                debug!(
                    self.logger,
                    "Found cached token for scope \"{}\", expires at {}",
                    &request.get_ref().scope,
                    Utc.timestamp(e.get().expires_at() as i64, 0).to_string(),
                );
                e.get_mut().refresh().await.map_err(|err| {
                    tonic::Status::internal(format!(
                        "Failed to refresh token for scope \"{}\": {}",
                        &request.get_ref().scope,
                        err
                    ))
                })?;
                e.into_mut()
            }
            std::collections::hash_map::Entry::Vacant(e) => match self.auth_providers.get(scope) {
                Some(AuthConfig::None) | None => {
                    return Ok(tonic::Response::new(ProtoToken {
                        token: "".to_owned(),
                        expires_at: 0,
                        scope: scope.clone(),
                    }));
                }
                Some(AuthConfig::Oidc { provider }) => e.insert(Box::new(
                    futures::future::ready(self.oidc_providers.get(provider).ok_or_else(|| {
                        tonic::Status::internal(format!(
                            "Oidc provider \"{}\" not found.",
                            provider
                        ))
                    }))
                    .and_then(|oidc_client| {
                        oidc_client
                            .authenticate()
                            .map_err(|e| tonic::Status::internal(e.to_string()))
                    })
                    .await?,
                )),
                Some(AuthConfig::SelfSigned) => e.insert(
                    self.self_signed
                        .generate(scope)
                        .map_err(|e| tonic::Status::internal(e.to_string()))
                        .map(Box::new)?,
                ),
                Some(AuthConfig::KeyFile { path }) => e.insert(
                    self.self_signed_with_file
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
                ),
            },
        };

        Ok(tonic::Response::new(ProtoToken {
            token: auth.token().to_owned(),
            expires_at: auth.expires_at(),
            scope: request.get_ref().scope.clone(),
        }))
    }
}
