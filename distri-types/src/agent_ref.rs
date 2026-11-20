//! A reference to an agent, optionally pinned to a saved version: `name` or
//! `name@N` (spec `2026-11-20-agent-versions.md`, V1).
//!
//! Agent names are letters, digits, `_` and `/`, so `@` never appears in a
//! name and splits the reference unambiguously. `N` is a saved `version_no`
//! (1, 2, …) or `latest`; `name` and `name@latest` both mean the current
//! definition.

use std::fmt;
use std::str::FromStr;

/// Which version of an agent a reference selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AgentVersionSel {
    /// The current definition (`name`, `name@latest`).
    #[default]
    Latest,
    /// A saved version (`name@N`, `N >= 1`).
    Version(u32),
}

impl AgentVersionSel {
    /// The pinned number, `None` for the latest.
    pub fn version_no(&self) -> Option<u32> {
        match self {
            AgentVersionSel::Latest => None,
            AgentVersionSel::Version(n) => Some(*n),
        }
    }
}

impl From<Option<u32>> for AgentVersionSel {
    fn from(v: Option<u32>) -> Self {
        v.map_or(AgentVersionSel::Latest, AgentVersionSel::Version)
    }
}

/// `name` or `name@N`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AgentRef {
    pub name: String,
    pub version: AgentVersionSel,
}

/// A malformed agent reference (`a@`, `a@x`, `a@0`, `@3`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid agent reference '{reference}': {reason}")]
pub struct AgentRefError {
    pub reference: String,
    pub reason: String,
}

impl AgentRef {
    /// The current definition of `name`.
    pub fn latest(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: AgentVersionSel::Latest,
        }
    }

    /// `name` pinned to `version_no` (`None`: the latest).
    pub fn new(name: impl Into<String>, version_no: Option<u32>) -> Self {
        Self {
            name: name.into(),
            version: version_no.into(),
        }
    }

    /// The pinned number, `None` for the latest.
    pub fn version_no(&self) -> Option<u32> {
        self.version.version_no()
    }

    /// Parse `name`, `name@N` or `name@latest`.
    pub fn parse(s: &str) -> Result<Self, AgentRefError> {
        let err = |reason: &str| AgentRefError {
            reference: s.to_string(),
            reason: reason.to_string(),
        };
        let Some((name, version)) = s.split_once('@') else {
            if s.is_empty() {
                return Err(err("the agent name is empty"));
            }
            return Ok(Self::latest(s));
        };
        if name.is_empty() {
            return Err(err("the agent name is empty"));
        }
        let version = match version {
            "" => return Err(err("the version after '@' is empty")),
            "latest" => AgentVersionSel::Latest,
            v => match v.parse::<u32>() {
                Ok(0) => return Err(err("versions start at 1")),
                Ok(n) if v.chars().all(|c| c.is_ascii_digit()) => AgentVersionSel::Version(n),
                _ => return Err(err("the version must be a number or 'latest'")),
            },
        };
        Ok(Self {
            name: name.to_string(),
            version,
        })
    }
}

impl FromStr for AgentRef {
    type Err = AgentRefError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// `name` for the latest, `name@N` for a pinned version.
impl fmt::Display for AgentRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.version {
            AgentVersionSel::Latest => f.write_str(&self.name),
            AgentVersionSel::Version(n) => write!(f, "{}@{}", self.name, n),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_prints() {
        for (input, name, version, printed) in [
            ("a", "a", AgentVersionSel::Latest, "a"),
            ("a@3", "a", AgentVersionSel::Version(3), "a@3"),
            ("a@latest", "a", AgentVersionSel::Latest, "a"),
            ("pkg/a@2", "pkg/a", AgentVersionSel::Version(2), "pkg/a@2"),
        ] {
            let r = AgentRef::parse(input).unwrap();
            assert_eq!(r.name, name, "{input}");
            assert_eq!(r.version, version, "{input}");
            assert_eq!(r.to_string(), printed, "{input}");
            // Printing and parsing again gives the same reference.
            assert_eq!(AgentRef::parse(&r.to_string()).unwrap(), r, "{input}");
        }
    }

    #[test]
    fn rejects_malformed_references() {
        for input in ["a@", "a@x", "a@0", "@3", "", "a@-1", "a@+2", "a@1@2"] {
            assert!(AgentRef::parse(input).is_err(), "{input} should be rejected");
        }
    }

    #[test]
    fn version_no() {
        assert_eq!(AgentRef::parse("a@7").unwrap().version_no(), Some(7));
        assert_eq!(AgentRef::parse("a").unwrap().version_no(), None);
        assert_eq!(AgentRef::new("a", Some(2)).to_string(), "a@2");
        assert_eq!(AgentRef::new("a", None), AgentRef::latest("a"));
    }
}
