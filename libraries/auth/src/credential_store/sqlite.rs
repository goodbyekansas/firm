use sqlite::Connection;
use sqlite::State;

use crate::credential_store::CredentialStore;
use crate::credential_store::Error as CredError;

pub struct Sqlite {
    connection: Connection,
}

impl Sqlite {
    pub fn try_new<P: AsRef<std::path::Path>>(path: P) -> Result<Self, sqlite::Error> {
        let init = "
CREATE TABLE IF NOT EXISTS credentials(
key VARCHAR(128) NOT NULL PRIMARY KEY,
value TEXT NOT NULL
);
";
        sqlite::open(path)
            .and_then(|connection| connection.execute(init).map(|_| connection))
            .map(|connection| Self { connection })
    }

    fn get(&self, key: &str) -> Result<Option<String>, sqlite::Error> {
        let query = "SELECT (value) FROM credentials WHERE key = :key";
        self.connection
            .prepare(query)
            .and_then(|mut statement| statement.bind((":key", key)).map(|_| statement))
            .and_then(|mut statement| statement.next().map(|val| (statement, val)))
            .and_then(|(statement, val)| match val {
                State::Row => statement.read::<String, _>("value").map(Some),
                State::Done => Ok(None),
            })
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
        dbg!(&res);
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
        assert!(res.is_ok());
        let res = res.unwrap();
        assert!(res.is_none());
    }
}
