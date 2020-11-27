use firm_types::{
    functions::{registry_client::RegistryClient, FunctionId},
    tonic,
};
use tonic_middleware::HttpStatusInterceptor;

use crate::{error, formatting::DisplayExt};
use error::BendiniError;

pub async fn run(
    mut client: RegistryClient<HttpStatusInterceptor>,
    function_id: String,
) -> Result<(), error::BendiniError> {
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
