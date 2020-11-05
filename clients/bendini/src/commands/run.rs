use std::collections::HashMap;

use firm_types::{
    execution::{execution_client::ExecutionClient, ExecutionParameters, Stream},
    functions::{ChannelSpec, ChannelType},
    registry::{
        registry_client::RegistryClient, Filters, NameFilter, Ordering, OrderingKey,
        VersionRequirement,
    },
    stream::ToChannel,
    tonic::{self, transport::Channel},
};
use tonic_middleware::HttpStatusInterceptor;

use crate::error;
use error::BendiniError;

// TODO this can be more general and move to firm_types
fn parse_arguments<'a, I>(
    inputs: I,
    arguments: Vec<(String, String)>,
) -> Result<Stream, Vec<String>>
where
    I: Iterator<Item = (&'a String, &'a ChannelSpec)>,
{
    let inputs = inputs.collect::<HashMap<_, _>>();
    let (values, errors): (Vec<_>, Vec<_>) = arguments
        .iter()
        .map(|(key, val)| {
            inputs
                .get(key)
                .ok_or(format!("argument {} is not expected.", key))
                .and_then(|input| {
                    let parsed_type = ChannelType::from_i32(input.r#type).ok_or(format!(
                        "argument type {} is out of range (out of date protobuf definitions?)",
                        input.r#type
                    ))?;
                    match parsed_type {
                        ChannelType::String => Ok(val.clone().to_channel()),
                        ChannelType::Bool => val
                            .parse::<bool>()
                            .map_err(|e| {
                                format!("cant parse argument {} into bool value. err: {}", key, e)
                            })
                            .map(|x| x.to_channel()),
                        ChannelType::Int => val
                            .parse::<i64>()
                            .map_err(|e| {
                                format!("cant parse argument {} into int value. err: {}", key, e)
                            })
                            .map(|x| x.to_channel()),
                        ChannelType::Float => val
                            .parse::<f32>()
                            .map_err(|e| {
                                format!("cant parse argument {} into float value. err: {}", key, e)
                            })
                            .map(|x| x.to_channel()),
                        ChannelType::Bytes => Ok(val.as_bytes().to_vec().to_channel()),
                    }
                    .map(|channel| (key.to_owned(), channel))
                })
        })
        .partition(Result::is_ok);

    if !errors.is_empty() {
        Err(errors.into_iter().map(Result::unwrap_err).collect())
    } else {
        Ok(Stream {
            channels: values.into_iter().map(Result::unwrap).collect(),
        })
    }
}

pub async fn run(
    mut registry_client: RegistryClient<HttpStatusInterceptor>,
    mut execution_client: ExecutionClient<Channel>,
    function_id: String,
    arguments: Vec<(String, String)>,
) -> Result<(), BendiniError> {
    let (function_name, function_version): (&str, &str) =
        match &function_id.splitn(2, ':').collect::<Vec<&str>>()[..] {
            [name, version] => Ok((*name, *version)),
            [name] => Ok((*name, "*")),
            _ => Err(BendiniError::FailedToParseFunction(function_id)),
        }?;

    let input_values = registry_client
        .list(tonic::Request::new(Filters {
            name: Some(NameFilter {
                pattern: function_name.to_owned(),
                exact_match: true,
            }),
            version_requirement: Some(VersionRequirement {
                expression: function_version.to_owned(),
            }),
            metadata: HashMap::new(),
            order: Some(Ordering {
                offset: 0,
                limit: 1,
                reverse: false,
                key: OrderingKey::NameVersion as i32,
            }),
        }))
        .await?
        .into_inner()
        .functions
        .first()
        .ok_or_else(|| BendiniError::FailedToFindFunction {
            name: function_name.to_owned(),
            version: function_version.to_owned(),
        })
        .and_then(|f| {
            parse_arguments(
                f.required_inputs.iter().chain(f.optional_inputs.iter()),
                arguments,
            )
            .map_err(|e| BendiniError::InvalidFunctionArguments(f.name.clone(), e))
        })?;

    println!(
        "Executing function: {}:{}",
        &function_name, &function_version
    );

    execution_client
        .execute_function(tonic::Request::new(ExecutionParameters {
            name: function_name.to_owned(),
            version_requirement: function_version.to_owned(),
            arguments: Some(input_values),
        }))
        .await
        .map_err(|e| e.into())
        .map(|r| println!("{:#?}", r.into_inner()))
}
