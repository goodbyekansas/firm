use std::collections::{hash_map::Entry, HashMap};

use firm_types::{
    auth::authentication_server::Authentication,
    auth::AcquireTokenParameters,
    functions::{
        registry_client::RegistryClient, registry_server::Registry, AttachmentData,
        AttachmentHandle, AttachmentStreamUpload, Filters, Function, Functions, Nothing, Ordering,
        OrderingKey,
    },
    tonic::{
        self,
        codegen::InterceptedService,
        metadata::AsciiMetadataValue,
        service::Interceptor,
        transport::{ClientTlsConfig, Endpoint, Error as TonicTransportError},
        Code, Request, Response, Status, Streaming,
    },
};
use futures::{stream, StreamExt, TryStreamExt};
use slog::{warn, Logger};
use thiserror::Error;
use tokio::runtime::Handle;
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
    ConnectionError(#[from] TonicTransportError),

    #[error("Invalid URI: {0}")]
    InvalidUri(String),

    #[error("Invalid Oauth token: {0}")]
    InvalidOauthToken(String),

    #[error(r#"Conflicting version name pair: "{0}" in registries "{1}" and "{2}""#)]
    ConflictingFunctions(String, String, String),

    #[error("Version parse error: {0}")]
    VersionParseError(String),
}

#[derive(Debug, Clone)]
struct AcquireAuthInterceptor {
    auth_service: AuthService,
    endpoint: Endpoint,
}

pub enum ListFunction {
    Functions,
    Versions,
}

impl Interceptor for AcquireAuthInterceptor {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        /*
         * TODO: Block on calls are really ugly. Unfortunately Interceptor does
         * not take async block and we need to use async functionality.
         * This can be fixed by implementing a tower Service as in
         * https://github.com/hyperium/tonic/blob/c62f382e3c6e9c0641decfafb2b8396fe52b6314/examples/src/tower/client.rs#L61
         */
        self.endpoint
            .uri()
            .host()
            .ok_or_else(|| {
                tonic::Status::internal("No host for registry endpoint, cannot acquire token")
            })
            .and_then(|host| {
                let host = host.to_owned();
                let auth = self.auth_service.clone();
                let handle = Handle::current();
                std::thread::spawn(move || {
                    handle.block_on(async {
                        auth.acquire_token(tonic::Request::new(AcquireTokenParameters {
                            scope: host.clone(),
                        }))
                        .await
                    })
                })
                .join()
                .map_err(|_| tonic::Status::internal("Failed to join acquire token thread"))?
            })
            .and_then(|token| {
                AsciiMetadataValue::from_str(&format!("bearer {}", token.get_ref().token)).map_err(
                    |e| {
                        tonic::Status::internal(format!(
                            "Invalid metadata value for bearer token: {}",
                            e
                        ))
                    },
                )
            })
            .map(|token| {
                request.metadata_mut().insert("authorization", token);
                request
            })
    }
}

async fn create_connection(
    auth_service: AuthService,
    registry: ExternalRegistry,
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
                        client: create_connection(auth_service.clone(), er).await?,
                    })
                })
                .try_collect::<Vec<RegistryConnection>>()
                .await?,
            conflict_resolution,
            log,
        })
    }

    fn try_insert_function(
        &self,
        mut hashmap: HashMap<String, Function>,
        function: Function,
    ) -> Result<HashMap<String, Function>, ProxyRegistryError> {
        match hashmap.entry(function.name.clone()) {
            Entry::Occupied(mut existing) => {
                let version = semver::Version::parse(&function.version)
                    .map_err(|e| ProxyRegistryError::VersionParseError(e.to_string()))?;
                match semver::Version::parse(&existing.get().version) {
                    Ok(existing_version) if existing_version < version => {
                        existing.insert(function);
                        Ok(hashmap)
                    }
                    Ok(existing_version) if existing_version == version => {
                        match self.conflict_resolution {
                            ConflictResolutionMethod::Error => {
                                Err(ProxyRegistryError::ConflictingFunctions(
                                    existing.key().clone(),
                                    function
                                        .metadata
                                        .get("registry")
                                        .unwrap_or(&String::from("unknown"))
                                        .to_owned(),
                                    existing
                                        .get()
                                        .metadata
                                        .get("registry")
                                        .unwrap_or(&String::from("unknown"))
                                        .to_owned(),
                                ))
                            }
                            ConflictResolutionMethod::UsePriority => Ok(hashmap),
                        }
                    }
                    _ => Ok(hashmap),
                }
            }
            Entry::Vacant(vacant) => {
                vacant.insert(function);
                Ok(hashmap)
            }
        }
    }

    fn try_insert_version(
        &self,
        mut hashmap: HashMap<String, Function>,
        function: Function,
    ) -> Result<HashMap<String, Function>, ProxyRegistryError> {
        match hashmap.entry(format!("{}:{}", function.name, function.version)) {
            Entry::Occupied(existing) => match self.conflict_resolution {
                ConflictResolutionMethod::Error => Err(ProxyRegistryError::ConflictingFunctions(
                    existing.key().clone(),
                    function
                        .metadata
                        .get("registry")
                        .unwrap_or(&String::from("unknown"))
                        .to_owned(),
                    existing
                        .get()
                        .metadata
                        .get("registry")
                        .unwrap_or(&String::from("unknown"))
                        .to_owned(),
                )),
                ConflictResolutionMethod::UsePriority => Ok(hashmap),
            },
            Entry::Vacant(vacant) => {
                vacant.insert(function);
                Ok(hashmap)
            }
        }
    }

    pub async fn list(
        &self,
        filters: Filters,
        list_function: &ListFunction,
    ) -> Result<Functions, tonic::Status> {
        let insert_function = match list_function {
            ListFunction::Functions => ProxyRegistry::try_insert_function,
            ListFunction::Versions => ProxyRegistry::try_insert_version,
        };
        let mut functions = stream::iter(
            self.connections
                .iter()
                .map(|connection| (connection.clone(), filters.clone())),
        )
        .then(|(mut connection, filters)| async move {
            match list_function {
                ListFunction::Functions => connection
                    .client
                    .list(tonic::Request::new(filters))
                    .await
                    .map(|functions| (connection.name.clone(), functions)),
                ListFunction::Versions => connection
                    .client
                    .list_versions(tonic::Request::new(filters))
                    .await
                    .map(|functions| (connection.name.clone(), functions)),
            }
        })
        .chain(
            stream::once(
                self.internal_registry
                    .list(tonic::Request::new(filters.clone())),
            )
            .map(|f| f.map(|functions| (String::from("internal"), functions))),
        )
        .try_collect::<Vec<(String, tonic::Response<Functions>)>>()
        .await?
        .into_iter()
        .map(|(name, mut functions)| {
            // insert registry into metadata
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
        .try_fold(HashMap::new(), |map, func| insert_function(self, map, func))
        .map_err(|e| tonic::Status::already_exists(e.to_string()))?
        .into_values()
        .map(|function| {
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
        .collect::<Vec<(semver::Version, Function)>>();

        // redo sorting, offset and limit since we do not know
        // anything about the relational ordering between different
        // registries
        let order = filters.order.unwrap_or(Ordering {
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

        Ok(Functions {
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
        })
    }
}

/// Implementation of Registry as a proxy
///
/// This basically forwards all calls to an internal registry except
/// `list` and `get` where external registries and internal is combined
#[tonic::async_trait]
impl Registry for ProxyRegistry {
    async fn list(&self, request: Request<Filters>) -> Result<Response<Functions>, Status> {
        ProxyRegistry::list(self, request.into_inner(), &ListFunction::Functions)
            .await
            .map(tonic::Response::new)
    }

    async fn list_versions(
        &self,
        request: Request<Filters>,
    ) -> Result<Response<Functions>, Status> {
        ProxyRegistry::list(self, request.into_inner(), &ListFunction::Versions)
            .await
            .map(Response::new)
    }

    async fn get(
        &self,
        request: Request<firm_types::functions::FunctionId>,
    ) -> Result<Response<Function>, Status> {
        let payload = request.into_inner();

        let res = stream::iter(
            self.connections
                .iter()
                .map(|client| (client.clone(), payload.clone())),
        )
        .then(|(mut connection, payload)| async move {
            connection
                .client
                .get(Request::new(payload))
                .await
                .map(|functions| (connection.name.clone(), functions))
        })
        .chain(
            stream::once(self.internal_registry.get(Request::new(payload.clone())))
                .map(|f| f.map(|functions| (String::from("internal"), functions))),
        )
        .collect::<Vec<Result<(String, Response<Function>), Status>>>()
        .await
        .into_iter()
        .filter(|v| !matches!(v, Err(e) if e.code() == Code::NotFound))
        .collect::<Result<Vec<(String, Response<Function>)>, Status>>()?
        .into_iter()
        .map(|(registry_name, response)| {
            let mut r = response.into_inner();
            r.metadata.insert("registry".to_owned(), registry_name);
            r
        })
        .try_fold(
            HashMap::new(),
            |mut hashmap: HashMap<String, Function>, function| match hashmap
                .entry(format!("{}:{}", function.name, function.version))
            {
                Entry::Occupied(existing) => match self.conflict_resolution {
                    ConflictResolutionMethod::Error => {
                        Err(ProxyRegistryError::ConflictingFunctions(
                            existing.key().clone(),
                            function
                                .metadata
                                .get("registry")
                                .unwrap_or(&String::from("unknown"))
                                .to_owned(),
                            existing
                                .get()
                                .metadata
                                .get("registry")
                                .unwrap_or(&String::from("unknown"))
                                .to_owned(),
                        ))
                    }
                    ConflictResolutionMethod::UsePriority => Ok(hashmap),
                },
                Entry::Vacant(vacant) => {
                    vacant.insert(function);
                    Ok(hashmap)
                }
            },
        )
        .map_err(|e| Status::already_exists(e.to_string()))?
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
        request: Request<firm_types::functions::FunctionData>,
    ) -> Result<Response<Function>, Status> {
        self.internal_registry.register(request).await
    }

    async fn register_attachment(
        &self,
        request: Request<AttachmentData>,
    ) -> Result<Response<AttachmentHandle>, Status> {
        self.internal_registry.register_attachment(request).await
    }

    async fn upload_streamed_attachment(
        &self,
        request: Request<Streaming<AttachmentStreamUpload>>,
    ) -> Result<Response<Nothing>, Status> {
        self.internal_registry
            .upload_streamed_attachment(request)
            .await
    }
}
