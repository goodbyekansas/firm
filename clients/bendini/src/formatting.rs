use std::{
    collections::HashMap,
    fmt::{self, Display},
};

use ansi_term::Colour::Green;
use chrono::{TimeZone, Utc};
use firm_types::{
    auth::RemoteAccessRequest,
    functions::{
        channel::Value, execution_result::Result as FunctionResult, Channel, ChannelSpec,
        ChannelType, ExecutionResult, Function, Functions, Runtime, RuntimeSpec, Stream,
    },
};
use futures::{future::join, Future};
use indicatif::MultiProgress;
use serde::{
    ser::{SerializeSeq, SerializeStruct},
    Serialize, Serializer,
};
use tokio::task;

const INDENT: &str = "  ";

pub struct Displayer<'a, T> {
    display: &'a T,
    format: DisplayFormat,
}

impl<T> std::ops::Deref for Displayer<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.display
    }
}

pub trait DisplayExt<'a, T> {
    fn display(&'a self) -> Displayer<T>;
    fn display_format(&'a self, format: DisplayFormat) -> Displayer<T>;
}

#[derive(Debug, PartialEq, Clone)]
pub enum DisplayFormat {
    Short,
    Long,
    // Full, TODO
    Json,
}

impl Serialize for Displayer<'_, Function> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer
            .serialize_struct("Function", if self.publisher.is_some() { 9 } else { 8 })?;
        state.serialize_field("name", &self.name)?;
        state.serialize_field("version", &self.version)?;
        state.serialize_field(
            "runtime",
            &self
                .runtime
                .as_ref()
                .unwrap_or(&RuntimeSpec {
                    name: "n/a".to_string(),
                    arguments: HashMap::default(),
                    entrypoint: "n/a".to_string(),
                })
                .name,
        )?;
        state.serialize_field(
            "created",
            &Utc.timestamp(self.created_at as i64, 0)
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string(),
        )?;
        state.serialize_field("metadata", &self.metadata)?;
        if let Some(p) = self.publisher.as_ref() {
            state.serialize_field("publisher", &format!("{} <{}>", p.name, p.email))?;
        }
        state.serialize_field(
            "required_inputs",
            &self
                .required_inputs
                .iter()
                .map(|(k, i)| (k.clone(), ChannelSpecSerializer(i.clone())))
                .collect::<HashMap<String, ChannelSpecSerializer>>(),
        )?;
        state.serialize_field(
            "optional_inputs",
            &self
                .optional_inputs
                .iter()
                .map(|(k, i)| (k.clone(), ChannelSpecSerializer(i.clone())))
                .collect::<HashMap<String, ChannelSpecSerializer>>(),
        )?;
        state.serialize_field(
            "outputs",
            &self
                .outputs
                .iter()
                .map(|(k, i)| (k.clone(), ChannelSpecSerializer(i.clone())))
                .collect::<HashMap<String, ChannelSpecSerializer>>(),
        )?;
        state.end()
    }
}

impl Serialize for Displayer<'_, Functions> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(self.functions.len()))?;
        for e in self.functions.iter() {
            SerializeSeq::serialize_element(&mut seq, &e.display())?;
        }
        seq.end()
    }
}

impl<'a, U> DisplayExt<'a, U> for U {
    fn display(&'a self) -> Displayer<U> {
        Displayer {
            display: self,
            format: DisplayFormat::Long,
        }
    }

    fn display_format(&'a self, format: DisplayFormat) -> Displayer<U> {
        Displayer {
            display: self,
            format,
        }
    }
}

#[macro_export]
macro_rules! warn {
    ($($args:tt)*) => {
        ansi_term::Color::Yellow.paint(format!($($args)*))
    };
}

#[macro_export]
macro_rules! error {
    ($($args:tt)*) => {
        ansi_term::Color::Red.paint(format!($($args)*))
    };
}

pub async fn with_progressbars<F, U, R>(function: F) -> R
where
    U: Future<Output = R>,
    F: Fn(&MultiProgress) -> U,
{
    let multi_progress = MultiProgress::new();
    join(
        function(&multi_progress),
        task::spawn_blocking(move || {
            multi_progress.join().map_or_else(
                |e| println!("Failed waiting for progress bar: {:?}", e),
                |_| (),
            )
        }),
    )
    .await
    .0
}

fn fmt_json<T: Serialize>(o: &T, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    if atty::is(atty::Stream::Stdout) {
        serde_json::to_string_pretty(o)
    } else {
        serde_json::to_string(o)
    }
    .map_err(|_| fmt::Error {})
    .and_then(|s| writeln!(f, "{}", s))
}

impl Display for Displayer<'_, Functions> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.format {
            DisplayFormat::Json => fmt_json(self, f),
            _ => self
                .functions
                .iter()
                .try_for_each(|func| writeln!(f, "{}", func.display_format(self.format.clone()))),
        }
    }
}

// impl display of listed functions
impl Display for Displayer<'_, Function> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.format {
            DisplayFormat::Long => {
                writeln!(f, "{}", Green.paint(&self.name))?;
                writeln!(f, "{}version: {}", INDENT, &self.version)?;
                writeln!(
                    f,
                    "{}runtime: {}",
                    INDENT,
                    self.runtime
                        .as_ref()
                        .unwrap_or(&RuntimeSpec {
                            name: "n/a".to_string(),
                            arguments: HashMap::default(),
                            entrypoint: "n/a".to_string(),
                        })
                        .name
                )?;

                writeln!(
                    f,
                    "{}codeUrl: {}",
                    INDENT,
                    self.code
                        .as_ref()
                        .and_then(|c| c.url.clone())
                        .map_or_else(|| String::from("n/a"), |code| code.url)
                )?;

                write!(
                    f,
                    "{}required inputs:{}",
                    INDENT,
                    self.required_inputs.display()
                )?;
                write!(
                    f,
                    "{}optional inputs:{}",
                    INDENT,
                    self.optional_inputs.display()
                )?;
                write!(f, "{}outputs:{}", INDENT, self.outputs.display())?;

                if self.metadata.is_empty() {
                    writeln!(f, "{}metadata: n/a", INDENT)?;
                } else {
                    writeln!(f, "{}metadata:", INDENT)?;
                    self.metadata
                        .clone()
                        .iter()
                        .try_for_each(|(x, y)| writeln!(f, "{}{}: {}", INDENT.repeat(2), x, y))?;
                }

                if let Some(p) = self.publisher.as_ref() {
                    writeln!(f, "{}published by: {} <{}>", INDENT, p.name, p.email)
                } else {
                    Ok(())
                }
            }
            DisplayFormat::Short => {
                writeln!(f, "{}", Green.paint(&self.name))?;
                writeln!(f, "{}version: {}", INDENT, &self.version)
            }
            DisplayFormat::Json => fmt_json(self, f),
        }
    }
}

impl Serialize for ChannelSpecSerializer {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("ChannelSpec", 2)?;
        state.serialize_field("description", &self.0.description)?;
        state.serialize_field("type", &self.0.r#type.display().to_string())?;
        state.end()
    }
}

pub struct ChannelSpecSerializer(ChannelSpec);

impl Display for Displayer<'_, HashMap<String, ChannelSpec>> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.format {
            DisplayFormat::Short => Ok(()),
            DisplayFormat::Long => {
                if self.is_empty() {
                    writeln!(f, " none")
                } else {
                    writeln!(f)?;
                    self.iter().try_for_each(|(name, channel_spec)| {
                        writeln!(
                            f,
                            "{tab}{name}[{type}]{description}",
                            tab = INDENT.repeat(2),
                            r#type = channel_spec.r#type.display(),
                            name = name,
                            description = if channel_spec.description.is_empty() {
                                String::new()
                            } else {
                                format!(" - {}", channel_spec.description)
                            }
                        )
                    })
                }
            }
            DisplayFormat::Json => fmt_json(
                &self
                    .iter()
                    .map(|(k, v)| (k.clone(), ChannelSpecSerializer(v.clone())))
                    .collect::<HashMap<String, ChannelSpecSerializer>>(),
                f,
            ),
        }
    }
}

impl Display for Displayer<'_, i32> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            ChannelType::from_i32(**self)
                .map(|at| match at {
                    ChannelType::String => "string",
                    ChannelType::Bool => "bool",
                    ChannelType::Int => "int",
                    ChannelType::Float => "float",
                    ChannelType::Bytes => "bytes",
                })
                .unwrap_or("invalid-type")
        )
    }
}

impl Display for Displayer<'_, ExecutionResult> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "Execution Id: {}",
            self.execution_id
                .as_ref()
                .map(|i| i.uuid.as_str())
                .unwrap_or("unknown")
        )?;

        match self.result.as_ref() {
            Some(FunctionResult::Ok(outputs)) => {
                writeln!(f, "Outputs:")?;
                write!(f, "{}", outputs.display())
            }
            Some(FunctionResult::Error(error)) => writeln!(f, "Error: {}", error.msg),
            None => writeln!(f, "No result set"),
        }
    }
}

impl Display for Displayer<'_, Stream> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.channels.iter().try_for_each(|(name, channel)| {
            writeln!(f, "{}{}: [{}]", INDENT, name, channel.display())
        })
    }
}

impl Display for Displayer<'_, Channel> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // TODO: truncate if too long
        write!(
            f,
            "{}",
            match self.value.as_ref() {
                Some(Value::Strings(v)) => v
                    .values
                    .iter()
                    .map(|v| format!(r#""{}""#, v))
                    .collect::<Vec<String>>()
                    .join(" "),
                Some(Value::Integers(v)) => v
                    .values
                    .iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<String>>()
                    .join(" "),
                Some(Value::Floats(v)) => v
                    .values
                    .iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<String>>()
                    .join(" "),
                Some(Value::Booleans(v)) => v
                    .values
                    .iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<String>>()
                    .join(" "),
                Some(Value::Bytes(v)) => v
                    .values
                    .iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<String>>()
                    .join(" "),
                None => "null".to_owned(),
            }
        )
    }
}

impl Display for Displayer<'_, Runtime> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.format == DisplayFormat::Json {
            // TODO: return write!(f, "{}", self.serialize());
            return Ok(());
        }

        write!(
            f,
            "{}{} ({})",
            INDENT,
            Green.paint(&self.name),
            &self.source
        )
    }
}

// impl display of listed auth request
impl Display for Displayer<'_, RemoteAccessRequest> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "{:<40}{:<22}{:^10}{}",
            self.id
                .as_ref()
                .map(|id| id.uuid.as_str())
                .unwrap_or("missing id"),
            Utc.timestamp(self.expires_at as i64, 0)
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S"),
            if self.approved { "yes" } else { "no" },
            self.subject
        )
    }
}

// impl display of listed auth requests
impl Display for Displayer<'_, &[RemoteAccessRequest]> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "{:<40}{:<22}{:<10}subject",
            "id", "expires at", "approved"
        )?;
        writeln!(f, "{:-<80}", "")?;
        self.iter().try_for_each(|r| write!(f, "{}", r.display()))
    }
}
