pub mod proto {
    tonic::include_proto!("functions");
}

use std::collections::HashMap;

use slog::{info, o, Drain};
use slog_async;
use slog_term;
use tonic::Request;
use structopt::StructOpt;

use proto::functions_client::FunctionsClient;
use proto::{ExecuteRequest, FunctionId, ListRequest};

// Parse a single key-value pair
fn parse_key_val<T, U>(s: &str) -> Result<(T, U), Box<dyn std::error::Error>>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + 'static,
    U: std::str::FromStr,
    U::Err: std::error::Error + 'static,
{
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid KEY=value: no `=` found in `{}`", s))?;
    Ok((s[..pos].parse()?, s[pos + 1..].parse()?))
}

#[derive(StructOpt, Debug)]
#[structopt(name = "bendini")]
struct BendiniArgs {
    // function executor servicen address
    #[structopt(short, long, default_value = "https://[::1]")]
    address: String,

    // function executor service port
    #[structopt(short, long, default_value = "1939")]
    port: u32,

    // number_of_values = 1 forces the user to repeat the -D option for each key-value pair:
    // bendini -A a=1 -A b=2
    // Without number_of_values = 1 you can do:
    // bendini -A a=1 b=2
    // but this makes adding an argument after the values impossible:
    // bendini -A a=1 -A b=2 my_input_file
    // becomes invalid.
    #[structopt(short = "A", parse(try_from_str = parse_key_val), number_of_values = 1)]
    function_arguments: Vec<(String, String)>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::CompactFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    let log = slog::Logger::root(drain, o!());

    
    let args = BendiniArgs::from_args();
    info!(log, "{:#?}", args); // TODO: Remove

    let address = format!("{}:{}",args.address, args.port);

    info!(log, "Bendini client connecting to \"{}\"", address);
    let mut client = FunctionsClient::connect(address).await?;
    info!(log, "Bendini connected!");

    info!(log, "Bendini running list request."); // TODO: Remvove
    let response = client
        .list(Request::new(ListRequest {
            name_filter: String::new(),
            tags_filter: HashMap::new(),
            limit: 0,
            offset: 0,
        }))
        .await?;

    info!(log, "Response: {:?}", response); // TODO: Remvove

    info!(log, "Bendini executing hello world function."); // TODO: Remvove

    let execute_response = client
        .execute(Request::new(ExecuteRequest {
            function: Some(FunctionId {
                value: response
                    .into_inner()
                    .functions
                    .first()
                    .and_then(|function| function.id.as_ref())
                    .and_then(|fun_id| Some(fun_id.value.clone()))
                    .ok_or("Failed to get function ID")?,
            }),
            arguments: String::new(), // TODO: put args as json here
        }))
        .await?;

    info!(log, "Execute Response: {:?}", execute_response); // TODO: Remvove

    Ok(())
}
