use std::path::{Path, PathBuf};

use regex::Regex;
use tempfile::TempDir;

use super::error::WasiError;

#[derive(Clone, Debug)]
pub struct Sandbox {
    path: PathBuf,
    regex: Regex,
}

impl Sandbox {
    pub fn new(map_dir: &Path) -> Result<Self, WasiError> {
        Ok(Self {
            path: TempDir::new()
                .map(|tmp_dir| tmp_dir.into_path())
                .map_err(|e| {
                    WasiError::SandboxError(format!("Failed to create sandbox temp folder: {}", e))
                })?,
            regex: Regex::new(&format!(r"(^|[=\s;:]){}(\b)", map_dir.display())).map_err(|e| {
                WasiError::SandboxError(format!("Failed to create regex for sandbox: {}", e))
            })?,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn map(&self, arg: &str) -> String {
        self.regex
            .replace_all(arg, |caps: &regex::Captures| {
                format!("{}{}{}", &caps[1], &self.path.to_string_lossy(), &caps[2])
            })
            .into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_map_sandbox_dir() {
        let sandbox = Sandbox::new(Path::new("bandbox")).unwrap();
        assert_eq!(
            sandbox.path().join("some").join("dir"),
            Path::new(&sandbox.map("bandbox/some/dir"))
        );

        assert_eq!(
            format!("--some-arg={}", sandbox.path().display()),
            sandbox.map("--some-arg=bandbox")
        );

        let sandbox = Sandbox::new(Path::new("sandbox")).unwrap();
        assert_eq!(
            format!("{0};{0}", sandbox.path().display()),
            sandbox.map("sandbox;sandbox")
        );

        assert_eq!(
            format!("{0}:{0}", sandbox.path().display()),
            sandbox.map("sandbox:sandbox")
        );

        assert_eq!(
            "some/dir/sandbox/something/else",
            sandbox.map("some/dir/sandbox/something/else")
        );

        assert_eq!(
            format!("{};kallekula/sandbox", sandbox.path().display()),
            sandbox.map("sandbox;kallekula/sandbox")
        );

        assert_eq!(
            format!("sandboxno;{}/yes", sandbox.path().display()),
            sandbox.map("sandboxno;sandbox/yes")
        );
    }
}
