pub mod rwstream;

use firm_protocols::functions::ChannelSpec;
use futures::{AsyncRead, AsyncWrite};
use thiserror::Error;

use std::{
    collections::VecDeque,
    ops::Deref,
    task::{Poll, Waker},
};

pub use rwstream::RWChannelStream;

#[derive(Debug, Error)]
pub enum StreamError {
    #[error("Failed to find required channel {0}")]
    RequiredChannelMissing(String),

    #[error("Channel \"{channel_name}\" is closed and cannot be written to.")]
    ChannelWriteClosed { channel_name: String },

    #[error("Failed to read channel \"{channel_name}\": {error}")]
    ChannelReadFailed { channel_name: String, error: String },

    #[error("Failed to write to channel \"{channel_name}\": {error}")]
    ChannelWriteFailed { channel_name: String, error: String },

    #[error("Failed to open channel for read in iterator: {error}")]
    ChannelIteratorFailed { error: String },
}

pub struct Channel {
    name: String,
    data: VecDeque<u8>,
    data_type: String,
    closed: bool,
    waker: Option<Waker>,
}

impl Channel {
    pub fn new<S: Into<String>>(name: S, data_type: S) -> Self {
        Channel {
            name: name.into(),
            data_type: data_type.into(),
            data: VecDeque::new(),
            closed: false,
            waker: None,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn data_type(&self) -> &str {
        &self.data_type
    }

    pub fn closed(&self) -> bool {
        self.closed
    }
}

impl AsyncRead for Channel {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        let end = std::cmp::min(buf.len(), self.data.len());

        if end == 0 && !self.closed() {
            self.get_mut().waker = Some(cx.waker().clone());
            return Poll::Pending;
        }

        self.get_mut()
            .data
            .drain(0..end)
            .enumerate()
            .for_each(|(i, v)| {
                buf[i] = v;
            });
        Poll::Ready(Ok(end))
    }
}

impl AsyncWrite for Channel {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        if this.closed {
            Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                StreamError::ChannelWriteClosed {
                    channel_name: this.name.clone(),
                },
            )))
        } else {
            this.data.extend(buf);
            if let Some(w) = this.waker.take() {
                w.wake()
            }
            Poll::Ready(Ok(buf.len()))
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.get_mut().closed = true;
        Poll::Ready(Ok(()))
    }
}

pub trait ChannelReader: Clone + AsyncRead {
    fn channel_id(&self) -> &str;
}

pub trait ChannelWriter: Clone + AsyncWrite {
    fn channel_id(&self) -> &str;
    fn close(&self) -> Result<(), StreamError>;
}

pub trait Stream<'a> {
    type StreamType: Sized;
    type IteratorItem: Deref<Target = Channel>;
    type IteratorType: Iterator<Item = Result<Self::IteratorItem, StreamError>>;
    type Reader: ChannelReader + 'a;
    type Writer: ChannelWriter + 'a;

    fn new_from_specs<Specs>(channel_specs: Specs) -> Self::StreamType
    where
        Specs: IntoIterator<Item = (String, ChannelSpec)>,
        Self: Sized;

    fn has_channel(&self, channel_name: &str) -> bool;
    fn read_channel(&'a self, channel_name: &str) -> Option<Self::Reader>;
    fn write_channel(&'a self, channel_name: &str) -> Option<Self::Writer>;

    fn readers(&'a self) -> Vec<Self::Reader>;
    fn writers(&'a self) -> Vec<Self::Writer>;

    fn iter(&'a self) -> Self::IteratorType;
}

impl<'a, T> Stream<'a> for &T
where
    T: Stream<'a, StreamType = T>,
{
    type StreamType = T;
    fn new_from_specs<Specs>(channel_specs: Specs) -> Self::StreamType
    where
        Specs: IntoIterator<Item = (String, ChannelSpec)>,
        Self: Sized,
    {
        T::new_from_specs(channel_specs)
    }

    fn has_channel(&self, channel_name: &str) -> bool {
        T::has_channel(self, channel_name)
    }

    fn read_channel(&'a self, channel_name: &str) -> Option<Self::Reader> {
        T::read_channel(self, channel_name)
    }

    fn write_channel(&'a self, channel_name: &str) -> Option<Self::Writer> {
        T::write_channel(self, channel_name)
    }

    type IteratorType = T::IteratorType;

    fn iter(&'a self) -> Self::IteratorType {
        T::iter(self)
    }

    type IteratorItem = T::IteratorItem;

    type Reader = T::Reader;
    type Writer = T::Writer;

    fn readers(&'a self) -> Vec<Self::Reader> {
        T::readers(self)
    }

    fn writers(&'a self) -> Vec<Self::Writer> {
        T::writers(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use rwstream::RWChannelStream;

    #[test]
    fn test_stream() {
        fn function_that_accept_stream(stream: impl for<'a> Stream<'a>) {
            {
                let _a = Stream::read_channel(&stream, "sune");
            }
        }

        let s1 = RWChannelStream::new(vec![]);
        function_that_accept_stream(&s1);
    }
}
