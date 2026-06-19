use directories::{BaseDirs, ProjectDirs};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub destination_root: PathBuf,
    pub filename_pattern: String,
    #[serde(default = "default_case_insensitive")]
    pub case_insensitive: bool,
    #[serde(default = "default_extensions")]
    pub extensions: Vec<String>,
    /// Path to the alias database. Defaults to the platform data dir when absent.
    pub database: Option<PathBuf>,
}

fn default_case_insensitive() -> bool {
    true
}

fn default_extensions() -> Vec<String> {
    ["jpg", "jpeg", "png", "gif", "webp"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Cannot determine default config directory")]
    NoConfigDir,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("YAML parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("Invalid filename_pattern: {0}")]
    InvalidPattern(#[from] regex::Error),
    #[error("filename_pattern has no 'username' capture group")]
    MissingUsernameGroup,
}

impl Config {
    /// Load and validate config from the given path.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path)?;
        let mut config: Config = serde_yaml::from_str(&text)?;
        // Expand ~ in paths
        config.destination_root = expand_tilde(config.destination_root);
        config.database = config.database.map(expand_tilde);
        config.validate()?;
        Ok(config)
    }

    /// Validate that the config values are well-formed.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let re = regex::Regex::new(&self.filename_pattern)?;
        if re.capture_names().flatten().all(|n| n != "username") {
            return Err(ConfigError::MissingUsernameGroup);
        }
        Ok(())
    }

    /// Default config file path for this platform.
    pub fn default_path() -> Option<PathBuf> {
        ProjectDirs::from("", "", "sortah")
            .map(|d| d.config_dir().join("config.yaml"))
    }

    /// Default database path for this platform.
    pub fn default_db_path() -> Option<PathBuf> {
        ProjectDirs::from("", "", "sortah")
            .map(|d| d.data_dir().join("mappings.db"))
    }

    /// Resolve the database path: use the explicit config value if set, else the platform default.
    pub fn resolved_db_path(&self) -> Option<PathBuf> {
        self.database.clone().or_else(Self::default_db_path)
    }

    /// Write a commented starter config to the given path, creating parent directories as needed.
    pub fn write_template(path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let template = r#"# sortah configuration
# Edit this file freely — sortah reads it on every run.

# Where sorted images will be placed. Each person gets a subdirectory here.
destination_root: ~/Pictures/Friends

# Regex applied to each image filename. Must contain a named capture group `username`.
# Example: for filenames like "joeBloggs_IMG_1234.jpg", this extracts "joeBloggs".
filename_pattern: '^(?P<username>[^_]+)_'

# Whether to compare usernames case-insensitively when matching against stored aliases.
# Aliases are always stored verbatim; this only affects the comparison at sort time.
# When true, a file with username "JoeBloggs" matches a stored alias "joeBloggs".
case_insensitive: true

# Image file extensions to process (case-insensitive).
extensions: [jpg, jpeg, png, gif, webp]

# Path to the alias database. Defaults to the platform data directory when omitted.
# database: ~/.local/share/sortah/mappings.db
"#;
        std::fs::write(path, template)?;
        Ok(())
    }
}

/// Expand a leading `~` to the current user's home directory.
fn expand_tilde(path: PathBuf) -> PathBuf {
    let s = match path.to_str() {
        Some(s) => s,
        None => return path,
    };
    if s == "~" {
        BaseDirs::new()
            .map(|d| d.home_dir().to_path_buf())
            .unwrap_or(path)
    } else if let Some(rest) = s.strip_prefix("~/") {
        BaseDirs::new()
            .map(|d| d.home_dir().join(rest))
            .unwrap_or(path)
    } else {
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_config(content: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    #[test]
    fn valid_config_loads() {
        let f = write_config(
            "destination_root: /tmp/dest\nfilename_pattern: '^(?P<username>[^_]+)_'\n",
        );
        let config = Config::load(f.path()).unwrap();
        assert_eq!(config.filename_pattern, "^(?P<username>[^_]+)_");
        assert!(config.case_insensitive); // default
    }

    #[test]
    fn missing_username_group_is_rejected() {
        let f = write_config(
            "destination_root: /tmp/dest\nfilename_pattern: '^([^_]+)_'\n",
        );
        let err = Config::load(f.path()).unwrap_err();
        assert!(matches!(err, ConfigError::MissingUsernameGroup));
    }

    #[test]
    fn invalid_regex_is_rejected() {
        let f = write_config(
            "destination_root: /tmp/dest\nfilename_pattern: '[invalid'\n",
        );
        let err = Config::load(f.path()).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidPattern(_)));
    }
}
