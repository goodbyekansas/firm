use std::path::{Path, PathBuf};

use regex::Regex;

use super::error::WasiError;
#[derive(Clone, Debug)]
pub struct Sandbox {
    host_path: PathBuf,
    regex: Regex,
    guest_path: PathBuf,
}

impl Sandbox {
    pub fn new(root_dir: &Path, guest_path: &Path) -> Result<Self, WasiError> {
        Ok(Self {
            host_path: tempfile::Builder::new()
                .prefix(&guest_path)
                .tempdir_in(root_dir)
                .map(|tmp_dir| tmp_dir.into_path())
                .map_err(|e| {
                    WasiError::SandboxError(format!("Failed to create sandbox temp folder: {}", e))
                })?,
            regex: Regex::new(&format!(r#"(^|[=\s;:"]){}(\b)"#, guest_path.display())).map_err(
                |e| WasiError::SandboxError(format!("Failed to create regex for sandbox: {}", e)),
            )?,
            guest_path: guest_path.to_owned(),
        })
    }

    pub fn host_path(&self) -> &Path {
        &self.host_path
    }

    pub fn guest_path(&self) -> &Path {
        &self.guest_path
    }

    pub fn map(&self, arg: &str) -> String {
        self.regex
            .replace_all(arg, |caps: &regex::Captures| {
                format!(
                    "{}{}{}",
                    &caps[1],
                    &self.host_path.to_string_lossy(),
                    &caps[2]
                )
            })
            .into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    macro_rules! sandbox {
        ($root_dir:expr, $guest_path:expr) => {
            Sandbox::new($root_dir.path(), Path::new($guest_path)).unwrap();
        };
    }
    #[test]
    fn test_map_sandbox_dir() {
        let root_dir = tempfile::TempDir::new().unwrap();
        let sandbox = sandbox!(root_dir, "bandbox");
        assert_eq!(
            sandbox.host_path().join("some").join("dir"),
            Path::new(&sandbox.map("bandbox/some/dir"))
        );

        assert_eq!(
            format!("--some-arg={}", sandbox.host_path().display()),
            sandbox.map("--some-arg=bandbox")
        );

        let sandbox = sandbox!(root_dir, "sandbox");
        assert_eq!(
            format!("{0};{0}", sandbox.host_path().display()),
            sandbox.map("sandbox;sandbox")
        );

        assert_eq!(
            format!("{0}:{0}", sandbox.host_path().display()),
            sandbox.map("sandbox:sandbox")
        );

        assert_eq!(
            "some/dir/sandbox/something/else",
            sandbox.map("some/dir/sandbox/something/else")
        );

        assert_eq!(
            format!("{};kallekula/sandbox", sandbox.host_path().display()),
            sandbox.map("sandbox;kallekula/sandbox")
        );

        assert_eq!(
            format!("sandboxno;{}/yes", sandbox.host_path().display()),
            sandbox.map("sandboxno;sandbox/yes")
        );

        let attachmentbox = sandbox!(root_dir, "attachments");
        assert_eq!(
            format!(
                "\'from start_blender import main;main.main(\"{}/menu-json\");\'",
                attachmentbox.host_path().display()
            ),
            attachmentbox
                .map("\'from start_blender import main;main.main(\"attachments/menu-json\");\'")
        )
    }
}
