use std::{
    fmt::Debug,
    path::{Path, PathBuf},
    sync::Arc,
};

use jsonwebtoken::{Algorithm, Header};
use ring::signature::KeyPair;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use simple_asn1::{ASN1Block, ASN1Class, ASN1EncodeErr, ToASN1, OID};
use slog::{info, Logger};
use thiserror::Error;

use super::Token;

pub struct JwtToken {
    token: String,
    audience: String,
    generator: TokenGenerator,
    expires_at: u64,
    claims: StandardClaims,
    subject: String,
}

impl Debug for JwtToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "JWT token: {{audience: {}, expires_at: {}}}",
            self.audience, self.expires_at
        )
    }
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

    async fn refresh(&mut self, logger: &Logger) -> Result<&mut dyn Token, String> {
        if chrono::Utc::now().timestamp() as u64 >= self.expires_at {
            info!(
                logger,
                "Refreshing internal auth token by generating a new one..."
            );
            self.generator
                .generate(&self.subject, &self.audience)
                .map_err(|e| e.to_string())
                .map(move |jwt_token| {
                    self.token = jwt_token.token;
                    self.expires_at = jwt_token.expires_at;
                    self.claims = jwt_token.claims;
                    self as &mut dyn Token
                })
        } else {
            Ok(self)
        }
    }

    fn expires_at(&self) -> u64 {
        self.expires_at
    }

    fn exp(&self) -> Option<u64> {
        Some(self.claims.exp)
    }

    fn iss(&self) -> Option<&str> {
        Some(&self.claims.iss)
    }

    fn iat(&self) -> Option<u64> {
        Some(self.claims.iat)
    }

    fn jti(&self) -> Option<&str> {
        None
    }

    fn nbf(&self) -> Option<u64> {
        None
    }

    fn sub(&self) -> Option<&str> {
        Some(&self.claims.sub)
    }

    fn aud(&self) -> Option<&str> {
        Some(&self.claims.aud)
    }

    fn claim(&self, _key: &str) -> Option<&serde_json::Value> {
        None
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

    #[error("Failed to determine host name: {0}")]
    FailedToDetermineHostName(#[source] std::io::Error),

    #[error("Failed to generate token: {0}")]
    FailedToGenerateToken(#[source] jsonwebtoken::errors::Error),

    #[error("Failed to generate token: {0}")]
    GenericTokenGenerationError(String),
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

    pub fn build(&self) -> Result<TokenGenerator, SelfSignedTokenError> {
        match self.private_key {
            Some(PrivateKeySource::RsaBytes(bytes)) => Ok(TokenGenerator {
                private_key: Arc::new(
                    jsonwebtoken::EncodingKey::from_rsa_pem(bytes)
                        .map_err(SelfSignedTokenError::FailedToReadKey)?,
                ),
                generated_key_pair: None,
                private_key_fingerprint: TokenGenerator::generate_fingerprint(bytes),
            }),
            Some(PrivateKeySource::RsaFile(path)) => std::fs::read(path)
                .map_err(|e| SelfSignedTokenError::FailedToReadKeyFile(path.to_owned(), e))
                .and_then(|bytes| {
                    Ok(TokenGenerator {
                        private_key: Arc::new(
                            jsonwebtoken::EncodingKey::from_rsa_pem(&bytes)
                                .map_err(SelfSignedTokenError::FailedToReadKey)?,
                        ),
                        private_key_fingerprint: TokenGenerator::generate_fingerprint(&bytes),
                        generated_key_pair: None,
                    })
                }),
            Some(PrivateKeySource::EcdsaBytes(bytes)) => Ok(TokenGenerator {
                private_key: Arc::new(
                    jsonwebtoken::EncodingKey::from_ec_pem(bytes)
                        .map_err(SelfSignedTokenError::FailedToReadKey)?,
                ),
                generated_key_pair: None,
                private_key_fingerprint: TokenGenerator::generate_fingerprint(bytes),
            }),
            Some(PrivateKeySource::EcdsaFile(path)) => std::fs::read(path)
                .map_err(|e| SelfSignedTokenError::FailedToReadKeyFile(path.to_owned(), e))
                .and_then(|bytes| {
                    Ok(TokenGenerator {
                        private_key: Arc::new(
                            jsonwebtoken::EncodingKey::from_ec_pem(&bytes)
                                .map_err(SelfSignedTokenError::FailedToReadKey)?,
                        ),
                        generated_key_pair: None,
                        private_key_fingerprint: TokenGenerator::generate_fingerprint(&bytes),
                    })
                }),
            None => ring::signature::EcdsaKeyPair::generate_pkcs8(
                &ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
                &ring::rand::SystemRandom::new(),
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
                private_key: Arc::new(jsonwebtoken::EncodingKey::from_ec_der(private_key.as_ref())),
                generated_key_pair: Some(Arc::new(DerKeyPair {
                    public_key,
                    private_key: private_key.as_ref().to_vec(),
                })),
                private_key_fingerprint: TokenGenerator::generate_fingerprint(private_key.as_ref()),
            }),
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

#[derive(Clone, Debug)]
pub struct TokenGenerator {
    private_key: Arc<jsonwebtoken::EncodingKey>,
    private_key_fingerprint: String,
    generated_key_pair: Option<Arc<DerKeyPair>>,
}

struct DerKeyPair {
    public_key: Vec<u8>,
    private_key: Vec<u8>,
}

struct EcdsaPublicKey<'a> {
    content: &'a [u8],
}

impl ToASN1 for EcdsaPublicKey<'_> {
    type Error = ASN1EncodeErr;
    fn to_asn1_class(&self, _: ASN1Class) -> Result<Vec<ASN1Block>, Self::Error> {
        Ok(vec![ASN1Block::Sequence(
            0,
            vec![
                ASN1Block::Sequence(
                    0,
                    vec![
                        // ecPublicKey (1.2.840.10045.2.1)
                        ASN1Block::ObjectIdentifier(
                            0,
                            OID::new(vec![
                                1u32.into(),
                                2u32.into(),
                                840u32.into(),
                                10045u32.into(),
                                2u32.into(),
                                1u32.into(),
                            ]),
                        ),
                        // prime256v1 (1.2.840.10045.3.1.7)
                        ASN1Block::ObjectIdentifier(
                            0,
                            OID::new(vec![
                                1u32.into(),
                                2u32.into(),
                                840u32.into(),
                                10045u32.into(),
                                3u32.into(),
                                1u32.into(),
                                7u32.into(),
                            ]),
                        ),
                    ],
                ),
                ASN1Block::BitString(0, self.content.len() * 8, self.content.to_vec()),
            ],
        )])
    }
}

impl Debug for DerKeyPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "DER Key Pair")
    }
}

struct PemKeyPair {
    public_key: String,
    private_key: String,
}

impl Debug for PemKeyPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "PEM Key Pair {{ public_key: {} }}", self.public_key)
    }
}

impl From<&DerKeyPair> for PemKeyPair {
    fn from(d: &DerKeyPair) -> Self {
        // Encode the public key in the same
        // format as openssl, which seems to be
        // what jsonwebtoken expects
        let asn1_public_key = simple_asn1::der_encode(
            &(EcdsaPublicKey {
                content: &d.public_key,
            }),
        )
        .unwrap(); // this is fine since the code in ToASN1 cannot fail

        Self {
            private_key: format!(
                "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----",
                base64::encode(&d.private_key)
            ),
            public_key: format!(
                "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----",
                base64::encode(&asn1_public_key)
            ),
        }
    }
}

#[allow(dead_code)]
enum TokenExpiry {
    ExpiresIn(u64),
    ExpiresAt(u64),
}

impl TokenGenerator {
    fn generate_fingerprint(key: &[u8]) -> String {
        format!("{:x}", sha2::Sha256::digest(key))[..16].to_string()
    }

    pub fn key_id(&self, subject: &str) -> String {
        format!("{}:{}", subject, self.private_key_fingerprint)
    }

    pub fn generate(
        &self,
        subject: &str,
        audience: &str,
    ) -> Result<JwtToken, SelfSignedTokenError> {
        let computer_name = hostname::get()
            .map_err(SelfSignedTokenError::FailedToDetermineHostName)?
            .to_string_lossy()
            .to_string();
        self.generate_impl(
            audience.to_owned(),
            format!("Avery@{}", computer_name),
            subject.to_owned(),
            TokenExpiry::ExpiresIn(3600u64),
        )
    }

    fn generate_impl(
        &self,
        audience: String,
        iss: String,
        sub: String,
        expires: TokenExpiry,
    ) -> Result<JwtToken, SelfSignedTokenError> {
        let now = chrono::Utc::now().timestamp() as u64;

        let claims = StandardClaims {
            sub: sub.clone(),
            exp: match expires {
                TokenExpiry::ExpiresIn(ein) => now + ein,
                TokenExpiry::ExpiresAt(eat) => eat,
            },
            iss,
            aud: audience.clone(),
            iat: now,
        };

        let mut header = Header::new(Algorithm::ES256); // TODO: This is not always true
        header.kid = Some(self.key_id(&sub));

        Ok(JwtToken {
            token: jsonwebtoken::encode(&header, &claims, &self.private_key)
                .map_err(SelfSignedTokenError::FailedToGenerateToken)?,
            expires_at: claims.exp,
            audience,
            generator: self.clone(),
            claims,
            subject: sub,
        })
    }

    pub fn save_keys<P1: AsRef<Path>, P2: AsRef<Path>>(
        &self,
        private_keyfile_name: P1,
        public_keyfile_name: P2,
    ) -> std::io::Result<()> {
        if let Some(keypair) = self.generated_key_pair.as_deref() {
            let pem: PemKeyPair = keypair.into();
            let (public_key, private_key) = (pem.public_key, pem.private_key);
            std::fs::write(private_keyfile_name.as_ref(), private_key)
                .and_then(|_| std::fs::write(public_keyfile_name.as_ref(), public_key))
                .and_then(|_| set_permissions(private_keyfile_name.as_ref(), 0o600))
                .and_then(|_| set_permissions(public_keyfile_name.as_ref(), 0o644))
        } else {
            Err(std::io::ErrorKind::InvalidData.into())
        }
    }

    pub fn public_key(&self) -> Option<Vec<u8>> {
        self.generated_key_pair
            .as_ref()
            .map(|kp| PemKeyPair::from(kp.as_ref()).public_key.as_bytes().to_vec())
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashSet;

    use jsonwebtoken::{DecodingKey, EncodingKey, Validation};

    use super::*;

    macro_rules! null_logger {
        () => {{
            slog::Logger::root(slog::Discard, slog::o!())
        }};
    }

    #[test]
    fn builder() {
        // No key provided
        let b = TokenGeneratorBuilder::new().build();
        assert!(b.is_ok());
        assert!(b.unwrap().generated_key_pair.is_some());

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
        assert!(b.unwrap().generated_key_pair.is_none());
    }

    #[test]
    fn generate() {
        let gen = TokenGeneratorBuilder::new().build().unwrap();
        let pem: PemKeyPair = gen.generated_key_pair.as_deref().unwrap().into();

        let res = gen.generate_impl(
            "everyone".to_owned(),
            "tester".to_owned(),
            "sub".to_owned(),
            TokenExpiry::ExpiresIn(3599u64),
        );
        assert!(res.is_ok());
        let token = res.unwrap();
        let mut aud = HashSet::new();
        aud.insert("everyone".to_owned());
        let clams = jsonwebtoken::decode::<StandardClaims>(
            token.token(),
            &DecodingKey::from_ec_der(
                gen.generated_key_pair
                    .as_ref()
                    .unwrap()
                    .public_key
                    .as_slice(),
            ),
            &Validation {
                algorithms: vec![Algorithm::ES256],
                iss: Some("tester".to_owned()),
                sub: Some("sub".to_owned()),
                aud: Some(aud.clone()),
                ..Default::default()
            },
        );
        assert!(clams.is_ok());

        // Test decoding of pem private key
        let pem_content = pem::parse(pem.private_key.as_bytes());
        assert!(pem_content.is_ok(), "PEM private key must be valid PEM");
        let encoding_key = EncodingKey::from_ec_pem(pem.private_key.as_bytes());
        assert!(
            encoding_key.is_ok(),
            "Must be able to parse PEM encoded version of the private key"
        );

        // Test PEM encoded variant
        let pem_content = pem::parse(pem.public_key.as_bytes());
        assert!(pem_content.is_ok(), "PEM public key must be valid PEM");
        let decoding_key = DecodingKey::from_ec_pem(pem.public_key.as_bytes());
        assert!(
            decoding_key.is_ok(),
            "Must be able to parse PEM encoded version of the public key"
        );

        let clams: Result<jsonwebtoken::TokenData<StandardClaims>, jsonwebtoken::errors::Error> =
            jsonwebtoken::decode::<StandardClaims>(
                token.token(),
                &decoding_key.unwrap(),
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
                TokenExpiry::ExpiresAt(0),
            )
            .unwrap();

        let exp = tok.expires_at();
        let old_tok = tok.token().to_owned();
        let tok2 = tok.refresh(&null_logger!()).await;
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
