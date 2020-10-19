use std::collections::HashMap;

use semver::Version;
use slog::{info, o, Logger};
use thiserror::Error;
use url::Url;
use uuid::Uuid;

pub use gbk_protocols::functions::{
    ArgumentType, AttachmentUploadResponse, AuthMethod, FunctionDescriptor, OrderingKey,
};

mod gcs;
mod https;
mod memory;
mod postgres;

#[derive(Debug, Clone)]
pub struct Function {
    pub id: Uuid,
    pub function_data: FunctionData,
}

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct FunctionInput {
    pub name: String,
    pub required: bool,
    pub argument_type: ArgumentType,
    pub default_value: String,
    pub from_execution_environment: bool,
}

#[derive(Debug, Clone)]
pub struct FunctionOutput {
    pub name: String,
    pub argument_type: ArgumentType,
}

#[derive(Debug, Clone)]
pub struct ExecutionEnvironment {
    pub name: String,
    pub entrypoint: String,
    pub function_arguments: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct Checksums {
    pub sha256: String,
}

#[derive(Debug, Clone)]
pub struct FunctionAttachment {
    pub id: Uuid,
    pub function_ids: Vec<Uuid>,
    pub data: FunctionAttachmentData,
}

#[derive(Debug, Clone)]
pub struct FunctionAttachmentData {
    pub name: String,
    pub metadata: HashMap<String, String>,
    pub checksums: Checksums,
}

#[derive(Debug, Clone)]
pub struct Filters {
    pub name: String,
    pub metadata: HashMap<String, Option<String>>,
    pub offset: usize,
    pub limit: usize,
    pub exact_name_match: bool,
    pub version_requirement: Option<semver::VersionReq>,
    pub order_descending: bool,
    pub order_by: OrderingKey,
}

impl Default for Filters {
    fn default() -> Self {
        Self {
            name: String::default(),
            metadata: HashMap::default(),
            offset: 0,
            limit: 100,
            exact_name_match: false,
            version_requirement: Option::default(),
            order_descending: false,
            order_by: OrderingKey::Name,
        }
    }
}

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("Storage backend error: {0}")]
    BackendError(Box<dyn std::error::Error + Sync + Send>),

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

    #[error("Could not find function: {0}")]
    FunctionNotFound(String),

    #[error("Could not find attachment: {0}")]
    AttachmentNotFound(String),
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
    async fn get(&self, id: &Uuid) -> Result<Function, StorageError>;
    async fn get_attachment(&self, id: &Uuid) -> Result<FunctionAttachment, StorageError>;
    async fn list(&self, filters: &Filters) -> Result<Vec<Function>, StorageError>;
}

pub trait AttachmentStorage: Send + Sync + std::fmt::Debug {
    fn get_upload_url(
        &self,
        attachment: &FunctionAttachment,
    ) -> Result<AttachmentUploadResponse, StorageError>;
    fn get_download_url(&self, attachment: &FunctionAttachment) -> Result<Url, StorageError>;
}

pub fn create_attachment_storage<S: AsRef<str>>(
    uri: S,
    log: Logger,
) -> Result<Box<dyn AttachmentStorage>, StorageError> {
    let uri = Url::parse(uri.as_ref())?;
    Ok(match uri.scheme() {
        "gcs" => {
            info!(log, "Creating Google Cloud Storage backend");
            uri.host_str()
                .ok_or_else(|| {
                    StorageError::InvalidAttachmentStorage(
                        "gcs attachment storage requires bucket name".to_owned(),
                    )
                })
                .map(|bucket_name| Box::new(gcs::GCSStorage::new(bucket_name)))?
        }
        "https" => {
            info!(log, "Creating Https Storage backend");
            Box::new(https::HttpsStorage::new(&uri, AuthMethod::Oauth2)?) // TODO auth method should come in from somewhere
        }
        st => return Err(StorageError::UnsupportedStorage(st.to_owned())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! null_logger {
        () => {{
            slog::Logger::root(slog::Discard, slog::o!())
        }};
    }

    #[test]
    fn creating_attachment_storage() {
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
