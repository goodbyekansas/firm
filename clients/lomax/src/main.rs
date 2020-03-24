#![deny(warnings)]

// module declarations
pub mod proto {
    tonic::include_proto!("functions");
}

mod manifest;

// std
use std::{
    collections::HashMap,
    fmt::{self, Display},
    path::PathBuf,
};

// 3rd party
use manifest::FunctionManifest;
use structopt::StructOpt;
use tokio::runtime;
use tonic::Request;

// internal
use proto::functions_registry_client::FunctionsRegistryClient;
use proto::{
    ArgumentType, ExecutionEnvironment, Function, FunctionDescriptor, FunctionId, FunctionInput,
    FunctionOutput, ListRequest, RegisterRequest,
};

// arguments
#[derive(StructOpt, Debug)]
#[structopt(name = "lomax")]
struct LomaxArgs {
    // function executor servicen address
    #[structopt(short, long, default_value = "tcp://[::1]")]
    address: String,

    // function executor service port
    #[structopt(short, long, default_value = "1939")]
    port: u32,

    // Command to run
    #[structopt(subcommand)]
    cmd: Command,
}

#[derive(StructOpt, Debug)]
enum Command {
    List {
        #[structopt(short, long)]
        pipeable_output: bool,
    },

    Register {
        #[structopt(parse(from_os_str))]
        code: PathBuf,

        #[structopt(parse(from_os_str))]
        manifest: PathBuf,
    },
}

// impl display of listed functions
impl Display for FunctionDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let env_name = self
            .execution_environment
            .clone()
            .unwrap_or(ExecutionEnvironment {
                name: "n/a".to_string(),
            })
            .name;

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
                writeln!(f, "{}{}", &t, &k.name)?;
                writeln!(f, "{}id:      {}", &t, id_str)?;
                writeln!(f, "{}name:    {}", &t, &k.name)?;
                // this will change to be more data
                writeln!(f, "{}exeEnv:  {}", &t, env_name)?;
                write!(f, "{}entry:   ", &t)?;
                if self.entrypoint.is_empty() {
                    writeln!(f, "n/a")?;
                } else {
                    writeln!(f, "{}", self.entrypoint)?;
                }
                write!(f, "{}codeUrl: ", &t)?;
                if self.code_url.is_empty() {
                    writeln!(f, "n/a")?;
                } else {
                    writeln!(f, "{}", self.code_url)?;
                }
                if k.inputs.is_empty() {
                    writeln!(f, "{}inputs:  [n/a]", &t)?;
                } else {
                    writeln!(f, "{}inputs:", &t)?;
                    k.inputs
                        .clone()
                        .into_iter()
                        .map(|i| writeln!(f, "{}{}", &t2, i))
                        .collect::<fmt::Result>()?;
                }
                if k.outputs.is_empty() {
                    writeln!(f, "\toutputs: [n/a]")?;
                } else {
                    writeln!(f, "\toutputs:")?;
                    k.outputs
                        .clone()
                        .into_iter()
                        .map(|i| writeln!(f, "{}{}", &t2, i))
                        .collect::<fmt::Result>()?;
                }
                if k.tags.is_empty() {
                    writeln!(f, "\ttags:    [n/a]")
                } else {
                    writeln!(f, "\ttags:")?;
                    k.tags
                        .clone()
                        .iter()
                        .map(|(x, y)| writeln!(f, "{}{}:{}", &t2, x, y))
                        .collect()
                }
            }

            None => writeln!(f, "function descriptor did not contain function 🤔"),
        }
    }
}

impl Display for Function {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let na = "n/a".to_string();
        let id_str = self.id.clone().unwrap_or(FunctionId { value: na }).value;
        writeln!(f, "\t{}", self.name)?;
        writeln!(f, "\tid:      {}", id_str)?;
        if self.inputs.is_empty() {
            writeln!(f, "\tinputs:  n/a")?;
        } else {
            writeln!(f, "\tinputs:")?;
            self.inputs
                .clone()
                .into_iter()
                .map(|i| writeln!(f, "\t\t {}", i))
                .collect::<fmt::Result>()?;
        }
        if self.outputs.is_empty() {
            writeln!(f, "\toutputs: n/a")?;
        } else {
            writeln!(f, "\toutputs:")?;
            self.outputs
                .clone()
                .into_iter()
                .map(|i| writeln!(f, "\t\t {}", i))
                .collect::<fmt::Result>()?;
        }
        if self.tags.is_empty() {
            writeln!(f, "\ttags:    n/a")
        } else {
            writeln!(f, "\ttags:")?;
            self.tags
                .clone()
                .iter()
                .map(|(x, y)| writeln!(f, "\t\t {}:{}", x, y))
                .collect()
        }
    }
}

impl Display for FunctionInput {
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

impl Display for FunctionOutput {
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

fn main() -> Result<(), u32> {
    // parse arguments
    let args = LomaxArgs::from_args();
    let address = format!("{}:{}", args.address, args.port);

    // handle async stuff in a non-async way
    let mut basic_rt = runtime::Builder::new()
        .basic_scheduler()
        .enable_all()
        .build()
        .map_err(|e| {
            println!(
                "Failed to create new runtime builder for async operations: {}",
                e
            );
            1u32
        })?;

    // call the client to connect and don't worry about async stuff
    let mut client = basic_rt
        .block_on(FunctionsRegistryClient::connect(address.clone()))
        .map_err(|e| {
            println!("Failed to connect to Avery at \"{}\": {}", address, e);
            2u32
        })?;

    match args.cmd {
        Command::List { .. } => {
            println!("Listing functions");
            let list_request = ListRequest {
                name_filter: String::new(),
                tags_filter: HashMap::new(),
                limit: 0,
                offset: 0,
            };

            let list_response = basic_rt
                .block_on(client.list(Request::new(list_request)))
                .map_err(|e| {
                    println!("Failed to list functions: {}", e);
                    3u32
                })?;

            list_response
                .into_inner()
                .functions
                .into_iter()
                .for_each(|f| println!("{}", f))
        }

        Command::Register { code, manifest } => {
            let manifest_path = manifest;
            let code_path = code;

            let manifest = FunctionManifest::parse(&manifest_path).map_err(|e| {
                println!("\"{}\".", e);
                1u32
            })?;

            println!("Registering function \"{}\"...", manifest.name());

            println!("Reading manifest file from: {}", manifest_path.display());
            let mut register_request: RegisterRequest = (&manifest).into();
            println!("Reading code file from: {}", code_path.display());
            register_request.code = std::fs::read(code_path).map_err(|e| {
                println!(
                    "Failed to read code for function {}: {}. Skipping.",
                    manifest.name(),
                    e
                );
                3u32
            })?;

            let r = basic_rt
                .block_on(client.register(tonic::Request::new(register_request)))
                .map_err(|e| {
                    println!(
                        "Failed to register function \"{}\". Err: {}",
                        manifest.name(),
                        e
                    );
                    3u32
                })?;

            println!(
                "Registered function \"{}\" ({}) with registry at {}",
                manifest.name(),
                r.into_inner().value,
                address
            );
        }
    }

    Ok(())
}
