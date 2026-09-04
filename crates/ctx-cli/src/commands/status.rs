//! Implementation of the `ctx status` command.
//!
//! Reports the synchronization status, session lock, and metadata of the
//! current project workspace.

use std::path::Path;

use anyhow::Result;
use ctx_core::config::{GlobalConfig, ProjectConfig};

/// Formats the status report for a given project configuration and global settings.
pub fn format_status_report(project: Option<&ProjectConfig>, global: &GlobalConfig) -> String {
    let mut out = String::new();
    out.push_str("ctx status report\n");
    out.push_str("──────────────────────────────────────────\n");
    out.push_str(&format!("  Global Sync Mode:  {:?}\n", global.sync_mode));
    out.push_str(&format!("  Sync Interval:     {}s\n", global.interval));

    match project {
        Some(p) => {
            out.push_str("──────────────────────────────────────────\n");
            out.push_str(&format!("  Project Name:      {}\n", p.project.name));
            out.push_str(&format!("  Project ID:        {}\n", p.project.id));
            out.push_str(&format!("  Server:            {}\n", p.project.server));
            out.push_str(&format!("  Git Branch:        {}\n", p.git.branch));
            out.push_str(&format!("  Git Remote:        {}\n", p.git.remote));
            out.push_str(&format!("  Secrets Provider:  {}\n", p.secrets.provider));
            out.push_str(&format!("  Tracked Secrets:   {}\n", p.secrets.refs.len()));
            out.push_str(&format!("  Auto Save Period:  {}s\n", p.sync.auto_save_interval));
        }
        None => {
            out.push_str("──────────────────────────────────────────\n");
            out.push_str("  Workspace:         No active ctx project found in current directory.\n");
            out.push_str("                     Run `ctx init <name>` to initialize a project.\n");
        }
    }
    out.push_str("──────────────────────────────────────────\n");
    out
}

/// Executes the `ctx status` command in the current working directory.
pub async fn status(config: &GlobalConfig) -> Result<()> {
    let current_dir = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
    let project_config = ProjectConfig::load(&current_dir).ok();
    let report = format_status_report(project_config.as_ref(), config);
    print!("{}", report);
    Ok(())
}

/// Convenience runner executing [`status`].
pub async fn run(config: &GlobalConfig) -> Result<()> {
    status(config).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctx_core::config::{
        AgentsSection, EnvironmentSection, GitSection, ProjectSection, SecretsSection, SyncSection,
    };
    use uuid::Uuid;

    #[test]
    fn test_format_status_report_without_project() {
        let global = GlobalConfig::default();
        let report = format_status_report(None, &global);
        assert!(report.contains("No active ctx project found"));
        assert!(report.contains("Global Sync Mode"));
    }

    #[test]
    fn test_format_status_report_with_project() {
        let global = GlobalConfig::default();
        let project = ProjectConfig::new(
            ProjectSection::new(Uuid::new_v4(), "alpha-service", "https://api.ctx.dev"),
            GitSection::new("origin", "feature/sync"),
            SecretsSection::default(),
            EnvironmentSection::default(),
            AgentsSection::default(),
            SyncSection::default(),
        );

        let report = format_status_report(Some(&project), &global);
        assert!(report.contains("alpha-service"));
        assert!(report.contains("https://api.ctx.dev"));
        assert!(report.contains("feature/sync"));
    }
}
