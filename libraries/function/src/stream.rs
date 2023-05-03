use thiserror::Error;

use std::{
    collections::{HashMap, VecDeque},
    sync::RwLock,
    task::Poll,
};

#[derive(Debug, Error)]
pub enum StreamValidationError {
    #[error(
        "Channel \"{channel_name}\" has unexpected type. Expected \"{expected}\", got \"{got}\""
    )]
    MismatchedChannelType {
        channel_name: String,
        expected: String,
        got: String,
    },

    #[error("Channel \"{0}\" was not expected by spec")]
    UnexpectedChannel(String),

    #[error("Failed to find required channel {0}")]
    RequiredChannelMissing(String),
}

pub struct Channel {
    name: String,
    data: VecDeque<u8>,
    data_type: String,
    closed: bool,
}

impl Channel {
    fn poll_read(&mut self, buf: &mut [u8]) -> Poll<usize> {
        let end = std::cmp::min(buf.len(), self.data.len());

        if end == 0 && !self.closed() {
            return Poll::Pending;
        }

        self.data.drain(0..end).enumerate().for_each(|(i, v)| {
            buf[i] = v;
        });
        Poll::Ready(end)
    }

    fn closed(&self) -> bool {
        self.closed
    }

    fn write(&mut self, buf: &[u8]) {
        self.data.extend(buf);
    }
}

struct ChannelReader<'a> {
    channel: &'a mut Channel,
}

impl<'a> ChannelReader<'a> {
    pub fn poll_read(&mut self, buf: &mut [u8]) -> Poll<usize> {
        self.channel.poll_read(buf)
    }
}

struct ChannelWriter<'a> {
    channel: &'a mut Channel,
}
impl<'a> ChannelWriter<'a> {
    pub fn write(&mut self, buf: &[u8]) {
        self.channel.write(buf);
    }
}

pub struct StreamReader {}
pub struct StreamWriter {}

/*
pub struct Stream<T = StreamReader> {
    channels: Vec<Channel>,
    phantom: PhantomData<T>,
}*/

/*impl Stream<T> {
    pub fn has_channel(&self, channel_name: &str) -> bool {
        self.channels.iter().any(|c| c.name == channel_name)
    }

    pub fn get_channel(&self, channel_name: &str) -> Option<&Channel> {
        self.channels.iter().find(|c| c.name == channel_name)
    }

    pub fn get_mut_channel(&self, channel_name: &str) -> Option<&mut Channel> {
        self.channels.iter_mut().find(|c| c.name == channel_name)
    }
}*/
/*
impl<T> Stream<T>
where
    T: StreamRead,
{
    pub fn reader<'channel_reader>(
        &'channel_reader mut self,
        channel_name: &str,
    ) -> Option<ChannelReader<'channel_reader>> {
        self.channels
            .iter_mut()
            .find(|c| c.name == channel_name)
            .map(|c| ChannelReader { channel: c })
    }
}

impl<T> Stream<T> where T: StreamWrite {
    pub fn writer<'channel_writer>(&mut self)
}

 */

/*
-------------------1
 */

pub trait ReadProxy<'channel> {
    fn poll_read(&'channel self, buf: &mut [u8]) -> Result<Poll<usize>, String>;
}

pub trait WriteProxy<'channel> {
    fn write(&'channel self, buf: &[u8]) -> Result<(), String>;
}

pub trait Stream<'a> {
    fn has_channel(&self, channel_name: &str) -> bool;
    fn read_channel(&self, channel_name: &str) -> Option<Box<dyn ReadProxy>>;
    fn write_channel(&self, channel_name: &str) -> Option<Box<dyn WriteProxy>>;
}

struct MyProxy<'a> {
    lock: &'a RwLock<Channel>,
}

impl<'channel> ReadProxy<'channel> for MyProxy<'channel> {
    fn poll_read(&'channel self, buf: &mut [u8]) -> Result<Poll<usize>, String> {
        self.lock
            .write()
            .map_err(|e| e.to_string())
            .map(|mut channel| channel.poll_read(buf))
    }
}

impl<'channel> WriteProxy<'channel> for MyProxy<'channel> {
    fn write(&'channel self, buf: &[u8]) -> Result<(), String> {
        self.lock
            .write()
            .map_err(|e| e.to_string())
            .map(|mut channel| channel.write(buf))
    }
}

struct MyStream {
    channels: HashMap<String, RwLock<Channel>>,
}

impl<'a> Stream<'a> for MyStream {
    fn has_channel(&self, channel_name: &str) -> bool {
        self.channels.contains_key(channel_name)
    }

    fn read_channel(&self, channel_name: &str) -> Option<Box<dyn ReadProxy>> {
        self.channels
            .get(channel_name)
            .map(|channel| Box::new(MyProxy { lock: channel }) as Box<dyn ReadProxy>)
    }

    fn write_channel(&self, channel_name: &str) -> Option<Box<dyn WriteProxy>> {
        self.channels
            .get(channel_name)
            .map(|channel| Box::new(MyProxy { lock: channel }) as Box<dyn WriteProxy>)
    }
}

// Test how it feels to use.
fn use_stream(stream: &dyn Stream) {
    let channel_name = "something";
    let reader = stream.read_channel(channel_name).unwrap();
    let writer = stream.write_channel(channel_name).unwrap();
    let mut buf = [0u8; 5];
    reader.poll_read(&mut buf);
    writer.write(&buf);
}
