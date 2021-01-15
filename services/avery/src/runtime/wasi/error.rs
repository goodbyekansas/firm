use std::{error::Error, io};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum WasiError {
    #[error("Unknown: {0}")]
    Unknown(String),

    #[error("Sandbox Error: {0}")]
    SandboxError(String),

    #[error("Failed to setup std IO: {0}")]
    FailedToSetupStdIO(io::Error),

    #[error("Failed to read string pointer for \"{0}\": {1}")]
    FailedToReadStringPointer(String, std::str::Utf8Error),

    #[error("Failed to find key: {0}")]
    FailedToFindKey(String),

    #[error("Failed to deref pointer.")]
    FailedToDerefPointer(),

    #[error("Failed to start process: {0}.")]
    FailedToStartProcess(io::Error),

    #[error("Failed to decode value from protobuf: {0}")]
    FailedToDecodeProtobuf(#[from] prost::DecodeError),

    #[error("Failed to encode value from protobuf: {0}")]
    FailedToEncodeProtobuf(#[from] prost::EncodeError),

    #[error("Failed to open file descriptor: {0}")]
    FailedToOpenFile(String),

    #[error("Failed to connect to address \"{0}\". IO Error: {1}")]
    FailedToConnect(String, io::Error),

    #[error("Failed to map attachment \"{0}\": {1}")]
    FailedToMapAttachment(String, Box<dyn Error>),

    #[error("Failed to unpack attachment \"{0}\": {1}")]
    FailedToUnpackAttachment(String, Box<dyn Error>),

    #[error("Failed to find attachment \"{0}\"")]
    FailedToFindAttachment(String),

    #[error("Failed to write to WASI buffer: {0}")]
    FailedToWriteBuffer(std::io::Error),

    #[error("Failed to read WASI buffer: {0}")]
    FailedToReadBuffer(std::io::Error),
}

pub type WasiResult<T> = std::result::Result<T, WasiError>;

pub trait ToErrorCode<T> {
    fn to_error_code(self) -> u32;
}

impl<T> ToErrorCode<T> for WasiResult<T> {
    fn to_error_code(self) -> u32 {
        match self {
            Ok(_) => 0,
            Err(e) => e.into(),
        }
    }
}

impl From<WasiError> for u32 {
    fn from(err: WasiError) -> Self {
        match err {
            WasiError::Unknown(_) => 1,
            WasiError::FailedToDerefPointer() => 2,
            WasiError::FailedToDecodeProtobuf(_) => 3,
            WasiError::FailedToReadStringPointer(..) => 5,
            WasiError::FailedToFindKey(_) => 6,
            WasiError::FailedToEncodeProtobuf(_) => 7,
            WasiError::FailedToStartProcess(_) => 8,
            WasiError::FailedToOpenFile(_) => 9,
            WasiError::FailedToConnect(..) => 10,
            WasiError::FailedToMapAttachment(..) => 11,
            WasiError::FailedToFindAttachment(_) => 12,
            WasiError::SandboxError(_) => 13,
            WasiError::FailedToSetupStdIO(_) => 14,
            WasiError::FailedToUnpackAttachment(..) => 15,
            WasiError::FailedToWriteBuffer(..) => 16,
            WasiError::FailedToReadBuffer(..) => 17,
        }
    }
}
