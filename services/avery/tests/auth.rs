use std::collections::HashSet;

use chrono::Utc;
use firm_types::{auth::authentication_server::Authentication, tonic};
use serde::Serialize;

use avery::auth::{AuthService, KeyStore, KeyStoreError};

const PRIVATE_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgiuNp+s23UTotSsEXctwtU0HAA7IHvodB8Q+KA7cW5AuhRANCAASFpp3A7q4Zjtnin9pDoSMzppIczS+O5UkeKM6Wr8HghHI/moGdWYkbGqUPnd2JTmz8YbpGoXz2KewpRQ4no4cx
-----END PRIVATE KEY-----
"#;

const PUBLIC_KEY: &str = r#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEhaadwO6uGY7Z4p/aQ6EjM6aSHM0vjuVJHijOlq/B4IRyP5qBnVmJGxqlD53diU5s/GG6RqF89insKUUOJ6OHMQ==
-----END PUBLIC KEY-----"#;

struct FakeKeyStore {
    key_data: Vec<u8>,
}

#[async_trait::async_trait]
impl KeyStore for FakeKeyStore {
    async fn get(&self, _id: &str) -> Result<Vec<u8>, KeyStoreError> {
        Ok(self.key_data.clone())
    }

    async fn set(&self, _id: &str, _key_data: &[u8]) -> Result<(), KeyStoreError> {
        Ok(())
    }
}

macro_rules! auth_service {
    () => {{
        auth_service!(FakeKeyStore {
            key_data: PUBLIC_KEY.as_bytes().to_vec(),
        })
    }};

    ($keystore:expr) => {{
        AuthService::new(Box::new($keystore))
    }};
}

macro_rules! auth_request {
    ($auth_service:expr, $subject:expr, $encoding_key:expr) => {{
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
        header.kid = Some(String::from("key-id"));

        $auth_service
            .authenticate(tonic::Request::new(
                firm_types::auth::AuthenticationParameters {
                    expected_audience: String::from("publiken"),
                    token: jsonwebtoken::encode(
                        &header,
                        &TestClaims {
                            aud: String::from("publiken"),
                            exp: (Utc::now().timestamp() + 1234) as u64,
                            sub: String::from($subject),
                        },
                        &$encoding_key,
                    )
                    .unwrap(),
                    create_remote_access_request: true,
                },
            ))
            .await
            .unwrap()
            .into_inner()
    }};
}

#[derive(Serialize)]
struct TestClaims {
    aud: String,
    exp: u64,
    sub: String,
}

#[tokio::test]
async fn authenticate() {
    const BAD_PRIVATE_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgv6FgVg2nDbcAvzC5zkG08ITR0czcjeN/y1g/0ggIdtOhRANCAARgG4M/Bd58ts9rGQHw7oL7SK1DMNNpKiY86tv2GM2Q1SHH9iY+FpQxkYbnuyf05u8+OqD5pv0UcfX9r57luz9+
-----END PRIVATE KEY-----"#;

    let mut auth_service = auth_service!();
    let encoding_key = jsonwebtoken::EncodingKey::from_ec_pem(PRIVATE_KEY.as_bytes()).unwrap();
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
    header.kid = Some(String::from("key-id"));

    // Test invalid token error
    assert!(
        matches!(
            auth_service
                .authenticate(tonic::Request::new(
                    firm_types::auth::AuthenticationParameters {
                        expected_audience: String::from("publiken"),
                        token: String::from("dgfijjw4iog89e4wjgdj94edg8904"),
                        create_remote_access_request: false,
                    },
                ))
                .await, Err(e) if e.code() == tonic::Code::InvalidArgument),
        "invalid token must generate invalid argument error"
    );

    // Test expiry date
    assert!(
        matches!(
            auth_service
                .authenticate(tonic::Request::new(
                    firm_types::auth::AuthenticationParameters {
                        expected_audience: String::from("publiken"),
                        token: jsonwebtoken::encode(
                            &header,
                            &TestClaims {
                                aud: String::from("publiken"),
                                exp: 0u64,
                                sub: String::from("marine"),
                            },
                            &encoding_key,
                        )
                        .unwrap(),
                        create_remote_access_request: false,
                    },
                ))
                .await, Err(e) if e.code() == tonic::Code::InvalidArgument),
        "expired token must generate invalid argument error"
    );

    // Audience
    assert!(
        matches!(
            auth_service
                .authenticate(tonic::Request::new(
                    firm_types::auth::AuthenticationParameters {
                        expected_audience: String::from("åskådare"),
                        token: jsonwebtoken::encode(
                            &header,
                            &TestClaims {
                                aud: String::from("läsekrets"),
                                exp: (Utc::now().timestamp() + 1234) as u64,
                                sub: String::from("u-boat"),
                            },
                            &encoding_key,
                        )
                        .unwrap(),
                        create_remote_access_request: false,
                    },
                ))
                .await, Err(e) if e.code() == tonic::Code::InvalidArgument),
        "audience mismatch must generate invalid argument error"
    );

    // Check auth with wrong private key
    assert!(
        matches!(
            auth_service
                .authenticate(tonic::Request::new(
                    firm_types::auth::AuthenticationParameters {
                        expected_audience: String::from("ja"),
                        token: jsonwebtoken::encode(
                            &header,
                            &TestClaims {
                                aud: String::from("ja"),
                                exp: (Utc::now().timestamp() + 1234) as u64,
                                sub: String::from("u-boat"),
                            },
                            &jsonwebtoken::EncodingKey::from_ec_pem(BAD_PRIVATE_KEY.as_bytes()).unwrap(),
                        )
                        .unwrap(),
                        create_remote_access_request: false,
                    },
                ))
                .await, Err(e) if e.code() == tonic::Code::InvalidArgument),
        "signing key mismatch must generate invalid argument error"
    );

    // Check token permission failure
    assert!(
        matches!(auth_service
            .authenticate(tonic::Request::new(
                firm_types::auth::AuthenticationParameters {
                    expected_audience: String::from("publiken"),
                    token: jsonwebtoken::encode(
                        &header,
                        &TestClaims {
                            aud: String::from("publiken"),
                            exp: (Utc::now().timestamp() + 1234) as u64,
                            sub: String::from("system"),
                        },
                        &encoding_key,
                    )
                    .unwrap(),
                    create_remote_access_request: false,
                },
            ))
                     .await, Err(e) if e.code() == tonic::Code::PermissionDenied),
        "Token without access must generate permission denied error"
    );

    // Test upsert
    assert!(
        matches!(
            auth_service
                .authenticate(tonic::Request::new(
                    firm_types::auth::AuthenticationParameters {
                        expected_audience: String::from("publiken"),
                        token: jsonwebtoken::encode(
                            &header,
                            &TestClaims {
                                aud: String::from("publiken"),
                                exp: (Utc::now().timestamp() + 1234) as u64,
                                sub: String::from("user@host"),
                            },
                            &encoding_key,
                        )
                        .unwrap(),
                        create_remote_access_request: true,
                    },
                ))
                .await,
            Ok(resp) if resp.get_ref().remote_access_request_id.is_some()
        ),
        "Token without subject access and wants to upsert must yield an ok response that contains a remote access request id",
    );

    // Test static access
    let mut access_list = HashSet::new();
    access_list.insert("user@host".to_owned());
    auth_service.with_access_list(access_list);

    assert!(
        matches!(
            auth_service
                .authenticate(tonic::Request::new(
                    firm_types::auth::AuthenticationParameters {
                        expected_audience: String::from("publiken"),
                        token: jsonwebtoken::encode(
                            &header,
                            &TestClaims {
                                aud: String::from("publiken"),
                                exp: (Utc::now().timestamp() + 1234) as u64,
                                sub: String::from("user@host"),
                            },
                            &encoding_key,
                        )
                        .unwrap(),
                        create_remote_access_request: true,
                    },
                ))
                .await,
            Ok(resp) if resp.get_ref().remote_access_request_id.is_none()
        ),
        "Token with subject access must yield an ok response without a remote access id"
    );
}

#[tokio::test]
async fn list() {
    let auth_service = auth_service!();
    let encoding_key = jsonwebtoken::EncodingKey::from_ec_pem(PRIVATE_KEY.as_bytes()).unwrap();
    let user_id = auth_request!(auth_service, "user@host", encoding_key);
    auth_request!(auth_service, "diffuser@host", encoding_key);
    let denied_user_id = auth_request!(auth_service, "nobody@host", encoding_key);
    auth_request!(auth_service, "fuser@host", encoding_key);
    auth_request!(auth_service, "defuser@host", encoding_key);

    let resp = auth_service
        .list_remote_access_requests(tonic::Request::new(
            firm_types::auth::RemoteAccessListParameters {
                subject_filter: String::new(),
                include_approved: true,
                order: None,
            },
        ))
        .await;
    assert!(resp.is_ok());
    assert_eq!(
        resp.unwrap().into_inner().requests.len(),
        5,
        "Expected to get all five inserted requests"
    );

    // Test sorting and filtering
    let resp = auth_service
        .list_remote_access_requests(tonic::Request::new(
            firm_types::auth::RemoteAccessListParameters {
                subject_filter: String::from("user"),
                include_approved: true,
                order: None,
            },
        ))
        .await;
    assert!(resp.is_ok());
    let requests = resp.unwrap().into_inner().requests;
    assert_eq!(
        requests.len(),
        4,
        r#"Expected to get the four inserted requests that matches "user""#
    );
    assert!(
        !requests.iter().any(|r| r.subject == "nobody@host"),
        r#"expected the user "nobody" to be filtered out"#
    );

    assert_eq!(requests[0].subject, "defuser@host");
    assert_eq!(requests[1].subject, "diffuser@host");
    assert_eq!(requests[2].subject, "fuser@host");
    assert_eq!(requests[3].subject, "user@host");

    // Test revers sorting
    let requests = auth_service
        .list_remote_access_requests(tonic::Request::new(
            firm_types::auth::RemoteAccessListParameters {
                subject_filter: String::new(),
                include_approved: true,
                order: Some(firm_types::auth::Ordering {
                    reverse: true,
                    ..Default::default()
                }),
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .requests;

    assert_eq!(
        requests.len(),
        5,
        "Expected limit to ensure the count was exactly 5."
    );
    assert_eq!(requests[0].subject, "user@host");
    assert_eq!(requests[1].subject, "nobody@host");
    assert_eq!(requests[2].subject, "fuser@host");
    assert_eq!(requests[3].subject, "diffuser@host");
    assert_eq!(requests[4].subject, "defuser@host");

    // Test limit and offset
    let requests = auth_service
        .list_remote_access_requests(tonic::Request::new(
            firm_types::auth::RemoteAccessListParameters {
                subject_filter: String::new(),
                include_approved: true,
                order: Some(firm_types::auth::Ordering {
                    reverse: true,
                    limit: 2,
                    offset: 1,
                    ..Default::default()
                }),
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .requests;

    assert_eq!(
        requests.len(),
        2,
        "Expected limit to ensure the count was exactly 2."
    );
    assert_eq!(requests[0].subject, "nobody@host");
    assert_eq!(requests[1].subject, "fuser@host");

    // Test approving a user and filter by it.
    let approval = auth_service
        .approve_remote_access_request(tonic::Request::new(
            firm_types::auth::RemoteAccessApproval {
                approved: true,
                id: user_id.remote_access_request_id.clone(),
            },
        ))
        .await;

    assert!(approval.is_ok(), "Expected approval request to succeed");
    let approval = approval.unwrap().into_inner();

    //Expiry date will be different each time which is why we skip checking it
    assert_eq!(
        approval.id, user_id.remote_access_request_id,
        "Approval id and remote access request id must be the same."
    );
    assert_eq!(
        approval.subject,
        String::from("user@host"),
        "Expected to grant approval for the subject coupled with the remote access request id."
    );
    assert!(approval.approved, "Expected approval to approve.");

    // Test if aproved requests are filtered out.
    let requests = auth_service
        .list_remote_access_requests(tonic::Request::new(
            firm_types::auth::RemoteAccessListParameters {
                subject_filter: String::new(),
                include_approved: false,
                order: Some(firm_types::auth::Ordering::default()),
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .requests;

    assert_eq!(requests.len(), 4, "Expected there to be 4 requests left.");
    assert!(
        !requests.iter().any(|r| r.subject == "user@host"),
        "Expected user@host to be filtered out"
    );

    let requests = auth_service
        .list_remote_access_requests(tonic::Request::new(
            firm_types::auth::RemoteAccessListParameters {
                subject_filter: String::new(),
                include_approved: true,
                order: Some(firm_types::auth::Ordering::default()),
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .requests;

    assert_eq!(requests.len(), 5, "Expected there to be 5 requests.");
    assert!(
        requests.iter().any(|r| r.subject == "user@host"),
        "Expected user@host to be in results"
    );

    // Test rejecting a user
    let denial = auth_service
        .approve_remote_access_request(tonic::Request::new(
            firm_types::auth::RemoteAccessApproval {
                approved: false,
                id: denied_user_id.remote_access_request_id.clone(),
            },
        ))
        .await;

    assert!(denial.is_ok(), "Expected denial request to succeed");
    let denial = denial.unwrap().into_inner();

    assert_eq!(
        denial.id, denied_user_id.remote_access_request_id,
        "Approval id and remote access request id must be the same."
    );
    assert_eq!(
        denial.subject,
        String::from("nobody@host"),
        "Expected to deny the subject coupled with the remote access request id."
    );
    assert!(!denial.approved, "Expected denial to deny.");

    let requests = auth_service
        .list_remote_access_requests(tonic::Request::new(
            firm_types::auth::RemoteAccessListParameters {
                subject_filter: String::new(),
                include_approved: true,
                order: Some(firm_types::auth::Ordering::default()),
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .requests;

    assert_eq!(requests.len(), 5, "Expected there to be 5 requests left.");
    assert!(
        requests.iter().any(|r| r.subject == "nobody@host"),
        "Expected nobody@host to still be in the list even if it was denied."
    );

    // Testing get
    let response = auth_service
        .get_remote_access_request(tonic::Request::new(
            user_id.remote_access_request_id.clone().unwrap(),
        ))
        .await;

    assert!(
        response.is_ok(),
        "Expected to be able to get a remote access request without error."
    );
    let response = response.unwrap().into_inner();
    assert_eq!(
        response.id, user_id.remote_access_request_id,
        "Expected to get a request with the same id as requested."
    );
    assert_eq!(
        response.subject,
        String::from("user@host"),
        "Expected subject to be of the requested remote access request."
    );
}
