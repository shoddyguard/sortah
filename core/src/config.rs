use directories::{BaseDirs, ProjectDirs};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub destination_root: Option<PathBuf>,
    #[serde(default = "default_case_insensitive")]
    pub case_insensitive: bool,
    #[serde(default = "default_extensions")]
    pub extensions: Vec<String>,
    /// Path to the alias database. Defaults to the platform data dir when absent.
    pub database: Option<PathBuf>,
    #[serde(default)]
    pub sort_in_place: bool,
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
    #[error("No destination configured: set `destination_root`, enable `sort_in_place`, or pass --dest")]
    NoDestination,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("YAML parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),
}

impl Config {
    /// Load and validate config from the given path.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path)?;
        let mut config: Config = serde_yaml::from_str(&text)?;
        config.destination_root = config.destination_root.map(expand_tilde);
        config.database = config.database.map(expand_tilde);
        config.validate()?;
        Ok(config)
    }

    /// Validate that the config values are well-formed.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.destination_root.is_none() && !self.sort_in_place {
            return Err(ConfigError::NoDestination);
        }
        Ok(())
    }

    /// Resolve the effective destination root.
    ///
    /// Precedence: dest_override > sort_in_place (source dir) > destination_root.
    pub fn resolve_dest_root(
        &self,
        source_dir: &Path,
        dest_override: Option<&Path>,
    ) -> Result<PathBuf, ConfigError> {
        if let Some(d) = dest_override {
            return Ok(d.to_path_buf());
        }
        if self.sort_in_place {
            return Ok(source_dir.to_path_buf());
        }
        self.destination_root.clone().ok_or(ConfigError::NoDestination)
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

    /// Serialize the current config to YAML and write it to `path`.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_yaml::to_string(self)?;
        std::fs::write(path, text)?;
        Ok(())
    }

    /// Write a commented starter config to the given path, creating parent directories as needed.
    pub fn write_template(path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let template = r#"# sortah configuration
# Edit this file freely — sortah reads it on every run.

# Where sorted images will be placed.
# Each image lands in: destination_root/<category>/<person>/filename
# People with no category use an "Uncategorised" folder.
destination_root: ~/Pictures/Friends

# Sort into the current working directory instead of destination_root.
# When true, files are organised in place: ./<category>/<person>/filename
# Takes precedence over destination_root; --dest still overrides both.
# sort_in_place: false

# Whether to match aliases case-insensitively against filenames.
# When true, alias "joeBloggs" matches a file containing "joebloggs".
case_insensitive: true

# Image file extensions to process (case-insensitive).
extensions: [jpg, jpeg, png, gif, webp, mp4]

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
        let f = write_config("destination_root: /tmp/dest\n");
        let config = Config::load(f.path()).unwrap();
        assert!(config.case_insensitive); // default
    }

    #[test]
    fn case_insensitive_can_be_disabled() {
        let f = write_config("destination_root: /tmp/dest\ncase_insensitive: false\n");
        let config = Config::load(f.path()).unwrap();
        assert!(!config.case_insensitive);
    }

    #[test]
    fn sort_in_place_loads_without_destination_root() {
        let f = write_config("sort_in_place: true\n");
        let config = Config::load(f.path()).unwrap();
        assert!(config.sort_in_place);
        assert!(config.destination_root.is_none());
    }

    #[test]
    fn no_dest_no_in_place_fails() {
        let f = write_config("case_insensitive: true\n");
        let err = Config::load(f.path()).unwrap_err();
        assert!(matches!(err, ConfigError::NoDestination));
    }

    #[test]
    fn resolve_dest_root_precedence() {
        use std::path::Path;
        let source = Path::new("/source");
        let override_path = Path::new("/override");

        let config_with_root = Config {
            destination_root: Some(PathBuf::from("/root")),
            case_insensitive: true,
            extensions: vec![],
            database: None,
            sort_in_place: false,
        };
        assert_eq!(
            config_with_root.resolve_dest_root(source, Some(override_path)).unwrap(),
            override_path
        );
        assert_eq!(
            config_with_root.resolve_dest_root(source, None).unwrap(),
            PathBuf::from("/root")
        );

        let config_in_place = Config {
            destination_root: None,
            case_insensitive: true,
            extensions: vec![],
            database: None,
            sort_in_place: true,
        };
        assert_eq!(
            config_in_place.resolve_dest_root(source, None).unwrap(),
            source
        );
        assert_eq!(
            config_in_place.resolve_dest_root(source, Some(override_path)).unwrap(),
            override_path,
            "--dest must win over sort_in_place"
        );
    }
}
