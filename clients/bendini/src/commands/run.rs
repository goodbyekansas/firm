use std::collections::HashMap;

use firm_types::{
    functions::{
        execution_client::ExecutionClient, registry_client::RegistryClient, ChannelSpec,
        ChannelType, ExecutionParameters, Filters, NameFilter, Ordering, OrderingKey, Stream,
        VersionRequirement,
    },
    stream::ToChannel,
    tonic,
};
use futures::{join, FutureExt, StreamExt, TryFutureExt};
use tonic_middleware::HttpStatusInterceptor;

use crate::{error, formatting::DisplayExt};
use error::BendiniError;
use regex::Regex;

fn argument_to_list(arg: &str) -> Vec<String> {
    // Unwrap is ok here since the regex is not dynamic so the
    // result will stay the same if the regex does
    let list_regex: Regex = Regex::new(r#"^\s*\[.*]\s*$"#).unwrap();
    if list_regex.is_match(arg) {
        let arg_regex =
            Regex::new(r#"(?P<match1>[\p{Emoji}\w.-]+)|"(?P<match2>[\p{Emoji}'\w\s.-]*)"|'(?P<match3>[\p{Emoji}\w\s.-]*)'"#).unwrap();
        arg_regex
            .captures_iter(arg)
            .filter_map(|capture| {
                capture
                    .name("match1")
                    .or_else(|| capture.name("match2"))
                    .or_else(|| capture.name("match3"))
                    .map(|m| m.as_str().to_owned())
            })
            .collect()
    } else {
        let pat: &[_] = &['"', '\''];
        vec![arg.trim_matches(pat).to_owned()]
    }
}

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
                    let val = argument_to_list(val);
                    Ok(match parsed_type {
                        ChannelType::String => val.to_channel(),
                        ChannelType::Bool => val
                            .into_iter()
                            .map(|b| {
                                b.parse::<bool>().map_err(|e| {
                                    format!(
                                        "Failed to parse {} (argument {}) into bool value. err: {}",
                                        b, key, e
                                    )
                                })
                            })
                            .collect::<Result<Vec<bool>, _>>()?
                            .to_channel(),
                        ChannelType::Int => val
                            .into_iter()
                            .map(|i| {
                                i.parse::<i64>().map_err(|e| {
                                    format!(
                                        "Failed to parse {} (argument {}) into int value. err: {}",
                                        i, key, e
                                    )
                                })
                            })
                            .collect::<Result<Vec<i64>, _>>()?
                            .to_channel(),
                        ChannelType::Float => val
                            .into_iter()
                            .map(|f| {
                                f.parse::<f64>().map_err(|e| {
                                    format!("Failed to parse {} (argument {}) into float value. err: {}", f, key, e)
                                })
                            })
                            .collect::<Result<Vec<f64>, _>>()?
                            .to_channel(),
                        ChannelType::Bytes => val
                            .into_iter()
                            .map(|b| {
                                b.parse::<u8>().map_err(|e| {
                                    format!("Failed to parse {} (argument {}) into byte value. err: {}", b, key, e)
                                })
                            })
                            .collect::<Result<Vec<u8>, _>>()?
                            .to_channel(),
                    })
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
    mut execution_client: ExecutionClient<HttpStatusInterceptor>,
    function_id: String,
    arguments: Vec<(String, String)>,
    follow_output: bool,
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

    let execution_id = execution_client
        .queue_function(tonic::Request::new(ExecutionParameters {
            name: function_name.to_owned(),
            version_requirement: function_version.to_owned(),
            arguments: Some(input_values),
        }))
        .await
        .map_err(BendiniError::from)?
        .into_inner();

    let mut outputs: HashMap<String, BufferedChannelPrinter> = HashMap::new();

    let follow_future = if follow_output {
        execution_client
            .function_output(execution_id.clone())
            .await
            .map_err(BendiniError::from)?
            .into_inner()
            .for_each(|chunk| {
                if let Ok(c) = chunk {
                    outputs
                        .entry(c.channel.clone())
                        .or_insert_with(|| BufferedChannelPrinter {
                            buffer: String::new(),
                            channel: c.channel.clone(),
                        })
                        .push(&c.output);
                };
                futures::future::ready(())
            })
            .boxed()
    } else {
        futures::future::ready(()).boxed()
    };

    let (_, res) = join!(
        follow_future,
        execution_client
            .run_function(execution_id)
            .map_err(BendiniError::from),
    );

    // Explicit memory drop to ensure that output gets flushed before printing the result.
    std::mem::drop(outputs);
    res.map(|r| println!("{}", r.into_inner().display()))
}

struct BufferedChannelPrinter {
    buffer: String,
    channel: String,
}

impl BufferedChannelPrinter {
    pub fn push(&mut self, content: &str) {
        self.buffer.push_str(content);
        let split = self.buffer.rsplitn(2, '\n').collect::<Vec<&str>>();

        if split.len() > 1 {
            println!(
                "[{}] {}",
                ansi_term::Colour::White.bold().paint(&self.channel),
                split[1]
            );
        }

        self.buffer = split[0].to_owned();
    }
}

impl Drop for BufferedChannelPrinter {
    fn drop(&mut self) {
        if !self.buffer.is_empty() {
            println!(
                "[{}] {}",
                ansi_term::Colour::White.bold().paint(&self.channel),
                self.buffer
            );
        }
    }
}

#[cfg(test)]
mod tests {

    use std::collections::HashMap;

    use super::{argument_to_list, parse_arguments};

    use firm_types::{
        channel_specs,
        functions::ChannelSpec,
        functions::ChannelType,
        stream::{StreamExt, TryFromChannel},
    };

    #[test]
    fn arg_to_list() {
        // command line: "[hej hej hej]"
        assert_eq!(
            argument_to_list("[hej hej hej]"),
            vec!["hej".to_owned(), "hej".to_owned(), "hej".to_owned()]
        );

        // command line: "[\"hej svejs\" svejs]"
        assert_eq!(
            argument_to_list(r#"["hej svejs" svejs]"#),
            vec!["hej svejs".to_owned(), "svejs".to_owned()]
        );

        // command line: "[ \"nu.du-hej hej\" nej]"
        assert_eq!(
            argument_to_list(r#"[ "nu.du-hej hej" nej]"#),
            vec!["nu.du-hej hej".to_owned(), "nej".to_owned()]
        );

        // command line: "[🐒 4]"
        assert_eq!(
            argument_to_list(r#"[🐒 4]"#),
            vec!["🐒".to_owned(), "4".to_owned()]
        );

        // command line: "\"this is \"not\" a list\""
        assert_eq!(
            argument_to_list(r#""this is "not" a list""#),
            vec!["this is \"not\" a list".to_owned()]
        );
    }

    #[test]
    fn parse_args() {
        let (required, optional): (HashMap<String, ChannelSpec>, _) = channel_specs!({

            // Strings
            "arg1" => ChannelSpec {
                description: String::new(),
                r#type: ChannelType::String as i32,
            },
            "arg2" => ChannelSpec {
                description: String::new(),
                r#type: ChannelType::String as i32,
            },

            // Ints
            "arg3" => ChannelSpec {
                description: String::new(),
                r#type: ChannelType::Int as i32,
            },
            "arg4" => ChannelSpec {
                description: String::new(),
                r#type: ChannelType::Int as i32,
            },

            // Bool
            "arg5" => ChannelSpec {
                description: String::new(),
                r#type: ChannelType::Bool as i32,
            },
            "arg6" => ChannelSpec {
                description: String::new(),
                r#type: ChannelType::Bool as i32,
            },

            // Bytes
            "arg7" => ChannelSpec {
                description: String::new(),
                r#type: ChannelType::Bytes as i32,
            },
            "arg8" => ChannelSpec {
                description: String::new(),
                r#type: ChannelType::Bytes as i32,
            },
            // Float
            "arg9" => ChannelSpec {
                description: String::new(),
                r#type: ChannelType::Float as i32,
            },
            "arg10" => ChannelSpec {
                description: String::new(),
                r#type: ChannelType::Float as i32,
            },
            // Test single fnutt
            "arg11" => ChannelSpec {
                description: String::new(),
                r#type: ChannelType::String as i32,
            },
            // Test fnutt inside double fnutt
            "arg12" => ChannelSpec {
                description: String::new(),
                r#type: ChannelType::String as i32,
            }
        });

        let res = parse_arguments(
            required.iter().chain(optional.unwrap_or_default().iter()),
            vec![
                // Strings
                ("arg1".to_owned(), "this is a string".to_owned()),
                ("arg2".to_owned(), "[this is a list of strings]".to_owned()),
                // Ints
                ("arg3".to_owned(), "1338".to_owned()),
                ("arg4".to_owned(), "[1 10 \"11\" 14]".to_owned()),
                // Bools
                ("arg5".to_owned(), "true".to_owned()),
                ("arg6".to_owned(), "[ true false false true ]".to_owned()),
                // Bytes
                ("arg7".to_owned(), "128".to_owned()),
                ("arg8".to_owned(), "[ 255 0 123 200 ]".to_owned()),
                // Floats
                ("arg9".to_owned(), "1.2341".to_owned()),
                ("arg10".to_owned(), "[ 2.22 3.52 8.45 9.999919 ]".to_owned()),
                // Test single fnutt
                (
                    "arg11".to_owned(),
                    "['this is a list' of strings]".to_owned(),
                ),
                // Test single fnutt inside double fnutt
                ("arg12".to_owned(), "[\"It's a boy\" of strings]".to_owned()),
            ],
        );

        assert!(res.is_ok());
        let stream = res.unwrap();

        // Strings
        assert_eq!(
            stream.get_channel_as_ref::<String>("arg1").unwrap(),
            "this is a string"
        );
        assert_eq!(
            stream.get_channel_as_ref::<[String]>("arg2").unwrap(),
            &["this", "is", "a", "list", "of", "strings"]
        );

        // Ints
        assert_eq!(
            i64::try_from(stream.get_channel("arg3").unwrap()).unwrap(),
            1338
        );
        assert_eq!(
            stream.get_channel_as_ref::<[i64]>("arg4").unwrap(),
            &[1, 10, 11, 14]
        );

        // Bools
        assert_eq!(
            bool::try_from(stream.get_channel("arg5").unwrap()).unwrap(),
            true
        );
        assert_eq!(
            stream.get_channel_as_ref::<[bool]>("arg6").unwrap(),
            &[true, false, false, true],
        );

        // Bytes
        assert_eq!(
            u8::try_from(stream.get_channel("arg7").unwrap()).unwrap(),
            128
        );

        assert_eq!(
            stream.get_channel_as_ref::<[u8]>("arg8").unwrap(),
            &[255, 0, 123, 200]
        );

        // Floats
        assert!(
            (f64::try_from(stream.get_channel("arg9").unwrap()).unwrap() - 1.2341).abs()
                < f64::EPSILON
        );

        let expected = &[2.22, 3.52, 8.45, 9.999919];
        assert!(stream
            .get_channel_as_ref::<[f64]>("arg10")
            .unwrap()
            .iter()
            .enumerate()
            .all(|(i, v)| (v - expected[i]).abs() < f64::EPSILON));

        // Test single fnutt
        assert_eq!(
            stream.get_channel_as_ref::<[String]>("arg11").unwrap(),
            &["this is a list", "of", "strings"]
        );

        // Test single fnutt inside double fnutt
        assert_eq!(
            stream.get_channel_as_ref::<[String]>("arg12").unwrap(),
            &["It's a boy", "of", "strings"]
        );
    }
}
