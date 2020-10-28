use std::collections::HashMap;

use firm_protocols::{
    execution::{execution_client::ExecutionClient, ExecutionParameters, InputValue},
    functions::{Input, Type},
    registry::{
        registry_client::RegistryClient, Filters, NameFilter, Ordering, OrderingKey,
        VersionRequirement,
    },
    tonic::{self, transport::Channel},
};
use tonic_middleware::HttpStatusInterceptor;

use crate::error;
use error::BendiniError;

fn validate_arguments(
    inputs: &[Input],
    arguments: Vec<(String, String)>,
) -> Result<Vec<InputValue>, Vec<String>> {
    let (values, errors): (Vec<_>, Vec<_>) = arguments
        .iter()
        .map(|(key, val)| {
            inputs
                .iter()
                .find(|k| &k.name == key)
                .ok_or(format!("argument {} is not expected.", key))
                .and_then(|input| {
                    let parsed_type = Type::from_i32(input.r#type).ok_or(format!(
                        "argument type {} is out of range (out of date protobuf definitions?)",
                        input.r#type
                    ))?;
                    match parsed_type {
                        Type::String => Ok(InputValue {
                            name: key.clone(),
                            r#type: input.r#type,
                            value: val.as_bytes().to_vec(),
                        }),
                        Type::Bool => val
                            .parse::<bool>()
                            .map_err(|e| {
                                format!("cant parse argument {} into bool value. err: {}", key, e)
                            })
                            .map(|x| InputValue {
                                name: key.clone(),
                                r#type: input.r#type,
                                value: vec![x as u8],
                            }),
                        Type::Int => val
                            .parse::<i64>()
                            .map_err(|e| {
                                format!("cant parse argument {} into int value. err: {}", key, e)
                            })
                            .map(|x| InputValue {
                                name: key.clone(),
                                r#type: input.r#type,
                                value: x.to_le_bytes().to_vec(),
                            }),
                        Type::Float => val
                            .parse::<f64>()
                            .map_err(|e| {
                                format!("cant parse argument {} into float value. err: {}", key, e)
                            })
                            .map(|x| InputValue {
                                name: key.clone(),
                                r#type: input.r#type,
                                value: x.to_le_bytes().to_vec(),
                            }),
                        Type::Bytes => Ok(InputValue {
                            name: key.clone(),
                            r#type: input.r#type,
                            value: val.as_bytes().to_vec(),
                        }),
                    }
                })
        })
        .partition(Result::is_ok);

    if !errors.is_empty() {
        Err(errors.into_iter().map(Result::unwrap_err).collect())
    } else {
        Ok(values.into_iter().map(Result::unwrap).collect())
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

    let (function, input_values) = registry_client
        .list(tonic::Request::new(Filters {
            name_filter: Some(NameFilter {
                pattern: function_name.to_owned(),
                exact_match: true,
            }),
            version_requirement: Some(VersionRequirement {
                expression: function_version.to_owned(),
            }),
            metadata_filter: HashMap::new(),
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
            validate_arguments(&f.inputs, arguments)
                .map(|input_values| (f.clone(), input_values))
                .map_err(|e| BendiniError::InvalidFunctionArguments(f.name.clone(), e))
        })?;

    println!("Executing function: {}:{}", function.name, function.version);

    execution_client
        .execute(tonic::Request::new(ExecutionParameters {
            function: Some(function),
            arguments: input_values,
        }))
        .await
        .map_err(|e| e.into())
        .map(|r| println!("{:#?}", r.into_inner()))
}
