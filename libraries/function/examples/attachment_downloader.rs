use std::{fs::OpenOptions, io::Write, path::PathBuf, task::Poll, time::Instant};

use sha2::{Digest, Sha256, Sha512};
use structopt::StructOpt;
use url::Url;

use function::{attachments::HttpAttachmentReader, io::PollRead};

#[derive(StructOpt, Debug)]
#[structopt(name = "Attachment downloader")]
struct Args {
    url: Url,

    #[structopt(short, long)]
    output: Option<PathBuf>,

    #[structopt(long)]
    print_checksums: Option<bool>,

    #[structopt(long)]
    print_response: Option<bool>,
}

#[allow(dead_code)]
fn bytes_to_str(mut bytes: usize) -> (f32, &'static str) {
    let mut factor = 0;
    let mut value = bytes as f32;
    while bytes >= 1024 {
        value /= 1024f32;
        bytes /= 1024;
        factor += 1;
    }

    (
        value,
        match factor {
            0 => "bytes",
            1 => "KiB",
            2 => "MiB",
            3 => "GiB",
            4 => "TiB",
            5 => "PiB",
            6 => "EiB",
            7 => "ZiB",
            8 => "YiB",
            _ => "?iB",
        },
    )
}

pub fn main() {
    let args = Args::from_args();
    let mut begin: Option<Instant> = None;
    let mut bytes = 0usize;
    let print_checksums = args.print_checksums.unwrap_or(false);
    let print_response = args.print_response.unwrap_or(false);
    let mut sha256 = Sha256::new();
    let mut sha512 = Sha512::new();

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
    let mut buf = [0u8; 8192];
    loop {
        match http.poll_read(&mut buf) {
            Ok(Poll::Ready(read)) if read == 0 => {
                match begin {
                    Some(timer) => println!(
                        "Finished reading file. Downloaded {} in {:.2} seconds ({}/s)",
                        {
                            let (f, s) = bytes_to_str(bytes);
                            format!("{:.2} {}", f, s)
                        },
                        timer.elapsed().as_secs_f32(),
                        {
                            let (f, s) = bytes_to_str(
                                (bytes as f32 / timer.elapsed().as_secs_f32()) as usize,
                            );
                            format!("{:.2} {}", f, s)
                        }
                    ),
                    None => {
                        println!("Finished reading file. Downloaded {}.", bytes);
                    }
                }
                if print_checksums {
                    let res_256 = sha256.finalize();
                    let res_512 = sha512.finalize();
                    println!("sha256: {:x}", res_256);
                    println!("sha512: {:x}", res_512);
                }

                if print_response {
                    http.response()
                        .map(|v| println!("{:#?}", v))
                        .unwrap_or_else(|| println!("No response"));
                }
                break;
            }
            Ok(Poll::Ready(read)) => {
                if begin.is_none() {
                    begin = Some(Instant::now());
                }
                sha256.update(&buf[0..read]);
                sha512.update(&buf[0..read]);
                file.write_all(&buf[0..read]).unwrap();
                bytes += read;
            }
            Ok(Poll::Pending) => {}
            Err(e) => {
                dbg!(&e);
                println!(
                    "Encountered error: {}, source: {:#?}",
                    e,
                    e.get_ref().and_then(|e| e.source())
                );
                break;
            }
        }
    }
}