use anyhow::Result;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use zed_interfaces::vcs::Vcs;

use crate::config::TagPolicy;

/// Tags go into a URL path segment: encode everything but RFC 3986
/// unreserved characters so `/`, `?`, `#`, … cannot alter the request.
const TAG_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

#[derive(Debug, PartialEq)]
pub enum TagCheck {
    /// Verification disabled or host not verifiable; publish proceeds.
    Skipped,
    Verified {
        commit: Option<String>,
    },
    Missing,
}

/// Server-side provenance enforcement: the declared backing repo must carry
/// the tag the client claims to have published from. GitHub is implemented;
/// GitLab/Bitbucket/Codeberg/SourceHut/hg hosts are the documented extension
/// point (add a variant + fetch here), and unverifiable hosts are allowed
/// through with a warning so self-hosted forges are not locked out.
pub struct TagVerifier {
    policy: TagPolicy,
    client: reqwest::Client,
    github_token: Option<String>,
}

impl TagVerifier {
    pub fn new(policy: TagPolicy) -> Self {
        Self {
            policy,
            client: reqwest::Client::builder()
                .user_agent(concat!("zed-api-server/", env!("CARGO_PKG_VERSION")))
                .build()
                .expect("reqwest client builds"),
            github_token: std::env::var("GITHUB_TOKEN").ok(),
        }
    }

    pub async fn verify(&self, vcs: Vcs, repo_url: &str, tag: &str) -> Result<TagCheck> {
        if self.policy == TagPolicy::Off {
            return Ok(TagCheck::Skipped);
        }
        if !vcs.uses_git_tags() {
            tracing::warn!(%vcs, repo_url, "tag verification unsupported for this vcs; allowing");
            return Ok(TagCheck::Skipped);
        }
        let Some((owner, repo)) = parse_github(repo_url) else {
            tracing::warn!(
                repo_url,
                "tag verification only implemented for github.com; allowing"
            );
            return Ok(TagCheck::Skipped);
        };
        let tag = utf8_percent_encode(tag, TAG_ENCODE_SET);
        let url = format!("https://api.github.com/repos/{owner}/{repo}/git/ref/tags/{tag}");
        let mut request = self.client.get(&url);
        if let Some(token) = &self.github_token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(TagCheck::Missing);
        }
        let body: serde_json::Value = response.error_for_status()?.json().await?;
        let commit = match body["object"]["type"].as_str() {
            Some("commit") => body["object"]["sha"].as_str().map(|s| s.to_string()),
            _ => None, // annotated tag object; commit resolution left to clients
        };
        Ok(TagCheck::Verified { commit })
    }
}

/// `https://github.com/{owner}/{repo}[.git]` -> (owner, repo)
pub fn parse_github(repo_url: &str) -> Option<(String, String)> {
    let rest = repo_url
        .strip_prefix("https://github.com/")
        .or_else(|| repo_url.strip_prefix("http://github.com/"))
        .or_else(|| repo_url.strip_prefix("git@github.com:"))?;
    let rest = rest.trim_end_matches('/').trim_end_matches(".git");
    let mut parts = rest.splitn(2, '/');
    match (parts.next(), parts.next()) {
        (Some(owner), Some(repo))
            if !owner.is_empty() && !repo.is_empty() && !repo.contains('/') =>
        {
            Some((owner.to_string(), repo.to_string()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_url_parsing() {
        assert_eq!(
            parse_github("https://github.com/acme/http-kit"),
            Some(("acme".into(), "http-kit".into()))
        );
        assert_eq!(
            parse_github("https://github.com/acme/http-kit.git"),
            Some(("acme".into(), "http-kit".into()))
        );
        assert_eq!(
            parse_github("git@github.com:acme/http-kit.git"),
            Some(("acme".into(), "http-kit".into()))
        );
        assert_eq!(parse_github("https://gitlab.com/acme/x"), None);
        assert_eq!(parse_github("https://github.com/acme"), None);
    }
}
