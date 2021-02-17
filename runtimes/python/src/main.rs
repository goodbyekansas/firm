use std::{env, fmt::Display};

use ::firm::{
    runtime_context::{RuntimeContext, RuntimeContextExt},
    AttachmentDownload,
};

use pyo3::{ffi, PyResult, Python};

// pub use to not have symbols stripped
// TODO: might be a less intrusive way to do this
pub use wasi_python_shims::*;
mod firm;
mod socket;

struct Entrypoint {
    module: String,
    function: String,
}

impl Display for Entrypoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "module: \"{}\", function: \"{}\"",
            &self.module, &self.function
        )
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let runtime_context = RuntimeContext::from_default()?;

    let mut parts = runtime_context.entrypoint.splitn(2, ':');

    let entrypoint = Entrypoint {
        module: parts.next().unwrap_or("main").to_owned(),
        function: parts.next().unwrap_or("main").to_owned(),
    };

    let code = runtime_context
        .code
        .ok_or("code is required for python")?
        .download_unpacked()?;

    env::set_var("PYTHONHOME", "/runtime-fs:{}");

    // python sdists always contain a single top-level
    // folder so add this to sys.path so we can
    // find the entrypoint module below
    let first_dir = std::fs::read_dir(code)?
        .next()
        .ok_or("no folder in unpacked python sdist")?
        .map(|de| de.path())?;

    env::set_var(
        "PYTHONPATH",
        // need to prepend a slash to the given path here
        // to make it absolute for python to be happy
        // if later this is done for us (download() returns
        // an absolute path), remove this slash
        format!("/runtime-fs/lib:/{}", first_dir.display()),
    );

    unsafe {
        // Add our module(s), this needs to be called before initalize
        // for it to be considered an "internal" module
        ffi::PyImport_AppendInittab("firm\0".as_ptr() as *const i8, Some(firm::init));
        ffi::PyImport_AppendInittab("wasi_socket\0".as_ptr() as *const i8, Some(socket::init));

        ffi::Py_InitializeEx(0);
        if ffi::Py_IsInitialized() == 0 {
            return Err(Box::<dyn std::error::Error>::from(
                "🐍 Python failed to initialize!",
            ));
        };
    }

    println!("Starting python code with entrypoint: {}", entrypoint);

    // Release the GIL so we can use with_gil and friends
    let ts = unsafe { ffi::PyEval_SaveThread() };

    let res = Python::with_gil(|py| -> PyResult<()> {
        socket::load_py_module(py)?;
        let main_module = py.import(&entrypoint.module)?;
        main_module.call0(&entrypoint.function)?;

        Ok(())
    });

    match res {
        Ok(_) => {}
        Err(pyerr) => {
            eprintln!("oh no! 🐍 error: {}", pyerr);
            ::firm::set_error(pyerr.to_string())?;
        }
    }

    unsafe {
        ffi::PyEval_RestoreThread(ts);
        ffi::Py_Finalize();
    }

    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("Unhandled Error: {}", e);
        match ::firm::set_error(format!("Unhandled Error: {}", e)) {
            Ok(_) => {}
            Err(e) => eprintln!("Failed to set function error: {}", e),
        }
    }
}
