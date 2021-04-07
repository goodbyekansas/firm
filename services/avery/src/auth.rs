use std::{collections::HashMap, sync::Arc};

use chrono::{TimeZone, Utc};
use firm_types::{
    auth::authentication_server::Authentication, auth::AcquireTokenParameters,
    auth::Token as ProtoToken, tonic,
};
use futures::TryFutureExt;
use slog::{debug, o, Logger};
use tokio::sync::RwLock;

use crate::{config::OidcConfig, oidc::Oidc};

#[derive(Clone)]
pub struct AuthService {
    oidc_mappings: Arc<HashMap<String, Oidc>>,
    tokens: Arc<RwLock<HashMap<String, Box<dyn Token>>>>,
    logger: Logger,
}

#[async_trait::async_trait]
pub trait Token: Send + Sync {
    fn token(&self) -> &str;
    async fn refresh(&mut self) -> Result<&mut dyn Token, String>;
    fn expires_at(&self) -> u64;
}

impl AuthService {
    pub fn new(
        oidc_mappings: HashMap<String, OidcConfig>,
        logger: Logger,
    ) -> Result<Self, url::ParseError> {
        Ok(Self {
            tokens: Arc::new(RwLock::new(HashMap::new())),
            oidc_mappings: Arc::new(
                oidc_mappings
                    .into_iter()
                    .map(|(key, value)| {
                        (
                            key.clone(),
                            Oidc::new(&value, logger.new(o!("scope" => "oidc", "host" => key))),
                        )
                    })
                    .collect::<HashMap<String, Oidc>>(),
            ),
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
        let mut cache = self.tokens.write().await;
        let auth = match cache.entry(request.get_ref().scope.clone()) {
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
            std::collections::hash_map::Entry::Vacant(e) => e.insert(Box::new(
                futures::future::ready(
                    self.oidc_mappings
                        .get(&request.get_ref().scope)
                        .ok_or_else(|| {
                            tonic::Status::unimplemented(
                                "Scope not found. Self signed token not implemented yet.",
                            )
                        }),
                )
                .and_then(|oidc_client| {
                    oidc_client
                        .authenticate()
                        .map_err(|e| tonic::Status::internal(e.to_string()))
                })
                .await?,
            )),
        };

        Ok(tonic::Response::new(ProtoToken {
            token: auth.token().to_owned(),
            expires_at: auth.expires_at(),
            scope: request.get_ref().scope.clone(),
        }))
    }
}
