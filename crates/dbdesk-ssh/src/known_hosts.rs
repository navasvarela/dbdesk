//! OpenSSH's `known_hosts` file: which host keys the user trusts, read the way `ssh` reads it,
//! and appended to when the user trusts a new one.
//!
//! russh has a reader too, but it splits fields on single spaces, fails on any entry whose key
//! type it can't parse, and ignores `@revoked`; this one follows `sshd(8)`'s description of the
//! format instead (patterns, negation, hashed names and markers), skipping what it can't read.

use std::io::{self, Write};
use std::path::Path;

use hmac::{Hmac, KeyInit, Mac};
use russh::keys::{Algorithm, PublicKey};
use sha1::Sha1;

/// What `known_hosts` says about one server: the keys it trusts, and the ones it revokes.
#[derive(Debug, Default)]
pub(crate) struct KnownHosts {
    trusted: Vec<Entry>,
    revoked: Vec<Entry>,
}

#[derive(Debug)]
struct Entry {
    /// From 1, as editors number lines.
    line: usize,
    key: PublicKey,
}

/// Whether a host key may be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    Trusted,
    /// No key of its type is trusted for the server yet.
    Unknown,
    /// Another key of the same type is trusted for the server, on this line.
    Changed {
        line: usize,
    },
    /// The key is marked `@revoked`, on this line.
    Revoked {
        line: usize,
    },
}

impl KnownHosts {
    /// The entries of `path` for `host` on `port`. A missing file trusts nothing.
    pub(crate) fn read(path: &Path, host: &str, port: u16) -> io::Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error),
        };
        Ok(Self::parse(&text, &host_name(host, port)))
    }

    fn parse(text: &str, name: &str) -> Self {
        let mut known = Self::default();
        for (index, line) in text.lines().enumerate() {
            let Some((marker, entry)) = parse_line(line, index + 1, name) else {
                continue;
            };
            match marker {
                Marker::None => known.trusted.push(entry),
                Marker::Revoked => known.revoked.push(entry),
            }
        }
        known
    }

    pub(crate) fn check(&self, key: &PublicKey) -> Verdict {
        if let Some(revoked) = self.revoked.iter().find(|entry| same_key(&entry.key, key)) {
            return Verdict::Revoked { line: revoked.line };
        }
        if self.trusted.iter().any(|entry| same_key(&entry.key, key)) {
            return Verdict::Trusted;
        }
        match self
            .trusted
            .iter()
            .find(|entry| entry.key.algorithm() == key.algorithm())
        {
            Some(other) => Verdict::Changed { line: other.line },
            None => Verdict::Unknown,
        }
    }

    /// The types of the keys trusted for the server, so the handshake can ask for one of
    /// them rather than a type that would be unknown.
    pub(crate) fn trusted_algorithms(&self) -> Vec<Algorithm> {
        let mut algorithms: Vec<Algorithm> = Vec::new();
        for entry in &self.trusted {
            let algorithm = entry.key.algorithm();
            if !algorithms.contains(&algorithm) {
                algorithms.push(algorithm);
            }
        }
        algorithms
    }
}

/// Adds a line trusting `key` for `host` on `port`, creating the file (and its folder) if
/// needed.
pub(crate) fn append(path: &Path, host: &str, port: u16, key: &PublicKey) -> io::Result<()> {
    if let Some(folder) = path.parent() {
        create_private_folder(folder)?;
    }
    let existing = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error),
    };
    let key = key
        .to_openssh()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    if existing.last().is_some_and(|last| *last != b'\n') {
        file.write_all(b"\n")?;
    }
    writeln!(file, "{} {key}", host_name(host, port))
}

/// `~/.ssh` is private, as `ssh` creates it.
fn create_private_folder(folder: &Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(folder)
}

/// How `known_hosts` names a server: its host, or `[host]:port` when not on port 22.
fn host_name(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_owned()
    } else {
        format!("[{host}]:{port}")
    }
}

enum Marker {
    None,
    Revoked,
}

/// The line's entry, if it is one for `name` with a key this crate can read.
fn parse_line(line: &str, number: usize, name: &str) -> Option<(Marker, Entry)> {
    let mut fields = line.split_whitespace();
    let mut first = fields.next()?;
    let marker = match first {
        _ if first.starts_with('#') => return None,
        "@revoked" => Marker::Revoked,
        // Host certificates are never asked for, so their authorities never apply.
        "@cert-authority" => return None,
        _ => Marker::None,
    };
    if !matches!(marker, Marker::None) {
        first = fields.next()?;
    }
    if !matches_patterns(first, name) {
        return None;
    }
    let key_type = fields.next()?;
    let key_data = fields.next()?;
    let key = PublicKey::from_openssh(&format!("{key_type} {key_data}")).ok()?;
    Some((marker, Entry { line: number, key }))
}

/// Whether `name` matches a comma-separated list of patterns: one of them matches it and no
/// negated one (`!pattern`) does.
fn matches_patterns(patterns: &str, name: &str) -> bool {
    let mut matched = false;
    for pattern in patterns.split(',') {
        match pattern.strip_prefix('!') {
            Some(negated) if matches_pattern(negated, name) => return false,
            Some(_) => {}
            None => matched |= matches_pattern(pattern, name),
        }
    }
    matched
}

fn matches_pattern(pattern: &str, name: &str) -> bool {
    match pattern.strip_prefix("|1|") {
        Some(hashed) => matches_hashed(hashed, name),
        None => matches_wildcard(pattern.as_bytes(), name.as_bytes()),
    }
}

/// A name hashed as `HashKnownHosts` writes it: `|1|salt|hash`, both in base64, the hash
/// being HMAC-SHA1 of the name keyed with the salt.
fn matches_hashed(hashed: &str, name: &str) -> bool {
    let Some((salt, hash)) = hashed.split_once('|') else {
        return false;
    };
    let decode = |text: &str| data_encoding::BASE64.decode(text.as_bytes()).ok();
    let (Some(salt), Some(hash)) = (decode(salt), decode(hash)) else {
        return false;
    };
    Hmac::<Sha1>::new_from_slice(&salt).is_ok_and(|mac| {
        mac.chain_update(name.as_bytes())
            .verify_slice(&hash)
            .is_ok()
    })
}

/// `*` matches any run of characters and `?` any one, as in `ssh_config(5)`'s patterns. Host
/// names are compared without regard to case.
fn matches_wildcard(pattern: &[u8], name: &[u8]) -> bool {
    match (pattern.split_first(), name.split_first()) {
        (None, None) => true,
        (Some((b'*', rest)), _) => {
            matches_wildcard(rest, name)
                || (!name.is_empty() && matches_wildcard(pattern, &name[1..]))
        }
        (Some((b'?', rest)), Some((_, name_rest))) => matches_wildcard(rest, name_rest),
        (Some((p, rest)), Some((n, name_rest))) if p.eq_ignore_ascii_case(n) => {
            matches_wildcard(rest, name_rest)
        }
        _ => false,
    }
}

/// The same key, comparing the key itself and not its comment.
fn same_key(a: &PublicKey, b: &PublicKey) -> bool {
    a.key_data() == b.key_data()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// docker/ssh's host key, and its first test client's key, which stands in for another
    /// server's.
    const HOST_KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOpXlsL9UGohFcHVtGiaA39aqjh2STGCGiAy2CKaGX9C";
    /// `HOST_KEY` for `db.example.com`, as `ssh-keygen -H` hashed it.
    const HASHED: &str = "|1|BtuMsmXjll5talTHVjEhUi2zxBI=|kngyWiWXBWfzezQpljZdNv8NPO8=";

    fn host_key() -> PublicKey {
        PublicKey::from_openssh(HOST_KEY).unwrap()
    }

    fn other_key() -> PublicKey {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docker/ssh/authorized_keys"
        ))
        .unwrap();
        PublicKey::from_openssh(text.lines().next().unwrap()).unwrap()
    }

    fn other_line() -> String {
        other_key().to_openssh().unwrap()
    }

    fn verdict(text: &str, name: &str) -> Verdict {
        KnownHosts::parse(text, name).check(&host_key())
    }

    #[test]
    fn matches_names_the_way_ssh_does() {
        let cases = [
            ("db.example.com", "db.example.com", true),
            ("DB.Example.com", "db.example.com", true),
            ("other.example.com", "db.example.com", false),
            ("*.example.com", "db.example.com", true),
            ("db?.example.com", "db1.example.com", true),
            ("db?.example.com", "db.example.com", false),
            ("web,db.example.com", "db.example.com", true),
            ("*.example.com,!db.example.com", "db.example.com", false),
            ("[db.example.com]:2222", "[db.example.com]:2222", true),
            ("db.example.com", "[db.example.com]:2222", false),
            (HASHED, "db.example.com", true),
            (HASHED, "web.example.com", false),
        ];
        for (patterns, name, expected) in cases {
            assert_eq!(
                matches_patterns(patterns, name),
                expected,
                "{patterns} {name}"
            );
        }
    }

    #[test]
    fn trusts_refuses_or_asks_about_a_key() {
        let other = other_line();
        let cases = [
            (format!("db.example.com {HOST_KEY}"), Verdict::Trusted),
            (
                format!("db.example.com\t{HOST_KEY} a comment"),
                Verdict::Trusted,
            ),
            (format!("{HASHED} {HOST_KEY}"), Verdict::Trusted),
            (String::new(), Verdict::Unknown),
            (format!("web.example.com {HOST_KEY}"), Verdict::Unknown),
            (format!("# db.example.com {HOST_KEY}"), Verdict::Unknown),
            (
                format!("web.example.com {HOST_KEY}\ndb.example.com {other}"),
                Verdict::Changed { line: 2 },
            ),
            (
                format!("db.example.com {HOST_KEY}\n@revoked * {HOST_KEY}"),
                Verdict::Revoked { line: 2 },
            ),
            (
                format!("@cert-authority *.example.com {other}"),
                Verdict::Unknown,
            ),
            (
                format!("db.example.com ssh-unheard-of AAAA\ndb.example.com {HOST_KEY}"),
                Verdict::Trusted,
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(verdict(&text, "db.example.com"), expected, "{text}");
        }
    }

    #[test]
    fn lists_the_types_of_the_trusted_keys() {
        let text = format!("db.example.com {HOST_KEY}\ndb.example.com {}", other_line());

        let known = KnownHosts::parse(&text, "db.example.com");

        assert_eq!(known.trusted_algorithms(), vec![Algorithm::Ed25519]);
        assert!(KnownHosts::default().trusted_algorithms().is_empty());
    }

    #[test]
    fn appends_a_trusted_key_on_a_line_of_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".ssh/known_hosts");

        append(&path, "db.example.com", 2222, &host_key()).unwrap();
        std::fs::write(&path, std::fs::read_to_string(&path).unwrap().trim_end()).unwrap();
        append(&path, "web.example.com", 22, &other_key()).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            text,
            format!(
                "[db.example.com]:2222 {HOST_KEY}\nweb.example.com {}\n",
                other_line()
            )
        );
        let read = KnownHosts::read(&path, "db.example.com", 2222).unwrap();
        assert_eq!(read.check(&host_key()), Verdict::Trusted);
    }

    #[test]
    fn a_missing_file_trusts_nothing() {
        let dir = tempfile::tempdir().unwrap();

        let known = KnownHosts::read(&dir.path().join("known_hosts"), "db", 22).unwrap();

        assert_eq!(known.check(&host_key()), Verdict::Unknown);
    }
}
