use std::{
    collections::HashMap,
    collections::HashSet,
    convert::Infallible,
    net::SocketAddr,
    net::{Ipv6Addr, SocketAddrV6},
    sync::Arc,
};

use chrono::{TimeZone, Utc};
use futures::TryFutureExt;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use rand::{seq::SliceRandom, Rng};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use slog::{info, warn, Logger};
use thiserror::Error;
use tokio::sync::oneshot::Sender;
use warp::Filter;

use crate::{auth::Token, system};

#[derive(Error, Debug)]
pub enum OidcError {
    #[error("Transport error: {0}")]
    TransportError(#[source] reqwest::Error),

    #[error("HTTP error: {0} {1}")]
    HttpError(#[source] reqwest::Error, String),

    #[error("JSON error: {0}")]
    JsonError(#[source] reqwest::Error),

    #[error("Failed to open browser for user consent: {0}")]
    FailedToOpenBrowser(#[source] std::io::Error),

    #[error("Failed to read OAuth callback result: {0}")]
    FailedToReadCallbackResult(#[source] tokio::sync::oneshot::error::RecvError),

    #[error("Failed to handle OAuth callback result: {0}")]
    FailedToHandleCallbackResult(String),

    #[error("OAuth state mismatch")]
    StateMismatch,

    #[error("OAuth authentication error: {0}")]
    AuthError(String),

    #[error("Failed to validate JWT claims")]
    FailedToValidateClaims,

    #[error("Failed to decode JWT header: {0}")]
    FailedToDecodeJwtHeader(#[source] jsonwebtoken::errors::Error),
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct AuthToken {
    access_token: String,
    expires_in: u64,
    id_token: String,
    refresh_token: String,
    scope: String,
    token_type: String,
}

impl AuthToken {
    fn refresh(&self, rt: RefreshedToken) -> Self {
        Self {
            access_token: rt.access_token,
            expires_in: rt.expires_in,
            id_token: rt.id_token,
            refresh_token: self.refresh_token.clone(),
            scope: rt.scope,
            token_type: rt.token_type,
        }
    }
}

#[derive(Deserialize, Debug, PartialEq)]
struct RefreshedToken {
    access_token: String,
    expires_in: u64,
    id_token: String,
    scope: String,
    token_type: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct AuthContext {
    expires_at: u64,
    client_id: String,
    client_secret: String,
    token_endpoint: String,
    jwks_uri: String,
    hosted_domain: Option<String>,
    id_token_signing_alg_values_supported: Vec<Algorithm>,
    claims: Claims,
}

impl AuthContext {
    fn calculate_expires_at(expires_at: u64, issued_at: u64, percent_margin: u64) -> u64 {
        issued_at
            + ((expires_at - issued_at) as f64 * (1f64 - (percent_margin as f64 * 0.01))) as u64
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OidcToken {
    auth_token: AuthToken,
    context: AuthContext,
}

impl OidcToken {
    fn new(auth_token: AuthToken, context: AuthContext) -> Self {
        Self {
            auth_token,
            context,
        }
    }
}

#[async_trait::async_trait]
impl Token for OidcToken {
    fn token(&self) -> &str {
        &self.auth_token.id_token
    }

    fn expires_at(&self) -> u64 {
        self.context.expires_at
    }

    async fn refresh(&mut self, logger: &Logger) -> Result<&mut dyn Token, String> {
        if chrono::Utc::now().timestamp() as u64 >= self.context.expires_at {
            info!(logger, "Refreshing auth token");

            let client = reqwest::Client::new();
            let params = [
                ("client_id", self.context.client_id.as_str()),
                ("client_secret", self.context.client_secret.as_str()),
                ("refresh_token", self.auth_token.refresh_token.as_str()),
                ("grant_type", "refresh_token"),
            ];

            return client
                .post(&self.context.token_endpoint)
                .form(&params)
                .send()
                .map_err(OidcError::TransportError)
                .and_then(|response| async move {
                    match response.error_for_status_ref() {
                        Err(e) => {
                            let err = OidcError::HttpError(
                                e,
                                response.text().await.unwrap_or_else(|e| {
                                    format!("Failed to get body of error response: {}", e)
                                }),
                            );
                            warn!(logger, "Failed to refresh token: {}", err);
                            Err(err)
                        }
                        Ok(_) => response
                            .json::<RefreshedToken>()
                            .map_err(OidcError::JsonError)
                            .and_then(|refreshed_token| async {
                                Oidc::validate_claims(
                                    &self.context.client_id,
                                    &self.context.hosted_domain,
                                    &self.context.jwks_uri,
                                    &refreshed_token.id_token,
                                    &self.context.id_token_signing_alg_values_supported,
                                )
                                .await
                                .map(|c| {
                                    (
                                        refreshed_token,
                                        AuthContext::calculate_expires_at(c.exp, c.iat, 10),
                                    )
                                })
                            })
                            .await
                            .map(move |(refreshed_token, expires_at)| {
                                info!(
                                    logger,
                                    "Token successfully refreshed, expires at: {}",
                                    Utc.timestamp(expires_at as i64, 0)
                                );

                                self.auth_token = self.auth_token.refresh(refreshed_token);
                                self.context.expires_at = expires_at;
                                self as &mut dyn crate::auth::Token
                            }),
                    }
                })
                .await
                .map_err(|e| e.to_string());
        }

        Ok(self)
    }

    fn exp(&self) -> Option<u64> {
        Some(self.context.claims.exp)
    }

    fn iss(&self) -> Option<&str> {
        Some(&self.context.claims.iss)
    }

    fn iat(&self) -> Option<u64> {
        Some(self.context.claims.iat)
    }

    fn jti(&self) -> Option<&str> {
        None
    }

    fn nbf(&self) -> Option<u64> {
        None
    }

    fn sub(&self) -> Option<&str> {
        Some(&self.context.claims.sub)
    }

    fn aud(&self) -> Option<&str> {
        Some(&self.context.claims.aud)
    }

    fn claim(&self, key: &str) -> Option<&serde_json::Value> {
        self.context.claims.extra.get(key)
    }
}

#[derive(Deserialize, Debug, Serialize, PartialEq)]
struct OidcConfig {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub device_authorization_endpoint: Option<String>,
    pub token_endpoint: String,
    pub userinfo_endpoint: Option<String>,
    pub revocation_endpoint: Option<String>,
    pub jwks_uri: String,
    pub response_types_supported: Vec<String>,
    pub subject_types_supported: Vec<String>,
    pub id_token_signing_alg_values_supported: Vec<Algorithm>,
    pub scopes_supported: Option<Vec<String>>,
    pub token_endpoint_auth_methods_supported: Option<Vec<String>>,
    pub claims_supported: Option<Vec<String>>,
    pub code_challenge_methods_supported: Option<Vec<String>>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
enum AuthResponseResult {
    Error(String),
    Code(String),
}

#[derive(Deserialize, Debug)]
struct AuthResponse {
    state: String,
    #[serde(flatten)]
    result: AuthResponseResult,
}

#[derive(Deserialize, Debug)]
struct JsonWebKeyset {
    keys: Vec<HashMap<String, String>>,
}

#[derive(Deserialize, Serialize, Debug, PartialEq)]
struct Claims {
    iss: String,
    sub: String,
    aud: String,
    exp: u64,
    iat: u64,

    #[serde(flatten)]
    extra: HashMap<String, serde_json::Value>,
}

pub struct Oidc {
    oidc_config: crate::config::OidcProvider,
    logger: Logger,
}

impl Oidc {
    pub fn new(oidc_config: &crate::config::OidcProvider, logger: Logger) -> Self {
        Self {
            oidc_config: oidc_config.clone(),
            logger,
        }
    }

    fn create_challenge<R: Rng>(mut rng: R) -> (String, String) {
        const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ\
                               abcdefghijklmnopqrstuvwxyz\
                               0123456789-.~_";
        const SIZE: usize = 128;

        let mut code_verifier = Vec::with_capacity(SIZE);
        let _ = (0..SIZE)
            .map(|_| code_verifier.push(*CHARS.choose(&mut rng).unwrap()))
            .collect::<()>();
        let code_challenge = base64::encode_config(
            Sha256::digest(code_verifier.as_slice()),
            base64::URL_SAFE_NO_PAD,
        );
        (
            unsafe { String::from_utf8_unchecked(code_verifier) },
            code_challenge,
        )
    }

    fn create_local_listener(
        sender: Sender<Result<AuthResponse, String>>,
    ) -> (SocketAddr, impl std::future::Future<Output = ()> + 'static) {
        let sender = Arc::new(std::sync::Mutex::new(Some(sender)));
        let sender2 = Arc::clone(&sender);
        warp::serve(
            warp::filters::any::any()
                .and(warp::filters::query::query::<AuthResponse>())
                .map(move |auth_response| {
                    if let Some(snd) = Arc::clone(&sender2)
                        .lock()
                        .ok()
                        .and_then(|mut mutex_guard| mutex_guard.take())
                    {
                        // Just ignore possible errors. If we can't notify whoever is reading
                        // what's the point of trying to notify then?
                        let _ = snd.send(Ok(auth_response));
                    }
                    warp::reply::html(include_str!("logged_in.html"))
                })
                .recover(move |_| {
                    if let Some(snd) = Arc::clone(&sender)
                        .lock()
                        .ok()
                        .and_then(|mut mutex_guard| mutex_guard.take())
                    {
                        // Just ignore possible errors. If we can't notify whoever is reading
                        // what's the point of trying to notify then?
                        let _ = snd.send(Err("Failed to parse query string.".to_owned()));
                    }

                    futures::future::ready(Ok::<_, Infallible>(warp::reply::with_status(
                        "Failed to parse query string",
                        StatusCode::BAD_REQUEST,
                    )))
                }),
        )
        .bind_with_graceful_shutdown(
            SocketAddrV6::new(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1), 0, 0, 0),
            system::shutdown_signal(slog::Logger::root(slog::Discard, slog::o!())),
        )
    }

    fn build_authorize_url(
        &self,
        endpoint: &str,
        code_challenge: &str,
        port: u16,
    ) -> Result<(String, String, String), OidcError> {
        let state_string = format!(
            "security_token={}",
            &Self::create_challenge(rand::thread_rng()).0
        );
        let redirect_uri = format!("http://[::1]:{}", port);
        Ok((
            reqwest::Client::new()
                .get(endpoint)
                .query(&[
                    ("client_id", self.oidc_config.client_id.as_str()),
                    ("redirect_uri", redirect_uri.as_str()),
                    ("response_type", "code"),
                    ("scope", "openid email"),
                    ("code_challenge", code_challenge),
                    ("code_challenge_method", "S256"),
                    ("state", &state_string),
                    ("access_type", "offline"),
                ])
                .query(&if let Some(hd) = self.oidc_config.hosted_domain.as_ref() {
                    vec![("hd", hd)]
                } else {
                    vec![]
                })
                .build()
                .map_err(|e| OidcError::HttpError(e, "Failed to build URL.".to_owned()))?
                .url()
                .to_string(),
            state_string,
            redirect_uri,
        ))
    }

    async fn authorize(
        &self,
        endpoint: &str,
        code_challenge: &str,
    ) -> Result<(String, String), OidcError> {
        let (sender, reader) = tokio::sync::oneshot::channel();
        let (addr, server_future) = Self::create_local_listener(sender);

        let handle = tokio::task::spawn(server_future);
        let (url, state_string, redirect_uri) =
            self.build_authorize_url(endpoint, code_challenge, addr.port())?;

        open::that(url)
            .and_then(|exit_status| {
                exit_status.success().then(|| ()).ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!(
                            "Starting the browser exited with a non-zero exit code: {:?}",
                            exit_status.code()
                        ),
                    )
                })
            })
            .map_err(OidcError::FailedToOpenBrowser)?;

        let auth_response = reader
            .await
            .map_err(OidcError::FailedToReadCallbackResult)
            .and_then(|res| res.map_err(OidcError::FailedToHandleCallbackResult))?;
        handle.abort();

        (auth_response.state == state_string)
            .then(|| auth_response.result)
            .ok_or(OidcError::StateMismatch)
            .and_then(|res| match res {
                AuthResponseResult::Error(e) => Err(OidcError::AuthError(e)),
                AuthResponseResult::Code(code) => Ok((code, redirect_uri)),
            })
    }

    async fn get_config(&self) -> Result<OidcConfig, OidcError> {
        reqwest::get(self.oidc_config.discovery_url.to_string())
            .map_err(OidcError::TransportError)
            .and_then(|response| async {
                match response.error_for_status_ref() {
                    Err(e) => Err(OidcError::HttpError(
                        e,
                        response.text().await.unwrap_or_else(|e| {
                            format!("Failed to get body of error response: {}", e)
                        }),
                    )),
                    Ok(_) => response
                        .json::<OidcConfig>()
                        .await
                        .map_err(OidcError::JsonError),
                }
            })
            .await
    }

    async fn exchange_authorization(
        &self,
        token_endpoint: &str,
        auth_token: &str,
        code_challenge: &str,
        redirect_uri: &str,
    ) -> Result<AuthToken, OidcError> {
        let client = reqwest::Client::new();
        let params = [
            ("client_id", self.oidc_config.client_id.as_str()),
            ("client_secret", self.oidc_config.client_secret.as_str()),
            ("code", auth_token),
            ("code_verifier", code_challenge),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect_uri),
        ];

        client
            .post(token_endpoint)
            .form(&params)
            .send()
            .map_err(OidcError::TransportError)
            .and_then(|response| async {
                match response.error_for_status_ref() {
                    Err(e) => Err(OidcError::HttpError(
                        e,
                        response.text().await.unwrap_or_else(|e| {
                            format!("Failed to get body of error response: {}", e)
                        }),
                    )),
                    Ok(_) => response
                        .json::<AuthToken>()
                        .await
                        .map_err(OidcError::JsonError),
                }
            })
            .await
    }

    async fn validate_claims(
        client_id: &str,
        hosted_domain: &Option<String>,
        jwks_uri: &str,
        id_token: &str,
        supported_algorithms: &[Algorithm],
    ) -> Result<Claims, OidcError> {
        reqwest::get(jwks_uri)
            .map_err(OidcError::TransportError)
            .and_then(|response| async {
                match response.error_for_status_ref() {
                    Err(e) => Err(OidcError::HttpError(
                        e,
                        response.text().await.unwrap_or_else(|e| {
                            format!("Failed to get body of error response: {}", e)
                        }),
                    )),
                    Ok(_) => response
                        .json::<JsonWebKeyset>()
                        .await
                        .map_err(OidcError::JsonError),
                }
            })
            .await
            .and_then(|keys| {
                jsonwebtoken::decode_header(id_token)
                    .map_err(OidcError::FailedToDecodeJwtHeader)
                    .and_then(|h| {
                        match h.kid.as_ref().and_then(|kid| {
                            keys.keys.iter().find(|key| key.get("kid") == Some(kid))
                        }) {
                            Some(k) => Box::new(std::iter::once(k))
                                as Box<dyn Iterator<Item = &HashMap<String, String>>>,
                            None => Box::new(keys.keys.iter())
                                as Box<dyn Iterator<Item = &HashMap<String, String>>>,
                        }
                        .map(|key| {
                            key.get("n")
                                .and_then(|n| key.get("e").map(|e| (n, e)))
                                .and_then(|(n, e)| {
                                    let mut expected_aud = HashSet::new();
                                    expected_aud.insert(client_id.to_owned());
                                    jsonwebtoken::decode::<Claims>(
                                        id_token,
                                        &DecodingKey::from_rsa_components(&n, &e),
                                        &Validation {
                                            leeway: 10,
                                            aud: Some(expected_aud),
                                            algorithms: supported_algorithms.to_vec(),
                                            ..Default::default()
                                        },
                                    )
                                    .ok()
                                    .and_then(|c| {
                                        if let Some(hd) = hosted_domain.as_ref() {
                                            (Some(hd.as_str())
                                                == c.claims
                                                    .extra
                                                    .get("hd")
                                                    .and_then(|v| v.as_str()))
                                            .then(|| c.claims)
                                        } else {
                                            Some(c.claims)
                                        }
                                    })
                                })
                        })
                        .find(|o| o.is_some())
                        .and_then(|o| o)
                        .ok_or(OidcError::FailedToValidateClaims)
                    })
            })
    }

    pub async fn authenticate(&self) -> Result<OidcToken, OidcError> {
        let cfg = self.get_config().await.map_err(|e| {
            warn!(self.logger, "Failed to get OIDC configuration: {}", e);
            e
        })?;

        let (code_verifier, code_challenge) = Self::create_challenge(rand::thread_rng());
        let auth_endpoint = cfg.authorization_endpoint.clone();

        self.authorize(&auth_endpoint, &code_challenge)
            .and_then(|(auth_token, redirect_uri)| {
                let token_endpoint = cfg.token_endpoint.clone();

                async move {
                    self.exchange_authorization(
                        &token_endpoint,
                        &auth_token,
                        &code_verifier,
                        &redirect_uri,
                    )
                    .await
                }
            })
            .and_then(|auth_token| {
                let jwks_uri = cfg.jwks_uri.clone();
                let supported_algorithms = cfg.id_token_signing_alg_values_supported.clone();
                let token_endpoint = cfg.token_endpoint.clone();
                async move {
                    Self::validate_claims(
                        &self.oidc_config.client_id,
                        &self.oidc_config.hosted_domain,
                        &jwks_uri,
                        &auth_token.id_token,
                        &supported_algorithms,
                    )
                    .await
                    .map(|c| {
                        let context = AuthContext {
                            expires_at: AuthContext::calculate_expires_at(c.exp, c.iat, 10),
                            client_id: self.oidc_config.client_id.clone(),
                            client_secret: self.oidc_config.client_secret.clone(),
                            token_endpoint,
                            jwks_uri,
                            hosted_domain: self.oidc_config.hosted_domain.clone(),
                            id_token_signing_alg_values_supported: supported_algorithms,
                            claims: c,
                        };
                        OidcToken::new(auth_token, context)
                    })
                }
            })
            .map_err(|e| {
                warn!(self.logger, "Failed authenticate with OIDC: {}", e);
                e
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use mockito::{mock, Matcher};

    use super::*;

    // This is the public key used in the jwk macro further down
    const _RSA_PUBLIC_KEY: &str = r#"-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAnzyis1ZjfNB0bBgKFMSv
vkTtwlvBsaJq7S5wA+kzeVOVpVWwkWdVha4s38XM/pa/yr47av7+z3VTmvDRyAHc
aT92whREFpLv9cj5lTeJSibyr/Mrm/YtjCZVWgaOYIhwrXwKLqPr/11inWsAkfIy
tvHWTxZYEcXLgAXFuUuaS3uF9gEiNQwzGTU1v0FqkqTBr4B8nW3HCN47XUu0t8Y0
e+lf4s4OxQawWD79J9/5d3Ry0vbV3Am1FtGJiJvOwRsIfVChDpYStTcHTCMqtvWb
V6L11BWkpzGXSW4Hv43qa+GSYOD2QU68Mb59oSk2OB+BtOLpJofmbGEGgvmwyCI9
MwIDAQAB
-----END PUBLIC KEY----- "#;

    const RSA_PRIVATE_KEY: &str = r#"-----BEGIN RSA PRIVATE KEY-----
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
-----END RSA PRIVATE KEY-----"#;

    macro_rules! jwk {
        () => {
r#"
{
"keys": [ {
    "kty": "RSA",
    "n": "nzyis1ZjfNB0bBgKFMSvvkTtwlvBsaJq7S5wA-kzeVOVpVWwkWdVha4s38XM_pa_yr47av7-z3VTmvDRyAHcaT92whREFpLv9cj5lTeJSibyr_Mrm_YtjCZVWgaOYIhwrXwKLqPr_11inWsAkfIytvHWTxZYEcXLgAXFuUuaS3uF9gEiNQwzGTU1v0FqkqTBr4B8nW3HCN47XUu0t8Y0e-lf4s4OxQawWD79J9_5d3Ry0vbV3Am1FtGJiJvOwRsIfVChDpYStTcHTCMqtvWbV6L11BWkpzGXSW4Hv43qa-GSYOD2QU68Mb59oSk2OB-BtOLpJofmbGEGgvmwyCI9Mw",
    "e": "AQAB"
} ]
}
"#
        };

        ($kid:expr) => {
            &format!(
                r#"
{{
"keys": [ {{
    "kty": "RSA",
    "kid": "{}",
    "n": "nzyis1ZjfNB0bBgKFMSvvkTtwlvBsaJq7S5wA-kzeVOVpVWwkWdVha4s38XM_pa_yr47av7-z3VTmvDRyAHcaT92whREFpLv9cj5lTeJSibyr_Mrm_YtjCZVWgaOYIhwrXwKLqPr_11inWsAkfIytvHWTxZYEcXLgAXFuUuaS3uF9gEiNQwzGTU1v0FqkqTBr4B8nW3HCN47XUu0t8Y0e-lf4s4OxQawWD79J9_5d3Ry0vbV3Am1FtGJiJvOwRsIfVChDpYStTcHTCMqtvWbV6L11BWkpzGXSW4Hv43qa-GSYOD2QU68Mb59oSk2OB-BtOLpJofmbGEGgvmwyCI9Mw",
    "e": "AQAB"
}} ]
}}
"#,
                $kid
            )
        };
    }

    macro_rules! null_logger {
        () => {{
            slog::Logger::root(slog::Discard, slog::o!())
        }};
    }

    macro_rules! create_oidc {
        () => {{
            create_oidc!("to_be_discovered")
        }};

        ($discovery_url:expr) => {{
            create_oidc!($discovery_url, "client_id")
        }};

        ($discovery_url:expr, $client_id:expr) => {{
            create_oidc!($discovery_url, $client_id, None)
        }};

        ($discovery_url:expr, $client_id:expr, $hosted_domain:expr) => {{
            Oidc::new(
                &crate::config::OidcProvider {
                    discovery_url: $discovery_url.to_owned(),
                    client_id: $client_id.to_owned(),
                    client_secret: "supersecret".to_owned(),
                    hosted_domain: $hosted_domain,
                },
                null_logger!(),
            )
        }};
    }

    #[tokio::test]
    async fn auth_exchange() {
        let oidc = create_oidc!();
        let _m = mock("POST", Matcher::Any)
            .with_status(400)
            .with_body("oh no".to_owned())
            .create();

        // Invalid server url
        let result = oidc
            .exchange_authorization(
                "Not really a server url",
                "auth_token",
                "code_challenge",
                "redirect_uri",
            )
            .await;

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), OidcError::TransportError(..)));

        // Invalid server response
        let result = oidc
            .exchange_authorization(
                &mockito::server_url(),
                "auth_token",
                "code_challenge",
                "redirect_uri",
            )
            .await;

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), OidcError::HttpError(..)));

        // Invalid json object
        let _m = mock("POST", Matcher::Any)
            .with_status(200)
            .with_body(
                r#"
{
 "boll": "true"
}"#
                .to_owned(),
            )
            .create();

        let result = oidc
            .exchange_authorization(
                &mockito::server_url(),
                "auth_token",
                "code_challenge",
                "redirect_uri",
            )
            .await;

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), OidcError::JsonError(..)));

        // Valid object
        let mock = mock("POST", Matcher::Any)
            .with_status(200)
            .with_body(
                r#"
{
"access_token": "1/fFAGRNJru1FTz70BzhT3Zg",
"expires_in": 3920,
"token_type": "Bearer",
"scope": "https://www.googleapis.com/auth/drive.metadata.readonly",
"refresh_token": "1//xEoDL4iW3cxlI7yDbSRFYNG01kVKM2C-259HOF2aQbI",
"id_token": "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiYWRtaW4iOnRydWUsImlhdCI6MTUxNjIzOTAyMn0.POstGetfAytaZS82wHcjoTyoqhMyxXiWdR7Nn7A29DNSl0EiXLdwJ6xC6AfgZWF1bOsS_TuYI3OG85AmiExREkrS6tDfTQ2B3WXlrr-wp5AokiRbz3_oB4OxG-W9KcEEbDRcZc0nH3L7LzYptiy1PtAylQGxHTWZXtGz4ht0bAecBgmpdgXMguEIcoqPJ1n3pIWk_dUZegpqx0Lka21H6XxUTxiy8OcaarA8zdnPUnV6AmNP3ecFawIFYdvJB_cm-GvpCSbr8G8y_Mllj8f4x9nBH8pQux89_6gUY618iYv7tuPWBFfEbLxtF2pZS6YC1aSfLQxeNe8djT9YjpvRZA"
} 
"#,
            )
            .create();

        let result = oidc
            .exchange_authorization(
                &mockito::server_url(),
                "auth_token",
                "code_challenge",
                "redirect_uri",
            )
            .await;

        assert!(mock.matched());

        assert!(result.is_ok());
        assert_eq!(
            result.unwrap(),
            AuthToken {
                access_token: "1/fFAGRNJru1FTz70BzhT3Zg".to_owned(),
                expires_in: 3920,
                id_token: "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiYWRtaW4iOnRydWUsImlhdCI6MTUxNjIzOTAyMn0.POstGetfAytaZS82wHcjoTyoqhMyxXiWdR7Nn7A29DNSl0EiXLdwJ6xC6AfgZWF1bOsS_TuYI3OG85AmiExREkrS6tDfTQ2B3WXlrr-wp5AokiRbz3_oB4OxG-W9KcEEbDRcZc0nH3L7LzYptiy1PtAylQGxHTWZXtGz4ht0bAecBgmpdgXMguEIcoqPJ1n3pIWk_dUZegpqx0Lka21H6XxUTxiy8OcaarA8zdnPUnV6AmNP3ecFawIFYdvJB_cm-GvpCSbr8G8y_Mllj8f4x9nBH8pQux89_6gUY618iYv7tuPWBFfEbLxtF2pZS6YC1aSfLQxeNe8djT9YjpvRZA".to_owned(),
                refresh_token: "1//xEoDL4iW3cxlI7yDbSRFYNG01kVKM2C-259HOF2aQbI".to_owned(),
                scope: "https://www.googleapis.com/auth/drive.metadata.readonly".to_owned(),
                token_type: "Bearer".to_owned(),
            }
        );
    }

    #[tokio::test]
    async fn get_config() {
        // No server
        let oidc = create_oidc!();
        let res = oidc.get_config().await;
        assert!(res.is_err());
        assert!(
            matches!(res.unwrap_err(), OidcError::TransportError(..)),
            "Without a valid url we get a transport error"
        );

        //Bad response
        let oidc = create_oidc!(mockito::server_url());
        let _m = mock("GET", Matcher::Any)
            .with_status(400)
            .with_body("oh no".to_owned())
            .create();
        let res = oidc.get_config().await;
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), OidcError::HttpError(..)));

        // Good config
        let cfg = OidcConfig {
            issuer: "i-have-issues".to_owned(),
            authorization_endpoint: "https://the-end-of-auth-point".to_owned(),
            device_authorization_endpoint: None,
            token_endpoint: "https:://token-of-endpoint-gratitude".to_owned(),
            userinfo_endpoint: None,
            revocation_endpoint: None,
            jwks_uri: "https://no".to_owned(),
            response_types_supported: vec!["ja".to_owned(), "nej".to_owned()],
            subject_types_supported: vec!["emails".to_owned(), "private".to_owned()],
            id_token_signing_alg_values_supported: vec![Algorithm::RS256],
            scopes_supported: None,
            token_endpoint_auth_methods_supported: None,
            claims_supported: None,
            code_challenge_methods_supported: None,
        };
        let mock = mock("GET", Matcher::Any)
            .with_body(serde_json::to_string(&cfg).unwrap())
            .create();

        let res = oidc.get_config().await;
        assert!(res.is_ok());
        assert!(mock.matched());
        assert_eq!(res.unwrap(), cfg);
    }

    #[tokio::test]
    async fn create_local_listener() {
        // Everything is good
        let (sender, reader) = tokio::sync::oneshot::channel();
        let (addr, server_future) = Oidc::create_local_listener(sender);
        let handle = tokio::task::spawn(server_future);
        let q = r#"state=security_token%3Dyas&code=edoc&scope=super"#;
        let resp = reqwest::get(format!("http://[::1]:{}/?{}", addr.port(), q)).await;

        handle.abort();
        assert!(resp.unwrap().status().is_success());

        let auth_response = reader.await.unwrap();
        assert!(auth_response.is_ok());

        let auth_response = auth_response.unwrap();
        assert!(matches!(auth_response.result, AuthResponseResult::Code(code) if code == "edoc"));
        assert_eq!(auth_response.state, "security_token=yas".to_owned());

        // Access denied
        let (sender, reader) = tokio::sync::oneshot::channel();
        let (addr, server_future) = Oidc::create_local_listener(sender);
        let handle = tokio::task::spawn(server_future);
        let q = r#"state=security_token%3Dyas&error=access_denied&scope=super"#;
        let resp = reqwest::get(format!("http://[::1]:{}/?{}", addr.port(), q)).await;

        handle.abort();
        assert!(resp.unwrap().status().is_success());

        let auth_response = reader.await.unwrap();
        assert!(auth_response.is_ok());

        let auth_response = auth_response.unwrap();
        assert!(
            matches!(auth_response.result, AuthResponseResult::Error(e) if e == "access_denied")
        );
        assert_eq!(auth_response.state, "security_token=yas".to_owned());

        //Without state
        let (sender, reader) = tokio::sync::oneshot::channel();
        let (addr, server_future) = Oidc::create_local_listener(sender);
        let handle = tokio::task::spawn(server_future);
        let q = r#"code=edoc&scope=super"#;
        let resp = reqwest::get(format!("http://[::1]:{}/?{}", addr.port(), q)).await;

        handle.abort();
        assert!(resp.unwrap().status().is_client_error());

        let auth_response = reader.await.unwrap();
        assert!(auth_response.is_err());
    }

    #[tokio::test]
    async fn validate_claims() {
        // Bad response
        let _m = mock("GET", Matcher::Any)
            .with_status(400)
            .with_body("sad times")
            .create();
        let client_id = "frequenter identifier".to_owned();
        let oidc = create_oidc!("discovery_url", client_id.clone());

        let result = Oidc::validate_claims(
            &oidc.oidc_config.client_id,
            &oidc.oidc_config.hosted_domain,
            &mockito::server_url(),
            "galningen som token",
            &[],
        )
        .await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), OidcError::HttpError(..)));

        // Bad json in good response
        let _m = mock("GET", Matcher::Any)
            .with_status(200)
            .with_body("This is not json, this is Johansson!")
            .create();

        let result = Oidc::validate_claims(
            &oidc.oidc_config.client_id,
            &oidc.oidc_config.hosted_domain,
            &mockito::server_url(),
            "galningen som token",
            &[],
        )
        .await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), OidcError::JsonError(..)));

        // Bad json web token
        let _m = mock("GET", Matcher::Any)
            .with_status(200)
            .with_body(jwk!())
            .create();

        let result = Oidc::validate_claims(
            &oidc.oidc_config.client_id,
            &oidc.oidc_config.hosted_domain,
            &mockito::server_url(),
            "galningen som token",
            &[],
        )
        .await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            OidcError::FailedToDecodeJwtHeader(..)
        ));

        // Checking validation failure
        let result = Oidc::validate_claims(
            &oidc.oidc_config.client_id,
            &oidc.oidc_config.hosted_domain,
                &mockito::server_url(),
                "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiYWRtaW4iOnRydWUsImlhdCI6MTUxNjIzOTAyMn0.POstGetfAytaZS82wHcjoTyoqhMyxXiWdR7Nn7A29DNSl0EiXLdwJ6xC6AfgZWF1bOsS_TuYI3OG85AmiExREkrS6tDfTQ2B3WXlrr-wp5AokiRbz3_oB4OxG-W9KcEEbDRcZc0nH3L7LzYptiy1PtAylQGxHTWZXtGz4ht0bAecBgmpdgXMguEIcoqPJ1n3pIWk_dUZegpqx0Lka21H6XxUTxiy8OcaarA8zdnPUnV6AmNP3ecFawIFYdvJB_cm-GvpCSbr8G8y_Mllj8f4x9nBH8pQux89_6gUY618iYv7tuPWBFfEbLxtF2pZS6YC1aSfLQxeNe8djT9YjpvRZA",
                &[Algorithm::RS256]
            )
            .await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            OidcError::FailedToValidateClaims
        ));

        let now = chrono::Utc::now().timestamp();
        let mut aud = HashSet::new();
        aud.insert(client_id.clone());

        let claims = Claims {
            iss: "".to_owned(),
            sub: "".to_owned(),
            aud: client_id.clone(),
            exp: (now + 3600) as u64,
            iat: now as u64,
            extra: HashMap::new(),
        };

        let header = jsonwebtoken::Header::new(Algorithm::RS256);
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(RSA_PRIVATE_KEY.as_bytes()).unwrap();
        let jwt = jsonwebtoken::encode(&header, &claims, &key).unwrap();

        let _m = mock("GET", Matcher::Any)
            .with_status(200)
            .with_body(jwk!())
            .create();

        let result = Oidc::validate_claims(
            &oidc.oidc_config.client_id,
            &oidc.oidc_config.hosted_domain,
            &mockito::server_url(),
            &jwt,
            &[Algorithm::RS256],
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), claims);

        // Different hosted domain must result in error
        let mut extra_claims = HashMap::new();
        extra_claims.insert(
            "hd".to_owned(),
            serde_json::Value::String("extremelybigbowls.com".to_owned()),
        );
        let oidc_with_hd = create_oidc!("sune", "bune", Some("mabrikaf.com".to_owned()));

        let invalid_claims = Claims {
            iss: "".to_owned(),
            aud: client_id.clone(),
            sub: "".to_owned(),
            exp: (now + 3600) as u64,
            iat: now as u64,
            extra: extra_claims,
        };

        let jwt = jsonwebtoken::encode(
            &jsonwebtoken::Header::new(Algorithm::RS256),
            &invalid_claims,
            &key,
        )
        .unwrap();

        let result = Oidc::validate_claims(
            &oidc_with_hd.oidc_config.client_id,
            &oidc_with_hd.oidc_config.hosted_domain,
            &mockito::server_url(),
            &jwt,
            &[Algorithm::RS256],
        )
        .await;
        assert!(
            result.is_err(),
            "Different hosted domain must result in error"
        );
        assert!(matches!(
            result.unwrap_err(),
            OidcError::FailedToValidateClaims
        ));

        // Test claims expiration from the past
        let invalid_claims = Claims {
            iss: "".to_owned(),
            sub: "".to_owned(),
            aud: client_id.clone(),
            exp: (now - 3600) as u64,
            iat: now as u64,
            extra: HashMap::new(),
        };

        let jwt = jsonwebtoken::encode(
            &jsonwebtoken::Header::new(Algorithm::RS256),
            &invalid_claims,
            &key,
        )
        .unwrap();

        let result = Oidc::validate_claims(
            &oidc.oidc_config.client_id,
            &oidc.oidc_config.hosted_domain,
            &mockito::server_url(),
            &jwt,
            &[Algorithm::RS256],
        )
        .await;
        assert!(
            result.is_err(),
            "Expiry date from the past must result in an error"
        );
        assert!(matches!(
            result.unwrap_err(),
            OidcError::FailedToValidateClaims
        ));

        // Claims without algorithms must result in error
        let jwt = jsonwebtoken::encode(&jsonwebtoken::Header::new(Algorithm::RS256), &claims, &key)
            .unwrap();

        let result = Oidc::validate_claims(
            &oidc.oidc_config.client_id,
            &oidc.oidc_config.hosted_domain,
            &mockito::server_url(),
            &jwt,
            &[Algorithm::HS384],
        )
        .await;
        assert!(
            result.is_err(),
            "Unsupported algorithm must result in an error"
        );
        assert!(matches!(
            result.unwrap_err(),
            OidcError::FailedToValidateClaims
        ));

        // Invald client_id must result in error
        let invalid_claims = Claims {
            iss: "".to_owned(),
            sub: "".to_owned(),
            aud: "I am not a client id".to_owned(),
            exp: (now + 3600) as u64,
            iat: now as u64,
            extra: HashMap::new(),
        };

        let jwt = jsonwebtoken::encode(
            &jsonwebtoken::Header::new(Algorithm::RS256),
            &invalid_claims,
            &key,
        )
        .unwrap();

        let result = Oidc::validate_claims(
            &oidc.oidc_config.client_id,
            &oidc.oidc_config.hosted_domain,
            &mockito::server_url(),
            &jwt,
            &[Algorithm::RS256],
        )
        .await;
        assert!(result.is_err(), "Invalid client Id must result in error");
        assert!(matches!(
            result.unwrap_err(),
            OidcError::FailedToValidateClaims
        ));

        // Key id in header only
        let mut header = jsonwebtoken::Header::new(Algorithm::RS256);
        header.kid = Some("1337deadbeef".to_owned());
        let jwt = jsonwebtoken::encode(&header, &claims, &key).unwrap();

        let result = Oidc::validate_claims(
            &oidc.oidc_config.client_id,
            &oidc.oidc_config.hosted_domain,
            &mockito::server_url(),
            &jwt,
            &[Algorithm::RS256],
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), claims);

        // Key id in header and key
        let _m = mock("GET", Matcher::Any)
            .with_status(200)
            .with_body(jwk!("1337deadbeef"))
            .create();

        let result = Oidc::validate_claims(
            &oidc.oidc_config.client_id,
            &oidc.oidc_config.hosted_domain,
            &mockito::server_url(),
            &jwt,
            &[Algorithm::RS256],
        )
        .await;
        assert!(dbg!(&result).is_ok());
        assert_eq!(result.unwrap(), claims);

        // Wrong key id in json web key set, note that this should still work
        // because the key ids are only used as a lookup optimization.
        // And invalid key id does not detract from the validity of the key.
        let _m = mock("GET", Matcher::Any)
            .with_status(200)
            .with_body(jwk!("1337alivebeef"))
            .create();

        let result = Oidc::validate_claims(
            &oidc.oidc_config.client_id,
            &oidc.oidc_config.hosted_domain,
            &mockito::server_url(),
            &jwt,
            &[Algorithm::RS256],
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), claims);
    }

    #[test]
    fn create_challenge() {
        use rand::SeedableRng;
        let rng = rand_pcg::Pcg64::seed_from_u64(2);

        let (verifier, challenge) = Oidc::create_challenge(rng);
        assert_eq!(verifier.len(), 128);
        // These values depend on the above seed (2)
        assert_eq!(verifier, "E1x.0izzwZJ8M-NdS8AOx8tZfAustd_xGy.rhcnfRtjW9mTdz.DWrTtka9cEdSkEHvp-G_tVJzOanfe7rUE_Pq9T8gKetyc8CVMPk7vS1V3-hHLKv8hkm3g~xGD~yydA");
        assert_eq!(challenge, "iETG2JlNPXjvv_pUjRlZtzog8djC8PzQP4ia6bDsLVE");
        assert_eq!(
            verifier
                .matches(
                    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-.~_"
                        .chars()
                        .collect::<Vec<char>>()
                        .as_slice(),
                )
                .count(),
            128
        );
        assert_eq!(
            base64::decode_config(challenge, base64::URL_SAFE_NO_PAD).unwrap(),
            sha2::Sha256::digest(verifier.as_bytes()).as_slice()
        );
    }

    #[test]
    fn calculate_expires_at() {
        assert_eq!(AuthContext::calculate_expires_at(110, 10, 25), 85u64);
    }

    #[test]
    fn build_authorize_url() {
        let client_id = "agent007";
        let oidc = create_oidc!("https://discovery-my-document.com", client_id);
        let (url, statestr, redirect) = oidc
            .build_authorize_url("https://test.com/oauth", "challenge-accepted", 1337u16)
            .unwrap();
        let url = url::Url::parse(&url).unwrap();
        let query = url.query_pairs();

        assert_eq!(
            query.clone().find_map(|(k, v)| (k == "state").then(|| v)),
            Some(Cow::Borrowed(statestr.as_str()))
        );
        assert_eq!(
            query
                .clone()
                .find_map(|(k, v)| (k == "redirect_uri").then(|| v)),
            Some(Cow::Borrowed(redirect.as_str()))
        );
        assert_eq!(
            query
                .clone()
                .find_map(|(k, v)| (k == "client_id").then(|| v)),
            Some(Cow::Borrowed(client_id))
        );
        assert_eq!(
            query
                .clone()
                .find_map(|(k, v)| (k == "response_type").then(|| v)),
            Some(Cow::Borrowed("code"))
        );
        assert!(
            query
                .clone()
                .find_map(|(k, v)| (k == "scope").then(|| v.contains("openid")))
                .unwrap_or_default(),
            "scope must contain at least openid"
        );
        assert_eq!(
            query
                .clone()
                .find_map(|(k, v)| (k == "code_challenge").then(|| v)),
            Some(Cow::Borrowed("challenge-accepted"))
        );
        assert_eq!(
            query
                .clone()
                .find_map(|(k, v)| (k == "code_challenge_method").then(|| v)),
            Some(Cow::Borrowed("S256"))
        );
        assert_eq!(
            query
                .clone()
                .find_map(|(k, v)| (k == "access_type").then(|| v)),
            Some(Cow::Borrowed("offline"))
        );
        assert_eq!(
            url::Url::parse(&redirect).unwrap().port(),
            Some(1337u16),
            "The redirect uri must use the port we sent in"
        );
    }

    #[tokio::test]
    async fn refresh() {
        // Do not refresh token
        let m = mock("POST", Matcher::Any).create();
        let mut auth = OidcToken::new(
            AuthToken {
                access_token: String::new(),
                expires_in: 2u64,
                id_token: String::new(),
                refresh_token: String::new(),
                scope: String::new(),
                token_type: String::new(),
            },
            AuthContext {
                expires_at: (chrono::Utc::now().timestamp() + 3600) as u64,
                client_id: String::new(),
                client_secret: String::new(),
                token_endpoint: String::new(),
                jwks_uri: String::new(),
                hosted_domain: None,
                id_token_signing_alg_values_supported: vec![],
                claims: Claims {
                    iss: "sssss".to_owned(),
                    sub: "submarine".to_owned(),
                    aud: "sune".to_owned(),
                    exp: 8u64,
                    iat: 5u64,
                    extra: HashMap::new(),
                },
            },
        );
        let log = null_logger!();
        let r = auth.refresh(&log).await;
        assert!(r.is_ok());
        assert!(
            !m.matched(),
            "A non expired token must not generate any api calls."
        );

        // Refresh token
        mockito::reset();
        let now = chrono::Utc::now().timestamp();
        let claims = Claims {
            iss: "".to_owned(),
            sub: "".to_owned(),
            aud: "sune".to_owned(),
            exp: (now + 3600) as u64,
            iat: now as u64,
            extra: HashMap::new(),
        };

        let header = jsonwebtoken::Header::new(Algorithm::RS256);
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(RSA_PRIVATE_KEY.as_bytes()).unwrap();
        let jwt = jsonwebtoken::encode(&header, &claims, &key).unwrap();

        let m = mock("POST", "/token")
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("client_id".to_owned(), "sune".to_owned()),
                Matcher::UrlEncoded("client_secret".to_owned(), "sunes-secret".to_owned()),
                Matcher::UrlEncoded("refresh_token".to_owned(), "super-fresh".to_owned()),
                Matcher::UrlEncoded("grant_type".to_owned(), "refresh_token".to_owned()),
            ]))
            .match_header("Content-Type", "application/x-www-form-urlencoded")
            .with_status(200)
            .with_body(format!(
                r#"
{{
  "access_token": "1/fFAGRNJru1FTz70BzhT3Zg",
  "expires_in": 3920,
  "scope": "https://scopy-scope.biz",
  "token_type": "Bearer",
  "id_token": "{}"
}}
"#,
                jwt
            ))
            .create();

        let jwks = mock("GET", "/jwks")
            .with_status(200)
            .with_body(jwk!())
            .create();

        auth.context.token_endpoint = format!("{}/{}", mockito::server_url(), "token".to_owned());
        auth.context.expires_at = (chrono::Utc::now().timestamp() - 500) as u64;
        auth.context.client_id = "sune".to_owned();
        auth.context.client_secret = "sunes-secret".to_owned();
        auth.auth_token.refresh_token = "super-fresh".to_owned();
        auth.context.jwks_uri = format!("{}/{}", mockito::server_url(), "jwks".to_owned());

        let result = auth.refresh(&log).await;
        assert!(
            result.is_err(),
            "Missing supported algorithms should fail the jwt claim validation"
        );
        assert!(m.matched());
        assert!(jwks.matched());

        if let Err(e) = result {
            assert!(e.contains("claims"));
        }

        auth.context.id_token_signing_alg_values_supported = vec![Algorithm::RS256];
        let result = auth.refresh(&log).await;
        assert!(result.is_ok());
    }
}
