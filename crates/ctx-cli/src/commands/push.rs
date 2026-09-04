//! Implementation of the `ctx push` command.
//!
//! Reads `.ctx/handoff.md` and `.ctx/state.json`, encrypts the handoff using `age`
//! via [`ctx_core::crypto::encrypt_bytes`], constructs a [`SyncSnapshot`],
//! and POSTs to `/api/sync/push`. Upon success, updates the local project state
//! to [`ProjectState::Synced`].

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use ctx_core::config::{GlobalConfig, ProjectConfig};
use ctx_core::crypto::encrypt_bytes;
use ctx_core::protocol::{SnapshotType, SyncSnapshot};
use ctx_core::state::{LocalState, ProjectState, StateTransition};
use uuid::Uuid;

/// Retrieves an auth token if configured in environment, keyring, or ~/.ctx.
pub fn get_auth_token() -> Option<String> {
    if let Ok(token) = std::env::var("CTX_TOKEN") {
        let trimmed = token.trim().to_string();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }
    if let Ok(entry) = keyring::Entry::new("ctx", "token")
        && let Ok(token) = entry.get_password() {
            let trimmed = token.trim().to_string();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    if let Ok(global_dir) = GlobalConfig::global_dir() {
        let token_path = global_dir.join("token");
        if token_path.exists()
            && let Ok(token) = fs::read_to_string(&token_path) {
                let trimmed = token.trim().to_string();
                if !trimmed.is_empty() {
                    return Some(trimmed);
                }
            }
    }
    None
}

/// Retrieves an age public key for encrypting snapshots.
///
/// Searches:
/// 1. `CTX_PUBLIC_KEY` environment variable.
/// 2. Derives from `CTX_SECRET_KEY` if set.
/// 3. Reads from `~/.ctx/key.txt`.
/// 4. Generates a new keypair and saves it to `~/.ctx/key.txt` if none exists.
pub fn get_public_key() -> Result<String> {
    if let Ok(pk) = std::env::var("CTX_PUBLIC_KEY") {
        let trimmed = pk.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
    }
    if let Ok(sk) = std::env::var("CTX_SECRET_KEY") {
        let trimmed = sk.trim();
        if let Ok(identity) = trimmed.parse::<age::x25519::Identity>() {
            return Ok(identity.to_public().to_string());
        }
    }
    if let Ok(global_dir) = GlobalConfig::global_dir() {
        let key_path = global_dir.join("key.txt");
        if key_path.exists()
            && let Ok(content) = fs::read_to_string(&key_path) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with("age1") {
                        return Ok(trimmed.to_string());
                    }
                    if trimmed.starts_with("AGE-SECRET-KEY-")
                        && let Ok(identity) = trimmed.parse::<age::x25519::Identity>() {
                            return Ok(identity.to_public().to_string());
                        }
                }
            }
        // Auto-generate if not found
        let (pub_key, sec_key) = ctx_core::crypto::generate_keypair();
        let _ = fs::create_dir_all(&global_dir);
        let _ = fs::write(&key_path, format!("{}\n{}\n", pub_key, sec_key));
        return Ok(pub_key);
    }
    bail!("No public key available for encryption")
}

/// Retrieves or generates a persistent machine identifier.
pub fn get_machine_id() -> Uuid {
    if let Ok(id_str) = std::env::var("CTX_MACHINE_ID")
        && let Ok(id) = Uuid::parse_str(id_str.trim()) {
            return id;
        }
    if let Ok(global_dir) = GlobalConfig::global_dir() {
        let machine_file = global_dir.join("machine_id");
        if machine_file.exists()
            && let Ok(content) = fs::read_to_string(&machine_file)
                && let Ok(id) = Uuid::parse_str(content.trim()) {
                    return id;
                }
        let new_id = Uuid::new_v4();
        let _ = fs::create_dir_all(&global_dir);
        let _ = fs::write(&machine_file, new_id.to_string());
        return new_id;
    }
    Uuid::new_v4()
}

/// Extracts the current Git commit SHA for the project directory.
pub fn get_git_commit(project_dir: &Path) -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(project_dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "HEAD".to_string())
}

/// Discovers all registered project paths from `~/.ctx/projects.json`.
pub fn discover_registered_projects() -> Vec<PathBuf> {
    if let Ok(global_dir) = GlobalConfig::global_dir() {
        let list_path = global_dir.join("projects.json");
        if list_path.exists()
            && let Ok(content) = fs::read_to_string(&list_path)
                && let Ok(paths) = serde_json::from_str::<Vec<PathBuf>>(&content) {
                    return paths;
                }
    }
    Vec::new()
}

/// Pushes the state of a single project directory to the remote server.
///
/// Reads `.ctx/handoff.md` and `state.json`, encrypts the handoff with age,
/// sends `POST /api/sync/push`, and updates local state to [`ProjectState::Synced`].
pub async fn push_project(project_dir: &Path) -> Result<()> {
    let config = ProjectConfig::load(project_dir)
        .with_context(|| format!("Failed to load project config at {}", project_dir.display()))?;

    let ctx_dir = project_dir.join(".ctx");
    let handoff_path = ctx_dir.join("handoff.md");

    let handoff_bytes = if handoff_path.exists() {
        fs::read(&handoff_path)
            .with_context(|| format!("Failed to read handoff at {}", handoff_path.display()))?
    } else {
        let default_handoff = ctx_core::handoff::Handoff::for_project(&config.project.name);
        default_handoff.to_markdown().into_bytes()
    };

    let state_path = ctx_dir.join("state.json");
    let state_json = if state_path.exists() {
        let s = fs::read_to_string(&state_path)
            .with_context(|| format!("Failed to read state at {}", state_path.display()))?;
        serde_json::from_str::<serde_json::Value>(&s).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let public_key = get_public_key()
        .context("Failed to obtain public key for snapshot encryption")?;

    let encrypted_handoff = encrypt_bytes(&public_key, &handoff_bytes)
        .context("Failed to encrypt handoff payload with age public key")?;

    let git_commit = get_git_commit(project_dir);
    let machine_id = get_machine_id();

    let snapshot = SyncSnapshot::new(
        config.project.id,
        machine_id,
        SnapshotType::Manual,
        git_commit,
        encrypted_handoff,
    )
    .with_state_json(state_json);

    let server_url = config.project.server.trim_end_matches('/');
    let endpoint = format!("{}/api/sync/push", server_url);

    let client = reqwest::Client::new();
    let mut req = client.post(&endpoint).json(&snapshot);
    if let Some(token) = get_auth_token() {
        req = req.bearer_auth(token);
    }

    let resp = req
        .send()
        .await
        .with_context(|| format!("Failed to send push request to {}", endpoint))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("Server returned error during push (HTTP {status}): {body}");
    }

    let mut state = LocalState::load(project_dir).unwrap_or_else(|_| LocalState::new(config.project.id));
    if let Err(e) = state.apply_transition(StateTransition::Push) {
        tracing::debug!("Transition to push returned {:?}, forcing state to Synced", e);
        state.state = ProjectState::Synced;
        state.last_sync = chrono::Utc::now();
        state.active_agent = None;
    }
    state
        .save(project_dir)
        .with_context(|| format!("Failed to save state.json at {}", project_dir.display()))?;

    println!(
        "Successfully pushed snapshot for project '{}' ({})",
        config.project.name, config.project.id
    );
    Ok(())
}

/// Reads `.ctx/handoff.md` + `state.json`, encrypts with age, POSTs to server `/api/sync/push`. Updates local state to Synced.
///
/// If `all` is `true`, iterates through all registered projects or defaults to current project.
pub async fn run(all: bool) -> Result<()> {
    if all {
        let projects = discover_registered_projects();
        if !projects.is_empty() {
            for proj_dir in projects {
                if let Err(e) = push_project(&proj_dir).await {
                    tracing::warn!("Failed to push project at {}: {}", proj_dir.display(), e);
                }
            }
            return Ok(());
        }
    }

    let project_dir = std::env::current_dir().context("Failed to get current directory")?;
    push_project(&project_dir).await
}

/// Convenience runner executing [`run`] with a GlobalConfig reference.
pub async fn push(_config: &ctx_core::config::GlobalConfig, all: bool) -> Result<()> {
    run(all).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_machine_id_deterministic_or_valid() {
        let id1 = get_machine_id();
        assert!(!id1.is_nil());
    }

    #[test]
    fn test_get_git_commit_returns_string() {
        let temp = std::env::temp_dir();
        let commit = get_git_commit(&temp);
        assert!(!commit.is_empty());
    }

    #[test]
    fn test_get_public_key_from_env() {
        let (pub_key, _sec_key) = ctx_core::crypto::generate_keypair();
        unsafe {
            std::env::set_var("CTX_PUBLIC_KEY", &pub_key);
        }
        let retrieved = get_public_key().expect("Should retrieve CTX_PUBLIC_KEY");
        assert_eq!(retrieved, pub_key);
        unsafe {
            std::env::remove_var("CTX_PUBLIC_KEY");
        }
    }
}
