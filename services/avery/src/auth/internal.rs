use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use jsonwebtoken::{Algorithm, Header};
use ring::signature::KeyPair;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::Token;
pub struct JwtToken {
    token: String,
    audience: String,
    generator: TokenGenerator,
    expires_at: u64,
}

#[cfg(unix)]
fn set_permissions(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    path.metadata().and_then(|m| {
        let mut perm = m.permissions();
        perm.set_mode(mode);
        std::fs::set_permissions(path, perm)
    })
}

#[cfg(windows)]
#[allow(clippy::unnecessary_wraps)]
fn set_permissions(_: &Path, _: u32) -> std::io::Result<()> {
    Ok(())
}

#[async_trait::async_trait]
impl Token for JwtToken {
    fn token(&self) -> &str {
        &self.token
    }

    async fn refresh(&mut self) -> Result<&mut dyn Token, String> {
        self.generator
            .generate(&self.audience)
            .map_err(|e| e.to_string())
            .map(move |jwt_token| {
                self.token = jwt_token.token;
                self.expires_at = jwt_token.expires_at;
                self as &mut dyn Token
            })
    }

    fn expires_at(&self) -> u64 {
        self.expires_at
    }
}
#[derive(Error, Debug)]
pub enum SelfSignedTokenError {
    #[error("Failed to read private key: {0}")]
    FailedToReadKey(#[source] jsonwebtoken::errors::Error),

    #[error("Failed to read private key file \"{0}\": {1}")]
    FailedToReadKeyFile(PathBuf, #[source] std::io::Error),

    #[error("Failed to generate key pair: {0}")]
    FailedToGenerateKeyPair(#[source] ring::error::Unspecified),

    #[error("Generated key rejected: {0}")]
    GeneratedKeyRejected(#[source] ring::error::KeyRejected),

    #[error("Failed to determine user")]
    FailedToDetermineUser,

    #[error("Failed to determine host name: {0}")]
    FailedToDetermineHostName(#[source] std::io::Error),

    #[error("Failed to generate token: {0}")]
    FailedToGenerateToken(#[source] jsonwebtoken::errors::Error),
}

enum PrivateKeySource<'a> {
    RsaBytes(&'a [u8]),
    RsaFile(&'a Path),
    EcdsaBytes(&'a [u8]),
    EcdsaFile(&'a Path),
}

pub struct TokenGeneratorBuilder<'a> {
    private_key: Option<PrivateKeySource<'a>>,
}

impl<'a> TokenGeneratorBuilder<'a> {
    pub fn new() -> Self {
        Self { private_key: None }
    }

    #[allow(dead_code)]
    pub fn with_rsa_private_key(&'a mut self, key: &'a [u8]) -> &'a mut Self {
        self.private_key = Some(PrivateKeySource::RsaBytes(key));
        self
    }

    pub fn with_rsa_private_key_from_file(&'a mut self, path: &'a Path) -> &'a mut Self {
        self.private_key = Some(PrivateKeySource::RsaFile(path));
        self
    }

    #[allow(dead_code)]
    pub fn with_ecdsa_private_key(&'a mut self, key: &'a [u8]) -> &'a mut Self {
        self.private_key = Some(PrivateKeySource::EcdsaBytes(key));
        self
    }

    pub fn with_ecdsa_private_key_from_file(&'a mut self, path: &'a Path) -> &'a mut Self {
        self.private_key = Some(PrivateKeySource::EcdsaFile(path));
        self
    }

    pub fn build(&mut self) -> Result<TokenGenerator, SelfSignedTokenError> {
        match self.private_key {
            Some(PrivateKeySource::RsaBytes(bytes)) => Ok(TokenGenerator {
                private_key: Arc::new(
                    jsonwebtoken::EncodingKey::from_rsa_pem(bytes)
                        .map_err(SelfSignedTokenError::FailedToReadKey)?,
                ),
                key_data: None,
            }),
            Some(PrivateKeySource::RsaFile(path)) => std::fs::read(path)
                .map_err(|e| SelfSignedTokenError::FailedToReadKeyFile(path.to_owned(), e))
                .and_then(|bytes| {
                    Ok(TokenGenerator {
                        private_key: Arc::new(
                            jsonwebtoken::EncodingKey::from_rsa_pem(&bytes)
                                .map_err(SelfSignedTokenError::FailedToReadKey)?,
                        ),
                        key_data: None,
                    })
                }),
            Some(PrivateKeySource::EcdsaBytes(bytes)) => Ok(TokenGenerator {
                private_key: Arc::new(
                    jsonwebtoken::EncodingKey::from_ec_pem(bytes)
                        .map_err(SelfSignedTokenError::FailedToReadKey)?,
                ),
                key_data: None,
            }),
            Some(PrivateKeySource::EcdsaFile(path)) => std::fs::read(path)
                .map_err(|e| SelfSignedTokenError::FailedToReadKeyFile(path.to_owned(), e))
                .and_then(|bytes| {
                    Ok(TokenGenerator {
                        private_key: Arc::new(
                            jsonwebtoken::EncodingKey::from_ec_pem(&bytes)
                                .map_err(SelfSignedTokenError::FailedToReadKey)?,
                        ),
                        key_data: None,
                    })
                }),
            None => {
                let rng = ring::rand::SystemRandom::new();
                ring::signature::EcdsaKeyPair::generate_pkcs8(
                    &ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
                    &rng,
                )
                .map_err(SelfSignedTokenError::FailedToGenerateKeyPair)
                .and_then(|keys| {
                    ring::signature::EcdsaKeyPair::from_pkcs8(
                        &ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
                        keys.as_ref(),
                    )
                    .map_err(SelfSignedTokenError::GeneratedKeyRejected)
                    .map(|key_pair| (key_pair.public_key().as_ref().to_vec(), keys))
                })
                .map(|(public_key, private_key)| TokenGenerator {
                    private_key: Arc::new(jsonwebtoken::EncodingKey::from_ec_der(
                        private_key.as_ref(),
                    )),
                    key_data: Some(Arc::new((public_key, private_key.as_ref().to_vec()))),
                })
            }
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct StandardClaims {
    sub: String,
    exp: u64,
    iss: String,
    aud: String,
    iat: u64,
}

#[derive(Clone)]
pub struct TokenGenerator {
    private_key: Arc<jsonwebtoken::EncodingKey>,
    key_data: Option<Arc<(Vec<u8>, Vec<u8>)>>,
}

impl TokenGenerator {
    pub fn generate(&self, audience: &str) -> Result<JwtToken, SelfSignedTokenError> {
        let computer_name = hostname::get()
            .map_err(SelfSignedTokenError::FailedToDetermineHostName)?
            .to_string_lossy()
            .to_string();
        self.generate_impl(
            audience.to_owned(),
            format!("Avery@{}", computer_name),
            format!(
                "{}@{}",
                crate::system::user().ok_or(SelfSignedTokenError::FailedToDetermineUser)?,
                computer_name,
            ),
            3600u64,
        )
    }

    fn generate_impl(
        &self,
        audience: String,
        iss: String,
        sub: String,
        expires_in: u64,
    ) -> Result<JwtToken, SelfSignedTokenError> {
        let now = chrono::Utc::now().timestamp() as u64;
        Ok(JwtToken {
            token: jsonwebtoken::encode(
                &Header::new(Algorithm::ES256),
                &StandardClaims {
                    sub,
                    exp: now + expires_in,
                    iss,
                    aud: audience.clone(),
                    iat: now,
                },
                &self.private_key,
            )
            .map_err(SelfSignedTokenError::FailedToGenerateToken)?,
            expires_at: now + expires_in,
            audience,
            generator: self.clone(),
        })
    }

    pub fn save_keys<P1: AsRef<Path>, P2: AsRef<Path>>(
        &self,
        private_keyfile_name: P1,
        public_keyfile_name: P2,
    ) -> std::io::Result<()> {
        if let Some((public_key, private_key)) = self.key_data.as_deref() {
            std::fs::write(
                private_keyfile_name.as_ref(),
                format!(
                    "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----",
                    base64::encode(private_key)
                ),
            )
            .and_then(|_| {
                std::fs::write(
                    public_keyfile_name.as_ref(),
                    format!(
                        "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----",
                        base64::encode(public_key)
                    ),
                )
            })
            .and_then(|_| set_permissions(private_keyfile_name.as_ref(), 0o600))
            .and_then(|_| set_permissions(public_keyfile_name.as_ref(), 0o644))
        } else {
            Err(std::io::ErrorKind::InvalidData.into())
        }
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashSet;

    use jsonwebtoken::{DecodingKey, Validation};

    use super::*;

    #[test]
    fn builder() {
        // No key provided
        let b = TokenGeneratorBuilder::new().build();
        assert!(b.is_ok());
        assert!(b.unwrap().key_data.is_some());

        // From bytes
        let b = TokenGeneratorBuilder::new()
            .with_rsa_private_key(
                br#"-----BEGIN RSA PRIVATE KEY-----
MIIEogIBAAKCAQEAnzyis1ZjfNB0bBgKFMSvvkTtwlvBsaJq7S5wA+kzeVOVpVWw
kWdVha4s38XM/pa/yr47av7+z3VTmvDRyAHcaT92whREFpLv9cj5lTeJSibyr/Mr
m/YtjCZVWgaOYIhwrXwKLqPr/11inWsAkfIytvHWTxZYEcXLgAXFuUuaS3uF9gEi
NQwzGTU1v0FqkqTBr4B8nW3HCN47XUu0t8Y0e+lf4s4OxQawWD79J9/5d3Ry0vbV
3Am1FtGJiJvOwRsIfVChDpYStTcHTCMqtvWbV6L11BWkpzGXSW4Hv43qa+GSYOD2
QU68Mb59oSk2OB+BtOLpJofmbGEGgvmwyCI9MwIDAQABAoIBACiARq2wkltjtcjs
kFvZ7w1JAORHbEufEO1Eu27zOIlqbgyAcAl7q+/1bip4Z/x1IVES84/yTaM8p0go
amMhvgry/mS8vNi1BN2SAZEnb/7xSxbflb70bX9RHLJqKnp5GZe2jexw+wyXlwaM
+bclUCrh9e1ltH7IvUrRrQnFJfh+is1fRon9Co9Li0GwoN0x0byrrngU8Ak3Y6D9
D8GjQA4Elm94ST3izJv8iCOLSDBmzsPsXfcCUZfmTfZ5DbUDMbMxRnSo3nQeoKGC
0Lj9FkWcfmLcpGlSXTO+Ww1L7EGq+PT3NtRae1FZPwjddQ1/4V905kyQFLamAA5Y
lSpE2wkCgYEAy1OPLQcZt4NQnQzPz2SBJqQN2P5u3vXl+zNVKP8w4eBv0vWuJJF+
hkGNnSxXQrTkvDOIUddSKOzHHgSg4nY6K02ecyT0PPm/UZvtRpWrnBjcEVtHEJNp
bU9pLD5iZ0J9sbzPU/LxPmuAP2Bs8JmTn6aFRspFrP7W0s1Nmk2jsm0CgYEAyH0X
+jpoqxj4efZfkUrg5GbSEhf+dZglf0tTOA5bVg8IYwtmNk/pniLG/zI7c+GlTc9B
BwfMr59EzBq/eFMI7+LgXaVUsM/sS4Ry+yeK6SJx/otIMWtDfqxsLD8CPMCRvecC
2Pip4uSgrl0MOebl9XKp57GoaUWRWRHqwV4Y6h8CgYAZhI4mh4qZtnhKjY4TKDjx
QYufXSdLAi9v3FxmvchDwOgn4L+PRVdMwDNms2bsL0m5uPn104EzM6w1vzz1zwKz
5pTpPI0OjgWN13Tq8+PKvm/4Ga2MjgOgPWQkslulO/oMcXbPwWC3hcRdr9tcQtn9
Imf9n2spL/6EDFId+Hp/7QKBgAqlWdiXsWckdE1Fn91/NGHsc8syKvjjk1onDcw0
NvVi5vcba9oGdElJX3e9mxqUKMrw7msJJv1MX8LWyMQC5L6YNYHDfbPF1q5L4i8j
8mRex97UVokJQRRA452V2vCO6S5ETgpnad36de3MUxHgCOX3qL382Qx9/THVmbma
3YfRAoGAUxL/Eu5yvMK8SAt/dJK6FedngcM3JEFNplmtLYVLWhkIlNRGDwkg3I5K
y18Ae9n7dHVueyslrb6weq7dTkYDi3iOYRW8HRkIQh06wEdbxt0shTzAJvvCQfrB
jg/3747WSsf/zBTcHihTRBdAv6OmdhV4/dD5YBfLAkLrd+mX7iE=
-----END RSA PRIVATE KEY-----"#,
            )
            .build();

        assert!(b.is_ok());
        assert!(b.unwrap().key_data.is_none());
    }

    #[test]
    fn generate() {
        let gen = TokenGeneratorBuilder::new().build().unwrap();
        let res = gen.generate_impl(
            "everyone".to_owned(),
            "tester".to_owned(),
            "sub".to_owned(),
            3599u64,
        );
        assert!(res.is_ok());
        let token = res.unwrap();
        let mut aud = HashSet::new();
        aud.insert("everyone".to_owned());
        let clams = jsonwebtoken::decode::<StandardClaims>(
            token.token(),
            &DecodingKey::from_ec_der(gen.key_data.unwrap().0.as_slice()),
            &Validation {
                algorithms: vec![Algorithm::ES256],
                iss: Some("tester".to_owned()),
                sub: Some("sub".to_owned()),
                aud: Some(aud),
                ..Default::default()
            },
        );
        assert!(clams.is_ok());
    }

    #[tokio::test]
    async fn refresh() {
        let gen = TokenGeneratorBuilder::new().build().unwrap();
        let mut tok = gen
            .generate_impl(
                "audience".to_owned(),
                "iss".to_owned(),
                "sub".to_owned(),
                1u64,
            )
            .unwrap();
        std::thread::sleep(std::time::Duration::from_secs(2));
        let exp = tok.expires_at();
        let old_tok = tok.token().to_owned();
        let tok2 = tok.refresh().await;
        assert!(tok2.is_ok());
        let tok2 = tok2.unwrap();
        assert!(
            tok2.expires_at() > exp,
            "refreshed token should have a higher expires_at then original"
        );
        assert_ne!(tok2.token(), old_tok, "refresh should give a unique token");
        assert_eq!(
            tok2.expires_at(),
            tok.expires_at(),
            "refresh() mutates the original token so it should match the returned token"
        );
    }
}
