use semver::{Version, VersionReq};

pub const SERVER_COMPAT: &str = ">=0.5.0, <0.6.0";
pub const UI_COMPAT: &str = ">=0.5.0, <0.6.0";

pub fn server_req() -> VersionReq {
    VersionReq::parse(SERVER_COMPAT).expect("SERVER_COMPAT must parse")
}

pub fn ui_req() -> VersionReq {
    VersionReq::parse(UI_COMPAT).expect("UI_COMPAT must parse")
}

pub fn matches_server(v: &Version) -> bool {
    server_req().matches(v)
}

pub fn matches_ui(v: &Version) -> bool {
    ui_req().matches(v)
}

/// Pick the newest version in `candidates` that matches `req`. Returns None if
/// no candidate matches.
pub fn pick_newest<'a, I: IntoIterator<Item = &'a Version>>(
    candidates: I,
    req: &VersionReq,
) -> Option<&'a Version> {
    candidates
        .into_iter()
        .filter(|v| req.matches(v))
        .max()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_req_parses() {
        // Must not panic.
        let _ = server_req();
    }

    #[test]
    fn ui_req_parses() {
        let _ = ui_req();
    }

    #[test]
    fn matches_inside_range() {
        assert!(matches_server(&Version::parse("0.5.0").unwrap()));
        assert!(matches_server(&Version::parse("0.5.7").unwrap()));
        assert!(matches_server(&Version::parse("0.5.99").unwrap()));
    }

    #[test]
    fn rejects_outside_range() {
        assert!(!matches_server(&Version::parse("0.4.9").unwrap()));
        assert!(!matches_server(&Version::parse("0.6.0").unwrap()));
        assert!(!matches_server(&Version::parse("1.0.0").unwrap()));
    }

    #[test]
    fn pick_newest_within_range() {
        let vs = vec![
            Version::parse("0.5.1").unwrap(),
            Version::parse("0.5.7").unwrap(),
            Version::parse("0.5.3").unwrap(),
            Version::parse("0.6.0").unwrap(),
            Version::parse("0.4.9").unwrap(),
        ];
        let pick = pick_newest(vs.iter(), &server_req())
            .expect("at least one candidate matches");
        assert_eq!(pick.to_string(), "0.5.7");
    }

    #[test]
    fn pick_newest_returns_none_when_no_match() {
        let vs = vec![
            Version::parse("0.4.9").unwrap(),
            Version::parse("0.6.0").unwrap(),
        ];
        assert!(pick_newest(vs.iter(), &server_req()).is_none());
    }
}
