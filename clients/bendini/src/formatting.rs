use std::{
    collections::HashMap,
    fmt::{self, Display},
};

use ansi_term::Colour::Green;
use firm_types::functions::{ChannelType, Function, Runtime, StreamSpec};
use futures::{future::join, Future};
use indicatif::MultiProgress;
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

#[derive(Debug, PartialEq)]
pub enum DisplayFormat {
    // Short,
    Long,
    // Full,
    JSON,
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

// impl display of listed functions
impl Display for Displayer<'_, Function> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.format == DisplayFormat::JSON {
            // TODO: return write!(f, "{}", self.serialize());
            return Ok(());
        }

        writeln!(f, "{}{}", INDENT, Green.paint(&self.name))?;
        writeln!(f, "{}version: {}", INDENT, &self.version)?;

        if self.format == DisplayFormat::Long {
            writeln!(
                f,
                "{}runtime:  {}",
                INDENT,
                self.runtime
                    .as_ref()
                    .unwrap_or(&Runtime {
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

            write!(f, "{}inputs: {}", INDENT, self.input.display())?;
            write!(f, "{}outputs: {}", INDENT, self.output.display())?;

            if self.metadata.is_empty() {
                writeln!(f, "{}metadata:    [n/a]", INDENT)
            } else {
                writeln!(f, "{}metadata:", INDENT)?;
                self.metadata
                    .clone()
                    .iter()
                    .map(|(x, y)| writeln!(f, "{}{}:{}", INDENT.repeat(2), x, y))
                    .collect()
            }
        } else {
            Ok(())
        }
    }
}

impl Display for Displayer<'_, Option<StreamSpec>> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_none() {
            writeln!(f, " [n/a]")
        } else {
            self.as_ref()
                .map(|stream_spec| {
                    stream_spec
                        .required
                        .iter()
                        .map(|(name, input)| {
                            writeln!(
                                f,
                                "{tab}[required]:{type}:{name}:{description}",
                                tab = INDENT.repeat(2),
                                r#type = input.r#type,
                                name = name,
                                description = input.description
                            )
                        })
                        .collect::<fmt::Result>()
                })
                .transpose()?;

            self.as_ref()
                .map(|stream_spec| {
                    stream_spec
                        .optional
                        .iter()
                        .map(|(name, input)| {
                            writeln!(
                                f,
                                "{tab}[optional]:{type}:{name}:{description}",
                                tab = INDENT.repeat(2),
                                r#type = input.r#type,
                                name = name,
                                description = input.description
                            )
                        })
                        .collect::<fmt::Result>()
                })
                .transpose()?;
            Ok(())
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
                    ChannelType::String => "[string ]",
                    ChannelType::Bool => "[bool   ]",
                    ChannelType::Int => "[int    ]",
                    ChannelType::Float => "[float  ]",
                    ChannelType::Bytes => "[bytes  ]",
                })
                .unwrap_or("[Invalid type ]")
        )
    }
}
