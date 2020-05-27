#![deny(warnings)]

mod formatting;
mod manifest;

// std
use std::{
    collections::HashMap,
    error::Error,
    future::Future,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
};

// 3rd party
use futures::future::{join, try_join_all};
use gbk_protocols::{
    functions::{
        functions_registry_client::FunctionsRegistryClient, AttachmentStreamUpload,
        FunctionAttachmentId, ListRequest, OrderingDirection, OrderingKey,
        RegisterAttachmentRequest, RegisterRequest,
    },
    tonic::{self, transport::Channel},
};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use rand::seq::SliceRandom;
use rand::thread_rng;
use structopt::StructOpt;
use tokio::task;

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

fn parse_key_val(s: &str) -> Result<(String, PathBuf), Box<dyn Error>> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid KEY=value: no `=` found in `{}`", s))?;
    let path: String = s[pos + 1..].parse()?;
    Ok((s[..pos].parse()?, PathBuf::from(path)))
}

#[derive(StructOpt, Debug)]
enum Command {
    List {
        #[structopt(short, long)]
        pipeable_output: bool,
    },

    Register {
        #[structopt(parse(from_os_str))]
        code: PathBuf,

        #[structopt(parse(from_os_str))]
        manifest: PathBuf,

        #[structopt(short = "a", parse(try_from_str = parse_key_val))]
        attachments: Vec<(String, PathBuf)>,
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

async fn upload_attachment(
    name: &str,
    path: &std::path::Path,
    metadata: HashMap<String, String>,
    mut client: FunctionsRegistryClient<Channel>,
    progressbar: ProgressBar,
) -> Result<FunctionAttachmentId, String> {
    const VEHICLES: [&str; 12] = [
        "🏇", "🏃", "🚙", "🚁", "🚕", "🚜", "🚌", "🚑", "🚚", "🚂", "🐌", "🚴",
    ];
    const CHUNK_SIZE: usize = 8192;

    let attachment = client
        .register_attachment(tonic::Request::new(RegisterAttachmentRequest {
            name: name.to_owned(),
            metadata,
        }))
        .await
        .map_err(|e| format!("Failed to register attachment \"{}\". Err: {}", name, e))?
        .into_inner();

    let mut file = std::fs::File::open(&path).map_err(|e| {
        format!(
            "Failed to read attachment {} file at \"{}\": {}",
            name,
            &path.display(),
            e
        )
    })?;

    let file_size = file
        .seek(SeekFrom::End(0))
        .and_then(|file_size| file.seek(SeekFrom::Start(0)).map(|_| file_size))
        .map_err(|e| format!("Failed to get size of file \"{}\": {}", path.display(), e))?;

    let mut rng = thread_rng();
    progressbar.set_length(file_size);
    progressbar.set_style(
        ProgressStyle::default_bar()
            .template(
                "{msg:.bold.green}\n{spinner:.green} [{elapsed_precise}] [{bar:.white.on_black/yellow}] {bytes}/{total_bytes} ({eta}, {bytes_per_sec})",
            )
            .progress_chars(&format!("-{}-", VEHICLES.choose(&mut rng).unwrap_or(&"💣"))),
    );
    progressbar.set_message(&format!("Uploading {}", name));
    progressbar.set_position(0);

    // generate an upload stream of chunks from the attachment file
    let mut reader = std::io::BufReader::with_capacity(CHUNK_SIZE, file);
    let attachment_id_clone = attachment.clone();
    let cloned_name = name.to_owned();
    let upload_stream = async_stream::stream! {
        let mut read_bytes = CHUNK_SIZE;

        let mut uploaded = 0u64;
        while read_bytes == CHUNK_SIZE {
            let mut buf = vec![0u8;CHUNK_SIZE];
            read_bytes = reader.read(&mut buf).unwrap(); // TODO: Do not unwrap
            buf.truncate(read_bytes);
            uploaded += read_bytes as u64;

            yield AttachmentStreamUpload {
                id: Some(attachment_id_clone.clone()),
                content: buf,
            };

            progressbar.set_position(uploaded);
        }
        progressbar.finish_with_message(&format!("Done uploading {}!", cloned_name));
    };

    // actually do the upload
    client
        .upload_streamed_attachment(tonic::Request::new(upload_stream))
        .await
        .map_err(|e| format!("Failed to upload {} attachment: {}", name, e))?;

    Ok(attachment)
}

#[tokio::main]
async fn main() -> Result<(), u32> {
    // parse arguments
    let args = LomaxArgs::from_args();
    let address = format!("{}:{}", args.address, args.port);

    // call the client to connect and don't worry about async stuff
    let mut client = FunctionsRegistryClient::connect(address.clone())
        .await
        .map_err(|e| {
            println!("Failed to connect to Avery at \"{}\": {}", address, e);
            2u32
        })?;

    match args.cmd {
        Command::List { .. } => {
            println!("Listing functions");
            let list_request = ListRequest {
                name_filter: String::new(),
                tags_filter: HashMap::new(),
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

        Command::Register {
            code,
            manifest,
            attachments,
        } => {
            let manifest_path = manifest;
            let code_path = code;

            let manifest = FunctionManifest::parse(&manifest_path).map_err(|e| {
                println!("\"{}\".", e);
                1u32
            })?;

            println!("Registering function \"{}\"...", manifest.name());

            println!("Reading manifest file from: {}", manifest_path.display());
            let mut register_request: RegisterRequest = (&manifest).into();
            println!("Reading code file from: {}", code_path.display());

            let code_attachment = with_progressbars(|mpb| {
                upload_attachment(
                    "code",
                    &code_path,
                    HashMap::new(),
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
            let attachments = with_progressbars(|mpb| {
                try_join_all(attachments.iter().map(|(a, p)| {
                    let client_clone = client.clone();
                    upload_attachment(
                        a,
                        p,
                        HashMap::new(),
                        client_clone,
                        mpb.add(ProgressBar::new(128)),
                    )
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
