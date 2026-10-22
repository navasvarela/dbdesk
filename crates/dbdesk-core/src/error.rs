use std::error::Error;

/// The engine-specific error a driver wraps. Boxed because core can't name driver crates'
/// error types; [`DriverError`]'s variants carry what callers need to match on.
pub type BoxError = Box<dyn Error + Send + Sync>;

#[derive(Debug, thiserror::Error)]
pub enum DriverError {
    /// The connection could not be established: unreachable host, bad credentials, missing
    /// file, and so on.
    #[error("could not connect")]
    Connect(#[source] BoxError),
    /// An established connection failed to do what was asked.
    #[error("the connection failed")]
    Connection(#[source] BoxError),
}

/// Formats an error and all its sources as one line, outermost first, e.g.
/// `could not connect: db error: FATAL: password authentication failed for user "juan"`.
pub fn describe_error(error: &(dyn Error + 'static)) -> String {
    let mut description = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        description.push_str(": ");
        description.push_str(&cause.to_string());
        source = cause.source();
    }
    description
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_the_whole_chain() {
        let error = DriverError::Connect(BoxError::from("connection refused"));

        assert_eq!(
            describe_error(&error),
            "could not connect: connection refused"
        );
    }
}
