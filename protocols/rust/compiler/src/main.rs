use std::{fs, path::PathBuf};

use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(name = "compiler", about = "A simple rust protoc command line")]
struct Options {
    /// Input proto files
    #[structopt(parse(from_os_str))]
    files: Vec<PathBuf>,

    /// Output folder
    #[structopt(short = "o", long = "out", default_value = ".", parse(from_os_str))]
    out_dir: PathBuf,

    /// Proto include paths
    #[structopt(short = "I", long = "include", parse(from_os_str))]
    includes: Vec<PathBuf>,

    /// determines if services should be included in the build output
    #[structopt(short = "s", long = "build-services")]
    build_services: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::from_args();
    fs::create_dir_all(&options.out_dir)?;
    tonic_build::configure()
        .out_dir(&options.out_dir)
        .build_client(options.build_services)
        .build_server(options.build_services)
        .compile(&options.files, &options.includes)?;
    Ok(())
}
