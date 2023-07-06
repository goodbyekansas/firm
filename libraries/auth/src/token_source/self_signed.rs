use std::{
    fmt::Debug,
    path::{Path, PathBuf},
};

use serde::Serialize;
use sha2::Digest;
use thiserror::Error;

use crate::Token;

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
    claims: Header,
}

#[derive(Debug, Clone)]
struct Header {
    jku: Option<String>,
    kid: bool,
}

impl Default for Header {
    fn default() -> Self {
        Self {
            jku: Default::default(),
            kid: true,
        }
    }
}

#[derive(Serialize, Debug, Clone, Default)]
pub struct StandardClaims {
    pub iss: Option<String>,
    pub sub: Option<String>,
    pub aud: Option<String>,
    pub exp: u64,
    pub iat: Option<u64>,
    pub nbf: Option<u64>,
    pub jti: Option<String>,
}

impl Provider {
    fn generate_fingerprint(key: &[u8]) -> String {
        format!("{:x}", sha2::Sha256::digest(key))[..16].to_string()
    }

    pub fn key_id(&self) -> &str {
        &self.private_key_fingerprint
    }

    pub fn generate<C: Serialize>(&self, claims: C) -> Result<Token, Error> {
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::EdDSA);
        header.kid = self.claims.kid.then(|| self.key_id().to_owned());
        header.jku = self.claims.jku.as_ref().cloned();

        jsonwebtoken::encode(&header, &claims, &self.private_key)
            .map_err(Error::FailedToEncodeToken)
            .and_then(|t| Token::try_new(t).map_err(Error::InvalidToken))
    }
}

#[derive(Debug)]
pub struct Builder<'a> {
    private_key: PrivateKeySource<'a>,
    header: Header,
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
            header: Default::default(),
        }
    }

    pub fn new_with_ed25519_private_key_from_file(path: &'a Path) -> Self {
        Self {
            private_key: PrivateKeySource::EddsaFile(path),
            header: Default::default(),
        }
    }

    pub fn with_kid(mut self, kid: bool) -> Self {
        self.header.kid = kid;
        self
    }

    pub fn with_jku<S: AsRef<str>>(mut self, jku: S) -> Self {
        self.header.jku = Some(jku.as_ref().to_owned());
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
            claims: self.header,
        })
    }
}

#[cfg(test)]
mod tests {

    use jsonwebtoken::jwk::{AlgorithmParameters, Jwk, OctetKeyPairParameters};
    use serde::Deserialize;

    use crate::token::Jwks;

    use super::*;

    static PRIVATE_KEY: &str = "-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIDjobHKy/8ilexeTOjo5if01J1l1vlNfc96WvzpgGddp
-----END PRIVATE KEY-----";

    macro_rules! jwks {
        () => {{
            let mut key_source = "memory:///".parse::<Jwks>().unwrap();
            key_source.add_key(Jwk {
                common: Default::default(),
                algorithm: AlgorithmParameters::OctetKeyPair(OctetKeyPairParameters {
                    key_type: jsonwebtoken::jwk::OctetKeyPairType::OctetKeyPair,
                    curve: jsonwebtoken::jwk::EllipticCurve::Ed25519,
                    x: String::from("Zbfwp1WfVrpr9aRdUwHD2aWZyAYc9ElOkOqq1MZzoyo"),
                }),
            });
            key_source
        }};
        ($kid:expr) => {{
            let mut key_source = "memory:///".parse::<Jwks>().unwrap();
            key_source.add_key(Jwk {
                common: jsonwebtoken::jwk::CommonParameters {
                    key_id: Some(String::from($kid)),
                    ..Default::default()
                },
                algorithm: AlgorithmParameters::OctetKeyPair(OctetKeyPairParameters {
                    key_type: jsonwebtoken::jwk::OctetKeyPairType::OctetKeyPair,
                    curve: jsonwebtoken::jwk::EllipticCurve::Ed25519,
                    x: String::from("Zbfwp1WfVrpr9aRdUwHD2aWZyAYc9ElOkOqq1MZzoyo"),
                }),
            });
            key_source
        }};
    }

    #[tokio::test]
    async fn test_generation() {
        let res = Builder::new_with_ed25519_private_key(PRIVATE_KEY.as_bytes())
            .with_kid(false)
            .build();
        assert!(
            res.is_ok(),
            "Expected to be able to build generator from valid ed25519 private key"
        );
        let generator = res.unwrap();
        assert_eq!(
            generator.key_id(),
            "e53f51e41ececd41",
            "Expected key id to be what it should"
        );

        let res = generator.generate(StandardClaims {
            aud: Some(String::from("publikum")),
            exp: u64::MAX,
            ..Default::default()
        });
        assert!(
            res.is_ok(),
            "Expected to be able to generate token with default claims"
        );

        let key_sources = [jwks!()];

        #[derive(Debug, Deserialize)]
        struct Claims {
            aud: String,
        }
        let token = res.unwrap();
        let res = token
            .validate::<Claims, _>(
                &key_sources,
                crate::token::ExpectedClaims {
                    iss: None,
                    aud: &["publikum"],
                    sub: None,
                    alg: &[jsonwebtoken::Algorithm::EdDSA],
                },
            )
            .await;

        assert!(res.is_ok(), "Expected generator to produce valid token");
        let claims = res.unwrap();
        assert_eq!(
            claims.aud, "publikum",
            "Expected generator to use requested claim"
        );
    }

    #[tokio::test]
    async fn test_generation_with_kid() {
        let res = Builder::new_with_ed25519_private_key(PRIVATE_KEY.as_bytes())
            .with_kid(true)
            .build();
        assert!(
            res.is_ok(),
            "Expected to be able to build generator from valid ed25519 private key"
        );
        let generator = res.unwrap();
        assert_eq!(
            generator.key_id(),
            "e53f51e41ececd41",
            "Expected key id to be what it should"
        );

        let res = generator.generate(StandardClaims {
            aud: Some(String::from("publikum")),
            exp: u64::MAX,
            ..Default::default()
        });
        assert!(
            res.is_ok(),
            "Expected to be able to generate token with default claims"
        );

        let key_sources = [jwks!(generator.key_id())];

        #[derive(Debug, Deserialize)]
        struct Claims {
            aud: String,
        }
        let token = res.unwrap();
        let res = token
            .validate::<Claims, _>(
                &key_sources,
                crate::token::ExpectedClaims {
                    iss: None,
                    aud: &["publikum"],
                    sub: None,
                    alg: &[jsonwebtoken::Algorithm::EdDSA],
                },
            )
            .await;

        assert!(
            dbg!(&res).is_ok(),
            "Expected generator to produce valid token"
        );
        let claims = res.unwrap();
        assert_eq!(
            claims.aud, "publikum",
            "Expected generator to use requested claim"
        );
    }
}
