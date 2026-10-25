//! Finding the password a connection signs in with: typed in the dialog, given at a prompt
//! for this session, or saved in the keychain.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use dbdesk_core::{DataSource, DataSourceId, Password, describe_error};
use dbdesk_store::{SecretError, SecretStore};

use crate::PasswordUpdate;
use crate::open::OpenError;

/// Passwords given at the prompt, kept in memory until dbdesk quits and never written to
/// disk. They are what lets a data source connect when its keychain entry can't be written,
/// or when the user chose not to save it.
///
/// A mutex rather than messages to the command loop, because each session looks its password
/// up from its own task; the lock is only held to copy one entry in or out.
#[derive(Debug, Clone, Default)]
pub(crate) struct SessionPasswords(Arc<Mutex<HashMap<DataSourceId, Password>>>);

impl SessionPasswords {
    pub(crate) fn get(&self, id: DataSourceId) -> Option<Password> {
        self.lock().get(&id).cloned()
    }

    pub(crate) fn insert(&self, id: DataSourceId, password: Password) {
        self.lock().insert(id, password);
    }

    /// Forgets the password, once the keychain holds a newer one or the data source is gone.
    pub(crate) fn remove(&self, id: DataSourceId) {
        self.lock().remove(&id);
    }

    /// A panic while holding the lock can't leave a map of passwords half-updated, so a
    /// poisoned lock is still safe to use.
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<DataSourceId, Password>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// What happens when a data source signs in with a password and none is known.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Prompting<'a> {
    /// Fail with [`OpenError::PasswordNeeded`], so the UI asks for one. The answers given so
    /// far are looked up first.
    Ask(&'a SessionPasswords),
    /// Connect without one, for front ends that can't ask (`dbdesk mcp`) and for the
    /// dialog's connection test, which uses what the form holds.
    Never,
}

/// Why the user has to be asked for a password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasswordNeeded {
    NoneSaved,
    /// The keychain couldn't be read: locked, declined, or not available on this system.
    KeychainUnavailable(String),
}

impl fmt::Display for PasswordNeeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoneSaved => f.write_str("no password is saved"),
            Self::KeychainUnavailable(error) => write!(f, "the keychain couldn't be read: {error}"),
        }
    }
}

/// The password to connect with, if any.
pub(crate) async fn resolve(
    data_source: &DataSource,
    secrets: &Arc<dyn SecretStore>,
    password: &PasswordUpdate,
    prompting: Prompting<'_>,
) -> Result<Option<Password>, OpenError> {
    if let PasswordUpdate::Set(password) = password {
        return Ok(Some(password.clone()));
    }
    if !data_source.config.driver().uses_password() {
        return Ok(None);
    }
    if let Prompting::Ask(remembered) = prompting
        && let Some(password) = remembered.get(data_source.id)
    {
        return Ok(Some(password));
    }
    let saved = read_saved(data_source.id, secrets).await?;
    match (saved, prompting) {
        (Ok(Some(password)), _) => Ok(Some(password)),
        (Ok(None), Prompting::Never) => Ok(None),
        (Ok(None), Prompting::Ask(_)) => Err(OpenError::PasswordNeeded(PasswordNeeded::NoneSaved)),
        (Err(error), Prompting::Never) => Err(OpenError::Password(error)),
        (Err(error), Prompting::Ask(_)) => Err(OpenError::PasswordNeeded(
            PasswordNeeded::KeychainUnavailable(describe_error(&error)),
        )),
    }
}

/// The keychain's answer, or the task failure that kept it from answering.
async fn read_saved(
    id: DataSourceId,
    secrets: &Arc<dyn SecretStore>,
) -> Result<Result<Option<Password>, SecretError>, OpenError> {
    let secrets = Arc::clone(secrets);
    tokio::task::spawn_blocking(move || secrets.password(id))
        .await
        .map_err(OpenError::Task)
}
