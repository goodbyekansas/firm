use std::{
    collections::hash_map::{Entry, HashMap},
    sync::RwLock,
};

use slog::Logger;
use uuid::Uuid;

use super::{Function, FunctionAttachmentData, FunctionData, FunctionStorage, StorageError};

pub struct MemoryStorage {
    functions: RwLock<HashMap<FunctionKey, Function>>,
    attachments: RwLock<HashMap<Uuid, FunctionAttachmentData>>,
}

#[derive(Eq, PartialEq, Hash)]
struct FunctionKey {
    name: String,
    version: semver::Version,
}

impl MemoryStorage {
    pub fn new(_log: Logger) -> Self {
        Self {
            functions: RwLock::new(HashMap::new()),
            attachments: RwLock::new(HashMap::new()),
        }
    }
}

#[async_trait::async_trait]
impl FunctionStorage for MemoryStorage {
    async fn insert(&self, function_data: FunctionData) -> Result<Uuid, StorageError> {
        let fk = FunctionKey {
            name: function_data.name.clone(),
            version: function_data.version.clone(),
        };
        match self
            .functions
            .write()
            .map_err(|e| {
                StorageError::Unknown(format!("Failed to acquire write lock for functions: {}", e))
            })?
            .entry(fk)
        {
            Entry::Occupied(entry) => Err(StorageError::VersionExists {
                name: entry.key().name.clone(),
                version: entry.key().version.clone(),
            }),
            Entry::Vacant(entry) => {
                let id = Uuid::new_v4();
                entry.insert(Function { id, function_data });
                Ok(id)
            }
        }
    }

    async fn insert_attachment(
        &self,
        function_attachment_data: super::FunctionAttachmentData,
    ) -> Result<Uuid, StorageError> {
        self.attachments
            .write()
            .map_err(|e| {
                StorageError::Unknown(format!(
                    "Failed to acquire write lock for attachments: {}",
                    e
                ))
            })
            .map(|mut attachments| {
                let id = Uuid::new_v4();
                attachments.insert(id, function_attachment_data);
                id
            })
    }
}
