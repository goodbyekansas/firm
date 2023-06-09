#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum AuthConfig {
    Oidc {
        provider: String,
    },
    SelfSigned,
    KeyFile {
        /// Path to the private key used to sign the JWT
        path: PathBuf,

        /// Override iss on JWT
        iss: Option<String>,

        /// Override sub on JWT
        sub: Option<String>,

        /// Override aud on JWT
        aud: Option<String>,

        /// Override kid on JWT
        kid: Option<String>,

        /// Override exp on JWT.
        /// Will expire in exp seconds
        exp: Option<usize>,
    },
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum KeyStore {
    Simple { url: String },
    None,
}

impl Default for KeyStore {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct AllowConfig {
    pub users: Vec<String>,
    // TODO for LDAP and google groups: pub groups: Vec<{ name: String, provider: String }>
}

#[derive(Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum IdentityProvider {
    Oidc { provider: String },
    Username,
    UsernameSuffix { suffix: String },
    Override { name: String, email: String },
}

impl Default for IdentityProvider {
    fn default() -> Self {
        Self::Username
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub struct Auth {
    #[serde(default)]
    pub identity: IdentityProvider,

    #[serde(default)]
    pub scopes: HashMap<String, AuthConfig>,

    #[serde(default)]
    pub key_store: KeyStore,

    #[serde(default)]
    pub allow: AllowConfig,
}

// For backwards compatibility, add things to this struct and
// convert it in the try from. TODO: consider removing the
// hosted_domain for next major release.
#[derive(Debug, Deserialize, PartialEq, Clone)]
#[serde(try_from = "OidcProviderConfig")]
pub struct OidcProvider {
    pub discovery_url: String,
    pub client_id: String,
    pub client_secret: String,

    #[serde(default)]
    pub hosted_domains: Vec<String>,
}
#[derive(Debug, Deserialize, PartialEq, Clone)]
pub struct OidcProviderConfig {
    pub discovery_url: String,
    pub client_id: String,
    pub client_secret: String,

    #[serde(default)]
    pub hosted_domain: Option<String>,
    #[serde(default)]
    pub hosted_domains: Vec<String>,
}

impl TryFrom<OidcProviderConfig> for OidcProvider {
    type Error = &'static str;

    fn try_from(value: OidcProviderConfig) -> Result<Self, Self::Error> {
        Ok(Self {
            discovery_url: value.discovery_url,
            client_id: value.client_id,
            client_secret: value.client_secret,
            hosted_domains: value
                .hosted_domain
                .map(|hd| vec![hd])
                .unwrap_or_default()
                .into_iter()
                .chain(value.hosted_domains)
                .collect(),
        })
    }
}
