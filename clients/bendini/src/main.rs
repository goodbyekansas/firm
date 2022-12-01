#[macro_use]
mod formatting;
mod auth;
mod commands;
mod error;
mod interactive_cert_verifier;
mod manifest;

use std::{fmt::Display, ops::Deref, path::PathBuf, str::FromStr, sync::Arc};

use firm_types::{
    auth::authentication_client::AuthenticationClient,
    auth::AcquireTokenParameters,
    functions::{execution_client::ExecutionClient, registry_client::RegistryClient},
    tonic::{
        self,
        transport::{Channel, ClientTlsConfig, Endpoint, Uri},
    },
};
use futures::TryFutureExt;
use structopt::StructOpt;
use tonic_middleware::HttpStatusInterceptor;
use tower::service_fn;

#[cfg(unix)]
use tokio::net::UnixStream;

#[cfg(windows)]
use tokio::net::windows::named_pipe::ClientOptions;

#[cfg(windows)]
use winapi::shared::winerror;

use error::BendiniError;

#[cfg(unix)]
mod system {
    use std::path::PathBuf;

    pub fn get_local_socket() -> Option<String> {
        use users::get_current_username;
        get_current_username().map(|username| {
            format!(
                "unix://localhost/tmp/avery-{username}.sock",
                username = username.to_string_lossy()
            )
        })
    }

    pub fn user_data_path() -> Option<PathBuf> {
        match std::env::var("XDG_DATA_HOME").ok() {
            Some(p) => Some(PathBuf::from(p)),
            None => std::env::var("HOME")
                .ok()
                .map(|p| PathBuf::from(p).join(".local").join("share")),
        }
        .map(|p| p.join("bendini"))
    }

    pub fn reset_sigpipe() {
        unsafe {
            libc::signal(libc::SIGPIPE, libc::SIG_DFL);
        }
    }
}

#[cfg(windows)]
mod system {
    use std::path::PathBuf;

    pub fn get_local_socket() -> Option<String> {
        use winapi::um::winbase::GetUserNameW;
        const CAPACITY: usize = 1024;
        let mut size = CAPACITY as u32;
        let mut name: [u16; CAPACITY] = [0; CAPACITY];
        unsafe {
            (GetUserNameW(name.as_mut_ptr(), &mut size as *mut u32) != 0).then(|| {
                format!(
                    r#"windows://./pipe/avery-{user}"#,
                    user = String::from_utf16_lossy(&name[..(size as usize) - 1])
                )
            })
        }
    }

    pub fn user_data_path() -> Option<PathBuf> {
        std::env::var("LOCALAPPDATA")
            .ok()
            .map(|p| PathBuf::from(p).join("bendini"))
    }

    pub fn reset_sigpipe() {
        // noop on windows
    }
}

#[derive(Debug, PartialEq)]
struct BendiniHost(String);
impl Default for BendiniHost {
    fn default() -> Self {
        Self(system::get_local_socket().unwrap_or_default())
    }
}
impl ToString for BendiniHost {
    fn to_string(&self) -> String {
        self.0.clone()
    }
}
impl FromStr for BendiniHost {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.to_owned()))
    }
}
impl Deref for BendiniHost {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Bendini is a CLI interface
/// to the function registry and
/// execution services
#[derive(StructOpt, Debug)]
#[structopt(name = "bendini")]
struct BendiniArgs {
    /// Host to use
    #[structopt(short, long, default_value)] // 🧦
    host: BendiniHost,

    /// Host to use for authentication.
    /// This needs to be a trusted connection,
    /// i.e. run without authentication since
    /// it's used to fetch authentication for other remote operations.
    #[structopt(long, default_value)] // 🧦
    auth_host: BendiniHost,

    /// Command to run
    #[structopt(subcommand)]
    cmd: Command,
}

#[derive(StructOpt, Debug, Clone)]
pub enum Ordering {
    Subject,
    ExpiresAt,
}

impl Default for Ordering {
    fn default() -> Self {
        Self::Subject
    }
}

impl FromStr for Ordering {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "subject" => Ok(Self::Subject),
            "expiry" => Ok(Self::ExpiresAt),
            _ => Err(format!(r#""{}" is not any of "subject" or "expiry""#, s)),
        }
    }
}

impl Display for Ordering {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Subject => "subject",
                Self::ExpiresAt => "expiry",
            }
        )
    }
}

#[derive(StructOpt, Debug, Clone)]
enum AuthCommand {
    /// List incoming remote access requests
    List {
        /// Filter on the subject of the remote access request
        #[structopt(short, long, default_value)]
        subject_filter: String,

        /// Include  already approved access requests
        #[structopt(short, long)]
        include_approved: bool,

        /// Order on subject or expiry date of access request
        #[structopt(short, long, default_value)]
        ordering: Ordering,
    },

    /// Approve incoming remote access request
    Approve {
        /// The id of the remote request to approve
        id: String,
    },

    /// Decline incoming remote access request
    Decline {
        /// The id of the remote request to decline
        id: String,
    },
}

impl FromStr for formatting::DisplayFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "json" => formatting::DisplayFormat::Json,
            "short" => formatting::DisplayFormat::Short,
            _ => formatting::DisplayFormat::Long,
        })
    }
}

impl Default for formatting::DisplayFormat {
    fn default() -> Self {
        formatting::DisplayFormat::Long
    }
}

impl Display for formatting::DisplayFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "{}",
            match self {
                formatting::DisplayFormat::Short => "short",
                formatting::DisplayFormat::Long => "long",
                formatting::DisplayFormat::Json => "json",
            }
        )
    }
}

#[derive(StructOpt, Debug, Clone)]
enum Command {
    /// List available functions
    List {
        /// Display format to use
        #[structopt(short, long, default_value)]
        format: formatting::DisplayFormat,
    },

    /// List versions of a function
    ListVersions {
        /// Name of the function to list versions for
        name: String,

        /// Display format to use
        #[structopt(short, long, default_value)]
        format: formatting::DisplayFormat,
    },

    /// List available runtimes
    ListRuntimes {
        /// List only runtimes matching name
        #[structopt(short, long)]
        name: Option<String>,
    },

    /// Register a new function and upload to
    /// a registry as given by the `host` option.
    Register {
        /// Path to a manifest or path to
        /// a folder containing a manifest.toml
        #[structopt(parse(from_os_str))]
        manifest: PathBuf,

        /// Name of the function publisher
        #[structopt(short = "n", long)]
        publisher_name: Option<String>,

        /// Email of the function publisher
        #[structopt(short = "e", long)]
        publisher_email: Option<String>,
    },

    /// Executes a function with arguments
    Run {
        /// Specification for function to run. A function named followed by
        /// a colon and a version requirement (my-function:0.4)
        function_id: String,

        /// Arguments to provide to the function when executing
        /// The arguments will be converted into the types
        /// expected by the function
        #[structopt(short = "i", parse(try_from_str = parse_key_val))]
        arguments: Vec<(String, String)>,

        /// Print output from the function while it is running
        #[structopt(short = "f", long = "follow")]
        follow_output: bool,
    },

    /// Gets information about a single function
    Get {
        /// Specification for function to get. A function named followed by
        /// a colon and a version (my-function:0.4.1)
        function_id: String,
    },

    /// List and approve remote access requests
    Auth {
        /// Authentication sub command to run
        #[structopt(subcommand)]
        command: AuthCommand,
    },
}

fn parse_key_val(s: &str) -> Result<(String, String), Box<dyn std::error::Error>> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid KEY=value: no `=` found in `{}`", s))?;
    Ok((s[..pos].parse()?, s[pos + 1..].parse()?))
}

#[tokio::main]
async fn main() {
    // rust ignores sigpipe by default which
    // causes problems when output is piped
    // to other programs, something we expect
    // to be done. See: https://github.com/rust-lang/rust/issues/46016
    let _ = system::reset_sigpipe();
    #[cfg(windows)]
    if atty::is(atty::Stream::Stdout) {
        if let Err(e) = ansi_term::enable_ansi_support() {
            eprintln!(
                "Failed to enable ANSI color support. WinAPI error code: {}",
                e
            );
        }
    }

    if let Err(e) = run().await {
        eprintln!("🐞 {}", error!("{}", e));
        std::process::exit(e.into())
    }
}

async fn connect(endpoint: Endpoint) -> Result<(Channel, bool), BendiniError> {
    let uri = endpoint.uri().clone();
    match uri.scheme_str() {
        Some("firm") | None => {
            let data_path = system::user_data_path()
                .ok_or_else(|| {
                    BendiniError::ConnectionError(
                        uri.to_string(),
                        "Failed to determine user data path for saving accepted certs".to_owned(),
                    )
                })
                .and_then(|p| {
                    std::fs::create_dir_all(&p).map_err(|e| {
                        BendiniError::ConnectionError(
                            uri.to_string(),
                            format!(
                                "Failed to create user data path for saving accepted certs: {}",
                                e
                            ),
                        )
                    })?;
                    Ok(p)
                })?;

            let rustls_config = rustls::ClientConfig::builder()
                .with_safe_defaults()
                .with_custom_certificate_verifier(Arc::new(
                    interactive_cert_verifier::InteractiveCertVerifier::new(
                        &data_path,
                        rustls_native_certs::load_native_certs()
                            .map_err(|e| {
                                BendiniError::ConnectionError(
                                    uri.to_string(),
                                    format!("Failed to load system CA roots: {}", e),
                                )
                            })?
                            .iter()
                            .map(|cert| rustls::Certificate(cert.0.clone()))
                            .collect::<Vec<_>>()
                            .as_slice(),
                    )
                    .map_err(|e| {
                        BendiniError::ConnectionError(
                            uri.to_string(),
                            format!("Failed to create internal cert verifier: {}", e),
                        )
                    })?,
                ))
                .with_no_client_auth();

            let mut http = hyper::client::HttpConnector::new();
            http.enforce_http(false);

            let connector = tower::ServiceBuilder::new()
                .layer_fn(move |s| {
                    let tls = rustls_config.clone();

                    hyper_rustls::HttpsConnectorBuilder::new()
                        .with_tls_config(tls)
                        .https_only()
                        .enable_http2()
                        .wrap_connector(s)
                })
                .service(http);

            // add some defaults for the firm:// protocol (is also the default one, hence None)
            Endpoint::from_shared(format!(
                "https://{}{}{}",
                uri.authority().map(|a| a.to_string()).unwrap_or_default(),
                uri.authority()
                    .and_then(|a| a.port())
                    .map(|_| "") // port is already in authority
                    .unwrap_or(":1939"),
                uri.path_and_query()
                    .map(|pq| pq.to_string())
                    .unwrap_or_default(),
            ))
            .map_err(|e| {
                BendiniError::ConnectionError(
                    uri.to_string(),
                    format!("Cannot construct firm:// URI: {}", e),
                )
            })?
            .connect_with_connector(connector)
            .await
            .map(|channel| (channel, true))
        }
        #[cfg(unix)]
        Some("unix") => endpoint
            .connect_with_connector(service_fn(|uri: Uri| async move {
                UnixStream::connect(uri.path()).await
            }))
            .await
            .map(|channel| (channel, false)),

        #[cfg(windows)]
        Some("windows") => endpoint
            .connect_with_connector(service_fn(|uri: Uri| {
                let pipe_path = format!(
                    r#"\\{}{}"#,
                    uri.host().unwrap_or("."),
                    uri.path().replace('/', "\\")
                );

                async move {
                    loop {
                        match ClientOptions::new().open(&pipe_path) {
                            Ok(client) => break Ok(client),
                            Err(e)
                                if e.raw_os_error() == Some(winerror::ERROR_PIPE_BUSY as i32) => {}
                            Err(e) => break Err(e),
                        }

                        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                    }
                }
            }))
            .await
            .map(|channel| (channel, false)),
        // always use TLS, we are not insane
        Some(_) => match endpoint.tls_config(ClientTlsConfig::new()) {
            Ok(endpoint_tls) => endpoint_tls.connect().await.map(|channel| (channel, true)),
            Err(e) => Err(e),
        },
    }
    .map_err(|e| BendiniError::ConnectionError(uri.to_string(), e.to_string()))
}

async fn run() -> Result<(), error::BendiniError> {
    let args = BendiniArgs::from_args();

    let endpoint = Endpoint::from_shared(args.host.clone())
        .map_err(|e| BendiniError::InvalidUri(e.to_string()))?;

    let (channel, acquire_credentials) = connect(endpoint.clone()).await?;

    let auth_client = futures::future::ready(Endpoint::from_shared(args.auth_host.clone()))
        .map_err(|e| BendiniError::InvalidUri(e.to_string()))
        .and_then(|endpoint| async {
            if args.host == args.auth_host {
                Ok(channel.clone())
            } else {
                connect(endpoint).await.map(|(channel, _)| channel)
            }
        })
        .await
        .map(AuthenticationClient::new)?;

    let bearer: Option<tonic::metadata::MetadataValue<_>> = if acquire_credentials {
        println!(
            "Acquiring credentials for host {} from Avery at {}...",
            args.host.clone(),
            args.auth_host.clone()
        );
        match auth::with_login(auth_client.clone(), || {
            let mut auth_client = auth_client.clone();
            let endpoint = endpoint.clone();
            async move {
                auth_client
                    .acquire_token(tonic::Request::new(AcquireTokenParameters {
                        scope: endpoint
                            .uri()
                            .authority()
                            .and_then(|a| match a.port() {
                                Some(_) => a.as_str().rsplit_once(':').map(|(a, _)| a),
                                None => Some(a.as_str()),
                            })
                            .map(|s| s.to_owned())
                            .unwrap_or_else(|| "localhost".to_owned()),
                    }))
                    .await
                    .map_err(BendiniError::from)
            }
        })
        .await
        {
            Ok(token) => {
                println!("Credentials acquired successfully!");
                Some(token.into_inner().token)
            }
            Err(e) => {
                println!(
                    "{} 🤞",
                    warn!(
                        r#"Acquiring credentials for scope "{}" \
                           failed with error: {}. \
                           Continuing without credentials set.
                          "#,
                        ansi_term::Style::new().bold().paint(args.host.clone()),
                        e
                    )
                );
                None
            }
        }
        .map(|t| {
            format!("bearer {}", t).parse().map_err(|e| {
                BendiniError::InvalidOauthToken(format!(
                    "Failed to convert oauth token to metadata value: {}",
                    e
                ))
            })
        })
        .transpose()?
    } else {
        None
    };

    // When calling non pure grpc endpoints we may get content that is not application/grpc.
    // Tonic doesn't handle these cases very well. We have to make a wrapper around
    // to handle these edge cases. We convert it into normal tonic statuses that tonic can
    // handle.
    let channel = tower::ServiceBuilder::new()
        .layer(tonic::service::interceptor(
            move |mut req: tonic::Request<()>| {
                if let Some(bearer) = bearer.clone() {
                    req.metadata_mut().insert("authorization", bearer);
                }
                Ok(req)
            },
        ))
        .layer_fn(HttpStatusInterceptor::new)
        .service(channel);

    auth::with_login(auth_client.clone(), || {
        let (registry_client, execution_client) = (
            RegistryClient::new(channel.clone()),
            ExecutionClient::new(channel.clone()),
        );

        let auth_client = auth_client.clone();
        let cmd = args.cmd.clone();
        async move {
            match cmd {
                Command::List { format } => {
                    commands::list::functions(registry_client, format).await
                }

                Command::ListVersions { name, format } => {
                    commands::list::versions(registry_client, &name, format).await
                }

                Command::Register {
                    manifest,
                    publisher_name,
                    publisher_email,
                } => {
                    futures::future::ready(match (publisher_name, publisher_email) {
                        (Some(publisher_name), Some(publisher_email)) => {
                            Ok((publisher_name, publisher_email))
                        }
                        (name, email) => auth_client
                            .clone()
                            .get_identity(tonic::Request::new(()))
                            .await
                            .map(|identity| {
                                let identity = identity.into_inner();
                                (
                                    name.unwrap_or(identity.name),
                                    email.unwrap_or(identity.email),
                                )
                            }),
                    })
                    .map_err(Into::into)
                    .and_then(|(name, email)| async move {
                        commands::register::run(
                            registry_client,
                            auth_client,
                            &manifest,
                            &name,
                            &email,
                        )
                        .await
                    })
                    .await
                }

                Command::Run {
                    function_id,
                    arguments,
                    follow_output,
                } => {
                    commands::run::run(
                        registry_client,
                        execution_client,
                        function_id,
                        arguments,
                        follow_output,
                    )
                    .await
                }
                Command::Get { function_id } => {
                    commands::get::run(registry_client, function_id).await
                }
                Command::ListRuntimes { name } => {
                    commands::list_runtimes::run(execution_client, name.unwrap_or_default()).await
                }
                Command::Auth { command } => match command {
                    AuthCommand::List {
                        subject_filter,
                        include_approved,
                        ordering,
                    } => {
                        commands::auth::list(
                            auth_client,
                            subject_filter,
                            include_approved,
                            ordering,
                        )
                        .await
                    }
                    AuthCommand::Approve { id } => {
                        commands::auth::approval(auth_client, true, id).await
                    }
                    AuthCommand::Decline { id } => {
                        commands::auth::approval(auth_client, false, id).await
                    }
                },
            }
        }
    })
    .await
}
