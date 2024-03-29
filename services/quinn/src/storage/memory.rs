use std::{
    collections::hash_map::{Entry, HashMap},
    sync::RwLock,
};

use slog::Logger;
use uuid::Uuid;

use super::{Function, FunctionAttachment, FunctionId, FunctionStorage, StorageError};

pub struct MemoryStorage {
    functions: RwLock<HashMap<FunctionId, Function>>,
    attachments: RwLock<HashMap<Uuid, FunctionAttachment>>,
}

impl MemoryStorage {
    pub fn new(_log: Logger) -> Self {
        Self {
            functions: RwLock::new(HashMap::new()),
            attachments: RwLock::new(HashMap::new()),
        }
    }

    fn list(
        &self,
        filters: &super::Filters,
        group_versions: bool,
    ) -> Result<Vec<Function>, StorageError> {
        self.functions
            .read()
            .map_err(|e| {
                StorageError::BackendError(
                    format!("Failed to acquire read lock for functions: {}", e).into(),
                )
            })
            .map(|f| {
                let funcs = f
                    .values()
                    .filter(|function| {
                        // Name
                        group_versions && function.name == filters.name
                            || function.name.contains(&filters.name)
                    })
                    .filter(|function| {
                        // Version requirement
                        filters
                            .version_requirement
                            .as_ref()
                            .map_or(true, |requirement| requirement.matches(&function.version))
                    });
                let funcs = if group_versions {
                    either::Either::Right(
                        funcs
                            .fold(
                                HashMap::new(),
                                |mut hashmap: HashMap<String, &Function>, function| match hashmap
                                    .entry(function.name.clone())
                                {
                                    Entry::Occupied(mut entry) => {
                                        (entry.get().version < function.version)
                                            .then(|| entry.insert(function));
                                        hashmap
                                    }
                                    Entry::Vacant(entry) => {
                                        entry.insert(function);
                                        hashmap
                                    }
                                },
                            )
                            .into_values(),
                    )
                } else {
                    either::Either::Left(funcs)
                };
                funcs
                    .filter(|fun| {
                        // Metadata
                        filters.metadata.iter().all(|(k, v)| match v {
                            None => fun.metadata.contains_key(k),
                            value => fun.metadata.get(k) == value.as_ref(),
                        })
                    })
                    .filter(|function| function.publisher.email.contains(&filters.publisher_email))
                    .cloned()
                    .collect::<Vec<Function>>()
            })
            .map(|mut hits| {
                let order = filters.order.as_ref().cloned().unwrap_or_default();
                hits.sort_unstable_by(|a, b| match order.key {
                    firm_types::functions::OrderingKey::NameVersion => match a.name.cmp(&b.name) {
                        std::cmp::Ordering::Equal => b.version.cmp(&a.version),
                        o => o,
                    },
                });

                if order.reverse {
                    hits.into_iter()
                        .rev()
                        .skip(order.offset)
                        .take(order.limit)
                        .collect()
                } else {
                    hits.into_iter()
                        .skip(order.offset)
                        .take(order.limit)
                        .collect()
                }
            })
    }
}

#[async_trait::async_trait]
impl FunctionStorage for MemoryStorage {
    async fn insert(&self, function_data: Function) -> Result<Function, StorageError> {
        let function_id = FunctionId {
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
            .entry(function_id)
        {
            Entry::Occupied(entry) => Err(StorageError::VersionExists {
                name: entry.key().name.clone(),
                version: entry.key().version.clone(),
            }),
            Entry::Vacant(entry) => {
                let mut function = function_data;
                function.created_at = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                entry.insert(function.clone());
                Ok(function)
            }
        }
    }

    async fn insert_attachment(
        &self,
        function_attachment_data: super::FunctionAttachmentData,
    ) -> Result<FunctionAttachment, StorageError> {
        self.attachments
            .write()
            .map_err(|e| {
                StorageError::BackendError(
                    format!("Failed to acquire write lock for attachments: {}", e).into(),
                )
            })
            .map(|mut attachments| {
                let id = Uuid::new_v4();
                let attachment = FunctionAttachment {
                    id,
                    data: function_attachment_data,
                    created_at: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                };
                attachments.insert(id, attachment.clone());
                attachment
            })
    }

    async fn get(&self, id: &FunctionId) -> Result<Function, StorageError> {
        self.functions
            .read()
            .map_err(|e| {
                StorageError::BackendError(
                    format!("Failed to acquire read lock for functions: {}", e).into(),
                )
            })
            .and_then(|functions| {
                functions
                    .get(id)
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
        MemoryStorage::list(self, filters, true)
    }

    async fn list_versions(&self, filters: &super::Filters) -> Result<Vec<Function>, StorageError> {
        MemoryStorage::list(self, filters, false)
    }
}
