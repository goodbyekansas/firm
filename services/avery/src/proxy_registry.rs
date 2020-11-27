use futures::{stream, StreamExt, TryStreamExt};
use thiserror::Error;
use url::Url;

use crate::{config::ConflictResolutionMethod, registry::RegistryService};
use firm_types::{
    functions::{registry_client::RegistryClient, registry_server::Registry},
    tonic,
};
use slog::{info, o, Logger};
use std::collections::{hash_map::Entry, HashMap};
use tonic::transport::{ClientTlsConfig, Endpoint};
use tonic_middleware::HttpStatusInterceptor;
#[derive(Debug, Clone)]
pub struct ProxyRegistry {
    internal_registry: RegistryService,
    channels: Vec<RegistryClient<HttpStatusInterceptor>>,
    conflict_resolution: ConflictResolutionMethod,
    log: Logger,
}

#[derive(Debug, Clone)]
pub struct ExternalRegistry {
    name: String,
    url: Url,
    oauth_token: Option<String>,
}

#[derive(Error, Debug)]
pub enum ProxyRegistryError {
    #[error("Connection Error: {0}")]
    ConnectionError(#[from] tonic::transport::Error),

    #[error("Invalid URI: {0}")]
    InvalidUri(String),

    #[error("Invalid Oauth token: {0}")]
    InvalidOauthToken(String),

    #[error("Conflicting version name pair: {0}")] //TODO send info on registry names
    ConflictingFunctions(String),
}

async fn create_connection(
    registry: ExternalRegistry,
    log: Logger,
) -> Result<RegistryClient<HttpStatusInterceptor>, ProxyRegistryError> {
    let mut endpoint = Endpoint::from_shared(registry.url.to_string())
        .map_err(|e| ProxyRegistryError::InvalidUri(e.to_string()))?;

    if endpoint.uri().scheme_str() == Some("https") {
        endpoint = endpoint.tls_config(ClientTlsConfig::new())?;
    }

    // When calling non pure grpc endpoints we may get content that is not application/grpc.
    // Tonic doesn't handle these cases very well. We have to make a wrapper around
    // to handle these edge cases. We convert it into normal tonic statuses that tonic can handle.
    let channel = HttpStatusInterceptor::new(endpoint.connect().await?);

    let bearer = registry
        .oauth_token
        .map(|token| {
            tonic::metadata::MetadataValue::from_str(&format!("Bearer {}", token)).map_err(|e| {
                ProxyRegistryError::InvalidOauthToken(format!(
                    "Failed to convert oauth token to metadata value: {}",
                    e
                ))
            })
        })
        .transpose()?;
    Ok(match bearer {
        Some(bearer) => {
            info!(log, "Using provided oauth2 credentials 🧸");
            RegistryClient::with_interceptor(channel, move |mut req: tonic::Request<()>| {
                req.metadata_mut().insert("authorization", bearer.clone());
                Ok(req)
            })
        }

        None => RegistryClient::new(channel),
    })
}

impl ExternalRegistry {
    pub fn new(name: String, url: Url) -> Self {
        Self {
            name,
            url,
            oauth_token: None,
        }
    }
    pub fn new_with_oauth(name: String, url: Url, oauth_token: String) -> Self {
        Self {
            name,
            url,
            oauth_token: Some(oauth_token),
        }
    }
}

impl ProxyRegistry {
    pub async fn new(
        external_registries: Vec<ExternalRegistry>,
        internal_registry: RegistryService,
        conflict_resolution: ConflictResolutionMethod,
        log: Logger,
    ) -> Result<Self, ProxyRegistryError> {
        Ok(Self {
            internal_registry,
            channels: stream::iter(external_registries)
                .then(|er| {
                    let reg_name = er.name.clone();
                    create_connection(
                        er,
                        log.new(o!("scope" => "connect", "registry" => reg_name)),
                    )
                })
                .try_collect::<Vec<RegistryClient<HttpStatusInterceptor>>>()
                .await?,
            conflict_resolution,
            log,
        })
    }
}

#[tonic::async_trait]
impl Registry for ProxyRegistry {
    async fn list(
        &self,
        request: firm_types::tonic::Request<firm_types::functions::Filters>,
    ) -> Result<
        firm_types::tonic::Response<firm_types::functions::Functions>,
        firm_types::tonic::Status,
    > {
        let payload = request.into_inner();
        Ok(tonic::Response::new(firm_types::functions::Functions {
            functions:
                stream::iter(
                    self.channels
                        .iter()
                        .map(|client| (client.clone(), payload.clone())),
                )
                .then(|(mut client, payload)| async move {
                    client.list(tonic::Request::new(payload)).await
                })
                .chain(stream::once(
                    self.internal_registry
                        .list(tonic::Request::new(payload.clone())),
                ))
                .try_collect::<Vec<tonic::Response<firm_types::functions::Functions>>>()
                .await?
                .into_iter()
                .flat_map(|response| response.into_inner().functions)
                .try_fold(HashMap::new(), |mut hashmap, function| {
                    match hashmap.entry(format!("{}:{}", function.name, function.version)) {
                        Entry::Occupied(existing) => match self.conflict_resolution {
                            ConflictResolutionMethod::Error => Err(
                                ProxyRegistryError::ConflictingFunctions(existing.key().clone()),
                            ),
                            ConflictResolutionMethod::UsePriority => Ok(hashmap),
                        },
                        Entry::Vacant(vacant) => {
                            vacant.insert(function);
                            Ok(hashmap)
                        }
                    }
                })
                .map_err(|e| tonic::Status::already_exists(e.to_string()))?
                .into_iter()
                .map(|(_, v)| v)
                .collect::<Vec<firm_types::functions::Function>>(),
        }))
    }

    async fn get(
        &self,
        request: firm_types::tonic::Request<firm_types::functions::FunctionId>,
    ) -> Result<
        firm_types::tonic::Response<firm_types::functions::Function>,
        firm_types::tonic::Status,
    > {
        self.internal_registry.get(request).await
    }

    async fn register(
        &self,
        request: firm_types::tonic::Request<firm_types::functions::FunctionData>,
    ) -> Result<
        firm_types::tonic::Response<firm_types::functions::Function>,
        firm_types::tonic::Status,
    > {
        self.internal_registry.register(request).await
    }

    async fn register_attachment(
        &self,
        request: firm_types::tonic::Request<firm_types::functions::AttachmentData>,
    ) -> Result<
        firm_types::tonic::Response<firm_types::functions::AttachmentHandle>,
        firm_types::tonic::Status,
    > {
        self.internal_registry.register_attachment(request).await
    }

    async fn upload_streamed_attachment(
        &self,
        request: firm_types::tonic::Request<
            firm_types::tonic::Streaming<firm_types::functions::AttachmentStreamUpload>,
        >,
    ) -> Result<
        firm_types::tonic::Response<firm_types::functions::Nothing>,
        firm_types::tonic::Status,
    > {
        self.internal_registry
            .upload_streamed_attachment(request)
            .await
    }
}
