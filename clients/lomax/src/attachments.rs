use std::{
    io::{Read, Seek, SeekFrom},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use futures::{FutureExt, StreamExt};
use gbk_protocols::{
    functions::{
        functions_registry_client::FunctionsRegistryClient, AttachmentStreamUpload,
        AttachmentUpload, AttachmentUploadResponse, FunctionAttachmentId,
    },
    tonic,
};
use indicatif::{ProgressBar, ProgressStyle};
use rand::seq::SliceRandom;
use rand::thread_rng;
use tonic_middleware::HttpStatusInterceptor;

use crate::manifest::AttachmentInfo;

const CHUNK_SIZE: usize = 8192;

pub async fn upload_attachment(
    attachment: &AttachmentInfo,
    mut client: FunctionsRegistryClient<HttpStatusInterceptor>,
    progressbar: ProgressBar,
) -> Result<FunctionAttachmentId, String> {
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
    match client
        .upload_attachment_url(tonic::Request::new(AttachmentUpload {
            id: Some(registered_attachment.clone()),
        }))
        .await
    {
        Err(_) => {
            println!("Failed to get url for attachment upload(normal). Trying streaming method.");
            upload_via_grpc(
                client,
                progressbar,
                registered_attachment.clone(),
                attachment.request.name.clone(),
                file,
                file_size as usize,
            )
            .boxed()
        }

        Ok(response) => upload_via_http(
            response.into_inner(),
            progressbar,
            attachment.request.name.clone(),
            file,
            &attachment.path,
            file_size as usize,
        )
        .boxed(),
    }
    .await
    .map(|_| registered_attachment)
}

trait AuthBuilder {
    fn build_auth(self, upload_info: &AttachmentUploadResponse) -> reqwest::RequestBuilder;
}

impl AuthBuilder for reqwest::RequestBuilder {
    fn build_auth(self, upload_info: &AttachmentUploadResponse) -> reqwest::RequestBuilder {
        match gbk_protocols::functions::AuthMethod::from_i32(upload_info.auth_method) {
            Some(gbk_protocols::functions::AuthMethod::Oauth2) => self.bearer_auth(
                std::env::var_os("OAUTH_TOKEN")
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default(),
            ),
            _ => self,
        }
    }
}

async fn upload_via_http(
    upload_info: AttachmentUploadResponse,
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
        .post(&upload_info.url)
        .build_auth(&upload_info)
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
    mut client: FunctionsRegistryClient<HttpStatusInterceptor>,
    progressbar: ProgressBar,
    attachment_id: FunctionAttachmentId,
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
