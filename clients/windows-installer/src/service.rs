use std::{
    ffi::OsString,
    os::windows::prelude::{OsStrExt, OsStringExt},
    ptr, u32,
};

use thiserror::Error;
use winapi::{
    shared::{
        minwindef::{DWORD, LPCVOID, LPDWORD},
        ntdef::NULL,
        winerror::{ERROR_MORE_DATA, ERROR_SERVICE_DOES_NOT_EXIST, ERROR_SERVICE_NOT_ACTIVE},
    },
    um::{
        errhandlingapi::GetLastError,
        winbase::{FormatMessageW, FORMAT_MESSAGE_FROM_SYSTEM},
        winnt::{
            DELETE, GENERIC_ALL, LPCWSTR, LPWSTR, SERVICE_AUTO_START, SERVICE_ERROR_NORMAL,
            SERVICE_USER_OWN_PROCESS, SERVICE_WIN32_OWN_PROCESS,
        },
        winsvc::{
            CloseServiceHandle, ControlService, CreateServiceW, DeleteService, EnumServicesStatusW,
            OpenSCManagerW, OpenServiceW, QueryServiceStatus, StartServiceW, ENUM_SERVICE_STATUSW,
            LPENUM_SERVICE_STATUSW, LPSERVICE_STATUS, SC_HANDLE, SERVICE_CONTROL_STOP,
            SERVICE_QUERY_STATUS, SERVICE_START, SERVICE_STATE_ALL, SERVICE_STATUS, SERVICE_STOP,
            SERVICE_STOPPED,
        },
    },
};

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("Service does not exist")]
    ServiceDoesNotExist,

    #[error("WinApiError: {:?}", last_error_message())]
    WinApiError,

    #[error(r#"Failed to create service "{0}": {:?}"#, last_error_message())]
    FailedToCreateService(String),

    #[error(r#"Failed to get service "{0}": {:?}"#, last_error_message())]
    FailedToGetService(String),

    #[error(r#"Failed to start service "{0}": {:?}"#, last_error_message())]
    FailedToStartService(String),

    #[error(r#"Failed to delete service: "{0}" {:?}"#, last_error_message())]
    FailedToDeleteService(String),

    #[error(r#"Failed to get service status "{0}": {:?}"#, last_error_message())]
    FailedToGetServiceStatus(String),

    #[error(r#"Timed out after {0} seconds waiting for service "{1}" to stop."#)]
    TimedOutStoppingService(u64, String),

    #[error(r#"Failed to stop service "{0}": {:?}"#, last_error_message())]
    FailedToStopService(String),

    #[error("Failed to get service manager: {:?}", last_error_message())]
    FailedToGetServiceManager,

    #[error(r#"Failed to lookup services: {:?}"#, last_error_message())]
    FailedToLookupServices,

    #[error("Failed to mark file for reboot deletion: {0}")]
    FailedToMarkFileForRebootDeletion(String),

    #[error("Failed to mark folder \"{0}\" for deletion: {1}")]
    FailedToMarkDirectoryForRebootDeletion(String, std::io::Error),
}

impl From<ServiceError> for u32 {
    fn from(service_error: ServiceError) -> Self {
        match service_error {
            ServiceError::ServiceDoesNotExist => 30,
            ServiceError::WinApiError => 31,
            ServiceError::FailedToCreateService(_) => 32,
            ServiceError::FailedToGetService(_) => 33,
            ServiceError::FailedToStartService(_) => 34,
            ServiceError::FailedToDeleteService(_) => 35,
            ServiceError::FailedToGetServiceStatus(_) => 36,
            ServiceError::TimedOutStoppingService(_, _) => 37,
            ServiceError::FailedToStopService(_) => 38,
            ServiceError::FailedToGetServiceManager => 39,
            ServiceError::FailedToLookupServices => 40,
            ServiceError::FailedToMarkFileForRebootDeletion(_) => 41,
            ServiceError::FailedToMarkDirectoryForRebootDeletion(_, _) => 42,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WinHandle {
    name: String,
    handle: SC_HANDLE,
}

impl Drop for WinHandle {
    fn drop(&mut self) {
        unsafe {
            CloseServiceHandle(self.handle);
        }
    }
}

struct UserServiceEnumerator<'a> {
    manager_handle: &'a WinHandle,
    buffer: Vec<WinHandle>,
    resume_handle: u32,
    required_buffer_size: u32,
}

impl UserServiceEnumerator<'_> {
    fn list_services(
        manager_handle: &WinHandle,
        required_buffer_size: &mut u32,
        page: &mut u32,
    ) -> Result<Vec<WinHandle>, ServiceError> {
        let mut buf = Vec::<u8>::with_capacity(*required_buffer_size as usize);
        let mut num_services = 0u32;

        unsafe {
            buf.set_len(*required_buffer_size as usize);
        }
        match unsafe {
            EnumServicesStatusW(
                manager_handle.handle,
                SERVICE_USER_OWN_PROCESS,
                SERVICE_STATE_ALL,
                buf.as_mut_ptr() as _,
                buf.len() as u32,
                required_buffer_size as LPDWORD,
                &mut num_services as LPDWORD,
                page as LPDWORD,
            )
        } {
            0 if unsafe { GetLastError() } != ERROR_MORE_DATA => {
                Err(ServiceError::FailedToLookupServices)
            }
            _ => {
                Ok(unsafe {
                    buf[0..std::mem::size_of::<ENUM_SERVICE_STATUSW>() * num_services as usize]
                        .align_to::<ENUM_SERVICE_STATUSW>()
                        .1 // TODO should we check 0 and 2 so they are empty?
                        .to_vec()
                }
                .iter()
                .filter_map(|service| {
                    get_service_handle(
                        &parse_str_ptr(service.lpServiceName).to_string_lossy(),
                        &manager_handle,
                    )
                    .ok()
                })
                .collect::<Vec<WinHandle>>())
            }
        }
    }

    fn try_new(manager_handle: &WinHandle) -> Result<UserServiceEnumerator, ServiceError> {
        let mut required_buffer_size: u32 = 0;
        let mut num_services = 0u32;
        let mut page = 0u32;
        match unsafe {
            EnumServicesStatusW(
                manager_handle.handle,
                SERVICE_USER_OWN_PROCESS,
                SERVICE_STATE_ALL,
                NULL as LPENUM_SERVICE_STATUSW,
                0,
                &mut required_buffer_size as LPDWORD,
                &mut num_services as LPDWORD,
                &mut page as LPDWORD,
            )
        } {
            0 if unsafe { GetLastError() } == ERROR_MORE_DATA => Ok((required_buffer_size, page)),
            _ => Err(ServiceError::FailedToLookupServices),
        }
        .and_then(|(mut required_buffer_size, mut resume_handle)| {
            UserServiceEnumerator::list_services(
                manager_handle,
                &mut required_buffer_size,
                &mut resume_handle,
            )
            .map(|buffer| UserServiceEnumerator {
                manager_handle,
                buffer,
                resume_handle,
                required_buffer_size,
            })
        })
    }
}

impl std::iter::Iterator for UserServiceEnumerator<'_> {
    type Item = WinHandle;

    fn next(&mut self) -> Option<Self::Item> {
        self.buffer.pop().or_else(|| {
            self.buffer = UserServiceEnumerator::list_services(
                self.manager_handle,
                &mut self.required_buffer_size,
                &mut self.resume_handle,
            )
            .unwrap_or_default();
            self.buffer.pop()
        })
    }
}

fn parse_str_ptr(str_ptr: LPWSTR) -> OsString {
    OsString::from_wide(unsafe {
        let len = (0..).take_while(|&i| *str_ptr.offset(i) != 0).count();
        std::slice::from_raw_parts(str_ptr, len)
    })
}

pub fn win_string(s: &str) -> Vec<u16> {
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
        let nul_range_end = message
            .iter()
            .position(|&c| c == b'\0'.into())
            .unwrap_or(size as usize);
        format!(
            "{} ({})",
            String::from_utf16_lossy(&message[..nul_range_end]),
            error_code
        )
    }
}

pub fn create_service(
    name: &str,
    path: &str,
    handle: &WinHandle,
    args: &[&str],
    service_type: u32,
) -> Result<WinHandle, ServiceError> {
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
            service_type,
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
        service_handle if service_handle == (NULL as SC_HANDLE) => {
            Err(ServiceError::FailedToCreateService(name.to_string()))
        }
        service_handle => Ok(WinHandle {
            name: name.to_string(),
            handle: service_handle,
        }),
    }
}

pub fn create_user_service(
    name: &str,
    path: &str,
    handle: &WinHandle,
    args: &[&str],
) -> Result<WinHandle, ServiceError> {
    create_service(name, path, handle, args, SERVICE_USER_OWN_PROCESS)
}

pub fn create_system_service(
    name: &str,
    path: &str,
    handle: &WinHandle,
    args: &[&str],
) -> Result<WinHandle, ServiceError> {
    create_service(name, path, handle, args, SERVICE_WIN32_OWN_PROCESS)
}

pub fn get_service_handle(
    name: &str,
    manager_handle: &WinHandle,
) -> Result<WinHandle, ServiceError> {
    let win_name = win_string(name);
    match unsafe {
        OpenServiceW(
            manager_handle.handle,
            win_name.as_ptr(),
            DELETE | SERVICE_START | SERVICE_STOP | SERVICE_QUERY_STATUS,
        )
    } {
        v if v == (NULL as SC_HANDLE)
            && unsafe { GetLastError() } == ERROR_SERVICE_DOES_NOT_EXIST =>
        {
            Err(ServiceError::ServiceDoesNotExist)
        }
        v if v == (NULL as SC_HANDLE) => Err(ServiceError::FailedToGetService(name.to_string())),
        handle => Ok(WinHandle {
            name: name.to_string(),
            handle,
        }),
    }
}

pub fn delete_service(service_handle: &WinHandle) -> Result<(), ServiceError> {
    (unsafe { DeleteService(service_handle.handle) } != 0)
        .then(|| ())
        .ok_or_else(|| ServiceError::FailedToDeleteService(service_handle.name.clone()))
}

pub fn start_service(service_handle: &WinHandle) -> Result<(), ServiceError> {
    (unsafe { StartServiceW(service_handle.handle, 0, ptr::null_mut()) } != 0)
        .then(|| ())
        .ok_or_else(|| ServiceError::FailedToStartService(service_handle.name.clone()))
}

pub fn start_services(services: Vec<WinHandle>) -> Result<(), ServiceError> {
    services.iter().try_for_each(start_service)
}

pub fn get_service_status(service_handle: &WinHandle) -> Result<SERVICE_STATUS, ServiceError> {
    let mut service_status = SERVICE_STATUS::default();
    (unsafe {
        QueryServiceStatus(
            service_handle.handle,
            &mut service_status as LPSERVICE_STATUS,
        )
    } != 0)
        .then(|| service_status)
        .ok_or_else(|| ServiceError::FailedToGetServiceStatus(service_handle.name.clone()))
}

pub fn stop_service(service_handle: &WinHandle) -> Result<(), ServiceError> {
    let mut status: SERVICE_STATUS = Default::default();
    match unsafe {
        ControlService(
            service_handle.handle,
            SERVICE_CONTROL_STOP,
            &mut status as LPSERVICE_STATUS,
        )
    } {
        v if v == 0 && unsafe { GetLastError() } != ERROR_SERVICE_NOT_ACTIVE => Err(
            ServiceError::FailedToStopService(service_handle.name.clone()),
        ),
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
            Err(ServiceError::TimedOutStoppingService(
                TIMEOUT_SECONDS,
                service_handle.name.clone(),
            ))
        } else {
            Ok(())
        }
    })
}

pub fn get_service_manager(permissions: DWORD) -> Result<WinHandle, ServiceError> {
    match unsafe { OpenSCManagerW(NULL as LPCWSTR, NULL as LPCWSTR, permissions) } {
        v if v == (NULL as SC_HANDLE) => Err(ServiceError::FailedToGetServiceManager),
        handle => Ok(WinHandle {
            name: String::from("Service Manager"),
            handle,
        }),
    }
}

pub fn get_services(
    service_manager: &WinHandle,
    filter: &str,
) -> Result<Vec<WinHandle>, ServiceError> {
    UserServiceEnumerator::try_new(service_manager)
        .map(|enumerator| enumerator.filter(|n| n.name.starts_with(filter)).collect())
}
