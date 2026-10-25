use dbdesk_core::{DataSourceId, Password};

/// Where data source passwords are kept. An abstraction so tests and other platforms can
/// swap the OS keychain for something else.
pub trait SecretStore: Send + Sync + 'static {
    /// The saved password, or `None` if there isn't one.
    fn password(&self, id: DataSourceId) -> Result<Option<Password>, SecretError>;

    fn set_password(&self, id: DataSourceId, password: &Password) -> Result<(), SecretError>;

    /// Removes the saved password. Succeeds if there was none.
    fn delete_password(&self, id: DataSourceId) -> Result<(), SecretError>;

    /// The saved password of the data source's MCP login (`DataSource::mcp_login`), kept
    /// apart from its own so neither replaces the other.
    fn mcp_password(&self, id: DataSourceId) -> Result<Option<Password>, SecretError>;

    fn set_mcp_password(&self, id: DataSourceId, password: &Password) -> Result<(), SecretError>;

    /// Removes the MCP login's saved password. Succeeds if there was none.
    fn delete_mcp_password(&self, id: DataSourceId) -> Result<(), SecretError>;
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
/// Secret Service on Linux. Entries are named `dbdesk` / `<data source id>`, and
/// `dbdesk` / `<data source id>/mcp` for an MCP login.
#[derive(Debug, Default, Clone, Copy)]
pub struct Keychain;

const KEYCHAIN_SERVICE: &str = "dbdesk";

impl Keychain {
    fn entry(account: &str) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(KEYCHAIN_SERVICE, account).map_err(SecretError)
    }

    fn own_entry(id: DataSourceId) -> Result<keyring::Entry, SecretError> {
        Self::entry(&id.to_string())
    }

    fn mcp_entry(id: DataSourceId) -> Result<keyring::Entry, SecretError> {
        Self::entry(&format!("{id}/mcp"))
    }
}

fn read(entry: &keyring::Entry) -> Result<Option<Password>, SecretError> {
    match entry.get_password() {
        Ok(password) => Ok(Some(Password::new(password))),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(SecretError(error)),
    }
}

fn write(entry: &keyring::Entry, password: &Password) -> Result<(), SecretError> {
    entry.set_password(password.expose()).map_err(SecretError)
}

fn delete(entry: &keyring::Entry) -> Result<(), SecretError> {
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(SecretError(error)),
    }
}

impl SecretStore for Keychain {
    fn password(&self, id: DataSourceId) -> Result<Option<Password>, SecretError> {
        read(&Self::own_entry(id)?)
    }

    fn set_password(&self, id: DataSourceId, password: &Password) -> Result<(), SecretError> {
        write(&Self::own_entry(id)?, password)
    }

    fn delete_password(&self, id: DataSourceId) -> Result<(), SecretError> {
        delete(&Self::own_entry(id)?)
    }

    fn mcp_password(&self, id: DataSourceId) -> Result<Option<Password>, SecretError> {
        read(&Self::mcp_entry(id)?)
    }

    fn set_mcp_password(&self, id: DataSourceId, password: &Password) -> Result<(), SecretError> {
        write(&Self::mcp_entry(id)?, password)
    }

    fn delete_mcp_password(&self, id: DataSourceId) -> Result<(), SecretError> {
        delete(&Self::mcp_entry(id)?)
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

    /// Like the test above, for the MCP login's entry, which must not touch the other.
    #[test]
    #[ignore = "uses the real OS keychain"]
    fn keeps_the_mcp_logins_password_apart() {
        let id = DataSourceId::generate();
        let own = Password::new("dbdesk keychain test");
        let mcp = Password::new("dbdesk keychain test, MCP");

        Keychain.set_password(id, &own).unwrap();
        Keychain.set_mcp_password(id, &mcp).unwrap();
        let read_back = (Keychain.password(id), Keychain.mcp_password(id));
        let deleted = (
            Keychain.delete_password(id),
            Keychain.delete_mcp_password(id),
        );

        assert_eq!(read_back.0.unwrap(), Some(own));
        assert_eq!(read_back.1.unwrap(), Some(mcp));
        deleted.0.unwrap();
        deleted.1.unwrap();
        assert_eq!(Keychain.mcp_password(id).unwrap(), None);
    }
}
