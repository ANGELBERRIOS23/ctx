//! Implementation of the `ctx claim` command.
//!
//! Acquires an exclusive session lock for a project via `POST /api/session/claim`,
//! launches a background heartbeat task to maintain the lock, updates local state
//! to [`ProjectState::Active`], and prints the name of the machine that was previously active.

use std::fs;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use ctx_core::config::{GlobalConfig, ProjectConfig};
use ctx_core::state::{LocalState, ProjectState};
use uuid::Uuid;

/// Resolves the project directory from an optional command-line argument.
pub fn resolve_project_dir(project: Option<&str>) -> Result<PathBuf> {
    match project {
        Some(p) => {
            let path = PathBuf::from(p);
            if path.exists() {
                Ok(path.canonicalize().unwrap_or(path))
            } else {
                let current = std::env::current_dir().context("Failed to get current working directory")?;
                let rel = current.join(p);
                if rel.exists() {
                    Ok(rel.canonicalize().unwrap_or(rel))
                } else {
                    Ok(path)
                }
            }
        }
        None => std::env::current_dir().context("Failed to get current working directory"),
    }
}

/// Retrieves an auth token if configured in environment, keyring, or ~/.ctx.
pub fn get_auth_token() -> Option<String> {
    if let Ok(token) = std::env::var("CTX_TOKEN") {
        let trimmed = token.trim().to_string();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }
    if let Ok(entry) = keyring::Entry::new("ctx", "token") {
        if let Ok(token) = entry.get_password() {
            let trimmed = token.trim().to_string();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }
    if let Ok(global_dir) = GlobalConfig::global_dir() {
        let token_path = global_dir.join("token");
        if token_path.exists() {
            if let Ok(token) = fs::read_to_string(&token_path) {
                let trimmed = token.trim().to_string();
                if !trimmed.is_empty() {
                    return Some(trimmed);
                }
            }
        }
    }
    None
}

/// Retrieves or generates a persistent machine identifier.
pub fn get_machine_id() -> Uuid {
    if let Ok(id_str) = std::env::var("CTX_MACHINE_ID") {
        if let Ok(id) = Uuid::parse_str(id_str.trim()) {
            return id;
        }
    }
    if let Ok(global_dir) = GlobalConfig::global_dir() {
        let machine_file = global_dir.join("machine_id");
        if machine_file.exists() {
            if let Ok(content) = fs::read_to_string(&machine_file) {
                if let Ok(id) = Uuid::parse_str(content.trim()) {
                    return id;
                }
            }
        }
        let new_id = Uuid::new_v4();
        let _ = fs::create_dir_all(&global_dir);
        let _ = fs::write(&machine_file, new_id.to_string());
        return new_id;
    }
    Uuid::new_v4()
}

/// Spawns a background task that periodically sends heartbeat refreshes to the server.
pub fn spawn_heartbeat_task(
    server_url: String,
    project_id: Uuid,
    machine_id: Uuid,
    token: Option<String>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        let url = format!("{}/api/session/heartbeat", server_url.trim_end_matches('/'));
        let heartbeat_body = serde_json::json!({
            "project_id": project_id,
            "machine_id": machine_id,
        });
        // Refresh heartbeat every 30 seconds (server lock timeout is 120s)
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            let mut req = client.post(&url).json(&heartbeat_body);
            if let Some(ref t) = token {
                req = req.bearer_auth(t);
            }
            match req.send().await {
                Ok(resp) if resp.status().is_success() => {
                    tracing::debug!("Heartbeat sent successfully for project {}", project_id);
                }
                Ok(resp) => {
                    tracing::warn!("Heartbeat returned status {}", resp.status());
                }
                Err(e) => {
                    tracing::warn!("Heartbeat request failed: {}", e);
                }
            }
        }
    })
}

/// Queries the server for the project details and resolves the previously active machine name.
pub async fn fetch_active_machine_name(
    client: &reqwest::Client,
    server_url: &str,
    project_id: Uuid,
    token: Option<&str>,
) -> Option<String> {
    let project_url = format!("{}/api/projects/{}", server_url.trim_end_matches('/'), project_id);
    let mut req = client.get(&project_url);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let resp = req.send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let project_info: ctx_core::protocol::ProjectInfo = resp.json().await.ok()?;
    let active_machine_id = project_info.active_machine?;

    // Attempt to lookup machine name by ID
    let machine_url = format!("{}/api/machines/{}", server_url.trim_end_matches('/'), active_machine_id);
    let mut m_req = client.get(&machine_url);
    if let Some(t) = token {
        m_req = m_req.bearer_auth(t);
    }
    if let Ok(m_resp) = m_req.send().await {
        if m_resp.status().is_success() {
            if let Ok(machine_info) = m_resp.json::<ctx_core::protocol::MachineInfo>().await {
                return Some(machine_info.name);
            }
        }
    }
    Some(active_machine_id.to_string())
}

/// Claims exclusive session ownership for a project.
///
/// Sends `POST /api/session/claim`, starts a heartbeat background task,
/// updates local state to Active, and prints the previously active machine name.
pub async fn run(project: Option<String>) -> Result<()> {
    let project_dir = resolve_project_dir(project.as_deref())?;
    let config = ProjectConfig::load(&project_dir)
        .with_context(|| format!("Failed to load project config at {}", project_dir.display()))?;

    let server_url = config.project.server.trim_end_matches('/');
    let client = reqwest::Client::new();
    let token = get_auth_token();

    let prev_machine_name = fetch_active_machine_name(&client, server_url, config.project.id, token.as_deref()).await;

    let machine_id = get_machine_id();
    let agent_name = config.agents.preferred.clone().or(config.agents.last_used.clone());

    let claim_req = serde_json::json!({
        "project_id": config.project.id,
        "machine_id": machine_id,
        "agent_name": agent_name,
    });

    let endpoint = format!("{}/api/session/claim", server_url);
    let mut req = client.post(&endpoint).json(&claim_req);
    if let Some(ref t) = token {
        req = req.bearer_auth(t);
    }

    let resp = req
        .send()
        .await
        .with_context(|| format!("Failed to send claim request to {}", endpoint))?;

    if resp.status() == reqwest::StatusCode::CONFLICT {
        let err_body = resp.text().await.unwrap_or_default();
        bail!("Session claim conflict: project is actively locked by another machine. {}", err_body);
    }

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("Failed to claim session (HTTP {status}): {body}");
    }

    // Start background heartbeat task
    let _heartbeat_handle = spawn_heartbeat_task(
        server_url.to_string(),
        config.project.id,
        machine_id,
        token.clone(),
    );

    // Update local state to Active
    let mut state = LocalState::load(&project_dir).unwrap_or_else(|_| LocalState::new(config.project.id));
    let active_agent = agent_name.unwrap_or_else(|| "default".to_string());
    if let Err(e) = state.claim(&active_agent) {
        tracing::debug!("Transition claim returned {:?}, forcing state to Active", e);
        state.state = ProjectState::Active;
        state.active_agent = Some(active_agent);
    }
    state
        .save(&project_dir)
        .with_context(|| format!("Failed to save state at {}", project_dir.display()))?;

    match prev_machine_name {
        Some(name) => println!("Previously active machine: {}", name),
        None => println!("Previously active machine: (none)"),
    }

    println!(
        "Successfully claimed session for project '{}' ({})",
        config.project.name, config.project.id
    );

    Ok(())
}

/// Convenience runner executing [`run`] with a GlobalConfig reference.
pub async fn claim(_config: &GlobalConfig, project: Option<&str>) -> Result<()> {
    run(project.map(|s| s.to_string())).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_project_dir_default() {
        let res = resolve_project_dir(None).expect("Resolves default dir");
        assert!(res.is_dir());
    }

    #[test]
    fn test_get_machine_id_not_empty() {
        let id = get_machine_id();
        assert!(!id.is_nil());
    }

    #[tokio::test]
    async fn test_spawn_heartbeat_task_cancels_cleanly() {
        let handle = spawn_heartbeat_task(
            "http://127.0.0.1:9".to_string(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            None,
        );
        handle.abort();
        let _ = handle.await;
    }
}
