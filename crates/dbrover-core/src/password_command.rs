//! A program whose standard output is a data source's password (W9), for short-lived tokens
//! and secret stores: `["aws", "rds", "generate-db-auth-token", "--hostname", "{host}", ...]`.
//!
//! It is a [`CommandTemplate`] whose placeholders can only be the server's own settings: there
//! is no object or selection when connecting, so `{table}` or `{value}` would never be filled.

use serde::{Deserialize, Serialize};

use crate::{
    CommandTemplate, DataSourceFields, Placeholder, RenderError, TemplateError, TemplateValues,
};

/// The command and its arguments, as `data_sources.toml` writes them:
/// `password_command = ["op", "read", "op://Work/{host}/password"]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Vec<String>", into = "Vec<String>")]
pub struct PasswordCommand(CommandTemplate);

impl PasswordCommand {
    /// Reads a command: the program, then its arguments.
    ///
    /// # Errors
    ///
    /// What [`CommandTemplate::parse`] refuses, and [`TemplateError::NotForPasswords`] for a
    /// placeholder other than `{host}`, `{port}`, `{user}` and `{database}`.
    pub fn parse(command: &[String]) -> Result<Self, TemplateError> {
        let template = CommandTemplate::parse(command)?;
        let refused = template
            .placeholders()
            .find(|placeholder| !fills_when_connecting(placeholder));
        match refused {
            Some(placeholder) => Err(TemplateError::NotForPasswords(placeholder.clone())),
            None => Ok(Self(template)),
        }
    }

    /// The command as written, which is what the user is asked to trust.
    pub fn source(&self) -> &[String] {
        self.0.source()
    }

    pub fn program(&self) -> &str {
        self.0.program()
    }

    /// The arguments after the program, filled in from the data source's settings.
    ///
    /// # Errors
    ///
    /// [`RenderError`] for a placeholder the settings don't fill, such as `{port}` when the
    /// port is still a `${NAME}` variable.
    pub fn render(&self, fields: &DataSourceFields) -> Result<Vec<String>, RenderError> {
        self.0.render(&TemplateValues::default(), fields)
    }
}

fn fills_when_connecting(placeholder: &Placeholder) -> bool {
    matches!(
        placeholder,
        Placeholder::Host | Placeholder::Port | Placeholder::User | Placeholder::Database
    )
}

impl TryFrom<Vec<String>> for PasswordCommand {
    type Error = TemplateError;

    fn try_from(command: Vec<String>) -> Result<Self, Self::Error> {
        Self::parse(&command)
    }
}

impl From<PasswordCommand> for Vec<String> {
    fn from(command: PasswordCommand) -> Self {
        command.source().to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(items: &[&str]) -> Result<PasswordCommand, TemplateError> {
        let items: Vec<String> = items.iter().map(|&item| item.to_owned()).collect();
        PasswordCommand::parse(&items)
    }

    #[test]
    fn fills_in_the_servers_settings() {
        let command = parse(&[
            "aws",
            "rds",
            "generate-db-auth-token",
            "--hostname",
            "{host}",
            "--port",
            "{port}",
            "--username",
            "{user}",
            "--database={database}",
        ])
        .unwrap();
        let fields = DataSourceFields {
            host: Some("db.example.com".to_owned()),
            port: Some(5432),
            user: Some("ada".to_owned()),
            database: Some("shop".to_owned()),
            file: None,
        };

        assert_eq!(command.program(), "aws");
        assert_eq!(
            command.render(&fields).unwrap(),
            [
                "rds",
                "generate-db-auth-token",
                "--hostname",
                "db.example.com",
                "--port",
                "5432",
                "--username",
                "ada",
                "--database=shop",
            ]
        );
    }

    #[test]
    fn refuses_what_connecting_cant_fill() {
        let cases: [(&[&str], &str); 4] = [
            (
                &["vault", "{table}"],
                "{table} can't be in a password command, which can only have {host}, {port}, \
                 {user} and {database}",
            ),
            (
                &["op", "read", "{file}"],
                "{file} can't be in a password command, which can only have {host}, {port}, \
                 {user} and {database}",
            ),
            (&["op", "{tabel}"], "{tabel} is not a placeholder"),
            (
                &[],
                "the command is empty; its first item is the program to run",
            ),
        ];
        for (items, message) in cases {
            assert_eq!(parse(items).unwrap_err().to_string(), message, "{items:?}");
        }
    }

    #[test]
    fn is_written_as_the_list_it_was_read_from() {
        #[derive(Debug, PartialEq, Serialize, Deserialize)]
        struct Holder {
            password_command: PasswordCommand,
        }
        let text = "password_command = [\"printf\", \"{user}\"]\n";
        let holder: Holder = toml::from_str(text).unwrap();
        assert_eq!(holder.password_command.source(), ["printf", "{user}"]);
        assert_eq!(toml::to_string(&holder).unwrap(), text);

        let error =
            toml::from_str::<Holder>("password_command = [\"printf\", \"{value}\"]\n").unwrap_err();
        assert!(error.message().contains("{value} can't be"), "{error}");
    }
}
