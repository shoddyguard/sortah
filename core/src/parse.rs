use regex::Regex;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("Invalid regex pattern: {0}")]
    InvalidPattern(#[from] regex::Error),
    #[error("Pattern has no 'username' capture group")]
    MissingUsernameGroup,
}

/// Extracts usernames from filenames using a configurable regex.
#[derive(Debug)]
pub struct FilenameParser {
    regex: Regex,
}

impl FilenameParser {
    pub fn new(pattern: &str) -> Result<Self, ParseError> {
        let regex = Regex::new(pattern)?;
        if regex.capture_names().flatten().all(|n| n != "username") {
            return Err(ParseError::MissingUsernameGroup);
        }
        Ok(Self { regex })
    }

    /// Extract the username from a filename (the bare name, not a full path).
    /// Returns `None` if the pattern does not match the filename.
    pub fn extract_username(&self, filename: &str) -> Option<String> {
        self.regex
            .captures(filename)
            .and_then(|caps| caps.name("username").map(|m| m.as_str().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parser(pattern: &str) -> FilenameParser {
        FilenameParser::new(pattern).unwrap()
    }

    #[test]
    fn extracts_username_prefix() {
        let p = parser(r"^(?P<username>[^_]+)_");
        assert_eq!(
            p.extract_username("joeBloggs_IMG_1234.jpg"),
            Some("joeBloggs".to_string())
        );
    }

    #[test]
    fn no_match_returns_none() {
        let p = parser(r"^(?P<username>[^_]+)_");
        assert_eq!(p.extract_username("no-underscore.jpg"), None);
    }

    #[test]
    fn missing_username_group_rejected() {
        let err = FilenameParser::new(r"^([^_]+)_").unwrap_err();
        assert!(matches!(err, ParseError::MissingUsernameGroup));
    }

    #[test]
    fn case_preserved_in_extraction() {
        let p = parser(r"^(?P<username>[^_]+)_");
        assert_eq!(
            p.extract_username("JoeBloggs_photo.png"),
            Some("JoeBloggs".to_string())
        );
    }
}
