use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use firm_protocols::tonic::{
    self,
    transport::{Body, Channel},
};

#[derive(Debug, Clone)]
pub struct HttpStatusInterceptor {
    channel: Channel,
}

impl HttpStatusInterceptor {
    pub fn new(channel: Channel) -> Self {
        Self { channel }
    }
}

#[derive(Debug)]
pub struct ResponseFuture {
    inner: tonic::transport::channel::ResponseFuture,
}

impl Future for ResponseFuture {
    type Output = Result<http::Response<Body>, tonic::transport::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.inner).poll(cx) {
            Poll::Ready(res) => {
                Poll::Ready(res.map(|response| {
                    let tonic_consumable_content = response
                        .headers()
                        .get(http::header::CONTENT_TYPE)
                        .map(|cont| {
                            cont.to_str()
                                .unwrap_or_default()
                                .contains("application/grpc")
                        });
                    match (response.status(), tonic_consumable_content) {
                        (http::StatusCode::UNAUTHORIZED, Some(false))
                        | (http::StatusCode::FORBIDDEN, Some(false)) => {
                            // the GCP auth frontend embeds useful
                            // info in the `www-authenticate` header
                            let auth_res = response
                                .headers()
                                .get("www-authenticate")
                                .map(|hv| hv.to_str().unwrap_or_default());

                            tonic::Status::unauthenticated(auth_res.unwrap_or_default())
                                .to_http()
                                .map(|_| Body::empty())
                        }
                        (http::StatusCode::NOT_FOUND, Some(false)) => {
                            tonic::Status::not_found("".to_owned())
                                .to_http()
                                .map(|_| Body::empty())
                        }
                        (code, Some(false)) => {
                            tonic::Status::unknown(
                                format!(
                                    "Recieved non grpc content with status code \"{}\" which is unhandled. Headers: {:#?}",
                                    code,
                                    response.headers()
                                )
                            )
                            .to_http()
                            .map(|_| Body::empty())
                        }
                        _ => response,
                    }
                }))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl tower::Service<http::Request<tonic::body::BoxBody>> for HttpStatusInterceptor {
    type Response = http::Response<Body>;
    type Error = tonic::transport::Error;
    type Future = ResponseFuture;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.channel.poll_ready(cx)
    }

    fn call(&mut self, request: http::Request<tonic::body::BoxBody>) -> Self::Future {
        // This is necessary because tonic internally uses `tower::buffer::Buffer`.
        // See https://github.com/tower-rs/tower/issues/547#issuecomment-767629149
        // for details on why this is necessary
        let clone = self.channel.clone();
        let mut inner = std::mem::replace(&mut self.channel, clone);

        ResponseFuture {
            inner: inner.call(request),
        }
    }
}
