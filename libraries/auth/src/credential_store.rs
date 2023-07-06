use thiserror::Error;

// TODO: keyring 0.9.0 seems to be the newest compatible with rust 1.60.0
//pub mod keyring;
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
