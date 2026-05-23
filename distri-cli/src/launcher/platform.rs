use anyhow::{anyhow, Result};

#[derive(Debug, Clone, Copy)]
pub struct Platform {
    pub os: &'static str,
    pub arch: &'static str,
}

impl Platform {
    pub fn current() -> Result<Self> {
        let os = match std::env::consts::OS {
            "macos" => "darwin",
            "linux" => "linux",
            "windows" => "windows",
            other => return Err(anyhow!("unsupported OS: {other}")),
        };
        let arch = match std::env::consts::ARCH {
            "aarch64" => "arm64",
            "x86_64" => "x64",
            other => return Err(anyhow!("unsupported arch: {other}")),
        };
        Ok(Self { os, arch })
    }

    pub fn server_artifact(&self, version: &str) -> String {
        format!("distri-server-{version}-{}-{}.tar.gz", self.os, self.arch)
    }

    pub fn ui_artifact(version: &str) -> String {
        format!("distri-ui-{version}.tar.gz")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_resolves_on_known_target() {
        let p = Platform::current().expect("test runs on a supported target");
        assert!(["darwin", "linux", "windows"].contains(&p.os));
        assert!(["arm64", "x64"].contains(&p.arch));
    }

    #[test]
    fn artifact_name_format() {
        let p = Platform {
            os: "darwin",
            arch: "arm64",
        };
        assert_eq!(
            p.server_artifact("0.5.3"),
            "distri-server-0.5.3-darwin-arm64.tar.gz"
        );
        assert_eq!(Platform::ui_artifact("0.5.7"), "distri-ui-0.5.7.tar.gz");
    }
}
