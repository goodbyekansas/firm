use std::{
    fs::OpenOptions,
    path::{Path, PathBuf},
};

use regex::Regex;
use tempfile::TempDir;
use wasmer_wasi::state::HostFile;

use super::error::WasiError;

pub struct StdIOFiles {
    pub stdout: HostFile,
    pub stderr: HostFile,
}

impl StdIOFiles {
    pub fn try_clone(&self) -> std::io::Result<Self> {
        Ok(StdIOFiles {
            stdout: HostFile::new(
                self.stdout.inner.try_clone()?,
                self.stdout.host_path.clone(),
                true,
                true,
                true,
            ),
            stderr: HostFile::new(
                self.stderr.inner.try_clone()?,
                self.stderr.host_path.clone(),
                true,
                true,
                true,
            ),
        })
    }
}

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
            regex: Regex::new(&format!(r#"(^|[=\s;:"]){}(\b)"#, map_dir.display())).map_err(
                |e| WasiError::SandboxError(format!("Failed to create regex for sandbox: {}", e)),
            )?,
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

    pub fn setup_stdio(&self) -> Result<StdIOFiles, WasiError> {
        let stdout_path = self.path().join("stdout");
        let stdout = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&stdout_path)
            .map_err(WasiError::FailedToSetupStdIO)?;

        let stderr_path = self.path().join("stderr");
        let stderr = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&stderr_path)
            .map_err(WasiError::FailedToSetupStdIO)?;
        Ok(StdIOFiles {
            stdout: HostFile::new(stdout, stdout_path, true, true, true),
            stderr: HostFile::new(stderr, stderr_path, true, true, true),
        })
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

        let attachmentbox = Sandbox::new(Path::new("attachments")).unwrap();
        assert_eq!(
            format!(
                "\'from start_blender import main;main.main(\"{}/menu-json\");\'",
                attachmentbox.path().display()
            ),
            attachmentbox
                .map("\'from start_blender import main;main.main(\"attachments/menu-json\");\'")
        )
    }
}