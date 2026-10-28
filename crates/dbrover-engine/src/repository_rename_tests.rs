//! Copies of the app released before the repository was renamed ask its old address for
//! releases and updates, and GitHub redirects them (see "Updates across the rename" in
//! `docs/releasing.md`). These tests check that both HTTP clients those copies use follow the
//! redirects, against a stand-in for GitHub on this machine: the release notice's
//! [`GithubReleases`], and `cargo-packager-updater`'s check and download, signature included.

use std::collections::HashMap;
use std::io::{BufRead as _, BufReader, Write as _};
use std::net::{TcpListener, TcpStream};

use cargo_packager_updater::semver::Version;
use cargo_packager_updater::url::Url;
use cargo_packager_updater::{Config, UpdaterBuilder};

use crate::releases::{GithubReleases, Release, ReleaseFeed as _};

/// The public half of a key made for these tests with `cargo packager signer generate`, and
/// the signature `cargo packager signer sign` made of [`PACKAGE`] with its private half, which
/// was thrown away. Not the release key.
const TEST_PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDFFRTgyRTBEOTdDMzhGNUIKUldSYmo4T1hEUzdvSGdzUjUvdlRZM0FpWFlmZU5ncmR1aUU0SysxMXpaTUNSbXpTb3VJa2hXUlAK";
const PACKAGE_SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIGNhcmdvLXBhY2thZ2VyIHNlY3JldCBrZXkKUlVSYmo4T1hEUzdvSHFmZElPYmdObTVPVHNWM1BkWE16dDNjcDVPaktxQ01ZQ01SOWdKbmNYL05nQWxEMkhKRjFwbjUxMStFaVZRS25vT3pVdUhSTHVvSjRHSFVpQWxFSUFjPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkzMjA1ODYxCWZpbGU6cGFja2FnZS5iaW4KQ3RhcmhORWs5SXVENmdsTUxWOWNzY0tzRTJOQWl4K1pDUkVGQmxNeWNlVTJGd2FJTDdOOURNdklydTJYc0s2cXhWdW9wMVRHTno0TFdJZE1kdGdmRFE9PQo=";
const PACKAGE: &[u8] = b"Not a real update package: a fixture for the redirect tests.\n";

/// What the stand-in answers for a path.
enum Answer {
    /// A redirect with this status line, to this path on the stand-in.
    Redirect(&'static str, &'static str),
    Body(Vec<u8>),
}

/// A stand-in for GitHub: answers each request from a fixed table, one connection at a time,
/// until the test process ends.
struct StandIn {
    listener: TcpListener,
}

impl StandIn {
    fn bind() -> Self {
        Self {
            listener: TcpListener::bind("127.0.0.1:0").unwrap(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.listener.local_addr().unwrap())
    }

    fn serve(self, answers: HashMap<&'static str, Answer>) {
        let base = self.url("");
        std::thread::spawn(move || {
            for stream in self.listener.incoming().flatten() {
                answer(stream, &answers, &base);
            }
        });
    }
}

fn answer(mut stream: TcpStream, answers: &HashMap<&'static str, Answer>, base: &str) {
    let mut request = BufReader::new(&stream);
    let mut request_line = String::new();
    request.read_line(&mut request_line).unwrap();
    // The headers aren't needed, but must be read before answering.
    let mut header = String::new();
    while request.read_line(&mut header).unwrap_or(0) > 2 {
        header.clear();
    }
    let path = request_line.split_whitespace().nth(1).unwrap_or_default();
    let response = match answers.get(path) {
        Some(Answer::Redirect(status, to)) => format!(
            "HTTP/1.1 {status}\r\nLocation: {base}{to}\r\nContent-Length: 0\r\n\
             Connection: close\r\n\r\n"
        )
        .into_bytes(),
        Some(Answer::Body(body)) => {
            let mut response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .into_bytes();
            response.extend_from_slice(body);
            response
        }
        None => {
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        }
    };
    stream.write_all(&response).unwrap();
}

#[test]
fn the_release_notice_follows_the_api_redirect() {
    let github = StandIn::bind();
    let old_address = github.url("/repos/navasvarela/dbdesk/releases/latest");
    github.serve(HashMap::from([
        (
            "/repos/navasvarela/dbdesk/releases/latest",
            Answer::Redirect("301 Moved Permanently", "/repositories/42/releases/latest"),
        ),
        (
            "/repositories/42/releases/latest",
            Answer::Body(
                br#"{"tag_name": "v0.13.0", "html_url": "https://example.com/v0.13.0"}"#.to_vec(),
            ),
        ),
    ]));

    let latest = GithubReleases::at(old_address, "test").latest().unwrap();

    assert_eq!(
        latest,
        Release {
            tag: "v0.13.0".to_owned(),
            page: "https://example.com/v0.13.0".to_owned(),
        }
    );
}

#[test]
fn the_updater_follows_the_redirects_to_the_manifest_and_the_package() {
    for (served, verifies) in [(PACKAGE.to_vec(), true), (b"tampered".to_vec(), false)] {
        let github = StandIn::bind();
        let old_manifest = github.url("/navasvarela/dbdesk/releases/latest/download/latest.json");
        let old_package = github.url("/navasvarela/dbdesk/releases/download/v0.13.0/package");
        let platform = serde_json::json!({
            "url": old_package,
            "signature": PACKAGE_SIGNATURE,
            "format": "app",
        });
        let platforms: serde_json::Map<_, _> = [
            "macos-aarch64",
            "macos-x86_64",
            "linux-x86_64",
            "linux-aarch64",
            "windows-x86_64",
            "windows-aarch64",
        ]
        .into_iter()
        .map(|target| (target.to_owned(), platform.clone()))
        .collect();
        let manifest = serde_json::json!({
            "version": "0.13.0",
            "pub_date": "2026-10-28T00:00:00Z",
            "platforms": platforms,
        });
        github.serve(HashMap::from([
            (
                "/navasvarela/dbdesk/releases/latest/download/latest.json",
                Answer::Redirect(
                    "301 Moved Permanently",
                    "/navasvarela/dbrover/releases/latest/download/latest.json",
                ),
            ),
            (
                "/navasvarela/dbrover/releases/latest/download/latest.json",
                Answer::Redirect("302 Found", "/assets/latest.json"),
            ),
            (
                "/assets/latest.json",
                Answer::Body(manifest.to_string().into_bytes()),
            ),
            (
                "/navasvarela/dbdesk/releases/download/v0.13.0/package",
                Answer::Redirect(
                    "301 Moved Permanently",
                    "/navasvarela/dbrover/releases/download/v0.13.0/package",
                ),
            ),
            (
                "/navasvarela/dbrover/releases/download/v0.13.0/package",
                Answer::Redirect("302 Found", "/assets/package"),
            ),
            ("/assets/package", Answer::Body(served)),
        ]));
        let config = Config {
            endpoints: vec![Url::parse(&old_manifest).unwrap()],
            pubkey: TEST_PUBLIC_KEY.to_owned(),
            windows: None,
        };

        let update = UpdaterBuilder::new(Version::new(0, 12, 0), config)
            .build()
            .unwrap()
            .check()
            .unwrap()
            .expect("0.13.0 is newer");
        let downloaded = update.download();

        assert_eq!(update.version, "0.13.0");
        if verifies {
            assert_eq!(downloaded.unwrap(), PACKAGE);
        } else {
            assert!(downloaded.is_err(), "a package with another signature");
        }
    }
}
