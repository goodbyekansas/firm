pub mod token;
pub mod token_authenticator;
pub mod token_source;

pub use token::Token;
pub use token_source::oidc::Provider as OidcProvider;
pub use token_source::self_signed::Provider as SelfSignedProvider;

use std::collections::HashMap;

pub trait CredentialStore {
    fn store(&mut self, key: &str, value: &str);
    fn retrieve(&self, key: &str) -> Option<&str>;
}

pub struct MemCredentialStore {
    credentials: HashMap<String, String>,
}

impl MemCredentialStore {
    pub fn new() -> Self {
        Self {
            credentials: HashMap::new(),
        }
    }
}

impl Default for MemCredentialStore {
    fn default() -> Self {
        Self::new()
    }
}

impl CredentialStore for MemCredentialStore {
    fn store(&mut self, key: &str, value: &str) {
        self.credentials.insert(key.to_owned(), value.to_owned());
    }

    fn retrieve(&self, key: &str) -> Option<&str> {
        self.credentials.get(key).map(|s| s.as_str())
    }
}
