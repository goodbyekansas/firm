pub mod proto {
    tonic::include_proto!("functions");
}

use std::collections::HashMap;

use tonic::Request;
use structopt::StructOpt;

use proto::functions_client::FunctionsClient;
use proto::{ExecuteRequest, FunctionId, ListRequest};

#[derive(StructOpt, Debug)]
#[structopt(name = "basic")]
struct BendiniArgs {
    #[structopt(short, long, default_value = "https://[::1]")]
    address: String,

    #[structopt(short, long, default_value = "1939")]
    port: u32
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {

    let args = BendiniArgs::from_args();
    println!("{:#?}", args);

    let address = format!("{}:{}",args.address, args.port);

    println!("Bendini client connecting to \"{}\"", address);
    let mut client = FunctionsClient::connect(address).await?;
    println!("Bendini connected!");

    println!("Bendini running list request.");
    let response = client
        .list(Request::new(ListRequest {
            name_filter: String::new(),
            tags_filter: HashMap::new(),
            limit: 0,
            offset: 0,
        }))
        .await?;

    println!("Response: {:?}", response);

    println!("Bendini executing hello world function.");

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

    println!("Execute Response: {:?}", execute_response);

    Ok(())
}
