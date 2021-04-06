use firm_types::{
    auth::authentication_server::Authentication, auth::AcquireTokenParameters, auth::Token, tonic,
};

#[derive(Clone, Debug)]
pub struct AuthService {}

#[tonic::async_trait]
impl Authentication for AuthService {
    async fn acquire_token(
        &self,
        _: tonic::Request<AcquireTokenParameters>,
    ) -> Result<tonic::Response<Token>, tonic::Status> {
        // TODO: Actually implement it
        Err(tonic::Status::unimplemented("Nope"))
    }
}
