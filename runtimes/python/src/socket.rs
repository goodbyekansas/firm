use std::io::{Read, Write};

use pyo3::{
    create_exception,
    exceptions::PyException,
    ffi,
    proc_macro::{pyclass, pyfunction, pymodule},
    types::PyModule,
    wrap_pyfunction, PyAny, PyResult, Python,
};

create_exception!(firm, ConnectionError, PyException);
create_exception!(firm, SocketError, PyException);

#[pyclass]
#[derive(Default)]
pub struct WasiSocket {
    stream: Option<::firm::net::TcpConnection>,
}

#[pyfunction]
fn connect(slf: &mut WasiSocket, address: (String, i64)) -> PyResult<()> {
    let address = format!("{}:{}", address.0, address.1);
    slf.stream = ::firm::net::connect(address)
        .map_err(|e| ConnectionError::new_err(e.to_string()))
        .map(Some)?;
    Ok(())
}

#[pyfunction]
fn send(slf: &mut WasiSocket, bytes: Vec<u8>, _flags: &PyAny) -> PyResult<usize> {
    slf.stream
        .as_mut()
        .ok_or_else(|| SocketError::new_err("Call connect() first!".to_owned()))
        .and_then(|s| {
            s.write(&bytes)
                .map_err(|e| SocketError::new_err(e.to_string()))
        })
}

#[pyfunction]
fn recv(slf: &mut WasiSocket, bufsize: usize, _flags: &PyAny) -> PyResult<Vec<u8>> {
    let mut buf = Vec::with_capacity(bufsize);
    slf.stream
        .as_mut()
        .ok_or_else(|| SocketError::new_err("Call connect() first!".to_owned()))
        .and_then(|s| {
            s.read(&mut buf)
                .map(|_| buf)
                .map_err(|e| SocketError::new_err(e.to_string()))
        })
}

#[pyfunction]
fn new_socket() -> WasiSocket {
    WasiSocket::default()
}

#[pymodule]
fn socket_module(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(new_socket, m)?)?;
    m.add_function(wrap_pyfunction!(connect, m)?)?;

    m.add_function(wrap_pyfunction!(send, m)?)?;
    m.add_function(wrap_pyfunction!(recv, m)?)?;

    Ok(())
}

pub extern "C" fn init() -> *mut ffi::PyObject {
    unsafe { PyInit_socket_module() }
}

pub fn load_py_module(py: Python<'_>) -> PyResult<&'_ PyModule> {
    PyModule::from_code(py, include_str!("socket.py"), "socket_shim", "socket")
}
