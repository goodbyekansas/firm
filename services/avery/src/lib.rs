pub mod auth;
pub mod config;
pub mod executor;
mod oidc;
pub mod proxy_registry;
pub mod registry;
pub mod runtime;
#[cfg(unix)]
pub mod unix;
#[cfg(unix)]
pub use unix as system;

#[cfg(windows)]
pub mod windows;
#[cfg(windows)]
pub use windows as system;
