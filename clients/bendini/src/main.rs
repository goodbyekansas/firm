mod commands;
mod error;
mod formatting;
mod manifest;

// std
use std::path::PathBuf;

// 3rd party
use firm_types::{
    functions::{execution_client::ExecutionClient, registry_client::RegistryClient},
    tonic::{
        self,
        transport::{ClientTlsConfig, Endpoint},
    },
};
use structopt::StructOpt;
use tonic_middleware::HttpStatusInterceptor;

use error::BendiniError;

/// Bendini is a CLI interface
/// to the function registry and
/// execution services
#[derive(StructOpt, Debug)]
#[structopt(name = "bendini")]
struct BendiniArgs {
    /// Registry address to use
    #[structopt(short, long, default_value = "http://[::1]")]
    address: String,

    /// Registry port to use
    #[structopt(short, long, default_value = "1939")]
    port: u32,

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
    let address = format!("{}:{}", args.address, args.port);

    let mut endpoint = Endpoint::from_shared(address.clone())
        .map_err(|e| BendiniError::InvalidUri(e.to_string()))?;

    if endpoint.uri().scheme_str() == Some("https") {
        endpoint = endpoint
            .tls_config(ClientTlsConfig::new())
            .map_err(|e| BendiniError::FailedToCreateTlsConfig(e.to_string()))?;
    }

    let channel = endpoint
        .connect()
        .await
        .map_err(|e| BendiniError::ConnectionError(endpoint.uri().to_string(), e.to_string()))?;

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

    let client = match bearer {
        Some(bearer) => {
            println!("Using provided oauth2 credentials 🐻");
            RegistryClient::with_interceptor(channel, move |mut req: tonic::Request<()>| {
                req.metadata_mut().insert("authorization", bearer.clone());
                Ok(req)
            })
        }

        None => RegistryClient::new(channel),
    };

    match args.cmd {
        Command::List { .. } => commands::list::run(client).await,

        Command::Register { manifest } => commands::register::run(client, &manifest).await,

        Command::Run {
            function_id,
            arguments,
        } => {
            commands::run::run(
                client,
                ExecutionClient::connect("http://[::1]:1939")
                    .await
                    .map_err(|e| {
                        BendiniError::ConnectionError("local avery".to_owned(), e.to_string())
                    })?,
                function_id,
                arguments,
            )
            .await
        }
        Command::Get { function_id } => commands::get::run(client, function_id).await,
    }
}
