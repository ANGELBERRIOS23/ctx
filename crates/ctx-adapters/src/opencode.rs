//! OpenCode agent adapter implementation for `ctx`.
//!
//! Implements the [`AgentAdapter`] interface for the OpenCode AI coding assistant.
//! OpenCode persists its conversation history and state into a SQLite database
//! (`opencode.db`) located in the platform data directory (`~/.local/share/opencode/`
//! on Linux, or platform equivalent on macOS/Windows).
//!
//! This adapter provides:
//! - Local installation detection (binary in `PATH` or data directory).
//! - Project instruction path resolution (`.opencode/instructions.md`).
//! - Instruction file generation formatting handoff state and security directives.
//! - Handoff context extraction from `opencode.db` (`session`, `message`, and `part` tables),
//!   supporting compaction markers (`tail_start_id`).
//! - Full-text search over sessions and messages (`search_sessions`).
//! - Listing recent sessions filtered by age (`list_recent_sessions`).

use std::path::{Path, PathBuf};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use sqlx::Row;

use ctx_core::handoff::{Decision, Handoff, TaskItem, TaskStatus};
use crate::adapter::{AdapterError, AgentAdapter, Result};

/// Represents a session matching a search query in OpenCode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMatch {
    /// Unique identifier of the session.
    pub session_id: String,
    /// Title of the matching session.
    pub title: String,
    /// Timestamp when the session was created or updated, if available.
    pub date: Option<DateTime<Utc>>,
    /// Contextual preview snippet showing the query match.
    pub preview: String,
}

impl SessionMatch {
    /// Returns the unique session ID.
    pub fn id(&self) -> &str {
        &self.session_id
    }
}

/// Summary information for a recent OpenCode session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
    /// Unique identifier of the session.
    pub id: String,
    /// Human-readable title of the session.
    pub title: String,
    /// Timestamp when the session was created or updated, if available.
    pub date: Option<DateTime<Utc>>,
    /// Number of messages recorded in this session.
    pub message_count: usize,
}

impl SessionInfo {
    /// Returns the unique session ID.
    pub fn session_id(&self) -> &str {
        &self.id
    }
}

/// Adapter for the OpenCode AI coding agent.
///
/// Implements [`AgentAdapter`] to detect OpenCode installation, format instructions in
/// `.opencode/instructions.md`, extract handoff context from `opencode.db` (SQLite),
/// search past sessions, and launch the `opencode` command.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCodeAdapter {
    /// Optional custom path to the OpenCode data directory (defaults to `~/.local/share/opencode`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_dir: Option<PathBuf>,
    /// Optional custom path directly to `opencode.db`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db_path: Option<PathBuf>,
}

impl OpenCodeAdapter {
    /// Creates a new default [`OpenCodeAdapter`].
    pub fn new() -> Self {
        Self {
            data_dir: None,
            db_path: None,
        }
    }

    /// Creates a new [`OpenCodeAdapter`] with a custom data directory.
    pub fn with_data_dir(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: Some(data_dir.into()),
            db_path: None,
        }
    }

    /// Creates a new [`OpenCodeAdapter`] with an explicit database file path.
    pub fn with_db_path(db_path: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: None,
            db_path: Some(db_path.into()),
        }
    }

    /// Resolves the directory where OpenCode stores its application data.
    ///
    /// Checks explicit configuration, standard XDG data directory (`~/.local/share/opencode`),
    /// macOS Application Support, and Windows AppData locations.
    pub fn resolve_data_dir(&self) -> Option<PathBuf> {
        if let Some(ref dir) = self.data_dir {
            return Some(dir.clone());
        }
        if let Some(ref db) = self.db_path {
            return db.parent().map(|p| p.to_path_buf());
        }

        let mut candidates = Vec::new();

        // 1. Check dirs::data_dir()
        if let Some(dir) = dirs::data_dir() {
            candidates.push(dir.join("opencode"));
        }

        // 2. Check dirs::data_local_dir()
        if let Some(dir) = dirs::data_local_dir() {
            let p = dir.join("opencode");
            if !candidates.contains(&p) {
                candidates.push(p);
            }
        }

        // 3. Fallback: ~/.local/share/opencode (common for CLI tools across OSes)
        if let Some(home) = dirs::home_dir() {
            let p = home.join(".local").join("share").join("opencode");
            if !candidates.contains(&p) {
                candidates.push(p);
            }
        }

        // Return first candidate that exists as a directory
        for cand in &candidates {
            if cand.is_dir() {
                return Some(cand.clone());
            }
        }

        candidates.into_iter().next()
    }

    /// Resolves the path to the OpenCode SQLite database (`opencode.db`).
    pub fn resolve_db_path(&self) -> Option<PathBuf> {
        if let Some(ref db) = self.db_path {
            return Some(db.clone());
        }
        if let Some(ref dir) = self.data_dir {
            return Some(dir.join("opencode.db"));
        }

        let mut candidates = Vec::new();

        // 1. Data dir
        if let Some(dir) = dirs::data_dir() {
            candidates.push(dir.join("opencode").join("opencode.db"));
        }

        // 2. Local data dir
        if let Some(dir) = dirs::data_local_dir() {
            let p = dir.join("opencode").join("opencode.db");
            if !candidates.contains(&p) {
                candidates.push(p);
            }
        }

        // 3. ~/.local/share/opencode/opencode.db
        if let Some(home) = dirs::home_dir() {
            let p = home
                .join(".local")
                .join("share")
                .join("opencode")
                .join("opencode.db");
            if !candidates.contains(&p) {
                candidates.push(p);
            }
        }

        // Return first candidate file that exists
        for cand in &candidates {
            if cand.is_file() {
                return Some(cand.clone());
            }
        }

        candidates.into_iter().next()
    }

    /// Checks if OpenCode is installed by verifying binary existence in `PATH` or data directory existence.
    pub fn is_installed(&self) -> bool {
        if let Some(dir) = self.resolve_data_dir() {
            if dir.is_dir() {
                return true;
            }
        }
        if let Some(db) = self.resolve_db_path() {
            if db.is_file() {
                return true;
            }
        }
        check_binary_exists("opencode")
    }

    /// Returns the path to the instructions markdown file for OpenCode.
    pub fn get_instruction_path(&self, project_dir: &Path) -> PathBuf {
        project_dir.join(".opencode").join("instructions.md")
    }

    /// Formats handoff state into a comprehensive `.opencode/instructions.md` document.
    pub fn format_instructions(&self, handoff: &Handoff) -> String {
        let mut md = String::new();

        // 1. Project Header
        let project_name = if handoff.project_name.trim().is_empty() {
            "Unnamed Project"
        } else {
            handoff.project_name.trim()
        };
        md.push_str(&format!("# Project: {}\n\n", project_name));

        // 2. Last Session
        md.push_str("## Last Session\n\n");
        let source_agent = if handoff.source_agent.trim().is_empty() {
            "opencode"
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

        // 3. Current State
        md.push_str("## Current State\n\n");
        if handoff.summary.trim().is_empty() {
            md.push_str("No summary provided.\n\n");
        } else {
            md.push_str(handoff.summary.trim());
            md.push_str("\n\n");
        }

        // 4. Completed Tasks
        md.push_str("## Completed Tasks\n\n");
        if handoff.completed.is_empty() {
            md.push_str("None recorded.\n\n");
        } else {
            for item in &handoff.completed {
                md.push_str(&item.to_markdown());
                md.push('\n');
            }
            md.push('\n');
        }

        // 5. In Progress Tasks
        md.push_str("## In Progress Tasks\n\n");
        if handoff.in_progress.is_empty() {
            md.push_str("None recorded.\n\n");
        } else {
            for item in &handoff.in_progress {
                md.push_str(&item.to_markdown());
                md.push('\n');
            }
            md.push('\n');
        }

        // 6. Pending Tasks
        md.push_str("## Pending Tasks\n\n");
        if handoff.pending.is_empty() {
            md.push_str("None recorded.\n\n");
        } else {
            for item in &handoff.pending {
                md.push_str(&item.to_markdown());
                md.push('\n');
            }
            md.push('\n');
        }

        // 7. Architectural Decisions
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

        // 8. Blockers
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
            "- Do NOT store, commit, or hardcode secrets, tokens, or credentials in project files or transcripts.\n",
        );
        md.push_str(
            "- All sensitive credentials must be accessed strictly through environment variables provided by the ctx vault.\n",
        );
        md.push_str(
            "- Never print decrypted secret values into session transcripts or chat outputs.\n",
        );

        md
    }

    /// Synchronously extracts a project handoff snapshot from a specific `opencode.db` database.
    pub fn extract_handoff_from_db(
        &self,
        db_path: &Path,
        project_dir: Option<&Path>,
    ) -> Result<Handoff> {
        let db_path_buf = db_path.to_path_buf();
        let proj_dir_buf = project_dir.map(|p| p.to_path_buf());
        let adapter = self.clone();

        block_on(async move {
            adapter
                .extract_handoff_from_db_async(&db_path_buf, proj_dir_buf.as_deref())
                .await
        })
    }

    /// Asynchronously extracts a project handoff snapshot from a specific `opencode.db` database.
    pub async fn extract_handoff_from_db_async(
        &self,
        db_path: &Path,
        project_dir: Option<&Path>,
    ) -> Result<Handoff> {
        let pool = open_pool_read_only(db_path).await?;

        if !table_exists(&pool, "session").await? {
            return Err(AdapterError::NoSessionFound(db_path.to_path_buf()));
        }

        let session_cols = get_table_columns(&pool, "session").await?;
        let has_col = |name: &str| session_cols.iter().any(|c| c.eq_ignore_ascii_case(name));

        let id_col = if has_col("id") { "id" } else { "rowid" };
        let title_col = if has_col("title") { "title" } else { "''" };

        let date_col = if has_col("updated_at") {
            Some("updated_at")
        } else if has_col("created_at") {
            Some("created_at")
        } else if has_col("date") {
            Some("date")
        } else if has_col("timestamp") {
            Some("timestamp")
        } else {
            None
        };

        let dir_col = if has_col("directory") {
            Some("directory")
        } else if has_col("cwd") {
            Some("cwd")
        } else if has_col("path") {
            Some("path")
        } else if has_col("project_path") {
            Some("project_path")
        } else {
            None
        };

        // Select targeted session
        let mut selected_session: Option<(String, String, Option<DateTime<Utc>>)> = None;

        if let Some(pdir) = project_dir {
            let proj_name = pdir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            let proj_path = pdir.to_string_lossy().into_owned();

            // 1. Try directory match if directory column exists
            if let Some(dir) = dir_col {
                let order_clause = match date_col {
                    Some(dc) => format!("ORDER BY s.{dc} DESC"),
                    None => "ORDER BY s.rowid DESC".to_string(),
                };
                let date_select = match date_col {
                    Some(dc) => format!("s.{dc} as date_val"),
                    None => "NULL as date_val".to_string(),
                };
                let q = format!(
                    "SELECT s.{id_col} as sess_id, s.{title_col} as sess_title, {date_select} \
                     FROM session s \
                     WHERE s.{dir} = ? OR s.{dir} LIKE ? \
                     {order_clause} LIMIT 1"
                );
                let pattern = format!("%{}%", proj_name);
                if let Ok(Some(row)) = sqlx::query(&q)
                    .bind(&proj_path)
                    .bind(&pattern)
                    .fetch_optional(&pool)
                    .await
                {
                    let s_id: String = row.try_get("sess_id").unwrap_or_default();
                    let s_title: String = row.try_get("sess_title").unwrap_or_default();
                    let s_date = extract_row_datetime(&row, "date_val");
                    selected_session = Some((s_id, s_title, s_date));
                }
            }

            // 2. Try title match if no directory match was found
            if selected_session.is_none() && !proj_name.is_empty() && has_col("title") {
                let order_clause = match date_col {
                    Some(dc) => format!("ORDER BY s.{dc} DESC"),
                    None => "ORDER BY s.rowid DESC".to_string(),
                };
                let date_select = match date_col {
                    Some(dc) => format!("s.{dc} as date_val"),
                    None => "NULL as date_val".to_string(),
                };
                let q = format!(
                    "SELECT s.{id_col} as sess_id, s.{title_col} as sess_title, {date_select} \
                     FROM session s \
                     WHERE s.{title_col} LIKE ? \
                     {order_clause} LIMIT 1"
                );
                let pattern = format!("%{}%", proj_name);
                if let Ok(Some(row)) = sqlx::query(&q)
                    .bind(&pattern)
                    .fetch_optional(&pool)
                    .await
                {
                    let s_id: String = row.try_get("sess_id").unwrap_or_default();
                    let s_title: String = row.try_get("sess_title").unwrap_or_default();
                    let s_date = extract_row_datetime(&row, "date_val");
                    selected_session = Some((s_id, s_title, s_date));
                }
            }
        }

        // 3. Fallback: most recent session overall
        if selected_session.is_none() {
            let order_clause = match date_col {
                Some(dc) => format!("ORDER BY s.{dc} DESC"),
                None => "ORDER BY s.rowid DESC".to_string(),
            };
            let date_select = match date_col {
                Some(dc) => format!("s.{dc} as date_val"),
                None => "NULL as date_val".to_string(),
            };
            let q = format!(
                "SELECT s.{id_col} as sess_id, s.{title_col} as sess_title, {date_select} \
                 FROM session s \
                 {order_clause} LIMIT 1"
            );
            let row = sqlx::query(&q)
                .fetch_optional(&pool)
                .await
                .map_err(|e| AdapterError::Other(format!("Failed to query most recent session: {e}")))?;

            if let Some(row) = row {
                let s_id: String = row.try_get("sess_id").unwrap_or_default();
                let s_title: String = row.try_get("sess_title").unwrap_or_default();
                let s_date = extract_row_datetime(&row, "date_val");
                selected_session = Some((s_id, s_title, s_date));
            }
        }

        let (session_id, session_title, session_date) = match selected_session {
            Some(s) => s,
            None => return Err(AdapterError::NoSessionFound(db_path.to_path_buf())),
        };

        // Check message and part tables
        let has_msg_table = table_exists(&pool, "message").await?;
        let has_part_table = table_exists(&pool, "part").await?;

        if !has_msg_table {
            return Err(AdapterError::ExtractionFailed(format!(
                "Message table not found in OpenCode database {}",
                db_path.display()
            )));
        }

        let msg_cols = get_table_columns(&pool, "message").await?;
        let msg_id_col = if msg_cols.iter().any(|c| c.eq_ignore_ascii_case("id")) {
            "id"
        } else {
            "rowid"
        };
        let msg_order_col = if msg_cols.iter().any(|c| c.eq_ignore_ascii_case("created_at")) {
            "created_at ASC"
        } else if msg_cols.iter().any(|c| c.eq_ignore_ascii_case("timestamp")) {
            "timestamp ASC"
        } else {
            "rowid ASC"
        };

        let msg_query = format!(
            "SELECT {msg_id_col} as m_id, role, content FROM message WHERE session_id = ? ORDER BY {msg_order_col}"
        );
        let msg_rows = sqlx::query(&msg_query)
            .bind(&session_id)
            .fetch_all(&pool)
            .await
            .map_err(|e| AdapterError::Other(format!("Failed to read messages: {e}")))?;

        let mut raw_messages: Vec<RawMessage> = msg_rows
            .into_iter()
            .map(|r| RawMessage {
                id: r.try_get("m_id").unwrap_or_default(),
                role: r.try_get("role").unwrap_or_default(),
                content: r.try_get("content").unwrap_or_default(),
            })
            .collect();

        // Inspect parts and look for compaction markers
        let mut compaction_summary: Option<String> = None;
        let mut compaction_tail_start_id: Option<String> = None;

        if has_part_table {
            let part_cols = get_table_columns(&pool, "part").await?;
            let has_tail_col = part_cols
                .iter()
                .any(|c| c.eq_ignore_ascii_case("tail_start_id"));

            let tail_select = if has_tail_col {
                "p.tail_start_id"
            } else {
                "NULL as tail_start_id"
            };

            let part_query = format!(
                "SELECT p.message_id, p.type as part_type, p.content as part_content, {tail_select} \
                 FROM part p \
                 JOIN message m ON p.message_id = m.{msg_id_col} \
                 WHERE m.session_id = ? \
                 ORDER BY p.rowid ASC"
            );

            if let Ok(part_rows) = sqlx::query(&part_query).bind(&session_id).fetch_all(&pool).await {
                for prow in part_rows {
                    let msg_id: String = prow.try_get("message_id").unwrap_or_default();
                    let part_type: String = prow.try_get("part_type").unwrap_or_default();
                    let part_content: String = prow.try_get("part_content").unwrap_or_default();
                    let tail_col_val: Option<String> = prow
                        .try_get::<String, _>("tail_start_id")
                        .ok()
                        .or_else(|| {
                            prow.try_get::<i64, _>("tail_start_id")
                                .ok()
                                .map(|i| i.to_string())
                        });

                    let is_compaction = part_type.eq_ignore_ascii_case("compaction")
                        || part_type.to_lowercase().contains("compact");

                    if is_compaction {
                        let mut resolved_tail = tail_col_val;
                        let mut resolved_summary = part_content.clone();

                        if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&part_content) {
                            if resolved_tail.is_none() {
                                if let Some(t_id) = json_val.get("tail_start_id") {
                                    if let Some(s) = t_id.as_str() {
                                        resolved_tail = Some(s.to_string());
                                    } else if let Some(n) = t_id.as_i64() {
                                        resolved_tail = Some(n.to_string());
                                    }
                                }
                            }
                            if let Some(s) = json_val.get("summary").and_then(|v| v.as_str()) {
                                resolved_summary = s.to_string();
                            } else if let Some(s) = json_val.get("content").and_then(|v| v.as_str()) {
                                resolved_summary = s.to_string();
                            }
                        }

                        if !resolved_summary.trim().is_empty() {
                            compaction_summary = Some(resolved_summary);
                        }
                        if resolved_tail.is_some() {
                            compaction_tail_start_id = resolved_tail;
                        }
                    } else {
                        // Supplement empty message content with text parts
                        if let Some(msg) = raw_messages.iter_mut().find(|m| m.id == msg_id) {
                            if msg.content.trim().is_empty() && !part_content.trim().is_empty() {
                                msg.content = part_content;
                            }
                        }
                    }
                }
            }
        }

        // Determine active messages starting from compaction tail_start_id
        let active_messages: &[RawMessage] = if let Some(ref tail_id) = compaction_tail_start_id {
            if let Some(pos) = raw_messages.iter().position(|m| m.id == *tail_id) {
                &raw_messages[pos..]
            } else {
                &raw_messages[..]
            }
        } else {
            &raw_messages[..]
        };

        if active_messages.is_empty() && compaction_summary.is_none() {
            return Err(AdapterError::ExtractionFailed(format!(
                "No valid messages or compaction summary found for OpenCode session '{}'",
                session_id
            )));
        }

        // Find assistant text and user text from the tail
        let mut last_assistant_text: Option<String> = None;
        let mut last_user_text: Option<String> = None;

        for msg in active_messages {
            let role = msg.role.to_lowercase();
            if role == "assistant" && !msg.content.trim().is_empty() {
                last_assistant_text = Some(msg.content.clone());
            } else if role == "user" && !msg.content.trim().is_empty() {
                last_user_text = Some(msg.content.clone());
            }
        }

        let primary_markdown = match (&last_assistant_text, &last_user_text, &compaction_summary) {
            (Some(assistant), _, _) => assistant.as_str(),
            (None, Some(user), _) => user.as_str(),
            (None, None, Some(compaction)) => compaction.as_str(),
            (None, None, None) => "",
        };

        let mut parsed_state = parse_markdown_state(primary_markdown);

        // Prepend or combine compaction summary if present
        if let Some(compaction) = compaction_summary {
            if parsed_state.summary.is_empty()
                || parsed_state
                    .summary
                    .starts_with("Session captured from OpenCode")
            {
                parsed_state.summary = compaction;
            } else if !parsed_state.summary.contains(compaction.trim()) {
                parsed_state.summary = format!("{}\n\n{}", compaction.trim(), parsed_state.summary.trim());
            }
        }

        // Resolve project name
        let resolved_project_name = project_dir
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| {
                if !session_title.trim().is_empty() {
                    session_title.trim().to_string()
                } else {
                    "unnamed".to_string()
                }
            });

        let mut handoff = Handoff::new();
        handoff.project_name = resolved_project_name;
        handoff.created_at = session_date.unwrap_or_else(Utc::now);
        handoff.source_agent = "opencode".to_string();
        handoff.source_machine = get_hostname();
        handoff.summary = parsed_state.summary;
        handoff.completed = parsed_state.completed;
        handoff.in_progress = parsed_state.in_progress;
        handoff.pending = parsed_state.pending;
        handoff.decisions = parsed_state.decisions;
        handoff.blockers = parsed_state.blockers;

        Ok(handoff)
    }

    /// Synchronously searches past OpenCode sessions and messages for the specified query string.
    pub fn search_sessions(&self, query: &str) -> Result<Vec<SessionMatch>> {
        let adapter = self.clone();
        let query_str = query.to_string();
        block_on(async move { adapter.search_sessions_async(&query_str).await })
    }

    /// Asynchronously searches past OpenCode sessions and messages for the specified query string.
    pub async fn search_sessions_async(&self, query: &str) -> Result<Vec<SessionMatch>> {
        let db_path = self.resolve_db_path().ok_or(AdapterError::MissingHomeDir)?;
        if !db_path.is_file() {
            return Err(AdapterError::NoSessionFound(db_path));
        }
        let pool = open_pool_read_only(&db_path).await?;

        if !table_exists(&pool, "session").await? {
            return Err(AdapterError::NoSessionFound(db_path));
        }

        let session_cols = get_table_columns(&pool, "session").await?;
        let has_msg_table = table_exists(&pool, "message").await?;

        let id_col = if session_cols.iter().any(|c| c.eq_ignore_ascii_case("id")) {
            "id"
        } else {
            "rowid"
        };
        let title_col = if session_cols.iter().any(|c| c.eq_ignore_ascii_case("title")) {
            "title"
        } else {
            "''"
        };
        let date_col = if session_cols.iter().any(|c| c.eq_ignore_ascii_case("updated_at")) {
            Some("updated_at")
        } else if session_cols.iter().any(|c| c.eq_ignore_ascii_case("created_at")) {
            Some("created_at")
        } else if session_cols.iter().any(|c| c.eq_ignore_ascii_case("date")) {
            Some("date")
        } else if session_cols.iter().any(|c| c.eq_ignore_ascii_case("timestamp")) {
            Some("timestamp")
        } else {
            None
        };

        let pattern = format!("%{}%", query);
        let mut matches = Vec::new();
        let mut seen_sessions = std::collections::HashSet::new();

        if has_msg_table {
            let date_select = match date_col {
                Some(dc) => format!("s.{dc} as date_val"),
                None => "NULL as date_val".to_string(),
            };
            let q = format!(
                "SELECT s.{id_col} as sess_id, s.{title_col} as sess_title, {date_select}, m.content as msg_content \
                 FROM session s \
                 LEFT JOIN message m ON s.{id_col} = m.session_id \
                 WHERE s.{title_col} LIKE ? OR m.content LIKE ? \
                 ORDER BY s.rowid DESC"
            );

            let rows = sqlx::query(&q)
                .bind(&pattern)
                .bind(&pattern)
                .fetch_all(&pool)
                .await
                .map_err(|e| AdapterError::Other(format!("Failed to search sessions: {e}")))?;

            for row in rows {
                let session_id: String = row.try_get("sess_id").unwrap_or_default();
                if session_id.is_empty() || seen_sessions.contains(&session_id) {
                    continue;
                }

                let title: String = row
                    .try_get("sess_title")
                    .unwrap_or_else(|_| "Untitled Session".to_string());
                let date = extract_row_datetime(&row, "date_val");
                let msg_content: Option<String> = row.try_get("msg_content").ok();

                let preview = if let Some(ref content) = msg_content {
                    extract_search_preview(content, query)
                } else {
                    format!("Title match: {}", title)
                };

                seen_sessions.insert(session_id.clone());
                matches.push(SessionMatch {
                    session_id,
                    title,
                    date,
                    preview,
                });
            }
        } else {
            let date_select = match date_col {
                Some(dc) => format!("{dc} as date_val"),
                None => "NULL as date_val".to_string(),
            };
            let q = format!(
                "SELECT {id_col} as sess_id, {title_col} as sess_title, {date_select} \
                 FROM session \
                 WHERE {title_col} LIKE ? \
                 ORDER BY rowid DESC"
            );

            let rows = sqlx::query(&q)
                .bind(&pattern)
                .fetch_all(&pool)
                .await
                .map_err(|e| AdapterError::Other(format!("Failed to search sessions: {e}")))?;

            for row in rows {
                let session_id: String = row.try_get("sess_id").unwrap_or_default();
                if session_id.is_empty() || seen_sessions.contains(&session_id) {
                    continue;
                }

                let title: String = row
                    .try_get("sess_title")
                    .unwrap_or_else(|_| "Untitled Session".to_string());
                let date = extract_row_datetime(&row, "date_val");
                let preview = format!("Title match: {}", title);

                seen_sessions.insert(session_id.clone());
                matches.push(SessionMatch {
                    session_id,
                    title,
                    date,
                    preview,
                });
            }
        }

        Ok(matches)
    }

    /// Synchronously lists recent OpenCode sessions ordered by date from the last `days` days.
    pub fn list_recent_sessions(&self, days: u32) -> Result<Vec<SessionInfo>> {
        let adapter = self.clone();
        block_on(async move { adapter.list_recent_sessions_async(days).await })
    }

    /// Asynchronously lists recent OpenCode sessions ordered by date from the last `days` days.
    pub async fn list_recent_sessions_async(&self, days: u32) -> Result<Vec<SessionInfo>> {
        let db_path = self.resolve_db_path().ok_or(AdapterError::MissingHomeDir)?;
        if !db_path.is_file() {
            return Err(AdapterError::NoSessionFound(db_path));
        }
        let pool = open_pool_read_only(&db_path).await?;

        if !table_exists(&pool, "session").await? {
            return Err(AdapterError::NoSessionFound(db_path));
        }

        let session_cols = get_table_columns(&pool, "session").await?;
        let has_msg_table = table_exists(&pool, "message").await?;

        let id_col = if session_cols.iter().any(|c| c.eq_ignore_ascii_case("id")) {
            "id"
        } else {
            "rowid"
        };
        let title_col = if session_cols.iter().any(|c| c.eq_ignore_ascii_case("title")) {
            "title"
        } else {
            "''"
        };
        let date_col = if session_cols.iter().any(|c| c.eq_ignore_ascii_case("updated_at")) {
            Some("updated_at")
        } else if session_cols.iter().any(|c| c.eq_ignore_ascii_case("created_at")) {
            Some("created_at")
        } else if session_cols.iter().any(|c| c.eq_ignore_ascii_case("date")) {
            Some("date")
        } else if session_cols.iter().any(|c| c.eq_ignore_ascii_case("timestamp")) {
            Some("timestamp")
        } else {
            None
        };

        let date_select = match date_col {
            Some(dc) => format!("s.{dc} as date_val"),
            None => "NULL as date_val".to_string(),
        };
        let date_group = match date_col {
            Some(dc) => format!(", s.{dc}"),
            None => String::new(),
        };
        let order_by = match date_col {
            Some(dc) => format!("ORDER BY s.{dc} DESC"),
            None => "ORDER BY s.rowid DESC".to_string(),
        };

        let q = if has_msg_table {
            format!(
                "SELECT s.{id_col} as sess_id, s.{title_col} as sess_title, {date_select}, COUNT(m.rowid) as msg_count \
                 FROM session s \
                 LEFT JOIN message m ON s.{id_col} = m.session_id \
                 GROUP BY s.{id_col}, s.{title_col}{date_group} \
                 {order_by}"
            )
        } else {
            format!(
                "SELECT s.{id_col} as sess_id, s.{title_col} as sess_title, {date_select}, 0 as msg_count \
                 FROM session s \
                 GROUP BY s.{id_col}, s.{title_col}{date_group} \
                 {order_by}"
            )
        };

        let rows = sqlx::query(&q)
            .fetch_all(&pool)
            .await
            .map_err(|e| AdapterError::Other(format!("Failed to list sessions: {e}")))?;

        let cutoff = Utc::now() - Duration::days(days as i64);
        let mut sessions = Vec::new();

        for row in rows {
            let id: String = row.try_get("sess_id").unwrap_or_default();
            let title: String = row
                .try_get("sess_title")
                .unwrap_or_else(|_| "Untitled Session".to_string());
            let date = extract_row_datetime(&row, "date_val");
            let msg_count: i64 = row.try_get("msg_count").unwrap_or(0);

            if days > 0 {
                if let Some(d) = date {
                    if d < cutoff {
                        continue;
                    }
                }
            }

            sessions.push(SessionInfo {
                id,
                title,
                date,
                message_count: msg_count as usize,
            });
        }

        Ok(sessions)
    }
}

impl AgentAdapter for OpenCodeAdapter {
    fn name(&self) -> &str {
        "opencode"
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
        let db_path = self.resolve_db_path().ok_or(AdapterError::MissingHomeDir)?;
        if !db_path.is_file() {
            return Err(AdapterError::NoSessionFound(db_path));
        }
        self.extract_handoff_from_db(&db_path, Some(project_dir))
    }

    fn launch_command(&self) -> &str {
        "opencode"
    }
}

/// Helper struct for raw messages loaded from SQLite.
struct RawMessage {
    id: String,
    role: String,
    content: String,
}

/// Opens a read-only SQLite pool connection for an existing file.
async fn open_pool_read_only(db_path: &Path) -> Result<SqlitePool> {
    if !db_path.is_file() {
        return Err(AdapterError::NoSessionFound(db_path.to_path_buf()));
    }
    let options = SqliteConnectOptions::new()
        .filename(db_path)
        .read_only(true);

    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .map_err(|e| {
            AdapterError::Other(format!(
                "Failed to open OpenCode SQLite database {}: {e}",
                db_path.display()
            ))
        })
}

/// Checks if a specific table exists in the SQLite database.
async fn table_exists(pool: &SqlitePool, table_name: &str) -> Result<bool> {
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name = ?",
    )
    .bind(table_name)
    .fetch_optional(pool)
    .await
    .map_err(|e| {
        AdapterError::Other(format!("Failed to verify table {table_name}: {e}"))
    })?;

    Ok(row.map(|r| r.0).unwrap_or(0) > 0)
}

/// Inspects and retrieves column names for a given SQLite table.
async fn get_table_columns(pool: &SqlitePool, table_name: &str) -> Result<Vec<String>> {
    let query = format!("PRAGMA table_info({})", table_name);
    let rows = sqlx::query(&query)
        .fetch_all(pool)
        .await
        .map_err(|e| {
            AdapterError::Other(format!("Failed to inspect table {table_name}: {e}"))
        })?;

    let columns = rows
        .into_iter()
        .filter_map(|r| r.try_get::<String, _>("name").ok())
        .collect();

    Ok(columns)
}

/// Extracts a `DateTime<Utc>` from a SQLite row column supporting string, integer, and float types.
fn extract_row_datetime(row: &sqlx::sqlite::SqliteRow, col: &str) -> Option<DateTime<Utc>> {
    if let Ok(s) = row.try_get::<String, _>(col) {
        return parse_sqlite_timestamp(&s);
    }
    if let Ok(i) = row.try_get::<i64, _>(col) {
        if i > 100_000_000_000 {
            return DateTime::from_timestamp_millis(i);
        } else if i > 0 {
            return DateTime::from_timestamp(i, 0);
        }
    }
    if let Ok(f) = row.try_get::<f64, _>(col) {
        let i = f as i64;
        if i > 100_000_000_000 {
            return DateTime::from_timestamp_millis(i);
        } else if i > 0 {
            return DateTime::from_timestamp(i, 0);
        }
    }
    None
}

/// Parses a date/time string from SQLite into a UTC [`DateTime`].
fn parse_sqlite_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    // RFC3339 / ISO8601
    if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
        return Some(dt.with_timezone(&Utc));
    }

    // "YYYY-MM-DD HH:MM:SS"
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S") {
        return Some(DateTime::from_naive_utc_and_offset(naive, Utc));
    }

    // "YYYY-MM-DD HH:MM:SS.fff"
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S%.f") {
        return Some(DateTime::from_naive_utc_and_offset(naive, Utc));
    }

    // "YYYY-MM-DD"
    if let Ok(naive_date) = chrono::NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        if let Some(naive) = naive_date.and_hms_opt(0, 0, 0) {
            return Some(DateTime::from_naive_utc_and_offset(naive, Utc));
        }
    }

    // Epoch numeric strings
    if let Ok(ts) = trimmed.parse::<i64>() {
        if ts > 100_000_000_000 {
            return DateTime::from_timestamp_millis(ts);
        } else if ts > 0 {
            return DateTime::from_timestamp(ts, 0);
        }
    }

    None
}

/// Generates a preview snippet around the query match.
fn extract_search_preview(content: &str, query: &str) -> String {
    let lower_content = content.to_lowercase();
    let lower_query = query.to_lowercase();

    if let Some(idx) = lower_content.find(&lower_query) {
        let start = idx.saturating_sub(40);
        let end = (idx + query.len() + 60).min(content.len());
        let mut preview = String::new();
        if start > 0 {
            preview.push_str("...");
        }
        preview.push_str(&content[start..end]);
        if end < content.len() {
            preview.push_str("...");
        }
        preview.replace('\n', " ").trim().to_string()
    } else {
        let end = 100.min(content.len());
        let mut preview = content[..end].to_string();
        if end < content.len() {
            preview.push_str("...");
        }
        preview.replace('\n', " ").trim().to_string()
    }
}

/// Executes an async future synchronously, safely handling existing Tokio runtime contexts.
fn block_on<F, T>(future: F) -> T
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    match tokio::runtime::Handle::try_current() {
        Ok(_) => {
            // Dedicated thread avoids re-entering the current tokio runtime thread
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("Failed to initialize single-threaded tokio runtime");
                rt.block_on(future)
            })
            .join()
            .expect("Worker thread panicked during async execution")
        }
        Err(_) => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to initialize single-threaded tokio runtime");
            rt.block_on(future)
        }
    }
}

/// Checks if a binary executable is available in PATH using `which` (Unix) or `where` (Windows).
fn check_binary_exists(binary: &str) -> bool {
    let which_cmd = if cfg!(target_os = "windows") {
        "where"
    } else {
        "which"
    };
    match std::process::Command::new(which_cmd).arg(binary).output() {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
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
    "localhost".to_string()
}

/// Parsed markdown section states.
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

/// Parses markdown text into tasks, decisions, blockers, and summary.
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

        if let Some(task) = parse_checkbox_task(trimmed) {
            match task.status {
                TaskStatus::Done => state.completed.push(task),
                TaskStatus::Partial => state.in_progress.push(task),
                TaskStatus::Pending => state.pending.push(task),
            }
            continue;
        }

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
                if let Some(desc) = extract_list_item(trimmed) {
                    state.decisions.push(parse_decision(desc));
                }
            }
            SectionContext::Blockers => {
                if let Some(desc) = extract_list_item(trimmed) {
                    state.blockers.push(desc.to_string());
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
    } else {
        state.summary = "Session captured from OpenCode".to_string();
    }

    state
}

/// Parses a checkbox task from markdown like `- [x] Task`.
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

/// Extracts list item text after bullet or number marker.
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

/// Parses a task item and extracts supplementary details if separated by " - ".
fn parse_task_item(desc: &str, status: TaskStatus) -> TaskItem {
    let trimmed = desc.trim();
    if let Some((title, detail)) = trimmed.split_once(" - ") {
        TaskItem::new(title.trim(), status).with_detail(detail.trim())
    } else {
        TaskItem::new(trimmed, status)
    }
}

/// Parses a decision line formatted as `**what**: why` or `what: why`.
fn parse_decision(desc: &str) -> Decision {
    let trimmed = desc.trim();
    if let Some(rest) = trimmed.strip_prefix("**") {
        if let Some((what, after_what)) = rest.split_once("**:") {
            let why = after_what.trim();
            return Decision::now(what.trim(), why);
        }
    }
    if let Some((what, why)) = trimmed.split_once(':') {
        Decision::now(what.trim(), why.trim())
    } else {
        Decision::now(trimmed, "")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn create_temp_db_path(prefix: &str) -> (PathBuf, PathBuf) {
        let temp_dir = std::env::temp_dir().join(format!(
            "opencode_test_{}_{}_{}",
            prefix,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("Valid unix timestamp")
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir).expect("Failed to create temporary directory");
        let db_path = temp_dir.join("opencode.db");
        (temp_dir, db_path)
    }

    async fn create_test_db(db_path: &Path) -> SqlitePool {
        let options = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .expect("Failed to create test database");

        sqlx::query(
            "CREATE TABLE session (
                id TEXT PRIMARY KEY,
                title TEXT,
                directory TEXT,
                updated_at TEXT
            )",
        )
        .execute(&pool)
        .await
        .expect("Failed to create session table");

        sqlx::query(
            "CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                role TEXT,
                content TEXT,
                created_at TEXT
            )",
        )
        .execute(&pool)
        .await
        .expect("Failed to create message table");

        sqlx::query(
            "CREATE TABLE part (
                id TEXT PRIMARY KEY,
                message_id TEXT,
                type TEXT,
                content TEXT,
                tail_start_id TEXT
            )",
        )
        .execute(&pool)
        .await
        .expect("Failed to create part table");

        pool
    }

    #[test]
    fn test_adapter_metadata() {
        let adapter = OpenCodeAdapter::new();
        assert_eq!(adapter.name(), "opencode");
        assert_eq!(adapter.launch_command(), "opencode");

        let project_dir = Path::new("/workspace/sample_proj");
        assert_eq!(
            adapter.instruction_path(project_dir),
            PathBuf::from("/workspace/sample_proj/.opencode/instructions.md")
        );
    }

    #[test]
    fn test_generate_instructions() {
        let adapter = OpenCodeAdapter::new();
        let mut handoff = Handoff::for_project("ctx-sync");
        handoff.summary = "Implemented sqlite parser".to_string();
        handoff.completed.push(TaskItem::done("Database connection"));
        handoff.in_progress.push(TaskItem::partial("Testing suite"));
        handoff.pending.push(TaskItem::pending("Cross-compilation"));
        handoff.decisions.push(Decision::now("Use sqlx", "Unified async driver"));
        handoff.blockers.push("Missing MinIO credentials".to_string());

        let instructions = adapter.generate_instructions(&handoff);

        assert!(instructions.contains("# Project: ctx-sync"));
        assert!(instructions.contains("## Current State\n\nImplemented sqlite parser"));
        assert!(instructions.contains("- [x] Database connection"));
        assert!(instructions.contains("- [-] Testing suite"));
        assert!(instructions.contains("- [ ] Cross-compilation"));
        assert!(instructions.contains("- **Use sqlx**: Unified async driver"));
        assert!(instructions.contains("- Missing MinIO credentials"));
        assert!(instructions.contains("## Security Directive"));
        assert!(instructions.contains("Do NOT store, commit, or hardcode secrets"));
    }

    #[tokio::test]
    async fn test_extract_handoff_from_db() {
        let (temp_dir, db_path) = create_temp_db_path("extract");
        let pool = create_test_db(&db_path).await;

        sqlx::query(
            "INSERT INTO session (id, title, directory, updated_at) VALUES (?, ?, ?, ?)",
        )
        .bind("sess_01")
        .bind("Feature Auth")
        .bind("/workspaces/my-app")
        .bind("2026-09-04 12:00:00")
        .execute(&pool)
        .await
        .expect("Insert session");

        sqlx::query(
            "INSERT INTO message (id, session_id, role, content, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind("msg_01")
        .bind("sess_01")
        .bind("user")
        .bind("Please add auth tokens")
        .bind("2026-09-04 12:00:05")
        .execute(&pool)
        .await
        .expect("Insert message 1");

        let assistant_content = "## Summary\nImplemented JWT verification.\n\n\
                                 ## Completed Tasks\n- [x] JWT token decoding\n\
                                 ## In Progress Tasks\n- [-] Token revocation list\n\
                                 ## Pending Tasks\n- [ ] Argon2 password hash\n\
                                 ## Decisions\n- **JWT Alg**: Use RS256 for public verification";

        sqlx::query(
            "INSERT INTO message (id, session_id, role, content, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind("msg_02")
        .bind("sess_01")
        .bind("assistant")
        .bind(assistant_content)
        .bind("2026-09-04 12:00:10")
        .execute(&pool)
        .await
        .expect("Insert message 2");

        let adapter = OpenCodeAdapter::with_db_path(&db_path);
        let project_dir = Path::new("/workspaces/my-app");
        let handoff = adapter
            .extract_handoff_from_db(&db_path, Some(project_dir))
            .expect("Handoff extraction should succeed");

        assert_eq!(handoff.project_name, "my-app");
        assert_eq!(handoff.source_agent, "opencode");
        assert!(handoff.summary.contains("Implemented JWT verification"));
        assert_eq!(handoff.completed.len(), 1);
        assert_eq!(handoff.completed[0].description, "JWT token decoding");
        assert_eq!(handoff.in_progress.len(), 1);
        assert_eq!(handoff.in_progress[0].description, "Token revocation list");
        assert_eq!(handoff.pending.len(), 1);
        assert_eq!(handoff.pending[0].description, "Argon2 password hash");
        assert_eq!(handoff.decisions.len(), 1);
        assert_eq!(handoff.decisions[0].what, "JWT Alg");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_compaction_marker_handling() {
        let (temp_dir, db_path) = create_temp_db_path("compact");
        let pool = create_test_db(&db_path).await;

        sqlx::query(
            "INSERT INTO session (id, title, directory, updated_at) VALUES (?, ?, ?, ?)",
        )
        .bind("sess_comp")
        .bind("Compacted Session")
        .bind("/projects/proj_compact")
        .bind("2026-09-04 14:00:00")
        .execute(&pool)
        .await
        .expect("Insert session");

        sqlx::query(
            "INSERT INTO message (id, session_id, role, content, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind("msg_old_1")
        .bind("sess_comp")
        .bind("user")
        .bind("Early message before compaction")
        .bind("2026-09-04 14:00:01")
        .execute(&pool)
        .await
        .expect("Insert old message");

        sqlx::query(
            "INSERT INTO message (id, session_id, role, content, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind("msg_old_2")
        .bind("sess_comp")
        .bind("assistant")
        .bind("Early assistant reply before compaction")
        .bind("2026-09-04 14:00:02")
        .execute(&pool)
        .await
        .expect("Insert old assistant message");

        // Compaction part on message 2 indicating tail starts at msg_tail_1
        sqlx::query(
            "INSERT INTO part (id, message_id, type, content, tail_start_id) VALUES (?, ?, ?, ?, ?)",
        )
        .bind("part_compact_01")
        .bind("msg_old_2")
        .bind("compaction")
        .bind(r#"{"summary": "Compacted history of early discussion", "tail_start_id": "msg_tail_1"}"#)
        .bind("msg_tail_1")
        .execute(&pool)
        .await
        .expect("Insert compaction part");

        // Tail messages
        sqlx::query(
            "INSERT INTO message (id, session_id, role, content, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind("msg_tail_1")
        .bind("sess_comp")
        .bind("user")
        .bind("New tail question")
        .bind("2026-09-04 14:05:00")
        .execute(&pool)
        .await
        .expect("Insert tail message 1");

        sqlx::query(
            "INSERT INTO message (id, session_id, role, content, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind("msg_tail_2")
        .bind("sess_comp")
        .bind("assistant")
        .bind("## Current State\nTail assistant answer.\n\n## Completed Tasks\n- [x] Handled tail task")
        .bind("2026-09-04 14:05:10")
        .execute(&pool)
        .await
        .expect("Insert tail message 2");

        let adapter = OpenCodeAdapter::with_db_path(&db_path);
        let handoff = adapter
            .extract_handoff_from_db(&db_path, None)
            .expect("Compaction extract should succeed");

        assert!(handoff.summary.contains("Compacted history of early discussion"));
        assert!(handoff.summary.contains("Tail assistant answer"));
        assert_eq!(handoff.completed.len(), 1);
        assert_eq!(handoff.completed[0].description, "Handled tail task");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_search_sessions() {
        let (temp_dir, db_path) = create_temp_db_path("search");
        let pool = create_test_db(&db_path).await;

        sqlx::query(
            "INSERT INTO session (id, title, directory, updated_at) VALUES (?, ?, ?, ?)",
        )
        .bind("sess_alpha")
        .bind("Alpha Project Setup")
        .bind("/workspaces/alpha")
        .bind("2026-09-04 10:00:00")
        .execute(&pool)
        .await
        .expect("Insert alpha");

        sqlx::query(
            "INSERT INTO message (id, session_id, role, content, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind("msg_alpha_1")
        .bind("sess_alpha")
        .bind("assistant")
        .bind("Configured postgresql connection pool")
        .bind("2026-09-04 10:00:05")
        .execute(&pool)
        .await
        .expect("Insert msg");

        sqlx::query(
            "INSERT INTO session (id, title, directory, updated_at) VALUES (?, ?, ?, ?)",
        )
        .bind("sess_beta")
        .bind("Beta Vault Storage")
        .bind("/workspaces/beta")
        .bind("2026-09-04 11:00:00")
        .execute(&pool)
        .await
        .expect("Insert beta");

        let adapter = OpenCodeAdapter::with_db_path(&db_path);

        // Search message content
        let results = adapter
            .search_sessions("postgresql")
            .expect("Search should succeed");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session_id, "sess_alpha");
        assert_eq!(results[0].title, "Alpha Project Setup");
        assert!(results[0].preview.contains("postgresql"));

        // Search session title
        let results_title = adapter
            .search_sessions("Vault")
            .expect("Search should succeed");
        assert_eq!(results_title.len(), 1);
        assert_eq!(results_title[0].session_id, "sess_beta");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_list_recent_sessions() {
        let (temp_dir, db_path) = create_temp_db_path("recent");
        let pool = create_test_db(&db_path).await;

        let now_str = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let old_str = (Utc::now() - Duration::days(40))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();

        sqlx::query(
            "INSERT INTO session (id, title, directory, updated_at) VALUES (?, ?, ?, ?)",
        )
        .bind("recent_01")
        .bind("Recent Session")
        .bind("/workspaces/recent")
        .bind(&now_str)
        .execute(&pool)
        .await
        .expect("Insert recent session");

        sqlx::query(
            "INSERT INTO session (id, title, directory, updated_at) VALUES (?, ?, ?, ?)",
        )
        .bind("old_01")
        .bind("Old Session")
        .bind("/workspaces/old")
        .bind(&old_str)
        .execute(&pool)
        .await
        .expect("Insert old session");

        sqlx::query(
            "INSERT INTO message (id, session_id, role, content, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind("m_rec_1")
        .bind("recent_01")
        .bind("user")
        .bind("Hello")
        .bind(&now_str)
        .execute(&pool)
        .await
        .expect("Insert message");

        let adapter = OpenCodeAdapter::with_db_path(&db_path);

        // List last 7 days: should only include recent_01
        let recent = adapter
            .list_recent_sessions(7)
            .expect("List recent should succeed");
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].id, "recent_01");
        assert_eq!(recent[0].title, "Recent Session");
        assert_eq!(recent[0].message_count, 1);

        // List last 60 days: should include both
        let all = adapter
            .list_recent_sessions(60)
            .expect("List all should succeed");
        assert_eq!(all.len(), 2);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_detect_installed_custom_dirs() {
        let (temp_dir, _) = create_temp_db_path("detect");
        let adapter = OpenCodeAdapter::with_data_dir(&temp_dir);
        assert!(adapter.detect_installed());

        let nonexistent = temp_dir.join("does_not_exist");
        let adapter_nonexistent = OpenCodeAdapter::with_data_dir(&nonexistent);
        if !check_binary_exists("opencode") {
            assert!(!adapter_nonexistent.detect_installed());
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
