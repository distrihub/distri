use anyhow::{anyhow, Context, Result};
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::path::Path;
use tar::Archive;

pub async fn download_to_bytes(http: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    let resp = http
        .get(url)
        .header(
            "User-Agent",
            concat!("distri-cli/", env!("CARGO_PKG_VERSION")),
        )
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .with_context(|| format!("non-success status from {url}"))?;
    Ok(resp.bytes().await?.to_vec())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

pub fn extract_tar_gz(tar_gz: &[u8], dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)
        .with_context(|| format!("create_dir_all {}", dest.display()))?;
    let dec = GzDecoder::new(tar_gz);
    let mut ar = Archive::new(dec);
    ar.unpack(dest)
        .with_context(|| format!("unpack tar.gz to {}", dest.display()))?;
    Ok(())
}

/// Download `url`, verify its sha256 against `expected_sha256` (case-insensitive
/// hex), and extract the tar.gz contents into `dest`.
pub async fn download_verify_extract(
    http: &reqwest::Client,
    url: &str,
    expected_sha256: &str,
    dest: &Path,
) -> Result<()> {
    let bytes = download_to_bytes(http, url).await?;
    let got = sha256_hex(&bytes);
    if !got.eq_ignore_ascii_case(expected_sha256) {
        return Err(anyhow!(
            "sha256 mismatch for {url}: got {got}, expected {expected_sha256}"
        ));
    }
    extract_tar_gz(&bytes, dest)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_tar_gz(files: &[(&str, &[u8])]) -> Vec<u8> {
        let buf = Vec::new();
        let mut gz = flate2::write::GzEncoder::new(buf, flate2::Compression::default());
        {
            let mut tar = tar::Builder::new(&mut gz);
            for (name, data) in files {
                let mut header = tar::Header::new_gnu();
                header.set_size(data.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                tar.append_data(&mut header, name, *data).unwrap();
            }
            tar.finish().unwrap();
        }
        gz.finish().unwrap()
    }

    #[test]
    fn sha256_known_vector() {
        assert_eq!(
            sha256_hex(b"hello"),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn extract_writes_files() {
        let bytes = make_tar_gz(&[
            ("hello.txt", b"world"),
            ("nested/inner.bin", &[0xde, 0xad]),
        ]);
        let tmp = TempDir::new().unwrap();
        extract_tar_gz(&bytes, tmp.path()).unwrap();
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("hello.txt")).unwrap(),
            "world"
        );
        assert_eq!(
            std::fs::read(tmp.path().join("nested/inner.bin")).unwrap(),
            vec![0xde, 0xad]
        );
    }

    #[test]
    fn extract_creates_dest_if_missing() {
        let bytes = make_tar_gz(&[("a.txt", b"a")]);
        let tmp = TempDir::new().unwrap();
        let dest = tmp.path().join("does/not/exist");
        assert!(!dest.exists());
        extract_tar_gz(&bytes, &dest).unwrap();
        assert!(dest.join("a.txt").exists());
    }
}
