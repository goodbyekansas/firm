use sqlite::Connection;

use crate::credential_store::CredentialStore;
use crate::credential_store::Error as CredError;

pub struct Sqlite {
    connection: Connection,
}

impl Sqlite {
    pub fn try_new<P: AsRef<std::path::Path>>(path: P) -> Result<Self, sqlite::Error> {
        let init = "
CREATE TABLE IF NOT EXISTS credentials(
key TEXT NOT NULL PRIMARY KEY,
value TEXT NOT NULL
);
";
        sqlite::open(path)
            .and_then(|connection| connection.execute(init).map(|_| connection))
            .map(|connection| Self { connection })
    }

    fn get(&self, key: &str) -> Result<Option<String>, sqlite::Error> {
        let query = format!(
            "
SELECT (value) FROM credentials WHERE key = '{}'
",
            key
        );
        let mut val = None;
        self.connection
            .iterate(query, |pairs| {
                for &value in pairs.iter() {
                    val = value.1.map(String::from);
                }
                true
            })
            .map(|_| val)
    }

    fn set(&self, key: &str, value: &str) -> Result<(), sqlite::Error> {
        let query = format!(
            "
INSERT OR REPLACE INTO credentials(key, value)
VALUES('{}', '{}')
",
            key, value
        );
        self.connection.execute(query)
    }
}

impl CredentialStore for Sqlite {
    fn store(&mut self, key: &str, value: &str) -> Result<(), CredError> {
        self.set(key, value)
            .map_err(|e| CredError::Generic(e.to_string()))
    }

    fn retrieve(&self, key: &str) -> Result<Option<String>, CredError> {
        self.get(key).map_err(|e| CredError::Generic(e.to_string()))
    }
}

#[cfg(test)]
mod test {
    use super::Sqlite;

    #[test]
    fn set_get() {
        let db = Sqlite::try_new(":memory:").expect("Expected to create in memory database");

        // Ensuring it doesn't error on inserts
        let res = db.set("sune", "rune");
        assert!(res.is_ok());
        let res = db.get("sune");
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), Some(String::from("rune")));

        // Ensuring inserting in existing works and value is updated.
        let res = db.set("sune", "bune");
        assert!(res.is_ok());
        let res = db.get("sune");
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), Some(String::from("bune")));

        // Getting value that does not existing
        let res = db.get("ja");
        assert!(res.is_err());
    }
}
