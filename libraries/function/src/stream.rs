pub mod rwstream;

use firm_protocols::functions::ChannelSpec;
use thiserror::Error;

use std::{collections::VecDeque, ops::Deref, task::Poll};

use crate::io::{PollRead, PollWrite};

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
}

impl Channel {
    pub fn new<S: Into<String>>(name: S, data_type: S) -> Self {
        Channel {
            name: name.into(),
            data_type: data_type.into(),
            data: VecDeque::new(),
            closed: false,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn data_type(&self) -> &str {
        &self.data_type
    }

    pub fn poll_read(&mut self, buf: &mut [u8]) -> Poll<usize> {
        let end = std::cmp::min(buf.len(), self.data.len());

        if end == 0 && !self.closed() {
            return Poll::Pending;
        }

        self.data.drain(0..end).enumerate().for_each(|(i, v)| {
            buf[i] = v;
        });
        Poll::Ready(end)
    }

    pub fn closed(&self) -> bool {
        self.closed
    }

    pub fn write(&mut self, buf: &[u8]) -> Result<(), StreamError> {
        if self.closed {
            Err(StreamError::ChannelWriteClosed {
                channel_name: self.name.clone(),
            })
        } else {
            self.data.extend(buf);
            Ok(())
        }
    }
}

pub trait ChannelReader: Clone + PollRead {
    fn channel_id(&self) -> &str;
}

pub trait ChannelWriter: Clone + PollWrite {
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
