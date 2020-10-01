use std::collections::HashMap;

use semver::Version;
use slog::{info, o, Logger};
use thiserror::Error;
use url::Url;
use uuid::Uuid;

pub use gbk_protocols::functions::{ArgumentType, AttachmentUploadResponse, AuthMethod};

mod memory;
mod postgres;

#[derive(Debug)]
pub struct Function {
    pub id: Uuid,
    pub function_data: FunctionData,
}

#[derive(Debug)]
pub struct FunctionData {
    pub name: String,
    pub version: Version,
    pub execution_environment: ExecutionEnvironment,
    pub inputs: Vec<FunctionInput>,
    pub outputs: Vec<FunctionOutput>,
    pub metadata: HashMap<String, String>,
    pub code: Option<Uuid>,
    pub attachments: Vec<Uuid>,
}

#[derive(Debug)]
pub struct FunctionInput {
    pub name: String,
    pub required: bool,
    pub argument_type: ArgumentType,
    pub default_value: String,
    pub from_execution_environment: bool,
}

#[derive(Debug)]
pub struct FunctionOutput {
    pub name: String,
    pub argument_type: ArgumentType,
}

#[derive(Debug)]
pub struct ExecutionEnvironment {
    pub name: String,
    pub entrypoint: String,
    pub function_arguments: HashMap<String, String>,
}

#[derive(Debug)]
pub struct Checksums {
    pub sha256: String,
}

#[derive(Debug)]
pub struct FunctionAttachment {
    pub id: Uuid,
    pub data: FunctionAttachmentData,
}

#[derive(Debug)]
pub struct FunctionAttachmentData {
    pub name: String,
    pub metadata: HashMap<String, String>,
    pub checksums: Checksums,
}

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("Unknown: {0}")]
    Unknown(String),

    #[error("Version {version} already exists for {name}")]
    VersionExists { name: String, version: Version },

    #[error("Invalid storage URI: {0}")]
    InvalidUri(#[from] url::ParseError),

    #[error("Unsupported storage backend \"{0}\"")]
    UnsupportedStorage(String),

    #[error("Connection Error: {0}")]
    ConnectionError(String),

    #[error("Invalid attachment storage specification: {0}")]
    InvalidAttachmentStorage(String),
}

pub async fn create_storage<S: AsRef<str>>(
    uri: S,
    log: Logger,
) -> Result<Box<dyn FunctionStorage>, StorageError> {
    let uri = Url::parse(uri.as_ref())?;
    Ok(match uri.scheme() {
        "memory" => {
            info!(log, "creating memory storage backend");
            Box::new(memory::MemoryStorage::new(log.new(o!("type" => "memory"))))
        }
        "postgres" | "postgresql" => {
            info!(log, "creating postgresql backend");
            postgres::PostgresStorage::new_with_init(&uri, log.new(o!("type" => "postgresql")))
                .await
                .map(Box::new)?
        }
        st => return Err(StorageError::UnsupportedStorage(st.to_owned())),
    })
}

#[async_trait::async_trait]
pub trait FunctionStorage: Send + Sync {
    async fn insert(&self, function_data: FunctionData) -> Result<Uuid, StorageError>;
    async fn insert_attachment(
        &self,
        function_attachment_data: FunctionAttachmentData,
    ) -> Result<Uuid, StorageError>;
}

pub trait FunctionAttachmentStorage: Send + Sync + std::fmt::Debug {
    fn get_upload_url(
        &self,
        attachment_id: Uuid,
        function_storage: &dyn FunctionStorage,
    ) -> Result<AttachmentUploadResponse, StorageError>;
}

#[derive(Debug)]
struct GCSStorage {
    bucket_name: String,
}

impl GCSStorage {
    fn new(bucket_name: &str) -> Self {
        Self {
            bucket_name: bucket_name.to_owned(),
        }
    }
}

impl FunctionAttachmentStorage for GCSStorage {
    fn get_upload_url(
        &self,
        attachment_id: Uuid,
        _function_storage: &dyn FunctionStorage,
    ) -> Result<AttachmentUploadResponse, StorageError> {
        Ok(AttachmentUploadResponse {
            // TODO when we have a proper get interface include function name and version in object name
            url: format!(
                "https://storage.googleapis.com/upload/storage/v1/b/{bucket_name}/o?uploadType=media&name={object_name}",
                bucket_name=self.bucket_name,
                object_name=attachment_id.to_string()
            ),
            auth_method: AuthMethod::Oauth2 as i32,
        })
    }
}

pub fn create_attachment_storage<S: AsRef<str>>(
    uri: S,
    log: Logger,
) -> Result<Box<dyn FunctionAttachmentStorage>, StorageError> {
    let uri = Url::parse(uri.as_ref())?;
    Ok(match uri.scheme() {
        "gcs" => {
            info!(log, "creating Google Cloud Storage backend");
            uri.host_str()
                .ok_or_else(|| {
                    StorageError::InvalidAttachmentStorage(
                        "gcs attachment storage requires bucket name".to_owned(),
                    )
                })
                .map(|bucket_name| Box::new(GCSStorage::new(bucket_name)))?
        }
        st => return Err(StorageError::UnsupportedStorage(st.to_owned())),
    })
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    macro_rules! null_logger {
        () => {{
            slog::Logger::root(slog::Discard, slog::o!())
        }};
    }

    #[test]
    fn test_gcs_bucket_url() {
        let uuid_str = "1a52540c-7edd-4f9e-916b-f9aaf165890e";
        let bucket_name = "hinken";
        let expected_storage_path = format!(
            "https://storage.googleapis.com/upload/storage/v1/b/{}/o?uploadType=media&name={}",
            bucket_name, uuid_str
        );
        let gcs_storage = GCSStorage::new(bucket_name);
        let mock_storage =
            futures::executor::block_on(create_storage("memory://".to_owned(), null_logger!()))
                .unwrap();
        let res =
            gcs_storage.get_upload_url(Uuid::parse_str(uuid_str).unwrap(), mock_storage.as_ref());

        assert!(res.is_ok());
        let resp = res.unwrap();
        assert_eq!(resp.url, expected_storage_path);
        assert_eq!(resp.auth_method, super::AuthMethod::Oauth2 as i32);
    }

    #[test]
    fn test_create_attachment_storage() {
        let res = create_attachment_storage("gcs://kallebula", null_logger!());
        assert!(res.is_ok());

        let res = create_attachment_storage("super-sune://", null_logger!());
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            StorageError::UnsupportedStorage {..}
        ));
    }
}
