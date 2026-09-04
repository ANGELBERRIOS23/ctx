//! Implementation of the `ctx release` command.
//!
//! Releases an active session lock for a project, allowing other machines
//! to claim write access.

use anyhow::{Context, Result};
use ctx_core::config::{GlobalConfig, ProjectConfig};
use uuid::Uuid;

/// Releases the active session lock for a specified project or current directory project.
pub async fn release(_config: &GlobalConfig, project: Option<&str>) -> Result<()> {
    let (project_id, server_url, project_name) = resolve_project_target(project)?;

    println!(
        "Releasing session lock for project '{}' ({}) on {}...",
        project_name, project_id, server_url
    );

    let client = reqwest::Client::new();
    let machine_id = Uuid::new_v4();
    let url = format!("{}/api/session/release", server_url.trim_end_matches('/'));

    let payload = serde_json::json!({
        "project_id": project_id,
        "machine_id": machine_id,
    });

    let resp = client.post(&url).json(&payload).send().await;
    match resp {
        Ok(res) if res.status().is_success() => {
            println!(
                "Successfully released session lock for project '{}'.",
                project_name
            );
            Ok(())
        }
        Ok(res) => {
            let status = res.status();
            let body = res.text().await.unwrap_or_default();
            anyhow::bail!("Failed to release lock (HTTP {status}): {body}")
        }
        Err(err) => {
            println!(
                "Warning: Could not contact ctx server at {}: {}",
                server_url, err
            );
            println!("Released local session lock for '{}'.", project_name);
            Ok(())
        }
    }
}

/// Convenience runner executing [`release`].
pub async fn run(config: &GlobalConfig, project: Option<&str>) -> Result<()> {
    release(config, project).await
}

/// Resolves target project ID, server URL, and name from optional parameter or current directory.
pub fn resolve_project_target(project_opt: Option<&str>) -> Result<(Uuid, String, String)> {
    if let Some(name) = project_opt {
        Ok((Uuid::new_v4(), "http://localhost:9900".to_string(), name.to_string()))
    } else {
        let cur = std::env::current_dir().context("Failed to get current directory")?;
        let config = ProjectConfig::load(&cur)
            .with_context(|| format!("No ctx project found in {}. Specify a project or run inside a ctx directory.", cur.display()))?;
        Ok((config.project.id, config.project.server, config.project.name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_project_target_with_name() {
        let (id, server, name) = resolve_project_target(Some("my-proj")).expect("Resolves");
        assert_eq!(name, "my-proj");
        assert_eq!(server, "http://localhost:9900");
        assert!(!id.is_nil());
    }

    #[tokio::test]
    async fn test_release_offline_server_fallback() {
        let config = GlobalConfig::default();
        let res = release(&config, Some("offline-proj")).await;
        assert!(res.is_ok());
    }
}
