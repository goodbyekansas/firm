mod commands;
mod error;
mod formatting;
mod manifest;

use std::path::PathBuf;

use firm_types::{
    functions::{execution_client::ExecutionClient, registry_client::RegistryClient},
    tonic::{
        self,
        transport::{ClientTlsConfig, Endpoint, Uri},
    },
};
use structopt::StructOpt;
use tokio::net::UnixStream;
use tonic_middleware::HttpStatusInterceptor;
use tower::service_fn;

use error::BendiniError;

/// Bendini is a CLI interface
/// to the function registry and
/// execution services
#[derive(StructOpt, Debug)]
#[structopt(name = "bendini")]
struct BendiniArgs {
    /// Host to use
    #[structopt(short, long, default_value = "unix://localhost/tmp/avery.sock")] // 🧦
    host: String,

    /// Command to run
    #[structopt(subcommand)]
    cmd: Command,
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
        eprintln!("🐞 {}", e);
        std::process::exit(e.into())
    }
}

async fn run() -> Result<(), error::BendiniError> {
    // parse arguments
    let args = BendiniArgs::from_args();

    let endpoint = Endpoint::from_shared(args.host.clone())
        .map_err(|e| BendiniError::InvalidUri(e.to_string()))?;

    let channel = match endpoint.uri().scheme_str() {
        Some("https") => match endpoint.tls_config(ClientTlsConfig::new()) {
            Ok(endpoint_tls) => endpoint_tls.connect().await,
            Err(e) => Err(e),
        },
        Some("unix") => {
            endpoint
                .connect_with_connector(service_fn(|uri: Uri| {
                    println!("using unix socket @ {}", uri.path());
                    UnixStream::connect(uri.path().to_owned())
                }))
                .await
        }
        _ => endpoint.connect().await,
    }
    .map_err(|e| BendiniError::ConnectionError(args.host.clone(), e.to_string()))?;

    // When calling non pure grpc endpoints we may get content that is not application/grpc.
    // Tonic doesn't handle these cases very well. We have to make a wrapper around
    // to handle these edge cases. We convert it into normal tonic statuses that tonic can handle.
    let channel = HttpStatusInterceptor::new(channel);
    let bearer = std::env::var_os("OAUTH_TOKEN")
        .map(|t| {
            tonic::metadata::MetadataValue::from_str(&format!("Bearer {}", t.to_string_lossy()))
                .map_err(|e| {
                    BendiniError::InvalidOauthToken(format!(
                        "Failed to convert oauth token to metadata value: {}",
                        e
                    ))
                })
        })
        .transpose()?;

    let registry_client = match bearer {
        Some(bearer) => {
            println!("Using provided oauth2 credentials 🐻");
            RegistryClient::with_interceptor(channel.clone(), move |mut req: tonic::Request<()>| {
                req.metadata_mut().insert("authorization", bearer.clone());
                Ok(req)
            })
        }

        None => RegistryClient::new(channel.clone()),
    };
    let execution_client = ExecutionClient::new(channel);

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
    }
}
