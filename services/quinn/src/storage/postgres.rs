use std::{
    collections::hash_map::HashMap,
    convert::{TryFrom, TryInto},
};

use bb8_postgres::PostgresConnectionManager;
use futures::future::TryFutureExt;
use gbk_protocols::functions::ArgumentType as ProtoArgType;
use postgres_types::{FromSql, ToSql};
use slog::{info, Logger};
use tokio_postgres::NoTls;

use crate::storage;

use uuid::Uuid;

#[derive(Debug, ToSql, FromSql)]
#[postgres(name = "argument_type")]
enum ArgumentType {
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
    inputs: Vec<FunctionInput>,
    outputs: Vec<FunctionOutput>,
    execution_environment: ExecutionEnvironment,
}

#[derive(Debug, ToSql, FromSql)]
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
}

#[derive(Debug, ToSql, FromSql)]
#[postgres(name = "attachment_with_functions")]
struct AttachmentWithFunctions {
    attachment: Attachment,
    function_ids: Vec<Uuid>,
}

impl From<AttachmentWithFunctions> for storage::FunctionAttachment {
    fn from(a: AttachmentWithFunctions) -> Self {
        Self {
            id: a.attachment.id,
            function_ids: a.function_ids,
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
            },
        }
    }
}

#[derive(Debug, ToSql, FromSql)]
#[postgres(name = "function_with_attachments")]
struct FunctionWithAttachments {
    func: Function,
    attachment_ids: Vec<Uuid>,
}

impl TryFrom<FunctionWithAttachments> for storage::Function {
    type Error = storage::StorageError;
    fn try_from(f: FunctionWithAttachments) -> Result<Self, Self::Error> {
        Ok(Self {
            id: f.func.id,
            function_data: super::FunctionData {
                name: f.func.name,
                version: f
                    .func
                    .version
                    .try_into()
                    .map_err(|e: String| storage::StorageError::BackendError(e.into()))?,
                execution_environment: f.func.execution_environment.into(),
                inputs: f.func.inputs.into_iter().map(|i| i.into()).collect(),
                outputs: f.func.outputs.into_iter().map(|o| o.into()).collect(),
                metadata: f
                    .func
                    .metadata
                    .into_iter()
                    .map(|(k, v)| (k, v.unwrap()))
                    .collect(),
                code: f.func.code,
                attachments: f.attachment_ids,
            },
        })
    }
}

#[derive(Debug, ToSql, FromSql)]
#[postgres(name = "function_input")]
struct FunctionInput {
    name: String,
    required: bool,
    argument_type: ArgumentType,
    default_value: String,
    from_execution_environment: bool,
}

impl From<storage::FunctionInput> for FunctionInput {
    fn from(fi: storage::FunctionInput) -> Self {
        Self {
            name: fi.name,
            required: fi.required,
            argument_type: fi.argument_type.into(),
            default_value: fi.default_value,
            from_execution_environment: fi.from_execution_environment,
        }
    }
}

impl From<FunctionInput> for storage::FunctionInput {
    fn from(fi: FunctionInput) -> Self {
        Self {
            name: fi.name,
            required: fi.required,
            argument_type: fi.argument_type.into(),
            default_value: fi.default_value,
            from_execution_environment: fi.from_execution_environment,
        }
    }
}

impl From<ProtoArgType> for ArgumentType {
    fn from(pa: ProtoArgType) -> Self {
        match pa {
            ProtoArgType::String => ArgumentType::String,
            ProtoArgType::Bool => ArgumentType::Bool,
            ProtoArgType::Int => ArgumentType::Int,
            ProtoArgType::Float => ArgumentType::Float,
            ProtoArgType::Bytes => ArgumentType::Bytes,
        }
    }
}

impl From<ArgumentType> for ProtoArgType {
    fn from(at: ArgumentType) -> Self {
        match at {
            ArgumentType::String => ProtoArgType::String,
            ArgumentType::Bool => ProtoArgType::Bool,
            ArgumentType::Int => ProtoArgType::Int,
            ArgumentType::Float => ProtoArgType::Float,
            ArgumentType::Bytes => ProtoArgType::Bytes,
        }
    }
}

#[derive(Debug, ToSql, FromSql)]
#[postgres(name = "function_output")]
struct FunctionOutput {
    name: String,
    argument_type: ArgumentType,
}

impl From<storage::FunctionOutput> for FunctionOutput {
    fn from(fo: super::FunctionOutput) -> Self {
        Self {
            name: fo.name,
            argument_type: fo.argument_type.into(),
        }
    }
}

impl From<FunctionOutput> for storage::FunctionOutput {
    fn from(fo: FunctionOutput) -> Self {
        Self {
            name: fo.name,
            argument_type: fo.argument_type.into(),
        }
    }
}

#[derive(Debug, ToSql, FromSql)]
#[postgres(name = "execution_environment")]
pub struct ExecutionEnvironment {
    pub name: String,
    pub entrypoint: String,
    pub arguments: HashMap<String, Option<String>>,
}

impl From<storage::ExecutionEnvironment> for ExecutionEnvironment {
    fn from(ee: super::ExecutionEnvironment) -> Self {
        Self {
            name: ee.name,
            entrypoint: ee.entrypoint,
            arguments: HStore(ee.function_arguments).into(),
        }
    }
}

impl From<ExecutionEnvironment> for storage::ExecutionEnvironment {
    fn from(ee: ExecutionEnvironment) -> Self {
        Self {
            name: ee.name,
            entrypoint: ee.entrypoint,
            function_arguments: ee
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
                storage::OrderingKey::Name => "name",
            }
        )
    }
}

#[async_trait::async_trait]
impl storage::FunctionStorage for PostgresStorage {
    async fn insert(
        &self,
        function_data: storage::FunctionData,
    ) -> Result<uuid::Uuid, storage::StorageError> {
        let metadata: HashMap<String, Option<String>> = HStore(function_data.metadata).into();
        let ee: ExecutionEnvironment = function_data.execution_environment.into();

        let name = function_data.name.clone();
        let version = function_data.version.clone();

        self.get_connection()
            .await?
            .query_one(
                "select insert_function($1, $2, $3, $4, $5, $6, $7, $8)",
                &[
                    &name,
                    &Version::from(&function_data.version),
                    &metadata,
                    &function_data.code,
                    &function_data
                        .inputs
                        .into_iter()
                        .map(FunctionInput::from)
                        .collect::<Vec<FunctionInput>>(),
                    &function_data
                        .outputs
                        .into_iter()
                        .map(FunctionOutput::from)
                        .collect::<Vec<FunctionOutput>>(),
                    &ee,
                    &function_data.attachments,
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
            .map(|r| r.get(0))
    }

    async fn insert_attachment(
        &self,
        function_attachment_data: storage::FunctionAttachmentData,
    ) -> Result<uuid::Uuid, storage::StorageError> {
        let metadata: HashMap<String, Option<String>> =
            HStore(function_attachment_data.metadata).into();
        let checksums = Checksums::from(function_attachment_data.checksums);

        self.get_connection()
            .await?
            .query_one(
                "select insert_attachment($1, $2, $3)",
                &[&function_attachment_data.name, &metadata, &checksums],
            )
            .await
            .map_err(|e| {
                storage::StorageError::BackendError(
                    format!("Failed to insert attachment: {}", e).into(),
                )
            })
            .map(|r| r.get(0))
    }

    async fn get(&self, id: &Uuid) -> Result<storage::Function, storage::StorageError> {
        self.get_connection()
            .await?
            .query("select get_function($1)", &[&id])
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
            .map(|row| row.get::<_, AttachmentWithFunctions>(0).into())
    }

    async fn list(
        &self,
        filters: &storage::Filters,
    ) -> Result<Vec<storage::Function>, storage::StorageError> {
        self.get_connection()
            .await?
            .query(
                "select list_functions($1, $2, $3, $4, $5, $6, $7, $8)",
                &[
                    &filters.name,
                    &filters.exact_name_match,
                    &filters.metadata,
                    &(filters.offset as i64),
                    &(filters.limit as i64),
                    &StringAdapter(filters.order_by).to_string(),
                    &filters.order_descending,
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
    use std::{
        panic::{self, AssertUnwindSafe},
        sync::Mutex,
    };

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
            let config = Configuration::new(null_logger!()).await.unwrap();
            let url = Url::parse(&config.functions_storage_uri).unwrap();
            let guard = SQL_MUTEX.lock().unwrap();
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
            let data = storage::FunctionData {
                name: "Super Snek!".to_owned(),
                version: Version::new(1, 2, 3),
                execution_environment: storage::ExecutionEnvironment {
                    name: "avlivningsmiljö".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    function_arguments: HashMap::new(),
                },
                inputs: vec![],
                outputs: vec![],
                metadata: HashMap::new(),
                code: None,
                attachments: vec![],
            };

            let res = storage.insert(data).await;
            assert!(res.is_ok());

            // Insert another function with same name and version (which shouldn't work)
            let same_data = storage::FunctionData {
                name: "Super Snek!".to_owned(),
                version: Version::new(1, 2, 3),
                execution_environment: storage::ExecutionEnvironment {
                    name: "avlivningsmiljö".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    function_arguments: HashMap::new(),
                },
                inputs: vec![],
                outputs: vec![],
                metadata: HashMap::new(),
                code: None,
                attachments: vec![],
            };

            let res = storage.insert(same_data).await;

            assert!(matches!(res.unwrap_err(), storage::StorageError::VersionExists { .. }));

            // Insert same function but newer with different version
            let data = storage::FunctionData {
                name: "Super Snek!".to_owned(),
                version: Version::new(1, 2, 4),
                execution_environment: storage::ExecutionEnvironment {
                    name: "avlivningsmiljö".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    function_arguments: HashMap::new(),
                },
                inputs: vec![],
                outputs: vec![],
                metadata: HashMap::new(),
                code: None,
                attachments: vec![],
            };

            let res = storage.insert(data).await;
            assert!(res.is_ok());

            // Insert different function but with same name as other
            let data = storage::FunctionData {
                name: "Bad Snek!".to_owned(),
                version: Version::new(1, 2, 3),
                execution_environment: storage::ExecutionEnvironment {
                    name: "avlivningsmiljö".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    function_arguments: HashMap::new(),
                },
                inputs: vec![],
                outputs: vec![],
                metadata: HashMap::new(),
                code: None,
                attachments: vec![],
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
            };

            let r = storage.insert_attachment(attachment1).await;
            assert!(r.is_ok());
            let att1_id = r.unwrap();

            let attachment2 = storage::FunctionAttachmentData {
                name: "Attached inferior snek!".to_owned(),
                metadata: hashmap!("mita".to_owned() => "deta".to_owned()),
                checksums: super::super::Checksums {
                    sha256: "6f7c7128c358626cfea2a83173b1626ec18412962969baba819e1ece1b22907e"
                        .to_owned(),
                },
            };

            let r = storage.insert_attachment(attachment2).await;
            assert!(r.is_ok());
            let att2_id = r.unwrap();

            let data = storage::FunctionData {
                name: "Bauta Snek!".to_owned(),
                version: Version::new(99, 3, 4),
                execution_environment: storage::ExecutionEnvironment {
                    name: "avlivningsmiljö".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    function_arguments: hashmap!("arg1".to_owned() => "some/path".to_owned(), "bune".to_owned() => "rune".to_owned()),
                },
                inputs: vec![
                    super::super::FunctionInput {
                        name: "best input".to_owned(),
                        required: true,
                        argument_type: super::super::ArgumentType::String,
                        default_value: "notig".to_owned(),
                        from_execution_environment: true,
                    },
                    super::super::FunctionInput {
                        name: "worst input".to_owned(),
                        required: false,
                        argument_type: super::super::ArgumentType::Bool,
                        default_value: "".to_owned(),
                        from_execution_environment: false,
                    },
                ],
                outputs: vec![
                    super::super::FunctionOutput {
                        name: "best output".to_owned(),
                        argument_type: super::super::ArgumentType::String,
                    },
                    super::super::FunctionOutput {
                        name: "worst output".to_owned(),
                        argument_type: super::super::ArgumentType::Bool,
                    },
                ],
                metadata: hashmap!("meta".to_owned() => "ja tack, fisk är gott".to_owned(), "will_explode".to_owned() => "very yes".to_owned()),
                code: Some(uuid::Uuid::new_v4()),
                attachments: vec![att1_id, att2_id],
            };

            let res = storage.insert(data).await;
            assert!(res.is_ok());
        });

        with_db!(db, {
            let storage = db.unwrap();

            let data = storage::FunctionData {
                name: "Super Snek!".to_owned(),
                version: Version::new(1, 2, 3),
                execution_environment: storage::ExecutionEnvironment {
                    name: "avlivningsmiljö".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    function_arguments: HashMap::new(),
                },
                inputs: vec![],
                outputs: vec![],
                metadata: HashMap::new(),
                code: None,
                attachments: vec![],
            };

            let res = storage.insert(data).await;
            assert!(res.is_ok());
            let ok_function_id = res.unwrap();

            // insert function with non-existent attachment
            // this should test that when inserting
            // into the attachment relation table fails,
            // the function is not inserted either
            // even though that happens before
            let data = storage::FunctionData {
                name: "Snek with no attachment".to_owned(),
                version: Version::new(1, 2, 3),
                execution_environment: storage::ExecutionEnvironment {
                    name: "avlivningsmiljö".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    function_arguments: HashMap::new(),
                },
                inputs: vec![],
                outputs: vec![],
                metadata: HashMap::new(),
                code: None,
                attachments: vec![Uuid::new_v4()],
            };

            let res = storage.insert(data).await;
            assert!(res.is_err());
            assert!(matches!(
                res.unwrap_err(),
                storage::StorageError::BackendError(..)
            ));

            let res = storage.list(&storage::Filters::default()).await;
            assert!(dbg!(&res).is_ok());
            let list = res.unwrap();
            assert_eq!(list.len(), 1);
            assert_eq!(list.first().map(|f| f.id), Some(ok_function_id));
        });
    }

    #[tokio::test]
    async fn get_function() {
        with_db!(db, {
            let storage = db.unwrap();
            let data = storage::FunctionData {
                name: "Super Snek!".to_owned(),
                version: Version::new(1, 2, 3),
                execution_environment: storage::ExecutionEnvironment {
                    name: "avlivningsmiljö".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    function_arguments: HashMap::new(),
                },
                inputs: vec![],
                outputs: vec![],
                metadata: HashMap::new(),
                code: None,
                attachments: vec![],
            };

            let id = storage.insert(data).await.unwrap();
            let res = storage.get(&id).await;

            assert!(res.is_ok());
            assert_eq!(res.unwrap().id, id);

            // test nonexistent
            let nilid = Uuid::nil();
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
            };

            let r = storage.insert_attachment(attachment1).await;
            assert!(r.is_ok());
            let att1_id = r.unwrap();

            let data = storage::FunctionData {
                name: "Super Snek!".to_owned(),
                version: Version::new(1, 2, 3),
                execution_environment: storage::ExecutionEnvironment {
                    name: "avlivningsmiljö".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    function_arguments: HashMap::new(),
                },
                inputs: vec![],
                outputs: vec![],
                metadata: HashMap::new(),
                code: None,
                attachments: vec![att1_id],
            };

            let id = storage.insert(data).await.unwrap();
            let res = storage.get(&id).await;
            assert!(res.is_ok());
            assert!(res.unwrap().function_data.attachments.contains(&att1_id));
        })
    }

    #[tokio::test]
    async fn get_attachment() {
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
            };

            let r = storage.insert_attachment(attachment1).await;
            assert!(r.is_ok());
            let att1_id = r.unwrap();

            let res = storage.get_attachment(&att1_id).await;
            assert!(res.is_ok());
            let returned_attachment = res.unwrap();
            assert_eq!(returned_attachment.id, att1_id);

            // first, check that the function id is not set
            // before the function is registered
            assert!(returned_attachment.function_ids.is_empty());

            // then, register a function with this attachment
            // and make sure that the function id of the
            // attachment is now set to that one
            let data = storage::FunctionData {
                name: "Super Snek!".to_owned(),
                version: Version::new(1, 2, 3),
                execution_environment: storage::ExecutionEnvironment {
                    name: "avlivningsmiljö".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    function_arguments: HashMap::new(),
                },
                inputs: vec![],
                outputs: vec![],
                metadata: HashMap::new(),
                code: None,
                attachments: vec![returned_attachment.id],
            };

            let function_id = storage.insert(data).await.unwrap();
            let returned_attachment = storage.get_attachment(&att1_id).await.unwrap();
            assert_eq!(returned_attachment.function_ids.len(), 1);
            assert_eq!(
                returned_attachment.function_ids.first().unwrap(),
                &function_id
            );

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
            let data = storage::FunctionData {
                name: "Aaa".to_owned(),
                version: Version::new(1, 2, 3),
                execution_environment: storage::ExecutionEnvironment {
                    name: "avlivningsmiljö".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    function_arguments: HashMap::new(),
                },
                inputs: vec![],
                outputs: vec![],
                metadata: string_hashmap!("fisk" => "haj", "orm" => "🐍", "snake" => "snek"),
                code: None,
                attachments: vec![],
            };

            let id = storage.insert(data).await.unwrap();
            let res = storage.list(&storage::Filters::default()).await;
            assert!(res.is_ok());

            let rows = res.unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows.first().map(|f| f.id), Some(id));

            // Test filtering
            let mut filt = storage::Filters::default();
            filt.name = "A".to_owned();
            let res = storage.list(&filt).await;
            assert!(res.is_ok());

            let rows = res.unwrap();
            assert_eq!(rows.len(), 1);

            // Test not finding
            let mut filt = storage::Filters::default();
            filt.name = "B".to_owned();
            let res = storage.list(&filt).await;
            assert!(res.is_ok());

            let rows = res.unwrap();
            assert!(rows.is_empty());

            // Test exact name match
            let mut filt = storage::Filters::default();
            filt.exact_name_match = true;
            filt.name = "a".to_owned();

            let res = storage.list(&filt).await;
            assert!(res.is_ok());

            let rows = res.unwrap();
            assert!(rows.is_empty());

            filt.name = "Aaa".to_owned();
            let res = storage.list(&filt).await;
            assert!(res.is_ok());

            let rows = res.unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows.first().map(|f| f.id), Some(id));

            // Metadata filtering
            let mut filt = storage::Filters::default();
            filt.metadata = hashmap!("fisk".to_owned() => Some("haj".to_owned()), "snake".to_owned() => Some("snek".to_owned()));

            let res = storage.list(&filt).await;
            assert!(res.is_ok());

            let rows = res.unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows.first().map(|f| f.id), Some(id));

            // Existing key and wrong value is not a match
            filt.metadata = hashmap!("fisk".to_owned() => Some("🦈".to_owned()));
            let res = storage.list(&filt).await;
            assert!(res.is_ok());

            let rows = res.unwrap();
            assert!(rows.is_empty());

            // Existing key with other key's value is not a match
            filt.metadata = hashmap!("fisk".to_owned() => Some("🐍".to_owned()));
            let res = storage.list(&filt).await;
            assert!(res.is_ok());

            let rows = res.unwrap();
            assert!(rows.is_empty());

            // Check that filtering on key only work
            filt.metadata = hashmap!("fisk".to_owned() => None);

            let res = storage.list(&filt).await;
            assert!(res.is_ok());

            let rows = res.unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows.first().map(|f| f.id), Some(id));
        })
    }

    #[tokio::test]
    async fn order_offset_and_limit() {
        with_db!(db, {
            let storage = db.unwrap();

            let function1_1_0_0 = storage::FunctionData {
                name: "function1".to_owned(),
                version: Version::new(1, 0, 0),
                execution_environment: storage::ExecutionEnvironment {
                    name: "avlivningsmiljö".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    function_arguments: HashMap::new(),
                },
                inputs: vec![],
                outputs: vec![],
                metadata: string_hashmap!("fisk" => "haj", "orm" => "🐍", "snake" => "snek"),
                code: None,
                attachments: vec![],
            };

            let function1_1_1_1 = storage::FunctionData {
                name: "function1".to_owned(),
                version: Version::new(1, 1, 1),
                execution_environment: storage::ExecutionEnvironment {
                    name: "avlivningsmiljö".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    function_arguments: HashMap::new(),
                },
                inputs: vec![],
                outputs: vec![],
                metadata: string_hashmap!("fisk" => "torsk", "orm" => "🐍", "snake" => "snek"),
                code: None,
                attachments: vec![],
            };

            let function2_1_1_1 = storage::FunctionData {
                name: "function2".to_owned(),
                version: Version::new(1, 1, 1),
                execution_environment: storage::ExecutionEnvironment {
                    name: "avlivningsmiljö".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    function_arguments: HashMap::new(),
                },
                inputs: vec![],
                outputs: vec![],
                metadata: string_hashmap!("fisk" => "abborre", "orm" => "🐍", "snake" => "snek"),
                code: None,
                attachments: vec![],
            };

            let function2_1_0_0 = storage::FunctionData {
                name: "function2".to_owned(),
                version: Version::new(1, 0, 0),
                execution_environment: storage::ExecutionEnvironment {
                    name: "avlivningsmiljö".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    function_arguments: HashMap::new(),
                },
                inputs: vec![],
                outputs: vec![],
                metadata: string_hashmap!("fisk" => "berggylta", "orm" => "nej", "snake" => "snek"),
                code: None,
                attachments: vec![],
            };

            let _function1_1_0_0_id = storage.insert(function1_1_0_0).await.unwrap();
            let _function1_1_1_1_id = storage.insert(function1_1_1_1).await.unwrap();

            let function2_1_1_1_id = storage.insert(function2_1_1_1).await.unwrap();
            let _function2_1_0_0_id = storage.insert(function2_1_0_0).await.unwrap();

            let mut filters = storage::Filters::default();
            filters.offset = 2;
            filters.limit = 2;

            let res = storage.list(&filters).await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(res.len(), 2);
            assert_eq!(
                res.first().map(|f| f.function_data.name.as_ref()),
                Some("function2")
            );
            assert_eq!(res.first().map(|f| f.id), Some(function2_1_1_1_id));
        });

        with_db!(db, {
            let storage = db.unwrap();

            // test version ordering
            let function2_1_10_0 = storage::FunctionData {
                name: "function2".to_owned(),
                version: Version::new(1, 10, 0),
                execution_environment: storage::ExecutionEnvironment {
                    name: "avlivningsmiljö".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    function_arguments: HashMap::new(),
                },
                inputs: vec![],
                outputs: vec![],
                metadata: string_hashmap!("fisk" => "berggylta", "orm" => "nej", "snake" => "snek"),
                code: None,
                attachments: vec![],
            };

            let function2_1_2_0 = storage::FunctionData {
                name: "function2".to_owned(),
                version: Version::new(1, 2, 0),
                execution_environment: storage::ExecutionEnvironment {
                    name: "avlivningsmiljö".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    function_arguments: HashMap::new(),
                },
                inputs: vec![],
                outputs: vec![],
                metadata: string_hashmap!("fisk" => "sutare", "orm" => "nej", "snake" => "snek"),
                code: None,
                attachments: vec![],
            };

            let function2_1_10_0_id = storage.insert(function2_1_10_0).await.unwrap();
            let _function2_1_2_0_id = storage.insert(function2_1_2_0).await.unwrap();

            let res = storage.list(&storage::Filters::default()).await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(res.first().map(|f| f.id), Some(function2_1_10_0_id));

            let function2_1_10_0_alpha = storage::FunctionData {
                name: "function2".to_owned(),
                version: Version::parse("1.10.0-alpha").unwrap(),
                execution_environment: storage::ExecutionEnvironment {
                    name: "avlivningsmiljö".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    function_arguments: HashMap::new(),
                },
                inputs: vec![],
                outputs: vec![],
                metadata: string_hashmap!("fisk" => "sutare", "orm" => "nej", "snake" => "snek"),
                code: None,
                attachments: vec![],
            };

            let function2_1_10_0_jaws = storage::FunctionData {
                name: "function2".to_owned(),
                version: Version::parse("1.10.0-jaws").unwrap(),
                execution_environment: storage::ExecutionEnvironment {
                    name: "avlivningsmiljö".to_owned(),
                    entrypoint: "ingångspoäng".to_owned(),
                    function_arguments: HashMap::new(),
                },
                inputs: vec![],
                outputs: vec![],
                metadata: string_hashmap!("fisk" => "brugd", "orm" => "nej", "snake" => "snek"),
                code: None,
                attachments: vec![],
            };

            let function2_1_10_0_alpha_id = storage.insert(function2_1_10_0_alpha).await.unwrap();
            let _function2_1_10_0_jaws_id = storage.insert(function2_1_10_0_jaws).await.unwrap();

            let mut filters = storage::Filters::default();
            filters.limit = 3;
            let res = storage.list(&filters).await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(res.first().map(|f| f.id), Some(function2_1_10_0_id));
            assert_eq!(res.last().map(|f| f.id), Some(function2_1_10_0_alpha_id));
        });
    }

    #[tokio::test]
    async fn version_filtering() {
        // Version filtering
        with_db!(db, {
            let storage = db.unwrap();

            let function_1_0_0 = storage::FunctionData {
                name: "birb".to_owned(),
                version: Version::new(1, 0, 0),
                execution_environment: storage::ExecutionEnvironment {
                    name: "beepboop".to_owned(),
                    entrypoint: "abandonAllHopeYeWhoEntersHere".to_owned(),
                    function_arguments: HashMap::new(),
                },
                inputs: vec![],
                outputs: vec![],
                metadata: HashMap::new(),
                code: None,
                attachments: vec![],
            };

            let function_1_1_0 = storage::FunctionData {
                name: "birb".to_owned(),
                version: Version::new(1, 1, 0),
                execution_environment: storage::ExecutionEnvironment {
                    name: "beepboop".to_owned(),
                    entrypoint: "abandonAllHopeYeWhoEntersHere".to_owned(),
                    function_arguments: HashMap::new(),
                },
                inputs: vec![],
                outputs: vec![],
                metadata: HashMap::new(),
                code: None,
                attachments: vec![],
            };

            let function_1_1_1 = storage::FunctionData {
                name: "birb".to_owned(),
                version: Version::new(1, 1, 1),
                execution_environment: storage::ExecutionEnvironment {
                    name: "beepboop".to_owned(),
                    entrypoint: "abandonAllHopeYeWhoEntersHere".to_owned(),
                    function_arguments: HashMap::new(),
                },
                inputs: vec![],
                outputs: vec![],
                metadata: HashMap::new(),
                code: None,
                attachments: vec![],
            };

            let function_1_2_0 = storage::FunctionData {
                name: "birb".to_owned(),
                version: Version::new(1, 2, 0),
                execution_environment: storage::ExecutionEnvironment {
                    name: "beepboop".to_owned(),
                    entrypoint: "abandonAllHopeYeWhoEntersHere".to_owned(),
                    function_arguments: HashMap::new(),
                },
                inputs: vec![],
                outputs: vec![],
                metadata: HashMap::new(),
                code: None,
                attachments: vec![],
            };

            let function_1_10_0 = storage::FunctionData {
                name: "birb".to_owned(),
                version: Version::new(1, 10, 0),
                execution_environment: storage::ExecutionEnvironment {
                    name: "beepboop".to_owned(),
                    entrypoint: "abandonAllHopeYeWhoEntersHere".to_owned(),
                    function_arguments: HashMap::new(),
                },
                inputs: vec![],
                outputs: vec![],
                metadata: HashMap::new(),
                code: None,
                attachments: vec![],
            };

            let function_2_2_0 = storage::FunctionData {
                name: "birb".to_owned(),
                version: Version::new(2, 2, 0),
                execution_environment: storage::ExecutionEnvironment {
                    name: "beepboop".to_owned(),
                    entrypoint: "abandonAllHopeYeWhoEntersHere".to_owned(),
                    function_arguments: HashMap::new(),
                },
                inputs: vec![],
                outputs: vec![],
                metadata: HashMap::new(),
                code: None,
                attachments: vec![],
            };

            let function_1_2_0_beta = storage::FunctionData {
                name: "birb".to_owned(),
                version: Version::parse("1.2.0-beta").unwrap(),
                execution_environment: storage::ExecutionEnvironment {
                    name: "beepboop".to_owned(),
                    entrypoint: "abandonAllHopeYeWhoEntersHere".to_owned(),
                    function_arguments: HashMap::new(),
                },
                inputs: vec![],
                outputs: vec![],
                metadata: HashMap::new(),
                code: None,
                attachments: vec![],
            };

            let function_1_0_0_id = storage.insert(function_1_0_0).await.unwrap();
            let _function_1_1_0_id = storage.insert(function_1_1_0).await.unwrap();
            let function_1_1_1_id = storage.insert(function_1_1_1).await.unwrap();
            let function_1_2_0_id = storage.insert(function_1_2_0).await.unwrap();
            let function_1_10_0_id = storage.insert(function_1_10_0).await.unwrap();
            let function_2_2_0_id = storage.insert(function_2_2_0).await.unwrap();
            let function_1_2_0_beta_id = storage.insert(function_1_2_0_beta).await.unwrap();

            // Exact match
            let mut filter = storage::Filters::default();
            filter.version_requirement = Some(VersionReq::parse("=1.0.0").unwrap());
            let res = storage.list(&filter).await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(res.len(), 1);
            assert_eq!(res.first().map(|f| f.id), Some(function_1_0_0_id));

            // Less than on full version
            filter.version_requirement = Some(VersionReq::parse("<1.2.0").unwrap());
            let res = storage.list(&filter).await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(res.len(), 3);
            assert_eq!(res.first().map(|f| f.id), Some(function_1_1_1_id));

            // Less than on minor version
            filter.version_requirement = Some(VersionReq::parse("<1.10.0 >=1.0.0").unwrap());
            let res = storage.list(&filter).await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(res.len(), 4);
            assert_eq!(res.first().map(|f| f.id), Some(function_1_2_0_id));

            // Less than on major version
            filter.version_requirement = Some(VersionReq::parse("<2.0.0").unwrap());
            let res = storage.list(&filter).await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(res.first().map(|f| f.id), Some(function_1_10_0_id));

            // There should not be a pre release here
            assert!(res.iter().all(|f| !f.function_data.version.is_prerelease()));

            // Less or equal
            filter.version_requirement = Some(VersionReq::parse("<=1.1.1").unwrap());
            let res = storage.list(&filter).await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(res.first().map(|f| f.id), Some(function_1_1_1_id));

            // Greater
            filter.version_requirement = Some(VersionReq::parse(">1.1.1").unwrap());
            let res = storage.list(&filter).await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(res.first().map(|f| f.id), Some(function_2_2_0_id));

            // Greater or equal
            filter.version_requirement = Some(VersionReq::parse(">=1.1.1").unwrap());
            let res = storage.list(&filter).await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(res.first().map(|f| f.id), Some(function_2_2_0_id));

            // Pre release only on exact match
            filter.version_requirement = Some(VersionReq::parse("=1.2.0-beta").unwrap());
            let res = storage.list(&filter).await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(res.len(), 1);
            assert_eq!(res.first().map(|f| f.id), Some(function_1_2_0_beta_id));

            // ~
            filter.version_requirement = Some(VersionReq::parse("~1").unwrap());
            let res = storage.list(&filter).await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(res.len(), 5);
            assert_eq!(res.first().map(|f| f.id), Some(function_1_10_0_id));

            filter.version_requirement = Some(VersionReq::parse("~1.1").unwrap());
            let res = storage.list(&filter).await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(res.len(), 2);
            assert_eq!(res.first().map(|f| f.id), Some(function_1_1_1_id));

            filter.version_requirement = Some(VersionReq::parse("~1.1.0").unwrap());
            let res = storage.list(&filter).await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(res.len(), 2);
            assert_eq!(res.first().map(|f| f.id), Some(function_1_1_1_id));

            // ^
            filter.version_requirement = Some(VersionReq::parse("^1.2.3").unwrap());
            let res = storage.list(&filter).await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(res.len(), 1);
            assert_eq!(res.first().map(|f| f.id), Some(function_1_10_0_id));

            // *
            filter.version_requirement = Some(VersionReq::parse("1.*").unwrap());
            let res = storage.list(&filter).await;
            assert!(res.is_ok());
            let res = res.unwrap();
            assert_eq!(res.len(), 5);
        });
    }
}
