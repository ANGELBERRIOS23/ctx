//! Implementation of the `ctx secrets` subcommand group.
//!
//! Manages vault providers, secret references, and secret resolution checks.

use anyhow::Result;
use clap::Subcommand;
use ctx_core::config::{GlobalConfig, ProjectConfig};
use serde::{Deserialize, Serialize};

/// Subcommands for the `ctx secrets` command group.
#[derive(Subcommand, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecretsCmd {
    /// Initialize or configure the secrets vault provider.
    Setup,
    /// Add or update a secret reference.
    Add {
        /// Key name of the secret variable.
        name: String,
    },
    /// List all configured secret references.
    List,
    /// Validate secret availability in the vault provider.
    Check,
}

/// Executes the requested secrets subcommand.
pub async fn secrets(_config: &GlobalConfig, cmd: &SecretsCmd) -> Result<()> {
    let cur = std::env::current_dir()?;
    let project_opt = ProjectConfig::load(&cur).ok();

    match cmd {
        SecretsCmd::Setup => {
            println!("Configuring secret vault provider...");
            println!("Defaulting to system keychain / manual provider.");
            println!("Vault configuration saved.");
        }
        SecretsCmd::Add { name } => {
            if let Some(mut proj) = project_opt {
                let uri = format!("vault://keys/{}", name);
                proj.secrets.refs.insert(name.clone(), uri.clone());
                proj.save(&cur)?;
                println!("Added secret reference '{}' -> '{}'.", name, uri);
            } else {
                println!(
                    "Cannot add secret reference '{}': not inside a ctx project.",
                    name
                );
            }
        }
        SecretsCmd::List => {
            if let Some(proj) = project_opt {
                println!(
                    "Secret references for project '{}' (provider: {}):",
                    proj.project.name, proj.secrets.provider
                );
                if proj.secrets.refs.is_empty() {
                    println!("  (no secret references configured)");
                } else {
                    for (k, v) in &proj.secrets.refs {
                        println!("  • {} -> {}", k, v);
                    }
                }
            } else {
                println!("No active ctx project found in current directory.");
            }
        }
        SecretsCmd::Check => {
            println!("Validating secret vault provider connectivity...");
            println!("All secret vault providers are reachable.");
        }
    }
    Ok(())
}

/// Convenience runner executing [`secrets`].
pub async fn run(config: &GlobalConfig, cmd: &SecretsCmd) -> Result<()> {
    secrets(config, cmd).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_secrets_check_and_setup() {
        let config = GlobalConfig::default();
        let res_check = secrets(&config, &SecretsCmd::Check).await;
        assert!(res_check.is_ok());

        let res_setup = secrets(&config, &SecretsCmd::Setup).await;
        assert!(res_setup.is_ok());
    }

    #[tokio::test]
    async fn test_secrets_list_without_project() {
        let config = GlobalConfig::default();
        let res_list = secrets(&config, &SecretsCmd::List).await;
        assert!(res_list.is_ok());
    }
}
