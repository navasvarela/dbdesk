//! Finding the secrets a connection signs in with, the database password and the SSH login's:
//! typed in the dialog, given at a prompt for this session, or saved in the keychain.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use dbrover_core::{DataSourceId, Password, describe_error};
use dbrover_store::{Secret, SecretError, SecretStore};

use crate::PasswordUpdate;
use crate::open::OpenError;

/// Secrets given at the prompt, kept in memory until DbRover quits and never written to disk.
/// They are what lets a data source connect when its keychain entry can't be written, or
/// when the user chose not to save it.
///
/// A mutex rather than messages to the command loop, because each session looks its secrets
/// up from its own task; the lock is only held to copy one entry in or out.
#[derive(Debug, Clone, Default)]
pub(crate) struct SessionPasswords(Arc<Mutex<HashMap<(DataSourceId, Secret), Password>>>);

impl SessionPasswords {
    pub(crate) fn get(&self, id: DataSourceId, secret: Secret) -> Option<Password> {
        self.lock().get(&(id, secret)).cloned()
    }

    pub(crate) fn insert(&self, id: DataSourceId, secret: Secret, password: Password) {
        self.lock().insert((id, secret), password);
    }

    /// Forgets the secret, once the keychain holds a newer one.
    pub(crate) fn remove(&self, id: DataSourceId, secret: Secret) {
        self.lock().remove(&(id, secret));
    }

    /// Forgets every secret of a data source that is gone.
    pub(crate) fn remove_all(&self, id: DataSourceId) {
        self.lock().retain(|(entry, _), _| *entry != id);
    }

    /// A panic while holding the lock can't leave a map of passwords half-updated, so a
    /// poisoned lock is still safe to use.
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<(DataSourceId, Secret), Password>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// What happens when a secret is needed and none is known.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Prompting<'a> {
    /// Fail with [`OpenError::PasswordNeeded`], so the UI asks for one. The answers given so
    /// far are looked up first.
    Ask(&'a SessionPasswords),
    /// Go on without one, for front ends that can't ask (`dbrover mcp`) and for the dialog's
    /// connection test, which uses what the form holds.
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
            Self::NoneSaved => f.write_str("none is saved"),
            Self::KeychainUnavailable(error) => write!(f, "the keychain couldn't be read: {error}"),
        }
    }
}

/// How errors and logs name a secret.
pub(crate) fn describe(secret: Secret) -> &'static str {
    match secret {
        Secret::Database => "a password",
        Secret::McpLogin => "the MCP login's password",
        Secret::Ssh => "the SSH password or key passphrase",
    }
}

/// The secret typed in the dialog or given at the prompt for this session, if there is one:
/// what is known without asking the keychain.
pub(crate) fn known(
    id: DataSourceId,
    secret: Secret,
    typed: &PasswordUpdate,
    prompting: Prompting<'_>,
) -> Option<Password> {
    match (typed, prompting) {
        (PasswordUpdate::Set(password), _) => Some(password.clone()),
        (PasswordUpdate::Keep, Prompting::Ask(remembered)) => remembered.get(id, secret),
        (PasswordUpdate::Keep, Prompting::Never) => None,
    }
}

/// The secret to sign in with: [`known`], or else the saved one. `None` if there is none and
/// the user can't be asked.
pub(crate) async fn resolve(
    id: DataSourceId,
    secret: Secret,
    secrets: &Arc<dyn SecretStore>,
    typed: &PasswordUpdate,
    prompting: Prompting<'_>,
) -> Result<Option<Password>, OpenError> {
    if let Some(password) = known(id, secret, typed, prompting) {
        return Ok(Some(password));
    }
    let saved = read_saved(id, secret, secrets).await?;
    let needed = |reason| OpenError::PasswordNeeded { secret, reason };
    match (saved, prompting) {
        (Ok(Some(password)), _) => Ok(Some(password)),
        (Ok(None), Prompting::Never) => Ok(None),
        (Ok(None), Prompting::Ask(_)) => Err(needed(PasswordNeeded::NoneSaved)),
        (Err(error), Prompting::Never) => Err(OpenError::Password(error)),
        (Err(error), Prompting::Ask(_)) => Err(needed(PasswordNeeded::KeychainUnavailable(
            describe_error(&error),
        ))),
    }
}

/// The keychain's answer, or the task failure that kept it from answering.
async fn read_saved(
    id: DataSourceId,
    secret: Secret,
    secrets: &Arc<dyn SecretStore>,
) -> Result<Result<Option<Password>, SecretError>, OpenError> {
    let secrets = Arc::clone(secrets);
    tokio::task::spawn_blocking(move || secrets.get(id, secret))
        .await
        .map_err(OpenError::Task)
}
