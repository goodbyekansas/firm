use firm_types::{
    functions::{registry_client::RegistryClient, FunctionId},
    tonic::{
        self,
        codegen::{Body, StdError},
    },
};

use crate::{error, formatting::DisplayExt};
use error::BendiniError;

pub async fn run<T>(
    mut client: RegistryClient<T>,
    function_id: String,
) -> Result<(), error::BendiniError>
where
    T: tonic::client::GrpcService<tonic::body::BoxBody>,
    T::ResponseBody: Body + Send + 'static,
    T::Error: Into<StdError>,
    <T::ResponseBody as Body>::Error: Into<StdError> + Send,
{
    let (function_name, function_version): (&str, &str) =
        match &function_id.splitn(2, ':').collect::<Vec<&str>>()[..] {
            [name, version] => Ok((*name, *version)),
            _ => Err(BendiniError::FailedToParseFunction(function_id)),
        }?;

    let response = client
        .get(tonic::Request::new(FunctionId {
            name: function_name.to_owned(),
            version: function_version.to_owned(),
        }))
        .await?;

    println!("{}", response.into_inner().display());
    Ok(())
}
