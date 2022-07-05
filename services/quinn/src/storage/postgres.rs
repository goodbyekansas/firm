use std::{
    collections::hash_map::HashMap,
    convert::{TryFrom, TryInto},
    time::SystemTime,
};

use bb8_postgres::PostgresConnectionManager;
use firm_types::functions::ChannelType;
use futures::future::TryFutureExt;
use postgres_types::{FromSql, ToSql};
use slog::{info, Logger};
use tokio_postgres::NoTls;

use crate::storage;

use uuid::Uuid;

#[derive(Debug, ToSql, FromSql)]
#[postgres(name = "argument_type")]
enum Type {
    #[postgres(name = "string")]
    String,

    #[postgres(name = "float")]
    Float,

    #[postgres(name = "bool")]
    Bool,

    #[postgres(name = "int")]
    Int,

    #[postgres(name = "bytes")]
    Bytes,
}

#[derive(Debug, ToSql, FromSql)]
#[postgres(name = "functions")]
struct Function {
    id: Uuid,
    name: String,
    version: Version,
    metadata: HashMap<String, Option<String>>,
    code: Option<Uuid>,
    required_inputs: Vec<ChannelSpec>,
    optional_inputs: Vec<ChannelSpec>,
    outputs: Vec<ChannelSpec>,
    runtime: Runtime,
    created_at: SystemTime,
    publisher_id: Uuid,
    signature: Option<Vec<u8>>,
}

#[derive(Debug, ToSql, FromSql)]
#[postgres(name = "publishers")]
pub struct Publisher {
    id: Uuid,
    name: String,
    email: String,
}

#[derive(Debug, ToSql, FromSql, Clone)]
#[postgres(name = "version")]
struct Version {
    major: i32,
    minor: i32,
    patch: i32,
    pre: Option<String>,
    build: Option<String>,
}

impl From<&semver::Version> for Version {
    fn from(v: &semver::Version) -> Self {
        let pre: String = v
            .pre
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<String>>()
            .join(".");
        let build: String = v
            .build
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<String>>()
            .join(".");
        Self {
            major: v.major as i32,
            minor: v.minor as i32,
            patch: v.patch as i32,
            pre: if pre.is_empty() { None } else { Some(pre) },
            build: if build.is_empty() { None } else { Some(build) },
        }
    }
}

impl TryFrom<Version> for semver::Version {
    type Error = String;
    fn try_from(v: Version) -> Result<Self, Self::Error> {
        let pre = v.pre.map(|p| format!("-{}", p)).unwrap_or_default();
        let build = v.build.map(|p| format!("+{}", p)).unwrap_or_default();
        semver::Version::parse(&format!(
            "{}.{}.{}{}{}",
            v.major, v.minor, v.patch, pre, build
        ))
        .map_err(|e| {
            format!(
                "Failed to parse semantic version from the PostgreSQL database: {}",
                e
            )
        })
    }
}

#[derive(Debug, ToSql, FromSql)]
#[postgres(name = "version_comparator")]
struct VersionComparator {
    version: Version,
    op: String,
}

struct VersionRequirements(Vec<VersionComparator>);

impl TryFrom<&semver::VersionReq> for VersionRequirements {
    type Error = String;
    fn try_from(req: &semver::VersionReq) -> Result<Self, Self::Error> {
        Ok(Self(
            semver_parser::RangeSet::parse(&req.to_string(), semver_parser::Compat::Cargo)
                .map_err(|e| format!("Failed to parse version requirement: {}", e))?
                .ranges
                .iter()
                .flat_map(|range| {
                    range
                        .comparator_set
                        .iter()
                        .map(|comparator| VersionComparator {
                            version: Version {
                                major: comparator.major as i32,
                                minor: comparator.minor as i32,
                                patch: comparator.patch as i32,
                                pre: comparator.pre.iter().fold(None, |acc, pred| {
                                    Some(format!(
                                        "{}{}",
                                        acc.map(|s| format!("{}.", s)).unwrap_or_default(),
                                        match pred {
                                            semver_parser::Identifier::Numeric(n) => n.to_string(),
                                            semver_parser::Identifier::AlphaNumeric(a) =>
                                                a.to_string(),
                                        }
                                    ))
                                }),
                                build: None,
                            },
                            op: match comparator.op {
                                semver_parser::Op::Lt => "<",
                                semver_parser::Op::Lte => "<=",
                                semver_parser::Op::Gt => ">",
                                semver_parser::Op::Gte => ">=",
                                semver_parser::Op::Eq => "=",
                            }
                            .to_owned(),
                        })
                })
                .collect(),
        ))
    }
}

#[derive(Debug, ToSql, FromSql)]
#[postgres(name = "attachments")]
struct Attachment {
    id: Uuid,
    name: String,
    metadata: HashMap<String, Option<String>>,
    checksums: Checksums,
    created_at: SystemTime,
    publisher_id: Uuid,
    signature: Option<Vec<u8>>,
}

#[derive(Debug, ToSql, FromSql)]
#[postgres(name = "attachment_with_publisher")]
struct AttachmentWithPublisher {
    attachment: Attachment,
    publisher: Publisher,
}

impl From<AttachmentWithPublisher> for storage::FunctionAttachment {
    fn from(a: AttachmentWithPublisher) -> Self {
        Self {
            id: a.attachment.id,
            data: storage::FunctionAttachmentData {
                name: a.attachment.name,
                // unwrap is ok here since we know what we put in
                metadata: a
                    .attachment
                    .metadata
                    .into_iter()
                    .map(|(k, v)| (k, v.unwrap()))
                    .collect(),
                checksums: a.attachment.checksums.into(),
                publisher: storage::Publisher {
                    name: a.publisher.name,
                    email: a.publisher.email,
                },
                signature: a.attachment.signature,
            },
            created_at: a
                .attachment
                .created_at
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }
}

#[derive(Debug, ToSql, FromSql)]
#[postgres(name = "function_with_attachments")]
struct FunctionWithAttachments {
    func: Function,
    attachment_ids: Vec<Uuid>,
    publisher: Publisher,
}

impl TryFrom<FunctionWithAttachments> for storage::Function {
    type Error = storage::StorageError;
    fn try_from(f: FunctionWithAttachments) -> Result<Self, Self::Error> {
        Ok(Self {
            name: f.func.name,
            version: f
                .func
                .version
                .try_into()
                .map_err(|e: String| storage::StorageError::BackendError(e.into()))?,
            runtime: f.func.runtime.into(),
            required_inputs: ChannelSpecs(f.func.required_inputs).into(),
            optional_inputs: ChannelSpecs(f.func.optional_inputs).into(),
            outputs: ChannelSpecs(f.func.outputs).into(),
            metadata: f
                .func
                .metadata
                .into_iter()
                .map(|(k, v)| (k, v.unwrap()))
                .collect(),
            code: f.func.code,
            attachments: f.attachment_ids,
            created_at: f
                .func
                .created_at
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            publisher: storage::Publisher {
                name: f.publisher.name.clone(),
                email: f.publisher.email.clone(),
            },
            signature: f.func.signature,
        })
    }
}

#[derive(Debug, ToSql, FromSql)]
#[postgres(name = "channel_spec")]
struct ChannelSpec {
    name: String,
    description: String,
    argument_type: Type,
}

struct ChannelSpecs(Vec<ChannelSpec>);

impl From<HashMap<String, storage::ChannelSpec>> for ChannelSpecs {
    fn from(specs: HashMap<String, storage::ChannelSpec>) -> Self {
        Self(
            specs
                .into_iter()
                .map(|(name, channel_spec)| ChannelSpec {
                    name,
                    description: channel_spec.description,
                    argument_type: channel_spec.argument_type.into(),
                })
                .collect(),
        )
    }
}

impl From<ChannelSpecs> for HashMap<String, storage::ChannelSpec> {
    fn from(stream_spec: ChannelSpecs) -> Self {
        stream_spec
            .0
            .into_iter()
            .map(|channel_spec| {
                (
                    channel_spec.name,
                    storage::ChannelSpec {
                        description: channel_spec.description,
                        argument_type: channel_spec.argument_type.into(),
                    },
                )
            })
            .collect()
    }
}

impl From<ChannelType> for Type {
    fn from(pa: ChannelType) -> Self {
        match pa {
            ChannelType::String => Type::String,
            ChannelType::Bool => Type::Bool,
            ChannelType::Int => Type::Int,
            ChannelType::Float => Type::Float,
            ChannelType::Bytes => Type::Bytes,
        }
    }
}

impl From<Type> for ChannelType {
    fn from(at: Type) -> Self {
        match at {
            Type::String => ChannelType::String,
            Type::Bool => ChannelType::Bool,
            Type::Int => ChannelType::Int,
            Type::Float => ChannelType::Float,
            Type::Bytes => ChannelType::Bytes,
        }
    }
}

#[derive(Debug, ToSql, FromSql)]
#[postgres(name = "runtime")]
pub struct Runtime {
    pub name: String,
    pub entrypoint: String,
    pub arguments: HashMap<String, Option<String>>,
}

impl From<storage::Runtime> for Runtime {
    fn from(ee: super::Runtime) -> Self {
        Self {
            name: ee.name,
            entrypoint: ee.entrypoint,
            arguments: HStore(ee.arguments).into(),
        }
    }
}

impl From<Runtime> for storage::Runtime {
    fn from(ee: Runtime) -> Self {
        Self {
            name: ee.name,
            entrypoint: ee.entrypoint,
            arguments: ee
                .arguments
                .into_iter()
                .map(|(k, v)| (k, v.unwrap()))
                .collect(),
        }
    }
}

#[derive(Debug, ToSql, FromSql)]
#[postgres(name = "checksums")]
struct Checksums {
    sha256: String,
}

impl From<storage::Checksums> for Checksums {
    fn from(chksums: storage::Checksums) -> Self {
        Self {
            sha256: chksums.sha256,
        }
    }
}

impl From<Checksums> for storage::Checksums {
    fn from(chksums: Checksums) -> Self {
        Self {
            sha256: chksums.sha256,
        }
    }
}

struct HStore(HashMap<String, String>);

impl From<HStore> for HashMap<String, Option<String>> {
    fn from(hs: HStore) -> Self {
        hs.0.into_iter().map(|(n, v)| (n, Some(v))).collect()
    }
}

pub struct PostgresStorage {
    connection_pool: bb8::Pool<PostgresConnectionManager<NoTls>>,
    log: slog::Logger,
}

impl PostgresStorage {
    pub async fn new(uri: &url::Url, log: Logger) -> Result<Self, storage::StorageError> {
        let config: tokio_postgres::Config = uri.to_string().parse().map_err(|e| {
            storage::StorageError::ConnectionError(format!("Invalid postgresql url: {}", e))
        })?;

        info!(
            log,
            "connecting to postgresql database \"{}\" at {:#?} as user {}.",
            config.get_dbname().unwrap_or("<default>"),
            config.get_hosts(),
            config.get_user().unwrap_or("<default_user>"),
        );
        let manager = PostgresConnectionManager::new(config, NoTls);
        let storage = Self {
            connection_pool: bb8::Pool::builder().build(manager).await.map_err(|e| {
                storage::StorageError::ConnectionError(format!(
                    "Failed to create postgresql pool: {}",
                    e
                ))
            })?,

            log,
        };

        Ok(storage)
    }

    async fn insert_publisher(
        &self,
        publisher: &storage::Publisher,
    ) -> Result<Publisher, storage::StorageError> {
        self.get_connection()
            .await?
            .query_one(
                "select insert_or_get_publisher($1, $2)",
                &[&publisher.name, &publisher.email],
            )
            .await
            .map_err(|e| storage::StorageError::BackendError(Box::new(e)))
            .map(|row| row.get::<_, Publisher>(0))
    }

    async fn get_connection(
        &self,
    ) -> Result<bb8::PooledConnection<'_, PostgresConnectionManager<NoTls>>, storage::StorageError>
    {
        self.connection_pool
            .get()
            .map_err(|e| {
                storage::StorageError::BackendError(
                    format!("Failed to obtain sql connection from pool: {}", e).into(),
                )
            })
            .await
    }

    pub async fn new_with_init(uri: &url::Url, log: Logger) -> Result<Self, storage::StorageError> {
        let storage = Self::new(uri, log).await?;

        info!(storage.log, "initializing database");
        storage.create_tables().await?;

        Ok(storage)
    }

    async fn create_tables(&self) -> Result<(), storage::StorageError> {
        info!(self.log, "executing sql file sql/create-tables.sql");
        self.get_connection()
            .and_then(|c| async move {
                c.batch_execute(include_str!("sql/create-tables.sql"))
                    .map_err(|e| {
                        storage::StorageError::BackendError(
                            format!("Failed to run database initialization: {}", e).into(),
                        )
                    })
                    .await
            })
            .await
    }

    #[cfg(all(test, feature = "postgres-tests"))]
    async fn clear(&self) -> Result<(), storage::StorageError> {
        self.get_connection()
            .await?
            .batch_execute("select clear_tables();")
            .await
            .map_err(|e| {
                storage::StorageError::BackendError(
                    format!("Failed to clear database: {}", e).into(),
                )
            })
    }
}

struct StringAdapter(storage::OrderingKey);

impl std::fmt::Display for StringAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self.0 {
                storage::OrderingKey::NameVersion => "name_version",
            }
        )
    }
}

#[async_trait::async_trait]
impl storage::FunctionStorage for PostgresStorage {
    async fn insert(
        &self,
        function_data: storage::Function,
    ) -> Result<storage::Function, storage::StorageError> {
        let metadata: HashMap<String, Option<String>> = HStore(function_data.metadata).into();
        let rt: Runtime = function_data.runtime.into();

        let name = function_data.name.clone();
        let version = function_data.version.clone();

        self.get_connection()
            .await?
            .query_one(
                "select insert_function($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
                &[
                    &name,
                    &Version::from(&function_data.version),
                    &metadata,
                    &function_data.code,
                    &ChannelSpecs::from(function_data.required_inputs).0,
                    &ChannelSpecs::from(function_data.optional_inputs).0,
                    &ChannelSpecs::from(function_data.outputs).0,
                    &rt,
                    &function_data.attachments,
                    &self.insert_publisher(&function_data.publisher).await?.id,
                    &function_data.signature,
                ],
            )
            .await
            .map_err(|e| match e.code() {
                Some(c) => {
                    if c == &tokio_postgres::error::SqlState::UNIQUE_VIOLATION {
                        storage::StorageError::VersionExists { name, version }
                    } else {
                        storage::StorageError::BackendError(Box::new(e))
                    }
                }
                None => storage::StorageError::BackendError(Box::new(e)),
            })
            .and_then(|row| row.get::<_, FunctionWithAttachments>(0).try_into())
    }

    async fn insert_attachment(
        &self,
        function_attachment_data: storage::FunctionAttachmentData,
    ) -> Result<storage::FunctionAttachment, storage::StorageError> {
        let metadata: HashMap<String, Option<String>> =
            HStore(function_attachment_data.metadata).into();
        let checksums = Checksums::from(function_attachment_data.checksums);

        self.get_connection()
            .await?
            .query_one(
                "select insert_attachment($1, $2, $3, $4, $5)",
                &[
                    &function_attachment_data.name,
                    &metadata,
                    &checksums,
                    &self
                        .insert_publisher(&function_attachment_data.publisher)
                        .await?
                        .id,
                    &function_attachment_data.signature,
                ],
            )
            .await
            .map_err(|e| {
                storage::StorageError::BackendError(
                    format!("Failed to insert attachment: {}", e).into(),
                )
            })
            .map(|row| row.get::<_, AttachmentWithPublisher>(0).into())
    }

    async fn get(
        &self,
        id: &storage::FunctionId,
    ) -> Result<storage::Function, storage::StorageError> {
        self.get_connection()
            .await?
            .query(
                "select get_function($1, $2)",
                &[&id.name, &Version::from(&id.version)],
            )
            .await
            .map_err(|e| {
                storage::StorageError::BackendError(
                    format!("Failed to get function with id {}: {}", id, e).into(),
                )
            })
            .and_then(|mut rows| {
                rows.pop()
                    .ok_or_else(|| storage::StorageError::FunctionNotFound(id.to_string()))
            })
            .and_then(|row| row.get::<_, FunctionWithAttachments>(0).try_into())
    }

    async fn get_attachment(
        &self,
        id: &Uuid,
    ) -> Result<storage::FunctionAttachment, storage::StorageError> {
        self.get_connection()
            .await?
            .query("select get_attachment($1)", &[&id])
            .await
            .map_err(|e| {
                storage::StorageError::BackendError(
                    format!("Failed to get attachment with id {}: {}", id, e).into(),
                )
            })
            .and_then(|mut rows| {
                rows.pop()
                    .ok_or_else(|| storage::StorageError::AttachmentNotFound(id.to_string()))
            })
            .map(|row| row.get::<_, AttachmentWithPublisher>(0).into())
    }

    async fn list(
        &self,
        filters: &storage::Filters,
    ) -> Result<Vec<storage::Function>, storage::StorageError> {
        let order = filters.order.as_ref().cloned().unwrap_or_default();
        self.get_connection()
            .await?
            .query(
                "select list_functions($1, $2, $3, $4, $5, $6, $7, $8)",
                &[
                    &filters.name,
                    &filters.metadata,
                    &(order.offset as i64),
                    &(order.limit as i64),
                    &StringAdapter(order.key).to_string(),
                    &order.reverse,
                    &filters
                        .version_requirement
                        .as_ref()
                        .map(|vr| {
                            VersionRequirements::try_from(vr)
                                .map_err(|e: String| storage::StorageError::BackendError(e.into()))
                                .map(|vreq| vreq.0)
                        })
                        .transpose()?
                        .and_then(|version_filters| {
                            if version_filters.is_empty() {
                                None
                            } else {
                                Some(version_filters)
                            }
                        }),
                    &filters.publisher_email,
                ],
            )
            .await
            .map_err(|e| {
                storage::StorageError::BackendError(
                    format!("Failed to list functions: {}", e).into(),
                )
            })
            .and_then(|rows| {
                rows.into_iter()
                    .map(|row| row.get::<_, FunctionWithAttachments>(0).try_into())
                    .collect()
            })
    }

    async fn list_versions(
        &self,
        filters: &storage::Filters,
    ) -> Result<Vec<storage::Function>, storage::StorageError> {
        let order = filters.order.as_ref().cloned().unwrap_or_default();
        self.get_connection()
            .await?
            .query(
                "select list_versions($1, $2, $3, $4, $5, $6, $7, $8)",
                &[
                    &filters.name,
                    &filters.metadata,
                    &(order.offset as i64),
                    &(order.limit as i64),
                    &StringAdapter(order.key).to_string(),
                    &order.reverse,
                    &filters
                        .version_requirement
                        .as_ref()
                        .map(|vr| {
                            VersionRequirements::try_from(vr)
                                .map_err(|e: String| storage::StorageError::BackendError(e.into()))
                                .map(|vreq| vreq.0)
                        })
                        .transpose()?
                        .and_then(|version_filters| {
                            if version_filters.is_empty() {
                                None
                            } else {
                                Some(version_filters)
                            }
                        }),
                    &filters.publisher_email,
                ],
            )
            .await
            .map_err(|e| {
                storage::StorageError::BackendError(
                    format!("Failed to list functions: {}", e).into(),
                )
            })
            .and_then(|rows| {
                rows.into_iter()
                    .map(|row| row.get::<_, FunctionWithAttachments>(0).try_into())
                    .collect()
            })
    }
}

#[cfg(all(test, feature = "postgres-tests"))]
mod tests {
    use std::collections::HashMap;
    use std::panic::{self, AssertUnwindSafe};
    use tokio::sync::Mutex;

    use config::File as ConfigFile;
    use futures::FutureExt;
    use lazy_static::lazy_static;
    use semver::{Version, VersionReq};
    use url::Url;

    use super::*;
    use crate::config::Configuration;
    use crate::storage::{self as storage, FunctionStorage};

    macro_rules! null_logger {
        () => {{
            slog::Logger::root(slog::Discard, slog::o!())
        }};
    }

    lazy_static! {
        static ref SQL_MUTEX: Mutex<()> = Mutex::new(());
    }

    macro_rules! with_db {
        ($database:ident, $body:block) => {{
            let config = Configuration::new_with_init(
                null_logger!(),
                ConfigFile::from_str(
                    r#"
attachment_storage_uri = "https://i-am-attachment.org"
"#,
                    config::FileFormat::Toml,
                ),
            )
            .await
            .unwrap();
            let url = Url::parse(&config.functions_storage_uri).unwrap();
            let guard = SQL_MUTEX.lock().await;
            if let Err(e) = AssertUnwindSafe(async move {
                let $database = PostgresStorage::new_with_init(&url, null_logger!()).await;
                if let Ok(ref db) = $database {
                    db.clear().await.unwrap();
                }
                $body
            })
            .catch_unwind()
            .await
            {
                drop(guard);
                panic::resume_unwind(e);
            }
        }};
    }

    macro_rules! hashmap {
        ($( $key: expr => $val: expr ),*) => {{
            let mut map = ::std::collections::HashMap::new();
            $( map.insert($key, $val); )*
                map
        }}
    }

    macro_rules! string_hashmap {
        ($( $key: expr => $val: expr ),*) => {{
            let mut map = ::std::collections::HashMap::new();
            $( map.insert(String::from($key), String::from($val)); )*
                map
        }}
    }

    #[tokio::test]
    async fn init_ok() {
        with_db!(db, {
            assert!(db.is_ok());
        });
    }

    #[tokio::test]
    async fn insert_function() {
        with_db!(db, {
            let storage = db.unwrap();
            let data = storage::Function {
                name: "Super Snek!".to_owned(),
                version: Version::new(1, 2, 3),
                runtime: storage::Runtime {
                    name: "springtid".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    arguments: HashMap::new(),
                },
                required_inputs: HashMap::new(),
                optional_inputs: HashMap::new(),
                outputs: HashMap::new(),
                metadata: HashMap::new(),
                code: None,
                attachments: vec![],
                created_at: 0,
                publisher: storage::Publisher {
                    name: String::from("sune"),
                    email: String::from("sune@sune.com"),
                },
                signature: None,
            };

            let res = storage.insert(data).await;
            assert!(res.is_ok());

            // make sure timestamp was set
            assert_ne!(res.unwrap().created_at, 0);

            // Insert another function with same name and version (which shouldn't work)
            let same_data = storage::Function {
                name: "Super Snek!".to_owned(),
                version: Version::new(1, 2, 3),
                runtime: storage::Runtime {
                    name: "springtid".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    arguments: HashMap::new(),
                },
                required_inputs: HashMap::new(),
                optional_inputs: HashMap::new(),
                outputs: HashMap::new(),
                metadata: HashMap::new(),
                code: None,
                attachments: vec![],
                created_at: 0,
                publisher: storage::Publisher {
                    name: String::from("sune"),
                    email: String::from("sune@sune.com"),
                },
                signature: None,
            };

            let res = storage.insert(same_data).await;

            assert!(matches!(
                res.unwrap_err(),
                storage::StorageError::VersionExists { .. }
            ));

            // Insert same function but newer with different version
            let data = storage::Function {
                name: "Super Snek!".to_owned(),
                version: Version::new(1, 2, 4),
                runtime: storage::Runtime {
                    name: "springtid".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    arguments: HashMap::new(),
                },
                required_inputs: HashMap::new(),
                optional_inputs: HashMap::new(),
                outputs: HashMap::new(),
                metadata: HashMap::new(),
                code: None,
                attachments: vec![],
                created_at: 0,
                publisher: storage::Publisher {
                    name: String::from("sune"),
                    email: String::from("sune@sune.com"),
                },
                signature: None,
            };

            let res = storage.insert(data).await;
            assert!(res.is_ok());

            // Insert different function but with same name as other
            let data = storage::Function {
                name: "Bad Snek!".to_owned(),
                version: Version::new(1, 2, 3),
                runtime: storage::Runtime {
                    name: "springtid".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    arguments: HashMap::new(),
                },
                required_inputs: HashMap::new(),
                optional_inputs: HashMap::new(),
                outputs: HashMap::new(),
                metadata: HashMap::new(),
                code: None,
                attachments: vec![],
                created_at: 0,
                publisher: storage::Publisher {
                    name: String::from("sune"),
                    email: String::from("sune@sune.com"),
                },
                signature: None,
            };

            let res = storage.insert(data).await;
            assert!(res.is_ok());

            // Test to use all fields
            let attachment1 = storage::FunctionAttachmentData {
                name: "Attached super snek!".to_owned(),
                metadata: hashmap!("meta".to_owned() => "data".to_owned()),
                checksums: super::super::Checksums {
                    sha256: "6f7c7128c358626cfea2a83173b1626ec18412962969baba819e1ece1b22907e"
                        .to_owned(),
                },
                publisher: storage::Publisher {
                    name: "Sunba".to_owned(),
                    email: "kolbals@korven.se".to_owned(),
                },
                signature: None,
            };

            let r = storage.insert_attachment(attachment1).await;
            assert!(r.is_ok());
            let attachment1 = r.unwrap();

            let attachment2 = storage::FunctionAttachmentData {
                name: "Attached inferior snek!".to_owned(),
                metadata: hashmap!("mita".to_owned() => "deta".to_owned()),
                checksums: super::super::Checksums {
                    sha256: "6f7c7128c358626cfea2a83173b1626ec18412962969baba819e1ece1b22907e"
                        .to_owned(),
                },
                publisher: storage::Publisher {
                    name: "Bunba".to_owned(),
                    email: "korven@korven.se".to_owned(),
                },
                signature: None,
            };

            let r = storage.insert_attachment(attachment2).await;
            assert!(r.is_ok());
            let attachment2 = r.unwrap();

            let data = storage::Function {
                name: "Bauta Snek!".to_owned(),
                version: Version::new(99, 3, 4),
                runtime: storage::Runtime {
                    name: "springtid".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    arguments: hashmap!("arg1".to_owned() => "some/path".to_owned(), "bune".to_owned() => "rune".to_owned()),
                },

                required_inputs: hashmap!("best_input".to_owned() => storage::ChannelSpec {
                    description: "notig".to_owned(),
                    argument_type: storage::ChannelType::String,
                }),
                optional_inputs: hashmap!("worst_input".to_owned() => storage::ChannelSpec {
                    argument_type: storage::ChannelType::Bool,
                    description: "it truly is".to_owned(),
                }),

                outputs: hashmap!("best output".to_owned() => storage::ChannelSpec {
                    argument_type: storage::ChannelType::String,
                    description: "beskrivning".to_owned(),
                },
                "worst output".to_owned() => storage::ChannelSpec {
                    argument_type: storage::ChannelType::Bool,
                    description: "kuvaus".to_owned(),
                }),

                metadata: hashmap!("meta".to_owned() => "ja tack, fisk är gott".to_owned(), "will_explode".to_owned() => "very yes".to_owned()),
                code: Some(uuid::Uuid::new_v4()),
                attachments: vec![attachment1.id, attachment2.id],
                created_at: 0,

                publisher: storage::Publisher {
                    name: String::from("sune"),
                    email: String::from("sune@sune.com"),
                },
                signature: None,
            };

            let res = storage.insert(data).await;
            assert!(res.is_ok());
        });

        with_db!(db, {
            let storage = db.unwrap();

            let data = storage::Function {
                name: "Super Snek!".to_owned(),
                version: Version::new(1, 2, 3),
                runtime: storage::Runtime {
                    name: "springtid".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    arguments: HashMap::new(),
                },
                required_inputs: HashMap::new(),
                optional_inputs: HashMap::new(),
                outputs: HashMap::new(),
                metadata: HashMap::new(),
                code: None,
                attachments: vec![],
                created_at: 0,
                publisher: storage::Publisher {
                    name: String::from("sune"),
                    email: String::from("sune@sune.com"),
                },
                signature: None,
            };

            let res = storage.insert(data).await;
            assert!(res.is_ok());
            let ok_function = res.unwrap();

            // insert function with non-existent attachment
            // this should test that when inserting
            // into the attachment relation table fails,
            // the function is not inserted either
            // even though that happens before
            let data = storage::Function {
                name: "Snek with no attachment".to_owned(),
                version: Version::new(1, 2, 3),
                runtime: storage::Runtime {
                    name: "springtid".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    arguments: HashMap::new(),
                },
                required_inputs: HashMap::new(),
                optional_inputs: HashMap::new(),
                outputs: HashMap::new(),
                metadata: HashMap::new(),
                code: None,
                attachments: vec![Uuid::new_v4()],
                created_at: 0,
                publisher: storage::Publisher {
                    name: String::from("sune"),
                    email: String::from("sune@sune.com"),
                },
                signature: None,
            };

            let res = storage.insert(data).await;
            assert!(res.is_err());
            assert!(matches!(
                res.unwrap_err(),
                storage::StorageError::BackendError(..)
            ));

            let res = storage.list(&storage::Filters::default()).await;
            assert!(res.is_ok());
            let list = res.unwrap();
            assert_eq!(list.len(), 1);
            assert_eq!(
                list.first().map(storage::FunctionId::from),
                Some(storage::FunctionId::from(&ok_function)),
            );
            assert_ne!(ok_function.created_at, 0);
        });
    }

    #[tokio::test]
    async fn get_function() {
        with_db!(db, {
            let storage = db.unwrap();
            let publisher = storage::Publisher {
                name: String::from("sune"),
                email: String::from("sune@sune.com"),
            };
            let data = storage::Function {
                name: "Super Snek!".to_owned(),
                version: Version::new(1, 2, 3),
                runtime: storage::Runtime {
                    name: "springtid".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    arguments: HashMap::new(),
                },
                required_inputs: HashMap::new(),
                optional_inputs: HashMap::new(),
                outputs: HashMap::new(),
                metadata: HashMap::new(),
                code: None,
                attachments: vec![],
                created_at: 0,
                publisher: publisher.clone(),
                signature: Some(b"1234".to_vec()),
            };

            let id = storage::FunctionId::from(&storage.insert(data).await.unwrap());
            let res = storage.get(&id).await;

            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(storage::FunctionId::from(&res), id);
            assert_eq!(b"1234", &res.signature.unwrap()[..4]);
            assert_eq!(res.publisher, publisher);

            // test nonexistent
            let nilid = storage::FunctionId {
                name: String::new(),
                version: semver::Version::new(0, 0, 0),
            };
            let res = storage.get(&nilid).await;
            assert!(res.is_err());
            assert!(matches!(
                res.unwrap_err(),
                storage::StorageError::FunctionNotFound(..)
            ));
        });
    }

    #[tokio::test]
    async fn get_function_with_attachment() {
        with_db!(db, {
            let storage = db.unwrap();
            // Test to use all fields
            let attachment1 = storage::FunctionAttachmentData {
                name: "Attached super snek!".to_owned(),
                metadata: hashmap!("meta".to_owned() => "data".to_owned()),
                checksums: super::super::Checksums {
                    sha256: "6f7c7128c358626cfea2a83173b1626ec18412962969baba819e1ece1b22907e"
                        .to_owned(),
                },
                publisher: storage::Publisher {
                    name: "Oran Gutang".to_owned(),
                    email: "Gutang@oran.se".to_owned(),
                },
                signature: Some(b"ababa".to_vec()),
            };

            let r = storage.insert_attachment(attachment1).await;
            assert!(r.is_ok());
            let att1_id = r.unwrap().id;

            let data = storage::Function {
                name: "Super Snek!".to_owned(),
                version: Version::new(1, 2, 3),
                runtime: storage::Runtime {
                    name: "springtid".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    arguments: HashMap::new(),
                },
                required_inputs: HashMap::new(),
                optional_inputs: HashMap::new(),
                outputs: HashMap::new(),
                metadata: HashMap::new(),
                code: None,
                attachments: vec![att1_id],
                created_at: 0,
                publisher: storage::Publisher {
                    name: String::from("sune"),
                    email: String::from("sune@sune.com"),
                },
                signature: None,
            };

            let id = storage::FunctionId::from(&storage.insert(data).await.unwrap());
            let res = storage.get(&id).await;
            assert!(res.is_ok());
            assert!(res.unwrap().attachments.contains(&att1_id));
        })
    }

    #[tokio::test]
    async fn get_attachment() {
        with_db!(db, {
            let storage = db.unwrap();
            // Test to use all fields
            let publisher = storage::Publisher {
                name: "Sten Snultra".to_owned(),
                email: "stensnulta@fisk.se".to_owned(),
            };
            let attachment1 = storage::FunctionAttachmentData {
                name: "Attached super snek!".to_owned(),
                metadata: hashmap!("meta".to_owned() => "data".to_owned()),
                checksums: super::super::Checksums {
                    sha256: "6f7c7128c358626cfea2a83173b1626ec18412962969baba819e1ece1b22907e"
                        .to_owned(),
                },
                publisher: publisher.clone(),
                signature: Some(b"sune".to_vec()),
            };

            let r = storage.insert_attachment(attachment1).await;
            assert!(r.is_ok());
            let r = r.unwrap();

            // make sure timestamp was set
            assert_ne!(r.created_at, 0);
            assert_eq!(r.data.publisher, publisher);
            assert_eq!(&r.data.signature.unwrap()[0..4], b"sune");

            let att1_id = r.id;

            let res = storage.get_attachment(&att1_id).await;
            assert!(res.is_ok());
            let returned_attachment = res.unwrap();
            assert_eq!(returned_attachment.id, att1_id);

            // test nonexistent
            let nilid = Uuid::nil();
            let res = storage.get_attachment(&nilid).await;
            assert!(res.is_err());
            assert!(matches!(
                res.unwrap_err(),
                storage::StorageError::AttachmentNotFound(..)
            ));
        });
    }

    #[tokio::test]
    async fn list() {
        with_db!(db, {
            let storage = db.unwrap();

            // empty list
            let res = storage.list(&storage::Filters::default()).await;
            assert!(res.is_ok());

            // Register and check registered items are listed
            let data = storage::Function {
                name: "Aaa".to_owned(),
                version: Version::new(1, 2, 3),
                runtime: storage::Runtime {
                    name: "springtid".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    arguments: HashMap::new(),
                },
                required_inputs: HashMap::new(),
                optional_inputs: HashMap::new(),
                outputs: HashMap::new(),
                metadata: string_hashmap!("fisk" => "haj", "orm" => "🐍", "snake" => "snek"),
                code: None,
                attachments: vec![],
                created_at: 0,
                publisher: storage::Publisher {
                    name: String::from("sune"),
                    email: String::from("sune@sune.com"),
                },
                signature: None,
            };

            let id = storage::FunctionId::from(&storage.insert(data).await.unwrap());
            let res = storage.list(&storage::Filters::default()).await;
            assert!(res.is_ok());

            let rows = res.unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(
                rows.first().map(storage::FunctionId::from).as_ref(),
                Some(&id)
            );

            // Test filtering
            let res = storage
                .list(&storage::Filters {
                    name: String::from("A"),
                    ..Default::default()
                })
                .await;
            assert!(res.is_ok());

            let rows = res.unwrap();
            assert_eq!(rows.len(), 1);

            // publisher
            let res = storage
                .list(&storage::Filters {
                    publisher_email: String::from("@sune.com"),
                    ..Default::default()
                })
                .await;
            assert!(res.is_ok());

            let rows = res.unwrap();
            assert_eq!(rows.len(), 1);

            // Test not finding
            let res = storage
                .list(&storage::Filters {
                    publisher_email: String::from("sune@suna.com"),
                    ..Default::default()
                })
                .await;
            assert!(res.is_ok());

            let rows = res.unwrap();
            assert!(rows.is_empty());

            // Test not finding publisher
            let res = storage
                .list(&storage::Filters {
                    name: String::from("B"),
                    ..Default::default()
                })
                .await;
            assert!(res.is_ok());

            let rows = res.unwrap();
            assert!(rows.is_empty());

            // Test versions (exact match on name)
            let res = storage
                .list_versions(&storage::Filters {
                    name: String::from("a"),
                    ..Default::default()
                })
                .await;
            assert!(res.is_ok());

            let rows = res.unwrap();
            assert!(
                rows.is_empty(),
                "Expected exact name match on non-existing function to return nothing"
            );

            let res = storage
                .list(&storage::Filters {
                    name: String::from("Aaa"),
                    ..Default::default()
                })
                .await;
            assert!(res.is_ok());

            let rows = res.unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(
                rows.first().map(storage::FunctionId::from).as_ref(),
                Some(&id)
            );

            // Metadata filtering
            let res = storage
            .list(&storage::Filters {
                metadata: hashmap!("fisk".to_owned() => Some("haj".to_owned()), "snake".to_owned() => Some("snek".to_owned())),
                ..Default::default()
            })
            .await;
            assert!(res.is_ok());

            let rows = res.unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(
                rows.first().map(storage::FunctionId::from).as_ref(),
                Some(&id)
            );

            // Existing key and wrong value is not a match
            let res = storage
                .list(&storage::Filters {
                    metadata: hashmap!("fisk".to_owned() => Some("🦈".to_owned())),
                    ..Default::default()
                })
                .await;
            assert!(res.is_ok());

            let rows = res.unwrap();
            assert!(rows.is_empty());

            // Existing key with other key's value is not a match
            let res = storage
                .list(&storage::Filters {
                    metadata: hashmap!("fisk".to_owned() => Some("🐍".to_owned())),
                    ..Default::default()
                })
                .await;
            assert!(res.is_ok());

            let rows = res.unwrap();
            assert!(rows.is_empty());

            // Check that filtering on key only work
            let res = storage
                .list(&storage::Filters {
                    metadata: hashmap!("fisk".to_owned() => None),
                    ..Default::default()
                })
                .await;
            assert!(res.is_ok());

            let rows = res.unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows.first().map(storage::FunctionId::from), Some(id));
        })
    }

    #[tokio::test]
    async fn order_offset_and_limit() {
        with_db!(db, {
            let storage = db.unwrap();

            let function1_1_0_0 = storage::Function {
                name: "function1".to_owned(),
                version: Version::new(1, 0, 0),
                runtime: storage::Runtime {
                    name: "springtid".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    arguments: HashMap::new(),
                },
                required_inputs: HashMap::new(),
                optional_inputs: HashMap::new(),
                outputs: HashMap::new(),
                metadata: string_hashmap!("fisk" => "haj", "orm" => "🐍", "snake" => "snek"),
                code: None,
                attachments: vec![],
                created_at: 0,
                publisher: storage::Publisher {
                    name: String::from("sune"),
                    email: String::from("sune@sune.com"),
                },
                signature: None,
            };

            let function1_1_1_1 = storage::Function {
                name: "function1".to_owned(),
                version: Version::new(1, 1, 1),
                runtime: storage::Runtime {
                    name: "springtid".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    arguments: HashMap::new(),
                },
                required_inputs: HashMap::new(),
                optional_inputs: HashMap::new(),
                outputs: HashMap::new(),
                metadata: string_hashmap!("fisk" => "torsk", "orm" => "🐍", "snake" => "snek"),
                code: None,
                attachments: vec![],
                created_at: 0,
                publisher: storage::Publisher {
                    name: String::from("sune"),
                    email: String::from("sune@sune.com"),
                },
                signature: None,
            };

            let function2_1_1_1 = storage::Function {
                name: "function2".to_owned(),
                version: Version::new(1, 1, 1),
                runtime: storage::Runtime {
                    name: "springtid".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    arguments: HashMap::new(),
                },
                required_inputs: HashMap::new(),
                optional_inputs: HashMap::new(),
                outputs: HashMap::new(),
                metadata: string_hashmap!("fisk" => "abborre", "orm" => "🐍", "snake" => "snek"),
                code: None,
                attachments: vec![],
                created_at: 0,
                publisher: storage::Publisher {
                    name: String::from("sune"),
                    email: String::from("sune@sune.com"),
                },
                signature: None,
            };

            let function2_1_0_0 = storage::Function {
                name: "function2".to_owned(),
                version: Version::new(1, 0, 0),
                runtime: storage::Runtime {
                    name: "springtid".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    arguments: HashMap::new(),
                },
                required_inputs: HashMap::new(),
                optional_inputs: HashMap::new(),
                outputs: HashMap::new(),
                metadata: string_hashmap!("fisk" => "berggylta", "orm" => "nej", "snake" => "snek"),
                code: None,
                attachments: vec![],
                created_at: 0,
                publisher: storage::Publisher {
                    name: String::from("sune"),
                    email: String::from("sune@sune.com"),
                },
                signature: None,
            };

            let _function1_1_0_0_id =
                storage::FunctionId::from(&storage.insert(function1_1_0_0).await.unwrap());
            let _function1_1_1_1_id =
                storage::FunctionId::from(&storage.insert(function1_1_1_1).await.unwrap());

            let function2_1_1_1_id =
                storage::FunctionId::from(&storage.insert(function2_1_1_1).await.unwrap());
            let _ = storage::FunctionId::from(&storage.insert(function2_1_0_0).await.unwrap());

            let res = storage
                .list(&storage::Filters {
                    order: Some(storage::Ordering {
                        offset: 1,
                        limit: 2,
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(res.len(), 1);
            assert_eq!(res.first().map(|f| f.name.as_ref()), Some("function2"));
            assert_eq!(
                res.first().map(storage::FunctionId::from).as_ref(),
                Some(&function2_1_1_1_id)
            );

            // reverse
            let res = storage
                .list(&storage::Filters {
                    order: Some(storage::Ordering {
                        limit: 1,
                        reverse: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(res.len(), 1);
            assert_eq!(res.first().map(|f| f.name.as_ref()), Some("function2"));
            assert_eq!(
                res.first().map(storage::FunctionId::from),
                Some(function2_1_1_1_id),
                "Expected reverse ordering to still return latest version (list reverses name sorting)"
            );
        });

        with_db!(db, {
            let storage = db.unwrap();

            // test version ordering
            let function2_1_10_0 = storage::Function {
                name: "function2".to_owned(),
                version: Version::new(1, 10, 0),
                runtime: storage::Runtime {
                    name: "springtid".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    arguments: HashMap::new(),
                },
                required_inputs: HashMap::new(),
                optional_inputs: HashMap::new(),
                outputs: HashMap::new(),
                metadata: string_hashmap!("fisk" => "berggylta", "orm" => "nej", "snake" => "snek"),
                code: None,
                attachments: vec![],
                created_at: 0,
                publisher: storage::Publisher {
                    name: String::from("sune"),
                    email: String::from("sune@sune.com"),
                },
                signature: None,
            };

            let function2_1_2_0 = storage::Function {
                name: "function2".to_owned(),
                version: Version::new(1, 2, 0),
                runtime: storage::Runtime {
                    name: "springtid".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    arguments: HashMap::new(),
                },
                required_inputs: HashMap::new(),
                optional_inputs: HashMap::new(),
                outputs: HashMap::new(),
                metadata: string_hashmap!("fisk" => "sutare", "orm" => "nej", "snake" => "snek"),
                code: None,
                attachments: vec![],
                created_at: 0,
                publisher: storage::Publisher {
                    name: String::from("sune"),
                    email: String::from("sune@sune.com"),
                },
                signature: None,
            };

            let function2_1_10_0_id =
                storage::FunctionId::from(&storage.insert(function2_1_10_0).await.unwrap());
            let _function2_1_2_0_id =
                storage::FunctionId::from(&storage.insert(function2_1_2_0).await.unwrap());

            let res = storage
                .list_versions(&storage::Filters {
                    name: String::from("function2"),
                    ..Default::default()
                })
                .await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(
                res.first().map(storage::FunctionId::from).as_ref(),
                Some(&function2_1_10_0_id)
            );

            let function2_1_10_0_alpha = storage::Function {
                name: "function2".to_owned(),
                version: Version::parse("1.10.0-alpha").unwrap(),
                runtime: storage::Runtime {
                    name: "springtid".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    arguments: HashMap::new(),
                },
                required_inputs: HashMap::new(),
                optional_inputs: HashMap::new(),
                outputs: HashMap::new(),
                metadata: string_hashmap!("fisk" => "sutare", "orm" => "nej", "snake" => "snek"),
                code: None,
                attachments: vec![],
                created_at: 0,
                publisher: storage::Publisher {
                    name: String::from("sune"),
                    email: String::from("sune@sune.com"),
                },
                signature: None,
            };

            let function2_1_10_0_jaws = storage::Function {
                name: "function2".to_owned(),
                version: Version::parse("1.10.0-jaws").unwrap(),
                runtime: storage::Runtime {
                    name: "springtid".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    arguments: HashMap::new(),
                },
                required_inputs: HashMap::new(),
                optional_inputs: HashMap::new(),
                outputs: HashMap::new(),
                metadata: string_hashmap!("fisk" => "brugd", "orm" => "nej", "snake" => "snek"),
                code: None,
                attachments: vec![],
                created_at: 0,
                publisher: storage::Publisher {
                    name: String::from("sune"),
                    email: String::from("sune@sune.com"),
                },
                signature: None,
            };

            let function2_1_10_0_alpha_id =
                storage::FunctionId::from(&storage.insert(function2_1_10_0_alpha).await.unwrap());
            let _function2_1_10_0_jaws_id =
                storage::FunctionId::from(&storage.insert(function2_1_10_0_jaws).await.unwrap());

            let res = storage
                .list_versions(&storage::Filters {
                    name: String::from("function2"),
                    order: Some(storage::Ordering {
                        limit: 3,
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(
                res.first().map(storage::FunctionId::from),
                Some(function2_1_10_0_id)
            );
            assert_eq!(
                res.last().map(storage::FunctionId::from),
                Some(function2_1_10_0_alpha_id)
            );
        });
    }

    #[tokio::test]
    async fn version_filtering() {
        // Version filtering
        with_db!(db, {
            let storage = db.unwrap();

            let function_1_0_0 = storage::Function {
                name: "birb".to_owned(),
                version: Version::new(1, 0, 0),
                runtime: storage::Runtime {
                    name: "beepboop".to_owned(),
                    entrypoint: "abandonAllHopeYeWhoEntersHere".to_owned(),
                    arguments: HashMap::new(),
                },
                required_inputs: HashMap::new(),
                optional_inputs: HashMap::new(),
                outputs: HashMap::new(),
                metadata: HashMap::new(),
                code: None,
                attachments: vec![],
                created_at: 0,
                publisher: storage::Publisher {
                    name: String::from("sune"),
                    email: String::from("sune@sune.com"),
                },
                signature: None,
            };

            let function_1_1_0 = storage::Function {
                name: "birb".to_owned(),
                version: Version::new(1, 1, 0),
                runtime: storage::Runtime {
                    name: "beepboop".to_owned(),
                    entrypoint: "abandonAllHopeYeWhoEntersHere".to_owned(),
                    arguments: HashMap::new(),
                },
                required_inputs: HashMap::new(),
                optional_inputs: HashMap::new(),
                outputs: HashMap::new(),
                metadata: HashMap::new(),
                code: None,
                attachments: vec![],
                created_at: 0,
                publisher: storage::Publisher {
                    name: String::from("sune"),
                    email: String::from("sune@sune.com"),
                },
                signature: None,
            };

            let function_1_1_1 = storage::Function {
                name: "birb".to_owned(),
                version: Version::new(1, 1, 1),
                runtime: storage::Runtime {
                    name: "beepboop".to_owned(),
                    entrypoint: "abandonAllHopeYeWhoEntersHere".to_owned(),
                    arguments: HashMap::new(),
                },
                required_inputs: HashMap::new(),
                optional_inputs: HashMap::new(),
                outputs: HashMap::new(),
                metadata: HashMap::new(),
                code: None,
                attachments: vec![],
                created_at: 0,
                publisher: storage::Publisher {
                    name: String::from("sune"),
                    email: String::from("sune@sune.com"),
                },
                signature: None,
            };

            let function_1_2_0 = storage::Function {
                name: "birb".to_owned(),
                version: Version::new(1, 2, 0),
                runtime: storage::Runtime {
                    name: "beepboop".to_owned(),
                    entrypoint: "abandonAllHopeYeWhoEntersHere".to_owned(),
                    arguments: HashMap::new(),
                },
                required_inputs: HashMap::new(),
                optional_inputs: HashMap::new(),
                outputs: HashMap::new(),
                metadata: HashMap::new(),
                code: None,
                attachments: vec![],
                created_at: 0,
                publisher: storage::Publisher {
                    name: String::from("sune"),
                    email: String::from("sune@sune.com"),
                },
                signature: None,
            };

            let function_1_10_0 = storage::Function {
                name: "birb".to_owned(),
                version: Version::new(1, 10, 0),
                runtime: storage::Runtime {
                    name: "beepboop".to_owned(),
                    entrypoint: "abandonAllHopeYeWhoEntersHere".to_owned(),
                    arguments: HashMap::new(),
                },
                required_inputs: HashMap::new(),
                optional_inputs: HashMap::new(),
                outputs: HashMap::new(),
                metadata: HashMap::new(),
                code: None,
                attachments: vec![],
                created_at: 0,
                publisher: storage::Publisher {
                    name: String::from("sune"),
                    email: String::from("sune@sune.com"),
                },
                signature: None,
            };

            let function_2_2_0 = storage::Function {
                name: "birb".to_owned(),
                version: Version::new(2, 2, 0),
                runtime: storage::Runtime {
                    name: "beepboop".to_owned(),
                    entrypoint: "abandonAllHopeYeWhoEntersHere".to_owned(),
                    arguments: HashMap::new(),
                },
                required_inputs: HashMap::new(),
                optional_inputs: HashMap::new(),
                outputs: HashMap::new(),
                metadata: HashMap::new(),
                code: None,
                attachments: vec![],
                created_at: 0,
                publisher: storage::Publisher {
                    name: String::from("sune"),
                    email: String::from("sune@sune.com"),
                },
                signature: None,
            };

            let function_1_2_0_beta = storage::Function {
                name: "birb".to_owned(),
                version: Version::parse("1.2.0-beta").unwrap(),
                runtime: storage::Runtime {
                    name: "beepboop".to_owned(),
                    entrypoint: "abandonAllHopeYeWhoEntersHere".to_owned(),
                    arguments: HashMap::new(),
                },
                required_inputs: HashMap::new(),
                optional_inputs: HashMap::new(),
                outputs: HashMap::new(),
                metadata: HashMap::new(),
                code: None,
                attachments: vec![],
                created_at: 0,
                publisher: storage::Publisher {
                    name: String::from("sune"),
                    email: String::from("sune@sune.com"),
                },
                signature: None,
            };

            let function_1_0_0_id =
                storage::FunctionId::from(&storage.insert(function_1_0_0).await.unwrap());
            let _function_1_1_0_id =
                storage::FunctionId::from(&storage.insert(function_1_1_0).await.unwrap());
            let function_1_1_1_id =
                storage::FunctionId::from(&storage.insert(function_1_1_1).await.unwrap());
            let function_1_2_0_id =
                storage::FunctionId::from(&storage.insert(function_1_2_0).await.unwrap());
            let function_1_10_0_id =
                storage::FunctionId::from(&storage.insert(function_1_10_0).await.unwrap());
            let function_2_2_0_id =
                storage::FunctionId::from(&storage.insert(function_2_2_0).await.unwrap());
            let function_1_2_0_beta_id =
                storage::FunctionId::from(&storage.insert(function_1_2_0_beta).await.unwrap());

            // Exact match
            let mut filter = storage::Filters {
                name: String::from("birb"),
                version_requirement: Some(VersionReq::parse("=1.0.0").unwrap()),
                ..Default::default()
            };
            let res = storage.list_versions(&filter).await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(res.len(), 1);
            assert_eq!(
                res.first().map(storage::FunctionId::from),
                Some(function_1_0_0_id)
            );

            // Less than on full version
            filter.version_requirement = Some(VersionReq::parse("<1.2.0").unwrap());
            let res = storage.list_versions(&filter).await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(res.len(), 3);
            assert_eq!(
                res.first().map(storage::FunctionId::from).as_ref(),
                Some(&function_1_1_1_id)
            );

            // Less than on minor version
            filter.version_requirement = Some(VersionReq::parse("<1.10.0 >=1.0.0").unwrap());
            let res = storage.list_versions(&filter).await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(res.len(), 4);
            assert_eq!(
                res.first().map(storage::FunctionId::from),
                Some(function_1_2_0_id)
            );

            // Less than on major version
            filter.version_requirement = Some(VersionReq::parse("<2.0.0").unwrap());
            let res = storage.list_versions(&filter).await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(
                res.first().map(storage::FunctionId::from).as_ref(),
                Some(&function_1_10_0_id)
            );

            // There should not be a pre release here
            assert!(res.iter().all(|f| !f.version.is_prerelease()));

            // Less or equal
            filter.version_requirement = Some(VersionReq::parse("<=1.1.1").unwrap());
            let res = storage.list_versions(&filter).await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(
                res.first().map(storage::FunctionId::from).as_ref(),
                Some(&function_1_1_1_id)
            );

            // Greater
            filter.version_requirement = Some(VersionReq::parse(">1.1.1").unwrap());
            let res = storage.list_versions(&filter).await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(
                res.first().map(storage::FunctionId::from).as_ref(),
                Some(&function_2_2_0_id)
            );

            // Greater or equal
            filter.version_requirement = Some(VersionReq::parse(">=1.1.1").unwrap());
            let res = storage.list_versions(&filter).await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(
                res.first().map(storage::FunctionId::from),
                Some(function_2_2_0_id)
            );

            // Pre release only on exact match
            filter.version_requirement = Some(VersionReq::parse("=1.2.0-beta").unwrap());
            let res = storage.list_versions(&filter).await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(res.len(), 1);
            assert_eq!(
                res.first().map(storage::FunctionId::from),
                Some(function_1_2_0_beta_id)
            );

            // ~
            filter.version_requirement = Some(VersionReq::parse("~1").unwrap());
            let res = storage.list_versions(&filter).await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(res.len(), 5);
            assert_eq!(
                res.first().map(storage::FunctionId::from).as_ref(),
                Some(&function_1_10_0_id)
            );

            filter.version_requirement = Some(VersionReq::parse("~1.1").unwrap());
            let res = storage.list_versions(&filter).await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(res.len(), 2);
            assert_eq!(
                res.first().map(storage::FunctionId::from).as_ref(),
                Some(&function_1_1_1_id)
            );

            filter.version_requirement = Some(VersionReq::parse("~1.1.0").unwrap());
            let res = storage.list_versions(&filter).await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(res.len(), 2);
            assert_eq!(
                res.first().map(storage::FunctionId::from),
                Some(function_1_1_1_id)
            );

            // ^
            filter.version_requirement = Some(VersionReq::parse("^1.2.3").unwrap());
            let res = storage.list_versions(&filter).await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(res.len(), 1);
            assert_eq!(
                res.first().map(storage::FunctionId::from),
                Some(function_1_10_0_id)
            );

            // *
            filter.version_requirement = Some(VersionReq::parse("1.*").unwrap());
            let res = storage.list_versions(&filter).await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(res.len(), 5);
        });
    }
}
