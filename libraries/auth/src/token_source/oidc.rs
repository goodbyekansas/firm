use std::{
    collections::HashMap,
    fmt::{Debug, Display},
    net::SocketAddr,
    net::{Ipv6Addr, SocketAddrV6},
    pin::Pin,
};

use base64::Engine;
use futures::{FutureExt, TryFutureExt};
use jsonwebtoken::Algorithm;
use rand::{seq::SliceRandom, Rng};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::sync::oneshot::{Receiver, Sender};
use url::Url;
use warp::Filter;

use crate::{
    token::{ExpectedClaims, Jwks},
    CredentialStore, Token,
};

#[derive(Error, Debug)]
pub enum Error {
    #[error("Transport error: {0}")]
    TransportError(#[source] reqwest::Error),

    #[error("HTTP error: {0} {1}")]
    HttpError(#[source] reqwest::Error, String),

    #[error("JSON error: {0}")]
    JsonError(#[source] reqwest::Error),

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

    #[error("Token Error: {0}")]
    TokenError(#[from] crate::token::TokenError),
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct AuthToken {
    #[serde(skip_serializing, default)]
    access_token: String,

    #[serde(skip_serializing, default)]
    expires_in: u64,

    #[serde(skip_serializing, default)]
    id_token: String,

    refresh_token: String,

    #[serde(skip_serializing, default)]
    scope: String,

    #[serde(skip_serializing, default)]
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

    async fn validate_claims(
        &self,
        oidc_config: &OidcConfig,
        client_id: &str,
    ) -> Result<Claims, Error> {
        let key_set = oidc_config
            .jwks_uri
            .parse::<Jwks>()
            .map_err(|e| Error::AuthError(format!("Failed to create JWKS: {}", e)))?; // TODO: Cache keyset

        Token::try_new(&self.id_token)
            .map_err(|e| Error::AuthError(format!("Failed to create token from JWT: {}", e)))?
            .validate(
                &[key_set],
                ExpectedClaims {
                    iss: [oidc_config.issuer.as_str()],
                    aud: &[client_id],
                    sub: None,
                    alg: &oidc_config.id_token_signing_alg_values_supported,
                },
            )
            .await
            .map_err(|e| {
                dbg!(e);
                Error::FailedToValidateClaims
            })
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct RefreshedToken {
    access_token: String,
    expires_in: u64,
    id_token: String,
    scope: String,
    token_type: String,
}

#[derive(Serialize, Deserialize)]
pub struct Provider {
    auth_token: AuthToken,
    client_id: String,
    client_secret: String,
    hosted_domains: Vec<String>,
    oidc_config: OidcConfig,

    #[serde(skip)]
    claims: Claims,

    #[serde(skip)]
    expires_at: u64,
}

#[derive(Debug)]
pub enum AuthenticationState {
    InteractiveLogin(InteractiveLogin),
    LoggedIn(Provider),
}

#[derive(Debug)]
pub struct InteractiveLogin {
    url: String,
    client_id: String,
    client_secret: String,
    oidc_config: OidcConfig,
    reader: Receiver<Result<AuthResponse, String>>,
    redirect_url: String,
    state_string: String,
    code_verifier: String,
    hosted_domains: Vec<String>,
    shutdown_writer: Sender<()>,
}

impl InteractiveLogin {
    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn discard(self) {
        let _ = self.shutdown_writer.send(());
    }

    pub async fn run(self) -> Result<Provider, Error> {
        let res = self.reader.await;
        let _ = self.shutdown_writer.send(());
        futures::future::ready(res)
            .map_err(Error::FailedToReadCallbackResult)
            .and_then(|res| async move { res.map_err(Error::FailedToHandleCallbackResult) })
            .and_then(|auth_response| async move {
                match (auth_response.state == self.state_string)
                    .then(|| auth_response.result)
                    .ok_or(Error::StateMismatch)?
                {
                    AuthResponseResult::Error(e) => Err(Error::AuthError(e)),
                    AuthResponseResult::Code(code) => Ok(code),
                }
            })
            .and_then(|code| {
                reqwest::Client::new()
                    .post(&self.oidc_config.token_endpoint)
                    .form(&[
                        ("client_id", self.client_id.as_str()),
                        ("client_secret", self.client_secret.as_str()),
                        ("code", &code),
                        ("code_verifier", &self.code_verifier),
                        ("grant_type", "authorization_code"),
                        ("redirect_uri", &self.redirect_url),
                    ])
                    .send()
                    .map_err(Error::TransportError)
                    .and_then(|response| async {
                        match response.error_for_status_ref() {
                            Err(e) => Err(Error::HttpError(
                                e,
                                response.text().await.unwrap_or_else(|e| {
                                    format!("Failed to get body of error response: {}", e)
                                }),
                            )),
                            Ok(_) => response.json::<AuthToken>().await.map_err(Error::JsonError),
                        }
                    })
                    .and_then(|auth_token| {
                        Provider::new(
                            auth_token,
                            self.client_id,
                            self.client_secret,
                            self.oidc_config,
                            self.hosted_domains,
                        )
                    })
            })
            .await
    }
}

impl Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "OIDC token: {{audience: {}, expires_at: {}}}",
            self.aud(),
            self.expires_at()
        )
    }
}

impl TryFrom<&Provider> for Token {
    type Error = Error;

    fn try_from(provider: &Provider) -> Result<Self, Self::Error> {
        provider.as_token()
    }
}

impl Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl Provider {
    async fn new(
        auth_token: AuthToken,
        client_id: String,
        client_secret: String,
        oidc_config: OidcConfig,
        hosted_domains: Vec<String>,
    ) -> Result<Self, Error> {
        auth_token
            .validate_claims(&oidc_config, &client_id)
            .await
            .map(|claims| Self {
                auth_token,
                client_id,
                client_secret,
                hosted_domains,
                oidc_config,
                expires_at: Self::calculate_expires_at(claims.exp, claims.iat, 10),
                claims,
            })
    }

    pub fn as_token(&self) -> Result<Token, Error> {
        Token::try_new(self.token()).map_err(Into::into)
    }

    pub fn as_str(&self) -> &str {
        self.token()
    }

    pub fn token(&self) -> &str {
        &self.auth_token.id_token
    }

    pub fn expires_at(&self) -> u64 {
        self.expires_at
    }

    pub fn expired(&self) -> bool {
        chrono::Utc::now().timestamp() as u64 >= self.expires_at
    }

    fn calculate_expires_at(expires_at: u64, issued_at: u64, percent_margin: u64) -> u64 {
        issued_at
            + ((expires_at - issued_at) as f64 * (1f64 - (percent_margin as f64 * 0.01))) as u64
    }

    async fn finish_refresh(
        &mut self,
        refreshed_token: RefreshedToken,
    ) -> Result<&mut Self, Error> {
        let auth_token = self.auth_token.refresh(refreshed_token);
        auth_token
            .validate_claims(&self.oidc_config, &self.client_id)
            .await
            .map(|c| {
                self.expires_at = Self::calculate_expires_at(c.exp, c.iat, 10);
                self.claims = c;
                self.auth_token = auth_token;
                self
            })
    }

    pub async fn refresh(
        &mut self,
        credstore: &mut (dyn CredentialStore + Send),
    ) -> Result<&mut Self, Error> {
        if self.expired() {
            reqwest::Client::new()
                .post(&self.oidc_config.token_endpoint)
                .form(&[
                    ("client_id", self.client_id.as_str()),
                    ("client_secret", self.client_secret.as_str()),
                    ("refresh_token", self.auth_token.refresh_token.as_str()),
                    ("grant_type", "refresh_token"),
                ])
                .send()
                .map_err(Error::TransportError)
                .and_then(|response| async {
                    match response.error_for_status_ref() {
                        Ok(_) => {
                            response
                                .json::<RefreshedToken>()
                                .map_err(Error::JsonError)
                                .and_then(|refreshed_token| self.finish_refresh(refreshed_token))
                                .await
                        }
                        Err(e) => {
                            let err = Error::HttpError(
                                e,
                                response.text().await.unwrap_or_else(|e| {
                                    format!("Failed to get body of error response: {}", e)
                                }),
                            );
                            //warn!(logger, "Failed to refresh token: {}", err);
                            Err(err)
                        }
                    }
                })
                .await?;
            let _ = serde_json::to_string(self)
                .map_err(|e| {
                    // TODO: log
                    e
                })
                .ok()
                .map(|json| {
                    credstore.store(
                        &format!("{}-{}", self.oidc_config.token_endpoint, self.client_id),
                        &json,
                    )
                });
        }

        Ok(self)
    }

    pub fn exp(&self) -> u64 {
        self.claims.exp
    }

    pub fn iss(&self) -> &str {
        &self.claims.iss
    }

    pub fn iat(&self) -> u64 {
        self.claims.iat
    }

    pub fn jti(&self) -> Option<&str> {
        None
    }

    pub fn nbf(&self) -> Option<u64> {
        None
    }

    pub fn sub(&self) -> &str {
        &self.claims.sub
    }

    pub fn aud(&self) -> &str {
        &self.claims.aud
    }

    pub fn claim(&self, key: &str) -> Option<String> {
        self.claims.extra.get(key).map(|v| v.to_string())
    }
}

#[derive(Deserialize, Debug, Serialize, PartialEq, Clone)]
pub struct OidcConfig {
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

#[derive(Deserialize, Debug, PartialEq)]
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

#[derive(Deserialize, Serialize, Debug, PartialEq, Default)]
struct Claims {
    iss: String,
    sub: String,
    aud: String,
    exp: u64,
    iat: u64,

    #[serde(flatten)]
    extra: HashMap<String, serde_json::Value>,
}

pub struct Builder<'cred> {
    discovery_url: String,
    client_id: String,
    client_secret: String,

    hosted_domains: Vec<String>,
    credstore: Option<&'cred mut dyn CredentialStore>,
    signal: Option<Pin<Box<dyn std::future::Future<Output = ()> + Send>>>,
    oidc_config: Option<OidcConfig>,

    successful_login_redirect: Url,
}

impl<'cred> Builder<'cred> {
    pub fn new_with_discovery(
        discovery_url: &str,
        client_id: &str,
        client_secret: &str,
        successful_login_redirect: &Url,
    ) -> Self {
        Self {
            discovery_url: discovery_url.to_owned(),
            client_id: client_id.to_owned(),
            client_secret: client_secret.to_owned(),
            hosted_domains: vec![],
            credstore: None,
            signal: None,
            oidc_config: None,
            successful_login_redirect: successful_login_redirect.clone(),
        }
    }

    pub fn new_with_config(
        config: OidcConfig,
        client_id: &str,
        client_secret: &str,
        successful_login_redirect: &Url,
    ) -> Self {
        Self {
            discovery_url: String::new(),
            client_id: client_id.to_owned(),
            client_secret: client_secret.to_owned(),
            hosted_domains: vec![],
            credstore: None,
            signal: None,
            oidc_config: Some(config),
            successful_login_redirect: successful_login_redirect.clone(),
        }
    }

    pub fn with_hosted_domains<I: IntoIterator<Item = String>>(&mut self, domains: I) -> &mut Self {
        self.hosted_domains.extend(domains.into_iter());
        self
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
        let code_challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(code_verifier.as_slice()));
        (
            unsafe { String::from_utf8_unchecked(code_verifier) },
            code_challenge,
        )
    }

    fn create_local_listener(
        &mut self,
        sender: Sender<Result<AuthResponse, String>>,
    ) -> (
        SocketAddr,
        impl std::future::Future<Output = ()> + 'static,
        Sender<()>,
    ) {
        static mut SENDER: Option<Sender<Result<AuthResponse, String>>> = None;
        unsafe {
            SENDER = Some(sender);
        }

        fn send_response(s: Result<AuthResponse, String>) {
            unsafe {
                if let Some(sender) = SENDER.take() {
                    let _ = sender.send(s);
                }
            }
        }

        let (shutdown_writer, shutdown_reader) = tokio::sync::oneshot::channel();
        let redirect = self.successful_login_redirect.to_string();

        let srv = warp::serve(
            warp::filters::any::any()
                .and(warp::filters::query::query::<AuthResponse>())
                .map(Ok)
                .or_else(|_| async {
                    Ok::<(Result<AuthResponse, String>,), std::convert::Infallible>((Err(
                        "Failed to parse query string.".to_string(),
                    ),))
                })
                .map(move |res: Result<AuthResponse, String>| match res {
                    Ok(resp) => {
                        send_response(Ok(resp));
                        warp::http::Response::builder()
                            .header("Location", &redirect)
                            .status(302)
                            .body("")
                    }
                    Err(msg) => {
                        send_response(Err(msg));
                        warp::http::Response::builder()
                            .header("content-type", "text/plain")
                            .status(StatusCode::BAD_REQUEST)
                            .body("Failed to parse query string")
                    }
                }),
        )
        .bind_with_graceful_shutdown(
            SocketAddrV6::new(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1), 0, 0, 0),
            match self.signal.take() {
                Some(s) => {
                    async move {
                        futures::select! {
                            () = s.fuse() => {},
                            () = async move {let _ = shutdown_reader.await;}.boxed().fuse() => {}
                        }
                    }
                }
                .boxed(),
                None => (async move {
                    let _ = shutdown_reader.await;
                })
                .boxed(),
            },
        );

        (srv.0, srv.1, shutdown_writer)
    }

    fn build_authorize_url(
        &self,
        endpoint: &str,
        code_challenge: &str,
        port: u16,
    ) -> Result<(String, String, String), Error> {
        let state_string = format!(
            "security_token={}",
            &Self::create_challenge(rand::thread_rng()).0
        );
        let redirect_uri = format!("http://[::1]:{}", port);
        Ok((
            reqwest::Client::new()
                .get(endpoint)
                .query(&[
                    ("client_id", self.client_id.as_str()),
                    ("redirect_uri", redirect_uri.as_str()),
                    ("response_type", "code"),
                    ("scope", "openid profile email"),
                    ("code_challenge", code_challenge),
                    ("code_challenge_method", "S256"),
                    ("state", &state_string),
                    ("access_type", "offline"),
                ])
                .query(&match self.hosted_domains.len() {
                    0 => vec![],
                    1 => self
                        .hosted_domains
                        .first()
                        .map(|hd| vec![("hd", hd.as_str())])
                        .unwrap_or_default(),
                    _ => vec![("hd", "*")],
                })
                .build()
                .map_err(|e| Error::HttpError(e, "Failed to build URL.".to_owned()))?
                .url()
                .to_string(),
            state_string,
            redirect_uri,
        ))
    }

    async fn get_config(&self) -> Result<OidcConfig, Error> {
        reqwest::get(&self.discovery_url)
            .map_err(Error::TransportError)
            .and_then(|response| async {
                match response.error_for_status_ref() {
                    Err(e) => Err(Error::HttpError(
                        e,
                        response.text().await.unwrap_or_else(|e| {
                            format!("Failed to get body of error response: {}", e)
                        }),
                    )),
                    Ok(_) => response
                        .json::<OidcConfig>()
                        .await
                        .map_err(Error::JsonError),
                }
            })
            .await
    }

    pub async fn build(mut self) -> Result<AuthenticationState, Error> {
        let cfg = match self.oidc_config.take() {
            Some(c) => c,
            None => self.get_config().await.map_err(|e| {
                //warn!(self.logger, "Failed to get OIDC configuration: {}", e);
                e
            })?,
        };

        let (code_verifier, code_challenge) = Self::create_challenge(rand::thread_rng());
        let auth_endpoint = cfg.authorization_endpoint.clone();

        let (sender, reader) = tokio::sync::oneshot::channel();
        let (addr, server_future, shutdown_writer) = self.create_local_listener(sender);

        tokio::task::spawn(server_future);
        let (url, state_string, redirect_url) =
            self.build_authorize_url(&auth_endpoint, &code_challenge, addr.port())?;

        Ok(
            match self
                .credstore
                .and_then(|credstore| {
                    credstore
                        .retrieve(&format!("{}-{}", &cfg.token_endpoint, self.client_id))
                        .map_err(|e| {
                            // TODO: Log errors
                            e
                        })
                        .ok()
                        .flatten()
                })
                .and_then(|creds_source| {
                    let deserialized: Result<Provider, _> =
                        serde_json::from_str(creds_source.as_ref());
                    deserialized
                        .map_err(|e| {
                            // TODO: Log error
                            e
                        })
                        .ok()
                }) {
                Some(provider) => AuthenticationState::LoggedIn(provider),
                None => AuthenticationState::InteractiveLogin(InteractiveLogin {
                    url,
                    redirect_url,
                    state_string,
                    client_id: self.client_id,
                    client_secret: self.client_secret,
                    reader,
                    shutdown_writer,
                    oidc_config: cfg,
                    code_verifier,
                    hosted_domains: self.hosted_domains,
                }),
            },
        )
    }

    pub fn with_shutdown_signal<S>(mut self, signal: S) -> Self
    where
        S: std::future::Future<Output = ()> + Send + 'static,
    {
        self.signal = Some(Box::pin(signal));
        self
    }

    pub fn with_credential_store<C>(mut self, credstore: &'cred mut C) -> Self
    where
        C: CredentialStore,
    {
        self.credstore = Some(credstore);
        self
    }
}

#[cfg(test)]
mod tests {
    use crate::Memory;

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

    use super::*;
    use mockito::mock;

    #[tokio::test]
    async fn builder() {
        let google_openid_config = include_str!("../../test_data/google-openid-config.json");
        let google_openid_config_data: serde_json::Value =
            serde_json::from_str(google_openid_config).unwrap();
        let _m = mock("GET", "/.well-known/openid-configuration")
            .with_status(200)
            .with_body(google_openid_config)
            .create();

        let res = Builder::new_with_discovery(
            &format!(
                "{}/.well-known/openid-configuration",
                &mockito::server_url()
            ),
            "6",
            "hxx707-super-volvo",
            &Url::parse("https://goodbyekansas.com").unwrap(),
        )
        .build()
        .await;
        assert!(res.is_ok());
        let state = res.unwrap();
        assert!(matches!(state, AuthenticationState::InteractiveLogin(_)));
        match state {
            AuthenticationState::InteractiveLogin(login) => {
                assert_eq!(
                    login.oidc_config.issuer,
                    *google_openid_config_data.get("issuer").unwrap()
                );
                assert_eq!(
                    login.oidc_config.authorization_endpoint,
                    *google_openid_config_data
                        .get("authorization_endpoint")
                        .unwrap()
                );
                assert_eq!(
                    login.oidc_config.token_endpoint,
                    *google_openid_config_data.get("token_endpoint").unwrap()
                );
                assert_eq!(
                    login.oidc_config.jwks_uri,
                    *google_openid_config_data.get("jwks_uri").unwrap()
                );

                let _ = login.shutdown_writer.send(());
            }
            AuthenticationState::LoggedIn(_provider) => {
                panic!("I am not logged in");
            }
        }
    }

    #[tokio::test]
    async fn login() {
        let google_openid_config = include_str!("../../test_data/google-openid-config.json");

        let _m = mock("GET", "/.well-known/openid-configuration")
            .with_status(200)
            .with_body(google_openid_config)
            .create();

        let res = Builder::new_with_discovery(
            &format!(
                "{}/.well-known/openid-configuration",
                &mockito::server_url()
            ),
            "6",
            "hxx707-super-volvo",
            &Url::parse("https://goodbyekansas.com").unwrap(),
        )
        .build()
        .await;
        assert!(res.is_ok());
        let state = res.unwrap();
        assert!(matches!(state, AuthenticationState::InteractiveLogin(_)));

        match state {
            AuthenticationState::InteractiveLogin(login) => {
                reqwest::get(format!(
                    "{}?state={}&code=0xDEADBEEF",
                    login.redirect_url, login.state_string
                ))
                .await
                .expect("Failed to contact local listener.");
                let res = login
                    .reader
                    .await
                    .expect("Failed to await one shot message.");
                assert!(res.is_ok());
                let auth_resp = res.unwrap();
                assert_eq!(auth_resp.state, login.state_string);
                assert_eq!(
                    auth_resp.result,
                    AuthResponseResult::Code(String::from("0xDEADBEEF"))
                );

                // Sending another request should do nothing
                reqwest::get(format!(
                    "{}?state={}&code=0xDEADBEEF",
                    login.redirect_url, login.state_string
                ))
                .await
                .expect("Failed to contact local listener.");

                let _ = login.shutdown_writer.send(());
            }
            AuthenticationState::LoggedIn(_provider) => {
                panic!("I am not logged in");
            }
        }
    }

    #[tokio::test]
    async fn login_error() {
        let google_openid_config = include_str!("../../test_data/google-openid-config.json");

        let _m = mock("GET", "/.well-known/openid-configuration")
            .with_status(200)
            .with_body(google_openid_config)
            .create();

        let res = Builder::new_with_discovery(
            &format!(
                "{}/.well-known/openid-configuration",
                &mockito::server_url()
            ),
            "6",
            "hxx707-super-volvo",
            &Url::parse("https://goodbyekansas.com").unwrap(),
        )
        .build()
        .await;
        assert!(res.is_ok());
        let state = res.unwrap();
        assert!(matches!(state, AuthenticationState::InteractiveLogin(_)));

        match state {
            AuthenticationState::InteractiveLogin(login) => {
                reqwest::get(format!(
                    "{}?state={}&error=0xDEADBEEF",
                    login.redirect_url, login.state_string
                ))
                .await
                .expect("Failed to contact local listener.");
                let res = login
                    .reader
                    .await
                    .expect("Failed to await one shot message.");
                assert!(res.is_ok());
                let auth_resp = res.unwrap();
                assert_eq!(auth_resp.state, login.state_string);
                assert_eq!(
                    auth_resp.result,
                    AuthResponseResult::Error(String::from("0xDEADBEEF"))
                );

                // Sending another request should do nothing
                reqwest::get(format!(
                    "{}?state={}&code=0xDEADBEEF",
                    login.redirect_url, login.state_string
                ))
                .await
                .expect("Failed to contact local listener.");
                let _ = login.shutdown_writer.send(());
            }
            AuthenticationState::LoggedIn(_provider) => {
                panic!("I am not logged in");
            }
        }
    }

    #[tokio::test]
    async fn cached_token() {
        let google_openid_config = include_str!("../../test_data/google-openid-config.json");
        let google_openid_config_data: OidcConfig =
            serde_json::from_str(google_openid_config).unwrap();

        let mut credstore = Memory::new();
        let res = credstore.store(
            &format!("{}-{}", google_openid_config_data.token_endpoint, "6"),
            serde_json::to_string(&Provider {
                auth_token: AuthToken {
                    access_token: String::from("access-permitted!"),
                    expires_in: 0,
                    id_token: String::new(),
                    refresh_token: String::from("refresh-me"),
                    scope: String::new(),
                    token_type: String::new(),
                },
                client_id: String::from("6"),
                client_secret: String::from("hxx797-super-volvo"),
                hosted_domains: vec![],
                oidc_config: google_openid_config_data.clone(),
                claims: Claims::default(),
                expires_at: 0,
            })
            .unwrap()
            .as_str(),
        );
        assert!(res.is_ok());

        let res = Builder::new_with_config(
            google_openid_config_data.clone(),
            "6",
            "hxx707-super-volvo",
            &Url::parse("https://goodbyekansas.com").unwrap(),
        )
        .with_credential_store(&mut credstore)
        .build()
        .await;

        assert!(res.is_ok());
        let res = res.unwrap();
        assert!(matches!(res, AuthenticationState::LoggedIn(_)));
        match res {
            AuthenticationState::LoggedIn(token) => {
                assert!(
                    token.auth_token.access_token.is_empty(),
                    "Access token must not be serialized"
                );

                assert_eq!(
                    token.auth_token.refresh_token, "refresh-me",
                    "Expected refresh token to have been serialized+deserialized"
                );

                assert_eq!(
                    token.oidc_config.token_endpoint, google_openid_config_data.token_endpoint,
                    "Token endpoint is needed to be able to refresh token"
                );
            }
            _ => panic!("Previous assert did not do its job!"),
        }
    }

    #[tokio::test]
    async fn interactive_login() {
        crate::token::allow_insecure_jwks();
        let mut google_openid_config_data: OidcConfig =
            serde_json::from_str(include_str!("../../test_data/google-openid-config.json"))
                .unwrap();

        #[derive(Serialize)]
        struct AuthTokenResponse {
            access_token: String,
            expires_in: u64,
            id_token: String,
            refresh_token: String,
            scope: String,
            token_type: String,
        }

        let now = chrono::Utc::now().timestamp();
        let mut extra = HashMap::new();
        extra.insert(
            "hd".to_owned(),
            serde_json::Value::String("nedry.jp.com".to_owned()),
        );
        let claims = Claims {
            iss: String::from("gorgel"),
            sub: String::new(),
            aud: String::from("datakörkort"),
            exp: (now + 3600) as u64,
            iat: now as u64,
            extra,
        };

        let header = jsonwebtoken::Header::new(Algorithm::RS256);
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(RSA_PRIVATE_KEY.as_bytes()).unwrap();
        let jwt = jsonwebtoken::encode(&header, &claims, &key).unwrap();

        let token = AuthTokenResponse {
            access_token: String::from("access"),
            expires_in: 0,
            id_token: jwt.clone(),
            refresh_token: String::from("refräsch"),
            scope: String::from("skåp"),
            token_type: String::from("tåken-typ"),
        };

        let _m = mock("POST", "/token")
            .with_status(200)
            .with_body(serde_json::to_string(&token).unwrap())
            .create();

        let _m2 = mock("GET", "/oauth2/v3/certs")
            .with_status(200)
            .with_body(jwk!())
            .create();

        google_openid_config_data.token_endpoint = format!("{}/token", &mockito::server_url());
        google_openid_config_data.jwks_uri = format!("{}/oauth2/v3/certs", &mockito::server_url());
        google_openid_config_data.id_token_signing_alg_values_supported = vec![Algorithm::RS256];
        google_openid_config_data.issuer = String::from("gorgel");

        let (writer, reader) = tokio::sync::oneshot::channel();
        let (shutdown_writer, mut shutdown_reader) = tokio::sync::oneshot::channel();
        let login = InteractiveLogin {
            url: String::from("https://log.me.in"),
            client_id: String::from("datakörkort"),
            client_secret: String::from("hemliga-klubben"),
            oidc_config: google_openid_config_data.clone(),
            reader,
            shutdown_writer,
            redirect_url: String::from("https:://dirigera.om.mig"),
            state_string: String::from("statlig-sträng"),
            code_verifier: String::from("verifiera-koden"),
            hosted_domains: vec![String::from("nedry.jp.com")],
        };

        writer
            .send(Ok(AuthResponse {
                state: String::from("statlig-sträng"),
                result: AuthResponseResult::Code(String::from("kåd")),
            }))
            .expect("Expected to be able to send message to interactive login");

        let res = login.run().await;
        assert!(dbg!(&res).is_ok());
        let mut res = res.unwrap();
        assert_eq!(res.auth_token.access_token, "access");
        assert_eq!(res.aud(), "datakörkort");

        assert!(
            shutdown_reader.try_recv().is_ok(),
            "Expected interactive login to have shut down the server after running"
        );

        res.expires_at = 0;
        let _m3 = mock("GET", "/token").with_status(200).with_body(
            serde_json::to_string(&RefreshedToken {
                access_token: String::from("access"),
                expires_in: 99999,
                id_token: jwt.clone(),
                scope: String::from("skåp"),
                token_type: String::from("🐻er"),
            })
            .unwrap(),
        );

        let mut credstore = Memory::new();
        let res = res.refresh(&mut credstore).await;
        assert_eq!(res.unwrap().as_str(), jwt);
        let res = credstore.retrieve(&format!(
            "{}-{}",
            google_openid_config_data.token_endpoint, "datakörkort"
        ));
        assert!(res.is_ok());
        let res = res.unwrap();
        assert!(res.is_some());
    }
}
