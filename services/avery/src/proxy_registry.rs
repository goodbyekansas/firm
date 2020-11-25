use url::Url;

use crate::registry::RegistryService;
use firm_types::{functions::registry_server::Registry, tonic};
#[derive(Debug, Clone)]
pub struct ProxyRegistry {
    _external_registries: Vec<ExternalRegistry>, // TODO add connection pooling
    internal_registry: RegistryService,
}
#[derive(Debug, Clone)]
pub struct ExternalRegistry {
    _url: Url,
    _oauth_token: Option<String>,
}

impl ExternalRegistry {
    pub fn new(url: Url) -> Self {
        Self {
            _url: url,
            _oauth_token: None,
        }
    }
    pub fn new_with_oauth(url: Url, oauth_token: String) -> Self {
        Self {
            _url: url,
            _oauth_token: Some(oauth_token),
        }
    }
}

impl ProxyRegistry {
    pub fn new(
        external_registries: Vec<ExternalRegistry>,
        internal_registry: RegistryService,
    ) -> Self {
        Self {
            _external_registries: external_registries,
            internal_registry,
        }
    }
}

#[tonic::async_trait]
impl Registry for ProxyRegistry {
    async fn list(
        &self,
        request: firm_types::tonic::Request<firm_types::functions::Filters>,
    ) -> Result<
        firm_types::tonic::Response<firm_types::functions::Functions>,
        firm_types::tonic::Status,
    > {
        self.internal_registry.list(request).await
    }

    async fn get(
        &self,
        request: firm_types::tonic::Request<firm_types::functions::FunctionId>,
    ) -> Result<
        firm_types::tonic::Response<firm_types::functions::Function>,
        firm_types::tonic::Status,
    > {
        self.internal_registry.get(request).await
    }

    async fn register(
        &self,
        request: firm_types::tonic::Request<firm_types::functions::FunctionData>,
    ) -> Result<
        firm_types::tonic::Response<firm_types::functions::Function>,
        firm_types::tonic::Status,
    > {
        self.internal_registry.register(request).await
    }

    async fn register_attachment(
        &self,
        request: firm_types::tonic::Request<firm_types::functions::AttachmentData>,
    ) -> Result<
        firm_types::tonic::Response<firm_types::functions::AttachmentHandle>,
        firm_types::tonic::Status,
    > {
        self.internal_registry.register_attachment(request).await
    }

    async fn upload_streamed_attachment(
        &self,
        request: firm_types::tonic::Request<
            firm_types::tonic::Streaming<firm_types::functions::AttachmentStreamUpload>,
        >,
    ) -> Result<
        firm_types::tonic::Response<firm_types::functions::Nothing>,
        firm_types::tonic::Status,
    > {
        self.internal_registry
            .upload_streamed_attachment(request)
            .await
    }
}
