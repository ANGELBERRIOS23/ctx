use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Errors that can occur when loading, parsing, or saving configuration files.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// An I/O error occurred while interacting with a configuration file or directory.
    #[error("I/O error at {path}: {source}")]
    Io {
        /// The path where the I/O error occurred.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// An error occurred while parsing YAML configuration data.
    #[error("Failed to parse YAML configuration at {path}: {source}")]
    YamlParse {
        /// The path of the file that failed parsing.
        path: PathBuf,
        /// The underlying YAML deserialization error.
        #[source]
        source: serde_yaml::Error,
    },

    /// An error occurred while serializing configuration data to YAML.
    #[error("Failed to serialize configuration to YAML: {0}")]
    YamlSerialize(#[source] serde_yaml::Error),

    /// The requested configuration file or directory was not found.
    #[error("Configuration file not found at: {0}")]
    NotFound(PathBuf),

    /// The user's home directory could not be determined.
    #[error("Failed to determine user home directory")]
    MissingHomeDir,

    /// Configuration validation failed.
    #[error("Validation error: {0}")]
    Validation(String),
}

/// A specialized `Result` type for configuration operations.
pub type Result<T> = std::result::Result<T, ConfigError>;

/// Reference to a vault secret. Never holds the plaintext secret value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretRef {
    /// The name of the environment variable or key identifier.
    pub key_name: String,
    /// The URI identifying the secret in the vault provider (e.g., `vault://api-keys/stripe`).
    pub vault_uri: String,
    /// Whether this secret is strictly required for the project or task.
    pub required: bool,
}

impl SecretRef {
    /// Creates a new secret reference.
    pub fn new(key_name: impl Into<String>, vault_uri: impl Into<String>, required: bool) -> Self {
        Self {
            key_name: key_name.into(),
            vault_uri: vault_uri.into(),
            required,
        }
    }
}

/// Project identity and connection settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSection {
    /// Unique project identifier (UUID).
    pub id: Uuid,
    /// Human-readable project name.
    pub name: String,
    /// URL or host address of the ctx server.
    pub server: String,
}

impl ProjectSection {
    /// Creates a new project section.
    pub fn new(id: Uuid, name: impl Into<String>, server: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            server: server.into(),
        }
    }
}

/// Git repository settings for synchronization and state tracking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitSection {
    /// Remote repository name or URL (e.g., "origin").
    pub remote: String,
    /// Git tracking branch (e.g., "main").
    pub branch: String,
}

impl GitSection {
    /// Creates a new git section.
    pub fn new(remote: impl Into<String>, branch: impl Into<String>) -> Self {
        Self {
            remote: remote.into(),
            branch: branch.into(),
        }
    }
}

/// Secrets configuration specifying the provider and key references.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SecretsSection {
    /// Secret vault provider identifier (e.g., "local", "keychain", "cloud").
    pub provider: String,
    /// Map of secret variable names to their corresponding vault URIs.
    #[serde(default)]
    pub refs: HashMap<String, String>,
}

impl SecretsSection {
    /// Creates a new secrets section.
    pub fn new(provider: impl Into<String>, refs: HashMap<String, String>) -> Self {
        Self {
            provider: provider.into(),
            refs,
        }
    }

    /// Converts secret reference entries into a vector of `SecretRef` objects.
    pub fn to_secret_refs(&self) -> Vec<SecretRef> {
        self.refs
            .iter()
            .map(|(key, uri)| SecretRef::new(key.clone(), uri.clone(), true))
            .collect()
    }
}

/// Environment requirements and tool specifications.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct EnvironmentSection {
    /// List of required developer tools and runtime dependencies (e.g., `["rust", "cargo"]`).
    #[serde(default)]
    pub tools: Vec<String>,
}

impl EnvironmentSection {
    /// Creates a new environment section with the specified tools.
    pub fn new(tools: Vec<String>) -> Self {
        Self { tools }
    }
}

/// AI coding agent configurations and historical usage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AgentsSection {
    /// Preferred AI agent identifier for this project (e.g., "claude", "cursor").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred: Option<String>,
    /// Last agent that interacted with this project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used: Option<String>,
}

impl AgentsSection {
    /// Creates a new agents section.
    pub fn new(preferred: Option<String>, last_used: Option<String>) -> Self {
        Self {
            preferred,
            last_used,
        }
    }
}

/// Project synchronization behavior and filter configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncSection {
    /// Time interval in seconds between automatic saves.
    #[serde(default = "default_auto_save_interval")]
    pub auto_save_interval: u64,
    /// Whether to synchronize binary assets and large artifacts.
    #[serde(default)]
    pub include_assets: bool,
    /// Path patterns to exclude from synchronization (e.g., `["target", ".git"]`).
    #[serde(default)]
    pub exclude: Vec<String>,
}

fn default_auto_save_interval() -> u64 {
    300
}

impl Default for SyncSection {
    fn default() -> Self {
        Self {
            auto_save_interval: default_auto_save_interval(),
            include_assets: false,
            exclude: Vec::new(),
        }
    }
}

impl SyncSection {
    /// Creates a new sync section.
    pub fn new(auto_save_interval: u64, include_assets: bool, exclude: Vec<String>) -> Self {
        Self {
            auto_save_interval,
            include_assets,
            exclude,
        }
    }
}

/// Project-level configuration stored in `.ctx/config.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectConfig {
    /// Project identification and server settings.
    pub project: ProjectSection,
    /// Git repository synchronization settings.
    pub git: GitSection,
    /// Secrets provider and secret references.
    pub secrets: SecretsSection,
    /// Environment tools requirements.
    pub environment: EnvironmentSection,
    /// AI agent preferences and history.
    pub agents: AgentsSection,
    /// Sync options and file exclusions.
    pub sync: SyncSection,
}

impl ProjectConfig {
    /// Creates a new `ProjectConfig` with the given section definitions.
    pub fn new(
        project: ProjectSection,
        git: GitSection,
        secrets: SecretsSection,
        environment: EnvironmentSection,
        agents: AgentsSection,
        sync: SyncSection,
    ) -> Self {
        Self {
            project,
            git,
            secrets,
            environment,
            agents,
            sync,
        }
    }

    /// Returns the path to the `.ctx` directory for the given project directory.
    pub fn ctx_dir(dir: &Path) -> PathBuf {
        dir.join(".ctx")
    }

    /// Returns the path to the `config.yaml` file inside the `.ctx` directory for the given project.
    pub fn config_path(dir: &Path) -> PathBuf {
        Self::ctx_dir(dir).join("config.yaml")
    }

    /// Loads project configuration from `<dir>/.ctx/config.yaml`.
    pub fn load(dir: &Path) -> Result<Self> {
        let path = Self::config_path(dir);
        if !path.exists() {
            return Err(ConfigError::NotFound(path));
        }
        let content = fs::read_to_string(&path).map_err(|source| ConfigError::Io {
            path: path.clone(),
            source,
        })?;
        let config: Self = serde_yaml::from_str(&content)
            .map_err(|source| ConfigError::YamlParse { path, source })?;
        Ok(config)
    }

    /// Saves project configuration to `<dir>/.ctx/config.yaml`.
    /// Creates the `.ctx` directory if it does not already exist.
    pub fn save(&self, dir: &Path) -> Result<()> {
        let ctx_dir = Self::ctx_dir(dir);
        if !ctx_dir.exists() {
            fs::create_dir_all(&ctx_dir).map_err(|source| ConfigError::Io {
                path: ctx_dir.clone(),
                source,
            })?;
        }
        let path = Self::config_path(dir);
        let yaml_str = serde_yaml::to_string(self).map_err(ConfigError::YamlSerialize)?;
        fs::write(&path, yaml_str).map_err(|source| ConfigError::Io { path, source })?;
        Ok(())
    }

    /// Parses a `ProjectConfig` from a YAML string.
    pub fn from_yaml_str(content: &str) -> Result<Self> {
        serde_yaml::from_str(content).map_err(|source| ConfigError::YamlParse {
            path: PathBuf::from("<memory>"),
            source,
        })
    }

    /// Serializes the `ProjectConfig` to a formatted YAML string.
    pub fn to_yaml_string(&self) -> Result<String> {
        serde_yaml::to_string(self).map_err(ConfigError::YamlSerialize)
    }
}

/// Returns the `.ctx` directory path within the specified directory.
pub fn ctx_dir(dir: &Path) -> PathBuf {
    ProjectConfig::ctx_dir(dir)
}

/// Global synchronization mode across development projects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    /// Automatically synchronize all projects known to ctx.
    #[default]
    Auto,
    /// Only synchronize explicitly selected projects.
    Selective,
}

/// Global configuration for the ctx daemon and CLI, stored in `~/.ctx/config.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalConfig {
    /// Global synchronization mode (`auto` or `selective`).
    pub sync_mode: SyncMode,
    /// Background synchronization interval in seconds.
    pub interval: u64,
    /// Automatically persist and sync project state when an AI agent exits.
    pub auto_save_on_agent_exit: bool,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            sync_mode: SyncMode::Auto,
            interval: 300,
            auto_save_on_agent_exit: true,
        }
    }
}

impl GlobalConfig {
    /// Creates a new `GlobalConfig`.
    pub fn new(sync_mode: SyncMode, interval: u64, auto_save_on_agent_exit: bool) -> Self {
        Self {
            sync_mode,
            interval,
            auto_save_on_agent_exit,
        }
    }

    /// Returns the global `.ctx` directory in the user's home directory.
    pub fn global_dir() -> Result<PathBuf> {
        let home = dirs::home_dir().ok_or(ConfigError::MissingHomeDir)?;
        Ok(home.join(".ctx"))
    }

    /// Returns the default global configuration file path (`~/.ctx/config.yaml`).
    pub fn default_path() -> Result<PathBuf> {
        Ok(Self::global_dir()?.join("config.yaml"))
    }

    /// Loads global configuration from a specified directory (`<dir>/config.yaml`).
    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join("config.yaml");
        if !path.exists() {
            return Err(ConfigError::NotFound(path));
        }
        let content = fs::read_to_string(&path).map_err(|source| ConfigError::Io {
            path: path.clone(),
            source,
        })?;
        let config: Self = serde_yaml::from_str(&content)
            .map_err(|source| ConfigError::YamlParse { path, source })?;
        Ok(config)
    }

    /// Saves global configuration to a specified directory (`<dir>/config.yaml`).
    /// Creates the directory if it does not already exist.
    pub fn save(&self, dir: &Path) -> Result<()> {
        if !dir.exists() {
            fs::create_dir_all(dir).map_err(|source| ConfigError::Io {
                path: dir.to_path_buf(),
                source,
            })?;
        }
        let path = dir.join("config.yaml");
        let yaml_str = serde_yaml::to_string(self).map_err(ConfigError::YamlSerialize)?;
        fs::write(&path, yaml_str).map_err(|source| ConfigError::Io { path, source })?;
        Ok(())
    }

    /// Loads the global configuration from the default user home directory path (`~/.ctx/config.yaml`).
    pub fn load_default() -> Result<Self> {
        let path = Self::default_path()?;
        if !path.exists() {
            return Err(ConfigError::NotFound(path));
        }
        let content = fs::read_to_string(&path).map_err(|source| ConfigError::Io {
            path: path.clone(),
            source,
        })?;
        let config: Self = serde_yaml::from_str(&content)
            .map_err(|source| ConfigError::YamlParse { path, source })?;
        Ok(config)
    }

    /// Saves the global configuration to the default user home directory path (`~/.ctx/config.yaml`).
    pub fn save_default(&self) -> Result<()> {
        let dir = Self::global_dir()?;
        self.save(&dir)
    }

    /// Parses a `GlobalConfig` from a YAML string.
    pub fn from_yaml_str(content: &str) -> Result<Self> {
        serde_yaml::from_str(content).map_err(|source| ConfigError::YamlParse {
            path: PathBuf::from("<memory>"),
            source,
        })
    }

    /// Serializes the `GlobalConfig` to a formatted YAML string.
    pub fn to_yaml_string(&self) -> Result<String> {
        serde_yaml::to_string(self).map_err(ConfigError::YamlSerialize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_PROJECT_YAML: &str = r#"
project:
  id: "550e8400-e29b-41d4-a716-446655440000"
  name: "ctx-agent-test"
  server: "https://ctx.example.com"
git:
  remote: "origin"
  branch: "main"
secrets:
  provider: "local"
  refs:
    API_KEY: "vault://keys/api_key"
    DB_PASSWORD: "vault://keys/db_password"
environment:
  tools:
    - "rust"
    - "cargo"
    - "git"
agents:
  preferred: "claude"
  last_used: "cursor"
sync:
  auto_save_interval: 60
  include_assets: false
  exclude:
    - "target"
    - ".git"
    - "*.tmp"
"#;

    #[test]
    fn test_parse_sample_project_config_yaml() {
        let config = ProjectConfig::from_yaml_str(SAMPLE_PROJECT_YAML)
            .expect("Failed to parse sample project YAML");

        let expected_uuid =
            Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").expect("Valid UUID");
        assert_eq!(config.project.id, expected_uuid);
        assert_eq!(config.project.name, "ctx-agent-test");
        assert_eq!(config.project.server, "https://ctx.example.com");

        assert_eq!(config.git.remote, "origin");
        assert_eq!(config.git.branch, "main");

        assert_eq!(config.secrets.provider, "local");
        assert_eq!(
            config.secrets.refs.get("API_KEY").map(String::as_str),
            Some("vault://keys/api_key")
        );
        assert_eq!(
            config.secrets.refs.get("DB_PASSWORD").map(String::as_str),
            Some("vault://keys/db_password")
        );

        assert_eq!(config.environment.tools, vec!["rust", "cargo", "git"]);

        assert_eq!(config.agents.preferred.as_deref(), Some("claude"));
        assert_eq!(config.agents.last_used.as_deref(), Some("cursor"));

        assert_eq!(config.sync.auto_save_interval, 60);
        assert!(!config.sync.include_assets);
        assert_eq!(config.sync.exclude, vec!["target", ".git", "*.tmp"]);
    }

    #[test]
    fn test_project_config_roundtrip_serialization() {
        let config =
            ProjectConfig::from_yaml_str(SAMPLE_PROJECT_YAML).expect("Failed to parse sample YAML");

        let yaml_str = config
            .to_yaml_string()
            .expect("Failed to serialize to YAML");
        let roundtrip_config =
            ProjectConfig::from_yaml_str(&yaml_str).expect("Failed to deserialize serialized YAML");

        assert_eq!(config, roundtrip_config);
    }

    #[test]
    fn test_load_and_save_project_config() {
        let temp_dir = std::env::temp_dir().join(format!("ctx_test_proj_{}", Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).expect("Failed to create temporary test directory");

        let config =
            ProjectConfig::from_yaml_str(SAMPLE_PROJECT_YAML).expect("Failed to parse sample YAML");

        config
            .save(&temp_dir)
            .expect("Failed to save project config");

        let expected_path = temp_dir.join(".ctx").join("config.yaml");
        assert!(expected_path.exists());

        let loaded_config =
            ProjectConfig::load(&temp_dir).expect("Failed to load project config from disk");
        assert_eq!(config, loaded_config);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_ctx_dir_helpers() {
        let dir = Path::new("/tmp/some_project");
        let expected = dir.join(".ctx");
        assert_eq!(ctx_dir(dir), expected);
        assert_eq!(ProjectConfig::ctx_dir(dir), expected);
        assert_eq!(
            ProjectConfig::config_path(dir),
            expected.join("config.yaml")
        );
    }

    #[test]
    fn test_global_config_serde_and_sync_mode() {
        let global_auto = GlobalConfig::new(SyncMode::Auto, 120, true);
        let yaml_auto = global_auto
            .to_yaml_string()
            .expect("Failed to serialize GlobalConfig");
        assert!(yaml_auto.contains("sync_mode: auto"));
        assert!(yaml_auto.contains("interval: 120"));
        assert!(yaml_auto.contains("auto_save_on_agent_exit: true"));

        let deserialized_auto =
            GlobalConfig::from_yaml_str(&yaml_auto).expect("Failed to parse GlobalConfig YAML");
        assert_eq!(global_auto, deserialized_auto);

        let global_sel = GlobalConfig::new(SyncMode::Selective, 300, false);
        let yaml_sel = global_sel
            .to_yaml_string()
            .expect("Failed to serialize GlobalConfig");
        assert!(yaml_sel.contains("sync_mode: selective"));
        let deserialized_sel =
            GlobalConfig::from_yaml_str(&yaml_sel).expect("Failed to parse GlobalConfig YAML");
        assert_eq!(global_sel, deserialized_sel);
    }

    #[test]
    fn test_global_config_save_and_load() {
        let temp_dir = std::env::temp_dir().join(format!("ctx_test_global_{}", Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).expect("Failed to create temp dir");

        let config = GlobalConfig::new(SyncMode::Selective, 45, false);
        config
            .save(&temp_dir)
            .expect("Failed to save global config");

        assert!(temp_dir.join("config.yaml").exists());

        let loaded = GlobalConfig::load(&temp_dir).expect("Failed to load global config from disk");
        assert_eq!(config, loaded);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_secret_ref_struct() {
        let secret = SecretRef::new("OPENAI_KEY", "vault://ai/openai", true);
        assert_eq!(secret.key_name, "OPENAI_KEY");
        assert_eq!(secret.vault_uri, "vault://ai/openai");
        assert!(secret.required);

        let yaml = serde_yaml::to_string(&secret).expect("Serialize SecretRef");
        let deserialized: SecretRef = serde_yaml::from_str(&yaml).expect("Deserialize SecretRef");
        assert_eq!(secret, deserialized);

        let mut refs_map = HashMap::new();
        refs_map.insert("KEY1".to_string(), "vault://k1".to_string());
        let sec_section = SecretsSection::new("keychain", refs_map);
        let secret_refs = sec_section.to_secret_refs();
        assert_eq!(secret_refs.len(), 1);
        assert_eq!(secret_refs[0].key_name, "KEY1");
        assert_eq!(secret_refs[0].vault_uri, "vault://k1");
        assert!(secret_refs[0].required);
    }

    #[test]
    fn test_config_not_found_error() {
        let non_existent = Path::new("/path/that/does/not/exist/for/ctx/test");
        let result = ProjectConfig::load(non_existent);
        assert!(result.is_err());
        match result.unwrap_err() {
            ConfigError::NotFound(p) => {
                assert_eq!(p, ProjectConfig::config_path(non_existent));
            }
            err => panic!("Expected ConfigError::NotFound, got {err:?}"),
        }
    }

    #[test]
    fn test_invalid_yaml_parse_error() {
        let invalid_yaml = "project: [invalid, yaml, structure";
        let result = ProjectConfig::from_yaml_str(invalid_yaml);
        assert!(result.is_err());
        match result.unwrap_err() {
            ConfigError::YamlParse { .. } => {}
            err => panic!("Expected ConfigError::YamlParse, got {err:?}"),
        }
    }
}
