use winapi::um::{
    winnt::{DELETE, SERVICE_USER_OWN_PROCESS, SERVICE_WIN32_OWN_PROCESS},
    winsvc::{SC_MANAGER_CREATE_SERVICE, SC_MANAGER_ENUMERATE_SERVICE},
};

use crate::service::{
    create_service, delete_service, get_service_handle, get_service_manager, get_services,
    start_service, stop_service, ServiceError, WinHandle,
};

pub struct Service {
    service_handle: WinHandle,
}

impl Service {
    pub fn stop(self) -> Result<Self, ServiceError> {
        stop_service(&self.service_handle).map(|_| self)
    }

    pub fn start(&self) -> Result<(), ServiceError> {
        start_service(&self.service_handle)
    }

    pub fn delete(self) -> Result<(), ServiceError> {
        delete_service(&self.service_handle)
    }
}

impl From<WinHandle> for Service {
    fn from(service_handle: WinHandle) -> Self {
        Self { service_handle }
    }
}

pub struct ServiceManager {
    service_manager_handle: WinHandle,
}

impl ServiceManager {
    pub fn try_new() -> Result<Self, ServiceError> {
        /*
        TODO: We do not really know if we need for example DELETE and SC_MANAGER_CREATE_SERVICE.
        We just take everything for now to ensure everything works but it is a bit greedy.
        We could also just let the user send them in but at the same time i do not want to leak the types.
        Also feels kinda dumb to just wrap the types.
        */
        get_service_manager(SC_MANAGER_ENUMERATE_SERVICE | DELETE | SC_MANAGER_CREATE_SERVICE).map(
            |service_manager_handle| Self {
                service_manager_handle,
            },
        )
    }

    pub fn get_service(&self, name: &str) -> Result<Service, ServiceError> {
        get_service_handle(&self.service_manager_handle, name).map(Service::from)
    }

    pub fn get_services(&self, filter: &str) -> Result<Vec<Service>, ServiceError> {
        get_services(&self.service_manager_handle, filter)
            .map(|services| services.into_iter().map(Service::from).collect())
    }

    pub fn start_services(&self, filter: &str) -> Result<(), ServiceError> {
        get_services(&self.service_manager_handle, filter)
            .and_then(|services| services.iter().try_for_each(start_service))
    }

    pub fn start_service(&self, name: &str) -> Result<(), ServiceError> {
        get_service_handle(&self.service_manager_handle, name)
            .and_then(|service| start_service(&service))
    }

    pub fn stop_services(&self, filter: &str) -> Result<(), ServiceError> {
        get_services(&self.service_manager_handle, filter)
            .and_then(|services| services.iter().try_for_each(stop_service))
    }

    pub fn stop_service(&self, name: &str) -> Result<(), ServiceError> {
        get_service_handle(&self.service_manager_handle, name)
            .and_then(|service| stop_service(&service))
    }

    pub fn delete_service(&self, name: &str) -> Result<(), ServiceError> {
        get_service_handle(&self.service_manager_handle, name)
            .and_then(|service| delete_service(&service))
    }

    pub fn create_user_service(
        &self,
        name: &str,
        path: &str,
        args: &[&str],
    ) -> Result<Service, ServiceError> {
        create_service(
            name,
            path,
            &self.service_manager_handle,
            args,
            SERVICE_USER_OWN_PROCESS,
        )
        .map(Service::from)
    }

    pub fn create_system_service(
        &self,
        name: &str,
        path: &str,
        args: &[&str],
    ) -> Result<Service, ServiceError> {
        create_service(
            name,
            path,
            &self.service_manager_handle,
            args,
            SERVICE_WIN32_OWN_PROCESS,
        )
        .map(Service::from)
    }
}
