//! Implementation of the `ctx init` command.
//!
//! Initializes a new `ctx` project workspace in the current or target directory,
//! creating the `.ctx/config.yaml` configuration file.

use std::path::Path;

use anyhow::{Context, Result};
use ctx_core::config::{
    AgentsSection, EnvironmentSection, GitSection, GlobalConfig, ProjectConfig,
    ProjectSection, SecretsSection, SyncSection,
};
use uuid::Uuid;

/// Initializes a new ctx project in the current working directory with the given name.
pub async fn init(config: &GlobalConfig, name: &str) -> Result<()> {
    let current_dir = std::env::current_dir().context("Failed to get current directory")?;
    let _ = init_in_dir(config, name, &current_dir)?;
    Ok(())
}

/// Convenience runner executing [`init`].
pub async fn run(config: &GlobalConfig, name: &str) -> Result<()> {
    init(config, name).await
}

/// Initializes a new ctx project in a specified directory path.
pub fn init_in_dir(_config: &GlobalConfig, name: &str, dir: &Path) -> Result<ProjectConfig> {
    let config_path = ProjectConfig::config_path(dir);
    if config_path.exists() {
        anyhow::bail!("A ctx project already exists at {}", config_path.display());
    }

    let project_id = Uuid::new_v4();
    let project_section = ProjectSection::new(project_id, name, "http://localhost:9900");
    let git_section = GitSection::new("origin", "main");
    let secrets_section = SecretsSection::default();
    let env_section = EnvironmentSection::default();
    let agents_section = AgentsSection::default();
    let sync_section = SyncSection::default();

    let project_config = ProjectConfig::new(
        project_section,
        git_section,
        secrets_section,
        env_section,
        agents_section,
        sync_section,
    );

    project_config
        .save(dir)
        .with_context(|| format!("Failed to save project configuration to {}", dir.display()))?;

    println!(
        "Initialized ctx project '{}' ({}) in {}",
        name,
        project_id,
        dir.display()
    );
    Ok(project_config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_in_temp_dir() {
        let temp_dir = std::env::temp_dir().join(format!("ctx_test_init_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).expect("Failed to create temporary test directory");

        let global_config = GlobalConfig::default();
        let project_config = init_in_dir(&global_config, "test-service", &temp_dir)
            .expect("Initialization must succeed");

        assert_eq!(project_config.project.name, "test-service");
        assert!(ProjectConfig::config_path(&temp_dir).exists());

        let loaded = ProjectConfig::load(&temp_dir).expect("Loaded configuration from disk");
        assert_eq!(loaded.project.name, "test-service");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_init_duplicate_fails() {
        let temp_dir = std::env::temp_dir().join(format!("ctx_test_init_dup_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).expect("Failed to create temporary test directory");

        let global_config = GlobalConfig::default();
        let _ = init_in_dir(&global_config, "test-service", &temp_dir)
            .expect("First initialization must succeed");

        let duplicate = init_in_dir(&global_config, "test-service", &temp_dir);
        assert!(duplicate.is_err());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
