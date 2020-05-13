use regex::Regex;
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

    pub fn map(&self, arg: &str) -> String {
        let regex = Regex::new(r"(^|[=\s;:])sandbox(\b)").unwrap();

        regex
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
        let sandbox = Sandbox::new();
        assert_eq!(
            sandbox.path().join("some").join("dir"),
            Path::new(&sandbox.map("sandbox/some/dir"))
        );

        assert_eq!(
            format!("--some-arg={}", sandbox.path().display()),
            sandbox.map("--some-arg=sandbox")
        );

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
