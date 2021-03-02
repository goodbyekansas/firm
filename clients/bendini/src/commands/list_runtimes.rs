use firm_types::{functions::execution_client::ExecutionClient, functions::RuntimeFilters, tonic};
use tonic::transport::Channel;

use crate::{error, formatting::DisplayExt};

pub async fn run(
    mut client: ExecutionClient<Channel>,
    name: String,
) -> Result<(), error::BendiniError> {
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
