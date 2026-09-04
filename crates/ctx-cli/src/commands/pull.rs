//! Implementation of the `ctx pull` command.
//!
//! Downloads the latest encrypted snapshot from the remote ctx server,
//! decrypts the handoff payload using `age` via [`ctx_core::crypto::decrypt_bytes`],
//! writes the resulting markdown to `.ctx/handoff.md`, and transitions
//! the local project state to [`ProjectState::Synced`].

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use ctx_core::config::{GlobalConfig, ProjectConfig};
use ctx_core::crypto::decrypt_bytes;
use ctx_core::protocol::SyncSnapshot;
use ctx_core::state::{LocalState, ProjectState, StateTransition};

/// Resolves the project directory from an optional command-line argument.
///
/// If `project` is provided, returns that path (or canonicalizes it if it exists).
/// If `None`, defaults to the current working directory.
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

/// Retrieves the age secret key for snapshot decryption.
///
/// Searches:
/// 1. `CTX_SECRET_KEY` environment variable.
/// 2. `~/.ctx/key.txt`.
/// 3. Local `.ctx/key.txt`.
pub fn get_secret_key() -> Result<String> {
    if let Ok(key) = std::env::var("CTX_SECRET_KEY") {
        let trimmed = key.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
    }
    if let Ok(global_dir) = GlobalConfig::global_dir() {
        let key_path = global_dir.join("key.txt");
        if key_path.exists() {
            let content = fs::read_to_string(&key_path)
                .with_context(|| format!("Failed to read key file at {}", key_path.display()))?;
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("AGE-SECRET-KEY-") {
                    return Ok(trimmed.to_string());
                }
            }
        }
    }
    let local_key = PathBuf::from(".ctx").join("key.txt");
    if local_key.exists() {
        let content = fs::read_to_string(&local_key)
            .with_context(|| format!("Failed to read key file at {}", local_key.display()))?;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("AGE-SECRET-KEY-") {
                return Ok(trimmed.to_string());
            }
        }
    }
    bail!("No age secret key found. Set CTX_SECRET_KEY or provide an identity in ~/.ctx/key.txt")
}

/// Pulls the latest snapshot for a single project directory.
///
/// Loads the project configuration, queries the server's `/api/sync/latest/{project_id}`
/// endpoint, decrypts the snapshot handoff payload, writes `.ctx/handoff.md`,
/// and transitions the local state to [`ProjectState::Synced`].
pub async fn pull_project(project_dir: &Path) -> Result<()> {
    let config = ProjectConfig::load(project_dir)
        .with_context(|| format!("Failed to load project configuration from {}", project_dir.display()))?;

    let server_url = config.project.server.trim_end_matches('/');
    let endpoint = format!("{}/api/sync/latest/{}", server_url, config.project.id);

    let client = reqwest::Client::new();
    let mut req = client.get(&endpoint);
    if let Some(token) = get_auth_token() {
        req = req.bearer_auth(token);
    }

    let resp = req
        .send()
        .await
        .with_context(|| format!("Failed to send pull request to {}", endpoint))?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        println!(
            "No remote snapshots found for project '{}' ({})",
            config.project.name, config.project.id
        );
        return Ok(());
    }

    if !resp.status().is_success() {
        bail!(
            "Server returned error status {} when pulling snapshot for project {}",
            resp.status(),
            config.project.id
        );
    }

    let snapshot: SyncSnapshot = resp
        .json()
        .await
        .context("Failed to parse snapshot JSON response from server")?;

    let secret_key = get_secret_key().context("Cannot decrypt snapshot without an age secret key")?;
    let decrypted_bytes = decrypt_bytes(&secret_key, &snapshot.handoff_blob)
        .context("Failed to decrypt snapshot handoff payload with age secret key")?;
    let handoff_content = String::from_utf8(decrypted_bytes)
        .context("Decrypted handoff payload is not valid UTF-8 text")?;

    let ctx_dir = project_dir.join(".ctx");
    if !ctx_dir.exists() {
        fs::create_dir_all(&ctx_dir)
            .with_context(|| format!("Failed to create .ctx directory at {}", ctx_dir.display()))?;
    }

    let handoff_path = ctx_dir.join("handoff.md");
    fs::write(&handoff_path, &handoff_content)
        .with_context(|| format!("Failed to write handoff file at {}", handoff_path.display()))?;

    let mut state = LocalState::load(project_dir).unwrap_or_else(|_| LocalState::new(config.project.id));
    if let Err(e) = state.apply_transition(StateTransition::Pull) {
        tracing::debug!("Transition to pull returned {:?}, forcing state to Synced", e);
        state.state = ProjectState::Synced;
        state.last_sync = chrono::Utc::now();
        state.active_agent = None;
    }
    state
        .save(project_dir)
        .with_context(|| format!("Failed to save state.json at {}", project_dir.display()))?;

    println!("Pull operation completed successfully.");
    Ok(())
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

/// Pulls the latest snapshot from the server, decrypts with age, writes `.ctx/handoff.md`, and updates state.
///
/// If `all` is `true`, pulls all known registered projects or defaults to the current project.
/// If `project` is provided, pulls that specific project directory.
pub async fn run(project: Option<String>, all: bool) -> Result<()> {
    if all {
        let projects = discover_registered_projects();
        if !projects.is_empty() {
            for proj_dir in projects {
                if let Err(e) = pull_project(&proj_dir).await {
                    tracing::warn!("Failed to pull project at {}: {}", proj_dir.display(), e);
                }
            }
            return Ok(());
        }
    }

    let project_dir = resolve_project_dir(project.as_deref())?;
    pull_project(&project_dir).await
}

/// Convenience runner executing [`run`] with project and all flags.
pub async fn pull(_config: &GlobalConfig, project: Option<&str>, all: bool) -> Result<()> {
    run(project.map(|s| s.to_string()), all).await
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_project_dir_none() {
        let dir = resolve_project_dir(None).expect("Should resolve current dir");
        assert!(dir.is_dir());
    }

    #[test]
    fn test_resolve_project_dir_some() {
        let temp = std::env::temp_dir();
        let resolved = resolve_project_dir(Some(temp.to_str().unwrap())).expect("Should resolve temp dir");
        assert_eq!(resolved, temp.canonicalize().unwrap_or(temp));
    }

    #[test]
    fn test_discover_registered_projects_empty() {
        // Without projects.json created, should return an empty or valid list
        let projects = discover_registered_projects();
        let _ = projects;
    }

    #[test]
    fn test_get_secret_key_from_env() {
        let (pub_key, sec_key) = ctx_core::crypto::generate_keypair();
        unsafe {
            std::env::set_var("CTX_SECRET_KEY", &sec_key);
        }
        let retrieved = get_secret_key().expect("Should read from CTX_SECRET_KEY");
        assert_eq!(retrieved, sec_key);
        unsafe {
            std::env::remove_var("CTX_SECRET_KEY");
        }
        let _ = pub_key;
    }
}
