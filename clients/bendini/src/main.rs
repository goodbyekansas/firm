mod commands;
mod error;
#[macro_use]
mod formatting;
mod manifest;

use std::{fmt::Display, ops::Deref, path::PathBuf, str::FromStr};

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
use tokio::net::NamedPipe;

use error::BendiniError;

#[cfg(unix)]
fn get_local_socket() -> Option<String> {
    use users::get_current_username;
    get_current_username().map(|username| {
        format!(
            "unix://localhost/tmp/avery-{username}.sock",
            username = username.to_string_lossy()
        )
    })
}

#[cfg(windows)]
fn get_local_socket() -> Option<String> {
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

#[derive(Debug, PartialEq)]
struct BendiniHost(String);
impl Default for BendiniHost {
    fn default() -> Self {
        Self(get_local_socket().unwrap_or_default())
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

#[derive(StructOpt, Debug)]
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

#[derive(StructOpt, Debug)]
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
        #[structopt(short, long, default_value)]
        id: String,
    },

    /// Decline incoming remote access request
    Decline {
        /// The id of the remote request to decline
        #[structopt(short, long, default_value)]
        id: String,
    },
}

#[derive(StructOpt, Debug)]
enum Command {
    /// List available functions
    List {
        #[structopt(short, long)]
        pipeable_output: bool,
    },

    /// List available runtimes
    ListRuntimes {
        /// List only runtimes matching name
        #[structopt(short, long)]
        name: Option<String>,
    },

    /// Register a new function
    Register {
        /// Path to a manifest or path to
        /// a folder containing a manifest.toml
        #[structopt(parse(from_os_str))]
        manifest: PathBuf,
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
        Some("https") => match endpoint.tls_config(ClientTlsConfig::new()) {
            Ok(endpoint_tls) => endpoint_tls.connect().await.map(|channel| (channel, true)),
            Err(e) => Err(e),
        },
        #[cfg(unix)]
        Some("unix") => endpoint
            .connect_with_connector(service_fn(|uri: Uri| {
                println!("using unix socket @ {}", uri.path());
                UnixStream::connect(uri.path().to_owned())
            }))
            .await
            .map(|channel| (channel, false)),

        #[cfg(windows)]
        Some("windows") => endpoint
            .connect_with_connector(service_fn(|uri: Uri| {
                let pipe_path = format!(
                    r#"\\{}{}"#,
                    uri.host().unwrap_or("."),
                    uri.path().replace("/", "\\")
                );
                println!("using named pipe @ {}", &pipe_path);
                NamedPipe::connect(pipe_path)
            }))
            .await
            .map(|channel| (channel, false)),
        _ => endpoint.connect().await.map(|channel| (channel, true)),
    }
    .map_err(|e| BendiniError::ConnectionError(uri.to_string(), e.to_string()))
}

async fn run() -> Result<(), error::BendiniError> {
    let args = BendiniArgs::from_args();

    let endpoint = Endpoint::from_shared(args.host.clone())
        .map_err(|e| BendiniError::InvalidUri(e.to_string()))?;

    let (channel, acquire_credentials) = connect(endpoint.clone()).await?;

    let mut auth_client = futures::future::ready(Endpoint::from_shared(args.auth_host.clone()))
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

    let bearer = if acquire_credentials {
        match auth_client
            .acquire_token(tonic::Request::new(AcquireTokenParameters {
                scope: endpoint
                    .uri()
                    .authority()
                    .map(|a| a.to_string())
                    .unwrap_or_else(|| "localhost".to_owned()),
            }))
            .await
        {
            Ok(token) => {
                let token = token.into_inner();
                (!token.token.is_empty()).then(|| token.token)
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
            tonic::metadata::MetadataValue::from_str(&format!("bearer {}", t)).map_err(|e| {
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
    // to handle these edge cases. We convert it into normal tonic statuses that tonic can handle.
    let channel = HttpStatusInterceptor::new(channel);

    let (registry_client, execution_client) = match bearer {
        Some(bearer) => {
            let bearer2 = bearer.clone();
            (
                RegistryClient::with_interceptor(
                    channel.clone(),
                    move |mut req: tonic::Request<()>| {
                        req.metadata_mut().insert("authorization", bearer.clone());
                        Ok(req)
                    },
                ),
                ExecutionClient::with_interceptor(
                    channel.clone(),
                    move |mut req: tonic::Request<()>| {
                        req.metadata_mut().insert("authorization", bearer2.clone());
                        Ok(req)
                    },
                ),
            )
        }

        None => (
            RegistryClient::new(channel.clone()),
            ExecutionClient::new(channel.clone()),
        ),
    };

    match args.cmd {
        Command::List { .. } => commands::list::run(registry_client).await,

        Command::Register { manifest } => commands::register::run(registry_client, &manifest).await,

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
        Command::Get { function_id } => commands::get::run(registry_client, function_id).await,
        Command::ListRuntimes { name } => {
            commands::list_runtimes::run(execution_client, name.unwrap_or_default()).await
        }
        Command::Auth { command } => match command {
            AuthCommand::List {
                subject_filter,
                include_approved,
                ordering,
            } => {
                commands::auth::list(auth_client, subject_filter, include_approved, ordering).await
            }
            AuthCommand::Approve { id } => commands::auth::approval(auth_client, true, id).await,
            AuthCommand::Decline { id } => commands::auth::approval(auth_client, false, id).await,
        },
    }
}
