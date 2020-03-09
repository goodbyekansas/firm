#![allow(warnings)]

// module declarations
pub mod proto {
    tonic::include_proto!("functions");
}

// std
use std::{
    i64,
    collections::HashMap,
    error::Error,
    fmt::{self, Display},
};

// 3rd party
use structopt::StructOpt;
use tokio::runtime;
use tonic::Request;
use serde_json::{self, json, Value};

// internal
use proto::functions_client::FunctionsClient;
use proto::{
    ArgumentType,
    ExecuteRequest,
    Function,
    FunctionId, 
    FunctionInput,
    FunctionOutput,
    ListRequest,
};

// arguments
#[derive(StructOpt, Debug)]
#[structopt(name = "bendini")]
struct BendiniArgs {
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
    Execute {
        function_id: String,
        #[structopt(short = "i", parse(try_from_str = parse_key_val))]
        arguments: Vec<(String, String)>,
    },
}

fn parse_key_val(s: &str) -> Result<(String, String), Box<dyn Error>> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid KEY=value: no `=` found in `{}`", s))?;
    Ok((s[..pos].parse()?, s[pos + 1..].parse()?))
}

// impl display of listed functions
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
            ArgumentType::Bool   => "[bool   ]",
            ArgumentType::Int    => "[int    ]",
            ArgumentType::Float  => "[float  ]",
        }).unwrap_or(               "[Invalid type ]");

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
            ArgumentType::Bool   => "[bool   ]",
            ArgumentType::Int    => "[int    ]",
            ArgumentType::Float  => "[float  ]",
        }).unwrap_or(               "[Invalid type ]");

        write!(f, "[ensured ]:{ftype}:{name}", name = self.name, ftype = tp,)
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
        .block_on(FunctionsClient::connect(address.clone()))
        .map_err(|e| {
            println!("Failed to connect to Avery at \"{}\": {}", address, e);
            2u32
        })?;

    match args.cmd {
        Command::List {pipeable_output} => {
            // only prints the id list
            if pipeable_output {
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
                    .for_each(
                        |f| println!(
                            "{}",
                            f.id.clone().unwrap_or(
                                FunctionId { value: "n/a".to_string() }
                            ).value.to_string()
                        )
                    )
            // prints the full record for each function
            } else {
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
        }
        Command::Execute {function_id, arguments} => {
            println!("Executing function: {}", function_id);

            let function_record = basic_rt
                .block_on(client.get(Request::new(FunctionId{value: function_id.clone()})))
                .map_err(|e| {
                    println!("{}", e);
                    4u32
                })?
                .into_inner();

            let mut json_string = "".to_string();
            if !function_record.inputs.is_empty() {
                //// assumming we have arguements
                let fm: HashMap<String, i32> = function_record.inputs.iter().map(
                    |f: &proto::FunctionInput| {
                        (f.name.clone(), f.r#type)
                    }
                ).collect();

                let hm: HashMap<String, Value> = arguments.iter().map(
                    |(key, val)| {
                        (
                            fm.get(key).ok_or(format!("argument {} is not expected for function {}", key, function_record.name)).and_then(|tp| {
                            let parsed_type = ArgumentType::from_i32(*tp).ok_or(format!("argument type {} is out of range (out of date protobuf definitions?)", tp))?;
                            match parsed_type {
                                ArgumentType::String => Ok(( key.clone(), Value::String(val.to_string()) )),
                                ArgumentType::Bool => {
                                    serde_json::from_str(&val)
                                        .map_err(|e| format!("cant parse argument {} into bool value. err: {}", key, e))
                                        .map(|x| (key.clone(), Value::Bool(x)) )
                                },
                                ArgumentType::Int => {
                                    serde_json::from_str(&val)
                                        .map_err(|e| format!("cant parse argument {} into int value. err: {}", key, e))
                                        .map(|x| (key.clone(), Value::Number(x)) )
                                },
                                ArgumentType::Float => {
                                    serde_json::from_str(&val)
                                        .map_err(|e| format!("cant parse argument {} into float value. err: {}", key, e))
                                        .map(|x| (key.clone(), Value::Number(x)) )
                                },
                            }
                        }))
                    }
                ).collect::<Result<HashMap<String, Value>, String>>().map_err(|e| {
                    println!("{}", e);
                    1u32
                })?;

                // this part will have to change once we get the new interface
                let json = json!(hm);
                json_string = json.to_string();
            }

            let request = ExecuteRequest {
                function: Some(FunctionId {
                    value: function_id.clone(),
                }),
                arguments: json_string
            };
            println!("request: {:?}", request);
            let execute_response = basic_rt
                .block_on(client.execute(Request::new(request)))
                .map_err(|e| {
                    println!("Failed to execute function with id {}: {}", function_id, e);
                    4u32
                })?;
            println!("{:?}", execute_response);
        }
    }

    Ok(())
}
