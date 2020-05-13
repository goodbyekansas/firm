use std::io;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum WasiError {
    #[error("Unknown: {0}")]
    Unknown(String),

    #[error("{0}")]
    ConversionError(String),

    #[error("Failed to read string pointer for \"{0}\"")]
    FailedToReadStringPointer(String),

    #[error("Failed to find key: {0}")]
    FailedToFindKey(String),

    #[error("Failed to deref pointer.")]
    FailedToDerefPointer(),

    #[error("Failed to start process: {0}.")]
    FailedToStartProcess(#[from] io::Error),

    #[error("Failed to decode value from protobuf: {0}")]
    FailedToDecodeProtobuf(#[from] prost::DecodeError),

    #[error("Failed to encode value from protobuf: {0}")]
    FailedToEncodeProtobuf(#[from] prost::EncodeError),

    #[error("Failed to open file descriptor: {0}")]
    FailedToOpenFile(String),

    #[error("Failed to connect to address \"{0}\". IO Error: {1}")]
    FailedToConnect(String, io::Error),
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
            WasiError::ConversionError(_) => 4,
            WasiError::FailedToReadStringPointer(_) => 5,
            WasiError::FailedToFindKey(_) => 6,
            WasiError::FailedToEncodeProtobuf(_) => 7,
            WasiError::FailedToStartProcess(_) => 8,
            WasiError::FailedToOpenFile(_) => 9,
            WasiError::FailedToConnect(..) => 10,
        }
    }
}
