use std::{
    ffi::OsString,
    os::windows::prelude::{OsStrExt, OsStringExt},
    ptr, u32,
};

use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};

use winapi::{
    shared::{
        minwindef::{DWORD, LPCVOID, LPDWORD},
        ntdef::NULL,
        winerror::{ERROR_MORE_DATA, ERROR_SERVICE_NOT_ACTIVE},
    },
    um::{
        errhandlingapi::GetLastError,
        winbase::{FormatMessageW, FORMAT_MESSAGE_FROM_SYSTEM},
        winnt::{
            DELETE, GENERIC_ALL, LPCWSTR, LPWSTR, SERVICE_AUTO_START, SERVICE_ERROR_NORMAL,
            SERVICE_USER_OWN_PROCESS,
        },
        winsvc::{
            CloseServiceHandle, ControlService, CreateServiceW, DeleteService, EnumServicesStatusW,
            OpenSCManagerW, OpenServiceW, QueryServiceStatus, StartServiceW, ENUM_SERVICE_STATUSW,
            LPENUM_SERVICE_STATUSW, LPSERVICE_STATUS, SC_HANDLE, SERVICE_ACTIVE,
            SERVICE_CONTROL_STOP, SERVICE_QUERY_STATUS, SERVICE_START, SERVICE_STATUS,
            SERVICE_STOP, SERVICE_STOPPED,
        },
    },
};

pub struct WinHandle {
    handle: SC_HANDLE,
}

impl Drop for WinHandle {
    fn drop(&mut self) {
        unsafe {
            CloseServiceHandle(self.handle);
        }
    }
}

fn parse_str_ptr(str_ptr: LPWSTR) -> OsString {
    OsString::from_wide(unsafe {
        let len = (0..).take_while(|&i| *str_ptr.offset(i) != 0).count();
        std::slice::from_raw_parts(str_ptr, len)
    })
}

fn win_string(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

pub fn last_error_message() -> String {
    unsafe {
        let error_code = GetLastError();
        const CAPACITY: usize = 512;
        let size = CAPACITY as u32;
        let mut message: [u16; CAPACITY] = [0; CAPACITY];
        FormatMessageW(
            FORMAT_MESSAGE_FROM_SYSTEM,
            NULL as LPCVOID,
            error_code,
            0,
            message.as_mut_ptr(),
            size,
            ptr::null_mut(),
        );
        format!(
            "{} ({})",
            String::from_utf16_lossy(&message[..(size as usize) - 1]),
            error_code
        )
    }
}

pub fn create_user_service(
    name: &str,
    path: &str,
    handle: &WinHandle,
    args: &[&str],
) -> Result<WinHandle, String> {
    let win_name = win_string(name);

    // Windows services do not have arguments at creation. You just add them to the path.
    let path = win_string(
        &args
            .iter()
            .fold(format!(r#""{}""#, path.to_owned()), |acc, &arg| {
                format!("{} {}", acc, arg)
            }),
    );

    match unsafe {
        CreateServiceW(
            handle.handle,
            win_name.as_ptr(),
            win_name.as_ptr(),
            GENERIC_ALL | SERVICE_START,
            SERVICE_USER_OWN_PROCESS,
            SERVICE_AUTO_START,
            SERVICE_ERROR_NORMAL,
            path.as_ptr(),
            NULL as LPCWSTR,
            NULL as LPDWORD,
            NULL as LPCWSTR,
            NULL as LPCWSTR,
            NULL as LPCWSTR,
        )
    } {
        service_handle if service_handle == (NULL as SC_HANDLE) => Err(format!(
            "Failed to create service: {}",
            last_error_message()
        )),
        service_handle => Ok(WinHandle {
            handle: service_handle,
        }),
    }
}

//TODO this should just be Result<WinHandle, String> and the error case is handled outside
pub fn get_service_handle(name: &str, manager_handle: &WinHandle) -> Result<WinHandle, String> {
    let win_name = win_string(name);
    match unsafe {
        OpenServiceW(
            manager_handle.handle,
            win_name.as_ptr(),
            DELETE | SERVICE_START | SERVICE_STOP | SERVICE_QUERY_STATUS,
        )
    } {
        v if v == (NULL as SC_HANDLE) => Err(format!(
            "Failed to get {} service: {}",
            name,
            last_error_message()
        )),
        handle => Ok(WinHandle { handle }),
    }
}

pub fn delete_service(handle: &WinHandle) -> Result<(), String> {
    (unsafe { DeleteService(handle.handle) } != 0)
        .then(|| ())
        .ok_or_else(last_error_message)
}

pub fn start_service(service_handle: &WinHandle) -> Result<(), String> {
    (unsafe { StartServiceW(service_handle.handle, 0, ptr::null_mut()) } != 0)
        .then(|| ())
        .ok_or_else(|| format!("Failed to start service: {}", last_error_message()))
}

pub fn get_service_status(service_handle: &WinHandle) -> Result<SERVICE_STATUS, String> {
    let mut service_status = SERVICE_STATUS::default();
    (unsafe {
        QueryServiceStatus(
            service_handle.handle,
            &mut service_status as LPSERVICE_STATUS,
        )
    } != 0)
        .then(|| service_status)
        .ok_or_else(|| format!("Failed to query service: {}", last_error_message()))
}

pub fn stop_service(service_handle: &WinHandle) -> Result<(), String> {
    let mut status: SERVICE_STATUS = Default::default();
    match unsafe {
        ControlService(
            service_handle.handle,
            SERVICE_CONTROL_STOP,
            &mut status as LPSERVICE_STATUS,
        )
    } {
        v if v == 0 && unsafe { GetLastError() } != ERROR_SERVICE_NOT_ACTIVE => {
            Err(format!("Failed to stop service: {}", last_error_message()))
        }
        _ => Ok(()),
    }
    .and_then(|_| {
        const TIMEOUT_SECONDS: u64 = 10;
        let timeout = std::time::Instant::now() + std::time::Duration::from_secs(TIMEOUT_SECONDS);

        while get_service_status(service_handle)?.dwCurrentState != SERVICE_STOPPED
            && std::time::Instant::now() < timeout
        {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        if std::time::Instant::now() > timeout {
            Err(format!(
                "Timed out after {} seconds waiting for service to stop.",
                TIMEOUT_SECONDS
            ))
        } else {
            Ok(())
        }
    })
}

pub fn get_service_manager(permissions: DWORD) -> Result<WinHandle, String> {
    match unsafe { OpenSCManagerW(NULL as LPCWSTR, NULL as LPCWSTR, permissions) } {
        v if v == (NULL as SC_HANDLE) => Err(format!(
            "Failed to get service manager: {}",
            last_error_message()
        )),
        handle => Ok(WinHandle { handle }),
    }
}

pub fn get_user_services(
    service_manager: &WinHandle,
    filter: &str,
) -> Result<Vec<WinHandle>, String> {
    let mut required_buffer_size: u32 = 0;
    let mut num_services = 0u32;
    let mut page = 0u32;
    match unsafe {
        EnumServicesStatusW(
            service_manager.handle,
            SERVICE_USER_OWN_PROCESS,
            SERVICE_ACTIVE,
            NULL as LPENUM_SERVICE_STATUSW,
            0,
            &mut required_buffer_size as LPDWORD,
            &mut num_services as LPDWORD,
            &mut page as LPDWORD,
        )
    } {
        0 if unsafe { GetLastError() } == ERROR_MORE_DATA => {
            Ok((required_buffer_size, num_services, page))
        }
        _ => Err(format!(
            "Could not search services: {}",
            last_error_message()
        )),
    }
    .and_then(|(mut required_buffer_size, mut num_services, mut page)| {
        let mut buf = Vec::<u8>::with_capacity(required_buffer_size as usize);

        unsafe {
            buf.set_len(required_buffer_size as usize);
        }

        match unsafe {
            EnumServicesStatusW(
                service_manager.handle,
                SERVICE_USER_OWN_PROCESS,
                SERVICE_ACTIVE,
                buf.as_mut_ptr() as _,
                buf.len() as u32,
                &mut required_buffer_size as LPDWORD,
                &mut num_services as LPDWORD,
                &mut page as LPDWORD,
            )
        } {
            0 if unsafe { GetLastError() } != ERROR_MORE_DATA => {
                Err(format!("Could not get services: {}", last_error_message()))
            }
            _res => {
                // TODO if _res == 0 then do another EnumServicesStatusW, because of paging
                unsafe {
                    buf[0..std::mem::size_of::<ENUM_SERVICE_STATUSW>() * num_services as usize]
                        .align_to::<ENUM_SERVICE_STATUSW>()
                        .1 // TODO should we check 0 and 2 so they are empty?
                        .to_vec()
                }
                .iter()
                .filter_map(|v| match parse_str_ptr(v.lpServiceName).to_string_lossy() {
                    x if x.starts_with(filter) => Some(x.into_owned()),
                    _ => None,
                })
                // TODO: It is possible that the service no longer exist at this point.
                // Could try to make this a bit more robust by not stopping on first error.
                .map(|service_name| get_service_handle(&service_name, service_manager))
                .collect::<Result<Vec<WinHandle>, String>>()
            }
        }
    })
}
const REGISTER_EVENT_BASE_KEY: &str = "SYSTEM\\CurrentControlSet\\Services\\EventLog\\Application";
pub fn register_windows_event(name: &str, exe_path: &str) -> Result<(), String> {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(REGISTER_EVENT_BASE_KEY)
        .and_then(|current_version| current_version.create_subkey(name))
        .and_then(|(app_key, _)| app_key.set_value("EventMessageFile", &exe_path))
        .map_err(|e| format!("Failed to register windows event \"{}\": {}", name, e))
}

pub fn deregister_windows_event(name: &str) -> Result<(), String> {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(REGISTER_EVENT_BASE_KEY)
        .and_then(|current_version| current_version.delete_subkey(name))
        .map_err(|e| format!("Failed to deregister windows event \"{}\": {}", name, e))
}
