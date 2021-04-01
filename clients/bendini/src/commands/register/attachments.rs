use std::{
    io::{Read, Seek, SeekFrom},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use firm_types::{
    functions::AttachmentUrl,
    functions::{registry_client::RegistryClient, AttachmentId, AttachmentStreamUpload},
    tonic,
};
use futures::{FutureExt, StreamExt};
use indicatif::{ProgressBar, ProgressStyle};
use rand::seq::SliceRandom;
use rand::thread_rng;
use tonic_middleware::HttpStatusInterceptor;

use crate::manifest::AttachmentInfo;

const CHUNK_SIZE: usize = 8192;

pub async fn register_and_upload_attachment(
    attachment: &AttachmentInfo,
    mut client: RegistryClient<HttpStatusInterceptor>,
    progressbar: ProgressBar,
) -> Result<AttachmentId, String> {
    const VEHICLES: [&str; 12] = [
        "🏇", "🏃", "🚙", "🚁", "🚕", "🚜", "🚌", "🚑", "🚚", "🚂", "🐌", "🚴",
    ];

    let registered_attachment = client
        .register_attachment(tonic::Request::new(attachment.request.clone()))
        .await
        .map_err(|e| {
            format!(
                "Failed to register attachment \"{}\". Err: {}",
                attachment.request.name, e
            )
        })?
        .into_inner();

    let mut file = std::fs::File::open(&attachment.path).map_err(|e| {
        format!(
            "Failed to read attachment {} file at \"{}\": {}",
            attachment.request.name,
            &attachment.path.display(),
            e
        )
    })?;

    let file_size = file
        .seek(SeekFrom::End(0))
        .and_then(|file_size| file.seek(SeekFrom::Start(0)).map(|_| file_size))
        .map_err(|e| {
            format!(
                "Failed to get size of file \"{}\": {}",
                attachment.path.display(),
                e
            )
        })?;

    let mut rng = thread_rng();
    progressbar.set_length(file_size);
    progressbar.set_style(
        ProgressStyle::default_bar()
            .template(
                "{msg:.bold.green}\n{spinner:.green} [{elapsed_precise}] [{bar:.white.on_black/yellow}] {bytes}/{total_bytes} ({eta}, {bytes_per_sec})",
            )
            .progress_chars(&format!("-{}-", VEHICLES.choose(&mut rng).unwrap_or(&"💣"))),
    );
    progressbar.set_message(&format!("Uploading {}", attachment.request.name));
    progressbar.set_position(0);

    let upload_url = registered_attachment.upload_url.ok_or_else(|| {
        String::from("No upload URL on registered attachment, cannot perform upload")
    })?;

    let parsed_url = url::Url::parse(&upload_url.url)
        .map_err(|e| format!("Failed to parse attachment upload URL: {}", e))?;

    let attachment_id = registered_attachment
        .id
        .ok_or_else(|| String::from("No id on registered attachment, cannot perform upload"))?;

    match parsed_url.scheme() {
        "grpc" => upload_via_grpc(
            // TODO: We can't assume that we can re-use the same client just because the transport is grpc.
            client,
            progressbar,
            attachment_id.clone(),
            attachment.request.name.clone(),
            file,
            file_size as usize,
        )
        .boxed(),

        "https" => upload_via_http(
            &upload_url,
            progressbar,
            attachment.request.name.clone(),
            file,
            &attachment.path,
            file_size as usize,
        )
        .boxed(),

        unsupported => futures::future::ready(Err(format!(
            "Do not know how to upload attachments for transport {} 🧐",
            unsupported,
        )))
        .boxed(),
    }
    .await
    .map(|_| attachment_id)
}

trait AuthBuilder {
    fn build_auth(self, upload_info: &AttachmentUrl) -> reqwest::RequestBuilder;
}

impl AuthBuilder for reqwest::RequestBuilder {
    fn build_auth(self, upload_url: &AttachmentUrl) -> reqwest::RequestBuilder {
        match firm_types::functions::AuthMethod::from_i32(upload_url.auth_method) {
            Some(firm_types::functions::AuthMethod::Oauth2) => self.bearer_auth(
                std::env::var_os("OAUTH_TOKEN")
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default(),
            ),
            _ => self,
        }
    }
}

async fn upload_via_http(
    upload_url: &AttachmentUrl,
    progressbar: ProgressBar,
    attachment_name: String,
    mut file: std::fs::File,
    file_name: &std::path::Path,
    file_size: usize,
) -> Result<(), String> {
    // TODO: this should be streamed
    let mut buf = Vec::with_capacity(file_size);
    file.read_to_end(&mut buf).map_err(|e| {
        format!(
            "Failed to read attachment file at {} for attachment {}: {}",
            file_name.display(),
            &attachment_name,
            e
        )
    })?;

    progressbar.set_position(0);
    reqwest::Client::new()
        .post(&upload_url.url)
        .build_auth(&upload_url)
        .body(buf)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| {
            progressbar.finish_at_current_pos();
            format!("Failed to upload {} attachment: {}", &attachment_name, e)
        })
        .map(|_| progressbar.finish_with_message(&format!("Done uploading {}!", &attachment_name)))
}

async fn upload_via_grpc(
    mut client: RegistryClient<HttpStatusInterceptor>,
    progressbar: ProgressBar,
    attachment_id: AttachmentId,
    attachment_name: String,
    file: std::fs::File,
    file_size: usize,
) -> Result<(), String> {
    let chunk_count = file_size / CHUNK_SIZE + (file_size % CHUNK_SIZE != 0) as usize; // 🧙‍♀️🧠
    let uploaded_chunk_count = Arc::new(AtomicUsize::new(0));
    let uploaded_chunk_count_clone = Arc::clone(&uploaded_chunk_count);

    // generate an upload stream of chunks from the attachment file
    let mut reader = std::io::BufReader::with_capacity(CHUNK_SIZE, file);
    let attachment_id_clone = attachment_id.clone();
    let cloned_name = attachment_name.to_owned();
    let upload_stream = async_stream::stream! {
        let mut read_bytes = CHUNK_SIZE;

        let mut uploaded = 0u64;

        while read_bytes == CHUNK_SIZE {
            let mut buf = vec![0u8;CHUNK_SIZE];

            read_bytes = reader.read(&mut buf).map_err(|e| format!("Failed to read chunk from attachment {}: {}", cloned_name, e))?;
            buf.truncate(read_bytes);
            uploaded += read_bytes as u64;

            yield Ok::<AttachmentStreamUpload, String>(
                AttachmentStreamUpload {
                id: Some(attachment_id_clone.clone()),
                content: buf,
            });
            progressbar.set_position(uploaded);
        }
        progressbar.finish_with_message(&format!("Done uploading {}!", cloned_name));
    }.map(move |res| match res {
        Ok(asu) => {
            uploaded_chunk_count_clone.fetch_add(1, Ordering::SeqCst);
            Some(asu)
        },
        Err(e) => {
            println!("{}", e);
            None
        },
    // This could potentially generate a part of an attachment (not a complete attachment)
    }).take_while(|a| futures::future::ready(a.is_some())).map(Option::unwrap);

    // actually do the upload
    client
        .upload_streamed_attachment(tonic::Request::new(upload_stream))
        .await
        .map_err(|e| format!("Failed to upload {} attachment: {}", attachment_name, e))
        .map(|_| ())?;

    if uploaded_chunk_count.load(Ordering::SeqCst) != chunk_count {
        Err(format!(
            "All chunks could not be uploaded for attachment {}.",
            attachment_name
        ))
    } else {
        Ok(())
    }
}
