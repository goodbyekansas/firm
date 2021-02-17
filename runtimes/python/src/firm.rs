use std::collections::HashMap;

use firm_types::functions;
use pyo3::{
    create_exception,
    exceptions::PyException,
    ffi,
    prelude::FromPyObject,
    proc_macro::{pyfunction, pymodule},
    types::PyBytes,
    types::{PyIterator, PyModule},
    wrap_pyfunction, IntoPy, PyAny, PyResult, Python, ToPyObject,
};

create_exception!(firm, GetInputError, PyException);
create_exception!(firm, SetOutputError, PyException);
create_exception!(firm, GetHostOsError, PyException);
create_exception!(firm, HostPathExistsError, PyException);
create_exception!(firm, HostProcessError, PyException);
create_exception!(firm, MapAttachmentError, PyException);
create_exception!(firm, SetErrorError, PyException);

/// Get an input designated by `key` as a "stream"
#[pyfunction]
fn get_input_stream(py: Python<'_>, key: String) -> PyResult<Option<&'_ PyIterator>> {
    firm::get_channel(key)
        .map_err(|e| GetInputError::new_err(e.to_string().into_py(py)))
        .and_then(|channel| {
            channel
                .value
                .map(|value| {
                    PyIterator::from_object(
                        py,
                        &match value {
                            functions::channel::Value::Strings(x) => x.values.to_object(py),
                            functions::channel::Value::Integers(x) => x.values.to_object(py),
                            functions::channel::Value::Floats(x) => x.values.to_object(py),
                            functions::channel::Value::Booleans(x) => x.values.to_object(py),
                            functions::channel::Value::Bytes(x) => x.values.to_object(py),
                        }
                        .to_object(py),
                    )
                })
                .transpose()
        })
}

/// Get a single input designated by `key`
///
/// Note that this always picks the _last_ value in the channel
/// so if you expect more than one value, use `get_input_stream`
/// instead.
#[pyfunction]
fn get_input(py: Python<'_>, key: String) -> PyResult<Option<&'_ PyAny>> {
    firm::get_channel(key)
        .map_err(|e| GetInputError::new_err(e.to_string().into_py(py)))
        .map(|channel| {
            channel.value.map(|value| match value {
                functions::channel::Value::Strings(mut x) => {
                    x.values.pop().to_object(py).into_ref(py)
                }
                functions::channel::Value::Integers(mut x) => {
                    x.values.pop().to_object(py).into_ref(py)
                }
                functions::channel::Value::Floats(mut x) => {
                    x.values.pop().to_object(py).into_ref(py)
                }
                functions::channel::Value::Booleans(mut x) => {
                    x.values.pop().to_object(py).into_ref(py)
                }
                functions::channel::Value::Bytes(mut x) => {
                    x.values.pop().to_object(py).into_ref(py)
                }
            })
        })
}

/// Representation of an output value
#[derive(FromPyObject)]
enum OutputValues<'a> {
    // this needs to not be a Rust type
    // since it overlaps with int
    #[pyo3(transparent, annotation = "bytes")]
    Bytes(&'a PyBytes),
    #[pyo3(transparent, annotation = "Sequence[str]")]
    Strings(Vec<String>),

    // bool needs to be before int since in python
    // a bool is also an int
    #[pyo3(transparent, annotation = "Sequence[bool]")]
    Booleans(Vec<bool>),
    #[pyo3(transparent, annotation = "Sequence[int]")]
    Integers(Vec<i64>),
    #[pyo3(transparent, annotation = "Sequence[float]")]
    Floats(Vec<f64>),
}

/// Set an output designated by `key` to `value`
///
/// `value` has to be a sequence of str, int, float, bool, or bytes
#[pyfunction]
fn set_output(key: String, values: OutputValues) -> PyResult<()> {
    // check the value of the first item
    match values {
        OutputValues::Bytes(bytes) => firm::set_output(key, bytes.as_bytes().to_vec()),
        OutputValues::Strings(strings) => firm::set_output(key, strings),
        OutputValues::Integers(integers) => firm::set_output(key, integers),
        OutputValues::Floats(floats) => firm::set_output(key, floats),
        OutputValues::Booleans(booleans) => firm::set_output(key, booleans),
    }
    .map_err(|e| SetOutputError::new_err(e.to_string()))
}

/// Get the host os
///
/// This is the OS where the wasi runtime executes on.
#[pyfunction]
fn get_host_os() -> PyResult<String> {
    firm::get_host_os().map_err(|e| GetHostOsError::new_err(e.to_string()))
}

/// Check if host path exists
///
/// Returns true or false depending on if the host path exists
#[pyfunction]
fn host_path_exists(path: String) -> PyResult<bool> {
    firm::host_path_exists(path).map_err(|e| HostPathExistsError::new_err(e.to_string()))
}

/// Starts a host process
///
/// This is not a blocking operation. Function returns the process PID
#[pyfunction]
fn start_host_process(
    name: String,
    args: Option<Vec<String>>,
    environment: Option<HashMap<String, String>>,
) -> PyResult<u64> {
    firm::start_host_process(
        name,
        &args.unwrap_or_default(),
        &environment.unwrap_or_default(),
    )
    .map_err(|e| HostProcessError::new_err(e.to_string()))
}

/// Starts a host process
///
/// Blocks until process has exited. Returns the process exit code
#[pyfunction]
fn run_host_process(
    name: String,
    args: Option<Vec<String>>,
    environment: Option<HashMap<String, String>>,
) -> PyResult<i32> {
    firm::run_host_process(
        name,
        &args.unwrap_or_default(),
        &environment.unwrap_or_default(),
    )
    .map_err(|e| HostProcessError::new_err(e.to_string()))
}

#[pyfunction]
fn map_attachment(attachment_name: String, unpack: Option<bool>) -> PyResult<String> {
    if unpack.unwrap_or(false) {
        firm::map_attachment_and_unpack(attachment_name)
    } else {
        firm::map_attachment(attachment_name)
    }
    .map_err(|e| MapAttachmentError::new_err(e.to_string()))
    .map(|p| p.to_string_lossy().into_owned())
}

#[pyfunction]
fn set_error(message: String) -> PyResult<()> {
    firm::set_error(message).map_err(|e| SetErrorError::new_err(e.to_string()))
}

/// Module for interacting with the firm
#[pymodule]
fn firm(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(get_input, m)?)?;
    m.add_function(wrap_pyfunction!(get_input_stream, m)?)?;

    m.add_function(wrap_pyfunction!(set_output, m)?)?;
    m.add_function(wrap_pyfunction!(set_error, m)?)?;

    m.add_function(wrap_pyfunction!(get_host_os, m)?)?;
    m.add_function(wrap_pyfunction!(host_path_exists, m)?)?;

    m.add_function(wrap_pyfunction!(start_host_process, m)?)?;
    m.add_function(wrap_pyfunction!(run_host_process, m)?)?;

    m.add_function(wrap_pyfunction!(map_attachment, m)?)?;

    Ok(())
}

// need to wrap this function because the python api
// expect a non-unsafe (safe?) function and the
// function generated by the `pymodule` proc-macro
// generates an extern "C" unsafe fn
pub extern "C" fn init() -> *mut ffi::PyObject {
    unsafe { PyInit_firm() }
}
