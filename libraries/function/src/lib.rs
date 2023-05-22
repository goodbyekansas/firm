pub mod stream;

pub mod io {
    use std::{
        io::{Cursor, Read, Write},
        task::Poll,
    };

    pub trait PollRead {
        fn poll_read(&mut self, buf: &mut [u8]) -> Result<Poll<usize>, std::io::Error>;
    }

    pub trait PollWrite {
        // TODO: "It is not considered an error if the entire buffer
        // could not be written to this writer."
        // Seems like returning ball is the wrong thing to do.
        fn poll_write(&mut self, buf: &[u8]) -> Result<Poll<()>, std::io::Error>;
    }

    impl<T: PollRead> PollRead for &mut T {
        fn poll_read(&mut self, buf: &mut [u8]) -> Result<Poll<usize>, std::io::Error> {
            PollRead::poll_read(*self, buf)
        }
    }

    impl<T: PollRead> PollRead for Box<T> {
        fn poll_read(&mut self, buf: &mut [u8]) -> Result<Poll<usize>, std::io::Error> {
            PollRead::poll_read(self.as_mut(), buf)
        }
    }

    impl<T: PollWrite> PollWrite for &mut T {
        fn poll_write(&mut self, buf: &[u8]) -> Result<Poll<()>, std::io::Error> {
            PollWrite::poll_write(*self, buf)
        }
    }

    impl<T> PollRead for Cursor<T>
    where
        T: AsRef<[u8]>,
    {
        fn poll_read(&mut self, buf: &mut [u8]) -> Result<Poll<usize>, std::io::Error> {
            match self.read(buf) {
                Ok(n) => Ok(Poll::Ready(n)),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(Poll::Pending),
                Err(e) => Err(e),
            }
        }
    }

    impl PollWrite for Cursor<&mut Vec<u8>> {
        fn poll_write(&mut self, buf: &[u8]) -> Result<Poll<()>, std::io::Error> {
            match self.write(buf) {
                Ok(_) => Ok(Poll::Ready(())),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(Poll::Pending),
                Err(e) => Err(e),
            }
        }
    }
}

pub mod attachments;
