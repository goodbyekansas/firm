use std::{
    collections::HashMap, convert::Infallible, path::PathBuf, str::FromStr, sync::Arc,
    time::Duration,
};

use chrono::Utc;
use firm_types::{
    auth::authentication_client::AuthenticationClient,
    auth::RemoteAccessRequestId,
    tonic::{self, transport::Channel, Status},
};
use futures::TryFutureExt;
use http::{Request, Response, Uri};
use hyper::{
    body::HttpBody,
    server::Server,
    service::{make_service_fn, service_fn},
    Body, Client,
};
use serde::Deserialize;
use slog::{debug, error, info, o, warn, Drain, Logger};
use structopt::StructOpt;
use tokio::sync::RwLock;
use tonic::{body::BoxBody, transport::Endpoint};

use crate::config;
use crate::tls;

#[cfg(windows)]
use crate::windows;

#[cfg(unix)]
use crate::unix;

#[cfg(windows)]
use windows as system;

#[cfg(unix)]
use unix as system;

type TokenCache = Arc<RwLock<HashMap<String, (i64, Client<LocalAveryConnector>)>>>;

#[derive(Debug, StructOpt)]
pub struct LomaxArgs {
    /// Config file to use for the proxy
    #[structopt(short, long)]
    config: Option<PathBuf>,

    /// Port to bind the proxy to
    ///
    /// This takes precedence over config settings
    #[structopt(short, long)]
    port: Option<u16>,

    /// Start Lomax as a windows service
    ///
    /// This will try to talk to the service control manager and can not be run manually
    #[cfg(windows)]
    #[structopt(short = "s", long = "service")]
    pub service: bool,
}

#[derive(Debug)]
pub enum ProxyError {
    GrpcError(Status),
    HttpError(hyper::Error),
}

trait IntoHyperResult {
    fn into_http_result(self) -> Result<Response<BoxBody>, hyper::Error>;
}

impl IntoHyperResult for Result<Response<BoxBody>, ProxyError> {
    fn into_http_result(self) -> Result<Response<BoxBody>, hyper::Error> {
        match self {
            Ok(x) => Ok(x),
            Err(ProxyError::GrpcError(status)) => Ok(status.to_http()),
            Err(ProxyError::HttpError(error)) => Err(error),
        }
    }
}

#[derive(Clone)]
pub struct LocalAveryConnector {
    pub uri: Uri,
    pub log: Logger,
}

impl LocalAveryConnector {
    pub fn new(uri: &Uri, log: Logger) -> Self {
        Self {
            uri: uri.clone(),
            log,
        }
    }
}

pub type LocalConnectorFuture<C> = std::pin::Pin<
    Box<
        dyn std::future::Future<
                Output = Result<
                    <C as hyper::service::Service<http::Uri>>::Response,
                    <C as hyper::service::Service<http::Uri>>::Error,
                >,
            > + Send,
    >,
>;

pub fn create_logger() -> Logger {
    Logger::root(
        slog_async::Async::new(
            slog_term::FullFormat::new(slog_term::TermDecorator::new().build())
                .build()
                .fuse(),
        )
        .build()
        .fuse(),
        o!(),
    )
}

fn get_connector(uri: &Uri, log: Logger) -> Option<LocalAveryConnector> {
    match uri.scheme_str() {
        #[cfg(unix)]
        Some("unix") => Some(LocalAveryConnector::new(uri, log)),

        #[cfg(windows)]
        Some("windows") => Some(LocalAveryConnector::new(uri, log)),

        _ => None,
    }
}

async fn grpc_connect(endpoint: Endpoint, logger: Logger) -> Result<Channel, tonic::Status> {
    let uri = endpoint.uri().clone();
    let connector = get_connector(
        &uri,
        logger.new(o!("scope" => "connector", "uri" => uri.to_string())),
    )
    .ok_or_else(|| {
        tonic::Status::aborted(format!(
            "Unsupported uri scheme, expected windows or unix, got \"{}\"",
            uri.scheme_str().unwrap_or_default()
        ))
    })?;

    endpoint
        .connect_with_connector(connector)
        .map_err(|e| {
            tonic::Status::unavailable(format!(
                "Failed to make local connection  @ \"{}\": {}",
                uri.path(),
                e
            ))
        })
        .await
}

struct Token(String);

#[derive(Deserialize)]
struct Claims {
    aud: String,
    sub: String,
    exp: i64,
}

#[derive(Clone)]
struct UserHostPair {
    username: String,
    hostname: String,
}

impl FromStr for Token {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lowercase_header = s.to_lowercase();
        if lowercase_header.starts_with("bearer ") {
            Ok(Token(
                s.split_whitespace().last().unwrap_or_default().to_owned(),
            ))
        } else {
            Err("Invalid authorization format".to_owned())
        }
    }
}

impl FromStr for UserHostPair {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.split('@').collect::<Vec<&str>>();
        if s.len() == 2 {
            Ok(UserHostPair {
                username: s[0].to_owned(),
                hostname: s[1].to_owned(),
            })
        } else {
            Err("Invalid audience format".to_owned())
        }
    }
}

fn get_user_socket(template: &str, username: &str) -> String {
    template.replace("{username}", username)
}

async fn wait_for_approval(
    auth_client: &mut AuthenticationClient<Channel>,
    id: RemoteAccessRequestId,
    timeout: Duration,
) -> Result<(), Status> {
    let mut req = tonic::Request::new(id.clone());
    req.set_timeout(timeout);

    auth_client
        .wait_for_remote_access_request(req)
        .await
        .map(|_| ())
}

async fn auth_against_avery(
    endpoint: Endpoint,
    user_host_pair: UserHostPair,
    token: String,
    config: Arc<config::Config>,
    logger: Logger,
) -> Option<Client<LocalAveryConnector>> {
    grpc_connect(endpoint, logger.new(o!("scope" => "grpc-connect")))
        .map_ok(AuthenticationClient::new)
        .and_then(|auth_client| {
            let user_host_pair = user_host_pair.clone();
            let mut auth_client2 = auth_client.clone();
            async move {
                auth_client2
                    .authenticate(tonic::Request::new(
                        firm_types::auth::AuthenticationParameters {
                            expected_audience: format!(
                                "{}@{}",
                                &user_host_pair.username, &user_host_pair.hostname,
                            ),
                            token,
                            create_remote_access_request: true,
                        },
                    ))
                    .await
            }
            .map_ok(|response| (auth_client, response))
        })
        .and_then(|(mut auth_client, response)| {
            let logger = logger.new(o!("scope" => "request-approval"));
            async move {
                let result = response.into_inner();
                match result.remote_access_request_id {
                    Some(id) => {
                        info!(
                            logger,
                            "Remote access request with id \"{}\" created. \
                             Waiting for approval.",
                            id.uuid
                        );

                        let approval_timeout = Duration::from_secs(2 * 60);

                        let mut auth_client2 = auth_client.clone();
                        wait_for_approval(&mut auth_client2, id.clone(), approval_timeout)
                            .or_else(|e| {
                                let id = id.clone();
                                async move {
                                    match e.code() {
                                        tonic::Code::Cancelled => {
                                            let _ = auth_client
                                                .cancel_remote_access_request(tonic::Request::new(
                                                    id.clone(),
                                                ))
                                                .await;
                                            Err(Status::deadline_exceeded(format!(
                                                "Remote access request with id \"{}\" \
                                                 was not approved after waiting for {} seconds",
                                                id.uuid,
                                                approval_timeout.as_secs()
                                            )))
                                        }
                                        _ => Err(Status::internal(format!(
                                            "Failed to wait for access request with id \"{}\": {}",
                                            id.uuid, e
                                        ))),
                                    }
                                }
                            })
                            .map_ok(|val| {
                                info!(
                                    logger,
                                    "Remote access request with id \"{}\" \
                                     was approved.",
                                    id.uuid
                                );
                                val
                            })
                            .await
                    }
                    None => Ok(()),
                }
            }
        })
        .await
        .and_then(|_| {
            let uri = get_user_socket(&config.user_socket_url, &user_host_pair.username);
            let sock = Uri::from_maybe_shared(uri.clone()).map_err(|e| {
                tonic::Status::internal(format!("Invalid local socket URI \"{}\": {}", uri, e))
            })?;
            get_connector(&sock, logger.new(o!("scope" => "connector", "uri" => uri))).ok_or_else(
                || {
                    tonic::Status::aborted(format!(
                        "Unsupported uri scheme, expected windows or unix, got \"{}\"",
                        sock.scheme_str().unwrap_or_default()
                    ))
                },
            )
        })
        .map(|connector| {
            debug!(logger, "Using backend uri \"{}\"", connector.uri);
            Client::builder()
                .http2_only(true)
                .set_host(false)
                .build(connector)
        })
        .map_err(|e| warn!(logger, "Error from authentication API: {}", e))
        .ok()
}

async fn authenticate(
    request: &Request<Body>,
    cache: TokenCache,
    config: Arc<config::Config>,
    logger: Logger,
) -> Option<Client<LocalAveryConnector>> {
    let (endpoint, user_host_pair, exp, token, logger) = request
        .headers()
        .get("Authorization")
        .or_else(|| {
            warn!(logger, "Missing authorization header in request");
            None
        })
        .and_then(|hv| {
            hv.to_str()
                .map_err(|e| warn!(logger, "Failed to parse authorization header: {}", e))
                .ok()
        })
        .and_then(|auth_header| {
            auth_header
                .parse::<Token>()
                .map_err(|e| {
                    warn!(
                        logger,
                        "Failed to parse bearer token from authorization header: {}", e
                    )
                })
                .ok()
        })
        .and_then(|token| {
            jsonwebtoken::dangerous_insecure_decode_with_validation::<Claims>(
                &token.0,
                &jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::ES256),
            )
            .map_err(|e| warn!(logger, "Failed to parse JWT: {}", e))
            .ok()
            .map(|claims| (token.0, claims))
        })
        .and_then(|(token, unparsed_claims)| {
            let logger = logger.new(o!("subject" => unparsed_claims.claims.sub));
            let exp = unparsed_claims.claims.exp;
            unparsed_claims
                .claims
                .aud
                .parse::<UserHostPair>()
                .map_err(|e| warn!(logger, "Failed to parse user and host from claims: {}", e))
                .ok()
                .map(|user_host_pair| (token, user_host_pair, exp, logger))
        })
        .and_then(|(token, user_host_pair, exp, logger)| {
            let uri = get_user_socket(&config.user_socket_url, &user_host_pair.username);
            Endpoint::from_shared(uri.clone())
                .map_err(|e| warn!(logger, "Invalid local socket uri \"{}\": {}", uri, e))
                .ok()
                .map(|endpoint| (endpoint, user_host_pair, exp, token, logger))
        })?;

    let avery_logger = logger.new(o!("scope" => "auth-avery"));
    match cache.write().await.entry(token.clone()) {
        std::collections::hash_map::Entry::Occupied(mut e) => {
            // the expiry of the token is checked elsewhere
            if Utc::now().timestamp() >= e.get().0 {
                auth_against_avery(endpoint, user_host_pair, token, config, avery_logger)
                    .await
                    .map(|c| {
                        e.insert((exp, c.clone()));
                        c
                    })
            } else {
                Some(e.get().1.clone())
            }
        }
        std::collections::hash_map::Entry::Vacant(e) => {
            auth_against_avery(endpoint, user_host_pair, token, config, avery_logger)
                .await
                .map(|c| {
                    e.insert((exp, c.clone()));
                    c
                })
        }
    }
}

async fn proxy(
    request: Request<Body>,
    cache: TokenCache,
    config: Arc<config::Config>,
    logger: Logger,
) -> Result<Response<BoxBody>, hyper::Error> {
    futures::future::ready(
        authenticate(
            &request,
            cache,
            config,
            logger.new(o!("scope" => "authenticate")),
        )
        .await
        .ok_or_else(|| ProxyError::GrpcError(Status::permission_denied("Failed to authenticate."))),
    )
    .and_then(|client| {
        client
            .request(request)
            .map_ok(|resp| {
                Response::new(
                    resp.map_err(|e| tonic::Status::unknown(e.to_string()))
                        .boxed_unsync(),
                )
            })
            .map_err(ProxyError::HttpError)
    })
    .await
    .map_err(|e| {
        warn!(logger, "Proxy error: {:#?}", e);
        e
    })
    .into_http_result()
}

pub async fn run<G>(
    args: LomaxArgs,
    started_callback: G,
    log: Logger,
) -> Result<(), Box<dyn std::error::Error>>
where
    G: FnOnce() -> Result<(), String>,
{
    info!(log, "Starting Lomax 🤿");

    let config = if let Some(f) = args.config {
        config::Config::new_with_file(f)
    } else {
        config::Config::new()
    }?;

    let (expected_cert_version, cert_version) = (
        tls::get_certificate_version(&config)?,
        tls::create_cert_version(&config),
    );
    if (!config.certificate_locations.key.exists() && config.create_self_signed_certificate)
        || cert_version != expected_cert_version
    {
        if cert_version != expected_cert_version {
            info!(
                log,
                "Certificate version differs (expected: {}, got: {}), generating new certificate",
                expected_cert_version,
                cert_version
            );
        }

        tls::create_certificate(&config, log.new(o!("scope" => "self-signed-cert")))?
    }

    info!(
        log,
        "Using certificate \"{}\" with private key \"{}\".",
        &config.certificate_locations.cert.display(),
        &config.certificate_locations.key.display(),
    );

    let acceptor = tls::TlsAcceptor::new(
        tls::get_tls_config(
            &config.certificate_locations.cert,
            &config.certificate_locations.key,
        )?,
        args.port.unwrap_or(config.port),
        log.new(o!("scope" => "tls")),
    )
    .await?;

    system::drop_privileges(&config.user, &config.group)?;

    let cache: TokenCache = Arc::new(RwLock::new(HashMap::new()));
    let config = Arc::new(config);
    started_callback().map_err(|e| format!("Failed to signal startup done: {}", e))?;
    Server::builder(acceptor)
        .http2_only(true)
        .serve(make_service_fn(
            |context: &tokio_rustls::server::TlsStream<tokio::net::TcpStream>| {
                let request_logger =
                    log.new(o!("client" => context.get_ref().0.peer_addr().map(|sa|
                                                                               sa.to_string()).unwrap_or_default()));
                let c = Arc::clone(&cache);
                let config = Arc::clone(&config);
                async move {
                    Ok::<_, Infallible>(service_fn(move |request| {
                        proxy(request, Arc::clone(&c), Arc::clone(&config), request_logger.new(o!()))
                    }))
                }
            },
        ))
        .with_graceful_shutdown(system::shutdown_signal(log.new(o!("scope" => "shutdown"))))
        .await?;

    info!(log, "Shutting down lomax...");
    Ok(())
}

#[tokio::main]
pub async fn run_with_tokio(args: LomaxArgs) -> Result<(), i32> {
    let log = create_logger();
    let exit_log = log.new(o!("scope" => "unhandled_error"));

    run(args, || Ok(()), log).await.map_err(|e| {
        error!(exit_log, "Unhandled error: {}. Exiting", e);
        1i32
    })
}
