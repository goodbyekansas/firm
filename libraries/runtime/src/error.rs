use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("Failed to create IO factory: {0}")]
    FailedToCreateIoFactory(String),

    #[error("Can not create IO operation for unknown id: {0}")]
    UnknownIoid(u64),

    #[error("Failed to read store directory \"{0}\": {1}")]
    FailedToReadStoreDirectory(PathBuf, String),

    #[error("Failed to parse {what} from string \"{content}\": {error}")]
    FailedToParseFromStringError {
        what: &'static str,
        content: String,
        error: String,
    },

    #[error("Failed to parse {what} from string \"{content}\".")]
    FailedToParseFromString { what: &'static str, content: String },

    #[error("Failed to create runtime process: {0}")]
    FailedToCreateRuntimeProcess(String),

    #[error("Failed to create event queue: {0}")]
    FailedToCreateEventQueue(String),

    #[error("Failed to register interests: {0}")]
    FailedToRegisterInterests(String),

    #[error("Failed to poll io event queue")]
    FailedToPollQueue(String),

    #[error("Failed to deregister io event queue")]
    FailedToDeregisterQueue(String),

    #[error("Failed to create store at \"{0}\"")]
    FailedToCreateStoreDir(PathBuf, #[source] std::io::Error),

    #[error("Failed to create function directory at \"{0}\": {1}")]
    FailedToCreateFunctionDir(PathBuf, #[source] std::io::Error),

    #[error("Failed to create executions directory in function directory at \"{0}\": {1}")]
    FailedToCreateExecutionsDir(PathBuf, #[source] std::io::Error),

    #[error("Failed to create cache directory in function directory at \"{0}\": {1}")]
    FailedToCreateCacheDir(PathBuf, #[source] std::io::Error),

    #[error("Failed to create execution directory for execution \"{1}\" at \"{0}\": {2}")]
    FailedToCreateExecutionDir(PathBuf, String, #[source] std::io::Error),

    #[error("Failed to find runtime \"{0}\" in any of the paths {1}")]
    FailedToFindRuntime(String, String),

    #[error("Failed to read from runtime directory \"{0}\": {1}")]
    FailedToReadRuntimeDir(PathBuf, #[source] std::io::Error),

    #[error("Runtime spec missing for function \"{0}\"")]
    RuntimeSpecMissing(String),

    #[error("Attachment error: \"{0}\"")]
    AttachmentError(#[from] function::attachments::AttachmentError),

    #[error("Parser state error: {0}")]
    ParserStateError(String),
}
