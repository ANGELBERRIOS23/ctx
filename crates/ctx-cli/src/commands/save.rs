//! Context save command implementation for `ctx-cli`.
//!
//! Extracts project state and handoff context from the active AI coding agent,
//! persists a Markdown handoff file to `.ctx/handoff.md`, pushes an encrypted
//! synchronization snapshot to the ctx server, and outputs a formatted summary.

use std::path::Path;

use anyhow::{Context, Result};
use console::style;
use ctx_adapters::opencode::OpenCodeAdapter;
use ctx_adapters::{
    AgentAdapter, ClaudeAdapter, CodexAdapter, CursorAdapter, GenericAdapter,
};
use ctx_core::config::ProjectConfig;
use ctx_core::crypto::{encrypt_bytes, generate_keypair};
use ctx_core::handoff::Handoff;
use ctx_core::protocol::{SnapshotType, SyncSnapshot};
use ctx_core::state::{LocalState, StateTransition};
use uuid::Uuid;

/// Detects the active AI coding agent based on environment variables,
/// process metadata, or project configuration.
pub fn detect_active_agent(project_dir: &Path, config: Option<&ProjectConfig>) -> Box<dyn AgentAdapter> {
    // 1. Check environment variables indicating an active agent session
    if std::env::var("CLAUDE_CODE").is_ok() || std::env::var("CLAUDE_PROJECT_DIR").is_ok() {
        return Box::new(ClaudeAdapter::new());
    }
    if std::env::var("CURSOR_SESSION").is_ok()
        || std::env::var("CURSOR_TRACE").is_ok()
        || std::env::var("TERM_PROGRAM").map(|v| v.to_ascii_lowercase().contains("cursor")).unwrap_or(false)
    {
        return Box::new(CursorAdapter::new());
    }
    if std::env::var("CODEX_ENV").is_ok() || std::env::var("CODEX_SESSION").is_ok() {
        return Box::new(CodexAdapter::new());
    }
    if std::env::var("OPENCODE_SESSION").is_ok() {
        return Box::new(OpenCodeAdapter::new());
    }

    // 2. Check project configuration for preferred or last used agent
    if let Some(cfg) = config
        && let Some(agent_name) = cfg.agents.last_used.as_ref().or(cfg.agents.preferred.as_ref()) {
            match agent_name.to_ascii_lowercase().as_str() {
                "claude" | "claude_code" => return Box::new(ClaudeAdapter::new()),
                "cursor" => return Box::new(CursorAdapter::new()),
                "codex" => return Box::new(CodexAdapter::new()),
                "opencode" => return Box::new(OpenCodeAdapter::new()),
                _ => {}
            }
        }

    // 3. Check for presence of agent instruction files in the workspace
    if project_dir.join("CLAUDE.md").exists() {
        return Box::new(ClaudeAdapter::new());
    }
    if project_dir.join(".cursor").exists() || project_dir.join(".cursorrules").exists() {
        return Box::new(CursorAdapter::new());
    }
    if project_dir.join(".opencode").exists() {
        return Box::new(OpenCodeAdapter::new());
    }
    if project_dir.join(".codex").exists() {
        return Box::new(CodexAdapter::new());
    }

    // 4. Check locally installed binaries
    let claude = ClaudeAdapter::new();
    if claude.detect_installed() {
        return Box::new(claude);
    }
    let cursor = CursorAdapter::new();
    if cursor.detect_installed() {
        return Box::new(cursor);
    }
    let opencode = OpenCodeAdapter::new();
    if opencode.detect_installed() {
        return Box::new(opencode);
    }
    let codex = CodexAdapter::new();
    if codex.detect_installed() {
        return Box::new(codex);
    }

    // 5. Fallback to generic adapter
    Box::new(GenericAdapter::new())
}

/// Saves the project context and handoff state from the current working directory.
///
/// Extracts the handoff snapshot from the active agent, saves `.ctx/handoff.md`,
/// pushes the snapshot to the remote server, and prints a summary.
pub async fn run(message: Option<String>) -> Result<()> {
    let current_dir = std::env::current_dir().context("Failed to get current directory")?;
    save_project_context(&current_dir, message).await?;
    Ok(())
}

/// Convenience runner executing [`run`] with a GlobalConfig reference.
pub async fn save(_config: &ctx_core::config::GlobalConfig, message: Option<&str>) -> Result<()> {
    run(message.map(|s| s.to_string())).await
}

/// Internal implementation of saving project context for a specific directory.
pub async fn save_project_context(project_dir: &Path, message: Option<String>) -> Result<Handoff> {
    let config = ProjectConfig::load(project_dir).ok();
    let project_name = config
        .as_ref()
        .map(|c| c.project.name.clone())
        .unwrap_or_else(|| {
            project_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unnamed-project")
                .to_string()
        });

    let agent = detect_active_agent(project_dir, config.as_ref());
    let agent_name = agent.name().to_string();

    // Extract or synthesize handoff
    let mut handoff = agent
        .extract_handoff(project_dir)
        .unwrap_or_else(|_| Handoff::for_project(&project_name));

    if handoff.project_name.is_empty() {
        handoff.project_name = project_name.clone();
    }
    handoff.source_agent = agent_name.clone();
    if let Ok(hostname) = std::env::var("HOSTNAME").or_else(|_| std::env::var("HOST")) {
        handoff.source_machine = hostname;
    }

    // Append or set summary if custom message was provided
    if let Some(msg) = message {
        let trimmed = msg.trim();
        if !trimmed.is_empty() {
            if handoff.summary.trim().is_empty() {
                handoff.summary = trimmed.to_string();
            } else {
                handoff.summary = format!("{}\n\n{}", handoff.summary.trim(), trimmed);
            }
        }
    }

    // Write .ctx/handoff.md
    let ctx_dir = project_dir.join(".ctx");
    if !ctx_dir.exists() {
        std::fs::create_dir_all(&ctx_dir)
            .with_context(|| format!("Failed to create directory '{}'", ctx_dir.display()))?;
    }
    let handoff_path = ctx_dir.join("handoff.md");
    let markdown_content = handoff.to_markdown();
    std::fs::write(&handoff_path, &markdown_content)
        .with_context(|| format!("Failed to write handoff file to '{}'", handoff_path.display()))?;

    // Update local state if state.json exists or create it
    let mut local_state = LocalState::load(project_dir).unwrap_or_else(|_| {
        let proj_id = config.as_ref().map(|c| c.project.id).unwrap_or_else(Uuid::new_v4);
        LocalState::new(proj_id)
    });
    let _ = local_state.apply_transition(StateTransition::Save);
    let _ = local_state.save(project_dir);

    // Push snapshot to server if configured
    let mut push_status = "Skipped (no server configured)".to_string();
    if let Some(ref cfg) = config {
        let server_url = cfg.project.server.trim_end_matches('/');
        if !server_url.is_empty() {
            let handoff_bytes = serde_json::to_vec(&handoff)
                .context("Failed to serialize handoff to JSON")?;

            // Encrypt before transit
            let (pub_key, _) = generate_keypair();
            let encrypted_blob = encrypt_bytes(&pub_key, &handoff_bytes)
                .unwrap_or(handoff_bytes);

            let snapshot = SyncSnapshot::new(
                cfg.project.id,
                Uuid::new_v4(),
                SnapshotType::Manual,
                &cfg.git.branch,
                encrypted_blob,
            );

            let client = reqwest::Client::new();
            let push_endpoint = format!("{}/api/sync/push", server_url);

            match client.post(&push_endpoint).json(&snapshot).send().await {
                Ok(resp) if resp.status().is_success() => {
                    push_status = format!("Pushed successfully to {}", server_url);
                }
                Ok(resp) => {
                    push_status = format!("Server returned status {} ({})", resp.status(), server_url);
                }
                Err(err) => {
                    push_status = format!("Failed to connect to server ({}) - saved locally: {}", server_url, err);
                }
            }
        }
    }

    // Print formatted summary
    println!("\n{}", style("─── Context Saved Successfully ───").bold().green());
    println!("  • Project:         {}", style(&project_name).cyan());
    println!("  • Active Agent:    {}", style(&agent_name).yellow());
    println!("  • Handoff File:    {}", style(handoff_path.display().to_string()).dim());
    println!("  • Server Push:     {}", style(&push_status).dim());

    if !handoff.summary.trim().is_empty() {
        println!("  • Summary:         {}", handoff.summary.trim());
    }
    println!(
        "  • Tasks:           {} completed, {} in progress, {} pending",
        handoff.completed.len(),
        handoff.in_progress.len(),
        handoff.pending.len()
    );
    if !handoff.decisions.is_empty() {
        println!("  • Decisions:       {} recorded", handoff.decisions.len());
    }
    if !handoff.blockers.is_empty() {
        println!("  • Blockers:        {} active", handoff.blockers.len());
    }
    println!();

    Ok(handoff)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctx_core::config::{
        AgentsSection, EnvironmentSection, GitSection, ProjectSection, SecretsSection,
        SyncSection,
    };
    use std::collections::HashMap;

    fn create_test_project_config(id: Uuid, name: &str, server: &str) -> ProjectConfig {
        ProjectConfig::new(
            ProjectSection::new(id, name, server),
            GitSection::new("origin", "main"),
            SecretsSection::new("manual", HashMap::new()),
            EnvironmentSection::default(),
            AgentsSection::new(Some("claude".to_string()), Some("cursor".to_string())),
            SyncSection::default(),
        )
    }

    #[tokio::test]
    async fn test_save_project_context_creates_handoff_file() {
        let temp_dir = std::env::temp_dir().join(format!("ctx_test_save_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).expect("Failed to create temp test directory");

        let proj_id = Uuid::new_v4();
        let config = create_test_project_config(proj_id, "test-save-app", "http://127.0.0.1:9999");
        config.save(&temp_dir).expect("Failed to save project config");

        let custom_msg = Some("Implemented database migrations and unit tests".to_string());
        let handoff = save_project_context(&temp_dir, custom_msg)
            .await
            .expect("save_project_context must succeed");

        assert_eq!(handoff.project_name, "test-save-app");
        assert!(handoff.summary.contains("Implemented database migrations"));

        let handoff_file = temp_dir.join(".ctx").join("handoff.md");
        assert!(handoff_file.exists());

        let content = std::fs::read_to_string(&handoff_file).expect("Read handoff.md");
        assert!(content.contains("test-save-app"));
        assert!(content.contains("Implemented database migrations"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_detect_active_agent_precedence() {
        let temp_dir = std::env::temp_dir().join(format!("ctx_test_agent_det_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).expect("Create temp dir");

        // 1. With Claude config
        let mut config = create_test_project_config(Uuid::new_v4(), "agent-app", "http://localhost");
        config.agents.last_used = Some("claude".to_string());
        let agent = detect_active_agent(&temp_dir, Some(&config));
        assert_eq!(agent.name(), "claude");

        // 2. With Cursor config
        config.agents.last_used = Some("cursor".to_string());
        let agent = detect_active_agent(&temp_dir, Some(&config));
        assert_eq!(agent.name(), "cursor");

        // 3. Fallback without config
        let fallback = detect_active_agent(&temp_dir, None);
        assert!(!fallback.name().is_empty());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
