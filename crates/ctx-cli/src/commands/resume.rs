//! Implementation of the `ctx resume` command.
//!
//! Synchronizes project state, claims an exclusive session lock, creates or detects
//! the appropriate AI coding agent adapter via [`ctx_adapters::adapter::create_adapter`],
//! generates the adapter-specific instruction file, and launches the agent.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use ctx_adapters::adapter::{create_adapter, AgentAdapter};
use ctx_core::config::ProjectConfig;
use ctx_core::handoff::Handoff;

/// Resolves the AI coding agent identifier to use.
///
/// Precedence:
/// 1. Explicitly passed `agent` argument.
/// 2. `config.agents.last_used`.
/// 3. `config.agents.preferred`.
/// 4. Fallback to `"generic"`.
pub fn resolve_selected_agent(agent_arg: Option<String>, config: &ProjectConfig) -> String {
    match agent_arg {
        Some(a) if !a.trim().is_empty() => a.trim().to_string(),
        _ => match &config.agents.last_used {
            Some(last) if !last.trim().is_empty() => last.clone(),
            _ => match &config.agents.preferred {
                Some(pref) if !pref.trim().is_empty() => pref.clone(),
                _ => "generic".to_string(),
            },
        },
    }
}

/// Spawns the agent CLI process within the specified project directory.
pub fn launch_agent_process(cmd_str: &str, project_dir: &Path) -> Result<()> {
    let parts: Vec<&str> = cmd_str.split_whitespace().collect();
    if let Some((exe, args)) = parts.split_first() {
        match std::process::Command::new(exe)
            .args(args)
            .current_dir(project_dir)
            .status()
        {
            Ok(status) => {
                println!("Agent process exited with status: {}", status);
            }
            Err(e) => {
                println!(
                    "Notice: Could not launch agent command '{}': {}. You can run it manually.",
                    cmd_str, e
                );
            }
        }
    }
    Ok(())
}

/// Resumes development work on the current project.
///
/// Workflow:
/// 1. Pulls the latest remote snapshot via [`super::pull::run`].
/// 2. Claims the session lock via [`super::claim::run`].
/// 3. Detects and creates the agent adapter via [`ctx_adapters::adapter::create_adapter`].
/// 4. Generates the tailored instructions file.
/// 5. Launches the AI coding agent.
///
/// If `agent` is not provided, defaults to `config.agents.last_used`.
pub async fn run(agent: Option<String>) -> Result<()> {
    println!("Step 1/5: Pulling latest project state from server...");
    super::pull::run(None, false)
        .await
        .context("Failed to pull latest state during resume")?;

    println!("Step 2/5: Claiming session lock on server...");
    super::claim::run(None)
        .await
        .context("Failed to claim session during resume")?;

    let project_dir = std::env::current_dir().context("Failed to determine current directory")?;
    let mut config = ProjectConfig::load(&project_dir)
        .with_context(|| format!("Failed to load project config from {}", project_dir.display()))?;

    let agent_name = resolve_selected_agent(agent, &config);

    // Update last_used agent in configuration
    config.agents.last_used = Some(agent_name.clone());
    let _ = config.save(&project_dir);

    println!("Step 3/5: Initializing adapter for agent '{}'...", agent_name);
    let adapter: Box<dyn AgentAdapter> = create_adapter(&agent_name);
    let installed = adapter.detect_installed();
    if !installed {
        println!(
            "Warning: Agent '{}' was not detected as installed in standard locations.",
            agent_name
        );
    }

    println!("Step 4/5: Generating agent instruction file...");
    let handoff_path = project_dir.join(".ctx").join("handoff.md");
    let handoff = if handoff_path.exists() {
        let content = fs::read_to_string(&handoff_path)
            .with_context(|| format!("Failed to read handoff at {}", handoff_path.display()))?;
        ctx_adapters::generic::GenericAdapter::parse_handoff_markdown(&content, &config.project.name)
            .unwrap_or_else(|_| Handoff::for_project(&config.project.name))
    } else {
        Handoff::for_project(&config.project.name)
    };

    let instructions = adapter.generate_instructions(&handoff);
    let instruction_path = adapter.instruction_path(&project_dir);

    if let Some(parent) = instruction_path.parent()
        && !parent.exists() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory {}", parent.display()))?;
        }
    fs::write(&instruction_path, &instructions)
        .with_context(|| format!("Failed to write instruction file at {}", instruction_path.display()))?;

    println!("Generated instruction file at {}", instruction_path.display());

    let launch_cmd = adapter.launch_command();
    println!("Step 5/5: Launching agent '{}' (command: '{}')...", adapter.name(), launch_cmd);
    launch_agent_process(launch_cmd, &project_dir)?;

    Ok(())
}

/// Convenience runner executing [`run`] with a GlobalConfig reference.
pub async fn resume(_config: &ctx_core::config::GlobalConfig, agent: Option<&str>) -> Result<()> {
    run(agent.map(|s| s.to_string())).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctx_core::config::{AgentsSection, EnvironmentSection, GitSection, ProjectSection, SecretsSection, SyncSection};
    use uuid::Uuid;

    fn sample_config() -> ProjectConfig {
        ProjectConfig::new(
            ProjectSection::new(Uuid::new_v4(), "test-proj", "http://localhost:8080"),
            GitSection::new("origin", "main"),
            SecretsSection::default(),
            EnvironmentSection::default(),
            AgentsSection::new(Some("cursor".to_string()), Some("claude".to_string())),
            SyncSection::default(),
        )
    }

    #[test]
    fn test_resolve_selected_agent_explicit() {
        let config = sample_config();
        assert_eq!(
            resolve_selected_agent(Some("codex".to_string()), &config),
            "codex"
        );
    }

    #[test]
    fn test_resolve_selected_agent_last_used() {
        let config = sample_config();
        assert_eq!(resolve_selected_agent(None, &config), "claude");
    }

    #[test]
    fn test_resolve_selected_agent_preferred_fallback() {
        let mut config = sample_config();
        config.agents.last_used = None;
        assert_eq!(resolve_selected_agent(None, &config), "cursor");
    }

    #[test]
    fn test_resolve_selected_agent_generic_fallback() {
        let mut config = sample_config();
        config.agents.last_used = None;
        config.agents.preferred = None;
        assert_eq!(resolve_selected_agent(None, &config), "generic");
    }
}
