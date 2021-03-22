use std::path::{Path, PathBuf};

use firm_types::tonic::{
    metadata::{errors::InvalidMetadataValue, AsciiMetadataValue, MetadataMap},
    Request,
};

pub fn apply_user_info<'a>(
    user_name: String,
    home_folder: &Path,
    metadata: &'a mut MetadataMap,
) -> Result<&'a MetadataMap, InvalidMetadataValue> {
    metadata.insert("username", AsciiMetadataValue::from_str(&user_name)?);
    metadata.insert(
        "home_folder",
        AsciiMetadataValue::from_str(&home_folder.to_string_lossy())?,
    );
    Ok(metadata)
}

#[derive(Clone, Debug)]
pub struct UserInfo {
    pub username: String,
    pub home_folder: PathBuf,
}

pub trait RequestUserInfo {
    fn get_user_info(&self) -> UserInfo;
}

impl<T> RequestUserInfo for Request<T> {
    fn get_user_info(&self) -> UserInfo {
        UserInfo {
            username: self
                .metadata()
                .get("username")
                .map(|v| v.to_str().unwrap_or_default().to_owned())
                .unwrap_or_default(),
            home_folder: PathBuf::from(
                self.metadata()
                    .get("home_folder")
                    .map(|v| v.to_str().unwrap_or_default())
                    .unwrap_or_default(),
            ),
        }
    }
}
