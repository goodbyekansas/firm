use firm_types::{
    auth::{authentication_client::AuthenticationClient, interactive_login_command},
    tonic::{self, transport::Channel},
};
use futures::{Future, TryFutureExt, TryStreamExt};

use crate::error::BendiniError;

pub async fn with_login<Fut, Function, T>(
    mut auth_client: AuthenticationClient<Channel>,
    f: Function,
) -> Result<T, BendiniError>
where
    Function: Fn() -> Fut,
    Fut: Future<Output = Result<T, BendiniError>>,
{
    f().or_else(|e| async move {
        match e {
            BendiniError::APIError { status } if status.code() == tonic::Code::Unauthenticated => {
                auth_client
                    .login(tonic::Request::new(()))
                    .map_err(BendiniError::from)
                    .and_then(|stream| async move {
                        stream
                            .into_inner()
                            .map_err(BendiniError::from)
                            .try_for_each(|command| async move {
                                match command.command {
                                    Some(interactive_login_command::Command::Browser(b)) => {
                                        open::that(b.url)
                                            .map_err(|e| {
                                                BendiniError::FailedToOpenBrowser(e.to_string())
                                            })
                                            .and_then(|exit_status| {
                                                exit_status.success().then(|| ()).ok_or_else(|| {
                                                    BendiniError::FailedToOpenBrowser(format!(
                                                        "Starting the browser exited with \
                                                         a non-zero exit code: {:?}",
                                                        exit_status.code()
                                                    ))
                                                })
                                            })
                                    }

                                    None => Ok(()), // 🤔 Avery wanted us to do nothing
                                }
                            })
                            .and_then(|_| f())
                            .await
                    })
                    .await
            }
            _ => Err(e),
        }
    })
    .await
}
