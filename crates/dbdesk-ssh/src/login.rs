//! Signing in to the SSH server: with the agent's keys, a key file, or a password.

use std::path::Path;
use std::sync::Arc;

use dbdesk_core::{Password, SshLogin};
use russh::client::Handle;
use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::AgentClient;
use russh::keys::{PrivateKey, PrivateKeyWithHashAlg, PublicKey};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{SecretNeeded, ServerCheck, SshError};

/// What signing in needs, read before connecting so a missing secret is asked for straight
/// away rather than after a round trip to the server.
pub(crate) enum Credentials {
    Agent,
    Key(Arc<PrivateKey>),
    Password(Password),
}

impl Credentials {
    /// # Errors
    ///
    /// [`SshError::SecretNeeded`] for a password login, or an encrypted key, without a
    /// secret; [`SshError::KeyFile`] if the key file can't be read or decrypted.
    pub(crate) async fn prepare(
        login: &SshLogin,
        secret: Option<&Password>,
    ) -> Result<Self, SshError> {
        match login {
            SshLogin::Agent => Ok(Self::Agent),
            SshLogin::Password => secret
                .cloned()
                .map(Self::Password)
                .ok_or(SshError::SecretNeeded(SecretNeeded::Password)),
            SshLogin::KeyFile { key_file } => read_key(key_file, secret).await.map(Self::Key),
        }
    }
}

async fn read_key(path: &Path, passphrase: Option<&Password>) -> Result<Arc<PrivateKey>, SshError> {
    let key_file_error = |source| SshError::KeyFile {
        path: path.to_owned(),
        source,
    };
    let text = tokio::fs::read_to_string(path)
        .await
        .map_err(|error| key_file_error(russh::keys::Error::from(error)))?;
    match russh::keys::decode_secret_key(&text, passphrase.map(Password::expose)) {
        Ok(key) => Ok(Arc::new(key)),
        Err(russh::keys::Error::KeyIsEncrypted) if passphrase.is_none() => {
            Err(SshError::SecretNeeded(SecretNeeded::Passphrase))
        }
        Err(error) => Err(key_file_error(error)),
    }
}

/// Signs in as `user`.
///
/// # Errors
///
/// [`SshError::LoginRefused`] if the server turns every attempt down, or the error that kept
/// one from being made.
pub(crate) async fn sign_in(
    session: &mut Handle<ServerCheck>,
    user: &str,
    credentials: Credentials,
) -> Result<(), SshError> {
    let (accepted, method) = match credentials {
        Credentials::Password(password) => {
            let result = session
                .authenticate_password(user, password.expose())
                .await
                .map_err(SshError::Handshake)?;
            (result.success(), "password")
        }
        Credentials::Key(key) => {
            let hash = session
                .best_supported_rsa_hash()
                .await
                .map_err(SshError::Handshake)?
                .flatten();
            let result = session
                .authenticate_publickey(user, PrivateKeyWithHashAlg::new(key, hash))
                .await
                .map_err(SshError::Handshake)?;
            (result.success(), "key")
        }
        Credentials::Agent => (
            sign_in_with_agent(session, user).await?,
            "keys in the SSH agent",
        ),
    };
    if accepted {
        Ok(())
    } else {
        Err(SshError::LoginRefused {
            user: user.to_owned(),
            method,
        })
    }
}

/// Tries each key the agent holds, as `ssh` does. `Ok(false)` if the server took none.
async fn sign_in_with_agent(
    session: &mut Handle<ServerCheck>,
    user: &str,
) -> Result<bool, SshError> {
    #[cfg(unix)]
    {
        let mut agent = AgentClient::connect_env()
            .await
            .map_err(|error| SshError::Agent(error.to_string()))?;
        try_agent_keys(session, user, &mut agent).await
    }
    #[cfg(windows)]
    {
        // Windows' own OpenSSH agent listens on a named pipe; PuTTY's Pageant on a window.
        const OPENSSH_AGENT_PIPE: &str = r"\\.\pipe\openssh-ssh-agent";
        match AgentClient::connect_named_pipe(OPENSSH_AGENT_PIPE).await {
            Ok(mut agent) => try_agent_keys(session, user, &mut agent).await,
            Err(_) => {
                let mut agent = AgentClient::connect_pageant()
                    .await
                    .map_err(|error| SshError::Agent(error.to_string()))?;
                try_agent_keys(session, user, &mut agent).await
            }
        }
    }
}

async fn try_agent_keys<S>(
    session: &mut Handle<ServerCheck>,
    user: &str,
    agent: &mut AgentClient<S>,
) -> Result<bool, SshError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let identities = agent
        .request_identities()
        .await
        .map_err(|error| SshError::Agent(error.to_string()))?;
    let keys: Vec<PublicKey> = identities
        .into_iter()
        .filter_map(|identity| match identity {
            AgentIdentity::PublicKey { key, .. } => Some(key),
            AgentIdentity::Certificate { .. } => None,
        })
        .collect();
    if keys.is_empty() {
        return Err(SshError::Agent("the SSH agent holds no keys".to_owned()));
    }
    let hash = session
        .best_supported_rsa_hash()
        .await
        .map_err(SshError::Handshake)?
        .flatten();
    for key in keys {
        let result = session
            .authenticate_publickey_with(user, key, hash, agent)
            .await
            .map_err(|error| SshError::Agent(error.to_string()))?;
        if result.success() {
            return Ok(true);
        }
    }
    Ok(false)
}
