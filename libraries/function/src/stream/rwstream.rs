use std::{
    collections::{HashMap, VecDeque},
    io::Write,
    sync::{Arc, RwLock, RwLockReadGuard},
    task::Poll,
};

use firm_protocols::functions::ChannelSpec;

use crate::io::{PollRead, PollWrite};

use super::{Channel, ChannelReader, ChannelWriter, Stream, StreamError};

#[derive(Clone)]
pub struct RWChannel {
    lock: Arc<RwLock<Channel>>,
    channel_name: String,
}

impl Write for RWChannel {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.lock
            .write()
            .map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    StreamError::ChannelWriteFailed {
                        channel_name: self.channel_name.clone(),
                        error: e.to_string(),
                    },
                )
            })
            .and_then(|mut channel| {
                channel.write(buf).map_err(|e| {
                    std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        StreamError::ChannelWriteFailed {
                            channel_name: self.channel_name.clone(),
                            error: e.to_string(),
                        },
                    )
                })
            })
            .map(|_| buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl PollRead for RWChannel {
    fn poll_read(&mut self, buf: &mut [u8]) -> Result<Poll<usize>, std::io::Error> {
        self.lock
            .write()
            .map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    StreamError::ChannelReadFailed {
                        channel_name: self.channel_name.clone(),
                        error: e.to_string(),
                    },
                )
            })
            .map(|mut channel| channel.poll_read(buf))
    }
}

impl ChannelReader for RWChannel {
    fn channel_id(&self) -> &str {
        &self.channel_name
    }
}

impl PollWrite for RWChannel {
    fn poll_write(&mut self, buf: &[u8]) -> Result<Poll<()>, std::io::Error> {
        self.lock
            .write()
            .map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    StreamError::ChannelWriteFailed {
                        channel_name: self.channel_name.clone(),
                        error: e.to_string(),
                    },
                )
            })
            .and_then(|mut channel| {
                channel.write(buf).map_err(|e| {
                    std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        StreamError::ChannelWriteFailed {
                            channel_name: self.channel_name.clone(),
                            error: e.to_string(),
                        },
                    )
                })
            })
            .map(Poll::Ready)
    }
}

impl ChannelWriter for RWChannel {
    fn close(&self) -> Result<(), StreamError> {
        self.lock
            .write()
            .map_err(|e| StreamError::ChannelWriteFailed {
                channel_name: self.channel_name.clone(),
                error: e.to_string(),
            })
            .map(|mut channel| {
                channel.closed = true;
            })
    }

    fn channel_id(&self) -> &str {
        &self.channel_name
    }
}

pub struct ChannelIter<'a> {
    values: std::collections::hash_map::Values<'a, String, Arc<RwLock<Channel>>>,
}

impl<'a> ChannelIter<'a> {
    pub fn new(
        values: std::collections::hash_map::Values<'a, String, Arc<RwLock<Channel>>>,
    ) -> Self {
        Self { values }
    }
}

impl<'a> Iterator for ChannelIter<'a> {
    type Item = Result<RwLockReadGuard<'a, Channel>, StreamError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.values.next().map(|channel_lock| {
            channel_lock
                .read()
                .map_err(|e| StreamError::ChannelIteratorFailed {
                    error: e.to_string(),
                })
        })
    }
}

pub struct RWChannelStream {
    channels: HashMap<String, Arc<RwLock<Channel>>>,
}

impl RWChannelStream {
    pub fn new(channels: Vec<Channel>) -> Self {
        let mut chan = HashMap::new();
        channels.into_iter().for_each(|channel| {
            chan.insert(channel.name.clone(), Arc::new(RwLock::new(channel)));
        });
        Self { channels: chan }
    }
}

impl<'a> Stream<'a> for RWChannelStream {
    type Reader = RWChannel;
    type Writer = RWChannel;

    type StreamType = Self;
    fn has_channel(&self, channel_name: &str) -> bool {
        self.channels.contains_key(channel_name)
    }

    fn read_channel(&'a self, channel_name: &str) -> Option<RWChannel> {
        self.channels.get(channel_name).map(|channel| RWChannel {
            lock: Arc::clone(channel),
            channel_name: String::from(channel_name),
        })
    }

    fn write_channel(&'a self, channel_name: &str) -> Option<RWChannel> {
        self.channels.get(channel_name).map(|channel| RWChannel {
            lock: Arc::clone(channel),
            channel_name: String::from(channel_name),
        })
    }

    fn new_from_specs<I>(channel_specs: I) -> Self
    where
        I: IntoIterator<Item = (String, ChannelSpec)>,
    {
        Self {
            channels: channel_specs
                .into_iter()
                .map(|(name, channel_spec)| {
                    (
                        name.clone(),
                        Arc::new(RwLock::new(Channel {
                            name,
                            data: VecDeque::new(),
                            // TODO: Change the "description" field to data_type once we changed it.
                            data_type: channel_spec.description,
                            closed: false,
                        })),
                    )
                })
                .collect(),
        }
    }

    type IteratorType = ChannelIter<'a>;
    type IteratorItem = RwLockReadGuard<'a, Channel>;

    fn iter(&'a self) -> Self::IteratorType {
        ChannelIter::new(self.channels.values())
    }

    fn readers(&'a self) -> Vec<Self::Reader> {
        self.channels
            .iter()
            .map(|(n, c)| RWChannel {
                lock: Arc::clone(c),
                channel_name: String::from(n),
            })
            .collect()
    }

    fn writers(&'a self) -> Vec<Self::Writer> {
        self.readers()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_read_and_writes() {
        let channels = vec![
            Channel::new("bergfink", "bird"),
            Channel::new("braxen", "fish"),
            Channel::new("lodjur", "cat"),
        ];
        let stream = RWChannelStream::new(channels);

        assert!(stream.has_channel("bergfink"));
        assert!(stream.has_channel("braxen"));
        assert!(stream.has_channel("lodjur"));

        assert!(!stream.has_channel("gris"));
        assert!(stream.read_channel("ozelot").is_none());
        assert!(stream.write_channel("stork").is_none());

        let mut bird_read = stream.read_channel("bergfink").unwrap();
        let mut bird_write = stream.write_channel("bergfink").unwrap();
        let mut fish_read = stream.read_channel("braxen").unwrap();
        let mut fish_write = stream.write_channel("braxen").unwrap();

        // Nothing written yet. Should give us pending.
        let mut buf = [0u8; 10];
        let read_res = bird_read.poll_read(&mut buf).unwrap();
        assert!(matches!(read_res, Poll::Pending));

        // We should have no data written on pending.
        assert_eq!(buf, [0u8; 10]);
        let write_res = bird_write.poll_write("fly high in the sky".as_bytes());
        assert!(write_res.is_ok());

        // Wrote to bird. Fish should still not have data.
        let read_res = fish_read.poll_read(&mut buf).unwrap();
        assert!(matches!(read_res, Poll::Pending));

        let mut read_data: Vec<u8> = vec![];
        let mut read_buf = [0u8; 10];
        while let Ok(Poll::Ready(read)) = bird_read.poll_read(&mut read_buf) {
            read_data.extend_from_slice(&read_buf[0..read]);
        }
        let message = String::from_utf8(read_data).unwrap();
        assert_eq!(message, "fly high in the sky");
        assert!(bird_write.close().is_ok());

        // Reading from a closed channel yields a read with 0 bytes
        let read_result = bird_read.poll_read(&mut read_buf).unwrap();
        matches!(read_result, Poll::Ready(0));

        let write_res = fish_write.poll_write("swim down below".as_bytes());
        assert!(write_res.is_ok());
        assert!(fish_write.close().is_ok());

        // We can't write to a closed channel.
        assert!(fish_write.poll_write("oh no!".as_bytes()).is_err());

        // Reading several smaller pieces.
        let mut read_data: Vec<u8> = vec![];
        let mut read_buf = [0u8; 2];
        loop {
            match fish_read.poll_read(&mut read_buf).unwrap() {
                Poll::Ready(read) => {
                    if read == 0 {
                        // eof
                        break;
                    }
                    read_data.extend_from_slice(&read_buf[0..read]);
                }
                Poll::Pending => panic!("Did not expect to get poll pending."),
            }
        }

        let message = String::from_utf8(read_data).unwrap();
        assert_eq!(message, "swim down below");
    }

    #[test]
    fn many_writers_many_readers() {
        let channels = vec![Channel::new("sarv", "fish"), Channel::new("nors", "fish")];
        let stream = RWChannelStream::new(channels);
        let mut sarv_read1 = stream.read_channel("sarv").unwrap();
        let mut sarv_read2 = stream.read_channel("sarv").unwrap();
        let mut sarv_write1 = stream.write_channel("sarv").unwrap();
        let mut sarv_write2 = stream.write_channel("sarv").unwrap();

        let data1 = "Hej".as_bytes();
        let data2 = "San".as_bytes();
        assert!(sarv_write1.poll_write(data1).is_ok());
        assert!(sarv_write2.poll_write(data2).is_ok());

        let mut read_buf1: Vec<u8> = vec![0u8; data1.len() + 1];
        let mut read_buf2: Vec<u8> = vec![0u8; data1.len() - 1];
        assert!(sarv_read1.poll_read(&mut read_buf1).is_ok());
        assert!(sarv_read2.poll_read(&mut read_buf2).is_ok());

        let message1 = String::from_utf8(read_buf1).unwrap();
        let message2 = String::from_utf8(read_buf2).unwrap();

        assert_eq!(message1, "HejS");
        assert_eq!(message2, "an");

        // Closing one should close the other
        assert!(sarv_write1.close().is_ok());

        let mut buf = [0u8; 6];
        matches!(sarv_read2.poll_read(&mut buf).unwrap(), Poll::Ready(0));
        matches!(sarv_read2.poll_read(&mut buf).unwrap(), Poll::Ready(0));
        assert!(sarv_write2.poll_write(&buf).is_err());
    }
}
