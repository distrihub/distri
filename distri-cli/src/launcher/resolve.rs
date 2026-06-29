use anyhow::{anyhow, Context, Result};
use semver::{Version, VersionReq};
use std::path::PathBuf;

use super::compat::{server_req, ui_req};
use super::download::download_verify_extract;
use super::platform::Platform;
use super::releases::{fetch_releases, versions_for_stream, GhRelease, Stream};
use crate::manifest::{self, EntryRecord, Manifest};

/// Options for resolving a server or UI artifact.
#[derive(Debug, Default, Clone)]
pub struct ResolveOpts {
    /// If set, resolve exactly this version (must still be in the released set).
    pub pinned_version: Option<Version>,
    /// Allow pre-release versions when picking newest within the compat range.
    pub allow_pre: bool,
    /// Skip the cache and re-resolve.
    pub force_refresh: bool,
}

pub async fn resolve_server(http: &reqwest::Client, opts: &ResolveOpts) -> Result<PathBuf> {
    resolve_artifact(http, Stream::Server, server_req(), opts).await
}

pub async fn resolve_ui(http: &reqwest::Client, opts: &ResolveOpts) -> Result<PathBuf> {
    resolve_artifact(http, Stream::Ui, ui_req(), opts).await
}

async fn resolve_artifact(
    http: &reqwest::Client,
    stream: Stream,
    req: VersionReq,
    opts: &ResolveOpts,
) -> Result<PathBuf> {
    let plat = Platform::current()?;
    let mut mf = manifest::read()?;

    // 1. Cache check (skip if force_refresh).
    if !opts.force_refresh {
        if let Some(rec) = current_record(&mf, stream) {
            if let Ok(v) = Version::parse(&rec.version) {
                let acceptable = match &opts.pinned_version {
                    Some(pin) => *pin == v,
                    None => req.matches(&v),
                };
                if acceptable && rec.path.exists() {
                    return Ok(rec.path.clone());
                }
            }
        }
    }

    // 2. Fetch release index, pick best version.
    let releases = fetch_releases(http, opts.allow_pre)
        .await
        .context("fetching release index")?;
    let candidates = versions_for_stream(&releases, stream);
    let pick = pick_release(&candidates, &req, opts.pinned_version.as_ref(), stream)?;

    // 3. Locate matching asset + sha256 sidecar.
    let want_asset = match stream {
        Stream::Server => plat.server_artifact(&pick.0.to_string()),
        Stream::Ui => Platform::ui_artifact(&pick.0.to_string()),
    };
    let want_sha = format!("{want_asset}.sha256");
    let asset = pick
        .1
        .assets
        .iter()
        .find(|a| a.name == want_asset)
        .ok_or_else(|| {
            anyhow!(
                "release {} has no asset {want_asset}; available: {}",
                pick.1.tag_name,
                pick.1
                    .assets
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            )
        })?;
    let sha_asset = pick
        .1
        .assets
        .iter()
        .find(|a| a.name == want_sha)
        .ok_or_else(|| {
            anyhow!(
                "release {} missing sha256 sidecar {want_sha}",
                pick.1.tag_name
            )
        })?;

    // 4. Fetch sha256 sidecar (small).
    let sha_resp = http
        .get(&sha_asset.browser_download_url)
        .header(
            "User-Agent",
            concat!("distri-cli/", env!("CARGO_PKG_VERSION")),
        )
        .send()
        .await?
        .error_for_status()?;
    let sha_text = sha_resp.text().await?;
    let expected_sha = sha_text
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow!("empty sha256 file at {}", sha_asset.browser_download_url))?
        .to_string();

    // 5. Download + verify + extract into the cache layout.
    let dest = match stream {
        Stream::Server => manifest::distri_home()?.join("bin"),
        Stream::Ui => manifest::distri_home()?.join("ui").join(pick.0.to_string()),
    };
    download_verify_extract(http, &asset.browser_download_url, &expected_sha, &dest).await?;

    // 6. Compute the binary path inside the extracted destination + persist manifest.
    let path = match stream {
        Stream::Server => {
            let bin_name = format!("distri-server-{}-{}-{}", pick.0, plat.os, plat.arch);
            let candidate = dest.join(&bin_name);
            if candidate.exists() {
                candidate
            } else {
                dest.join("distri-server")
            }
        }
        Stream::Ui => dest.clone(),
    };

    let rec = EntryRecord {
        version: pick.0.to_string(),
        installed_at: chrono::Utc::now(),
        sha256: expected_sha,
        path: path.clone(),
    };
    match stream {
        Stream::Server => mf.server = Some(rec),
        Stream::Ui => mf.ui = Some(rec),
    }
    manifest::write(&mf)?;

    Ok(path)
}

fn current_record(mf: &Manifest, stream: Stream) -> Option<&EntryRecord> {
    match stream {
        Stream::Server => mf.server.as_ref(),
        Stream::Ui => mf.ui.as_ref(),
    }
}

fn pick_release<'a>(
    candidates: &'a [(Version, &'a GhRelease)],
    req: &VersionReq,
    pinned: Option<&Version>,
    stream: Stream,
) -> Result<&'a (Version, &'a GhRelease)> {
    if let Some(pin) = pinned {
        return candidates
            .iter()
            .find(|(v, _)| v == pin)
            .ok_or_else(|| anyhow!("pinned {stream:?} version {pin} not found in releases"));
    }
    candidates
        .iter()
        .filter(|(v, _)| req.matches(v))
        .max_by(|a, b| a.0.cmp(&b.0))
        .ok_or_else(|| {
            anyhow!(
                "no compatible {stream:?} release found for compat range; \
                 have {} candidate(s), none match",
                candidates.len()
            )
        })
}

#[cfg(test)]
mod tests {
    use super::super::releases::{GhAsset, GhRelease};
    use super::*;

    fn release(tag: &str, assets: Vec<(&str, &str)>) -> GhRelease {
        GhRelease {
            tag_name: tag.into(),
            draft: false,
            prerelease: false,
            assets: assets
                .into_iter()
                .map(|(n, u)| GhAsset {
                    name: n.into(),
                    browser_download_url: u.into(),
                })
                .collect(),
        }
    }

    #[test]
    fn pick_release_chooses_newest_in_range() {
        let r1 = release("server-v0.5.1", vec![]);
        let r2 = release("server-v0.5.7", vec![]);
        let r3 = release("server-v0.5.3", vec![]);
        let candidates = vec![
            (Version::parse("0.5.1").unwrap(), &r1),
            (Version::parse("0.5.7").unwrap(), &r2),
            (Version::parse("0.5.3").unwrap(), &r3),
        ];
        let req = VersionReq::parse(">=0.5.0, <0.6.0").unwrap();
        let pick = pick_release(&candidates, &req, None, Stream::Server).unwrap();
        assert_eq!(pick.0.to_string(), "0.5.7");
    }

    #[test]
    fn pick_release_honors_pin() {
        let r1 = release("server-v0.5.1", vec![]);
        let r2 = release("server-v0.5.7", vec![]);
        let candidates = vec![
            (Version::parse("0.5.1").unwrap(), &r1),
            (Version::parse("0.5.7").unwrap(), &r2),
        ];
        let req = VersionReq::parse(">=0.5.0, <0.6.0").unwrap();
        let pin = Version::parse("0.5.1").unwrap();
        let pick = pick_release(&candidates, &req, Some(&pin), Stream::Server).unwrap();
        assert_eq!(pick.0.to_string(), "0.5.1");
    }

    #[test]
    fn pick_release_rejects_unknown_pin() {
        let r1 = release("server-v0.5.1", vec![]);
        let candidates = vec![(Version::parse("0.5.1").unwrap(), &r1)];
        let req = VersionReq::parse(">=0.5.0, <0.6.0").unwrap();
        let pin = Version::parse("0.5.99").unwrap();
        assert!(pick_release(&candidates, &req, Some(&pin), Stream::Server).is_err());
    }

    #[test]
    fn pick_release_errors_when_no_match() {
        let r1 = release("server-v0.6.0", vec![]);
        let candidates = vec![(Version::parse("0.6.0").unwrap(), &r1)];
        let req = VersionReq::parse(">=0.5.0, <0.6.0").unwrap();
        assert!(pick_release(&candidates, &req, None, Stream::Server).is_err());
    }
}
