use firm_types::{
    functions::execution_client::ExecutionClient,
    functions::RuntimeFilters,
    tonic::{
        self,
        codegen::{Body, StdError},
    },
};

use crate::{error, formatting::DisplayExt};

pub async fn run<T>(mut client: ExecutionClient<T>, name: String) -> Result<(), error::BendiniError>
where
    T: tonic::client::GrpcService<tonic::body::BoxBody>,
    T::ResponseBody: Body + Send + 'static,
    T::Error: Into<StdError>,
    <T::ResponseBody as Body>::Error: Into<StdError> + Send,
{
    println!("Listing runtimes");
    let list_request = RuntimeFilters { name };

    let list_response = client
        .list_runtimes(tonic::Request::new(list_request))
        .await?;

    list_response
        .into_inner()
        .runtimes
        .into_iter()
        .for_each(|f| println!("{}", f.display()));

    Ok(())
}
