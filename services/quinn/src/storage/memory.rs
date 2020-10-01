use std::collections::hash_map::{Entry, HashMap};

use slog::Logger;
use uuid::Uuid;

use super::{Function, FunctionAttachmentData, FunctionData, FunctionStorage, StorageError};

pub struct MemoryStorage {
    functions: HashMap<FunctionKey, Function>,
    attachments: HashMap<Uuid, FunctionAttachmentData>,
}

#[derive(Eq, PartialEq, Hash)]
struct FunctionKey {
    name: String,
    version: semver::Version,
}

impl MemoryStorage {
    pub fn new(_log: Logger) -> Self {
        Self {
            functions: HashMap::new(),
            attachments: HashMap::new(),
        }
    }
}

impl FunctionStorage for MemoryStorage {
    fn insert(&mut self, function_data: FunctionData) -> Result<Uuid, StorageError> {
        let fk = FunctionKey {
            name: function_data.name.clone(),
            version: function_data.version.clone(),
        };
        match self.functions.entry(fk) {
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

    fn insert_attachment(
        &mut self,
        function_attachment_data: super::FunctionAttachmentData,
    ) -> Result<Uuid, StorageError> {
        let id = Uuid::new_v4();
        self.attachments.insert(id, function_attachment_data);
        Ok(id)
    }
}
