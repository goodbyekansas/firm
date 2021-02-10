pub mod thread;

use std::{
    ffi::CString,
    os::raw::{c_char, c_int},
};

#[allow(non_camel_case_types)]
type mode_t = u32;

#[allow(non_camel_case_types)]
type uid_t = u32;

// A type used to hold group IDs.  According to POSIX, this shall
// be an integer type.
#[allow(non_camel_case_types)]
type gid_t = u32;

#[allow(non_camel_case_types)]
type size_t = u32;

/// Change permissions on `pathname` to `mode`
#[no_mangle]
pub extern "C" fn chmod(_pathname: *const c_char, _mode: mode_t) -> c_int {
    -1
}

#[no_mangle]
pub extern "C" fn dup(_oldfd: c_int) -> c_int {
    -1
}

#[no_mangle]
pub extern "C" fn umask(_mask: mode_t) -> mode_t {
    0
}

const DEFAULT_USERNAME: &str = "wasi-user";
const DEFAULT_HOMEDIR: &str = "/homeless/";
const DEFAULT_SHELL: &str = "/unshelled";

struct PasswdBuffer {
    buffer: *mut c_char,
    capacity: usize,
    offset: usize,
}

impl PasswdBuffer {
    fn new(buffer: *mut c_char, capacity: usize) -> Self {
        Self {
            buffer,
            capacity,
            offset: 0,
        }
    }
}

struct PasswdBuilder {
    passwd: Box<passwd>,
}

impl PasswdBuilder {
    fn new() -> Self {
        Self {
            passwd: Box::new(passwd {
                pw_name: std::ptr::null_mut::<c_char>(),
                pw_uid: 0,
                pw_gid: 0,
                pw_dir: std::ptr::null_mut::<c_char>(),
                pw_shell: std::ptr::null_mut::<c_char>(),
            }),
        }
    }

    unsafe fn from_raw(passwd: *mut passwd) -> Self {
        Self {
            passwd: Box::from_raw(passwd),
        }
    }

    fn name(mut self, name: &str) -> Self {
        self.passwd.pw_name = CString::new(name).unwrap().into_raw();

        self
    }

    unsafe fn name_with_buffer(mut self, name: &str, buffer: &mut PasswdBuffer) -> Self {
        if buffer.capacity >= name.len() + buffer.offset {
            buffer
                .buffer
                .add(buffer.offset)
                .copy_from(name.as_ptr() as *const c_char, name.len());
            self.passwd.pw_name = buffer.buffer.add(buffer.offset);
            buffer.offset += name.len();
        }

        self
    }

    fn name_from_ptr(mut self, name: *mut c_char) -> Self {
        self.passwd.pw_name = name;

        self
    }

    fn gid(mut self, gid: gid_t) -> Self {
        self.passwd.pw_gid = gid;

        self
    }

    fn uid(mut self, uid: uid_t) -> Self {
        self.passwd.pw_uid = uid;

        self
    }

    fn home_dir(mut self, home_dir: &str) -> Self {
        self.passwd.pw_dir = CString::new(home_dir).unwrap().into_raw();

        self
    }

    unsafe fn home_dir_with_buffer(mut self, home_dir: &str, buffer: &mut PasswdBuffer) -> Self {
        if buffer.capacity >= home_dir.len() + buffer.offset {
            buffer
                .buffer
                .add(buffer.offset)
                .copy_from(home_dir.as_ptr() as *const c_char, home_dir.len());
            self.passwd.pw_dir = buffer.buffer.add(buffer.offset);
            buffer.offset += home_dir.len();
        }

        self
    }

    fn shell(mut self, shell: &str) -> Self {
        self.passwd.pw_shell = CString::new(shell).unwrap().into_raw();

        self
    }

    unsafe fn shell_with_buffer(mut self, shell: &str, buffer: &mut PasswdBuffer) -> Self {
        if buffer.capacity >= shell.len() + buffer.offset {
            buffer
                .buffer
                .add(buffer.offset)
                .copy_from(shell.as_ptr() as *const c_char, shell.len());
            self.passwd.pw_shell = buffer.buffer.add(buffer.offset);
            buffer.offset += shell.len();
        }

        self
    }

    fn into_raw(self) -> *mut passwd {
        Box::into_raw(self.passwd)
    }

    fn build(self) -> Box<passwd> {
        self.passwd
    }
}

impl Default for Box<passwd> {
    fn default() -> Self {
        PasswdBuilder::new()
            .name(DEFAULT_USERNAME)
            .home_dir(DEFAULT_HOMEDIR)
            .shell(DEFAULT_SHELL)
            .gid(1)
            .uid(1)
            .build()
    }
}

/// Passwd entry
#[repr(C)]
pub struct passwd {
    pw_name: *mut c_char,
    pw_uid: uid_t,
    pw_gid: gid_t,
    pw_dir: *mut c_char,
    pw_shell: *mut c_char,
}

/// Get a pwd entry from a uid
///
/// The caller is responsible for freeing allocated memory
/// # Safety
/// Freeing the original name and replacing it with the input name.
#[no_mangle]
pub unsafe extern "C" fn getpwnam(name: *const c_char) -> *mut passwd {
    let mut passwd = Box::<passwd>::default();
    let c_string = CString::from_raw(passwd.pw_name);
    std::mem::drop(c_string);
    passwd.pw_name = name as *mut c_char;

    Box::into_raw(passwd)
}

/// Get a pwd entry from a uid
///
/// The caller is responsible for freeing allocated memory
#[no_mangle]
pub extern "C" fn getpwuid(_uid: uid_t) -> *mut passwd {
    Box::into_raw(Box::<passwd>::default())
}

/// Get a pwd entry from a username, using a provided buffer
///
/// # Safety
/// It is unsafe to fill a buffer
#[no_mangle]
pub unsafe extern "C" fn getpwnam_r(
    name: *const c_char,
    pwd: *mut passwd,
    buffer: *mut c_char,
    bufsize: size_t,
    result: *mut *mut passwd,
) -> c_int {
    let pwd = PasswdBuilder::from_raw(pwd);
    let mut buffer = PasswdBuffer::new(buffer, bufsize as usize);

    *result = pwd
        .gid(1)
        .uid(1)
        .name_from_ptr(name as *mut c_char)
        .home_dir_with_buffer(DEFAULT_HOMEDIR, &mut buffer)
        .shell_with_buffer(DEFAULT_SHELL, &mut buffer)
        .into_raw();

    0
}

/// Get a pwd entry from a uid, using a provided buffer
///
/// # Safety
/// It is unsafe to fill a buffer
#[no_mangle]
pub unsafe extern "C" fn getpwuid_r(
    _uid: uid_t,
    pwd: *mut passwd,
    buffer: *mut c_char,
    bufsize: size_t,
    result: *mut *mut passwd,
) -> c_int {
    let pwd = PasswdBuilder::from_raw(pwd);
    let mut buffer = PasswdBuffer::new(buffer, bufsize as usize);

    *result = pwd
        .gid(1)
        .uid(1)
        .name_with_buffer(DEFAULT_USERNAME, &mut buffer)
        .home_dir_with_buffer(DEFAULT_HOMEDIR, &mut buffer)
        .shell_with_buffer(DEFAULT_SHELL, &mut buffer)
        .into_raw();

    0
}

static mut PWENT_INDEX: usize = 0;

/// Get the next pw entry
///
/// # Safety
/// The global pw entry index is mutated
#[no_mangle]
pub unsafe extern "C" fn getpwent() -> *mut passwd {
    if PWENT_INDEX == 0 {
        PWENT_INDEX += 1;
        Box::into_raw(Box::<passwd>::default())
    } else {
        std::ptr::null_mut()
    }
}

/// End pwent iteration
///
/// # Safety
/// The global pw entry index is mutated
#[no_mangle]
pub unsafe extern "C" fn endpwent() {
    PWENT_INDEX = 0;
}

/// Start a pwent iteration
///
/// # Safety
/// The global pw entry index is mutated
#[no_mangle]
pub unsafe extern "C" fn setpwent() {
    PWENT_INDEX = 0;
}

/// The getegid() function shall return the effective group ID of the calling process.
#[no_mangle]
pub extern "C" fn getegid() -> gid_t {
    1
}

/// The geteuid() function shall return the effective user ID of the calling process.
/// The geteuid() function shall not modify errno.
#[no_mangle]
pub extern "C" fn geteuid() -> gid_t {
    1
}

/// The getgid() function shall return the real group ID of the calling process.
/// The getgid() function shall not modify errno.
#[no_mangle]
pub extern "C" fn getgid() -> gid_t {
    1
}

/// The getuid() function shall return the real user ID of the calling process.
#[no_mangle]
pub extern "C" fn getuid() -> gid_t {
    1
}
