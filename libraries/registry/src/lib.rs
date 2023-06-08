use std::str::FromStr;

use firm_protocols::functions::Function;
use url::Url;

pub struct FunctionUrl(Url);

/// Resolve a function from a URL
pub fn resolve(url: Url) -> Result<Option<Function>, String> {
    match url.scheme() {
        "file" => FsResolver::new(url).resolve(),
        x => panic!("unsupported transport {}", x),
    }
}

impl FromStr for FunctionUrl {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        todo!("parse URL from {} like fn:>=1.0.0 -> firm://fn?version=largerThanOrEq_1_0_0, ./path/something/something -> file://./path/something/something", s);
    }
}

pub trait Resolver {
    fn resolve(&self) -> Result<Option<Function>, String>;
}

pub struct FsResolver {
    url: Url,
}

impl FsResolver {
    pub fn new(url: Url) -> Self {
        Self { url }
    }
}

impl Resolver for FsResolver {
    fn resolve(&self) -> Result<Option<Function>, String> {
        todo!("Parse a function from {}", self.url);
    }
}
