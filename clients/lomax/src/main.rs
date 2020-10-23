#![deny(warnings)]

mod attachments;
mod formatting;
mod manifest;

// std
use std::{collections::HashMap, future::Future, path::PathBuf};

// 3rd party
use futures::future::{join, try_join_all};
use gbk_protocols::{
    functions::{
        functions_registry_client::FunctionsRegistryClient, ListRequest, OrderingDirection,
        OrderingKey, RegisterRequest,
    },
    tonic::{
        self,
        transport::{ClientTlsConfig, Endpoint},
    },
};
use indicatif::{MultiProgress, ProgressBar};
use structopt::StructOpt;
use tokio::task;
use tonic_middleware::HttpStatusInterceptor;

// internal
use formatting::DisplayExt;
use manifest::FunctionManifest;

// arguments
#[derive(StructOpt, Debug)]
#[structopt(name = "lomax")]
struct LomaxArgs {
    // function executor servicen address
    #[structopt(short, long, default_value = "tcp://[::1]")]
    address: String,

    // function executor service port
    #[structopt(short, long, default_value = "1939")]
    port: u32,

    // Command to run
    #[structopt(subcommand)]
    cmd: Command,
}

#[derive(StructOpt, Debug)]
enum Command {
    List {
        #[structopt(short, long)]
        pipeable_output: bool,
    },

    Register {
        #[structopt(parse(from_os_str))]
        manifest: PathBuf,
    },
}

async fn with_progressbars<F, U, R>(function: F) -> R
where
    U: Future<Output = R>,
    F: Fn(&MultiProgress) -> U,
{
    let multi_progress = MultiProgress::new();
    join(
        function(&multi_progress),
        task::spawn_blocking(move || {
            multi_progress.join().map_or_else(
                |e| println!("Failed waiting for progress bar: {:?}", e),
                |_| (),
            )
        }),
    )
    .await
    .0
}

#[tokio::main]
async fn main() -> Result<(), u32> {
    // parse arguments
    let args = LomaxArgs::from_args();
    let address = format!("{}:{}", args.address, args.port);

    let mut endpoint = Endpoint::new(address.clone()).map_err(|e| {
        println!("Invalid URI supplied: {}", e);
        2u32
    })?;
    if endpoint.uri().scheme_str() == Some("https") {
        endpoint = endpoint.tls_config(ClientTlsConfig::new()).map_err(|e| {
            println!("Failed to create TLS config: {}", e);
            2u32
        })?;
    }
    let channel = endpoint.connect().await.map_err(|e| {
        println!("Failed to connect to registry at \"{}\": {}", address, e);
        2u32
    })?;

    // When calling non pure grpc endpoints we may get content that is not application/grpc.
    // Tonic doesn't handle these cases very well. We have to make a wrapper around
    // to handle these edge cases. We convert it into normal tonic statuses that tonic can handle.
    let channel = HttpStatusInterceptor::new(channel);
    let bearer = std::env::var_os("OAUTH_TOKEN")
        .map(|t| {
            tonic::metadata::MetadataValue::from_str(&format!("Bearer {}", t.to_string_lossy()))
                .map_err(|e| {
                    println!("Failed to convert oauth token to metadata value: {}", e);
                    2u32
                })
        })
        .transpose()?;

    let mut client = match bearer {
        Some(bearer) => {
            println!("Using provided oauth2 credentials 🐻");
            FunctionsRegistryClient::with_interceptor(channel, move |mut req: tonic::Request<()>| {
                req.metadata_mut().insert("authorization", bearer.clone());
                Ok(req)
            })
        }

        None => FunctionsRegistryClient::new(channel),
    };

    match args.cmd {
        Command::List { .. } => {
            println!("Listing functions");
            let list_request = ListRequest {
                name_filter: String::new(),
                metadata_filter: HashMap::new(),
                metadata_key_filter: vec![],
                limit: 25,
                offset: 0,
                exact_name_match: false,
                order_direction: OrderingDirection::Ascending as i32,
                order_by: OrderingKey::Name as i32,
                version_requirement: None,
            };

            let list_response = client
                .list(tonic::Request::new(list_request))
                .await
                .map_err(|e| {
                    println!("Failed to list functions: {}", e);
                    3u32
                })?;

            list_response
                .into_inner()
                .functions
                .into_iter()
                .for_each(|f| println!("{}", f.display()))
        }

        Command::Register { manifest } => {
            let manifest_path = if manifest.is_dir() {
                manifest.join("manifest.toml")
            } else {
                manifest
            };

            let manifest = FunctionManifest::parse(&manifest_path).map_err(|e| {
                println!("\"{}\".", e);
                1u32
            })?;

            println!("Registering function \"{}\"...", manifest.name());

            println!("Reading manifest file from: {}", manifest_path.display());
            let mut register_request: RegisterRequest = (&manifest).into();
            let code = manifest.code().map_err(|e| {
                println!("Failed to parse code from the manifest: {}", e);
                3u32
            })?;

            // Code is optional. Functions could have their code located in gcp or other places
            // there is no need for the function to contain the code in that case.
            if let Some(code) = code {
                println!("Uploading code file from: {}", code.path.display());

                let code_attachment = with_progressbars(|mpb| {
                    attachments::upload_attachment(
                        &code,
                        client.clone(),
                        mpb.add(ProgressBar::new(128)),
                    )
                })
                .await
                .map_err(|e| {
                    println!("Failed to upload code attachment: {}", e);
                    3u32
                })?;
                register_request.code = Some(code_attachment);
            }

            let attachments = manifest.attachments().map_err(|e| {
                println!("Failed to parse attachments from the manifest: {}", e);
                3u32
            })?;

            let attachments = with_progressbars(|mpb| {
                try_join_all(attachments.iter().map(|a| {
                    let client_clone = client.clone();
                    attachments::upload_attachment(a, client_clone, mpb.add(ProgressBar::new(128)))
                }))
            })
            .await
            .map_err(|e| {
                println!(
                    "Failed to upload attachments. At least one failed to upload: {}",
                    e
                );
                3u32
            })?;

            register_request.attachment_ids = attachments;

            let r = client
                .register(tonic::Request::new(register_request))
                .await
                .map_err(|e| {
                    println!(
                        "Failed to register function \"{}\". Err: {}",
                        manifest.name(),
                        e
                    );
                    3u32
                })?;

            println!(
                "Registered function \"{}\" ({}) with registry at {}",
                manifest.name(),
                r.into_inner().value,
                address
            );
        }
    }

    Ok(())
}
