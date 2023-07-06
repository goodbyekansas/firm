use thiserror::Error;

pub mod keyring;
pub mod memory;
pub mod sqlite;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Generic credential store error: {0}")]
    Generic(String),
}

pub trait CredentialStore {
    fn store(&mut self, key: &str, value: &str) -> Result<(), Error>;
    fn retrieve(&self, key: &str) -> Result<Option<String>, Error>;
}
