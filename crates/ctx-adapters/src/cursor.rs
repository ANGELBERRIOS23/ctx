//! Cursor AI code editor adapter implementation for `ctx`.
//!
//! Implements [`AgentAdapter`] for the Cursor AI code editor (a VS Code fork).
//! Cursor stores its configuration and extensions in `~/.cursor/`, Composer
//! chat history in SQLite databases (`state.vscdb` / `*.vscdb` / `*.sqlite`),
//! and project-level AI instructions in `.cursorrules`.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Connection, Row, SqliteConnection};

use ctx_core::handoff::{Decision, Handoff, TaskItem, TaskStatus};

use crate::adapter::{AdapterError, AgentAdapter, Result};

/// A single message or bubble within a Cursor Composer session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposerMessage {
    /// Unique identifier of the message bubble, if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    /// Role of the sender (e.g. `"user"`, `"assistant"`, `"ai"`).
    pub role: String,

    /// Text content of the message.
    pub text: String,

    /// Timestamp of when the message was sent or created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<DateTime<Utc>>,
}

/// A Cursor Composer session extracted from local SQLite databases.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComposerSession {
    /// Unique identifier of the composer session (UUID or string key).
    pub id: String,

    /// Title or prompt summary of the composer session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    /// Creation timestamp of the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,

    /// Last update timestamp of the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,

    /// Associated workspace or project directory path, if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,

    /// Ordered list of messages (bubbles) within this composer session.
    #[serde(default)]
    pub messages: Vec<ComposerMessage>,
}

impl ComposerSession {
    /// Creates a new, empty [`ComposerSession`] with the specified identifier.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: None,
            created_at: None,
            updated_at: None,
            workspace_path: None,
            messages: Vec::new(),
        }
    }

    /// Computes the effective timestamp for this session, using `updated_at`,
    /// `created_at`, the most recent message timestamp, or the current time.
    pub fn effective_timestamp(&self) -> DateTime<Utc> {
        self.updated_at
            .or(self.created_at)
            .or_else(|| self.messages.iter().rev().find_map(|m| m.timestamp))
            .unwrap_or_else(Utc::now)
    }
}

/// Agent adapter for the Cursor AI code editor.
///
/// Implements [`AgentAdapter`] to detect Cursor installations, generate `.cursorrules`
/// instruction files, extract handoff state from SQLite composer session databases,
/// search composer history, and launch Cursor.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CursorAdapter {
    /// Optional custom path to the Cursor configuration directory (defaults to `~/.cursor`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_home: Option<PathBuf>,
}

impl CursorAdapter {
    /// Creates a new default [`CursorAdapter`].
    pub fn new() -> Self {
        Self { cursor_home: None }
    }

    /// Creates a new [`CursorAdapter`] with a custom Cursor home directory.
    pub fn with_cursor_home(cursor_home: impl Into<PathBuf>) -> Self {
        Self {
            cursor_home: Some(cursor_home.into()),
        }
    }

    /// Resolves the root Cursor configuration directory (`~/.cursor` or custom override).
    pub fn resolve_cursor_home(&self) -> Option<PathBuf> {
        self.cursor_home
            .clone()
            .or_else(|| dirs::home_dir().map(|h| h.join(".cursor")))
    }

    /// Returns `true` if the `cursor` binary exists in `PATH` or `~/.cursor/` exists.
    pub fn is_installed(&self) -> bool {
        if let Some(home) = self.resolve_cursor_home() {
            if home.is_dir() {
                return true;
            }
        }
        Self::check_binary_exists("cursor")
    }

    /// Returns the path to the instructions file (`.cursorrules`) within the target project directory.
    pub fn get_instruction_path(&self, project_dir: &Path) -> PathBuf {
        project_dir.join(".cursorrules")
    }

    /// Formats handoff state into a structured `.cursorrules` markdown instruction document.
    pub fn format_instructions(&self, handoff: &Handoff) -> String {
        let mut md = String::new();

        // 1. Project name
        let project_name = if handoff.project_name.trim().is_empty() {
            "Unnamed Project"
        } else {
            handoff.project_name.trim()
        };
        md.push_str(&format!("# Project: {}\n\n", project_name));

        // 2. Last Session metadata
        md.push_str("## Last Session\n\n");
        let source_agent = if handoff.source_agent.trim().is_empty() {
            "cursor"
        } else {
            handoff.source_agent.trim()
        };
        let source_machine = if handoff.source_machine.trim().is_empty() {
            "unknown"
        } else {
            handoff.source_machine.trim()
        };
        md.push_str(&format!("- **Agent:** {}\n", source_agent));
        md.push_str(&format!("- **Machine:** {}\n", source_machine));
        md.push_str(&format!(
            "- **Timestamp:** {}\n",
            handoff.created_at.to_rfc3339()
        ));
        if !handoff.git_branch.trim().is_empty() {
            md.push_str(&format!("- **Git Branch:** {}\n", handoff.git_branch.trim()));
        }
        if !handoff.git_commit.trim().is_empty() {
            md.push_str(&format!("- **Git Commit:** {}\n", handoff.git_commit.trim()));
        }
        md.push('\n');

        // 3. Current State / Summary
        md.push_str("## Current State\n\n");
        if handoff.summary.trim().is_empty() {
            md.push_str("No summary provided.\n\n");
        } else {
            md.push_str(handoff.summary.trim());
            md.push_str("\n\n");
        }

        // 4. Completed Items
        md.push_str("## Completed Items\n\n");
        if handoff.completed.is_empty() {
            md.push_str("None recorded.\n\n");
        } else {
            for item in &handoff.completed {
                md.push_str(&item.to_markdown());
                md.push('\n');
            }
            md.push('\n');
        }

        // 5. In Progress Items
        md.push_str("## In Progress Items\n\n");
        if handoff.in_progress.is_empty() {
            md.push_str("None recorded.\n\n");
        } else {
            for item in &handoff.in_progress {
                md.push_str(&item.to_markdown());
                md.push('\n');
            }
            md.push('\n');
        }

        // 6. Pending Items
        md.push_str("## Pending Items\n\n");
        if handoff.pending.is_empty() {
            md.push_str("None recorded.\n\n");
        } else {
            for item in &handoff.pending {
                md.push_str(&item.to_markdown());
                md.push('\n');
            }
            md.push('\n');
        }

        // 7. Decisions
        md.push_str("## Decisions\n\n");
        if handoff.decisions.is_empty() {
            md.push_str("None recorded.\n\n");
        } else {
            for decision in &handoff.decisions {
                md.push_str(&decision.to_markdown());
                md.push('\n');
            }
            md.push('\n');
        }

        // 8. Blockers (if any)
        if !handoff.blockers.is_empty() {
            md.push_str("## Blockers\n\n");
            for blocker in &handoff.blockers {
                md.push_str(&format!("- {}\n", blocker.trim()));
            }
            md.push('\n');
        }

        // 9. Security Directive
        md.push_str("## Security Directive\n\n");
        md.push_str(
            "- **CRITICAL:** Never store secrets in plaintext. Never log secret values. Encrypt before network transit.\n",
        );
        md.push_str(
            "- All sensitive credentials, API keys, and access tokens must be accessed strictly through environment variables provided by the ctx vault.\n",
        );
        md.push_str(
            "- Never print or commit decrypted secret values into project files, configuration, or transcripts.\n",
        );

        md
    }

    /// Discovers candidate Cursor SQLite database files (`*.vscdb`, `*.sqlite`, `*.db`, `*.sqlite3`).
    ///
    /// Scans the configured `cursor_home` (or `~/.cursor`), along with standard Cursor storage
    /// directories (`globalStorage` and `workspaceStorage`).
    pub fn find_all_sqlite_databases(&self) -> Vec<PathBuf> {
        let mut candidates = Vec::new();

        // 1. Search in configured cursor_home or ~/.cursor/
        if let Some(home) = self.resolve_cursor_home() {
            if home.is_dir() {
                collect_sqlite_files(&home, &mut candidates, 0, 5);
            }
        }

        // 2. Search in standard OS Cursor application directories
        for standard_dir in self.resolve_standard_storage_dirs() {
            if standard_dir.is_dir() {
                collect_sqlite_files(&standard_dir, &mut candidates, 0, 4);
            }
        }

        // Deduplicate and filter existing non-empty files
        candidates.sort();
        candidates.dedup();
        candidates.retain(|path| {
            fs::metadata(path)
                .map(|m| m.is_file() && m.len() > 0)
                .unwrap_or(false)
        });

        candidates
    }

    /// Retrieves all Composer sessions found across local SQLite storage,
    /// sorted in descending order of effective timestamp (newest first).
    pub fn get_all_sessions(&self) -> Result<Vec<ComposerSession>> {
        let dbs = self.find_all_sqlite_databases();
        if dbs.is_empty() {
            let fallback_dir = self
                .resolve_cursor_home()
                .unwrap_or_else(|| PathBuf::from("~/.cursor"));
            return Err(AdapterError::NoSessionFound(fallback_dir));
        }

        block_on_async(move || {
            Box::pin(async move {
                let mut sessions_map: std::collections::HashMap<String, ComposerSession> =
                    std::collections::HashMap::new();

                for db in dbs {
                    if let Ok(loaded) = load_sessions_from_db_async(&db).await {
                        for s in loaded {
                            merge_session(&mut sessions_map, s);
                        }
                    }
                }

                let mut result: Vec<ComposerSession> = sessions_map.into_values().collect();
                result.sort_by(|a, b| b.effective_timestamp().cmp(&a.effective_timestamp()));
                Ok(result)
            })
        })
    }

    /// Retrieves all Composer sessions found in a specific directory or SQLite database file.
    pub fn get_all_sessions_from_dir(&self, dir: &Path) -> Result<Vec<ComposerSession>> {
        let mut dbs = Vec::new();
        if dir.is_file() {
            dbs.push(dir.to_path_buf());
        } else if dir.is_dir() {
            collect_sqlite_files(dir, &mut dbs, 0, 5);
        }

        if dbs.is_empty() {
            return Err(AdapterError::NoSessionFound(dir.to_path_buf()));
        }

        block_on_async(move || {
            Box::pin(async move {
                let mut sessions_map: std::collections::HashMap<String, ComposerSession> =
                    std::collections::HashMap::new();

                for db in dbs {
                    if let Ok(loaded) = load_sessions_from_db_async(&db).await {
                        for s in loaded {
                            merge_session(&mut sessions_map, s);
                        }
                    }
                }

                let mut result: Vec<ComposerSession> = sessions_map.into_values().collect();
                result.sort_by(|a, b| b.effective_timestamp().cmp(&a.effective_timestamp()));
                Ok(result)
            })
        })
    }

    /// Searches local Composer sessions whose title, workspace path, or message content
    /// matches the given query. The search is case-insensitive.
    pub fn search_sessions(&self, query: &str) -> Result<Vec<ComposerSession>> {
        let all_sessions = self.get_all_sessions()?;
        Ok(filter_sessions_by_query(all_sessions, query))
    }

    /// Searches Composer sessions in a specific directory or database for the given query.
    pub fn search_sessions_in_dir(&self, dir: &Path, query: &str) -> Result<Vec<ComposerSession>> {
        let sessions = self.get_all_sessions_from_dir(dir)?;
        Ok(filter_sessions_by_query(sessions, query))
    }

    /// Extracts a project handoff snapshot by reading Composer sessions from a specific directory or file.
    pub fn extract_handoff_from_dir(&self, dir: &Path, project_dir: &Path) -> Result<Handoff> {
        let sessions = self.get_all_sessions_from_dir(dir)?;
        if sessions.is_empty() {
            return Err(AdapterError::NoSessionFound(dir.to_path_buf()));
        }

        let selected = self
            .select_best_session(&sessions, Some(project_dir))
            .ok_or_else(|| AdapterError::NoSessionFound(dir.to_path_buf()))?;

        self.extract_handoff_from_session(selected, project_dir)
    }

    /// Converts a [`ComposerSession`] into a [`Handoff`] snapshot for the specified project.
    pub fn extract_handoff_from_session(
        &self,
        session: &ComposerSession,
        project_dir: &Path,
    ) -> Result<Handoff> {
        let project_name = project_dir
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
            .or_else(|| {
                session.workspace_path.as_ref().and_then(|wp| {
                    Path::new(wp)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(|s| s.to_string())
                })
            })
            .or_else(|| session.title.clone())
            .unwrap_or_else(|| "cursor-project".to_string());

        // Locate primary text candidate: preference given to assistant messages with state
        let assistant_msg = session
            .messages
            .iter()
            .rev()
            .find(|m| m.role == "assistant" || m.role == "ai");

        let primary_text = if let Some(msg) = assistant_msg {
            &msg.text
        } else if let Some(last_msg) = session.messages.last() {
            &last_msg.text
        } else if let Some(ref title) = session.title {
            title
        } else {
            "Session captured from Cursor Composer"
        };

        let mut parsed_state = parse_markdown_state(primary_text);

        // Aggregate tasks, decisions, and blockers from all messages in the session
        for msg in &session.messages {
            let msg_state = parse_markdown_state(&msg.text);
            for item in msg_state.completed {
                if !parsed_state.completed.iter().any(|x| x.description == item.description) {
                    parsed_state.completed.push(item);
                }
            }
            for item in msg_state.in_progress {
                if !parsed_state.in_progress.iter().any(|x| x.description == item.description) {
                    parsed_state.in_progress.push(item);
                }
            }
            for item in msg_state.pending {
                if !parsed_state.pending.iter().any(|x| x.description == item.description) {
                    parsed_state.pending.push(item);
                }
            }
            for item in msg_state.decisions {
                if !parsed_state.decisions.iter().any(|x| x.what == item.what) {
                    parsed_state.decisions.push(item);
                }
            }
            for item in msg_state.blockers {
                if !parsed_state.blockers.contains(&item) {
                    parsed_state.blockers.push(item);
                }
            }
        }

        let summary = if !parsed_state.summary.is_empty() {
            parsed_state.summary
        } else if let Some(ref title) = session.title {
            title.clone()
        } else {
            "Session captured from Cursor Composer".to_string()
        };

        let (git_branch, git_commit) = detect_git_info(project_dir);

        let mut handoff = Handoff::for_project(project_name);
        handoff.created_at = session.effective_timestamp();
        handoff.source_agent = "cursor".to_string();
        handoff.source_machine = get_hostname();
        handoff.git_branch = git_branch;
        handoff.git_commit = git_commit;
        handoff.summary = summary;
        handoff.completed = parsed_state.completed;
        handoff.in_progress = parsed_state.in_progress;
        handoff.pending = parsed_state.pending;
        handoff.decisions = parsed_state.decisions;
        handoff.blockers = parsed_state.blockers;

        if session.messages.len() > 1 {
            handoff.notes = Some(format!(
                "Cursor session '{}' with {} message(s).",
                session.id,
                session.messages.len()
            ));
        }

        Ok(handoff)
    }

    /// Selects the best matching session for a target project directory.
    pub fn select_best_session<'a>(
        &self,
        sessions: &'a [ComposerSession],
        project_dir: Option<&Path>,
    ) -> Option<&'a ComposerSession> {
        if sessions.is_empty() {
            return None;
        }

        if let Some(proj_dir) = project_dir {
            let proj_name = proj_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_lowercase();
            let proj_str = proj_dir.to_string_lossy().to_lowercase();

            // 1. Exact workspace path match
            for s in sessions {
                if let Some(ref wp) = s.workspace_path {
                    let wp_lower = wp.to_lowercase();
                    if wp_lower == proj_str || (!proj_name.is_empty() && wp_lower.ends_with(&proj_name)) {
                        return Some(s);
                    }
                }
            }

            // 2. Title or content match on project name
            if !proj_name.is_empty() {
                for s in sessions {
                    if let Some(ref t) = s.title {
                        if t.to_lowercase().contains(&proj_name) {
                            return Some(s);
                        }
                    }
                }
            }
        }

        // Fallback: return the newest session
        sessions.first()
    }

    /// Checks whether an executable exists in PATH using `which` (Unix) or `where` (Windows),
    /// or by direct filesystem inspection of directories in `PATH`.
    fn check_binary_exists(binary: &str) -> bool {
        let which_cmd = if cfg!(target_os = "windows") {
            "where"
        } else {
            "which"
        };
        if let Ok(output) = std::process::Command::new(which_cmd).arg(binary).output() {
            if output.status.success() {
                return true;
            }
        }

        // Fallback: inspect PATH directories directly
        if let Some(path_var) = std::env::var_os("PATH") {
            for mut dir in std::env::split_paths(&path_var) {
                dir.push(binary);
                if dir.is_file() {
                    return true;
                }
                #[cfg(target_os = "windows")]
                {
                    dir.set_extension("exe");
                    if dir.is_file() {
                        return true;
                    }
                    dir.set_extension("cmd");
                    if dir.is_file() {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Resolves standard OS-specific storage directories for Cursor.
    fn resolve_standard_storage_dirs(&self) -> Vec<PathBuf> {
        let mut dirs_list = Vec::new();

        #[cfg(target_os = "macos")]
        {
            if let Some(home) = dirs::home_dir() {
                let base = home
                    .join("Library")
                    .join("Application Support")
                    .join("Cursor")
                    .join("User");
                dirs_list.push(base.join("globalStorage"));
                dirs_list.push(base.join("workspaceStorage"));
            }
        }

        #[cfg(target_os = "windows")]
        {
            if let Some(config_dir) = dirs::config_dir() {
                let base = config_dir.join("Cursor").join("User");
                dirs_list.push(base.join("globalStorage"));
                dirs_list.push(base.join("workspaceStorage"));
            }
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            if let Some(config_dir) = dirs::config_dir() {
                let base = config_dir.join("Cursor").join("User");
                dirs_list.push(base.join("globalStorage"));
                dirs_list.push(base.join("workspaceStorage"));
            }
        }

        dirs_list
    }
}

impl AgentAdapter for CursorAdapter {
    fn name(&self) -> &str {
        "cursor"
    }

    fn detect_installed(&self) -> bool {
        self.is_installed()
    }

    fn instruction_path(&self, project_dir: &Path) -> PathBuf {
        self.get_instruction_path(project_dir)
    }

    fn generate_instructions(&self, handoff: &Handoff) -> String {
        self.format_instructions(handoff)
    }

    fn extract_handoff(&self, project_dir: &Path) -> Result<Handoff> {
        let sessions = self.get_all_sessions()?;
        if sessions.is_empty() {
            let home_dir = self
                .resolve_cursor_home()
                .unwrap_or_else(|| PathBuf::from("~/.cursor"));
            return Err(AdapterError::NoSessionFound(home_dir));
        }

        let selected = self
            .select_best_session(&sessions, Some(project_dir))
            .ok_or_else(|| {
                let home_dir = self
                    .resolve_cursor_home()
                    .unwrap_or_else(|| PathBuf::from("~/.cursor"));
                AdapterError::NoSessionFound(home_dir)
            })?;

        self.extract_handoff_from_session(selected, project_dir)
    }

    fn launch_command(&self) -> &str {
        "cursor"
    }
}

/// Recursively scans a directory for SQLite database files (`*.vscdb`, `*.sqlite`, `*.db`, `*.sqlite3`).
fn collect_sqlite_files(dir: &Path, files: &mut Vec<PathBuf>, current_depth: usize, max_depth: usize) {
    if current_depth > max_depth || !dir.is_dir() {
        return;
    }

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_sqlite_files(&path, files, current_depth + 1, max_depth);
        } else if path.is_file() {
            let extension = path
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("")
                .to_lowercase();
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_lowercase();

            if extension == "vscdb"
                || extension == "sqlite"
                || extension == "sqlite3"
                || extension == "db"
                || filename.ends_with(".vscdb")
            {
                files.push(path);
            }
        }
    }
}

/// Inspects workspace storage directory for `workspace.json` indicating project directory path.
fn extract_workspace_path_from_db_dir(db_path: &Path) -> Option<String> {
    let parent = db_path.parent()?;
    let workspace_json = parent.join("workspace.json");
    if workspace_json.is_file() {
        if let Ok(content) = fs::read_to_string(&workspace_json) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(folder) = val.get("folder").and_then(|f| f.as_str()) {
                    let cleaned = folder.strip_prefix("file://").unwrap_or(folder);
                    return Some(cleaned.to_string());
                }
            }
        }
    }
    None
}

/// Reads and parses Composer sessions from a single SQLite database file.
pub async fn load_sessions_from_db_async(db_path: &Path) -> Result<Vec<ComposerSession>> {
    if !db_path.is_file() {
        return Ok(Vec::new());
    }

    let options = SqliteConnectOptions::new()
        .filename(db_path)
        .read_only(true);

    let mut conn = match SqliteConnection::connect_with(&options).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                "Failed to open SQLite database at {}: {}",
                db_path.display(),
                e
            );
            return Ok(Vec::new());
        }
    };

    let table_rows = match sqlx::query("SELECT name FROM sqlite_master WHERE type='table'")
        .fetch_all(&mut conn)
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(
                "Failed to inspect SQLite database tables at {}: {}",
                db_path.display(),
                e
            );
            return Ok(Vec::new());
        }
    };

    let table_names: Vec<String> = table_rows
        .into_iter()
        .filter_map(|r| r.try_get::<String, _>(0).ok())
        .collect();

    let mut sessions_map: std::collections::HashMap<String, ComposerSession> =
        std::collections::HashMap::new();
    let default_workspace = extract_workspace_path_from_db_dir(db_path);

    // 1. Process `cursorDiskKV` table
    if table_names.iter().any(|t| t == "cursorDiskKV") {
        if let Ok(rows) = sqlx::query(
            "SELECT key, value FROM cursorDiskKV WHERE key LIKE 'composerData:%' OR key LIKE 'bubbleId:%'",
        )
        .fetch_all(&mut conn)
        .await
        {
            parse_cursor_disk_kv_rows(rows, &mut sessions_map, default_workspace.as_deref());
        }
    }

    // 2. Process `ItemTable` table
    if table_names.iter().any(|t| t == "ItemTable") {
        if let Ok(rows) = sqlx::query(
            "SELECT key, value FROM ItemTable WHERE key = 'composer.composerData' OR key LIKE '%composer%' OR key LIKE '%chatdata%'",
        )
        .fetch_all(&mut conn)
        .await
        {
            parse_item_table_rows(rows, &mut sessions_map, default_workspace.as_deref());
        }
    }

    // 3. Process generic `composer_sessions` table (if present in custom/test schemas)
    if table_names.iter().any(|t| t == "composer_sessions") {
        if let Ok(rows) = sqlx::query(
            "SELECT id, title, data, created_at, updated_at FROM composer_sessions",
        )
        .fetch_all(&mut conn)
        .await
        {
            parse_composer_sessions_table_rows(
                rows,
                &mut sessions_map,
                default_workspace.as_deref(),
            );
        }
    }

    let mut result: Vec<ComposerSession> = sessions_map.into_values().collect();
    result.sort_by(|a, b| b.effective_timestamp().cmp(&a.effective_timestamp()));
    Ok(result)
}

/// Parses rows from `cursorDiskKV` table.
fn parse_cursor_disk_kv_rows(
    rows: Vec<sqlx::sqlite::SqliteRow>,
    sessions_map: &mut std::collections::HashMap<String, ComposerSession>,
    default_workspace: Option<&str>,
) {
    let mut pending_bubbles: std::collections::HashMap<String, Vec<ComposerMessage>> =
        std::collections::HashMap::new();

    for row in rows {
        let key: String = match row.try_get("key") {
            Ok(k) => k,
            Err(_) => continue,
        };

        let value_str = match extract_row_value_as_string(&row) {
            Some(s) => s,
            None => continue,
        };

        if let Some(composer_id) = key.strip_prefix("composerData:") {
            let composer_id = composer_id.trim();
            if composer_id.is_empty() {
                continue;
            }

            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&value_str) {
                let session = sessions_map
                    .entry(composer_id.to_string())
                    .or_insert_with(|| ComposerSession::new(composer_id));

                if session.title.is_none() {
                    session.title = val
                        .get("name")
                        .or_else(|| val.get("title"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                }

                if session.created_at.is_none() {
                    session.created_at = val.get("createdAt").and_then(parse_timestamp);
                }

                if session.updated_at.is_none() {
                    session.updated_at = val
                        .get("lastUpdatedAt")
                        .or_else(|| val.get("updatedAt"))
                        .and_then(parse_timestamp);
                }

                if session.workspace_path.is_none() {
                    session.workspace_path = val
                        .get("workspaceId")
                        .or_else(|| val.get("projectPath"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .or_else(|| default_workspace.map(|s| s.to_string()));
                }

                if let Some(arr) = val
                    .get("conversation")
                    .or_else(|| val.get("bubbles"))
                    .or_else(|| val.get("messages"))
                    .and_then(|v| v.as_array())
                {
                    for item in arr {
                        if let Some(msg) = parse_bubble_json(item) {
                            if !session.messages.iter().any(|m| m.text == msg.text) {
                                session.messages.push(msg);
                            }
                        }
                    }
                }
            }
        } else if let Some(rest) = key.strip_prefix("bubbleId:") {
            let parts: Vec<&str> = rest.split(':').collect();
            let (composer_id, bubble_id) = if parts.len() >= 2 {
                (parts[0], Some(parts[1]))
            } else if parts.len() == 1 {
                ("", Some(parts[0]))
            } else {
                ("", None)
            };

            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&value_str) {
                let resolved_cid = if !composer_id.is_empty() {
                    composer_id.to_string()
                } else if let Some(cid) = val.get("composerId").and_then(|v| v.as_str()) {
                    cid.to_string()
                } else {
                    continue;
                };

                if let Some(mut msg) = parse_bubble_json(&val) {
                    if msg.id.is_none() {
                        msg.id = bubble_id.map(|s| s.to_string());
                    }
                    pending_bubbles
                        .entry(resolved_cid)
                        .or_default()
                        .push(msg);
                }
            }
        }
    }

    // Merge bubbles into their corresponding sessions
    for (cid, bubbles) in pending_bubbles {
        let session = sessions_map
            .entry(cid.clone())
            .or_insert_with(|| ComposerSession::new(cid));

        if session.workspace_path.is_none() {
            session.workspace_path = default_workspace.map(|s| s.to_string());
        }

        for b in bubbles {
            if !session
                .messages
                .iter()
                .any(|m| m.id.is_some() && m.id == b.id && m.text == b.text)
            {
                session.messages.push(b);
            }
        }
    }
}

/// Parses rows from `ItemTable` table.
fn parse_item_table_rows(
    rows: Vec<sqlx::sqlite::SqliteRow>,
    sessions_map: &mut std::collections::HashMap<String, ComposerSession>,
    default_workspace: Option<&str>,
) {
    for row in rows {
        let value_str = match extract_row_value_as_string(&row) {
            Some(s) => s,
            None => continue,
        };

        let val: serde_json::Value = match serde_json::from_str(&value_str) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if let Some(arr) = val.get("allComposers").and_then(|v| v.as_array()) {
            for item in arr {
                parse_composer_object(item, sessions_map, default_workspace);
            }
        } else if let Some(arr) = val.as_array() {
            for item in arr {
                parse_composer_object(item, sessions_map, default_workspace);
            }
        } else if val.get("composerId").is_some() || val.get("id").is_some() {
            parse_composer_object(&val, sessions_map, default_workspace);
        } else if let Some(obj) = val.as_object() {
            for (_, sub_val) in obj {
                if sub_val.get("composerId").is_some() || sub_val.get("conversation").is_some() {
                    parse_composer_object(sub_val, sessions_map, default_workspace);
                }
            }
        }
    }
}

/// Parses an individual composer JSON object into a [`ComposerSession`].
fn parse_composer_object(
    val: &serde_json::Value,
    sessions_map: &mut std::collections::HashMap<String, ComposerSession>,
    default_workspace: Option<&str>,
) {
    let composer_id = val
        .get("composerId")
        .or_else(|| val.get("id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let session = sessions_map
        .entry(composer_id.clone())
        .or_insert_with(|| ComposerSession::new(composer_id));

    if session.title.is_none() {
        session.title = val
            .get("name")
            .or_else(|| val.get("title"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
    }

    if session.created_at.is_none() {
        session.created_at = val.get("createdAt").and_then(parse_timestamp);
    }

    if session.updated_at.is_none() {
        session.updated_at = val
            .get("lastUpdatedAt")
            .or_else(|| val.get("updatedAt"))
            .and_then(parse_timestamp);
    }

    if session.workspace_path.is_none() {
        session.workspace_path = val
            .get("workspaceId")
            .or_else(|| val.get("projectPath"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| default_workspace.map(|s| s.to_string()));
    }

    if let Some(arr) = val
        .get("conversation")
        .or_else(|| val.get("bubbles"))
        .or_else(|| val.get("messages"))
        .and_then(|v| v.as_array())
    {
        for item in arr {
            if let Some(msg) = parse_bubble_json(item) {
                if !session.messages.iter().any(|m| m.text == msg.text) {
                    session.messages.push(msg);
                }
            }
        }
    }
}

/// Parses a bubble / message JSON element into a [`ComposerMessage`].
fn parse_bubble_json(val: &serde_json::Value) -> Option<ComposerMessage> {
    if let Some(s) = val.as_str() {
        return Some(ComposerMessage {
            id: None,
            role: "user".to_string(),
            text: s.to_string(),
            timestamp: None,
        });
    }

    let id = val
        .get("bubbleId")
        .or_else(|| val.get("id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let role = val
        .get("type")
        .or_else(|| val.get("role"))
        .and_then(|v| v.as_str())
        .unwrap_or("assistant")
        .to_string();

    let text_val = val
        .get("text")
        .or_else(|| val.get("rawText"))
        .or_else(|| val.get("content"))
        .unwrap_or(val);

    let text = extract_text_content(text_val);
    if text.trim().is_empty() {
        return None;
    }

    let timestamp = val.get("createdAt").and_then(parse_timestamp);

    Some(ComposerMessage {
        id,
        role,
        text,
        timestamp,
    })
}

/// Parses rows from custom or legacy `composer_sessions` table.
fn parse_composer_sessions_table_rows(
    rows: Vec<sqlx::sqlite::SqliteRow>,
    sessions_map: &mut std::collections::HashMap<String, ComposerSession>,
    default_workspace: Option<&str>,
) {
    for row in rows {
        let id: String = match row.try_get("id") {
            Ok(i) => i,
            Err(_) => continue,
        };

        let title: Option<String> = row.try_get("title").ok();
        let data_str: Option<String> = row.try_get("data").ok();
        let created_at: Option<DateTime<Utc>> = row.try_get("created_at").ok();
        let updated_at: Option<DateTime<Utc>> = row.try_get("updated_at").ok();

        let session = sessions_map
            .entry(id.clone())
            .or_insert_with(|| ComposerSession::new(id));

        if session.title.is_none() {
            session.title = title;
        }
        if session.created_at.is_none() {
            session.created_at = created_at;
        }
        if session.updated_at.is_none() {
            session.updated_at = updated_at;
        }
        if session.workspace_path.is_none() {
            session.workspace_path = default_workspace.map(|s| s.to_string());
        }

        if let Some(ds) = data_str {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&ds) {
                if let Some(arr) = val
                    .get("messages")
                    .or_else(|| val.get("bubbles"))
                    .and_then(|v| v.as_array())
                {
                    for item in arr {
                        if let Some(msg) = parse_bubble_json(item) {
                            session.messages.push(msg);
                        }
                    }
                } else if let Some(msg) = parse_bubble_json(&val) {
                    session.messages.push(msg);
                }
            } else if !ds.trim().is_empty() {
                session.messages.push(ComposerMessage {
                    id: None,
                    role: "assistant".to_string(),
                    text: ds,
                    timestamp: updated_at.or(created_at),
                });
            }
        }
    }
}

/// Helper function to extract a string value from an SQLite row value column (text or blob).
fn extract_row_value_as_string(row: &sqlx::sqlite::SqliteRow) -> Option<String> {
    if let Ok(s) = row.try_get::<String, _>("value") {
        Some(s)
    } else if let Ok(bytes) = row.try_get::<Vec<u8>, _>("value") {
        Some(String::from_utf8_lossy(&bytes).into_owned())
    } else {
        None
    }
}

/// Parses timestamp representations from JSON (epoch millis, epoch seconds, or RFC3339 strings).
fn parse_timestamp(val: &serde_json::Value) -> Option<DateTime<Utc>> {
    match val {
        serde_json::Value::Number(n) => {
            if let Some(millis) = n.as_i64() {
                if millis > 100_000_000_000 {
                    DateTime::from_timestamp_millis(millis)
                } else {
                    DateTime::from_timestamp(millis, 0)
                }
            } else {
                None
            }
        }
        serde_json::Value::String(s) => DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
            .or_else(|| {
                if let Ok(millis) = s.parse::<i64>() {
                    if millis > 100_000_000_000 {
                        DateTime::from_timestamp_millis(millis)
                    } else {
                        DateTime::from_timestamp(millis, 0)
                    }
                } else {
                    None
                }
            }),
        _ => None,
    }
}

/// Recursively extracts text from a JSON value.
fn extract_text_content(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => {
            let mut parts = Vec::new();
            for item in arr {
                let text = extract_text_content(item);
                if !text.trim().is_empty() {
                    parts.push(text);
                }
            }
            parts.join("\n")
        }
        serde_json::Value::Object(map) => {
            if let Some(text_val) = map.get("text").and_then(|v| v.as_str()) {
                text_val.to_string()
            } else if let Some(content_val) = map.get("content") {
                extract_text_content(content_val)
            } else if let Some(summary_val) = map.get("summary").and_then(|v| v.as_str()) {
                summary_val.to_string()
            } else if let Some(raw_val) = map.get("rawText").and_then(|v| v.as_str()) {
                raw_val.to_string()
            } else {
                String::new()
            }
        }
        _ => String::new(),
    }
}

/// Merges an incoming session into the existing sessions map.
fn merge_session(
    sessions_map: &mut std::collections::HashMap<String, ComposerSession>,
    incoming: ComposerSession,
) {
    let entry = sessions_map
        .entry(incoming.id.clone())
        .or_insert_with(|| ComposerSession::new(&incoming.id));

    if entry.title.is_none() {
        entry.title = incoming.title;
    }
    if entry.workspace_path.is_none() {
        entry.workspace_path = incoming.workspace_path;
    }
    if entry.created_at.is_none() {
        entry.created_at = incoming.created_at;
    }
    if incoming.updated_at.is_some() {
        entry.updated_at = incoming.updated_at;
    }

    for msg in incoming.messages {
        if !entry.messages.iter().any(|m| m.text == msg.text) {
            entry.messages.push(msg);
        }
    }
}

/// Filters a list of sessions by a search query string.
fn filter_sessions_by_query(
    sessions: Vec<ComposerSession>,
    query: &str,
) -> Vec<ComposerSession> {
    let query_trimmed = query.trim().to_lowercase();
    if query_trimmed.is_empty() {
        return sessions;
    }

    sessions
        .into_iter()
        .filter(|session| {
            if session.id.to_lowercase().contains(&query_trimmed) {
                return true;
            }
            if let Some(ref title) = session.title {
                if title.to_lowercase().contains(&query_trimmed) {
                    return true;
                }
            }
            if let Some(ref wp) = session.workspace_path {
                if wp.to_lowercase().contains(&query_trimmed) {
                    return true;
                }
            }
            session.messages.iter().any(|msg| {
                msg.text.to_lowercase().contains(&query_trimmed)
                    || msg.role.to_lowercase().contains(&query_trimmed)
            })
        })
        .collect()
}

/// Extracted markdown state components.
#[derive(Debug, Default)]
struct ExtractedMarkdownState {
    summary: String,
    completed: Vec<TaskItem>,
    in_progress: Vec<TaskItem>,
    pending: Vec<TaskItem>,
    decisions: Vec<Decision>,
    blockers: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum SectionContext {
    None,
    Summary,
    Completed,
    InProgress,
    Pending,
    Decisions,
    Blockers,
    Other,
}

/// Parses markdown text into task items, decisions, blockers, and summary.
fn parse_markdown_state(text: &str) -> ExtractedMarkdownState {
    let mut state = ExtractedMarkdownState::default();
    let mut current_section = SectionContext::None;
    let mut summary_lines = Vec::new();
    let mut general_lines = Vec::new();
    let mut in_code_block = false;

    for raw_line in text.lines() {
        let trimmed = raw_line.trim();

        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block {
            continue;
        }

        if trimmed.starts_with('#') {
            let heading_text = trimmed.trim_start_matches('#').trim().to_lowercase();
            if heading_text.contains("summary") || heading_text.contains("current state") {
                current_section = SectionContext::Summary;
            } else if heading_text.contains("completed")
                || heading_text.contains("done")
                || heading_text.contains("finished")
            {
                current_section = SectionContext::Completed;
            } else if heading_text.contains("in progress")
                || heading_text.contains("in-progress")
                || heading_text.contains("working")
            {
                current_section = SectionContext::InProgress;
            } else if heading_text.contains("pending")
                || heading_text.contains("to do")
                || heading_text.contains("todo")
                || heading_text.contains("next step")
            {
                current_section = SectionContext::Pending;
            } else if heading_text.contains("decision") {
                current_section = SectionContext::Decisions;
            } else if heading_text.contains("blocker") || heading_text.contains("impediment") {
                current_section = SectionContext::Blockers;
            } else {
                current_section = SectionContext::Other;
            }
            continue;
        }

        // Parse markdown checkbox task if present
        if let Some(task) = parse_checkbox_task(trimmed) {
            match task.status {
                TaskStatus::Done => state.completed.push(task),
                TaskStatus::Partial => state.in_progress.push(task),
                TaskStatus::Pending => state.pending.push(task),
            }
            continue;
        }

        // Check for explicit Blocker / Decision prefix
        let unbulleted = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("+ "))
            .unwrap_or(trimmed)
            .trim();

        if let Some(rest) = unbulleted
            .strip_prefix("Blocker:")
            .or_else(|| unbulleted.strip_prefix("blocker:"))
            .or_else(|| unbulleted.strip_prefix("Blockers:"))
            .or_else(|| unbulleted.strip_prefix("blockers:"))
        {
            let b = rest.trim();
            if !b.is_empty() {
                state.blockers.push(b.to_string());
                continue;
            }
        }

        if let Some(rest) = unbulleted
            .strip_prefix("Decision:")
            .or_else(|| unbulleted.strip_prefix("decision:"))
            .or_else(|| unbulleted.strip_prefix("Decisions:"))
            .or_else(|| unbulleted.strip_prefix("decisions:"))
        {
            let d = rest.trim();
            if !d.is_empty() {
                state.decisions.push(parse_decision(d));
                continue;
            }
        }

        // Handle section context
        match current_section {
            SectionContext::Summary => {
                if !trimmed.is_empty() {
                    summary_lines.push(trimmed);
                }
            }
            SectionContext::Completed => {
                if let Some(desc) = extract_list_item(trimmed) {
                    state.completed.push(parse_task_item(desc, TaskStatus::Done));
                }
            }
            SectionContext::InProgress => {
                if let Some(desc) = extract_list_item(trimmed) {
                    state
                        .in_progress
                        .push(parse_task_item(desc, TaskStatus::Partial));
                }
            }
            SectionContext::Pending => {
                if let Some(desc) = extract_list_item(trimmed) {
                    state.pending.push(parse_task_item(desc, TaskStatus::Pending));
                }
            }
            SectionContext::Decisions => {
                let item = extract_list_item(trimmed).unwrap_or(trimmed);
                if !item.is_empty() {
                    state.decisions.push(parse_decision(item));
                }
            }
            SectionContext::Blockers => {
                let item = extract_list_item(trimmed).unwrap_or(trimmed);
                if !item.is_empty() {
                    state.blockers.push(item.to_string());
                }
            }
            SectionContext::None => {
                if !trimmed.is_empty() {
                    general_lines.push(trimmed);
                }
            }
            SectionContext::Other => {}
        }
    }

    if !summary_lines.is_empty() {
        state.summary = summary_lines.join("\n");
    } else if !general_lines.is_empty() {
        state.summary = general_lines.join("\n");
    }

    state
}

/// Parses a checkbox task from a line like `- [x] Task`.
fn parse_checkbox_task(line: &str) -> Option<TaskItem> {
    let stripped = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| line.strip_prefix("+ "))?;

    let trimmed = stripped.trim_start();
    if let Some(rest) = trimmed
        .strip_prefix("[x] ")
        .or_else(|| trimmed.strip_prefix("[X] "))
    {
        Some(parse_task_item(rest, TaskStatus::Done))
    } else if let Some(rest) = trimmed
        .strip_prefix("[-] ")
        .or_else(|| trimmed.strip_prefix("[/] "))
    {
        Some(parse_task_item(rest, TaskStatus::Partial))
    } else if let Some(rest) = trimmed.strip_prefix("[ ] ") {
        Some(parse_task_item(rest, TaskStatus::Pending))
    } else {
        None
    }
}

/// Extracts list item text after a bullet or numeric marker.
fn extract_list_item(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))
        .or_else(|| {
            if let Some(dot_idx) = trimmed.find(". ") {
                let num_part = &trimmed[..dot_idx];
                if !num_part.is_empty() && num_part.chars().all(|c| c.is_ascii_digit()) {
                    return Some(&trimmed[dot_idx + 2..]);
                }
            }
            None
        })
        .map(|s| s.trim())
}

/// Parses a task item string, extracting supplementary details separated by `" - "`.
fn parse_task_item(desc: &str, status: TaskStatus) -> TaskItem {
    let trimmed = desc.trim();
    if let Some((title, detail)) = trimmed.split_once(" - ") {
        TaskItem::new(title.trim(), status).with_detail(detail.trim())
    } else {
        TaskItem::new(trimmed, status)
    }
}

/// Parses a decision line formatted as `**what**: why`, `what: why`, or `what - why`.
fn parse_decision(desc: &str) -> Decision {
    let trimmed = desc.trim();
    let trimmed = trimmed
        .strip_prefix("Decision:")
        .or_else(|| trimmed.strip_prefix("decision:"))
        .or_else(|| trimmed.strip_prefix("Decisions:"))
        .or_else(|| trimmed.strip_prefix("decisions:"))
        .unwrap_or(trimmed)
        .trim();

    if let Some(rest) = trimmed.strip_prefix("**") {
        if let Some((what, after_what)) = rest.split_once("**") {
            let why = after_what
                .trim()
                .strip_prefix(':')
                .or_else(|| after_what.trim().strip_prefix('-'))
                .unwrap_or(after_what.trim())
                .trim();
            return Decision::now(what.trim(), why);
        }
    }
    if let Some((what, why)) = trimmed.split_once(':') {
        Decision::now(what.trim(), why.trim())
    } else if let Some((what, why)) = trimmed.split_once(" - ") {
        Decision::now(what.trim(), why.trim())
    } else {
        Decision::now(trimmed, "")
    }
}

/// Inspects git repository metadata in `project_dir` for branch and commit.
fn detect_git_info(project_dir: &Path) -> (String, String) {
    let branch = std::process::Command::new("git")
        .arg("-C")
        .arg(project_dir)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    let commit = std::process::Command::new("git")
        .arg("-C")
        .arg(project_dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    (branch, commit)
}

/// Resolves the local system hostname safely without panicking.
fn get_hostname() -> String {
    if let Ok(host) = std::env::var("HOSTNAME") {
        if !host.trim().is_empty() {
            return host.trim().to_string();
        }
    }
    if let Ok(host) = std::env::var("HOST") {
        if !host.trim().is_empty() {
            return host.trim().to_string();
        }
    }
    if let Ok(output) = std::process::Command::new("hostname").output() {
        if output.status.success() {
            let host = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !host.is_empty() {
                return host;
            }
        }
    }
    "unknown-host".to_string()
}

/// Helper to execute an async future synchronously from any thread or runtime context.
fn block_on_async<F, T>(future_factory: F) -> Result<T>
where
    F: FnOnce() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T>> + Send>>
        + Send
        + 'static,
    T: Send + 'static,
{
    let handle = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| AdapterError::Other(format!("Failed to build Tokio runtime: {e}")))?;
        rt.block_on(future_factory())
    });

    handle
        .join()
        .map_err(|_| AdapterError::Other("Worker thread panicked while executing task".to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("{prefix}_{nanos}_{counter}"));
        fs::create_dir_all(&dir).expect("Failed to create unique temp dir");
        dir
    }

    #[test]
    fn test_adapter_metadata_and_instructions() {
        let adapter = CursorAdapter::new();
        assert_eq!(adapter.name(), "cursor");
        assert_eq!(adapter.launch_command(), "cursor");

        let project_dir = Path::new("/workspace/my-app");
        assert_eq!(
            adapter.instruction_path(project_dir),
            PathBuf::from("/workspace/my-app/.cursorrules")
        );
        assert_eq!(
            adapter.get_instruction_path(project_dir),
            PathBuf::from("/workspace/my-app/.cursorrules")
        );

        let mut handoff = Handoff::for_project("ctx")
            .with_summary("Refactoring agent adapters for Cursor")
            .with_source("Cursor", "dev-machine-1")
            .with_git("feature/cursor", "abcdef12");

        handoff.add_completed(
            TaskItem::done("Implement CursorAdapter").with_detail("Added full SQLite support"),
        );
        handoff.add_in_progress(TaskItem::partial("Write unit tests"));
        handoff.add_pending(TaskItem::pending("Integrate with CLI sync"));
        handoff.add_decision(Decision::new(
            "Use .cursorrules",
            "Cursor reads project rules from .cursorrules",
            Utc::now(),
        ));
        handoff.add_blocker("Waiting for API key confirmation");

        let instructions = adapter.generate_instructions(&handoff);

        assert!(instructions.contains("# Project: ctx"));
        assert!(instructions.contains("## Last Session"));
        assert!(instructions.contains("- **Agent:** Cursor"));
        assert!(instructions.contains("- **Machine:** dev-machine-1"));
        assert!(instructions.contains("- **Git Branch:** feature/cursor"));
        assert!(instructions.contains("- **Git Commit:** abcdef12"));
        assert!(instructions.contains("## Current State"));
        assert!(instructions.contains("Refactoring agent adapters for Cursor"));
        assert!(instructions.contains("## Completed Items"));
        assert!(instructions.contains("- [x] Implement CursorAdapter - Added full SQLite support"));
        assert!(instructions.contains("## In Progress Items"));
        assert!(instructions.contains("- [-] Write unit tests"));
        assert!(instructions.contains("## Pending Items"));
        assert!(instructions.contains("- [ ] Integrate with CLI sync"));
        assert!(instructions.contains("## Decisions"));
        assert!(instructions.contains("- **Use .cursorrules**: Cursor reads project rules from .cursorrules"));
        assert!(instructions.contains("## Blockers"));
        assert!(instructions.contains("- Waiting for API key confirmation"));
        assert!(instructions.contains("## Security Directive"));
        assert!(instructions.contains("Never store secrets in plaintext. Never log secret values. Encrypt before network transit."));
    }

    #[test]
    fn test_detect_installed() {
        let temp_dir = unique_temp_dir("cursor_detect");
        let adapter = CursorAdapter::with_cursor_home(&temp_dir);
        assert!(adapter.detect_installed());

        let nonexistent = temp_dir.join("nonexistent_sub_dir");
        let adapter_nonexistent = CursorAdapter::with_cursor_home(&nonexistent);
        // Returns true only if 'cursor' binary is in system PATH, or false otherwise
        let _ = adapter_nonexistent.detect_installed();

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_extract_handoff_from_cursor_sqlite_item_table() {
        let temp_dir = unique_temp_dir("cursor_sqlite_item_table");
        let db_path = temp_dir.join("state.vscdb");

        // Create SQLite database and ItemTable synchronously inside runtime
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        rt.block_on(async {
            let options = SqliteConnectOptions::new()
                .filename(&db_path)
                .create_if_missing(true);
            let mut conn = SqliteConnection::connect_with(&options)
                .await
                .expect("Failed to connect to SQLite test DB");

            sqlx::query("CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value TEXT);")
                .execute(&mut conn)
                .await
                .expect("Failed to create ItemTable");

            let composer_json = serde_json::json!({
                "allComposers": [
                    {
                        "composerId": "sess-test-1",
                        "name": "Database Migration Feature",
                        "createdAt": 1725450000000_i64,
                        "lastUpdatedAt": 1725453600000_i64,
                        "workspaceId": "/workspace/my-app",
                        "conversation": [
                            {
                                "bubbleId": "b1",
                                "type": "user",
                                "text": "Please migrate database to PostgreSQL"
                            },
                            {
                                "bubbleId": "b2",
                                "type": "ai",
                                "text": "## Summary\nMigrated database to PostgreSQL\n\n## Tasks\n- [x] Schema migration - Added tables\n- [-] Index tuning\n- [ ] Load tests\n\n## Decisions\n- **Database**: Use PostgreSQL 16\n\nBlocker: Awaiting staging DB credentials"
                            }
                        ]
                    }
                ]
            });

            sqlx::query("INSERT INTO ItemTable (key, value) VALUES (?, ?);")
                .bind("composer.composerData")
                .bind(composer_json.to_string())
                .execute(&mut conn)
                .await
                .expect("Failed to insert composerData");
        });

        let adapter = CursorAdapter::with_cursor_home(&temp_dir);
        let project_dir = Path::new("/workspace/my-app");

        let handoff = adapter
            .extract_handoff_from_dir(&temp_dir, project_dir)
            .expect("Handoff extraction must succeed");

        assert_eq!(handoff.project_name, "my-app");
        assert_eq!(handoff.source_agent, "cursor");
        assert_eq!(handoff.summary, "Migrated database to PostgreSQL");

        assert_eq!(handoff.completed.len(), 1);
        assert_eq!(handoff.completed[0].description, "Schema migration");
        assert_eq!(
            handoff.completed[0].detail.as_deref(),
            Some("Added tables")
        );

        assert_eq!(handoff.in_progress.len(), 1);
        assert_eq!(handoff.in_progress[0].description, "Index tuning");

        assert_eq!(handoff.pending.len(), 1);
        assert_eq!(handoff.pending[0].description, "Load tests");

        assert_eq!(handoff.decisions.len(), 1);
        assert_eq!(handoff.decisions[0].what, "Database");
        assert_eq!(handoff.decisions[0].why, "Use PostgreSQL 16");

        assert_eq!(handoff.blockers, vec!["Awaiting staging DB credentials"]);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_extract_handoff_from_cursor_disk_kv() {
        let temp_dir = unique_temp_dir("cursor_sqlite_disk_kv");
        let db_path = temp_dir.join("state.vscdb");

        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        rt.block_on(async {
            let options = SqliteConnectOptions::new()
                .filename(&db_path)
                .create_if_missing(true);
            let mut conn = SqliteConnection::connect_with(&options)
                .await
                .expect("Failed to connect to SQLite test DB");

            sqlx::query("CREATE TABLE cursorDiskKV (key TEXT PRIMARY KEY, value TEXT);")
                .execute(&mut conn)
                .await
                .expect("Failed to create cursorDiskKV table");

            let composer_data = serde_json::json!({
                "composerId": "sess-kv-100",
                "name": "Auth Overhaul",
                "createdAt": 1725460000000_i64,
                "lastUpdatedAt": 1725465000000_i64,
                "workspaceId": "/workspace/auth-service"
            });

            let bubble1 = serde_json::json!({
                "bubbleId": "bubble-1",
                "type": "user",
                "text": "Upgrade to JWT tokens"
            });

            let bubble2 = serde_json::json!({
                "bubbleId": "bubble-2",
                "type": "ai",
                "text": "## Summary\nUpgraded authentication system to JWT\n\n- [x] Issue access tokens\n- [-] Refresh token endpoint\n- [ ] Argon2 password hashing\n\nDecision: JWT Secret - Store in ctx vault"
            });

            sqlx::query("INSERT INTO cursorDiskKV (key, value) VALUES (?, ?);")
                .bind("composerData:sess-kv-100")
                .bind(composer_data.to_string())
                .execute(&mut conn)
                .await
                .expect("Failed to insert composerData");

            sqlx::query("INSERT INTO cursorDiskKV (key, value) VALUES (?, ?);")
                .bind("bubbleId:sess-kv-100:bubble-1")
                .bind(bubble1.to_string())
                .execute(&mut conn)
                .await
                .expect("Failed to insert bubble-1");

            sqlx::query("INSERT INTO cursorDiskKV (key, value) VALUES (?, ?);")
                .bind("bubbleId:sess-kv-100:bubble-2")
                .bind(bubble2.to_string())
                .execute(&mut conn)
                .await
                .expect("Failed to insert bubble-2");
        });

        let adapter = CursorAdapter::with_cursor_home(&temp_dir);
        let project_dir = Path::new("/workspace/auth-service");

        let handoff = adapter
            .extract_handoff_from_dir(&temp_dir, project_dir)
            .expect("Handoff extraction from cursorDiskKV must succeed");

        assert_eq!(handoff.project_name, "auth-service");
        assert_eq!(handoff.source_agent, "cursor");
        assert_eq!(handoff.summary, "Upgraded authentication system to JWT");

        assert_eq!(handoff.completed.len(), 1);
        assert_eq!(handoff.completed[0].description, "Issue access tokens");

        assert_eq!(handoff.in_progress.len(), 1);
        assert_eq!(handoff.in_progress[0].description, "Refresh token endpoint");

        assert_eq!(handoff.pending.len(), 1);
        assert_eq!(handoff.pending[0].description, "Argon2 password hashing");

        assert_eq!(handoff.decisions.len(), 1);
        assert_eq!(handoff.decisions[0].what, "JWT Secret");
        assert_eq!(handoff.decisions[0].why, "Store in ctx vault");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_search_sessions() {
        let temp_dir = unique_temp_dir("cursor_search_sessions");
        let db_path = temp_dir.join("state.vscdb");

        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        rt.block_on(async {
            let options = SqliteConnectOptions::new()
                .filename(&db_path)
                .create_if_missing(true);
            let mut conn = SqliteConnection::connect_with(&options)
                .await
                .expect("Failed to connect to SQLite test DB");

            sqlx::query("CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value TEXT);")
                .execute(&mut conn)
                .await
                .expect("Failed to create ItemTable");

            let data = serde_json::json!({
                "allComposers": [
                    {
                        "composerId": "sess-alpha",
                        "name": "Frontend Navigation Redesign",
                        "createdAt": 1725400000000_i64,
                        "conversation": [
                            {
                                "bubbleId": "b1",
                                "type": "user",
                                "text": "Fix responsive navbar styling"
                            }
                        ]
                    },
                    {
                        "composerId": "sess-beta",
                        "name": "Backend Payment Gateway",
                        "createdAt": 1725450000000_i64,
                        "conversation": [
                            {
                                "bubbleId": "b2",
                                "type": "user",
                                "text": "Integrate Stripe webhooks"
                            }
                        ]
                    }
                ]
            });

            sqlx::query("INSERT INTO ItemTable (key, value) VALUES (?, ?);")
                .bind("composer.composerData")
                .bind(data.to_string())
                .execute(&mut conn)
                .await
                .expect("Failed to insert composerData");
        });

        let adapter = CursorAdapter::with_cursor_home(&temp_dir);

        // Search for 'Stripe' (in message text)
        let stripe_results = adapter
            .search_sessions_in_dir(&temp_dir, "Stripe")
            .expect("Search for Stripe should succeed");
        assert_eq!(stripe_results.len(), 1);
        assert_eq!(stripe_results[0].id, "sess-beta");

        // Search for 'navigation' (in title, case-insensitive)
        let nav_results = adapter
            .search_sessions_in_dir(&temp_dir, "navigation")
            .expect("Search for navigation should succeed");
        assert_eq!(nav_results.len(), 1);
        assert_eq!(nav_results[0].id, "sess-alpha");

        // Empty query returns all sessions
        let all_results = adapter
            .search_sessions_in_dir(&temp_dir, "")
            .expect("Empty query should return all sessions");
        assert_eq!(all_results.len(), 2);

        // Nonexistent query returns empty vec
        let empty_results = adapter
            .search_sessions_in_dir(&temp_dir, "nonexistent-query-xyz")
            .expect("Search should succeed");
        assert!(empty_results.is_empty());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_no_session_found_error() {
        let temp_dir = unique_temp_dir("cursor_empty_dir");
        let adapter = CursorAdapter::with_cursor_home(&temp_dir);

        let err = adapter
            .extract_handoff(Path::new("/workspace/some-project"))
            .unwrap_err();

        match err {
            AdapterError::NoSessionFound(path) => {
                assert_eq!(path, temp_dir);
            }
            other => panic!("Expected NoSessionFound error, got {:?}", other),
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
