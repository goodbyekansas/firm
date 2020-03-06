#![deny(warnings)]

// module declarations
pub mod proto {
    tonic::include_proto!("functions");
}

// std
use std::{
    collections::HashMap,
    fmt::{self, Display},
};

// 3rd party
use structopt::StructOpt;
use tokio::runtime;
use tonic::Request;

// internal
use proto::functions_client::FunctionsClient;
use proto::{ExecuteRequest, Function, FunctionId, FunctionInput, FunctionOutput, ListRequest};

// arguments
#[derive(StructOpt, Debug)]
#[structopt(name = "bendini")]
struct BendiniArgs {
    /// function executor servicen address
    #[structopt(short, long, default_value = "tcp://[::1]")]
    address: String,

    /// function executor service port
    #[structopt(short, long, default_value = "1939")]
    port: u32,

    /// Command to run
    #[structopt(subcommand)]
    cmd: Command,
}

#[derive(StructOpt, Debug)]
enum Command {
    List,
    Execute { function_id: String },
}

// impl display of listed functions
impl Display for Function {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let na = "n/a".to_string();
        let id_str = self.id.clone().unwrap_or(FunctionId { value: na }).value;
        writeln!(f, "{}", self.name)?;
        writeln!(f, "\tid: {}", id_str)?;
        if self.inputs.is_empty() {
            writeln!(f, "\tinputs:  n/a")?;
        } else {
            writeln!(f, "\tinputs:")?;
            self.inputs
                .clone()
                .into_iter()
                .map(|i| writeln!(f, "\t\t{}", i))
                .collect::<fmt::Result>()?;
        }
        if self.outputs.is_empty() {
            writeln!(f, "\toutputs: n/a")?;
        } else {
            writeln!(f, "\toutputs:")?;
            self.outputs
                .clone()
                .into_iter()
                .map(|i| writeln!(f, "\t\t{}", i))
                .collect::<fmt::Result>()?;
        }
        if self.tags.is_empty() {
            writeln!(f, "\ttags:    n/a")
        } else {
            writeln!(f, "\ttags:")?;
            self.tags
                .clone()
                .iter()
                .map(|(x, y)| writeln!(f, "\t\t{}:{}", x, y))
                .collect()
        }
    }
}

impl Display for FunctionInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let required = if self.required {
            "required"
        } else {
            "optional"
        };
        let default_value = if self.default_value.is_empty() {
            "n/a"
        } else {
            &self.default_value
        };
        write!(
            f,
            "{name}:{req_opt}:{ftype}: {default}",
            name = self.name,
            req_opt = required,
            ftype = self.r#type,
            default = default_value,
        )
    }
}

impl Display for FunctionOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{name}:{ftype}", name = self.name, ftype = self.r#type,)
    }
}

fn main() -> Result<(), u32> {
    // parse arguments
    let args = BendiniArgs::from_args();
    let address = format!("{}:{}", args.address, args.port);

    // handle async stuff in a non-async way
    let mut basic_rt = runtime::Builder::new()
        .basic_scheduler()
        .enable_all()
        .build().map_err(|e| {
            println!("Failed to create new runtime builder for async operations: {}", e);
            1u32
        })?;

    // call the client to connect and don't worry about async stuff
    let mut client = basic_rt.block_on(FunctionsClient::connect(address.clone())).map_err(|e| {
        println!("Failed to connect to Avery at \"{}\": {}", address, e);
        2u32
    })?;

    match args.cmd {
        Command::List => {
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
        Command::Execute { function_id } => {
            println!("Executing function with id {}", function_id);
            let execute_response = basic_rt
                .block_on(client.execute(Request::new(ExecuteRequest {
                    function: Some(FunctionId {
                        value: function_id.clone(),
                    }),
                    arguments: String::new(),
                })))
                .map_err(|e| {
                    println!("Failed to execute function with id {}: {}", function_id, e);
                    4u32
                })?;
            println!("{:?}", execute_response);
        }
    }

    Ok(())
}
