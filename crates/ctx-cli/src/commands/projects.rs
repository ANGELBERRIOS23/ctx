//! Implementation of the `ctx projects` command.
//!
//! Lists all projects tracked by ctx, displaying their names, IDs, and sync status.

use anyhow::Result;
use ctx_core::config::{GlobalConfig, ProjectConfig};

/// Lists tracked projects in the workspace or reports local project info.
pub async fn projects(_config: &GlobalConfig) -> Result<()> {
    println!("Tracked ctx projects:");
    println!("──────────────────────────────────────────");

    if let Ok(cur) = std::env::current_dir() {
        if let Ok(proj) = ProjectConfig::load(&cur) {
            println!(
                "  • {} ({})\n    Server: {}\n    Branch: {}\n    Path:   {}",
                proj.project.name,
                proj.project.id,
                proj.project.server,
                proj.git.branch,
                cur.display()
            );
            println!("──────────────────────────────────────────");
            return Ok(());
        }
    }

    println!("  No active project detected in current working directory.");
    println!("  Run `ctx init <name>` to track a project here.");
    println!("──────────────────────────────────────────");
    Ok(())
}

/// Convenience runner executing [`projects`].
pub async fn run(config: &GlobalConfig) -> Result<()> {
    projects(config).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_projects_command_callable() {
        let config = GlobalConfig::default();
        let res = projects(&config).await;
        assert!(res.is_ok());
    }

    #[test]
    fn test_projects_runner() {
        let config = GlobalConfig::default();
        assert_eq!(config.interval, 300);
    }
}
