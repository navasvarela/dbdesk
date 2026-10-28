use std::sync::Arc;

use dbdesk_core::{DataSourceId, Password};

use crate::paths::{APP_NAME, PREVIOUS_APP_NAME};

/// Where data source passwords are kept. An abstraction so tests and other platforms can
/// swap the OS keychain for something else.
pub trait SecretStore: Send + Sync + 'static {
    /// The saved secret, or `None` if there isn't one.
    fn get(&self, id: DataSourceId, secret: Secret) -> Result<Option<Password>, SecretError>;

    fn set(&self, id: DataSourceId, secret: Secret, value: &Password) -> Result<(), SecretError>;

    /// Removes the saved secret. Succeeds if there was none.
    fn delete(&self, id: DataSourceId, secret: Secret) -> Result<(), SecretError>;
}

/// The secrets a data source can have. Each is an entry of its own, so none replaces another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Secret {
    /// The password the data source signs in to its database with.
    Database,
    /// The password of its MCP login (`DataSource::mcp_login`).
    McpLogin,
    /// What its SSH login needs: the SSH user's password, or the key file's passphrase.
    Ssh,
}

impl Secret {
    pub const ALL: [Self; 3] = [Self::Database, Self::McpLogin, Self::Ssh];
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

/// The platform credential store, where the app keeps passwords, finding those saved under its
/// previous name too.
pub fn os_keychain() -> Arc<dyn SecretStore> {
    let current = Keychain { service: APP_NAME };
    // With one name, moving an entry would write it and then delete it from the same place.
    if APP_NAME == PREVIOUS_APP_NAME {
        return Arc::new(current);
    }
    Arc::new(CarriedOver {
        current,
        previous: Keychain {
            service: PREVIOUS_APP_NAME,
        },
    })
}

/// The platform credential store: Keychain on macOS, Credential Manager on Windows,
/// Secret Service on Linux. Entries are named `<service>` / `<data source id>`, then `/mcp` for
/// an MCP login's password and `/ssh` for the SSH login's secret. The service is the app's
/// name.
#[derive(Debug, Clone, Copy)]
struct Keychain {
    service: &'static str,
}

impl Keychain {
    fn entry(&self, id: DataSourceId, secret: Secret) -> Result<keyring::Entry, SecretError> {
        let account = match secret {
            Secret::Database => id.to_string(),
            Secret::McpLogin => format!("{id}/mcp"),
            Secret::Ssh => format!("{id}/ssh"),
        };
        keyring::Entry::new(self.service, &account).map_err(SecretError)
    }
}

impl SecretStore for Keychain {
    fn get(&self, id: DataSourceId, secret: Secret) -> Result<Option<Password>, SecretError> {
        match self.entry(id, secret)?.get_password() {
            Ok(password) => Ok(Some(Password::new(password))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(SecretError(error)),
        }
    }

    fn set(&self, id: DataSourceId, secret: Secret, value: &Password) -> Result<(), SecretError> {
        self.entry(id, secret)?
            .set_password(value.expose())
            .map_err(SecretError)
    }

    fn delete(&self, id: DataSourceId, secret: Secret) -> Result<(), SecretError> {
        match self.entry(id, secret)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(SecretError(error)),
        }
    }
}

/// Finds secrets saved under the app's previous name too, and moves each to the current name
/// the first time it is read. Moving them on first use rather than all at the first start
/// keeps macOS from asking for Keychain access to every saved password at once, before the
/// user has asked for anything. See "Migrating existing installs" in `docs/releasing.md`.
struct CarriedOver<Current, Previous> {
    current: Current,
    previous: Previous,
}

impl<Current: SecretStore, Previous: SecretStore> SecretStore for CarriedOver<Current, Previous> {
    fn get(&self, id: DataSourceId, secret: Secret) -> Result<Option<Password>, SecretError> {
        if let Some(value) = self.current.get(id, secret)? {
            return Ok(Some(value));
        }
        let Some(value) = self.previous.get(id, secret)? else {
            return Ok(None);
        };
        // The old entry goes only once the new one is saved, so a failure on the way never
        // loses the secret. Either failure leaves an entry that is found again next time, and
        // the secret is usable now regardless.
        if self.current.set(id, secret, &value).is_ok() {
            let _ = self.previous.delete(id, secret);
        }
        Ok(Some(value))
    }

    fn set(&self, id: DataSourceId, secret: Secret, value: &Password) -> Result<(), SecretError> {
        self.current.set(id, secret, value)
    }

    fn delete(&self, id: DataSourceId, secret: Secret) -> Result<(), SecretError> {
        let current = self.current.delete(id, secret);
        let previous = self.previous.delete(id, secret);
        current.and(previous)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    /// A store in memory that can be told to refuse writes, standing in for a keychain.
    #[derive(Default)]
    struct MemoryStore {
        entries: Mutex<HashMap<(DataSourceId, Secret), String>>,
        refuses_writes: bool,
    }

    impl MemoryStore {
        fn holding(id: DataSourceId, secret: Secret, value: &str) -> Self {
            let store = Self::default();
            store.insert(id, secret, value);
            store
        }

        fn insert(&self, id: DataSourceId, secret: Secret, value: &str) {
            self.entries
                .lock()
                .unwrap()
                .insert((id, secret), value.to_owned());
        }

        fn value(&self, id: DataSourceId, secret: Secret) -> Option<String> {
            self.entries.lock().unwrap().get(&(id, secret)).cloned()
        }
    }

    impl SecretStore for MemoryStore {
        fn get(&self, id: DataSourceId, secret: Secret) -> Result<Option<Password>, SecretError> {
            Ok(self.value(id, secret).map(Password::new))
        }

        fn set(
            &self,
            id: DataSourceId,
            secret: Secret,
            value: &Password,
        ) -> Result<(), SecretError> {
            if self.refuses_writes {
                return Err(SecretError::unavailable("locked"));
            }
            self.insert(id, secret, value.expose());
            Ok(())
        }

        fn delete(&self, id: DataSourceId, secret: Secret) -> Result<(), SecretError> {
            self.entries.lock().unwrap().remove(&(id, secret));
            Ok(())
        }
    }

    fn carried_over(
        current: MemoryStore,
        previous: MemoryStore,
    ) -> CarriedOver<MemoryStore, MemoryStore> {
        CarriedOver { current, previous }
    }

    #[test]
    fn reads_the_current_name_first_and_moves_what_only_the_previous_one_has() {
        let id = DataSourceId::generate();
        let both = carried_over(
            MemoryStore::holding(id, Secret::Database, "new"),
            MemoryStore::holding(id, Secret::Database, "old"),
        );
        let only_previous = carried_over(
            MemoryStore::default(),
            MemoryStore::holding(id, Secret::Database, "old"),
        );
        let neither = carried_over(MemoryStore::default(), MemoryStore::default());

        assert_eq!(
            both.get(id, Secret::Database).unwrap(),
            Some(Password::new("new"))
        );
        assert_eq!(
            both.previous.value(id, Secret::Database).as_deref(),
            Some("old")
        );

        assert_eq!(
            only_previous.get(id, Secret::Database).unwrap(),
            Some(Password::new("old"))
        );
        assert_eq!(
            only_previous.current.value(id, Secret::Database).as_deref(),
            Some("old")
        );
        assert_eq!(only_previous.previous.value(id, Secret::Database), None);
        assert_eq!(only_previous.get(id, Secret::Ssh).unwrap(), None);

        assert_eq!(neither.get(id, Secret::Database).unwrap(), None);
    }

    #[test]
    fn keeps_the_previous_entry_when_the_current_name_refuses_it() {
        let id = DataSourceId::generate();
        let store = carried_over(
            MemoryStore {
                refuses_writes: true,
                ..MemoryStore::default()
            },
            MemoryStore::holding(id, Secret::McpLogin, "old"),
        );

        assert_eq!(
            store.get(id, Secret::McpLogin).unwrap(),
            Some(Password::new("old"))
        );
        assert_eq!(
            store.previous.value(id, Secret::McpLogin).as_deref(),
            Some("old")
        );
        assert_eq!(store.current.value(id, Secret::McpLogin), None);
    }

    #[test]
    fn saves_under_the_current_name_and_deletes_under_both() {
        let id = DataSourceId::generate();
        let store = carried_over(
            MemoryStore::default(),
            MemoryStore::holding(id, Secret::Ssh, "old"),
        );

        store.set(id, Secret::Ssh, &Password::new("new")).unwrap();
        assert_eq!(store.current.value(id, Secret::Ssh).as_deref(), Some("new"));
        assert_eq!(
            store.previous.value(id, Secret::Ssh).as_deref(),
            Some("old")
        );

        store.delete(id, Secret::Ssh).unwrap();
        assert_eq!(store.current.value(id, Secret::Ssh), None);
        assert_eq!(store.previous.value(id, Secret::Ssh), None);
    }

    /// Touches the real OS keychain, so it only runs on request:
    /// `cargo test -p dbdesk-store -- --ignored`. Cleans up the entries it creates.
    #[test]
    #[ignore = "uses the real OS keychain"]
    fn keeps_each_secret_apart_in_the_os_keychain() {
        let id = DataSourceId::generate();
        let value = |secret: Secret| Password::new(format!("dbdesk keychain test, {secret:?}"));

        let keychain = Keychain { service: APP_NAME };
        for secret in Secret::ALL {
            keychain.set(id, secret, &value(secret)).unwrap();
        }
        let read_back: Vec<_> = Secret::ALL
            .iter()
            .map(|secret| keychain.get(id, *secret))
            .collect();
        let deleted: Vec<_> = Secret::ALL
            .iter()
            .map(|secret| keychain.delete(id, *secret))
            .collect();

        for (secret, read) in Secret::ALL.into_iter().zip(read_back) {
            assert_eq!(read.unwrap(), Some(value(secret)));
        }
        for result in deleted {
            result.unwrap();
        }
        assert_eq!(keychain.get(id, Secret::Ssh).unwrap(), None);
        keychain.delete(id, Secret::Ssh).unwrap();
    }
}
