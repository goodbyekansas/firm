use std::collections::hash_map::HashMap;

use gbk_protocols::functions::ArgumentType as ProtoArgType;
use postgres::NoTls;
use postgres_types::{FromSql, ToSql};
use r2d2_postgres::PostgresConnectionManager;
use slog::{info, Logger};

use super::{FunctionAttachmentData, FunctionData, FunctionStorage, StorageError};

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
#[postgres(name = "function_input")]
struct FunctionInput {
    name: String,
    required: bool,
    argument_type: ArgumentType,
    default_value: String,
    from_execution_environment: bool,
}

impl From<super::FunctionInput> for FunctionInput {
    fn from(fi: super::FunctionInput) -> Self {
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

#[derive(Debug, ToSql, FromSql)]
#[postgres(name = "function_output")]
struct FunctionOutput {
    name: String,
    argument_type: ArgumentType,
}

impl From<super::FunctionOutput> for FunctionOutput {
    fn from(fo: super::FunctionOutput) -> Self {
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

impl From<super::ExecutionEnvironment> for ExecutionEnvironment {
    fn from(ee: super::ExecutionEnvironment) -> Self {
        Self {
            name: ee.name,
            entrypoint: ee.entrypoint,
            arguments: HStore(ee.function_arguments).into(),
        }
    }
}

#[derive(Debug, ToSql, FromSql)]
#[postgres(name = "checksums")]
struct Checksums {
    sha256: String,
}

impl From<super::Checksums> for Checksums {
    fn from(chksums: super::Checksums) -> Self {
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
    connection_pool: r2d2::Pool<PostgresConnectionManager<NoTls>>,
    log: slog::Logger,
}

impl PostgresStorage {
    pub fn new(uri: &url::Url, log: Logger) -> Result<Self, StorageError> {
        let config: postgres::Config = uri
            .to_string()
            .parse()
            .map_err(|e| StorageError::ConnectionError(format!("Invalid postgresql url: {}", e)))?;

        info!(
            log,
            "connecting to postgresql database {} at {:#?}",
            config.get_dbname().unwrap_or("<default>"),
            config.get_hosts()
        );
        let manager = PostgresConnectionManager::new(config, NoTls);
        let storage = Self {
            connection_pool: r2d2::Pool::new(manager).map_err(|e| {
                StorageError::ConnectionError(format!("Failed to create postgresql pool: {}", e))
            })?,

            log,
        };

        Ok(storage)
    }

    pub fn new_with_init(uri: &url::Url, log: Logger) -> Result<Self, StorageError> {
        let storage = Self::new(uri, log)?;

        info!(storage.log, "initializing database");
        storage.create_tables()?;

        Ok(storage)
    }

    fn create_tables(&self) -> Result<(), StorageError> {
        let mut client = self.connection_pool.get().map_err(|e| {
            StorageError::ConnectionError(format!(
                "Failed obtain connection to initialize database: {}",
                e
            ))
        })?;

        info!(self.log, "executing sql file sql/create-tables.sql");
        client
            .batch_execute(include_str!("sql/create-tables.sql"))
            .map_err(|e| {
                StorageError::Unknown(format!("Failed to run database initialization: {}", e))
            })
    }

    #[cfg(all(test, feature = "postgres-tests"))]
    fn clear(&self) -> Result<(), StorageError> {
        let mut client = self.connection_pool.get().map_err(|e| {
            StorageError::Unknown(format!("Failed obtain connection to clear database: {}", e))
        })?;
        client
            .batch_execute("select clear_tables();")
            .map_err(|e| StorageError::Unknown(format!("Failed to clear database: {}", e)))
    }
}

impl FunctionStorage for PostgresStorage {
    fn insert(&mut self, function_data: FunctionData) -> Result<uuid::Uuid, StorageError> {
        let mut client = self.connection_pool.get().map_err(|e| {
            StorageError::Unknown(format!("Failed to obtain sql connection from pool: {}", e))
        })?;
        let metadata: HashMap<String, Option<String>> = HStore(function_data.metadata).into();
        let ee: ExecutionEnvironment = function_data.execution_environment.into();

        let name = function_data.name.clone();
        let version = function_data.version.clone();

        client
            .query_one(
                "select insert_function($1, $2, $3, $4, $5, $6, $7, $8)",
                &[
                    &name,
                    &version.to_string(),
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
            .map_err(|e| match e.code() {
                Some(c) => {
                    if c == &postgres::error::SqlState::UNIQUE_VIOLATION {
                        StorageError::VersionExists { name, version }
                    } else {
                        StorageError::Unknown(e.to_string())
                    }
                }
                None => StorageError::Unknown(e.to_string()),
            })
            .map(|r| r.get(0))
    }

    fn insert_attachment(
        &mut self,
        function_attachment_data: FunctionAttachmentData,
    ) -> Result<uuid::Uuid, StorageError> {
        let mut client = self.connection_pool.get().map_err(|e| {
            StorageError::Unknown(format!("Failed to obtain sql connection from pool: {}", e))
        })?;

        let metadata: HashMap<String, Option<String>> =
            HStore(function_attachment_data.metadata).into();
        let checksums = Checksums::from(function_attachment_data.checksums);

        client
            .query_one(
                "select insert_attachment($1, $2, $3)",
                &[&function_attachment_data.name, &metadata, &checksums],
            )
            .map_err(|e| StorageError::Unknown(format!("Failed to insert attachment: {}", e)))
            .map(|r| r.get(0))
    }
}

#[cfg(all(test, feature = "postgres-tests"))]
mod tests {
    use std::collections::HashMap;
    use std::{panic, sync::Mutex};

    use lazy_static::lazy_static;
    use semver::Version;
    use url::Url;

    use super::*;
    use crate::config::Configuration;
    use crate::storage::ExecutionEnvironment;

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
            let config = Configuration::new(null_logger!()).unwrap();
            let url = Url::parse(&config.functions_storage_uri).unwrap();

            {
                let guard = SQL_MUTEX.lock().unwrap();
                if let Err(e) = panic::catch_unwind(|| {
                    let $database = PostgresStorage::new_with_init(&url, null_logger!());
                    if let Ok(ref db) = $database {
                        db.clear().unwrap();
                    }

                    $body
                }) {
                    drop(guard);
                    panic::resume_unwind(e);
                }
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

    #[test]
    fn init_ok() {
        with_db!(db, {
            assert!(db.is_ok());
        });
    }

    #[test]
    fn insert_function() {
        with_db!(db, {
            let mut storage = db.unwrap();
            let data = FunctionData {
                name: "Super Snek!".to_owned(),
                version: Version::new(1, 2, 3),
                execution_environment: ExecutionEnvironment {
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

            let res = storage.insert(data);
            assert!(res.is_ok());

            // Insert another function with same name and version (which shouldn't work)
            let same_data = FunctionData {
                name: "Super Snek!".to_owned(),
                version: Version::new(1, 2, 3),
                execution_environment: ExecutionEnvironment {
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

            let res = storage.insert(same_data);

            assert!(matches!(res.unwrap_err(), StorageError::VersionExists { .. }));

            // Insert same function but newer with different version
            let data = FunctionData {
                name: "Super Snek!".to_owned(),
                version: Version::new(1, 2, 4),
                execution_environment: ExecutionEnvironment {
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

            let res = storage.insert(data);
            assert!(res.is_ok());

            // Insert different function but with same name as other
            let data = FunctionData {
                name: "Bad Snek!".to_owned(),
                version: Version::new(1, 2, 3),
                execution_environment: ExecutionEnvironment {
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

            let res = storage.insert(data);
            assert!(res.is_ok());

            // Test to use all fields
            let attachment1 = FunctionAttachmentData {
                name: "Attached super snek!".to_owned(),
                metadata: hashmap!("meta".to_owned() => "data".to_owned()),
                checksums: super::super::Checksums {
                    sha256: "6f7c7128c358626cfea2a83173b1626ec18412962969baba819e1ece1b22907e"
                        .to_owned(),
                },
            };

            let r = storage.insert_attachment(attachment1);
            assert!(r.is_ok());
            let att1_id = r.unwrap();

            let attachment2 = FunctionAttachmentData {
                name: "Attached inferior snek!".to_owned(),
                metadata: hashmap!("mita".to_owned() => "deta".to_owned()),
                checksums: super::super::Checksums {
                    sha256: "6f7c7128c358626cfea2a83173b1626ec18412962969baba819e1ece1b22907e"
                        .to_owned(),
                },
            };

            let r = storage.insert_attachment(attachment2);
            assert!(r.is_ok());
            let att2_id = r.unwrap();

            let data = FunctionData {
                name: "Bauta Snek!".to_owned(),
                version: Version::new(99, 3, 4),
                execution_environment: ExecutionEnvironment {
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

            let res = storage.insert(data);
            assert!(res.is_ok());
        });
    }

    #[test]
    fn bad_insert() {}
}
