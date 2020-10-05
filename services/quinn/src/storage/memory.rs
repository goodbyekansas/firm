use std::{
    collections::hash_map::{Entry, HashMap},
    sync::RwLock,
};

use slog::Logger;
use uuid::Uuid;

use super::{Function, FunctionAttachment, FunctionData, FunctionStorage, StorageError};

pub struct MemoryStorage {
    functions: RwLock<HashMap<FunctionKey, Function>>,
    attachments: RwLock<HashMap<Uuid, FunctionAttachment>>,
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
                StorageError::BackendError(
                    format!("Failed to acquire write lock for functions: {}", e).into(),
                )
            })?
            .entry(fk)
        {
            Entry::Occupied(entry) => Err(StorageError::VersionExists {
                name: entry.key().name.clone(),
                version: entry.key().version.clone(),
            }),
            Entry::Vacant(entry) => {
                let id = Uuid::new_v4();

                // Update all attachments with function id
                self.attachments.write()
                    .map_err(|e| StorageError::BackendError(format!("Failed to acquire write lock for attachments: {}", e).into()))
                    .and_then(|mut attachments| {
                        function_data.attachments.iter().chain(function_data.code.iter()).try_for_each(|att_id| {
                            attachments
                                .get_mut(att_id)
                                .ok_or_else(|| StorageError::BackendError("Failed to get mutable attachment for updating function reference id.".into()))
                                .map(|att| {
                                    att.function_ids.push(id);
                                })
                        })
                    })?;

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
                StorageError::BackendError(
                    format!("Failed to acquire write lock for attachments: {}", e).into(),
                )
            })
            .map(|mut attachments| {
                let id = Uuid::new_v4();
                attachments.insert(
                    id,
                    FunctionAttachment {
                        id,
                        function_ids: vec![],
                        data: function_attachment_data,
                    },
                );
                id
            })
    }

    async fn get(&self, id: &Uuid) -> Result<Function, StorageError> {
        self.functions
            .read()
            .map_err(|e| {
                StorageError::BackendError(
                    format!("Failed to acquire read lock for functions: {}", e).into(),
                )
            })
            .and_then(|functions| {
                functions
                    .values()
                    .find(|f| &f.id == id)
                    .cloned()
                    .ok_or_else(|| StorageError::FunctionNotFound(id.to_string()))
            })
    }

    async fn get_attachment(&self, id: &Uuid) -> Result<FunctionAttachment, StorageError> {
        self.attachments
            .read()
            .map_err(|e| {
                StorageError::BackendError(
                    format!("Failed to acquire read lock for attachments: {}", e).into(),
                )
            })
            .and_then(|attachments| {
                attachments
                    .get(id)
                    .cloned()
                    .ok_or_else(|| StorageError::AttachmentNotFound(id.to_string()))
            })
    }

    async fn list(&self, filters: &super::Filters) -> Result<Vec<Function>, StorageError> {
        self.functions
            .read()
            .map_err(|e| {
                StorageError::BackendError(
                    format!("Failed to acquire read lock for functions: {}", e).into(),
                )
            })
            .map(|f| {
                f.values()
                    .filter(|fun| {
                        // Name
                        match filters.exact_name_match {
                            true => fun.function_data.name == filters.name,
                            false => fun.function_data.name.contains(&filters.name),
                        }
                    })
                    .filter(|fun| {
                        // Version requirement
                        filters
                            .version_requirement
                            .as_ref()
                            .map(|requirement| requirement.matches(&fun.function_data.version))
                            .unwrap_or(true)
                    })
                    .filter(|fun| {
                        // Metadata
                        filters.metadata.iter().all(|(k, v)| match v {
                            None => fun.function_data.metadata.contains_key(k),
                            value => fun.function_data.metadata.get(k) == value.as_ref(),
                        })
                    })
                    .cloned()
                    .collect::<Vec<Function>>()
            })
            .map(|mut hits| {
                hits.sort_unstable_by(|a, b| match (filters.order_by, filters.order_descending) {
                    (gbk_protocols::functions::OrderingKey::Name, false) => {
                        match a.function_data.name.cmp(&b.function_data.name) {
                            std::cmp::Ordering::Equal => {
                                b.function_data.version.cmp(&a.function_data.version)
                            }
                            o => o,
                        }
                    }
                    (gbk_protocols::functions::OrderingKey::Name, true) => {
                        match b.function_data.name.cmp(&a.function_data.name) {
                            std::cmp::Ordering::Equal => {
                                b.function_data.version.cmp(&a.function_data.version)
                            }
                            o => o,
                        }
                    }
                });
                hits.into_iter()
                    .skip(filters.offset)
                    .take(filters.limit)
                    .collect()
            })
    }
}
