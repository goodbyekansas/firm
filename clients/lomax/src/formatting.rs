use std::fmt::{self, Display};

use gbk_protocols::functions::{
    ArgumentType, ExecutionEnvironment, FunctionDescriptor, FunctionHostFolderMount, FunctionId,
    FunctionInput, FunctionOutput,
};

pub struct Displayer<'a, T> {
    display: &'a T,
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
}

impl<'a, U> DisplayExt<'a, U> for U
where
    U: prost::Message,
{
    fn display(&'a self) -> Displayer<U> {
        Displayer { display: self }
    }
}

// impl display of listed functions
impl Display for Displayer<'_, FunctionDescriptor> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let exe_env = self
            .execution_environment
            .clone()
            .unwrap_or(ExecutionEnvironment {
                name: "n/a".to_string(),
                args: vec![],
                entrypoint: "n/a".to_string(),
            });

        match &self.function {
            Some(k) => {
                let id_str =
                    k.id.clone()
                        .unwrap_or(FunctionId {
                            value: "n/a".to_string(),
                        })
                        .value;

                // on the cmd line each tab is 8 spaces
                let t = "\t";
                let t2 = "\t\t ";

                // print everything
                writeln!(f, "{}{}", t, &k.name)?;
                writeln!(f, "{}id:      {}", t, id_str)?;
                writeln!(f, "{}name:    {}", t, &k.name)?;
                writeln!(f, "{}version: {}", t, &k.version)?;
                // this will change to be more data
                writeln!(f, "{}exeEnv:  {}", t, exe_env.name)?;
                write!(f, "{}entry:   ", t)?;
                if exe_env.entrypoint.is_empty() {
                    writeln!(f, "n/a")?;
                } else {
                    writeln!(f, "{}", exe_env.entrypoint)?;
                }
                write!(f, "{}codeUrl: ", t)?;

                if self.host_folder_mounts.is_empty() {
                    writeln!(f, "\tmounts: [n/a]")?;
                } else {
                    writeln!(f, "\tmounts:")?;
                    self.host_folder_mounts
                        .clone()
                        .into_iter()
                        .map(|mount| writeln!(f, "{}{}", t2, mount.display()))
                        .collect::<fmt::Result>()?;
                }

                writeln!(
                    f,
                    "{}",
                    self.code
                        .as_ref()
                        .map_or_else(|| "n/a".to_owned(), |ref code| code.url.clone())
                )?;

                if k.inputs.is_empty() {
                    writeln!(f, "{}inputs:  [n/a]", t)?;
                } else {
                    writeln!(f, "{}inputs:", t)?;
                    k.inputs
                        .clone()
                        .into_iter()
                        .map(|i| writeln!(f, "{}{}", t2, i.display()))
                        .collect::<fmt::Result>()?;
                }
                if k.outputs.is_empty() {
                    writeln!(f, "\toutputs: [n/a]")?;
                } else {
                    writeln!(f, "\toutputs:")?;
                    k.outputs
                        .clone()
                        .into_iter()
                        .map(|i| writeln!(f, "{}{}", t2, i.display()))
                        .collect::<fmt::Result>()?;
                }
                if k.metadata.is_empty() {
                    writeln!(f, "\tmetadata:    [n/a]")
                } else {
                    writeln!(f, "\tmetadata:")?;
                    k.metadata
                        .clone()
                        .iter()
                        .map(|(x, y)| writeln!(f, "{}{}:{}", t2, x, y))
                        .collect()
                }
            }

            None => writeln!(f, "function descriptor did not contain function 🤔"),
        }
    }
}

impl Display for Displayer<'_, FunctionHostFolderMount> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "[{}] {}:{}",
            if self.required {
                "required"
            } else {
                "optional"
            },
            self.target_folder_path,
            self.host_folder_path
        )
    }
}

impl Display for Displayer<'_, FunctionInput> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let required = if self.required {
            "[required]"
        } else {
            "[optional]"
        };
        let default_value = if self.default_value.is_empty() {
            "n/a"
        } else {
            &self.default_value
        };

        let tp = ArgumentType::from_i32(self.r#type)
            .map(|at| match at {
                ArgumentType::String => "[string ]",
                ArgumentType::Bool => "[bool   ]",
                ArgumentType::Int => "[int    ]",
                ArgumentType::Float => "[float  ]",
                ArgumentType::Bytes => "[bytes  ]",
            })
            .unwrap_or("[Invalid type ]");

        write!(
            f,
            "{req_opt}:{ftype}:{name}: {default}",
            name = self.name,
            req_opt = required,
            ftype = tp,
            default = default_value,
        )
    }
}

impl Display for Displayer<'_, FunctionOutput> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let tp = ArgumentType::from_i32(self.r#type)
            .map(|at| match at {
                ArgumentType::String => "[string ]",
                ArgumentType::Bool => "[bool   ]",
                ArgumentType::Int => "[int    ]",
                ArgumentType::Float => "[float  ]",
                ArgumentType::Bytes => "[bytes  ]",
            })
            .unwrap_or("[Invalid type ]");

        write!(f, "[ensured ]:{ftype}:{name}", name = self.name, ftype = tp)
    }
}
