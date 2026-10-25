use dbdesk_core::{DataSourceId, Password};

/// Where data source passwords are kept. An abstraction so tests and other platforms can
/// swap the OS keychain for something else.
pub trait SecretStore: Send + Sync + 'static {
    /// The saved password, or `None` if there isn't one.
    fn password(&self, id: DataSourceId) -> Result<Option<Password>, SecretError>;

    fn set_password(&self, id: DataSourceId, password: &Password) -> Result<(), SecretError>;

    /// Removes the saved password. Succeeds if there was none.
    fn delete_password(&self, id: DataSourceId) -> Result<(), SecretError>;
}

#[derive(Debug, thiserror::Error)]
#[error("could not access the OS keychain")]
pub struct SecretError(#[source] keyring::Error);

impl SecretError {
    /// A store that can't be used, for [`SecretStore`]s other than the OS keychain (such as
    /// tests standing in for a locked one).
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self(keyring::Error::PlatformFailure(reason.into().into()))
    }
}

/// The platform credential store: Keychain on macOS, Credential Manager on Windows,
/// Secret Service on Linux. Entries are named `dbdesk` / `<data source id>`.
#[derive(Debug, Default, Clone, Copy)]
pub struct Keychain;

const KEYCHAIN_SERVICE: &str = "dbdesk";

impl Keychain {
    fn entry(id: DataSourceId) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(KEYCHAIN_SERVICE, &id.to_string()).map_err(SecretError)
    }
}

impl SecretStore for Keychain {
    fn password(&self, id: DataSourceId) -> Result<Option<Password>, SecretError> {
        match Self::entry(id)?.get_password() {
            Ok(password) => Ok(Some(Password::new(password))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(SecretError(error)),
        }
    }

    fn set_password(&self, id: DataSourceId, password: &Password) -> Result<(), SecretError> {
        Self::entry(id)?
            .set_password(password.expose())
            .map_err(SecretError)
    }

    fn delete_password(&self, id: DataSourceId) -> Result<(), SecretError> {
        match Self::entry(id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(SecretError(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Touches the real OS keychain, so it only runs on request:
    /// `cargo test -p dbdesk-store -- --ignored`. Cleans up the entry it creates.
    #[test]
    #[ignore = "uses the real OS keychain"]
    fn round_trips_a_password_through_the_os_keychain() {
        let id = DataSourceId::generate();
        let password = Password::new("dbdesk keychain test");

        Keychain.set_password(id, &password).unwrap();
        let read_back = Keychain.password(id);
        let deleted = Keychain.delete_password(id);

        assert_eq!(read_back.unwrap(), Some(password));
        deleted.unwrap();
        assert_eq!(Keychain.password(id).unwrap(), None);
        Keychain.delete_password(id).unwrap();
    }
}
