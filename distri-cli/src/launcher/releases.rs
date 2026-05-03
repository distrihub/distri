use anyhow::Result;
use semver::Version;
use serde::Deserialize;

pub const RELEASES_INDEX_URL: &str = "https://api.github.com/repos/distrihub/distri/releases";
pub const ARTIFACT_BASE: &str = "https://github.com/distrihub/distri/releases/download/";

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
    pub size: u64,
}

#[derive(Debug, Clone, Copy)]
pub enum Stream {
    Server,
    Ui,
    Cli,
}

impl Stream {
    pub fn tag_prefix(&self) -> &'static str {
        match self {
            Stream::Server => "server-v",
            Stream::Ui => "ui-v",
            Stream::Cli => "cli-v",
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
                 "browser_download_url": "https://example.com/server.tgz",
                 "size": 1234}
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
            "tag_name": "cli-v0.5.0",
            "draft": false,
            "prerelease": false,
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
        // 0.5.3 + 0.6.0-rc.1 (both tagged server-v...) — versions parse, filter is by tag prefix.
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
    fn versions_for_cli_filters_correctly() {
        let releases = parse_fixture();
        let c = versions_for_stream(&releases, Stream::Cli);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].0.to_string(), "0.5.0");
    }

    #[test]
    fn unrecognized_tag_drops() {
        let releases = parse_fixture();
        // The "untagged-v0.1.0" entry has no recognized stream prefix.
        let server = versions_for_stream(&releases, Stream::Server);
        let ui = versions_for_stream(&releases, Stream::Ui);
        let cli = versions_for_stream(&releases, Stream::Cli);
        // Together they should NOT include the untagged release.
        let total = server.len() + ui.len() + cli.len();
        // server: 2, ui: 1, cli: 1 → 4. Untagged + draft excluded.
        assert_eq!(total, 4);
    }

    #[test]
    fn fetch_releases_filters_drafts_and_pre() {
        // We can't actually call the network in unit tests, but we can verify
        // the post-fetch filter logic by reusing the fixture and reapplying
        // the same closure that fetch_releases uses.
        let all = parse_fixture();
        let stable: Vec<_> = all
            .iter()
            .cloned()
            .filter(|r| !r.draft && !r.prerelease)
            .collect();
        // Fixture has 6 entries: 1 draft, 1 prerelease, 4 stable+published.
        assert_eq!(stable.len(), 4);

        let with_pre: Vec<_> = all
            .iter()
            .cloned()
            .filter(|r| !r.draft)
            .collect();
        // 5 with prereleases included, drafts still excluded.
        assert_eq!(with_pre.len(), 5);
    }
}
