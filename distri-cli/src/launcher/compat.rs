use semver::VersionReq;

pub const SERVER_COMPAT: &str = ">=0.5.0, <0.6.0";
pub const UI_COMPAT: &str = ">=0.5.0, <0.6.0";

pub fn server_req() -> VersionReq {
    VersionReq::parse(SERVER_COMPAT).expect("SERVER_COMPAT must parse")
}

pub fn ui_req() -> VersionReq {
    VersionReq::parse(UI_COMPAT).expect("UI_COMPAT must parse")
}

#[cfg(test)]
mod tests {
    use super::*;
    use semver::Version;

    #[test]
    fn server_req_parses() {
        let _ = server_req();
    }

    #[test]
    fn ui_req_parses() {
        let _ = ui_req();
    }

    #[test]
    fn matches_inside_range() {
        let req = server_req();
        assert!(req.matches(&Version::parse("0.5.0").unwrap()));
        assert!(req.matches(&Version::parse("0.5.7").unwrap()));
        assert!(req.matches(&Version::parse("0.5.99").unwrap()));
    }

    #[test]
    fn rejects_outside_range() {
        let req = server_req();
        assert!(!req.matches(&Version::parse("0.4.9").unwrap()));
        assert!(!req.matches(&Version::parse("0.6.0").unwrap()));
        assert!(!req.matches(&Version::parse("1.0.0").unwrap()));
    }
}
