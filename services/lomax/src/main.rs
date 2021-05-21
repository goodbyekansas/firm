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
    server::Server,
    service::{make_service_fn, service_fn},
    Body, Client,
};
use serde::Deserialize;
use slog::{debug, error, info, o, warn, Drain, Logger};
use structopt::StructOpt;
use tokio::sync::RwLock;
use tonic::{body::BoxBody, transport::Endpoint};

mod config;
mod tls;

#[cfg(windows)]
mod windows;

#[cfg(unix)]
mod unix;

#[cfg(windows)]
use windows as system;

#[cfg(unix)]
use unix as system;

type TokenCache = Arc<RwLock<HashMap<String, (i64, Client<LocalAveryConnector>)>>>;

#[derive(Debug, StructOpt)]
struct LomaxArgs {
    /// Config file to use for the proxy
    #[structopt(short, long)]
    config: Option<PathBuf>,

    /// Port to bind the proxy to
    ///
    /// This takes precedence over config settings
    #[structopt(short, long)]
    port: Option<u16>,
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
struct LocalAveryConnector {
    uri: Uri,
}

impl LocalAveryConnector {
    pub fn new(uri: &Uri) -> Self {
        Self { uri: uri.clone() }
    }
}

type LocalConnectorFuture<C> = std::pin::Pin<
    Box<
        dyn std::future::Future<
                Output = Result<
                    <C as hyper::service::Service<http::Uri>>::Response,
                    <C as hyper::service::Service<http::Uri>>::Error,
                >,
            > + Send,
    >,
>;

fn get_connector(uri: &Uri) -> Option<LocalAveryConnector> {
    match uri.scheme_str() {
        #[cfg(unix)]
        Some("unix") => Some(LocalAveryConnector::new(uri)),

        #[cfg(windows)]
        Some("windows") => Some(LocalAveryConnector::new(uri)),

        _ => None,
    }
}

async fn grpc_connect(endpoint: Endpoint, _logger: Logger) -> Result<Channel, tonic::Status> {
    let uri = endpoint.uri().clone();
    let connector = get_connector(&uri).ok_or_else(|| {
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

async fn wait_for_approval(
    auth_client: &mut AuthenticationClient<Channel>,
    id: RemoteAccessRequestId,
) -> Result<(), Status> {
    loop {
        if auth_client
            .get_remote_access_request(tonic::Request::new(id.clone()))
            .await?
            .into_inner()
            .approved
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

async fn auth_against_avery(
    endpoint: Endpoint,
    user_host_pair: UserHostPair,
    token: String,
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
                        tokio::time::timeout(
                            Duration::from_secs(60), // TODO this should be configurable
                            wait_for_approval(&mut auth_client, id.clone()),
                        )
                        .await
                        .map_err(|e| {
                            Status::deadline_exceeded(format!(
                                "Remote access request with id \"{}\" \
                                 was not approved after waiting for {}",
                                id.uuid, e
                            ))
                        })
                        .and_then(|timeout_result| timeout_result)
                        .map(|val| {
                            info!(
                                logger,
                                "Remote access request with id \"{}\" \
                                 was approved.",
                                id.uuid
                            );
                            val
                        })
                    }
                    None => Ok(()),
                }
            }
        })
        .await
        .and_then(|_| {
            let sock = Uri::from_maybe_shared(system::get_local_socket(&user_host_pair.username))
                .map_err(|e| {
                tonic::Status::internal(format!("Invalid local socket URI: {}", e))
            })?;
            get_connector(&sock).ok_or_else(|| {
                tonic::Status::aborted(format!(
                    "Unsupported uri scheme, expected windows or unix, got \"{}\"",
                    sock.scheme_str().unwrap_or_default()
                ))
            })
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
            Endpoint::from_shared(system::get_local_socket(&user_host_pair.username))
                .map_err(|e| warn!(logger, "Invalid local socket uri: {}", e))
                .ok()
                .map(|endpoint| (endpoint, user_host_pair, exp, token, logger))
        })?;

    let avery_logger = logger.new(o!("scope" => "auth-avery"));
    match cache.write().await.entry(token.clone()) {
        std::collections::hash_map::Entry::Occupied(mut e) => {
            // the expiry of the token is checked elsewhere
            if Utc::now().timestamp() >= e.get().0 {
                auth_against_avery(endpoint, user_host_pair, token, avery_logger)
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
            auth_against_avery(endpoint, user_host_pair, token, avery_logger)
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
    logger: Logger,
) -> Result<Response<BoxBody>, hyper::Error> {
    futures::future::ready(
        authenticate(&request, cache, logger.new(o!("scope" => "authenticate")))
            .await
            .ok_or_else(|| {
                ProxyError::GrpcError(Status::permission_denied("Failed to authenticate."))
            }),
    )
    .and_then(|client| {
        client
            .request(request)
            .map_ok(|resp| resp.map(BoxBody::map_from))
            .map_err(ProxyError::HttpError)
    })
    .await
    .map_err(|e| {
        warn!(logger, "Proxy error: {:#?}", e);
        e
    })
    .into_http_result()
}

async fn run(log: Logger) -> Result<(), Box<dyn std::error::Error>> {
    info!(log, "Starting Lomax 🤿");
    let args = LomaxArgs::from_args();

    let config = if let Some(f) = args.config {
        config::Config::new_with_file(f)
    } else {
        config::Config::new()
    }?;

    if !config.certificate_locations.key.exists() && config.create_self_signed_certificate {
        std::fs::create_dir_all(
            config
                .certificate_locations
                .key
                .parent()
                .ok_or("Failed to get certificate key parent directory")?,
        )
        .map_err(|e| format!("Failed to create certificate directory: {}", e))?;

        info!(log, "Generating self signed certificate.");
        let mut alt_names = config.certificate_alt_names;
        alt_names.extend(vec![
            hostname::get()
                .map_err(|e| format!("Failed to get host name: {}", e))?
                .to_string_lossy()
                .to_string(),
            "localhost".to_string(),
        ]);
        let cert = rcgen::generate_simple_self_signed(alt_names)
            .map_err(|e| format!("Failed to generate self signed certificate: {}", e))?;

        let (cert, key) = cert
            .serialize_pem()
            .map(|pem_cert| (pem_cert, cert.serialize_private_key_pem()))
            .map_err(|e| format!("Failed to serialize certificate: {}", e))?;

        std::fs::write(&config.certificate_locations.key, key)
            .map_err(|e| format!("Failed to write certificate key: {}", e))?;
        std::fs::write(&config.certificate_locations.cert, cert)
            .map_err(|e| format!("Failed to write certificate: {}", e))?;
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
    Server::builder(acceptor)
        .http2_only(true)
        .serve(make_service_fn(
            |context: &tokio_rustls::server::TlsStream<tokio::net::TcpStream>| {
                let request_logger =
                    log.new(o!("client" => context.get_ref().0.peer_addr().map(|sa|
                                                                               sa.to_string()).unwrap_or_default()));
                let c = Arc::clone(&cache);
                async move {
                    Ok::<_, Infallible>(service_fn(move |request| {
                        proxy(request, Arc::clone(&c), request_logger.new(o!()))
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
async fn main() -> Result<(), i32> {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    let log = Logger::root(drain, o!());

    let exit_log = log.new(o!("scope" => "unhandled_error"));
    run(log).await.map_err(|e| {
        error!(exit_log, "Unhandled error: {}. Exiting", e);
        1i32
    })
}