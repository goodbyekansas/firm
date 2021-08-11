use avery::{run, system};
use structopt::StructOpt;

fn main() -> Result<(), i32> {
    let args = run::AveryArgs::from_args();
    system::bootstrap(args)
}
