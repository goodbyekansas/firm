use std::{fs::OpenOptions, io::Write, path::PathBuf, task::Poll};

use structopt::StructOpt;
use url::Url;

use function::{attachments::HttpAttachmentReader, io::PollRead};

#[derive(StructOpt, Debug)]
#[structopt(name = "Attachment downloader")]
struct Args {
    url: Url,

    #[structopt(short, long)]
    output: Option<PathBuf>,
}

pub fn main() {
    let args = Args::from_args();

    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .create(true)
        .open(
            args.output
                .unwrap_or_else(|| PathBuf::from("downloaded-file.download")),
        )
        .unwrap();

    println!("Downloading url {}", args.url);
    let mut http = HttpAttachmentReader::new(args.url);
    let mut buf = [0u8; 1024];
    loop {
        match http.poll_read(&mut buf) {
            Ok(Poll::Ready(read)) if read == 0 => {
                println!("Finished reading file.");
                break;
            }
            Ok(Poll::Ready(read)) => {
                file.write_all(&buf[0..read]).unwrap();
            }
            Ok(Poll::Pending) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(e) => {
                println!("Encountered error: {}", e);
                break;
            }
        }
    }
}
