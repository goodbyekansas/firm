use ansi_term::Colour::{Green, Red};
use firm_types::{
    auth::{
        authentication_client::AuthenticationClient, Ordering as ProtoOrdering, OrderingKey,
        RemoteAccessApproval, RemoteAccessListParameters, RemoteAccessRequestId,
    },
    tonic,
};

use crate::{error, formatting::DisplayExt, Ordering};
use error::BendiniError;

impl From<Ordering> for i32 {
    fn from(ordering: Ordering) -> Self {
        (match ordering {
            Ordering::ExpiresAt => OrderingKey::ExpiresAt,
            Ordering::Subject => OrderingKey::Subject,
        }) as i32
    }
}

pub async fn list(
    mut client: AuthenticationClient<tonic::transport::Channel>,
    subject_filter: String,
    include_approved: bool,
    ordering: Ordering,
) -> Result<(), BendiniError> {
    let requests = client
        .list_remote_access_requests(tonic::Request::new(RemoteAccessListParameters {
            subject_filter,
            include_approved,
            order: Some(ProtoOrdering {
                key: ordering.into(),
                reverse: false,
                offset: 0,
                limit: 21,
            }),
        }))
        .await?
        .into_inner()
        .requests;
    print!(
        "{}",
        (&requests[..(std::cmp::min(requests.len(), 20))]).display()
    );
    if requests.len() > 20 {
        println!("...")
    }
    Ok(())
}

pub async fn approval(
    mut client: AuthenticationClient<tonic::transport::Channel>,
    approve: bool,
    id: String,
) -> Result<(), BendiniError> {
    client
        .approve_remote_access_request(tonic::Request::new(RemoteAccessApproval {
            approved: approve,
            id: Some(RemoteAccessRequestId { uuid: id.clone() }),
        }))
        .await
        .map(|_| {
            println!(
                "{} request with id {}",
                if approve {
                    Green.paint("Approved")
                } else {
                    Red.paint("Declined")
                },
                id
            );
        })
        .map_err(|e| e.into())
}
