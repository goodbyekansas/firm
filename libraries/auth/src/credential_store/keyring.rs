use keyring::{error::Error, Entry};

use crate::credential_store::CredentialStore;
use crate::credential_store::Error as CredError;

pub struct KeyRing {
    service_name: String,
}

impl KeyRing {
    pub fn new<S: AsRef<str>>(service_name: S) -> Self {
        Self {
            service_name: service_name.as_ref().to_string(),
        }
    }

    fn get(&self, user: &str) -> Result<Option<String>, Error> {
        Entry::new(&self.service_name, user).and_then(|entry| match entry.get_password() {
            Ok(pass) => Ok(Some(pass)),
            Err(Error::NoEntry) => Ok(None),
            Err(e) => Err(e),
        })
    }

    fn set(&self, user: &str, value: &str) -> Result<(), Error> {
        Entry::new(&self.service_name, user).and_then(|entry| entry.set_password(value))
    }
}

impl CredentialStore for KeyRing {
    fn store(&mut self, key: &str, value: &str) -> Result<(), super::Error> {
        self.set(key, value)
            .map_err(|e| CredError::Generic(e.to_string()))
    }

    fn retrieve(&self, key: &str) -> Result<Option<String>, super::Error> {
        self.get(key).map_err(|e| CredError::Generic(e.to_string()))
    }
}

#[cfg(test)]
mod test {
    use super::KeyRing;
    use keyring::{credential::CredentialBuilderApi, mock::MockCredentialBuilder};

    // Haven't found a good way to test this (whithout overly complex
    // solutions or things that changes the implementation
    // drastically). So I'm just going to test it with that tools are
    // available.

    // The builder they provide saves no state at all. This means that
    // if you get an entry, change the password and then try to get
    // the same entry again the password will be lost. So I can't
    // check if setting actually works.

    // The problem is the builder. It is expected to create new
    // entries every time (no refs). The method that creates the
    // entries `build` is not mut so you can't track state there
    // (unless you want to go with locks) and it's the only function
    // you got available where you could possibly do that. The entry
    // is the state and we're decoupled from it.

    // Could make our own credentials implementation that uses a bus
    // to signal back what has been set on the credentials and let the
    // builder know. The amount of lines in mock code would greatly
    // outnumber the original implementation in lines. Can't be
    // bothered.

    #[test]
    fn keyring() {
        let builder = Box::new(MockCredentialBuilder {})
            as Box<(dyn CredentialBuilderApi + Send + Sync + 'static)>;
        keyring::set_default_credential_builder(builder);
        let keyring = KeyRing::new("test-service");

        // Getting key that does not exist results in none
        let res = keyring.get("second floor basement");
        assert!(res.is_ok());
        let res = res.unwrap();
        assert!(res.is_none());

        // Setting values and ensure they get updated.
        let res = keyring.set("psychomantis", "gene therapy?");
        assert!(res.is_ok());

        let res = keyring.get("psychomantis");
        assert!(res.is_ok());
        // Password is lost here.
        //let res = res.unwrap();
        //assert!(res.is_some());
        //let key = res.unwrap();
        //assert_eq!(key, "gene therapy?");
    }
}
