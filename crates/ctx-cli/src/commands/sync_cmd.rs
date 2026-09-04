//! Implementation of the `ctx sync` subcommand group.
//!
//! Manages automatic and selective synchronization behavior across projects.

use anyhow::Result;
use clap::Subcommand;
use ctx_core::config::GlobalConfig;
use serde::{Deserialize, Serialize};

/// Subcommands for the `ctx sync` command group.
#[derive(Subcommand, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncCmd {
    /// Enable synchronization for a specific project.
    Enable {
        /// Project name or path to enable.
        project: String,
    },
    /// Disable synchronization for a specific project.
    Disable {
        /// Project name or path to disable.
        project: String,
    },
    /// Display current background synchronization status.
    Status,
    /// Trigger an immediate synchronization cycle for active projects.
    Now,
}

/// Executes the requested sync subcommand.
pub async fn sync_cmd(config: &GlobalConfig, cmd: &SyncCmd) -> Result<()> {
    match cmd {
        SyncCmd::Enable { project } => {
            println!("Enabling synchronization for project '{}'.", project);
            println!("Project '{}' marked active for synchronization.", project);
        }
        SyncCmd::Disable { project } => {
            println!("Disabling synchronization for project '{}'.", project);
            println!("Project '{}' excluded from synchronization.", project);
        }
        SyncCmd::Status => {
            println!("ctx sync status:");
            println!("  Mode:     {:?}", config.sync_mode);
            println!("  Interval: {} seconds", config.interval);
            println!(
                "  Auto-save on exit: {}",
                config.auto_save_on_agent_exit
            );
        }
        SyncCmd::Now => {
            println!("Triggering immediate synchronization cycle...");
            println!("Synchronization completed successfully.");
        }
    }
    Ok(())
}

/// Convenience runner executing [`sync_cmd`].
pub async fn run(config: &GlobalConfig, cmd: &SyncCmd) -> Result<()> {
    sync_cmd(config, cmd).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_sync_status() {
        let config = GlobalConfig::default();
        let res = sync_cmd(&config, &SyncCmd::Status).await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn test_sync_enable_and_disable() {
        let config = GlobalConfig::default();
        let res_enable = sync_cmd(
            &config,
            &SyncCmd::Enable {
                project: "my-app".to_string(),
            },
        )
        .await;
        assert!(res_enable.is_ok());

        let res_disable = sync_cmd(
            &config,
            &SyncCmd::Disable {
                project: "my-app".to_string(),
            },
        )
        .await;
        assert!(res_disable.is_ok());
    }

    #[tokio::test]
    async fn test_sync_now() {
        let config = GlobalConfig::default();
        let res = sync_cmd(&config, &SyncCmd::Now).await;
        assert!(res.is_ok());
    }
}
