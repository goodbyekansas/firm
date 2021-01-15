use std::{
    convert::TryFrom,
    io::{self, Read, Seek, SeekFrom, Write},
    net::TcpStream,
};

use serde::{de, Deserialize, Serialize};
use wasmer_wasi::{
    state::{WasiFile, WasiFs, WasiFsError, VIRTUAL_ROOT_FD},
    types,
};

use super::{
    error::{WasiError, WasiResult},
    WasmItemPtr, WasmString,
};

#[derive(Debug, Serialize)]
struct SocketFile {
    address: String,
    #[serde(skip_serializing)]
    stream: TcpStream,
}

impl SocketFile {
    pub fn new<S: AsRef<str>>(address: S) -> Result<Self, io::Error> {
        let stream = TcpStream::connect(address.as_ref())?;
        Ok(SocketFile {
            address: address.as_ref().to_owned(),
            stream,
        })
    }
}

impl<'de> Deserialize<'de> for SocketFile {
    fn deserialize<D>(deserializer: D) -> Result<SocketFile, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "snake_case")]
        enum Field {
            Address,
        }

        struct SocketFileVisitor;
        impl<'de> de::Visitor<'de> for SocketFileVisitor {
            type Value = SocketFile;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("struct SocketFile")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: de::SeqAccess<'de>,
            {
                let address: String = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                SocketFile::new(address).map_err(|e| {
                    de::Error::custom(format!(
                        "Failed to connect to socket while creating SocketFile: {}",
                        e
                    ))
                })
            }

            fn visit_map<V>(self, mut map: V) -> Result<Self::Value, V::Error>
            where
                V: de::MapAccess<'de>,
            {
                let mut address = None;
                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Address => {
                            if address.is_some() {
                                return Err(de::Error::duplicate_field("address"));
                            }
                            address = Some(map.next_value()?);
                        }
                    }
                }

                let address: String = address.ok_or_else(|| de::Error::missing_field("address"))?;
                SocketFile::new(address).map_err(|e| {
                    de::Error::custom(format!(
                        "Failed to connect to socket while creating SocketFile: {}",
                        e
                    ))
                })
            }
        }

        const FIELDS: &[&str] = &["address"];
        deserializer.deserialize_struct("SocketFile", FIELDS, SocketFileVisitor)
    }
}

impl Read for SocketFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.stream.read(buf)
    }
}

impl Write for SocketFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stream.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

impl Seek for SocketFile {
    fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(io::ErrorKind::Other, "can not seek socket"))
    }
}

#[typetag::serde]
impl WasiFile for SocketFile {
    fn last_accessed(&self) -> u64 {
        0
    }

    fn last_modified(&self) -> u64 {
        0
    }

    fn created_time(&self) -> u64 {
        0
    }

    fn size(&self) -> u64 {
        0
    }

    fn set_len(&mut self, _new_size: types::__wasi_filesize_t) -> Result<(), WasiFsError> {
        Err(WasiFsError::PermissionDenied)
    }

    fn unlink(&mut self) -> Result<(), WasiFsError> {
        Ok(())
    }

    fn bytes_available(&self) -> Result<usize, WasiFsError> {
        let mut buff = [0; 1024];
        self.stream
            .peek(&mut buff)
            .map_err(|_| WasiFsError::IOError)
    }

    #[cfg(unix)]
    fn get_raw_fd(&self) -> Option<i32> {
        use std::os::unix::io::AsRawFd;
        Some(self.stream.as_raw_fd())
    }

    #[cfg(not(unix))]
    fn get_raw_fd(&self) -> Option<i32> {
        use std::os::windows::io::AsRawSocket;
        // TODO: Returns an u64. May get truncated.
        Some(self.stream.as_raw_socket())
    }
}

pub fn connect(fs: &mut WasiFs, address: WasmString, fd_out: WasmItemPtr<u32>) -> WasiResult<()> {
    let address: String = String::try_from(address)
        .map_err(|e| WasiError::FailedToReadStringPointer("address".to_owned(), e))?;

    let socket_file =
        SocketFile::new(&address).map_err(|e| WasiError::FailedToConnect(address.clone(), e))?;

    let fd = fs
        .open_file_at(
            VIRTUAL_ROOT_FD,
            Box::new(socket_file),
            types::__WASI_O_CREAT, // open_flags
            format!("{}.sock", &address),
            types::__WASI_RIGHT_FD_READ
                | types::__WASI_RIGHT_FD_WRITE
                | types::__WASI_RIGHT_FD_SEEK,
            0, // rights_inheriting
            0, // fd_flags
        )
        .map_err(|e| WasiError::FailedToOpenFile(format!("{:#?}", e)))?;

    fd_out.set(fd)
}
