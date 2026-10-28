use std::fmt;

/// A database password.
///
/// `Debug` never prints the value, so a password can't leak into logs by being part of a
/// `{:?}`-formatted struct. Read it with [`Password::expose`] only where a driver needs it.
#[derive(Clone, PartialEq, Eq)]
pub struct Password(String);

impl Password {
    pub fn new(password: impl Into<String>) -> Self {
        Self(password.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Password(***)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_hides_the_value() {
        assert_eq!(format!("{:?}", Password::new("hunter2")), "Password(***)");
    }
}
