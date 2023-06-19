use std::{collections::HashMap, marker::PhantomData};

use futures::TryFutureExt;

pub mod token;
pub mod token_source;
use serde::Deserialize;
use token::Jwks;
pub use token::Token;

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

pub struct TokenStore {
    providers: ProviderMap,
}

pub struct ProviderId {
    aliases: Vec<String>,
}

pub struct ProviderMap {
    ids: Vec<(ProviderId, Box<dyn TokenProvider>)>,
}

impl ProviderMap {
    pub fn get(&self, scope: &str) -> Option<&dyn TokenProvider> {
        self.ids
            .iter()
            .find(|(id, _)| id.aliases.iter().any(|s| s == scope))
            .map(|(_, provider)| provider.as_ref())
    }

    pub fn get_mut(&mut self, scope: &str) -> Option<&mut (dyn TokenProvider + 'static)> {
        self.ids
            .iter_mut()
            .find(|(id, _)| id.aliases.iter().any(|s| s == scope))
            .map(|(_, provider)| provider.as_mut())
    }

    pub fn insert(&mut self, scope: &str, provider: Box<dyn TokenProvider>) {
        if !self
            .ids
            .iter()
            .flat_map(|(id, _)| id.aliases.iter())
            .any(|a| a == scope)
        {
            self.ids.push((
                ProviderId {
                    aliases: vec![scope.to_owned()],
                },
                provider,
            ));
        }
    }

    pub fn insert_alias(&mut self, scope: &str, alias_for: &str) {
        if let Some((ref mut id, _)) = self
            .ids
            .iter_mut()
            .find(|(id, _)| id.aliases.iter().any(|s| s == scope))
        {
            id.aliases.push(alias_for.to_owned());
        }
    }
}

impl TokenStore {
    pub fn new(providers: ProviderMap) -> Self {
        Self { providers }
    }

    pub async fn acquire_token(
        &mut self,
        scope: &str,
        credstore: Option<&mut (dyn CredentialStore + Send)>,
    ) -> Result<Token, Box<dyn std::error::Error + Send + Sync + 'static>> {
        futures::future::ready(
            self.providers
                .get_mut(scope)
                .ok_or_else(|| format!("No token provider found for scope {}", scope).into()),
        )
        .and_then(|p| p.acquire_token(credstore))
        .await
    }
}

#[async_trait::async_trait]
pub trait TokenProvider {
    async fn acquire_token(
        &mut self,
        credstore: Option<&mut (dyn CredentialStore + Send)>,
    ) -> Result<Token, Box<dyn std::error::Error + Send + Sync + 'static>>;
}

#[derive(thiserror::Error, Debug)]
pub enum TokenAuthenticatorError {
    #[error("Invalid token: {0}")]
    InvalidToken(#[source] token::TokenError),

    // TODO: We may not want to be this clear about the error.
    #[error("Invalid subject: {0}")]
    InvalidSubject(String),
}

pub struct TokenAuthenticator<ApprovalMethod: TokenAuthenticatorApproval> {
    allowed_subjects: Vec<String>,
    allowed_key_sources: Vec<Jwks>,
    requests: HashMap<AccessRequestId, AccessRequest>,
    marker: PhantomData<ApprovalMethod>,
    allowed_issuers: Vec<String>,
    allowed_audiences: Vec<String>,
}

pub trait TokenAuthenticatorApproval {}
pub struct InteractiveApproval {}
pub struct PreconfiguredApproval {}

impl TokenAuthenticatorApproval for InteractiveApproval {}
impl TokenAuthenticatorApproval for PreconfiguredApproval {}

impl<T: TokenAuthenticatorApproval> TokenAuthenticator<T> {
    pub fn new<S1: AsRef<str>, S2: AsRef<str>>(
        allowed_subjects: &[String],
        allowed_key_sources: &[Jwks],
        allowed_issuers: &[S1],
        allowed_audiences: &[S2],
    ) -> Self {
        Self {
            allowed_subjects: allowed_subjects.to_vec(),
            allowed_key_sources: allowed_key_sources.to_vec(),
            requests: HashMap::new(),
            marker: PhantomData,
            allowed_issuers: allowed_issuers
                .iter()
                .map(|s| s.as_ref().to_owned())
                .collect(),
            allowed_audiences: allowed_audiences
                .iter()
                .map(|s| s.as_ref().to_owned())
                .collect(),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct AccessRequestId(uuid::Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalState {
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
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AccessRequest {
    id: AccessRequestId,
    subject: String,
    state: ApprovalState,
}

impl AccessRequest {
    fn new(subject: String, state: ApprovalState) -> Self {
        Self {
            id: AccessRequestId(uuid::Uuid::new_v4()),
            subject,
            state,
        }
    }

    pub fn expired(&self, now: u64) -> bool {
        match self.state {
            ApprovalState::Denied => true,
            ApprovalState::Approved => false,
            ApprovalState::ApprovedExpiresAt(exp) => exp < now,
            ApprovalState::Pending(exp) => exp < now,
        }
    }

    pub fn needs_approval(&self) -> bool {
        !self.state.done()
    }

    pub fn allowed(&self) -> bool {
        self.state.allowed(chrono::Utc::now().timestamp() as u64)
    }

    pub fn subject(&self) -> &str {
        &self.subject
    }

    pub fn id(&self) -> AccessRequestId {
        self.id
    }
}

#[derive(Deserialize)]
struct RequiredClaims {
    sub: String,
    exp: Option<u64>,
}

impl<T: TokenAuthenticatorApproval> TokenAuthenticator<T> {
    pub fn requests(&self) -> impl Iterator<Item = &AccessRequest> {
        let now = chrono::Utc::now().timestamp() as u64;
        self.requests.values().filter(move |x| !x.expired(now))
    }

    pub fn get_request(&self, id: AccessRequestId) -> Option<&AccessRequest> {
        self.requests.get(&id)
    }

    pub fn approve_request(&mut self, id: AccessRequestId) {
        if let Some(req) = self.requests.get_mut(&id) {
            req.state = match req.state {
                ApprovalState::Pending(exp) => ApprovalState::ApprovedExpiresAt(exp),
                s => s,
            }
        }
    }

    pub fn deny_request(&mut self, id: AccessRequestId) {
        if let Some(req) = self.requests.get_mut(&id) {
            req.state = ApprovalState::Denied
        }
    }

    pub fn clear_expired_requests(&mut self) {
        let now = chrono::Utc::now().timestamp() as u64;
        self.requests.retain(move |_, x| !x.expired(now))
    }

    pub fn pending_requests(&self) -> Vec<&AccessRequest> {
        self.requests
            .values()
            .filter(|req| matches!(req.state, ApprovalState::Pending(_)))
            .collect()
    }

    pub fn requests_for_subject(&self, sub: &str) -> Vec<&AccessRequest> {
        self.requests
            .values()
            .filter(|req| req.subject == sub)
            .collect()
    }

    fn insert_request(&mut self, req: AccessRequest) -> AccessRequestId {
        let id = req.id;
        self.requests.insert(id, req);
        id
    }

    async fn validate_token(
        &self,
        token: Token,
    ) -> Result<RequiredClaims, TokenAuthenticatorError> {
        token
            .validate(
                // validate token
                &self.allowed_key_sources,
                token::ExpectedClaims {
                    iss: self
                        .allowed_issuers
                        .iter()
                        .map(AsRef::as_ref)
                        .collect::<Vec<_>>()
                        .as_slice(),
                    aud: self
                        .allowed_audiences
                        .iter()
                        .map(AsRef::as_ref)
                        .collect::<Vec<_>>()
                        .as_slice(),
                    sub: None,
                    alg: &[jsonwebtoken::Algorithm::EdDSA],
                },
            )
            .map_err(TokenAuthenticatorError::InvalidToken)
            .await
    }
}

impl TokenAuthenticator<PreconfiguredApproval> {
    pub async fn authenticate(
        &mut self,
        token: Token,
    ) -> Result<AccessRequestId, TokenAuthenticatorError> {
        self.validate_token(token)
            .await
            .map(
                |claims: RequiredClaims| match self.requests_for_subject(&claims.sub).first() {
                    Some(req) => req.id,
                    None => self.insert_request(AccessRequest::new(
                        claims.sub.clone(),
                        if self.allowed_subjects.contains(&claims.sub) {
                            ApprovalState::Approved
                        } else {
                            ApprovalState::Denied
                        },
                    )),
                },
            )
    }
}

impl TokenAuthenticator<InteractiveApproval> {
    pub async fn authenticate(
        &mut self,
        token: Token,
    ) -> Result<AccessRequestId, TokenAuthenticatorError> {
        fn default_expiry_date() -> u64 {
            const DEFAULT_EXPIRY: u64 = 3600;
            chrono::Utc::now().timestamp() as u64 + DEFAULT_EXPIRY
        }

        let claims = self.validate_token(token).await.and_then(
            |claims: RequiredClaims| -> Result<AccessRequestId, TokenAuthenticatorError> {
                if self.allowed_subjects.contains(&claims.sub) {
                    let req = self
                        .requests_for_subject(&claims.sub)
                        .first()
                        .cloned()
                        .cloned(); // double ref

                    Ok(match req {
                        Some(request) => {
                            if let ApprovalState::Pending(_) = request.state {
                                match self.requests.get_mut(&request.id) {
                                    Some(mutation_request) => {
                                        mutation_request.state = ApprovalState::Pending(
                                            claims.exp.unwrap_or_else(default_expiry_date),
                                        );
                                        request.id
                                    }
                                    None => {
                                        // Weird rugpull case.
                                        self.insert_request(AccessRequest::new(
                                            claims.sub,
                                            ApprovalState::Pending(
                                                claims.exp.unwrap_or_else(default_expiry_date),
                                            ),
                                        ))
                                    }
                                }
                            } else {
                                // Approved, Denied, ApprovedExpiresAt
                                request.id
                            }
                        }
                        None => {
                            // No Access request. Create a pending.
                            self.insert_request(AccessRequest::new(
                                claims.sub,
                                ApprovalState::Pending(
                                    claims.exp.unwrap_or_else(default_expiry_date),
                                ),
                            ))
                        }
                    })
                } else {
                    // Do not need to keep track of it if we are never going to accept it.
                    // The alternative is to keep a bunch of denied reqeusts around.
                    Err(TokenAuthenticatorError::InvalidSubject(claims.sub))
                }
            },
        );
        claims
    }
}

#[cfg(test)]
mod tests {
    use reqwest::Url;

    use super::*;
    #[test]
    fn test_preconfigured_approval() {
        let key_sources = [Jwks::try_new(Url::parse("file://../test_data/jwks").unwrap()).unwrap()];

        let _a = TokenAuthenticator::<PreconfiguredApproval>::new(
            &[String::from("sakarias")],
            &key_sources,
            &["utfärdare"],
            &["publik"],
        );
    }
}
