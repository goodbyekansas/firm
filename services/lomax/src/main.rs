use structopt::StructOpt;

mod config;
mod run;
mod tls;

#[cfg(unix)]
mod unix;

#[cfg(unix)]
use unix as system;

#[cfg(windows)]
mod windows;

#[cfg(windows)]
use windows as system;

fn main() -> Result<(), i32> {
    let args = run::LomaxArgs::from_args();
    system::bootstrap(args)
}
