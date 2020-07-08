#![deny(warnings)]

// std
use std::{
    collections::HashMap,
    error::Error,
    fmt::{self, Display},
    i64,
    io::Write,
};

// 3rd party
use structopt::StructOpt;
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};
use tokio::runtime;
use uuid::Uuid;

// internal
//use proto::functions_client::FunctionsClient;
use gbk_protocols::{
    functions::{
        execute_response::Result as ProtoResult, functions_client::FunctionsClient, ArgumentType,
        ExecuteRequest, ExecuteResponse, Function, FunctionArgument, FunctionId, FunctionInput,
        FunctionOutput, FunctionResult, ListRequest, OrderingDirection, OrderingKey, ReturnValue,
        VersionRequirement,
    },
    tonic::Request,
};

/// Bendini is a command line client to Avery, the function executor service of the GBK pipeline
#[derive(StructOpt, Debug)]
#[structopt(name = "bendini")]
struct BendiniArgs {
    /// Function executor service address
    #[structopt(short, long, default_value = "tcp://[::1]")]
    address: String,

    /// Function executor service port
    #[structopt(short, long, default_value = "1939")]
    port: u32,

    /// Command to run
    #[structopt(subcommand)]
    cmd: Command,
}

#[derive(StructOpt, Debug)]
enum Command {
    /// Lists functions known to the service
    List {
        #[structopt(short, long)]
        pipeable_output: bool,
    },

    /// Executes a function with arguments
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

trait DisplayColored {
    fn print_colored(&self, f: &mut dyn WriteColor) -> std::io::Result<()>;
}

// impl display of listed functions
impl DisplayColored for Function {
    fn print_colored(&self, f: &mut dyn WriteColor) -> std::io::Result<()> {
        let na = "n/a".to_string();
        let id_str = self.id.clone().unwrap_or(FunctionId { value: na }).value;

        f.set_color(
            ColorSpec::new()
                .set_bold(true)
                .set_intense(true)
                .set_fg(Some(Color::Blue)),
        )?;
        writeln!(f, "\t{}", self.name)?;
        f.reset()?;
        write!(f, "\tid:      ")?;
        f.set_color(ColorSpec::new().set_fg(Some(Color::Cyan)))?;
        writeln!(f, "{}", id_str)?;
        f.reset()?;
        writeln!(f, "\tversion: {}", self.version)?;
        if self.inputs.is_empty() {
            writeln!(f, "\tinputs:  [n/a]")?;
        } else {
            writeln!(f, "\tinputs:")?;
            self.inputs
                .clone()
                .into_iter()
                .map(|i| {
                    write!(f, "\t\t")?;
                    i.print_colored(f)?;
                    writeln!(f)
                })
                .collect::<std::io::Result<()>>()?;
        }
        if self.outputs.is_empty() {
            writeln!(f, "\toutputs: [n/a]")?;
        } else {
            writeln!(f, "\toutputs:")?;
            self.outputs
                .clone()
                .into_iter()
                .map(|o| {
                    write!(f, "\t\t")?;
                    o.print_colored(f)?;
                    writeln!(f)
                })
                .collect::<std::io::Result<()>>()?;
        }
        if self.metadata.is_empty() {
            writeln!(f, "\tmetadata:    [n/a]")
        } else {
            writeln!(f, "\tmetadata:")?;
            self.metadata
                .clone()
                .iter()
                .map(|(x, y)| writeln!(f, "\t\t {}:{}", x, y))
                .collect()
        }
    }
}

impl DisplayColored for FunctionInput {
    fn print_colored(&self, f: &mut dyn WriteColor) -> std::io::Result<()> {
        let required = if self.required {
            f.set_color(ColorSpec::new().set_fg(Some(Color::Green)))?;
            "[required]"
        } else {
            f.set_color(ColorSpec::new().set_fg(Some(Color::Yellow)))?;
            "[optional]"
        };
        let default_value = if self.default_value.is_empty() {
            "n/a"
        } else {
            &self.default_value
        };

        let from_exe_env = if self.from_execution_environment {
            "[from_exe_env]"
        } else {
            ""
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
            "{req_opt}{from_exe_env}:{ftype}:{name}: {default}",
            name = self.name,
            from_exe_env = from_exe_env,
            req_opt = required,
            ftype = tp,
            default = default_value,
        )?;

        f.reset()
    }
}

impl DisplayColored for FunctionOutput {
    fn print_colored(&self, f: &mut dyn WriteColor) -> std::io::Result<()> {
        let tp = ArgumentType::from_i32(self.r#type)
            .map(|at| match at {
                ArgumentType::String => "[string ]",
                ArgumentType::Bool => "[bool   ]",
                ArgumentType::Int => "[int    ]",
                ArgumentType::Float => "[float  ]",
                ArgumentType::Bytes => "[bytes  ]",
            })
            .unwrap_or("[Invalid type ]");

        write!(f, "[ensured ]:{ftype}:{name}", name = self.name, ftype = tp,)
    }
}

macro_rules! bytes_as_64_bit_array {
    ($bytes: expr) => {{
        [
            $bytes.get(0).cloned().unwrap_or_default(),
            $bytes.get(1).cloned().unwrap_or_default(),
            $bytes.get(2).cloned().unwrap_or_default(),
            $bytes.get(3).cloned().unwrap_or_default(),
            $bytes.get(4).cloned().unwrap_or_default(),
            $bytes.get(5).cloned().unwrap_or_default(),
            $bytes.get(6).cloned().unwrap_or_default(),
            $bytes.get(7).cloned().unwrap_or_default(),
        ]
    }};
}

struct Displayer<'a, T> {
    display: &'a T,
}

trait DisplayExt<'a, T>
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

impl Display for Displayer<'_, FunctionResult> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.display
            .values
            .iter()
            .map(|rv| writeln!(f, "{}", rv.display()))
            .collect::<fmt::Result>()
    }
}

impl Display for Displayer<'_, ReturnValue> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (tp, val) = ArgumentType::from_i32(self.display.r#type)
            .map(|at| match at {
                ArgumentType::String => (
                    "[string ]",
                    get_reasonable_value_string(&self.display.value),
                ),
                ArgumentType::Bool => (
                    "[bool   ]",
                    if self.display.value[0] == 0 {
                        String::from("true")
                    } else {
                        String::from("false")
                    },
                ),
                ArgumentType::Int => (
                    "[int    ]",
                    format!(
                        "{}",
                        i64::from_le_bytes(bytes_as_64_bit_array!(self.display.value))
                    ),
                ),
                ArgumentType::Float => (
                    "[float  ]",
                    format!(
                        "{}",
                        f64::from_le_bytes(bytes_as_64_bit_array!(self.display.value))
                    ),
                ),
                ArgumentType::Bytes => (
                    "[bytes  ]",
                    get_reasonable_value_string(&self.display.value),
                ),
            })
            .unwrap_or(("[Invalid type ]", String::new()));

        write!(
            f,
            "{ftype}:{name}: {val}",
            name = self.display.name,
            ftype = tp,
            val = val
        )
    }
}

impl Display for Displayer<'_, ExecuteResponse> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let na = "n/a".to_string();
        let id_str = self
            .display
            .function
            .clone()
            .unwrap_or(FunctionId { value: na })
            .value;
        let result = self.display.result.clone();
        writeln!(f, "\tid:     {}", id_str)?;

        match result {
            Some(ProtoResult::Ok(r)) => writeln!(f, "\tresult: {}", r.display()),
            Some(ProtoResult::Error(e)) => writeln!(f, "\terror: {}", e.msg),
            None => writeln!(f, "\tfunction execution did not produce a result? 🧐"),
        }
    }
}

fn get_reasonable_value_string(argument_value: &[u8]) -> String {
    const MAX_PRINTABLE_VALUE_LENGTH: usize = 256;
    if argument_value.len() < MAX_PRINTABLE_VALUE_LENGTH {
        String::from_utf8(argument_value.to_vec())
            .unwrap_or_else(|_| String::from("invalid utf-8 string 🚑"))
    } else {
        format!(
            "too long value (> {} bytes, vaccuum tubes will explode) 💣",
            MAX_PRINTABLE_VALUE_LENGTH
        )
    }
}

#[derive(Clone, Debug)]
struct BendiniError {
    exit_code: i32,
}

impl From<i32> for BendiniError {
    fn from(v: i32) -> Self {
        Self { exit_code: v }
    }
}

impl From<std::io::Error> for BendiniError {
    fn from(_: std::io::Error) -> Self {
        Self { exit_code: 8 }
    }
}

fn run() -> Result<(), BendiniError> {
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
            1
        })?;

    // call the client to connect and don't worry about async stuff
    let mut client = basic_rt
        .block_on(FunctionsClient::connect(address.clone()))
        .map_err(|e| {
            println!("Failed to connect to Avery at \"{}\": {}", address, e);
            2
        })?;

    let mut stdout = StandardStream::stdout(ColorChoice::Always);

    match args.cmd {
        Command::List { pipeable_output } => {
            // only prints the id list
            if pipeable_output {
                let list_request = ListRequest {
                    name_filter: String::new(),
                    metadata_filter: HashMap::new(),
                    metadata_key_filter: vec![],
                    exact_name_match: false,
                    order_by: OrderingKey::Name as i32,
                    order_direction: OrderingDirection::Ascending as i32,
                    version_requirement: None,
                    offset: 0,
                    limit: 25,
                };

                let list_response = basic_rt
                    .block_on(client.list(Request::new(list_request)))
                    .map_err(|e| {
                        println!("Failed to list functions: {}", e);
                        3
                    })?;

                list_response
                    .into_inner()
                    .functions
                    .into_iter()
                    .for_each(|f| {
                        println!(
                            "{}",
                            f.id.unwrap_or(FunctionId {
                                value: "n/a".to_string()
                            })
                            .value
                        )
                    })
            // prints the full record for each function
            } else {
                stdout.set_color(ColorSpec::new().set_fg(Some(Color::Green)))?;
                writeln!(&mut stdout, "Functions: ")?;
                stdout.reset()?;
                let list_request = ListRequest {
                    name_filter: String::new(),
                    metadata_filter: HashMap::new(),
                    metadata_key_filter: vec![],
                    exact_name_match: false,
                    order_by: OrderingKey::Name as i32,
                    order_direction: OrderingDirection::Ascending as i32,
                    version_requirement: None,
                    limit: 25,
                    offset: 0,
                };

                let list_response = basic_rt
                    .block_on(client.list(Request::new(list_request)))
                    .map_err(|e| {
                        println!("Failed to list functions: {}", e);
                        3
                    })?;

                list_response
                    .into_inner()
                    .functions
                    .into_iter()
                    .for_each(|f| {
                        writeln!(&mut stdout)
                            .and_then(|_| f.print_colored(&mut stdout))
                            .map_or_else(|e| eprintln!("Failed to print colored: {}", e), |_| ());
                    })
            }
        }
        Command::Execute {
            function_id,
            arguments,
        } => {
            let function_record: Result<Function, i32> = match Uuid::parse_str(&function_id) {
                Err(_) => {
                    let split = function_id.splitn(2, ':').collect::<Vec<&str>>();
                    // Not UUID assuming it's a name:version
                    let (function_name, function_version): (&str, &str) = match &split[..] {
                        [name, version] => Ok((*name, *version)),
                        [name] => Ok((*name, "*")),
                        _ => {
                            println!("Invalid function name and/or version specifier.");
                            Err(4)
                        }
                    }?;

                    Ok(basic_rt
                        .block_on(client.list(Request::new(ListRequest {
                            name_filter: function_name.to_owned(),
                            metadata_filter: HashMap::new(),
                            metadata_key_filter: vec![],
                            offset: 0,
                            limit: 1,
                            exact_name_match: true,
                            version_requirement: Some(VersionRequirement {
                                expression: function_version.to_owned(),
                            }),
                            order_direction: OrderingDirection::Ascending as i32,
                            order_by: OrderingKey::Name as i32,
                        })))
                        .map_err(|e| {
                            println!("{}", e);
                            4
                        })?
                        .into_inner()
                        .functions
                        .first()
                        .ok_or_else(|| {
                            println!(
                                "Could not find a function with name: \"{}\" and version: \"{}\"",
                                function_name, function_version
                            );
                            4
                        })?
                        .clone())
                }
                Ok(func_id) => Ok(basic_rt
                    .block_on(client.get(Request::new(FunctionId {
                        value: func_id.to_string(),
                    })))
                    .map_err(|e| {
                        println!("{}", e);
                        4
                    })?
                    .into_inner()),
            };

            let function_record = function_record?;

            println!(
                "Executing function: {}",
                function_record
                    .id
                    .clone()
                    .unwrap_or_else(|| FunctionId {
                        value: "Empty Id".to_owned(),
                    })
                    .value
            );

            let dst_arguments: Vec<FunctionArgument> = if !function_record.inputs.is_empty() {
                // assumming we have arguements
                let fm: HashMap<String, i32> = function_record
                    .inputs
                    .iter()
                    .map(|f: &FunctionInput| (f.name.clone(), f.r#type))
                    .collect();

                arguments.iter().map(
                    |(key, val)| {
                            fm.get(key).ok_or(format!("argument {} is not expected for function {}", key, function_record.name)).and_then(|tp| {
                            let parsed_type = ArgumentType::from_i32(*tp).ok_or(format!("argument type {} is out of range (out of date protobuf definitions?)", tp))?;
                            match parsed_type {
                                ArgumentType::String => Ok(
                                    FunctionArgument {
                                        name:key.clone(),
                                        r#type: *tp,
                                        value: val.as_bytes().to_vec(),
                                    }
                                ),
                                ArgumentType::Bool => {
                                    val.parse::<bool>()
                                        .map_err(|e| format!("cant parse argument {} into bool value. err: {}", key, e))
                                        .map(|x|
                                            FunctionArgument {
                                                name:key.clone(),
                                                r#type: *tp,
                                                value: vec![x as u8],
                                            }
                                        )
                                },
                                ArgumentType::Int => {
                                    val.parse::<i64>()
                                        .map_err(|e| format!("cant parse argument {} into int value. err: {}", key, e))
                                        .map(|x|
                                            FunctionArgument {
                                                name:key.clone(),
                                                r#type: *tp,
                                                value: x.to_le_bytes().to_vec(),
                                            }
                                        )
                                },
                                ArgumentType::Float => {
                                    val.parse::<f64>()
                                        .map_err(|e| format!("cant parse argument {} into float value. err: {}", key, e))
                                        .map(|x|
                                            FunctionArgument {
                                                name:key.clone(),
                                                r#type: *tp,
                                                value: x.to_le_bytes().to_vec(),
                                            }
                                        )
                                },
                                ArgumentType::Bytes => Ok(
                                    FunctionArgument {
                                        name:key.clone(),
                                        r#type: *tp,
                                        value: val.as_bytes().to_vec(),
                                    }
                                ),
                            }
                        })
                    }
                ).collect::<Result<Vec<FunctionArgument>, String>>().map_err(|e| {
                    println!("{}", e);
                    1
                })?
            } else {
                Vec::new()
            };

            let function_name = function_record.name.clone();
            let request = ExecuteRequest {
                function: function_record.id,
                arguments: dst_arguments,
            };

            println!("Function Execution Response");
            let execute_response = basic_rt
                .block_on(client.execute(Request::new(request)))
                .map_err(|e| {
                    println!("Failed to execute function \"{}\": {}", function_name, e);
                    4
                })?;
            println!("{}", execute_response.into_inner().display());
        }
    }

    Ok(())
}

fn main() {
    match run() {
        Ok(_) => (),
        Err(e) => std::process::exit(e.exit_code),
    }
}
