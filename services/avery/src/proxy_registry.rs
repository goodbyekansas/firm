use std::collections::{hash_map::Entry, HashMap};

use firm_types::{
    auth::authentication_server::Authentication,
    auth::AcquireTokenParameters,
    functions::{
        registry_client::RegistryClient, registry_server::Registry, Functions, Ordering,
        OrderingKey,
    },
    tonic::{self, codegen::InterceptedService, service::Interceptor, Status},
};
use futures::{stream, StreamExt, TryStreamExt};
use slog::{o, warn, Logger};
use thiserror::Error;
use tokio::runtime::Handle;
use tonic::{
    metadata::AsciiMetadataValue,
    transport::{ClientTlsConfig, Endpoint},
};
use tonic_middleware::HttpStatusInterceptor;
use url::Url;

use crate::{auth::AuthService, config::ConflictResolutionMethod, registry::RegistryService};

type RegClient = RegistryClient<InterceptedService<HttpStatusInterceptor, AcquireAuthInterceptor>>;

#[derive(Debug, Clone)]
struct RegistryConnection {
    name: String,
    client: RegClient,
}

/// A forwarding proxy registry
///
/// The registry supports forwarding `list` and `get` calls to a list of external registries and
/// then combining it with the built-in internal registry
#[derive(Debug, Clone)]
pub struct ProxyRegistry {
    internal_registry: RegistryService,
    connections: Vec<RegistryConnection>,
    conflict_resolution: ConflictResolutionMethod,
    log: Logger,
}

/// Representation of an external registry
#[derive(Debug, Clone)]
pub struct ExternalRegistry {
    name: String,
    url: Url,
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

#[derive(Debug, Clone)]
struct AcquireAuthInterceptor {
    auth_service: AuthService,
    logger: Logger,
    endpoint: Endpoint,
}

impl Interceptor for AcquireAuthInterceptor {
    fn call(&mut self, mut request: tonic::Request<()>) -> Result<tonic::Request<()>, Status> {
        /*
         * TODO: Block on calls are really ugly. Unfortunately Interceptor does
         * not take async block and we need to use async functionality.
         * This can be fixed by implementing a tower Service as in
         * https://github.com/hyperium/tonic/blob/c62f382e3c6e9c0641decfafb2b8396fe52b6314/examples/src/tower/client.rs#L61
         */
        if let Some(token) = self
            .endpoint
            .uri()
            .host()
            .and_then(|host| {
                let host = host.to_owned();
                let auth = self.auth_service.clone();
                let handle = Handle::current();
                let logger = self.logger.new(o!());
                std::thread::spawn(move || {
                    handle
                        .block_on(async {
                            auth.acquire_token(tonic::Request::new(AcquireTokenParameters {
                                scope: host.clone(),
                            }))
                            .await
                        })
                        .map_err(|e| {
                            warn!(
                                logger,
                                "Requesting auth for scope \"{}\" failed with error: {}", host, e
                            );
                            e
                        })
                        .ok()
                })
                .join()
                .ok()
                .and_then(|op| op)
            })
            .and_then(|result| {
                AsciiMetadataValue::from_str(&format!("bearer {}", &result.get_ref().token)).ok()
            })
        {
            request.metadata_mut().insert("authorization", token);
        }

        Ok(request)
    }
}

async fn create_connection(
    auth_service: AuthService,
    registry: ExternalRegistry,
    logger: Logger,
) -> Result<RegClient, ProxyRegistryError> {
    let mut endpoint = Endpoint::from_shared(registry.url.to_string())
        .map_err(|e| ProxyRegistryError::InvalidUri(e.to_string()))?;

    if endpoint.uri().scheme_str() == Some("https") {
        endpoint = endpoint.tls_config(ClientTlsConfig::new())?;
    }

    // When calling non pure grpc endpoints we may get content that is not application/grpc.
    // Tonic doesn't handle these cases very well. We have to make a wrapper around
    // to handle these edge cases. We convert it into normal tonic statuses that tonic can handle.
    Ok(RegistryClient::with_interceptor(
        tower::ServiceBuilder::new()
            .layer_fn(HttpStatusInterceptor::new)
            .service(endpoint.connect_lazy()),
        AcquireAuthInterceptor {
            auth_service,
            logger,
            endpoint,
        },
    ))
}

impl ExternalRegistry {
    /// Create a new external registry descriptor
    ///
    /// # Parameters
    /// `name`: A semantic name for the registry (used to identify this registry in list results)
    /// `url`: A url pointing to the external registry
    pub fn new(name: String, url: Url) -> Self {
        Self { name, url }
    }
}

impl ProxyRegistry {
    /// Create a new proxy registry
    ///
    /// # Parameters
    /// `external_registries`: A list of external registry descriptors. The order in the list will
    /// be used as the priority order when handling conflicting versions of functions.
    /// `internal_registry`: An implementation of the internal registry. This is called directly,
    /// requiring no network access.
    /// `conflict_resolution`: The conflict resolution method to use when functions with the same
    /// name and version are found in different registries.
    pub async fn new(
        external_registries: Vec<ExternalRegistry>,
        internal_registry: RegistryService,
        conflict_resolution: ConflictResolutionMethod,
        auth_service: AuthService,
        log: Logger,
    ) -> Result<Self, ProxyRegistryError> {
        Ok(Self {
            internal_registry,
            connections: stream::iter(external_registries)
                .then(|er| async {
                    let reg_name = er.name.clone();

                    Ok::<RegistryConnection, ProxyRegistryError>(RegistryConnection {
                        name: reg_name.clone(),
                        client: create_connection(
                            auth_service.clone(),
                            er,
                            log.new(o!("registry" => reg_name)),
                        )
                        .await?,
                    })
                })
                .try_collect::<Vec<RegistryConnection>>()
                .await?,
            conflict_resolution,
            log,
        })
    }
}

/// Implementation of Registry as a proxy
///
/// This basically forwards all calls to an internal registry except
/// `list` and `get` where external registries and internal is combined
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
        let mut functions = stream::iter(
            self.connections
                .iter()
                .map(|connection| (connection.clone(), payload.clone())),
        )
        .then(|(mut connection, payload)| async move {
            connection
                .client
                .list(tonic::Request::new(payload))
                .await
                .map(|functions| (connection.name.clone(), functions))
        })
        .chain(
            stream::once(
                self.internal_registry
                    .list(tonic::Request::new(payload.clone())),
            )
            .map(|f| f.map(|functions| (String::from("internal"), functions))),
        )
        .try_collect::<Vec<(String, tonic::Response<firm_types::functions::Functions>)>>()
        .await?
        .into_iter()
        .map(|(name, mut functions)| {
            // insert metadata
            functions
                .get_mut()
                .functions
                .iter_mut()
                .for_each(|function| {
                    function
                        .metadata
                        .insert("registry".to_owned(), name.clone());
                });

            functions
        })
        .flat_map(|functions| functions.into_inner().functions)
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
        .map(|(_, function)| {
            (
                // The version is only used for sorting so if something is wrong with it,
                // jus sort it last. This error is also very unlikely since the version is parsed
                // and validated when the function is registered
                semver::Version::parse(&function.version).unwrap_or_else(|_| {
                    let mut v = semver::Version::new(0, 0, 1);
                    v.pre
                        .push(semver::Identifier::AlphaNumeric(String::from("invalid")));
                    v
                }),
                function,
            )
        })
        .collect::<Vec<(semver::Version, firm_types::functions::Function)>>();

        // redo sorting, offset and limit since we do not know
        // anything about the relational ordering between different
        // registries
        let order = payload.order.unwrap_or_else(|| Ordering {
            key: OrderingKey::NameVersion as i32,
            reverse: false,
            offset: 0,
            limit: 100,
        });
        let offset: usize = order.offset as usize;
        let limit: usize = order.limit as usize;

        if OrderingKey::from_i32(order.key).is_none() {
            warn!(
                self.log,
                "Ordering key was out of range ({}). Out of date protobuf definitions?", order.key
            );
        }

        functions.sort_unstable_by(|(a_semver, a_function), (b_semver, b_function)| {
            match OrderingKey::from_i32(order.key) {
                Some(OrderingKey::NameVersion) | None => {
                    match a_function.name.cmp(&b_function.name) {
                        std::cmp::Ordering::Equal => b_semver.cmp(a_semver),
                        o => o,
                    }
                }
            }
        });

        Ok(tonic::Response::new(Functions {
            functions: if order.reverse {
                functions
                    .into_iter()
                    .map(|(_version, function)| function)
                    .rev()
                    .skip(offset)
                    .take(limit)
                    .collect::<Vec<_>>()
            } else {
                functions
                    .into_iter()
                    .map(|(_version, function)| function)
                    .skip(offset)
                    .take(limit)
                    .collect::<Vec<_>>()
            },
        }))
    }

    async fn get(
        &self,
        request: firm_types::tonic::Request<firm_types::functions::FunctionId>,
    ) -> Result<
        firm_types::tonic::Response<firm_types::functions::Function>,
        firm_types::tonic::Status,
    > {
        let payload = request.into_inner();

        let res = stream::iter(
            self.connections
                .iter()
                .map(|client| (client.clone(), payload.clone())),
        )
        .then(|(mut connection, payload)| async move {
            connection
                .client
                .get(tonic::Request::new(payload))
                .await
                .map(|functions| (connection.name.clone(), functions))
        })
        .chain(
            stream::once(
                self.internal_registry
                    .get(tonic::Request::new(payload.clone())),
            )
            .map(|f| f.map(|functions| (String::from("internal"), functions))),
        )
        .collect::<Vec<Result<(String, tonic::Response<firm_types::functions::Function>), tonic::Status>>>()
        .await
        .into_iter()
        .filter(|v| !matches!(v, Err(e) if e.code() == tonic::Code::NotFound))
        .collect::<Result<Vec<(String, tonic::Response<firm_types::functions::Function>)>, tonic::Status>>()?
        .into_iter()
        .map(|(registry_name, response)| {
            let mut r = response.into_inner();
            r.metadata.insert("registry".to_owned(), registry_name);
            r
        })
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
        .next()
        .ok_or_else(|| {
            tonic::Status::not_found(format!(
                "Failed to find function with name: \"{}\" and version \"{}\"",
                payload.name, payload.version
            ))
        })?; // We've already handled the case where we find several. First should be safe to call.
        Ok(tonic::Response::new(res))
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
