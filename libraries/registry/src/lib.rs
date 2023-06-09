use std::{
    collections::HashMap, fmt::Display, fs::OpenOptions, ops::Deref, path::PathBuf, str::FromStr,
};

use firm_protocols::functions::{Filters, Function, FunctionId};
use regex::RegexSet;
use url::Url;

pub struct FunctionUrl(Url);

impl Deref for FunctionUrl {
    type Target = Url;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Display for FunctionUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Resolve a function from a URL
pub async fn resolve(url: &FunctionUrl) -> Result<Option<Function>, String> {
    match url.0.scheme() {
        "file" => FsResolver::new(&url.0).resolve().await,
        "firm+file" => FsRegistryResolver::new(&url.0).resolve().await,
        x => Err(format!("unsupported transport {}", x)),
    }
}

#[async_trait::async_trait]
pub trait Registry {
    async fn list(&self, filters: Filters) -> Vec<Function>;
    async fn get(&self, id: FunctionId) -> Option<Function>;
}

pub struct CachingRegistry {}

#[async_trait::async_trait]
impl Registry for CachingRegistry {
    async fn list(&self, _filters: Filters) -> Vec<Function> {
        todo!()
    }

    async fn get(&self, _id: FunctionId) -> Option<Function> {
        todo!()
    }
}

pub struct GrpcRegistry {}

#[async_trait::async_trait]
impl Registry for GrpcRegistry {
    async fn list(&self, _filters: Filters) -> Vec<Function> {
        todo!()
    }

    async fn get(&self, _id: FunctionId) -> Option<Function> {
        todo!()
    }
}

#[allow(dead_code)]
pub struct FsRegistry {
    root: PathBuf,
}

pub struct DbRegistry {}

impl FsRegistry {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

#[async_trait::async_trait]
impl Registry for FsRegistry {
    async fn list(&self, _filters: Filters) -> Vec<Function> {
        todo!()
    }

    async fn get(&self, _id: FunctionId) -> Option<Function> {
        todo!()
    }
}

#[async_trait::async_trait]
impl Registry for DbRegistry {
    async fn list(&self, _filters: Filters) -> Vec<Function> {
        todo!()
    }

    async fn get(&self, _id: FunctionId) -> Option<Function> {
        todo!()
    }
}

impl FromStr for FunctionUrl {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let set = RegexSet::new(&[
            "^https://.*",
            "^grpc://.*",
            "^firm://.*",
            "^[a-z][a-z0-9\\-]{2,}(:.*)?$", // function_name:params
        ])
        .unwrap();
        let matches: Vec<_> = set.matches(s).into_iter().collect();
        match matches.as_slice() {
            &[0] | &[1] | &[0, 3] | &[1, 3] => Url::parse(&format!("firm+{}", s))
                .map_err(|e| e.to_string())
                .map(FunctionUrl),
            firm @ &[3] | firm @ &[2] | firm @ &[2, 3] => {
                let split = s
                    .rsplit_once(':')
                    .map(|(begin, version)| {
                        (
                            begin,
                            format!(
                                "?{}",
                                url::form_urlencoded::Serializer::new(String::new())
                                    .append_pair("version", version)
                                    .finish()
                            ),
                        )
                    })
                    .unwrap_or_else(|| (s, String::new()));
                Url::parse(&format!(
                    "{}{}{}",
                    {
                        if firm == [3] {
                            "firm://"
                        } else {
                            ""
                        }
                    },
                    split.0,
                    split.1
                ))
                .map_err(|e| e.to_string())
                .map(FunctionUrl)
            }
            _ => Url::parse(&if s.starts_with("file://") {
                Ok(s.to_string())
            } else if s.starts_with('/') {
                Ok(format!("file://{}", s))
            } else {
                std::env::current_dir()
                    .map_err(|e| e.to_string())
                    .map(|working_dir| {
                        format!(
                            "file://{}",
                            working_dir.join(s).into_os_string().to_string_lossy()
                        )
                    })
            }?)
            .map_err(|e| e.to_string())
            .and_then(|url| {
                if url.query().is_some() {
                    Url::parse(&format!("firm+{}", url))
                        .map_err(|e| e.to_string())
                        .map(FunctionUrl)
                } else {
                    Ok(FunctionUrl(url))
                }
            }),
        }
    }
}

#[async_trait::async_trait]
pub trait Resolver {
    async fn resolve(&self) -> Result<Option<Function>, String>;
}

pub struct FsRegistryResolver {
    inner: FsRegistry,
    id: FunctionId,
}

impl FsRegistryResolver {
    pub fn new(url: &Url) -> Self {
        let query = url.query_pairs().collect::<HashMap<_, _>>();
        Self {
            inner: FsRegistry::new(PathBuf::from(url.path())),
            id: FunctionId {
                name: query
                    .get("function")
                    .expect("TODO: produce error, function name is required")
                    .clone()
                    .into_owned(),
                version: query
                    .get("version")
                    .map(|s| s.clone().into_owned())
                    .unwrap_or_default(),
            },
        }
    }
}

#[async_trait::async_trait]
impl Resolver for FsRegistryResolver {
    async fn resolve(&self) -> Result<Option<Function>, String> {
        Ok(self.inner.get(self.id.clone()).await)
    }
}

#[allow(dead_code)]
pub struct FsResolver {
    url: Url,
}

impl FsResolver {
    pub fn new(url: &Url) -> Self {
        Self { url: url.clone() }
    }
}

#[async_trait::async_trait]
impl Resolver for FsResolver {
    async fn resolve(&self) -> Result<Option<Function>, String> {
        let manifest = PathBuf::from(self.url.path()).join("function.toml");
        manifest
            .exists()
            .then(|| {
                OpenOptions::new()
                    .read(true)
                    .open(manifest)
                    .map_err(|e| e.to_string())
                    .and_then(|file| {
                        function::io::function_from_toml(file).map_err(|e| e.to_string())
                    })
            })
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::FunctionUrl;

    #[test]
    fn test_url_parsing() {
        let cd = std::env::current_dir().unwrap();

        let uri = "a/file/path";
        let url = uri.parse::<FunctionUrl>();
        assert!(url.is_ok());
        let url = url.unwrap();
        assert_eq!(url.scheme(), "file");
        assert_eq!(
            url.path(),
            cd.join("a/file/path").into_os_string().to_string_lossy()
        );

        let uri = "a/file/path?function=sune";
        let url = uri.parse::<FunctionUrl>();
        assert!(url.is_ok());
        let url = url.unwrap();
        assert_eq!(url.scheme(), "firm+file");
        assert_eq!(
            url.path(),
            cd.join("a/file/path").into_os_string().to_string_lossy()
        );

        let uri = "function-name:1.2.3";
        let url = uri.parse::<FunctionUrl>();
        assert!(url.is_ok());
        let url = url.unwrap();
        assert_eq!(url.scheme(), "firm");

        let uri = "path-or-is-it";
        let url = uri.parse::<FunctionUrl>();
        assert!(url.is_ok());
        let url = url.unwrap();
        assert_eq!(url.scheme(), "firm");

        let uri = "./this-is-a-path";
        let url = uri.parse::<FunctionUrl>();
        assert!(url.is_ok());
        let url = url.unwrap();
        assert_eq!(url.scheme(), "file");

        let uri = "firm://function-name:>1.2.3";
        let url = uri.parse::<FunctionUrl>();
        assert!(url.is_ok());
        let url = url.unwrap();
        assert_eq!(url.scheme(), "firm");

        let uri = "grpc://some.registry.com/function-name/1.2.3";
        let url = uri.parse::<FunctionUrl>();
        assert!(url.is_ok());
        let url = url.unwrap();
        assert_eq!(url.scheme(), "firm+grpc");

        let uri = "https://some.other.registry.com/function-name/1.2.3";
        let url = uri.parse::<FunctionUrl>();
        assert!(url.is_ok());
        let url = url.unwrap();
        assert_eq!(url.scheme(), "firm+https");
    }
}
