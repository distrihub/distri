use anyhow::Result;
use semver::Version;
use serde::Deserialize;

pub const RELEASES_INDEX_URL: &str = "https://api.github.com/repos/distrihub/distri/releases";

#[derive(Debug, Deserialize, Clone)]
pub struct GhRelease {
    pub tag_name: String,
    pub draft: bool,
    pub prerelease: bool,
    pub assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct GhAsset {
    pub name: String,
    pub browser_download_url: String,
}

#[derive(Debug, Clone, Copy)]
pub enum Stream {
    Server,
    Ui,
}

impl Stream {
    pub fn tag_prefix(&self) -> &'static str {
        match self {
            Stream::Server => "server-v",
            Stream::Ui => "ui-v",
        }
    }
}

pub async fn fetch_releases(http: &reqwest::Client, allow_pre: bool) -> Result<Vec<GhRelease>> {
    let resp = http
        .get(RELEASES_INDEX_URL)
        .header(
            "User-Agent",
            concat!("distri-cli/", env!("CARGO_PKG_VERSION")),
        )
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?;
    let releases: Vec<GhRelease> = resp.json().await?;
    Ok(releases
        .into_iter()
        .filter(|r| !r.draft && (allow_pre || !r.prerelease))
        .collect())
}

/// Filter `releases` to only those tagged for `stream` and parse the tag suffix
/// as a semver `Version`. Releases whose tag doesn't parse are dropped.
pub fn versions_for_stream<'a>(
    releases: &'a [GhRelease],
    stream: Stream,
) -> Vec<(Version, &'a GhRelease)> {
    releases
        .iter()
        .filter_map(|r| {
            r.tag_name
                .strip_prefix(stream.tag_prefix())
                .and_then(|s| Version::parse(s).ok())
                .map(|v| (v, r))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"[
        {
            "tag_name": "server-v0.5.3",
            "draft": false,
            "prerelease": false,
            "assets": [
                {"name": "distri-server-0.5.3-darwin-arm64.tar.gz",
                 "browser_download_url": "https://example.com/server.tgz"}
            ]
        },
        {
            "tag_name": "ui-v0.5.7",
            "draft": false,
            "prerelease": false,
            "assets": []
        },
        {
            "tag_name": "server-v0.6.0-rc.1",
            "draft": false,
            "prerelease": true,
            "assets": []
        },
        {
            "tag_name": "draft-v9.9.9",
            "draft": true,
            "prerelease": false,
            "assets": []
        },
        {
            "tag_name": "untagged-v0.1.0",
            "draft": false,
            "prerelease": false,
            "assets": []
        }
    ]"#;

    fn parse_fixture() -> Vec<GhRelease> {
        serde_json::from_str(FIXTURE).expect("fixture parses")
    }

    #[test]
    fn versions_for_server_filters_correctly() {
        let releases = parse_fixture();
        let s = versions_for_stream(&releases, Stream::Server);
        assert_eq!(s.len(), 2);
        let versions: Vec<String> = s.iter().map(|(v, _)| v.to_string()).collect();
        assert!(versions.contains(&"0.5.3".to_string()));
        assert!(versions.contains(&"0.6.0-rc.1".to_string()));
    }

    #[test]
    fn versions_for_ui_filters_correctly() {
        let releases = parse_fixture();
        let u = versions_for_stream(&releases, Stream::Ui);
        assert_eq!(u.len(), 1);
        assert_eq!(u[0].0.to_string(), "0.5.7");
    }

    #[test]
    fn unrecognized_tag_drops() {
        let releases = parse_fixture();
        let server = versions_for_stream(&releases, Stream::Server);
        let ui = versions_for_stream(&releases, Stream::Ui);
        // server: 2, ui: 1 → 3. Untagged + draft + cli (no longer a stream) excluded.
        assert_eq!(server.len() + ui.len(), 3);
    }

    #[test]
    fn fetch_releases_filters_drafts_and_pre() {
        let all = parse_fixture();
        let stable: Vec<_> = all
            .iter()
            .cloned()
            .filter(|r| !r.draft && !r.prerelease)
            .collect();
        // 5 entries: 1 draft, 1 prerelease, 3 stable+published.
        assert_eq!(stable.len(), 3);

        let with_pre: Vec<_> = all.iter().cloned().filter(|r| !r.draft).collect();
        assert_eq!(with_pre.len(), 4);
    }
}
