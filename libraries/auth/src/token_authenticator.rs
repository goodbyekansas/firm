use std::{collections::HashMap, marker::PhantomData};

use crate::{
    token::{Jwks, TokenError},
    Token,
};
use futures::TryFutureExt;
use regex::Regex;
use reqwest::Url;
use serde::{de::DeserializeOwned, Deserialize};

#[derive(thiserror::Error, Debug)]
pub enum TokenAuthenticatorError {
    #[error("Invalid token: {0}")]
    InvalidToken(#[source] crate::token::TokenError),

    // TODO: We may not want to be this clear about the error.
    #[error("Invalid subject: {0}")]
    InvalidSubject(String),

    #[error("Invalid source: \"{0}\"")]
    InvalidSource(String),
}

#[derive(PartialEq)]
enum Trusted {
    Jwks,
    Issuers(Vec<String>),
}

pub struct TokenAuthenticator<ApprovalMethod: TokenAuthenticatorApproval> {
    allowed_subjects: Vec<String>,
    allowed_audiences: Vec<String>,
    requests: HashMap<AccessRequestId, AccessRequest>,
    marker: PhantomData<ApprovalMethod>,
    mode: Trusted,
    trusted_jwks: Vec<String>,
    algorithms: Vec<jsonwebtoken::Algorithm>,
}

pub trait TokenAuthenticatorApproval {
    type Claims: DeserializeOwned;
    fn request_from_claims(claims: Self::Claims, allowed_subjects: &[String]) -> AccessRequest;
}

struct InteractiveApproval {}
struct PreconfiguredApproval {}

#[derive(Deserialize)]
struct InteractiveRequiredClaims {
    sub: String,
    exp: u64,
}

impl TokenAuthenticatorApproval for InteractiveApproval {
    type Claims = InteractiveRequiredClaims;
    fn request_from_claims(claims: Self::Claims, allowed_subjects: &[String]) -> AccessRequest {
        AccessRequest::new(
            claims.sub.clone(),
            if allowed_subjects.contains(&claims.sub) {
                ApprovalState::Approved
            } else {
                ApprovalState::Pending(claims.exp)
            },
        )
    }
}

#[derive(Deserialize)]
struct PreconfiguredRequiredClaims {
    sub: String,
}

impl TokenAuthenticatorApproval for PreconfiguredApproval {
    type Claims = PreconfiguredRequiredClaims;
    fn request_from_claims(claims: Self::Claims, allowed_subjects: &[String]) -> AccessRequest {
        AccessRequest::new(
            claims.sub.clone(),
            if allowed_subjects.contains(&claims.sub) {
                ApprovalState::Approved
            } else {
                ApprovalState::Denied
            },
        )
    }
}

impl<T: TokenAuthenticatorApproval> TokenAuthenticator<T> {
    pub fn new_trusted_jwks<S1: AsRef<str>, S2: AsRef<str>, S3: AsRef<str>>(
        trusted_jwks: &[S1],
        allowed_subjects: &[S2],
        allowed_audiences: &[S3],
        algorithms: &[jsonwebtoken::Algorithm],
    ) -> Self {
        Self {
            allowed_subjects: allowed_subjects
                .iter()
                .map(|s| s.as_ref().to_owned())
                .collect(),
            allowed_audiences: allowed_audiences
                .iter()
                .map(|s| s.as_ref().to_owned())
                .collect(),
            requests: HashMap::new(),
            marker: PhantomData,
            mode: Trusted::Jwks,
            trusted_jwks: trusted_jwks.iter().map(|s| s.as_ref().to_owned()).collect(),
            algorithms: algorithms.to_vec(),
        }
    }

    pub fn new_trusted_issuers<S1: AsRef<str>, S2: AsRef<str>, S3: AsRef<str>, S4: AsRef<str>>(
        trusted_issuers: &[S1],
        trusted_jwks: &[S2],
        allowed_subjects: &[S3],
        allowed_audiences: &[S4],
        algorithms: &[jsonwebtoken::Algorithm],
    ) -> Self {
        Self {
            allowed_subjects: allowed_subjects
                .iter()
                .map(|s| s.as_ref().to_owned())
                .collect(),
            allowed_audiences: allowed_audiences
                .iter()
                .map(|s| s.as_ref().to_owned())
                .collect(),
            requests: HashMap::new(),
            marker: PhantomData,
            mode: Trusted::Issuers(
                trusted_issuers
                    .iter()
                    .map(|s| s.as_ref().to_owned())
                    .collect(),
            ),
            trusted_jwks: trusted_jwks.iter().map(|s| s.as_ref().to_owned()).collect(),
            algorithms: algorithms.to_vec(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApprovalState {
    Approved,
    Denied,
    ApprovedExpiresAt(u64),
    Pending(u64),
}

impl ApprovalState {
    pub fn done(&self) -> bool {
        !matches!(self, ApprovalState::Pending(_))
    }

    pub fn allowed(&self, now: u64) -> bool {
        match self {
            ApprovalState::Approved => true,
            ApprovalState::ApprovedExpiresAt(exp) => *exp > now,
            _ => false,
        }
    }

    pub fn expires_at(&self) -> Option<u64> {
        match self {
            ApprovalState::ApprovedExpiresAt(exp) => Some(*exp),
            ApprovalState::Pending(exp) => Some(*exp),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct AccessRequestId(Token);

impl AsRef<Token> for AccessRequestId {
    fn as_ref(&self) -> &Token {
        &self.0
    }
}

impl From<Token> for AccessRequestId {
    fn from(t: Token) -> Self {
        Self(t)
    }
}

impl From<&Token> for AccessRequestId {
    fn from(t: &Token) -> Self {
        Self(t.clone())
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AccessRequest {
    subject: String,
    state: ApprovalState,
}

impl AccessRequest {
    fn new(subject: String, state: ApprovalState) -> Self {
        Self { subject, state }
    }

    pub fn expired(&self, now: u64) -> bool {
        self.expires_at().map(|exp| exp < now).unwrap_or(false)
    }

    pub fn needs_approval(&self) -> bool {
        !self.state.done()
    }

    pub fn allowed(&self, now: u64) -> bool {
        self.state.allowed(now)
    }

    pub fn subject(&self) -> &str {
        &self.subject
    }

    pub fn id(&self) -> &str {
        &self.subject
    }

    pub fn expires_at(&self) -> Option<u64> {
        self.state.expires_at()
    }
}

impl<T: TokenAuthenticatorApproval> TokenAuthenticator<T> {
    pub fn requests(&self, now: u64) -> impl Iterator<Item = &AccessRequest> {
        self.requests.values().filter(move |x| !x.expired(now))
    }

    pub fn get_request(&self, id: &AccessRequestId) -> Option<&AccessRequest> {
        self.requests.get(id)
    }

    pub fn has_request(&self, id: &AccessRequestId) -> bool {
        self.requests.contains_key(id)
    }

    pub fn approve_request(&mut self, id: &AccessRequestId) {
        if let Some(req) = self.requests.get_mut(id) {
            req.state = match req.state {
                ApprovalState::Pending(exp) => ApprovalState::ApprovedExpiresAt(exp),
                s => s,
            }
        }
    }

    pub fn deny_request(&mut self, id: &AccessRequestId) {
        if let Some(req) = self.requests.get_mut(id) {
            req.state = ApprovalState::Denied
        }
    }

    pub fn clear_expired_requests(&mut self, now: u64) {
        self.requests.retain(move |_, x| !x.expired(now))
    }

    pub fn pending_requests(&self) -> Vec<&AccessRequest> {
        self.requests
            .values()
            .filter(|req| matches!(req.state, ApprovalState::Pending(_)))
            .collect()
    }

    fn insert_request(&mut self, token: Token, req: AccessRequest) -> AccessRequestId {
        let id = AccessRequestId(token);
        self.requests.insert(id.clone(), req);
        id
    }

    fn expand_sources(&self, subject: &str) -> Vec<Jwks> {
        let subject_regex = Regex::new(r"\{\s*\{\s*subject\s*\}\s*\}").expect("Regex to be valid.");
        self.trusted_jwks
            .iter()
            .filter_map(|source| {
                // We are only interested in the ones where we can
                // replace the subject in the source when in Jwks
                // mode.
                if self.mode == Trusted::Jwks && !subject_regex.is_match(source) {
                    None
                } else {
                    let replaced_source = subject_regex.replace_all(source, subject);
                    Url::parse(&replaced_source)
                        .map_err(|e| TokenAuthenticatorError::InvalidSource(e.to_string()))
                        .and_then(|url| {
                            Jwks::try_new(url)
                                .map_err(|e| TokenAuthenticatorError::InvalidSource(e.to_string()))
                        })
                        .ok()
                }
            })
            .collect::<Vec<Jwks>>()
    }

    async fn validate_token<'a, D>(&self, token: &Token) -> Result<D, TokenAuthenticatorError>
    where
        D: DeserializeOwned,
    {
        #[derive(Deserialize)]
        struct SubjectOnlyClaims {
            sub: String,
        }

        let mut validation = jsonwebtoken::Validation::default();
        validation.insecure_disable_signature_validation();
        let claims = jsonwebtoken::decode::<SubjectOnlyClaims>(
            token.as_str(),
            &jsonwebtoken::DecodingKey::from_secret("secret".as_ref()),
            &validation,
        )
        .map_err(|e| TokenAuthenticatorError::InvalidToken(TokenError::Validation(e)))?
        .claims;

        // validate token
        token
            .validate(
                &self.expand_sources(&claims.sub),
                crate::token::ExpectedClaims {
                    iss: match &self.mode {
                        Trusted::Jwks => Vec::with_capacity(0),
                        Trusted::Issuers(issuers) => {
                            issuers.iter().map(AsRef::as_ref).collect::<Vec<_>>()
                        }
                    },
                    aud: self
                        .allowed_audiences
                        .iter()
                        .map(AsRef::as_ref)
                        .collect::<Vec<_>>()
                        .as_slice(),
                    sub: None,
                    alg: &self.algorithms,
                },
            )
            .map_err(TokenAuthenticatorError::InvalidToken)
            .await
    }
}

impl<T: TokenAuthenticatorApproval> TokenAuthenticator<T> {
    pub async fn authenticate(
        &mut self,
        token: Token,
    ) -> Result<AccessRequestId, TokenAuthenticatorError> {
        let id = token.clone().into();
        if self.has_request(&id) {
            Ok(id)
        } else {
            self.validate_token(&token)
                .map_ok(|claims| T::request_from_claims(claims, &self.allowed_subjects))
                .await
                .map(|req| self.insert_request(token, req))
        }
    }
}

#[cfg(test)]
mod tests {
    use jsonwebtoken::jwk::{Jwk, JwkSet};
    use tempfile::TempDir;

    // https://www.rfc-editor.org/rfc/rfc8037.html#appendix-A.2
    use super::*;
    use crate::token_source::self_signed::{Builder, StandardClaims};

    static PRIVATE_KEY: &str = "-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIDjobHKy/8ilexeTOjo5if01J1l1vlNfc96WvzpgGddp
-----END PRIVATE KEY-----";

    // For reference:
    //"-----BEGIN PUBLIC KEY-----
    //MCowBQYDK2VwAyEAZbfwp1WfVrpr9aRdUwHD2aWZyAYc9ElOkOqq1MZzoyo=
    //-----END PUBLIC KEY-----";

    macro_rules! now {
        () => {
            chrono::Utc::now().timestamp() as u64
        };
    }

    fn generate_token(claims: StandardClaims, set_kid: bool) -> Token {
        Builder::new_with_ed25519_private_key(PRIVATE_KEY.as_bytes())
            .with_kid(set_kid)
            .build()
            .unwrap()
            .generate(claims)
            .expect("Expected to get a token")
    }

    fn generate_jwks(jwks: &JwkSet, filename: &str) -> TempDir {
        let f = tempfile::Builder::new()
            .prefix("firm-libauth-jwks-")
            .tempdir()
            .expect("Expected to be able to create temp dir");

        serde_json::to_writer_pretty(
            std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(f.path().join(filename))
                .unwrap_or_else(|_| {
                    panic!(
                        "Expected to be able to open temp file at {} for writing",
                        f.path().join(filename).display()
                    )
                }),
            jwks,
        )
        .expect("Expected to be able to serialize jwks to file");

        f
    }

    #[tokio::test]
    async fn interactive_approval() {
        let jwks_dir = generate_jwks(
            &JwkSet {
                keys: vec![Jwk {
                    common: jsonwebtoken::jwk::CommonParameters::default(),
                    algorithm: jsonwebtoken::jwk::AlgorithmParameters::OctetKeyPair(
                        jsonwebtoken::jwk::OctetKeyPairParameters {
                            key_type: jsonwebtoken::jwk::OctetKeyPairType::OctetKeyPair,
                            curve: jsonwebtoken::jwk::EllipticCurve::Ed25519,
                            x: String::from("Zbfwp1WfVrpr9aRdUwHD2aWZyAYc9ElOkOqq1MZzoyo"),
                        },
                    ),
                }],
            },
            "sune.json",
        );

        let mut authenticator = TokenAuthenticator::<InteractiveApproval>::new_trusted_jwks(
            &[format!(
                "file://{}/{{{{subject}}}}.json",
                jwks_dir.path().display()
            )],
            &[] as &[&str],
            &["other_user@other-dator"],
            &[jsonwebtoken::Algorithm::EdDSA],
        );

        let res = authenticator
            .authenticate(generate_token(
                StandardClaims {
                    iss: Some(String::from("sune@sunes-dator")),
                    sub: Some(String::from("sune")),
                    aud: Some(String::from("other_user@other-dator")),
                    exp: now!() + 120,
                    ..Default::default()
                },
                false,
            ))
            .await
            .unwrap();
        {
            let request = authenticator.get_request(&res).unwrap();
            assert!(!request.allowed(now!()));
            assert!(request.needs_approval());
        }

        authenticator.approve_request(&res);

        {
            let request = authenticator.get_request(&res).unwrap();
            assert!(request.allowed(now!()));
            assert!(!request.needs_approval());
        }

        // test that it works when sune is pre-approved
        let mut authenticator = TokenAuthenticator::<InteractiveApproval>::new_trusted_jwks(
            &[format!(
                "file://{}/{{{{subject}}}}.json",
                jwks_dir.path().display()
            )],
            &["sune"],
            &["other_user@other-dator"],
            &[jsonwebtoken::Algorithm::EdDSA],
        );
        let res = authenticator
            .authenticate(generate_token(
                StandardClaims {
                    iss: Some(String::from("sune@sunes-dator")),
                    sub: Some(String::from("sune")),
                    aud: Some(String::from("other_user@other-dator")),
                    exp: now!() + 120,
                    ..Default::default()
                },
                false,
            ))
            .await
            .unwrap();
        {
            let request = authenticator.get_request(&res).unwrap();
            assert!(request.allowed(now!()));
            assert!(!request.needs_approval());
        }

        // create one, deny it, then create another
        let mut authenticator = TokenAuthenticator::<InteractiveApproval>::new_trusted_jwks(
            &[format!(
                "file://{}/{{{{subject}}}}.json",
                jwks_dir.path().display()
            )],
            &[] as &[&str],
            &["other_user@other-dator"],
            &[jsonwebtoken::Algorithm::EdDSA],
        );

        let res = authenticator
            .authenticate(generate_token(
                StandardClaims {
                    iss: Some(String::from("sune@sunes-dator")),
                    sub: Some(String::from("sune")),
                    aud: Some(String::from("other_user@other-dator")),
                    exp: now!() + 120,
                    ..Default::default()
                },
                false,
            ))
            .await
            .unwrap();
        {
            let request = authenticator.get_request(&res).unwrap();
            assert!(!request.allowed(now!()));
            assert!(request.needs_approval());
        }

        authenticator.deny_request(&res);

        {
            let request = authenticator.get_request(&res).unwrap();
            assert!(!request.allowed(now!()));
            assert!(!request.needs_approval());
        }

        let res2 = authenticator
            .authenticate(generate_token(
                StandardClaims {
                    iss: Some(String::from("sune@sunes-dator")),
                    sub: Some(String::from("sune")),
                    aud: Some(String::from("other_user@other-dator")),
                    exp: now!() + 3600,
                    ..Default::default()
                },
                false,
            ))
            .await
            .unwrap();

        assert_ne!(
            res, res2,
            "Expected a new request when authenticating with a new token"
        );

        {
            let request = authenticator.get_request(&res2).unwrap();
            assert!(
                !request.allowed(now!()),
                "Expected a new request for a previously denied subject to not be pre-approved"
            );
            assert!(
                request.needs_approval(),
                "Expected a new request for a previously denied subject to need approval"
            );
        }

        // reusing token
        let mut authenticator = TokenAuthenticator::<InteractiveApproval>::new_trusted_jwks(
            &[format!(
                "file://{}/{{{{subject}}}}.json",
                jwks_dir.path().display()
            )],
            &[] as &[&str],
            &["other_user@other-dator"],
            &[jsonwebtoken::Algorithm::EdDSA],
        );

        let token = generate_token(
            StandardClaims {
                iss: Some(String::from("sune@sunes-dator")),
                sub: Some(String::from("sune")),
                aud: Some(String::from("other_user@other-dator")),
                exp: now!() + 360,
                ..Default::default()
            },
            false,
        );

        let res = authenticator.authenticate(token.clone()).await.unwrap();
        {
            let request = authenticator.get_request(&res).unwrap();
            assert!(!request.allowed(now!()));
            assert!(request.needs_approval());
        }

        authenticator.approve_request(&res);

        let res2 = authenticator.authenticate(token).await.unwrap();
        assert_eq!(res, res2);

        {
            let request = authenticator.get_request(&res).unwrap();
            assert!(
                request.allowed(now!()),
                "Expected request to be approved now"
            );
            assert!(
                !request.needs_approval(),
                "Expected request to be approved after approving"
            );

            assert!(
                !request.allowed(now!() + 365),
                "Expected request to have an expiry"
            );
        }
    }

    #[tokio::test]
    async fn preconfigured_approval() {
        let jwks_dir = generate_jwks(
            &JwkSet {
                keys: vec![Jwk {
                    common: jsonwebtoken::jwk::CommonParameters::default(),
                    algorithm: jsonwebtoken::jwk::AlgorithmParameters::OctetKeyPair(
                        jsonwebtoken::jwk::OctetKeyPairParameters {
                            key_type: jsonwebtoken::jwk::OctetKeyPairType::OctetKeyPair,
                            curve: jsonwebtoken::jwk::EllipticCurve::Ed25519,
                            x: String::from("Zbfwp1WfVrpr9aRdUwHD2aWZyAYc9ElOkOqq1MZzoyo"),
                        },
                    ),
                }],
            },
            "sune.json",
        );

        let mut authenticator = TokenAuthenticator::<PreconfiguredApproval>::new_trusted_jwks(
            &[format!(
                "file://{}/{{{{subject}}}}.json",
                jwks_dir.path().display()
            )],
            &["sune"],
            &["other_user@other-dator"],
            &[jsonwebtoken::Algorithm::EdDSA],
        );

        let res = authenticator
            .authenticate(generate_token(
                StandardClaims {
                    iss: Some(String::from("sune@sunes-dator")),
                    sub: Some(String::from("sune")),
                    aud: Some(String::from("other_user@other-dator")),
                    exp: now!() + 120,
                    ..Default::default()
                },
                false,
            ))
            .await
            .unwrap();
        {
            let request = authenticator.get_request(&res).unwrap();
            assert!(request.allowed(now!()));
            assert!(!request.needs_approval());
            assert!(!request.expired(now!()))
        }
    }
}
