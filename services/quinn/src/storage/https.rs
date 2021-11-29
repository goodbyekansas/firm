use url::Url;

use super::{AttachmentStorage, AttachmentUrl, AuthMethod, FunctionAttachment, StorageError};

#[derive(Debug)]
pub struct HttpsStorage {
    base: Url,
    auth_method: AuthMethod,
}

impl HttpsStorage {
    pub fn new(base: &Url, auth: AuthMethod) -> Result<Self, StorageError> {
        if base.cannot_be_a_base() || !base.path().ends_with('/') {
            Err(StorageError::BackendError(
                "Url provided to Https Storage can not be used as a base".into(),
            ))
        } else {
            Ok(Self {
                base: base.clone(),
                auth_method: auth,
            })
        }
    }
}

impl AttachmentStorage for HttpsStorage {
    fn get_upload_url(
        &self,
        attachment: &FunctionAttachment,
    ) -> Result<AttachmentUrl, StorageError> {
        Ok(AttachmentUrl {
            url: self.base.join(&attachment.id.to_string())?.to_string(),
            auth_method: self.auth_method as i32,
        })
    }

    fn get_download_url(
        &self,
        attachment: &FunctionAttachment,
    ) -> Result<AttachmentUrl, StorageError> {
        Ok(AttachmentUrl {
            url: self.base.join(&attachment.id.to_string())?.to_string(),
            auth_method: self.auth_method as i32,
        })
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::storage::{Checksums, FunctionAttachmentData, Publisher};
    use std::collections::HashMap;

    #[test]
    fn https_bucket_url() {
        let uuid_str = "1a52540c-7edd-4f9e-916b-f9aaf165890e";

        let expected_storage_path = format!("https://example.net/submarine/{}", uuid_str);
        let base: Url = Url::parse("https://example.net/submarine/").unwrap();
        let https_storage = HttpsStorage::new(&base, AuthMethod::Oauth2).unwrap();

        let res = https_storage.get_upload_url(&FunctionAttachment {
            id: Uuid::parse_str(uuid_str).unwrap(),
            data: FunctionAttachmentData {
                name: "Nej".to_owned(),
                metadata: HashMap::new(),
                checksums: Checksums {
                    sha256: "nej".to_owned(),
                },
                publisher: Publisher {
                    name: "Sju Rygg".to_owned(),
                    email: "sju@fetfisk.se".to_owned(),
                },
                signature: None,
            },
            created_at: 1337,
        });

        assert!(res.is_ok());
        let resp = res.unwrap();
        assert_eq!(resp.url, expected_storage_path);
        assert_eq!(resp.auth_method, super::AuthMethod::Oauth2 as i32);

        // test host-only url
        let expected_storage_path = format!("https://example.net/{}", uuid_str);
        let base: Url = Url::parse("https://example.net").unwrap();
        let https_storage = HttpsStorage::new(&base, AuthMethod::Oauth2).unwrap();

        let res = https_storage.get_upload_url(&FunctionAttachment {
            id: Uuid::parse_str(uuid_str).unwrap(),
            data: FunctionAttachmentData {
                name: "Nej".to_owned(),
                metadata: HashMap::new(),
                checksums: Checksums {
                    sha256: "nej".to_owned(),
                },
                publisher: Publisher {
                    name: "Horn Simpa".to_owned(),
                    email: "hornsimpa@fulfisk.se".to_owned(),
                },
                signature: None,
            },
            created_at: 1337,
        });

        assert!(res.is_ok());
        let resp = res.unwrap();
        assert_eq!(resp.url, expected_storage_path);
        assert_eq!(resp.auth_method, super::AuthMethod::Oauth2 as i32);
    }

    #[test]
    fn https_bad_url() {
        let base: Url = Url::parse("https://example.net/submarine").unwrap();
        let https_storage = HttpsStorage::new(&base, AuthMethod::Oauth2);
        assert!(https_storage.is_err());
    }
}
