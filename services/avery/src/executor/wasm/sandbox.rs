use std::path::{Path, PathBuf};
use tempfile::TempDir;

pub struct Sandbox {
    path: PathBuf,
}

impl Sandbox {
    pub fn new() -> Self {
        Self {
            // TODO: Do not unwrap here
            path: TempDir::new().unwrap().into_path(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}
