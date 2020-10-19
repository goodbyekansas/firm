use std::path::Path;

use function_protocols::{
    registry::{registry_client::RegistryClient, FunctionData},
    tonic,
};
use futures::future::try_join_all;
use indicatif::ProgressBar;
use tonic_middleware::HttpStatusInterceptor;

use crate::{error, formatting::with_progressbars, manifest::FunctionManifest};
use error::BendiniError;

mod attachments;

pub async fn run(
    mut client: RegistryClient<HttpStatusInterceptor>,
    manifest: &Path,
) -> Result<(), BendiniError> {
    let manifest_path = if manifest.is_dir() {
        manifest.join("manifest.toml")
    } else {
        manifest.to_owned()
    };

    let manifest = FunctionManifest::parse(&manifest_path)?;

    println!("Registering function \"{}\"...", manifest.name());

    println!("Reading manifest file from: {}", manifest_path.display());
    let mut register_request: FunctionData = (&manifest).into();
    let code = manifest.code()?;

    // Code is optional. Functions could have their code located in gcp or other places
    // there is no need for the function to contain the code in that case.
    if let Some(code) = code {
        println!("Uploading code file from: {}", code.path.display());

        let code_attachment = with_progressbars(|mpb| {
            attachments::register_and_upload_attachment(
                &code,
                client.clone(),
                mpb.add(ProgressBar::new(128)),
            )
        })
        .await
        .map_err(|e| BendiniError::FailedToUploadAttachment("code".to_owned(), e))?;
        register_request.code_attachment_id = Some(code_attachment);
    }

    let attachments = manifest.attachments()?;
    let attachments = with_progressbars(|mpb| {
        try_join_all(attachments.iter().map(|a| {
            let client_clone = client.clone();
            attachments::register_and_upload_attachment(
                a,
                client_clone,
                mpb.add(ProgressBar::new(128)),
            )
        }))
    })
    .await // TODO: Would be good to get the key of the attachment in the error message
    .map_err(|e| BendiniError::FailedToUploadAttachment("N/A".to_owned(), e))?;

    register_request.attachment_ids = attachments;

    client
        .register(tonic::Request::new(register_request))
        .await
        .map_err(|e| {
            BendiniError::FailedToRegisterFunction(manifest.name().to_owned(), e.to_string())
        })
        .map(|r| r.into_inner())
        .map(|registered_function| {
            // TODO: Maybe tell where it was registered
            println!(
                "Registered function \"{}:{}\"",
                registered_function.name, registered_function.version
            );
        })
}
