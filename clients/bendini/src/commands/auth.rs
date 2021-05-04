use ansi_term::Colour::{Green, Red};
use firm_types::{
    auth::RemoteAccessRequest,
    auth::{
        authentication_client::AuthenticationClient, Ordering as ProtoOrdering, OrderingKey,
        RemoteAccessApproval, RemoteAccessListParameters,
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

async fn list_internal(
    mut client: AuthenticationClient<tonic::transport::Channel>,
    subject_filter: String,
    include_approved: bool,
    ordering: Ordering,
    limit: u32,
    offset: u32,
) -> Result<Vec<RemoteAccessRequest>, tonic::Status> {
    client
        .list_remote_access_requests(tonic::Request::new(RemoteAccessListParameters {
            subject_filter,
            include_approved,
            order: Some(ProtoOrdering {
                key: ordering.into(),
                reverse: false,
                offset,
                limit,
            }),
        }))
        .await
        .map(|r| r.into_inner().requests)
}

fn shorten_ids(mut requests: Vec<RemoteAccessRequest>) -> Vec<RemoteAccessRequest> {
    requests.iter_mut().for_each(|request| {
        if let Some(id) = request.id.as_mut() {
            id.uuid = id.uuid[..8].to_owned()
        };
    });
    requests
}

pub async fn list(
    client: AuthenticationClient<tonic::transport::Channel>,
    subject_filter: String,
    include_approved: bool,
    ordering: Ordering,
) -> Result<(), BendiniError> {
    let requests = shorten_ids(
        list_internal(client, subject_filter, include_approved, ordering, 21, 0).await?,
    );
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
    short_id: String,
) -> Result<(), BendiniError> {
    let mut hits = Vec::new();
    loop {
        let requests = list_internal(
            client.clone(),
            String::new(),
            false,
            Ordering::default(),
            99,
            0,
        )
        .await?;

        let requests_len = requests.len();

        hits.extend(requests.into_iter().filter(|req| match req.id.as_ref() {
            Some(id) => id.uuid.starts_with(&short_id),
            None => false,
        }));

        if requests_len < 99 {
            break;
        }
    }

    match hits.len() {
        1 => client
            .approve_remote_access_request(tonic::Request::new(RemoteAccessApproval {
                approved: approve,
                id: hits[0].id.clone(),
            }))
            .await
            .map(|response| {
                println!(
                    r#"{} request with id "{}""#,
                    if approve {
                        Green.paint("Approved")
                    } else {
                        Red.paint("Declined")
                    },
                    response
                        .into_inner()
                        .id
                        .map(|id| id.uuid)
                        .unwrap_or_default()
                );
            })
            .map_err(|e| e.into()),

        x if x > 1 => {
            eprintln!(
                "{}",
                crate::error!(
                    r#"Id "{}" is ambiguous, specify full id. These are the matching requests:"#,
                    short_id
                )
            );
            println!("{}", hits.as_slice().display());
            Ok(())
        }

        _ => {
            eprintln!(
                "{}",
                crate::error!(
                    r#"Could not find a request with id matching "{}", "#,
                    short_id
                )
            );
            Ok(())
        }
    }
}
