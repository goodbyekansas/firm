use lazy_static::lazy_static;
use std::{collections::HashMap, ffi::c_void, sync::RwLock};

/// Lock data structure that is used as a cookie.
#[repr(C)]
pub struct WasiThreadLock {
    locked: bool,
}

lazy_static! {
    static ref TSS_STORAGE: RwLock<HashMap<u64, u64>> = RwLock::new(HashMap::new());
}

/// Initialize the threading library at lightning speed!
#[no_mangle]
pub extern "C" fn wt_init_thread() {}

/// Start a new thread (not actually, just returns 0 whatever you pass in)
#[no_mangle]
pub extern "C" fn wt_start_new_thread(_func: extern "C" fn(*mut c_void), _arg: *mut c_void) -> u64 {
    0
}

/// Gets the current thread identifier which happens to be a constant (1).
#[no_mangle]
pub extern "C" fn wt_get_thread_ident() -> u64 {
    // there can be only one: https://open.spotify.com/track/78UcxP3Xm0EOXneQUDsStA?si=geoeQYBJTrCRINNvrCd-6w
    1
}

/// Exits the current thread.
#[no_mangle]
pub extern "C" fn wt_exit_thread() {}

/// Allocate a new lock.
///
/// The caller is responsible for calling wt_free_lock with the returned pointer.
#[no_mangle]
pub extern "C" fn wt_allocate_lock() -> *mut WasiThreadLock {
    // into_raw is used to give ownership of the memory
    // to the caller. Hopefully we get it back in wt_free_lock later...;
    Box::into_raw(Box::new(WasiThreadLock { locked: false }))
}

/// Frees memory associated with the passed in lock.
///# Safety
/// Deallocates the lock from the raw pointer passed in.
#[no_mangle]
pub unsafe extern "C" fn wt_free_lock(lock: *mut WasiThreadLock) {
    if !lock.is_null() {
        let _ = Box::from_raw(lock);
    }
}

/// Tries to acquire the lock and returns whether it was successful
///# Safety
/// Takes and releases ownership of the passed in pointer.
#[no_mangle]
pub unsafe extern "C" fn wt_acquire_lock(lock: *mut WasiThreadLock) -> bool {
    let mut lock = Box::from_raw(lock);

    // into_raw is important to let the caller continue to own the memory
    if lock.locked {
        Box::into_raw(lock);
        false
    } else {
        // Would be cool if we could wait but how could we with one thread?
        lock.locked = true;
        Box::into_raw(lock);
        true
    }
}

/// Releases the passed in lock.
///# Safety
/// Takes and releases ownership of the passed in pointer.
#[no_mangle]
pub unsafe extern "C" fn wt_release_lock(lock: *mut WasiThreadLock) {
    let mut lock = Box::from_raw(lock);

    lock.locked = false;
    // into_raw is needed to let the caller continue to own the memory
    Box::into_raw(lock);
}

/// Creates thread specific storage
///
///(or just returns true without doing any work)
#[no_mangle]
pub extern "C" fn wt_tss_create(_key: u64) -> bool {
    true
}

/// Deletes thread specific storage at the provided key.
#[no_mangle]
pub extern "C" fn wt_tss_delete(key: u64) -> bool {
    TSS_STORAGE.write().map_or(false, |mut tss| {
        tss.remove(&key);
        true
    })
}

/// Sets thread specific storage at the provided key.
#[no_mangle]
pub extern "C" fn wt_tss_set(key: u64, value: *mut c_void) -> bool {
    TSS_STORAGE.write().map_or(false, |mut tss| {
        tss.insert(key, value as u64);
        true
    })
}

/// Gets the thread specific storage stored at provided key.
#[no_mangle]
pub extern "C" fn wt_tss_get(key: u64) -> *mut c_void {
    TSS_STORAGE.read().map_or(std::ptr::null_mut(), |tss| {
        tss.get(&key)
            .map(|a| *a as *mut c_void)
            .unwrap_or(std::ptr::null_mut())
    })
}
