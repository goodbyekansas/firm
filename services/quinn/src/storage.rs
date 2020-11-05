use std::{collections::HashMap, fmt::Display};

use semver::Version;
use slog::{info, o, Logger};
use thiserror::Error;
use url::Url;
use uuid::Uuid;

pub use firm_types::{
    functions::{AttachmentUrl, AuthMethod, ChannelType},
    registry::{AttachmentHandle, OrderingKey},
};

mod gcs;
mod https;
mod memory;
mod postgres;

#[derive(Eq, PartialEq, Hash, Debug, Clone)]
pub struct FunctionId {
    pub name: String,
    pub version: semver::Version,
}

impl Display for FunctionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{}:{}", self.name, self.version)
    }
}

impl From<&Function> for FunctionId {
    fn from(f: &Function) -> Self {
        Self {
            name: f.name.clone(),
            version: f.version.clone(),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Function {
    pub name: String,
    pub version: Version,
    pub runtime: Runtime,
    pub required_inputs: HashMap<String, ChannelSpec>,
    pub optional_inputs: HashMap<String, ChannelSpec>,
    pub outputs: HashMap<String, ChannelSpec>,
    pub metadata: HashMap<String, String>,
    pub code: Option<Uuid>,
    pub attachments: Vec<Uuid>,
    pub created_at: u64,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ChannelSpec {
    pub description: String,
    pub argument_type: ChannelType,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Runtime {
    pub name: String,
    pub entrypoint: String,
    pub arguments: HashMap<String, String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Checksums {
    pub sha256: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FunctionAttachment {
    pub id: Uuid,
    pub data: FunctionAttachmentData,
    pub created_at: u64,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FunctionAttachmentData {
    pub name: String,
    pub metadata: HashMap<String, String>,
    pub checksums: Checksums,
}

#[derive(Debug, Clone, Eq, PartialEq, Default)]
pub struct NameFilter {
    pub pattern: String,
    pub exact_match: bool,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Ordering {
    pub key: OrderingKey,
    pub reverse: bool,
    pub offset: usize,
    pub limit: usize,
}
#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct Filters {
    pub name: Option<NameFilter>,
    pub version_requirement: Option<semver::VersionReq>,
    pub order: Option<Ordering>,
    pub metadata: HashMap<String, Option<String>>,
}

impl Default for Ordering {
    fn default() -> Self {
        Self {
            key: OrderingKey::NameVersion,
            reverse: false,
            offset: 0,
            limit: 100,
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
    async fn insert(&self, function_data: Function) -> Result<Function, StorageError>;
    async fn insert_attachment(
        &self,
        function_attachment_data: FunctionAttachmentData,
    ) -> Result<FunctionAttachment, StorageError>;
    async fn get(&self, id: &FunctionId) -> Result<Function, StorageError>;
    async fn get_attachment(&self, id: &Uuid) -> Result<FunctionAttachment, StorageError>;
    async fn list(&self, filters: &Filters) -> Result<Vec<Function>, StorageError>;
}

pub trait AttachmentStorage: Send + Sync + std::fmt::Debug {
    fn get_upload_url(
        &self,
        attachment: &FunctionAttachment,
    ) -> Result<AttachmentUrl, StorageError>;
    fn get_download_url(
        &self,
        attachment: &FunctionAttachment,
    ) -> Result<AttachmentUrl, StorageError>;
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
