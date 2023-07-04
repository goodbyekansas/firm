use std::collections::HashMap;

use crate::credential_store::CredentialStore;
use crate::credential_store::Error as CredError;

pub struct Memory {
    credentials: HashMap<String, String>,
}

impl Memory {
    pub fn new() -> Self {
        Self {
            credentials: HashMap::new(),
        }
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

impl CredentialStore for Memory {
    fn store(&mut self, key: &str, value: &str) -> Result<(), CredError> {
        self.credentials.insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    fn retrieve(&self, key: &str) -> Result<Option<String>, CredError> {
        Ok(self.credentials.get(key).map(|v| v.to_owned()))
    }
}
