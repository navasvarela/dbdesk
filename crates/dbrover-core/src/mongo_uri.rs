//! What the data source form reads from a pasted MongoDB connection string: its user and
//! password, which DbRover keeps out of the string, the host that names the data source, and
//! the database it starts in. The driver reads the rest of the string itself.

/// A connection string taken apart: `mongodb://user:password@host1,host2/database?options`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MongoUri {
    /// The user, percent-decoded; empty when the string names none.
    pub user: String,
    /// The password, percent-decoded, when the string has one.
    pub password: Option<String>,
    /// The first host, without its port: what names the data source.
    pub host: String,
    /// The database in the path, when there is one.
    pub database: Option<String>,
    /// The string without its user and password.
    pub without_credentials: String,
}

impl MongoUri {
    pub const SCHEMES: [&'static str; 2] = ["mongodb://", "mongodb+srv://"];

    /// Takes `text` apart, or `None` when it isn't a MongoDB connection string.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let scheme = Self::SCHEMES
            .into_iter()
            .find(|scheme| text.starts_with(scheme))?;
        let rest = &text[scheme.len()..];
        let authority_end = rest.find(['/', '?']).unwrap_or(rest.len());
        let (authority, after) = rest.split_at(authority_end);
        let (credentials, hosts) = match authority.rfind('@') {
            Some(at) => (Some(&authority[..at]), &authority[at + 1..]),
            None => (None, authority),
        };
        let (user, password) = match credentials {
            Some(credentials) => match credentials.split_once(':') {
                Some((user, password)) => (decode(user)?, Some(decode(password)?)),
                None => (decode(credentials)?, None),
            },
            None => (String::new(), None),
        };
        let first = hosts.split(',').next().unwrap_or_default();
        let host = match first.strip_prefix('[') {
            Some(bracketed) => bracketed.split(']').next().unwrap_or_default(),
            None => first.split(':').next().unwrap_or_default(),
        };
        let path = after
            .strip_prefix('/')
            .map(|path| path.split('?').next().unwrap_or_default())
            .filter(|database| !database.is_empty());
        let database = match path {
            Some(path) => Some(decode(path)?),
            None => None,
        };
        Some(Self {
            user,
            password,
            host: host.to_owned(),
            database,
            without_credentials: format!("{scheme}{hosts}{after}"),
        })
    }
}

/// `text` with its `%XX` escapes decoded; `None` for an escape that isn't one, or bytes that
/// aren't UTF-8.
fn decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = text.get(index + 1..index + 3)?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_the_credentials_out_of_an_atlas_string() {
        let uri = MongoUri::parse(
            " mongodb+srv://app%40shop:p%3Ass@cluster0.ab1cd.mongodb.net/orders?retryWrites=true ",
        );
        assert_eq!(
            uri,
            Some(MongoUri {
                user: "app@shop".to_owned(),
                password: Some("p:ss".to_owned()),
                host: "cluster0.ab1cd.mongodb.net".to_owned(),
                database: Some("orders".to_owned()),
                without_credentials:
                    "mongodb+srv://cluster0.ab1cd.mongodb.net/orders?retryWrites=true".to_owned(),
            })
        );
    }

    #[test]
    fn reads_several_hosts_a_bare_string_and_ipv6() {
        let replica = MongoUri::parse("mongodb://a:27017,b:27018/?replicaSet=rs0");
        assert_eq!(
            replica
                .as_ref()
                .map(|uri| (uri.host.as_str(), uri.database.as_deref())),
            Some(("a", None))
        );
        assert_eq!(
            replica.map(|uri| uri.without_credentials),
            Some("mongodb://a:27017,b:27018/?replicaSet=rs0".to_owned())
        );
        let bare = MongoUri::parse("mongodb://localhost");
        assert_eq!(
            bare.map(|uri| (uri.user, uri.password, uri.host)),
            Some((String::new(), None, "localhost".to_owned()))
        );
        let ipv6 = MongoUri::parse("mongodb://[::1]:27017/admin");
        assert_eq!(ipv6.map(|uri| uri.host), Some("::1".to_owned()));
    }

    #[test]
    fn refuses_what_is_not_a_connection_string() {
        assert_eq!(MongoUri::parse("localhost:27017"), None);
        assert_eq!(MongoUri::parse("mongodb://bad%zzescape@host"), None);
    }
}
