use std::{
    fmt::Debug,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::Digest;
use thiserror::Error;

use crate::{CredentialStore, Token, TokenProvider};

#[derive(Error, Debug)]
pub enum Error {
    #[error("Failed to read private key: {0}")]
    FailedToReadKey(#[source] jsonwebtoken::errors::Error),

    #[error("Failed to read private key file \"{0}\": {1}")]
    FailedToReadKeyFile(PathBuf, #[source] std::io::Error),

    #[error("Failed to encode token: {0}")]
    FailedToEncodeToken(#[source] jsonwebtoken::errors::Error),

    #[error("Generated invalid token: {0}")]
    InvalidToken(#[source] crate::token::TokenError),
}

#[derive(Clone)]
pub struct Provider {
    private_key: jsonwebtoken::EncodingKey,
    private_key_fingerprint: String,
    claims: Claims,
}

#[derive(Debug, Default, Clone)]
struct Claims {
    iss: Option<String>,
    sub: Option<String>,
    aud: Option<String>,
    exp: Option<usize>,
    jku: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct StandardClaims {
    sub: String,
    exp: u64,
    iss: String,
    aud: String,
    iat: u64,
}

#[async_trait::async_trait]
impl TokenProvider for Provider {
    async fn acquire_token(
        &mut self,
        _credstore: Option<&mut (dyn CredentialStore + Send)>,
    ) -> Result<Token, Box<dyn std::error::Error + Send + Sync + 'static>> {
        self.generate().map_err(Into::into)
    }
}

impl Provider {
    fn generate_fingerprint(key: &[u8]) -> String {
        format!("{:x}", sha2::Sha256::digest(key))[..16].to_string()
    }

    pub fn key_id(&self) -> &str {
        &self.private_key_fingerprint
    }

    pub fn generate(&self) -> Result<Token, Error> {
        let now = chrono::Utc::now().timestamp() as u64;

        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::EdDSA);
        header.kid = Some(self.key_id().to_owned());
        header.jku = self.claims.jku.as_ref().cloned();

        jsonwebtoken::encode(
            &header,
            &StandardClaims {
                sub: self
                    .claims
                    .sub
                    .as_ref()
                    .cloned()
                    .unwrap_or_else(|| String::from("sub")),
                exp: self
                    .claims
                    .exp
                    .as_ref()
                    .map(|exp| now + *exp as u64)
                    .unwrap_or_else(|| now + 3600),
                iss: self
                    .claims
                    .iss
                    .as_ref()
                    .cloned()
                    .unwrap_or_else(|| String::from("issuer")),
                aud: self
                    .claims
                    .aud
                    .as_ref()
                    .cloned()
                    .unwrap_or_else(|| String::from("audience")),
                iat: now,
            },
            &self.private_key,
        )
        .map_err(Error::FailedToEncodeToken)
        .and_then(|t| Token::try_new(t).map_err(Error::InvalidToken))
    }
}

#[derive(Debug)]
pub struct Builder<'a> {
    private_key: PrivateKeySource<'a>,
    claims: Claims,
}

#[derive(Debug)]
enum PrivateKeySource<'a> {
    EddsaBytes(&'a [u8]),
    EddsaFile(&'a Path),
}

impl<'a> Builder<'a> {
    pub fn new_with_ed25519_private_key(key: &'a [u8]) -> Self {
        Self {
            private_key: PrivateKeySource::EddsaBytes(key),
            claims: Default::default(),
        }
    }

    pub fn new_with_ed25519_private_key_from_file(path: &'a Path) -> Self {
        Self {
            private_key: PrivateKeySource::EddsaFile(path),
            claims: Default::default(),
        }
    }

    pub fn with_iss(&mut self, iss: String) -> &mut Self {
        self.claims.iss = Some(iss);
        self
    }

    pub fn with_sub(&mut self, sub: String) -> &mut Self {
        self.claims.sub = Some(sub);
        self
    }

    pub fn with_aud(&mut self, aud: String) -> &mut Self {
        self.claims.aud = Some(aud);
        self
    }

    pub fn with_exp(&mut self, exp: usize) -> &mut Self {
        self.claims.exp = Some(exp);
        self
    }

    pub fn with_jku(&mut self, jku: String) -> &mut Self {
        self.claims.jku = Some(jku);
        self
    }

    pub fn build(self) -> Result<Provider, Error> {
        fn from_file<F>(
            path: &Path,
            decoder: F,
        ) -> Result<(jsonwebtoken::EncodingKey, String), Error>
        where
            F: FnOnce(&[u8]) -> Result<jsonwebtoken::EncodingKey, jsonwebtoken::errors::Error>,
        {
            std::fs::read(path)
                .map_err(|e| Error::FailedToReadKeyFile(path.to_owned(), e))
                .and_then(|bytes| from_bytes(&bytes, decoder))
        }

        fn from_bytes<F>(b: &[u8], decoder: F) -> Result<(jsonwebtoken::EncodingKey, String), Error>
        where
            F: FnOnce(&[u8]) -> Result<jsonwebtoken::EncodingKey, jsonwebtoken::errors::Error>,
        {
            decoder(b)
                .map_err(Error::FailedToReadKey)
                .map(|key| (key, Provider::generate_fingerprint(b)))
        }

        match self.private_key {
            PrivateKeySource::EddsaBytes(bytes) => {
                from_bytes(bytes, jsonwebtoken::EncodingKey::from_ed_pem)
            }
            PrivateKeySource::EddsaFile(path) => {
                from_file(path, jsonwebtoken::EncodingKey::from_ed_pem)
            }
        }
        .map(|(private_key, private_key_fingerprint)| Provider {
            private_key,
            private_key_fingerprint,
            claims: self.claims,
        })
    }
}
