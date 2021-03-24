use std::path::PathBuf;

use firm_types::tonic::{
    metadata::{errors::InvalidMetadataValue, AsciiMetadataValue},
    Request, Status,
};

#[derive(Clone, Debug)]
pub struct UserInfo {
    pub username: String,
    pub home_dir: PathBuf,
}

pub trait RequestUserInfoExt: Sized {
    fn get_user_info(&self) -> Option<UserInfo>;
    fn with_user_info(self, user_info: &UserInfo) -> Result<Self, InvalidMetadataValue>;
}

impl<T> RequestUserInfoExt for Request<T> {
    fn get_user_info(&self) -> Option<UserInfo> {
        Some(UserInfo {
            username: self
                .metadata()
                .get("username")
                .and_then(|v| v.to_str().map(|s| s.to_owned()).ok())?,
            home_dir: PathBuf::from(
                self.metadata()
                    .get("home_dir")
                    .and_then(|v| v.to_str().ok())?,
            ),
        })
    }

    fn with_user_info(mut self, user_info: &UserInfo) -> Result<Self, InvalidMetadataValue> {
        let metadata = self.metadata_mut();
        metadata.insert(
            "username",
            AsciiMetadataValue::from_str(&user_info.username)?,
        );
        metadata.insert(
            "home_dir",
            AsciiMetadataValue::from_str(&user_info.home_dir.to_string_lossy())?,
        );
        Ok(self)
    }
}

pub trait IntoTonicStatus: Sized {
    fn into_tonic_status(self) -> Result<UserInfo, Status>;
}

impl IntoTonicStatus for Option<UserInfo> {
    fn into_tonic_status(self) -> Result<UserInfo, Status> {
        self.ok_or_else(|| {
            Status::unauthenticated("Failed to find user information in grpc metadata")
        })
    }
}
