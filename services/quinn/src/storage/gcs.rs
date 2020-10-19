use url::Url;

use super::{AttachmentStorage, FunctionAttachment, StorageError};
use gbk_protocols::functions::{AttachmentUploadResponse, AuthMethod};

#[derive(Debug)]
pub struct GCSStorage {
    bucket_name: String,
}

impl GCSStorage {
    pub fn new(bucket_name: &str) -> Self {
        Self {
            bucket_name: bucket_name.to_owned(),
        }
    }

    fn get_object_name(&self, attachment: &FunctionAttachment) -> String {
        attachment.id.to_string()
    }
}

impl AttachmentStorage for GCSStorage {
    fn get_upload_url(
        &self,
        attachment: &FunctionAttachment,
    ) -> Result<AttachmentUploadResponse, StorageError> {
        Ok(AttachmentUploadResponse {
            url: format!(
                "https://storage.googleapis.com/upload/storage/v1/b/{bucket_name}/o?uploadType=media&name={object_name}",
                bucket_name = self.bucket_name,
                object_name = self.get_object_name(attachment),
            ),
            auth_method: AuthMethod::Oauth2 as i32,
        })
    }

    fn get_download_url(&self, attachment: &FunctionAttachment) -> Result<Url, StorageError> {
        Ok(Url::parse(&format!(
            "https://storage.googleapis.com/storage/v1/b/{bucket_name}/o/{object_name}?alt=media",
            bucket_name = self.bucket_name,
            object_name = self.get_object_name(attachment),
        ))?)
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::storage::{Checksums, FunctionAttachmentData};
    use std::collections::HashMap;

    #[test]
    fn gcs_bucket_url() {
        let uuid_str = "1a52540c-7edd-4f9e-916b-f9aaf165890e";
        let bucket_name = "hinken";

        let expected_storage_path = format!(
            "https://storage.googleapis.com/upload/storage/v1/b/{}/o?uploadType=media&name={}",
            bucket_name, uuid_str
        );
        let gcs_storage = GCSStorage::new(bucket_name);

        let res = gcs_storage.get_upload_url(&FunctionAttachment {
            id: Uuid::parse_str(uuid_str).unwrap(),
            function_ids: vec![],
            data: FunctionAttachmentData {
                name: "Nej".to_owned(),
                metadata: HashMap::new(),
                checksums: Checksums {
                    sha256: "nej".to_owned(),
                },
            },
        });

        assert!(res.is_ok());
        let resp = res.unwrap();
        assert_eq!(resp.url, expected_storage_path);
        assert_eq!(resp.auth_method, super::AuthMethod::Oauth2 as i32);
    }
}
