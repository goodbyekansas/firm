use std::{
    collections::HashMap,
    fmt::{self, Display},
};

use ansi_term::Colour::Green;
use function_protocols::functions::{Function, Input, Output, Runtime, Type};
use futures::{future::join, Future};
use indicatif::MultiProgress;
use tokio::task;

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

pub trait DisplayExt<'a, T>
where
    T: prost::Message,
{
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

impl<'a, U> DisplayExt<'a, U> for U
where
    U: prost::Message,
{
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

        let t = "  ";
        let t2 = "    ";

        writeln!(f, "{}{}", t, Green.paint(&self.name))?;
        writeln!(f, "{}version: {}", t, &self.version)?;

        if self.format == DisplayFormat::Long {
            writeln!(
                f,
                "{}runtime:  {}",
                t,
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
                t,
                self.code
                    .as_ref()
                    .and_then(|c| c.url.clone())
                    .map_or_else(|| String::from("n/a"), |code| code.url)
            )?;

            if self.inputs.is_empty() {
                writeln!(f, "{}inputs:  [n/a]", t)?;
            } else {
                writeln!(f, "{}inputs:", t)?;
                self.inputs
                    .clone()
                    .into_iter()
                    .map(|i| writeln!(f, "{}{}", t2, i.display()))
                    .collect::<fmt::Result>()?;
            }
            if self.outputs.is_empty() {
                writeln!(f, "{}outputs: [n/a]", t)?;
            } else {
                writeln!(f, "{}outputs:", t)?;
                self.outputs
                    .clone()
                    .into_iter()
                    .map(|i| writeln!(f, "{}{}", t2, i.display()))
                    .collect::<fmt::Result>()?;
            }
            if self.metadata.is_empty() {
                writeln!(f, "{}metadata:    [n/a]", t)
            } else {
                writeln!(f, "{}metadata:", t)?;
                self.metadata
                    .clone()
                    .iter()
                    .map(|(x, y)| writeln!(f, "{}{}:{}", t2, x, y))
                    .collect()
            }
        } else {
            Ok(())
        }
    }
}

impl Display for Displayer<'_, Input> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let required = if self.required {
            "[required]"
        } else {
            "[optional]"
        };

        write!(
            f,
            "{req_opt}:{ftype}:{name}",
            name = self.name,
            req_opt = required,
            ftype = self.r#type,
        )
    }
}

impl Display for Displayer<'_, Output> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[ensured ]:{ftype}:{name}",
            name = self.name,
            ftype = self.r#type
        )
    }
}

impl Display for Displayer<'_, i32> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            Type::from_i32(**self)
                .map(|at| match at {
                    Type::String => "[string ]",
                    Type::Bool => "[bool   ]",
                    Type::Int => "[int    ]",
                    Type::Float => "[float  ]",
                    Type::Bytes => "[bytes  ]",
                })
                .unwrap_or("[Invalid type ]")
        )
    }
}
