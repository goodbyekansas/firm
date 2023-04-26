mod file_system;

use std::path::PathBuf;

use file_system::{FileSystem, FileSystemType};

fn main() {
    let mut fs = FileSystem::new(FileSystemType::Fuse, &PathBuf::from("./ultra"));
    match fs
        .mount()
        .map(|_| std::thread::sleep(std::time::Duration::from_secs(10)))
        .and_then(|_| fs.unmount())
    {
        Ok(_) => println!("Exiting."),
        Err(e) => println!("Error occured {}", e),
    }
}
