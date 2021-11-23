use std::collections::HashMap;

use firm_types::{
    functions::{registry_client::RegistryClient, Filters, Ordering, OrderingKey},
    tonic::{
        self,
        codegen::{Body, StdError},
    },
};

use crate::{error, formatting::DisplayExt};

pub async fn run<T>(mut client: RegistryClient<T>) -> Result<(), error::BendiniError>
where
    T: tonic::client::GrpcService<tonic::body::BoxBody>,
    T::ResponseBody: Body + Send + 'static,
    T::Error: Into<StdError>,
    <T::ResponseBody as Body>::Error: Into<StdError> + Send,
{
    println!("Listing functions");
    let list_request = Filters {
        name: None,
        metadata: HashMap::new(),
        order: Some(Ordering {
            limit: 25,
            offset: 0,
            reverse: false,
            key: OrderingKey::NameVersion as i32,
        }),
        version_requirement: None,
    };

    let list_response = client.list(tonic::Request::new(list_request)).await?;

    list_response
        .into_inner()
        .functions
        .into_iter()
        .for_each(|f| println!("{}", f.display()));

    Ok(())
}
