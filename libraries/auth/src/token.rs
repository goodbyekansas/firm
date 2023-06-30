use std::{
    fs::OpenOptions,
    io::BufReader,
    path::PathBuf,
    str::FromStr,
    sync::atomic::{AtomicBool, Ordering},
};

use futures::{StreamExt, TryFutureExt};
use jsonwebtoken::{
    jwk::{Jwk, JwkSet},
    DecodingKey,
};
use reqwest::Url;
use serde::de::DeserializeOwned;
use thiserror::Error;

fn format_validation_errors(v: &[TokenError]) -> String {
    v.iter()
        .map(|te| te.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
pub(crate) fn allow_insecure_jwks() {
    ALLOW_INSECURE_JWKS.store(true, Ordering::Relaxed);
}

static ALLOW_INSECURE_JWKS: AtomicBool = AtomicBool::new(false);

#[derive(Error, Debug)]
pub enum TokenError {
    #[error("Token error: {0}")]
    Unknown(String),

    #[error("Remote error: {0}")]
    Remote(String),

    #[error("Failed to authenticate key store request: {0}")]
    Authentication(String),

    #[error("Failed to parse token headers: {0}")]
    HeaderParse(#[source] jsonwebtoken::errors::Error),

    #[error("Failed to decode JWK: {0}")]
    JwkDecode(#[source] jsonwebtoken::errors::Error),

    #[error("Token validation error: {0}")]
    Validation(#[from] jsonwebtoken::errors::Error),

    #[error("No matching keys for validating token")]
    NoKeys,

    #[error("Token specifies an unauthorized JKU: {0}")]
    UnauthorizedJku(Url),

    #[error("Invalid URL: {0}")]
    UrlParse(#[from] url::ParseError),

    #[error("Failed to validate token signature: {}", format_validation_errors(.0))]
    ValidationErrors(Vec<TokenError>),
}

trait ResultIteratorExt<O, E> {
    fn partition_results<A, B>(self) -> (A, B)
    where
        A: Default + Extend<O>,
        B: Default + Extend<E>;
}

impl<O, E, I> ResultIteratorExt<O, E> for I
where
    I: Iterator<Item = Result<O, E>>,
{
    fn partition_results<A, B>(self) -> (A, B)
    where
        A: Default + Extend<O>,
        B: Default + Extend<E>,
    {
        let mut oks = A::default();
        let mut errs = B::default();

        self.for_each(|res| match res {
            Ok(o) => oks.extend(Some(o)),
            Err(e) => errs.extend(Some(e)),
        });

        (oks, errs)
    }
}

#[derive(Debug, Clone)]
pub struct Token {
    jwt: String,
    header: jsonwebtoken::Header,
}

impl PartialEq for Token {
    fn eq(&self, other: &Self) -> bool {
        self.jwt == other.jwt
    }
}

impl Eq for Token {}

impl std::hash::Hash for Token {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.jwt.hash(state)
    }
}

pub struct ExpectedClaims<'a, I: IntoIterator<Item = &'a str>> {
    pub iss: I,
    pub aud: &'a [&'a str],
    pub sub: Option<&'a str>,
    pub alg: &'a [jsonwebtoken::Algorithm],
}

impl AsRef<str> for Token {
    fn as_ref(&self) -> &str {
        &self.jwt
    }
}

impl Token {
    pub fn try_new<S: AsRef<str>>(token: S) -> Result<Self, TokenError> {
        jsonwebtoken::decode_header(token.as_ref())
            .map_err(TokenError::HeaderParse)
            .map(|header| Self {
                jwt: token.as_ref().to_owned(),
                header,
            })
    }

    pub fn as_bearer(&self) -> String {
        format!("Authorization: Bearer {}", self.as_str())
    }

    fn validate_with_keys<
        'a,
        T: DeserializeOwned,
        I: IntoIterator<Item = Jwk>,
        I2: IntoIterator<Item = &'a str>,
    >(
        &self,
        keys: I,
        expected: ExpectedClaims<'a, I2>,
    ) -> Result<T, TokenError> {
        let issuers = expected.iss.into_iter().collect::<Vec<_>>();
        let res_iter = keys.into_iter().map(|jwk| {
            DecodingKey::from_jwk(&jwk)
                .map_err(TokenError::JwkDecode)
                .and_then(|key| {
                    let mut validation = jsonwebtoken::Validation::default();
                    validation.algorithms = expected.alg.to_vec();
                    validation.validate_exp = true;
                    validation.validate_nbf = true;
                    validation.leeway = 10;

                    if !issuers.is_empty() {
                        validation.set_issuer(&issuers);
                    }

                    validation.set_audience(expected.aud);
                    validation.sub = expected.sub.map(ToOwned::to_owned);

                    jsonwebtoken::decode(&self.jwt, &key, &validation)
                        .map(|td| td.claims)
                        .map_err(Into::into)
                })
        });

        let mut errors = Vec::new();
        for res in res_iter {
            match res {
                Ok(claims) => return Ok(claims),
                Err(e) => errors.push(e),
            }
        }

        if errors.is_empty() {
            errors.push(TokenError::NoKeys)
        }

        Err(TokenError::ValidationErrors(errors))
    }

    pub async fn validate<'a, T: DeserializeOwned, I: IntoIterator<Item = &'a str>>(
        &self,
        key_sources: &[Jwks],
        expected: ExpectedClaims<'a, I>,
    ) -> Result<T, TokenError> {
        match &self.header.jku {
            Some(jku) => {
                // the token specifies a JKU, validate that the JKU exists in key_sources
                // and if it does, only use the matching key source
                futures::future::ready(Url::parse(jku).map_err(Into::into).and_then(|url| {
                    key_sources
                        .iter()
                        .find(|s| s.matches(&url))
                        .ok_or_else(|| TokenError::UnauthorizedJku(url.clone()))
                        .and_then(|_| Jwks::try_new(url))
                }))
                .and_then(|source| async move { source.get(self.header.kid.as_deref()).await })
                .await
                .and_then(|keys| self.validate_with_keys(keys, expected))
            }
            None => {
                // the token does not specify a JKU, use all key_sources
                let keys: Vec<Result<Vec<Jwk>, TokenError>> =
                    futures::stream::iter(key_sources.iter())
                        .then(|source| source.get(self.header.kid.as_deref()))
                        .collect()
                        .await;

                let (keys, fetch_errors): (Vec<_>, Vec<_>) = keys.into_iter().partition_results();
                self.validate_with_keys(keys.into_iter().flatten(), expected)
                    .map_err(|errors| match errors {
                        TokenError::ValidationErrors(mut errors) => {
                            errors.extend(fetch_errors);
                            TokenError::ValidationErrors(errors)
                        }
                        e => e,
                    })
            }
        }
    }

    pub fn as_str(&self) -> &str {
        &self.jwt
    }
}

#[derive(Debug, Clone)]
pub struct Jwks {
    source: JwksSource,
}

#[derive(Debug, Clone)]
enum JwksSource {
    Memory(Vec<Jwk>),
    File(PathBuf),
    Http(Url),
}

impl Jwks {
    pub fn try_new(url: Url) -> Result<Self, TokenError> {
        Ok(Self {
            source: match url.scheme() {
                "file" => Ok(JwksSource::File(PathBuf::from(url.path()))),
                "https" => Ok(JwksSource::Http(url)),
                "http" => {
                    if ALLOW_INSECURE_JWKS.load(Ordering::Relaxed) {
                        Ok(JwksSource::Http(url))
                    } else {
                        Err(TokenError::Unknown(String::from(
                            "JWKS without host verification is not supported",
                        )))
                    }
                }
                "memory" => Ok(JwksSource::Memory(vec![])),
                _ => Err(TokenError::Unknown(format!(
                    "Transport \"{}\", not supported.",
                    url.scheme()
                ))),
            }?,
        })
    }

    pub fn add_key(&mut self, jwk: Jwk) {
        if let JwksSource::Memory(ref mut keys) = self.source {
            keys.push(jwk)
        }
    }

    /// Return true if the authority of the JWKS matches `other`
    ///
    /// The definition of authority depends on protocol.
    pub fn matches(&self, other: &Url) -> bool {
        match &self.source {
            JwksSource::File(path) => &Url::from_file_path(path).unwrap() == other, // TODO
            JwksSource::Http(url) => url == other,
            JwksSource::Memory(_) => true,
        }
    }

    async fn get(&self, key_id: Option<&str>) -> Result<Vec<Jwk>, TokenError> {
        let (haystack, needle) = match &self.source {
            JwksSource::File(path) => OpenOptions::new()
                .read(true)
                .open(path)
                .map_err(|e| {
                    TokenError::Unknown(format!(
                        "Failed to open file \"{}\": {}",
                        path.display(),
                        e
                    ))
                })
                .map(BufReader::new)
                .and_then(|reader| {
                    serde_json::from_reader(reader)
                        .map_err(|e| TokenError::Unknown(format!("Failed to parse jwks: {}", e)))
                        .map(|haystack: JwkSet| (haystack, key_id))
                }),
            JwksSource::Http(url) => reqwest::get(url.clone())
                .map_err(|e| TokenError::Remote(e.to_string()))
                .and_then(|response| {
                    response
                        .json::<JwkSet>()
                        .map_err(|e| TokenError::Unknown(e.to_string()))
                })
                .await
                .map(|haystack| (haystack, key_id)),
            JwksSource::Memory(keys) => Ok((
                JwkSet {
                    keys: keys.to_vec(),
                },
                key_id,
            )),
        }?;

        if needle.is_none() {
            Ok(haystack.keys)
        } else {
            Ok(haystack
                .keys
                .into_iter()
                .filter(|k| k.common.key_id.as_deref() == needle)
                .collect())
        }
    }
}

impl FromStr for Jwks {
    type Err = TokenError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Url::parse(s)
            .map_err(|e| TokenError::Unknown(e.to_string()))
            .and_then(Jwks::try_new)
    }
}
