//! Semantic versioning support

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

/// Semantic version following semver.org specification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SemanticVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl SemanticVersion {
    /// Create a new semantic version
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Check if this version is compatible with another (same major version)
    pub fn is_compatible_with(&self, other: &Self) -> bool {
        self.major == other.major
    }

    /// Increment major version (resets minor and patch to 0)
    pub fn increment_major(&self) -> Self {
        Self::new(self.major + 1, 0, 0)
    }

    /// Increment minor version (resets patch to 0)
    pub fn increment_minor(&self) -> Self {
        Self::new(self.major, self.minor + 1, 0)
    }

    /// Increment patch version
    pub fn increment_patch(&self) -> Self {
        Self::new(self.major, self.minor, self.patch + 1)
    }
}

impl Default for SemanticVersion {
    fn default() -> Self {
        Self::new(1, 0, 0)
    }
}

impl fmt::Display for SemanticVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl FromStr for SemanticVersion {
    type Err = ParseVersionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 3 {
            return Err(ParseVersionError::InvalidFormat(s.to_string()));
        }

        let major = parts[0]
            .parse()
            .map_err(|_| ParseVersionError::InvalidNumber(parts[0].to_string()))?;
        let minor = parts[1]
            .parse()
            .map_err(|_| ParseVersionError::InvalidNumber(parts[1].to_string()))?;
        let patch = parts[2]
            .parse()
            .map_err(|_| ParseVersionError::InvalidNumber(parts[2].to_string()))?;

        Ok(Self::new(major, minor, patch))
    }
}

impl PartialOrd for SemanticVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SemanticVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.major.cmp(&other.major) {
            Ordering::Equal => match self.minor.cmp(&other.minor) {
                Ordering::Equal => self.patch.cmp(&other.patch),
                other => other,
            },
            other => other,
        }
    }
}

/// Error type for version parsing
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseVersionError {
    /// Invalid format (expected X.Y.Z)
    InvalidFormat(String),
    /// Invalid number in version component
    InvalidNumber(String),
}

impl fmt::Display for ParseVersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFormat(s) => write!(f, "Invalid version format: '{}' (expected X.Y.Z)", s),
            Self::InvalidNumber(s) => write!(f, "Invalid version number: '{}'", s),
        }
    }
}

impl std::error::Error for ParseVersionError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let v = SemanticVersion::new(1, 2, 3);
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
    }

    #[test]
    fn test_default() {
        let v = SemanticVersion::default();
        assert_eq!(v, SemanticVersion::new(1, 0, 0));
    }

    #[test]
    fn test_display() {
        let v = SemanticVersion::new(1, 2, 3);
        assert_eq!(v.to_string(), "1.2.3");
    }

    #[test]
    fn test_parse_valid() {
        let v: SemanticVersion = "1.2.3".parse().unwrap();
        assert_eq!(v, SemanticVersion::new(1, 2, 3));

        let v: SemanticVersion = "0.0.0".parse().unwrap();
        assert_eq!(v, SemanticVersion::new(0, 0, 0));

        let v: SemanticVersion = "100.200.300".parse().unwrap();
        assert_eq!(v, SemanticVersion::new(100, 200, 300));
    }

    #[test]
    fn test_parse_invalid_format() {
        let result: Result<SemanticVersion, _> = "1.2".parse();
        assert!(matches!(result, Err(ParseVersionError::InvalidFormat(_))));

        let result: Result<SemanticVersion, _> = "1.2.3.4".parse();
        assert!(matches!(result, Err(ParseVersionError::InvalidFormat(_))));

        let result: Result<SemanticVersion, _> = "1".parse();
        assert!(matches!(result, Err(ParseVersionError::InvalidFormat(_))));
    }

    #[test]
    fn test_parse_invalid_number() {
        let result: Result<SemanticVersion, _> = "a.2.3".parse();
        assert!(matches!(result, Err(ParseVersionError::InvalidNumber(_))));

        let result: Result<SemanticVersion, _> = "1.b.3".parse();
        assert!(matches!(result, Err(ParseVersionError::InvalidNumber(_))));

        let result: Result<SemanticVersion, _> = "1.2.c".parse();
        assert!(matches!(result, Err(ParseVersionError::InvalidNumber(_))));
    }

    #[test]
    fn test_ordering() {
        let v1 = SemanticVersion::new(1, 0, 0);
        let v2 = SemanticVersion::new(2, 0, 0);
        let v3 = SemanticVersion::new(1, 1, 0);
        let v4 = SemanticVersion::new(1, 0, 1);

        assert!(v1 < v2);
        assert!(v1 < v3);
        assert!(v1 < v4);
        assert!(v3 < v2);
        assert!(v4 < v3);
    }

    #[test]
    fn test_is_compatible_with() {
        let v1 = SemanticVersion::new(1, 0, 0);
        let v2 = SemanticVersion::new(1, 5, 3);
        let v3 = SemanticVersion::new(2, 0, 0);

        assert!(v1.is_compatible_with(&v2));
        assert!(v2.is_compatible_with(&v1));
        assert!(!v1.is_compatible_with(&v3));
    }

    #[test]
    fn test_increment() {
        let v = SemanticVersion::new(1, 2, 3);

        assert_eq!(v.increment_major(), SemanticVersion::new(2, 0, 0));
        assert_eq!(v.increment_minor(), SemanticVersion::new(1, 3, 0));
        assert_eq!(v.increment_patch(), SemanticVersion::new(1, 2, 4));
    }

    #[test]
    fn test_serde_json() {
        let v = SemanticVersion::new(1, 2, 3);
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, r#"{"major":1,"minor":2,"patch":3}"#);

        let parsed: SemanticVersion = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, v);
    }
}
