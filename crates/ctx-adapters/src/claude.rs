//! Claude Code agent adapter implementation for `ctx`.
//!
//! Implements the [`AgentAdapter`] interface for Anthropic's Claude Code CLI.
//! Handles detecting installation, locating and formatting `CLAUDE.md` instructions,
//! and extracting session handoffs from `~/.claude/` session logs.

use std::fs;
use std::path::{Path, PathBuf};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use ctx_core::handoff::{Decision, Handoff, TaskItem, TaskStatus};
use crate::adapter::{AdapterError, AgentAdapter, Result};

/// Adapter for Anthropic's Claude Code CLI agent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeAdapter;

impl ClaudeAdapter {
    /// Creates a new [`ClaudeAdapter`] instance.
    pub fn new() -> Self {
        Self
    }

    /// Checks if Claude configuration directory exists at the specified path.
    pub fn detect_installed_at(&self, claude_dir: &Path) -> bool {
        claude_dir.is_dir()
    }

    /// Extracts a project handoff snapshot from a specific Claude configuration directory.
    pub fn extract_handoff_from_dir(
        &self,
        claude_dir: &Path,
        project_dir: Option<&Path>,
    ) -> Result<Handoff> {
        let session_file = Self::find_most_recent_session_file(claude_dir, project_dir)?;
        self.parse_session_file(&session_file, project_dir)
    }

    /// Parses a Claude Code JSONL session file into a [`Handoff`] snapshot.
    pub fn parse_session_file(
        &self,
        session_file: &Path,
        project_dir: Option<&Path>,
    ) -> Result<Handoff> {
        let content = fs::read_to_string(session_file).map_err(AdapterError::Io)?;

        let fallback_name = project_dir
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned());

        self.parse_session_content(&content, fallback_name.as_deref())
    }

    /// Parses the raw text content of a Claude Code JSONL session file into a [`Handoff`] snapshot.
    pub fn parse_session_content(
        &self,
        content: &str,
        project_name: Option<&str>,
    ) -> Result<Handoff> {
        let mut last_compacted_text: Option<String> = None;
        let mut last_assistant_text: Option<String> = None;
        let mut last_user_text: Option<String> = None;
        let mut last_timestamp: Option<DateTime<Utc>> = None;
        let mut session_cwd: Option<String> = None;
        let mut last_git_branch: Option<String> = None;
        let mut last_git_commit: Option<String> = None;
        let mut structured_tasks: Vec<TaskItem> = Vec::new();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let event: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("Skipping invalid JSON line in Claude session: {}", e);
                    continue;
                }
            };

            // Capture session timestamp
            if let Some(ts_str) = event.get("timestamp").and_then(|v| v.as_str())
                && let Ok(ts) = DateTime::parse_from_rfc3339(ts_str) {
                    last_timestamp = Some(ts.with_timezone(&Utc));
                }

            // Capture session cwd
            if let Some(cwd_str) = event.get("cwd").and_then(|v| v.as_str()) {
                session_cwd = Some(cwd_str.to_string());
            }

            // Capture git metadata
            if let Some(branch) = event.get("gitBranch").and_then(|v| v.as_str()) {
                last_git_branch = Some(branch.to_string());
            }
            if let Some(commit) = event.get("gitCommit").and_then(|v| v.as_str()) {
                last_git_commit = Some(commit.to_string());
            }

            // Parse structured tasks if present
            if let Some(attachment) = event.get("attachment")
                && attachment.get("type").and_then(|v| v.as_str()) == Some("todo_reminder")
                    && let Some(items) = attachment.get("content").and_then(|v| v.as_array()) {
                        parse_structured_todos(items, &mut structured_tasks);
                    }
            if let Some(todos) = event.get("todos").and_then(|v| v.as_array()) {
                parse_structured_todos(todos, &mut structured_tasks);
            }

            // Check if this is a compaction or summary event
            let is_compact = event
                .get("type")
                .and_then(|v| v.as_str())
                .map(|t| {
                    t == "compact"
                        || t == "compacted"
                        || t == "summary"
                        || t == "compact_boundary"
                })
                .unwrap_or(false)
                || event.get("is_compact").and_then(|v| v.as_bool()).unwrap_or(false)
                || event.get("compact").is_some()
                || event.get("compacted").is_some();

            if is_compact {
                let text = if let Some(s) = event.get("summary").and_then(|v| v.as_str()) {
                    s.to_string()
                } else {
                    extract_text_content(&event)
                };
                if !text.trim().is_empty() {
                    last_compacted_text = Some(text);
                }
                continue;
            }

            // Check if this is an assistant message
            let is_assistant = event
                .get("type")
                .and_then(|v| v.as_str())
                .map(|t| t == "assistant")
                .unwrap_or(false)
                || event
                    .get("message")
                    .and_then(|m| m.get("role"))
                    .and_then(|r| r.as_str())
                    .map(|r| r == "assistant")
                    .unwrap_or(false);

            if is_assistant {
                let content_val = event
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .unwrap_or(&event);
                let text = extract_text_content(content_val);
                if !text.trim().is_empty() {
                    last_assistant_text = Some(text);
                }
                continue;
            }

            // Check if this is a user message
            let is_user = event
                .get("type")
                .and_then(|v| v.as_str())
                .map(|t| t == "user")
                .unwrap_or(false)
                || event
                    .get("message")
                    .and_then(|m| m.get("role"))
                    .and_then(|r| r.as_str())
                    .map(|r| r == "user")
                    .unwrap_or(false);

            if is_user {
                let content_val = event
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .unwrap_or(&event);
                let text = extract_text_content(content_val);
                if !text.trim().is_empty() {
                    last_user_text = Some(text);
                }
            }
        }

        // Determine primary text: compacted event preferred, then assistant message, then user message
        let primary_text = match (last_compacted_text, last_assistant_text, last_user_text) {
            (Some(compacted), _, _) => compacted,
            (None, Some(assistant), _) => assistant,
            (None, None, Some(user)) => user,
            (None, None, None) => {
                return Err(AdapterError::ExtractionFailed(
                    "No valid messages or compacted events found in Claude session".to_string(),
                ));
            }
        };

        let mut parsed_state = parse_markdown_state(&primary_text);

        // Merge any structured tasks that weren't already found in markdown text
        for item in structured_tasks {
            match item.status {
                TaskStatus::Done => {
                    if !parsed_state.completed.iter().any(|x| x.description == item.description) {
                        parsed_state.completed.push(item);
                    }
                }
                TaskStatus::Partial => {
                    if !parsed_state.in_progress.iter().any(|x| x.description == item.description) {
                        parsed_state.in_progress.push(item);
                    }
                }
                TaskStatus::Pending => {
                    if !parsed_state.pending.iter().any(|x| x.description == item.description) {
                        parsed_state.pending.push(item);
                    }
                }
            }
        }

        // Determine project name
        let resolved_project_name = project_name
            .map(|s| s.to_string())
            .or_else(|| {
                session_cwd.as_ref().and_then(|cwd| {
                    Path::new(cwd)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                })
            })
            .unwrap_or_else(|| "unnamed".to_string());

        let mut handoff = Handoff::new();
        handoff.project_name = resolved_project_name;
        handoff.created_at = last_timestamp.unwrap_or_else(Utc::now);
        handoff.source_agent = "claude".to_string();
        handoff.source_machine = get_hostname();
        handoff.git_branch = last_git_branch.unwrap_or_default();
        handoff.git_commit = last_git_commit.unwrap_or_default();
        handoff.summary = parsed_state.summary;
        handoff.completed = parsed_state.completed;
        handoff.in_progress = parsed_state.in_progress;
        handoff.pending = parsed_state.pending;
        handoff.decisions = parsed_state.decisions;
        handoff.blockers = parsed_state.blockers;

        Ok(handoff)
    }

    /// Finds the most recently modified `.jsonl` session file in Claude's configuration hierarchy.
    fn find_most_recent_session_file(
        claude_dir: &Path,
        _project_dir: Option<&Path>,
    ) -> Result<PathBuf> {
        let mut candidates = Vec::new();

        // 1. Search in ~/.claude/sessions/
        let sessions_dir = claude_dir.join("sessions");
        if sessions_dir.is_dir() {
            let _ = collect_jsonl_files(&sessions_dir, &mut candidates);
        }

        // 2. Search in ~/.claude/projects/
        let projects_dir = claude_dir.join("projects");
        if projects_dir.is_dir() {
            let _ = collect_jsonl_files(&projects_dir, &mut candidates);
        }

        // 3. Fallback: search claude_dir root
        if candidates.is_empty() && claude_dir.is_dir() {
            let _ = collect_jsonl_files(claude_dir, &mut candidates);
        }

        if candidates.is_empty() {
            return Err(AdapterError::NoSessionFound(sessions_dir));
        }

        // Sort by file modification time descending
        candidates.sort_by(|a, b| {
            let time_a = fs::metadata(a)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            let time_b = fs::metadata(b)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            time_b.cmp(&time_a)
        });

        Ok(candidates.remove(0))
    }

    /// Checks if a binary command exists in PATH using `which` (Unix) or `where` (Windows).
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
}

impl AgentAdapter for ClaudeAdapter {
    fn name(&self) -> &str {
        "claude"
    }

    fn detect_installed(&self) -> bool {
        // 1. Check if ~/.claude/ directory exists
        if let Some(home) = dirs::home_dir()
            && home.join(".claude").is_dir() {
                return true;
            }

        // 2. Check if 'claude' binary exists in PATH
        Self::check_binary_exists("claude")
    }

    fn instruction_path(&self, project_dir: &Path) -> PathBuf {
        project_dir.join(".claude").join("CLAUDE.md")
    }

    fn generate_instructions(&self, handoff: &Handoff) -> String {
        let mut md = String::new();

        // 1. Project name
        let project_name = if handoff.project_name.trim().is_empty() {
            "Unnamed Project"
        } else {
            handoff.project_name.trim()
        };
        md.push_str(&format!("# Project: {}\n\n", project_name));

        // 2. Last Session (source agent+machine+timestamp from handoff)
        md.push_str("## Last Session\n\n");
        let source_agent = if handoff.source_agent.trim().is_empty() {
            "unknown"
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

        // 3. Current State (summary)
        md.push_str("## Current State\n\n");
        if handoff.summary.trim().is_empty() {
            md.push_str("No summary provided.\n\n");
        } else {
            md.push_str(handoff.summary.trim());
            md.push_str("\n\n");
        }

        // 4. Completed items
        md.push_str("## Completed Items\n\n");
        if handoff.completed.is_empty() {
            md.push_str("None.\n\n");
        } else {
            for item in &handoff.completed {
                md.push_str(&item.to_markdown());
                md.push('\n');
            }
            md.push('\n');
        }

        // 5. In Progress items
        md.push_str("## In Progress Items\n\n");
        if handoff.in_progress.is_empty() {
            md.push_str("None.\n\n");
        } else {
            for item in &handoff.in_progress {
                md.push_str(&item.to_markdown());
                md.push('\n');
            }
            md.push('\n');
        }

        // 6. Pending items
        md.push_str("## Pending Items\n\n");
        if handoff.pending.is_empty() {
            md.push_str("None.\n\n");
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
            md.push_str("None.\n\n");
        } else {
            for decision in &handoff.decisions {
                md.push_str(&decision.to_markdown());
                md.push('\n');
            }
            md.push('\n');
        }

        // 8. Security directive (secrets in env vars)
        md.push_str("## Security Directive\n\n");
        md.push_str("- Do NOT store, commit, or hardcode secrets, tokens, or credentials in project files or transcripts.\n");
        md.push_str("- All sensitive credentials must be accessed strictly through environment variables provided by the ctx vault.\n");
        md.push_str("- Never print decrypted secret values into session transcripts or chat outputs.\n");

        md
    }

    fn extract_handoff(&self, project_dir: &Path) -> Result<Handoff> {
        let home = dirs::home_dir().ok_or(AdapterError::MissingHomeDir)?;
        let claude_dir = home.join(".claude");
        self.extract_handoff_from_dir(&claude_dir, Some(project_dir))
    }

    fn launch_command(&self) -> &str {
        "claude"
    }
}

/// Recursively collects all `.jsonl` files in a directory.
fn collect_jsonl_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    let entries = fs::read_dir(dir).map_err(AdapterError::Io)?;

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.is_dir() {
            let _ = collect_jsonl_files(&path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
    Ok(())
}

/// Recursively extracts plain text from a serde JSON value (string, array of blocks, or objects).
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
            } else {
                String::new()
            }
        }
        _ => String::new(),
    }
}

/// Parses structured todos from JSON array elements.
fn parse_structured_todos(items: &[serde_json::Value], tasks: &mut Vec<TaskItem>) {
    for item in items {
        if let Some(s) = item.as_str() {
            if let Some(task) = parse_checkbox_task(s) {
                tasks.push(task);
            }
        } else if let Some(obj) = item.as_object() {
            let status_str = obj
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("pending")
                .to_lowercase();
            let content = obj
                .get("content")
                .or_else(|| obj.get("text"))
                .or_else(|| obj.get("description"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if !content.trim().is_empty() {
                let status = match status_str.as_str() {
                    "completed" | "done" => TaskStatus::Done,
                    "in_progress" | "inprogress" | "partial" => TaskStatus::Partial,
                    _ => TaskStatus::Pending,
                };
                tasks.push(TaskItem::new(content.trim(), status));
            }
        }
    }
}

/// Extracted markdown components.
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
        state.summary = "Session captured from Claude Code".to_string();
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
    if let Some(rest) = trimmed.strip_prefix("[x] ").or_else(|| trimmed.strip_prefix("[X] ")) {
        Some(parse_task_item(rest, TaskStatus::Done))
    } else if let Some(rest) = trimmed
        .strip_prefix("[-] ")
        .or_else(|| trimmed.strip_prefix("[/] "))
    {
        Some(parse_task_item(rest, TaskStatus::Partial))
    } else { trimmed.strip_prefix("[ ] ").map(|rest| parse_task_item(rest, TaskStatus::Pending)) }
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

/// Parses task item and extracts supplementary details if separated by " - ".
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
    if let Some(rest) = trimmed.strip_prefix("**")
        && let Some((what, after_what)) = rest.split_once("**:") {
            let why = after_what.trim();
            return Decision::now(what.trim(), why);
        }
    if let Some((what, why)) = trimmed.split_once(':') {
        Decision::now(what.trim(), why.trim())
    } else {
        Decision::now(trimmed, "")
    }
}

/// Resolves the local system hostname safely without panicking.
fn get_hostname() -> String {
    if let Ok(host) = std::env::var("HOSTNAME")
        && !host.trim().is_empty() {
            return host.trim().to_string();
        }
    if let Ok(host) = std::env::var("HOST")
        && !host.trim().is_empty() {
            return host.trim().to_string();
        }
    if let Ok(output) = std::process::Command::new("hostname").output()
        && output.status.success() {
            let host = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !host.is_empty() {
                return host;
            }
        }
    "unknown-host".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        std::env::temp_dir().join(format!("{prefix}_{nanos}_{counter}"))
    }

    #[test]
    fn test_generate_instructions_output_format() {
        let adapter = ClaudeAdapter::new();

        let mut handoff = Handoff::for_project("ctx")
            .with_summary("Refactoring agent adapters for Claude Code")
            .with_source("Claude Code", "dev-machine-1")
            .with_git("main", "a1b2c3d4");

        handoff.add_completed(
            TaskItem::done("Implement AgentAdapter trait").with_detail("Added unit tests"),
        );
        handoff.add_in_progress(TaskItem::partial("Write ClaudeAdapter tests"));
        handoff.add_pending(TaskItem::pending("Integrate with CLI save command"));
        handoff.add_decision(Decision::new(
            "Use workspace dependencies",
            "Keep dependencies unified across all crates",
            Utc::now(),
        ));

        let instructions = adapter.generate_instructions(&handoff);

        // Section 1: Project name
        assert!(
            instructions.contains("# Project: ctx"),
            "Missing Project Name section: {}",
            instructions
        );

        // Section 2: Last Session (source agent+machine+timestamp from handoff)
        assert!(instructions.contains("## Last Session"));
        assert!(instructions.contains("- **Agent:** Claude Code"));
        assert!(instructions.contains("- **Machine:** dev-machine-1"));
        assert!(instructions.contains("- **Timestamp:**"));
        assert!(instructions.contains("- **Git Branch:** main"));
        assert!(instructions.contains("- **Git Commit:** a1b2c3d4"));

        // Section 3: Current State (summary)
        assert!(instructions.contains("## Current State"));
        assert!(instructions.contains("Refactoring agent adapters for Claude Code"));

        // Section 4: Completed items
        assert!(instructions.contains("## Completed Items"));
        assert!(instructions.contains("- [x] Implement AgentAdapter trait - Added unit tests"));

        // Section 5: In Progress items
        assert!(instructions.contains("## In Progress Items"));
        assert!(instructions.contains("- [-] Write ClaudeAdapter tests"));

        // Section 6: Pending items
        assert!(instructions.contains("## Pending Items"));
        assert!(instructions.contains("- [ ] Integrate with CLI save command"));

        // Section 7: Decisions
        assert!(instructions.contains("## Decisions"));
        assert!(instructions.contains("Use workspace dependencies"));
        assert!(instructions.contains("Keep dependencies unified across all crates"));

        // Section 8: Security directive (secrets in env vars)
        assert!(instructions.contains("## Security Directive"));
        assert!(instructions.contains("environment variables"));
        assert!(instructions.contains("ctx vault"));
        assert!(instructions.contains("secrets"));
    }

    #[test]
    fn test_generate_instructions_empty_handoff() {
        let adapter = ClaudeAdapter::new();
        let handoff = Handoff::new();

        let instructions = adapter.generate_instructions(&handoff);

        assert!(instructions.contains("# Project: Unnamed Project"));
        assert!(instructions.contains("## Last Session"));
        assert!(instructions.contains("- **Agent:** unknown"));
        assert!(instructions.contains("- **Machine:** unknown"));
        assert!(instructions.contains("## Current State"));
        assert!(instructions.contains("No summary provided."));
        assert!(instructions.contains("## Completed Items"));
        assert!(instructions.contains("None."));
        assert!(instructions.contains("## In Progress Items"));
        assert!(instructions.contains("None."));
        assert!(instructions.contains("## Pending Items"));
        assert!(instructions.contains("None."));
        assert!(instructions.contains("## Decisions"));
        assert!(instructions.contains("None."));
        assert!(instructions.contains("## Security Directive"));
    }

    #[test]
    fn test_instruction_path() {
        let adapter = ClaudeAdapter::new();
        let project_dir = Path::new("/workspace/my_project");
        let expected = project_dir.join(".claude").join("CLAUDE.md");
        assert_eq!(adapter.instruction_path(project_dir), expected);
    }

    #[test]
    fn test_launch_command() {
        let adapter = ClaudeAdapter::new();
        assert_eq!(adapter.launch_command(), "claude");
        assert_eq!(adapter.name(), "claude");
    }

    #[test]
    fn test_detect_installed_at() {
        let adapter = ClaudeAdapter::new();
        let temp_dir = unique_temp_dir("ctx_test_claude_dir");
        assert!(!adapter.detect_installed_at(&temp_dir));

        fs::create_dir_all(&temp_dir).expect("Failed to create temporary directory");
        assert!(adapter.detect_installed_at(&temp_dir));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_parse_session_content_from_compacted_event() {
        let adapter = ClaudeAdapter::new();

        let jsonl = r###"
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"Initial user message"}]},"timestamp":"2026-09-04T10:00:00Z"}
{"type":"compacted","summary":"## Summary\nSuccessfully completed core sync logic.\n\n## Completed\n- [x] Write state machine\n- [x] Add protocol serialization\n\n## In Progress\n- [-] Claude adapter extraction\n\n## Pending\n- [ ] Codex adapter\n\n## Decisions\n- **Use JSONL for sessions**: Easy append-only parsing","timestamp":"2026-09-04T12:00:00Z","cwd":"/workspace/ctx"}
"###;

        let handoff = adapter
            .parse_session_content(jsonl, Some("ctx"))
            .expect("Should parse compacted session");

        assert_eq!(handoff.project_name, "ctx");
        assert_eq!(handoff.source_agent, "claude");
        assert!(handoff.summary.contains("Successfully completed core sync logic."));

        assert_eq!(handoff.completed.len(), 2);
        assert_eq!(handoff.completed[0].description, "Write state machine");
        assert_eq!(handoff.completed[0].status, TaskStatus::Done);
        assert_eq!(handoff.completed[1].description, "Add protocol serialization");

        assert_eq!(handoff.in_progress.len(), 1);
        assert_eq!(handoff.in_progress[0].description, "Claude adapter extraction");
        assert_eq!(handoff.in_progress[0].status, TaskStatus::Partial);

        assert_eq!(handoff.pending.len(), 1);
        assert_eq!(handoff.pending[0].description, "Codex adapter");
        assert_eq!(handoff.pending[0].status, TaskStatus::Pending);

        assert_eq!(handoff.decisions.len(), 1);
        assert_eq!(handoff.decisions[0].what, "Use JSONL for sessions");
        assert_eq!(handoff.decisions[0].why, "Easy append-only parsing");
    }

    #[test]
    fn test_parse_session_content_from_assistant_message() {
        let adapter = ClaudeAdapter::new();

        let jsonl = r###"
{"type":"user","message":{"role":"user","content":"Please update the tests"},"timestamp":"2026-09-04T14:00:00Z"}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"I updated all test cases.\n\n- [x] Fix unit tests\n- [-] Benchmark latency\n- [ ] Add fuzz testing"}]},"timestamp":"2026-09-04T14:05:00Z","gitBranch":"feature-tests","gitCommit":"beef123"}
"###;

        let handoff = adapter
            .parse_session_content(jsonl, Some("test-project"))
            .expect("Should parse assistant message session");

        assert_eq!(handoff.project_name, "test-project");
        assert_eq!(handoff.git_branch, "feature-tests");
        assert_eq!(handoff.git_commit, "beef123");
        assert!(handoff.summary.contains("I updated all test cases."));

        assert_eq!(handoff.completed.len(), 1);
        assert_eq!(handoff.completed[0].description, "Fix unit tests");

        assert_eq!(handoff.in_progress.len(), 1);
        assert_eq!(handoff.in_progress[0].description, "Benchmark latency");

        assert_eq!(handoff.pending.len(), 1);
        assert_eq!(handoff.pending[0].description, "Add fuzz testing");
    }

    #[test]
    fn test_extract_handoff_from_directory_finds_most_recent() {
        let adapter = ClaudeAdapter::new();
        let temp_claude = unique_temp_dir("ctx_test_claude_sessions");
        let sessions_dir = temp_claude.join("sessions");
        fs::create_dir_all(&sessions_dir).expect("Failed to create mock sessions directory");

        let old_file = sessions_dir.join("old_session.jsonl");
        let new_file = sessions_dir.join("new_session.jsonl");

        {
            let mut f1 = fs::File::create(&old_file).expect("Failed to create old file");
            writeln!(
                f1,
                r#"{{"type":"assistant","message":{{"role":"assistant","content":"Old session work"}}}}"#
            )
            .expect("Failed to write old session");
        }

        // Sleep briefly to ensure distinct file modification times
        std::thread::sleep(std::time::Duration::from_millis(50));

        {
            let mut f2 = fs::File::create(&new_file).expect("Failed to create new file");
            writeln!(
                f2,
                r#"{{"type":"assistant","message":{{"role":"assistant","content":"Newest session work\n- [x] Task in newest session"}}}}"#
            )
            .expect("Failed to write new session");
        }

        let handoff = adapter
            .extract_handoff_from_dir(&temp_claude, Some(Path::new("/projects/demo")))
            .expect("Extraction from mock directory should succeed");

        assert_eq!(handoff.project_name, "demo");
        assert!(handoff.summary.contains("Newest session work"));
        assert_eq!(handoff.completed.len(), 1);
        assert_eq!(handoff.completed[0].description, "Task in newest session");

        let _ = fs::remove_dir_all(&temp_claude);
    }

    #[test]
    fn test_extract_handoff_no_session_found() {
        let adapter = ClaudeAdapter::new();
        let temp_dir = unique_temp_dir("ctx_test_empty_claude");
        fs::create_dir_all(&temp_dir).expect("Failed to create temp dir");

        let result = adapter.extract_handoff_from_dir(&temp_dir, None);
        assert!(result.is_err());
        match result.unwrap_err() {
            AdapterError::NoSessionFound(p) => {
                assert!(p.ends_with("sessions"));
            }
            err => panic!("Expected NoSessionFound error, got: {err:?}"),
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_adapter_serde_roundtrip() {
        let adapter = ClaudeAdapter::new();
        let json = serde_json::to_string(&adapter).expect("Serialize ClaudeAdapter");
        let deserialized: ClaudeAdapter =
            serde_json::from_str(&json).expect("Deserialize ClaudeAdapter");
        assert_eq!(adapter, deserialized);
    }
}

