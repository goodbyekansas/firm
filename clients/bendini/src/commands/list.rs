use firm_types::{
    functions::{registry_client::RegistryClient, Filters, Ordering, OrderingKey},
    tonic::{
        self,
        codegen::{Body, StdError},
    },
};

use crate::{
    error,
    formatting::{self, DisplayExt},
};

pub async fn functions<T>(
    mut client: RegistryClient<T>,
    format: formatting::DisplayFormat,
) -> Result<(), error::BendiniError>
where
    T: tonic::client::GrpcService<tonic::body::BoxBody>,
    T::ResponseBody: Body + Send + 'static,
    T::Error: Into<StdError>,
    <T::ResponseBody as Body>::Error: Into<StdError> + Send,
{
    let list_response = client
        .list(tonic::Request::new(Filters {
            order: Some(Ordering {
                limit: 25,
                offset: 0,
                reverse: false,
                key: OrderingKey::NameVersion as i32,
            }),
            ..Default::default()
        }))
        .await?;

    println!("{}", list_response.into_inner().display_format(format));
    Ok(())
}

pub async fn versions<T>(
    mut client: RegistryClient<T>,
    name: &str,
    format: formatting::DisplayFormat,
) -> Result<(), error::BendiniError>
where
    T: tonic::client::GrpcService<tonic::body::BoxBody>,
    T::ResponseBody: Body + Send + 'static,
    T::Error: Into<StdError>,
    <T::ResponseBody as Body>::Error: Into<StdError> + Send,
{
    let list_response = client
        .list_versions(tonic::Request::new(Filters {
            name: name.to_string(),
            order: Some(Ordering {
                limit: 25,
                offset: 0,
                reverse: false,
                key: OrderingKey::NameVersion as i32,
            }),
            ..Default::default()
        }))
        .await?;

    println!("{}", list_response.into_inner().display_format(format));
    Ok(())
}
