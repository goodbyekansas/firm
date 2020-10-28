use firm_protocols::tonic::Status;
use thiserror::Error;

use crate::manifest::ManifestError;

#[derive(Error, Debug)]
pub enum BendiniError {
    #[error("Unknown error: {message}")]
    Unknown { message: String, exit_code: i32 },

    #[error("Remote API error: {} ({})", .status.message(), .status.code())]
    APIError {
        #[from]
        status: Status,
    },

    #[error("Failed to find function with name \"{name}\" and version constraint \"{version}\".")]
    FailedToFindFunction { name: String, version: String },

    #[error("Failed to parse function specifier \"{0}\"")]
    FailedToParseFunction(String),

    #[error("{0}")]
    FailedToParseManifest(#[from] ManifestError),

    #[error("Failed to upload attachment \"{0}\": {1}")]
    FailedToUploadAttachment(String, String),

    #[error("Failed to register function \"{0}\": {1}")]
    FailedToRegisterFunction(String, String),

    #[error("Invalid URI specified: {0}")]
    InvalidUri(String),

    #[error("Failed to create TLS config: {0}")]
    FailedToCreateTlsConfig(String),

    #[error("Connection error connecting to \"{0}\": {1}")]
    ConnectionError(String, String),

    #[error("OAUTH token was invalid: {0}")]
    InvalidOauthToken(String),

    #[error("Invalid arguments supplied to function \"{0}\": {1:#?}")]
    InvalidFunctionArguments(String, Vec<String>),
}

impl From<BendiniError> for i32 {
    fn from(bendini_error: BendiniError) -> Self {
        match bendini_error {
            BendiniError::Unknown {
                message: _,
                exit_code,
            } => exit_code,
            BendiniError::APIError { .. } => 5i32,
            BendiniError::FailedToFindFunction { .. } => 6i32,
            BendiniError::FailedToParseFunction(_) => 7i32,
            BendiniError::FailedToParseManifest { .. } => 8i32,
            BendiniError::FailedToUploadAttachment(..) => 9i32,
            BendiniError::FailedToRegisterFunction(..) => 10i32,
            BendiniError::InvalidUri(_) => 11i32,
            BendiniError::FailedToCreateTlsConfig(_) => 12i32,
            BendiniError::ConnectionError(_, _) => 13i32,
            BendiniError::InvalidOauthToken(_) => 14i32,
            BendiniError::InvalidFunctionArguments(_, _) => 15i32,
        }
    }
}
