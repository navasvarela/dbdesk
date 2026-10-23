//! Where to learn about new dbdesk releases: the newest published GitHub release.

use std::sync::Arc;
use std::time::Duration;

use dbdesk_core::describe_error;
use serde::Deserialize;

use crate::Event;
use crate::events::EventSink;

/// How long a check may take before it is given up. Long enough for a slow network, short
/// enough that a firewall that swallows the request doesn't leave a check hanging.
const TIMEOUT: Duration = Duration::from_secs(15);

/// A published release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// The release's tag, e.g. `v0.2.0`.
    pub tag: String,
    /// The release's page, with its notes and downloads.
    pub page: String,
}

/// Tells the engine about the newest release, so the update check can be tested without a
/// network.
pub trait ReleaseFeed: Send + Sync {
    /// The newest published release. Blocking: the engine calls it on its blocking pool.
    ///
    /// # Errors
    ///
    /// When the feed can't be reached or read, or nothing has been released yet.
    fn latest(&self) -> Result<Release, ReleaseError>;
}

#[derive(Debug, thiserror::Error)]
pub enum ReleaseError {
    #[error("no release has been published yet")]
    NoReleases,
    #[error("could not reach GitHub")]
    Request(#[source] ureq::Error),
    #[error("GitHub's answer didn't describe a release")]
    Parse(#[source] serde_json::Error),
}

/// The newest non-draft, non-prerelease release of a GitHub repository, from GitHub's REST
/// API. Unauthenticated, which GitHub allows 60 times an hour per address: plenty for one
/// check per launch.
pub struct GithubReleases {
    url: String,
    agent: ureq::Agent,
}

impl GithubReleases {
    /// `repository` is `owner/name`; `user_agent` names the app, which GitHub requires.
    pub fn new(repository: &str, user_agent: &str) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .user_agent(user_agent)
            .build()
            .into();
        Self {
            url: format!("https://api.github.com/repos/{repository}/releases/latest"),
            agent,
        }
    }
}

impl ReleaseFeed for GithubReleases {
    fn latest(&self) -> Result<Release, ReleaseError> {
        let response = self
            .agent
            .get(&self.url)
            .header("Accept", "application/vnd.github+json")
            .call();
        let body = match response {
            Ok(mut response) => response
                .body_mut()
                .read_to_string()
                .map_err(ReleaseError::Request)?,
            // GitHub's answer when a repository has no published release.
            Err(ureq::Error::StatusCode(404)) => return Err(ReleaseError::NoReleases),
            Err(error) => return Err(ReleaseError::Request(error)),
        };
        parse_release(&body).map_err(ReleaseError::Parse)
    }
}

/// The two fields dbdesk needs from GitHub's release object.
#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
}

fn parse_release(body: &str) -> Result<Release, serde_json::Error> {
    let release: GithubRelease = serde_json::from_str(body)?;
    Ok(Release {
        tag: release.tag_name,
        page: release.html_url,
    })
}

/// Asks the feed on the blocking pool and reports the answer as [`Event::LatestRelease`].
pub(crate) async fn check(feed: Arc<dyn ReleaseFeed>, events: EventSink) {
    let result = match tokio::task::spawn_blocking(move || feed.latest()).await {
        Ok(Ok(release)) => Ok(release),
        Ok(Err(error)) => Err(describe_error(&error)),
        Err(error) => Err(describe_error(&error)),
    };
    match &result {
        Ok(release) => tracing::info!("the newest release is {}", release.tag),
        Err(error) => tracing::warn!("could not check for updates: {error}"),
    }
    events.send(Event::LatestRelease(result));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_tag_and_page_and_ignores_the_rest() {
        let body = r#"{
            "tag_name": "v0.2.0",
            "html_url": "https://github.com/navasvarela/dbdesk/releases/tag/v0.2.0",
            "name": "dbdesk 0.2.0",
            "draft": false,
            "assets": []
        }"#;
        assert_eq!(
            parse_release(body).ok(),
            Some(Release {
                tag: "v0.2.0".to_owned(),
                page: "https://github.com/navasvarela/dbdesk/releases/tag/v0.2.0".to_owned(),
            })
        );
        assert!(parse_release(r#"{"message": "API rate limit exceeded"}"#).is_err());
    }
}
