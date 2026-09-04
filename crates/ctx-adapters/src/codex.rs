//! OpenAI Codex agent adapter implementation.
//!
//! Provides [`CodexAdapter`] which implements [`AgentAdapter`] for the OpenAI Codex CLI.
//! It handles detecting the installation of Codex, locating and generating `AGENTS.md`
//! instruction files, extracting handoff state from `~/.codex/sessions/` rollout files,
//! and specifying the `codex` launch command.

use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use ctx_core::handoff::{Decision, Handoff, TaskItem};

use crate::adapter::{AdapterError, AgentAdapter, Result};

/// Adapter for the OpenAI Codex CLI agent.
///
/// Implements [`AgentAdapter`] to detect Codex installation, generate `AGENTS.md` instructions,
/// extract handoff context from `~/.codex/sessions/` rollout files, and launch the `codex` command.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CodexAdapter {
    /// Optional custom path to the codex home directory (defaults to `~/.codex`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_home: Option<PathBuf>,
}

impl CodexAdapter {
    /// Creates a new default [`CodexAdapter`].
    pub fn new() -> Self {
        Self { codex_home: None }
    }

    /// Creates a new [`CodexAdapter`] with a custom codex home directory.
    pub fn with_codex_home(codex_home: impl Into<PathBuf>) -> Self {
        Self {
            codex_home: Some(codex_home.into()),
        }
    }

    /// Resolves the root codex configuration directory (`~/.codex` or custom override).
    pub fn resolve_codex_home(&self) -> Option<PathBuf> {
        self.codex_home
            .clone()
            .or_else(|| dirs::home_dir().map(|h| h.join(".codex")))
    }

    /// Resolves the codex sessions directory (`~/.codex/sessions` or custom override).
    pub fn resolve_sessions_dir(&self) -> Option<PathBuf> {
        self.resolve_codex_home().map(|h| h.join("sessions"))
    }

    /// Returns `true` if the `codex` binary is found in `PATH` or the `~/.codex/` directory exists.
    pub fn is_installed(&self) -> bool {
        if let Some(home) = self.resolve_codex_home() {
            if home.is_dir() {
                return true;
            }
        }
        is_binary_in_path("codex")
    }

    /// Returns the path to the instruction file (`AGENTS.md`) within the target project directory.
    pub fn get_instruction_path(&self, project_dir: &Path) -> PathBuf {
        project_dir.join("AGENTS.md")
    }

    /// Formats handoff state into a structured `AGENTS.md` instructions document.
    pub fn format_instructions(&self, handoff: &Handoff) -> String {
        let mut md = String::new();

        // 1. Project
        if handoff.project_name.trim().is_empty() {
            md.push_str("# Project\n\n");
        } else {
            md.push_str(&format!("# Project: {}\n\n", handoff.project_name.trim()));
        }

        // 2. Last Session
        md.push_str("## Last Session\n\n");
        md.push_str(&format!(
            "- **Timestamp:** {}\n",
            handoff.created_at.to_rfc3339()
        ));
        if !handoff.source_agent.trim().is_empty() {
            md.push_str(&format!(
                "- **Source Agent:** {}\n",
                handoff.source_agent.trim()
            ));
        }
        if !handoff.source_machine.trim().is_empty() {
            md.push_str(&format!(
                "- **Source Machine:** {}\n",
                handoff.source_machine.trim()
            ));
        }
        if !handoff.git_branch.trim().is_empty() || !handoff.git_commit.trim().is_empty() {
            let branch = if handoff.git_branch.trim().is_empty() {
                "unknown"
            } else {
                handoff.git_branch.trim()
            };
            let commit = if handoff.git_commit.trim().is_empty() {
                "head"
            } else {
                handoff.git_commit.trim()
            };
            md.push_str(&format!("- **Git:** {} ({})\n", branch, commit));
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

        // 4. Completed
        md.push_str("## Completed\n\n");
        if handoff.completed.is_empty() {
            md.push_str("None recorded.\n\n");
        } else {
            for item in &handoff.completed {
                md.push_str(&item.to_markdown());
                md.push('\n');
            }
            md.push('\n');
        }

        // 5. In Progress
        md.push_str("## In Progress\n\n");
        if handoff.in_progress.is_empty() {
            md.push_str("None.\n\n");
        } else {
            for item in &handoff.in_progress {
                md.push_str(&item.to_markdown());
                md.push('\n');
            }
            md.push('\n');
        }

        // 6. Pending
        md.push_str("## Pending\n\n");
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
            md.push_str("None recorded.\n\n");
        } else {
            for decision in &handoff.decisions {
                md.push_str(&decision.to_markdown());
                md.push('\n');
            }
            md.push('\n');
        }

        // 8. Security directive
        md.push_str("## Security Directive\n\n");
        md.push_str(
            "> **CRITICAL:** Never store secrets in plaintext. Never log secret values. Encrypt before network transit.\n",
        );

        md
    }

    /// Extracts handoff state by scanning the provided sessions directory for the most recent rollout file.
    pub fn extract_handoff_from_dir(
        &self,
        sessions_dir: &Path,
        project_dir: &Path,
    ) -> Result<Handoff> {
        let most_recent = find_most_recent_rollout_file(sessions_dir)?;
        let project_name = project_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        parse_rollout_file(&most_recent, project_name)
    }
}

impl AgentAdapter for CodexAdapter {
    fn name(&self) -> &str {
        "codex"
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
        let sessions_dir = self.resolve_sessions_dir().ok_or_else(|| {
            AdapterError::SessionDirectoryNotFound(PathBuf::from("~/.codex/sessions"))
        })?;
        self.extract_handoff_from_dir(&sessions_dir, project_dir)
    }

    fn launch_command(&self) -> &str {
        "codex"
    }
}

/// Recursively searches for rollout JSONL files within the given directory.
fn find_rollout_files(dir: &Path, results: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            find_rollout_files(&path, results)?;
        } else if path.is_file() {
            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if (file_name.starts_with("rollout") || file_name.contains("rollout"))
                && file_name.ends_with(".jsonl")
            {
                results.push(path);
            }
        }
    }
    Ok(())
}

/// Recursively searches for any JSONL files within the given directory as fallback.
fn find_all_jsonl_files(dir: &Path, results: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            find_all_jsonl_files(&path, results)?;
        } else if path.is_file() {
            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if file_name.ends_with(".jsonl") {
                results.push(path);
            }
        }
    }
    Ok(())
}

/// Finds the most recent rollout JSONL file inside the provided sessions directory.
pub fn find_most_recent_rollout_file(sessions_dir: &Path) -> Result<PathBuf> {
    if !sessions_dir.is_dir() {
        return Err(AdapterError::SessionDirectoryNotFound(
            sessions_dir.to_path_buf(),
        ));
    }

    let mut rollout_files = Vec::new();
    find_rollout_files(sessions_dir, &mut rollout_files)?;

    if rollout_files.is_empty() {
        // Fallback: search for any .jsonl file in sessions dir
        find_all_jsonl_files(sessions_dir, &mut rollout_files)?;
    }

    if rollout_files.is_empty() {
        return Err(AdapterError::NoRolloutFiles(sessions_dir.to_path_buf()));
    }

    // Sort by modified time with filename tie-breaking (since ISO timestamps are in names)
    let most_recent = rollout_files
        .into_iter()
        .max_by(|a, b| {
            let meta_a = fs::metadata(a).and_then(|m| m.modified()).ok();
            let meta_b = fs::metadata(b).and_then(|m| m.modified()).ok();
            match (meta_a, meta_b) {
                (Some(ta), Some(tb)) if ta != tb => ta.cmp(&tb),
                _ => a.file_name().cmp(&b.file_name()),
            }
        })
        .ok_or_else(|| AdapterError::NoRolloutFiles(sessions_dir.to_path_buf()))?;

    Ok(most_recent)
}

/// Reads and parses a rollout JSONL file, extracting user messages and state into a [`Handoff`].
pub fn parse_rollout_file(path: &Path, project_name: &str) -> Result<Handoff> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);

    let mut session_cwd: Option<String> = None;
    let mut session_timestamp: Option<DateTime<Utc>> = None;
    let mut user_messages: Vec<(Option<DateTime<Utc>>, String)> = Vec::new();

    for line_res in reader.lines() {
        let line = line_res?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let val: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let item_type = val.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let ts = val.get("timestamp").and_then(|t| t.as_str()).and_then(|s| {
            DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        });

        if item_type == "session_meta" {
            if let Some(payload) = val.get("payload") {
                if let Some(cwd) = payload.get("cwd").and_then(|c| c.as_str()) {
                    session_cwd = Some(cwd.to_string());
                }
            }
            if session_timestamp.is_none() {
                session_timestamp = ts;
            }
        } else if item_type == "response_item" {
            if let Some(payload) = val.get("payload") {
                let role = payload.get("role").and_then(|r| r.as_str()).unwrap_or("");
                if role == "user" {
                    if let Some(content) = payload.get("content") {
                        let text = extract_text_from_value(content);
                        if !text.trim().is_empty() {
                            user_messages.push((ts, text));
                        }
                    }
                }
            }
        }
    }

    let final_project_name = if !project_name.trim().is_empty() && project_name != "unknown" {
        project_name.to_string()
    } else if let Some(ref cwd) = session_cwd {
        Path::new(cwd)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("codex-project")
            .to_string()
    } else {
        "codex-project".to_string()
    };

    let mut handoff = Handoff::for_project(final_project_name);
    handoff.source_agent = "codex".to_string();
    handoff.source_machine = get_machine_name();

    if user_messages.is_empty() {
        handoff.created_at = session_timestamp.unwrap_or_else(Utc::now);
        handoff.summary = "Codex session recorded but no user messages found.".to_string();
        return Ok(handoff);
    }

    // Filter out internal system-injected messages from primary candidate if user prompts exist
    let substantive_messages: Vec<&(Option<DateTime<Utc>>, String)> = user_messages
        .iter()
        .filter(|(_, text)| {
            let t = text.trim();
            !t.starts_with("<recommended_plugins>")
                && !t.starts_with("<multi_agent_mode>")
                && !t.starts_with(
                    "The following is the Codex agent history whose request action you are assessing.",
                )
        })
        .collect();

    let (latest_ts, primary_text) = if let Some(sub) = substantive_messages.last() {
        (sub.0, &sub.1)
    } else {
        let last = user_messages.last().expect("user_messages is not empty");
        (last.0, &last.1)
    };

    handoff.created_at = latest_ts.or(session_timestamp).unwrap_or_else(Utc::now);

    // Parse state (tasks, decisions, blockers, summary) from primary text and all substantive messages
    let (
        parsed_summary,
        mut completed,
        mut in_progress,
        mut pending,
        mut decisions,
        mut blockers,
    ) = parse_markdown_state(primary_text);

    // If there were other substantive messages with tasks or decisions, aggregate them
    let other_substantive = if substantive_messages.is_empty() {
        &[][..]
    } else {
        &substantive_messages[..substantive_messages.len().saturating_sub(1)]
    };
    for msg in other_substantive {
        let (_, c, ip, p, d, b) = parse_markdown_state(&msg.1);
        for item in c {
            if !completed.contains(&item) {
                completed.push(item);
            }
        }
        for item in ip {
            if !in_progress.contains(&item) {
                in_progress.push(item);
            }
        }
        for item in p {
            if !pending.contains(&item) {
                pending.push(item);
            }
        }
        for item in d {
            if !decisions
                .iter()
                .any(|existing| existing.what == item.what && existing.why == item.why)
            {
                decisions.push(item);
            }
        }
        for item in b {
            if !blockers.contains(&item) {
                blockers.push(item);
            }
        }
    }

    handoff.summary = parsed_summary.unwrap_or_else(|| primary_text.trim().to_string());
    handoff.completed = completed;
    handoff.in_progress = in_progress;
    handoff.pending = pending;
    handoff.decisions = decisions;
    handoff.blockers = blockers;

    if user_messages.len() > 1 {
        let mut notes_buf = String::new();
        notes_buf.push_str(&format!(
            "Session contained {} user response item(s).\n\nLast user message:\n{}",
            user_messages.len(),
            primary_text.trim()
        ));
        handoff.notes = Some(notes_buf);
    }

    Ok(handoff)
}

/// Helper function to extract text content from an arbitrary JSON value.
fn extract_text_from_value(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => {
            let mut parts = Vec::new();
            for item in arr {
                match item {
                    serde_json::Value::String(s) => parts.push(s.clone()),
                    serde_json::Value::Object(obj) => {
                        if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                            parts.push(text.to_string());
                        }
                    }
                    _ => {}
                }
            }
            parts.join("\n")
        }
        serde_json::Value::Object(obj) => {
            if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                text.to_string()
            } else {
                String::new()
            }
        }
        _ => String::new(),
    }
}

/// Parses state (summary, completed, in_progress, pending, decisions, blockers) from markdown text.
fn parse_markdown_state(
    text: &str,
) -> (
    Option<String>,
    Vec<TaskItem>,
    Vec<TaskItem>,
    Vec<TaskItem>,
    Vec<Decision>,
    Vec<String>,
) {
    let mut summary = None;
    let mut completed: Vec<TaskItem> = Vec::new();
    let mut in_progress: Vec<TaskItem> = Vec::new();
    let mut pending: Vec<TaskItem> = Vec::new();
    let mut decisions: Vec<Decision> = Vec::new();
    let mut blockers: Vec<String> = Vec::new();

    let mut current_section = "";
    let mut summary_lines = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("# ") || trimmed.starts_with("## ") || trimmed.starts_with("### ") {
            let header = trimmed
                .trim_start_matches('#')
                .trim()
                .to_ascii_lowercase();
            if header.contains("summary") || header.contains("current state") {
                current_section = "summary";
            } else if header.contains("completed") {
                current_section = "completed";
            } else if header.contains("in progress") {
                current_section = "in_progress";
            } else if header.contains("pending") {
                current_section = "pending";
            } else if header.contains("decision") {
                current_section = "decisions";
            } else if header.contains("blocker") {
                current_section = "blockers";
            } else {
                current_section = "";
            }
            continue;
        }

        if current_section == "summary" {
            if !trimmed.is_empty() {
                summary_lines.push(trimmed);
            }
            continue;
        }

        // Check for checkbox tasks:
        // - [x] or - [X]
        if let Some(rest) = trimmed
            .strip_prefix("- [x]")
            .or_else(|| trimmed.strip_prefix("- [X]"))
            .or_else(|| trimmed.strip_prefix("* [x]"))
            .or_else(|| trimmed.strip_prefix("* [X]"))
        {
            let (desc, detail) = split_task_desc_detail(rest.trim());
            let mut item = TaskItem::done(desc);
            if let Some(d) = detail {
                item = item.with_detail(d);
            }
            completed.push(item);
            continue;
        }

        // - [-]
        if let Some(rest) = trimmed
            .strip_prefix("- [-]")
            .or_else(|| trimmed.strip_prefix("* [-]"))
        {
            let (desc, detail) = split_task_desc_detail(rest.trim());
            let mut item = TaskItem::partial(desc);
            if let Some(d) = detail {
                item = item.with_detail(d);
            }
            in_progress.push(item);
            continue;
        }

        // - [ ]
        if let Some(rest) = trimmed
            .strip_prefix("- [ ]")
            .or_else(|| trimmed.strip_prefix("* [ ]"))
        {
            let (desc, detail) = split_task_desc_detail(rest.trim());
            let mut item = TaskItem::pending(desc);
            if let Some(d) = detail {
                item = item.with_detail(d);
            }
            pending.push(item);
            continue;
        }

        // Decisions: - **what**: why
        if current_section == "decisions" || trimmed.to_ascii_lowercase().starts_with("decision:") {
            if let Some(rest) = trimmed.strip_prefix("- **") {
                if let Some((what, why_part)) = rest.split_once("**:") {
                    let why = why_part.trim();
                    let what = what.trim();
                    if !decisions
                        .iter()
                        .any(|d| d.what == what && d.why == why)
                    {
                        decisions.push(Decision::now(what, why));
                    }
                    continue;
                }
            } else if let Some(rest) = trimmed.strip_prefix("Decision:") {
                let rest = rest.trim();
                let (what, why) = rest
                    .split_once(':')
                    .or_else(|| rest.split_once('-'))
                    .unwrap_or((rest, ""));
                let what = what.trim();
                let why = why.trim();
                if !decisions
                    .iter()
                    .any(|d| d.what == what && d.why == why)
                {
                    decisions.push(Decision::now(what, why));
                }
                continue;
            }
        }

        // Blockers
        if current_section == "blockers" {
            let item = trimmed.trim_start_matches('-').trim_start_matches('*').trim();
            if !item.is_empty() {
                blockers.push(item.to_string());
            }
            continue;
        } else if let Some(rest) = trimmed.strip_prefix("Blocker:") {
            let b = rest.trim();
            if !b.is_empty() {
                blockers.push(b.to_string());
            }
            continue;
        }
    }

    if !summary_lines.is_empty() {
        summary = Some(summary_lines.join("\n"));
    }

    (summary, completed, in_progress, pending, decisions, blockers)
}

/// Splits a task line into description and optional detail separated by " - ".
fn split_task_desc_detail(s: &str) -> (&str, Option<&str>) {
    if let Some((desc, detail)) = s.split_once(" - ") {
        (desc.trim(), Some(detail.trim()))
    } else {
        (s.trim(), None)
    }
}

/// Determines whether a given binary exists in any directory listed in `PATH`.
fn is_binary_in_path(binary: &str) -> bool {
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

/// Returns the host or machine name.
fn get_machine_name() -> String {
    std::env::var("CTX_MACHINE_NAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "unknown-machine".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_instruction_path_and_launch_command() {
        let adapter = CodexAdapter::new();
        assert_eq!(adapter.name(), "codex");
        assert_eq!(adapter.launch_command(), "codex");

        let project_dir = Path::new("/workspace/project_abc");
        assert_eq!(
            adapter.instruction_path(project_dir),
            PathBuf::from("/workspace/project_abc/AGENTS.md")
        );
        assert_eq!(
            adapter.get_instruction_path(project_dir),
            PathBuf::from("/workspace/project_abc/AGENTS.md")
        );
    }

    #[test]
    fn test_generate_instructions_contains_required_sections() {
        let adapter = CodexAdapter::new();
        let mut handoff = Handoff::for_project("ctx")
            .with_summary("Refactoring core adapters")
            .with_source("Codex", "macbook-pro")
            .with_git("main", "abc1234");

        handoff.add_completed(TaskItem::done("Setup adapter trait").with_detail("Includes tests"));
        handoff.add_in_progress(TaskItem::partial("Implement Codex adapter"));
        handoff.add_pending(TaskItem::pending("Implement Claude adapter"));
        handoff.add_decision(Decision::new(
            "Use AGENTS.md",
            "Codex reads AGENTS.md natively",
            Utc::now(),
        ));

        let instructions = adapter.generate_instructions(&handoff);

        // Verify Project
        assert!(instructions.contains("# Project: ctx"));
        // Verify Last Session
        assert!(instructions.contains("## Last Session"));
        assert!(instructions.contains("- **Source Agent:** Codex"));
        assert!(instructions.contains("- **Source Machine:** macbook-pro"));
        assert!(instructions.contains("- **Git:** main (abc1234)"));
        // Verify Current State
        assert!(instructions.contains("## Current State"));
        assert!(instructions.contains("Refactoring core adapters"));
        // Verify Completed
        assert!(instructions.contains("## Completed"));
        assert!(instructions.contains("- [x] Setup adapter trait - Includes tests"));
        // Verify In Progress
        assert!(instructions.contains("## In Progress"));
        assert!(instructions.contains("- [-] Implement Codex adapter"));
        // Verify Pending
        assert!(instructions.contains("## Pending"));
        assert!(instructions.contains("- [ ] Implement Claude adapter"));
        // Verify Decisions
        assert!(instructions.contains("## Decisions"));
        assert!(instructions.contains("- **Use AGENTS.md**: Codex reads AGENTS.md natively"));
        // Verify Security Directive
        assert!(instructions.contains("## Security Directive"));
        assert!(instructions.contains("Never store secrets in plaintext. Never log secret values. Encrypt before network transit."));
    }

    #[test]
    fn test_extract_handoff_from_rollout_jsonl() {
        let temp_dir = std::env::temp_dir().join(format!(
            "codex_test_sessions_{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(12345)
        ));
        let session_sub_dir = temp_dir.join("2026").join("09").join("04");
        fs::create_dir_all(&session_sub_dir).expect("Failed to create temp session dir");

        // Create older rollout file
        let old_file = session_sub_dir.join("rollout-2026-09-04T10-00-00-11111111-1111-1111-1111-111111111111.jsonl");
        let old_content = r#"{"type":"session_meta","payload":{"cwd":"/tmp/old_project"}}
{"type":"response_item","payload":{"role":"user","content":[{"type":"input_text","text":"Old task"}]}}
"#;
        fs::write(&old_file, old_content).expect("Failed to write old rollout file");

        // Wait slightly or create newer file with higher timestamp name
        let new_file = session_sub_dir.join("rollout-2026-09-04T12-00-00-22222222-2222-2222-2222-222222222222.jsonl");
        let new_content = r###"{"type":"session_meta","payload":{"cwd":"/tmp/new_project"}}
{"type":"response_item","payload":{"role":"developer","content":[{"type":"input_text","text":"System prompt"}]}}
{"type":"response_item","payload":{"role":"user","content":[{"type":"input_text","text":"## Summary\nImplemented payment gateway\n\n## Tasks\n- [x] Stripe webhook - Verified signatures\n- [-] Refund API\n- [ ] Unit tests\n\n## Decisions\n- **Webhooks**: Use HMAC verification\n\nBlocker: Waiting for merchant keys"}]}}
"###;
        fs::write(&new_file, new_content).expect("Failed to write new rollout file");

        let adapter = CodexAdapter::with_codex_home(&temp_dir);
        let project_dir = Path::new("/workspace/my_project");

        let handoff = adapter
            .extract_handoff_from_dir(&temp_dir, project_dir)
            .expect("Handoff extraction should succeed");

        assert_eq!(handoff.project_name, "my_project");
        assert_eq!(handoff.source_agent, "codex");
        assert_eq!(handoff.summary, "Implemented payment gateway");

        assert_eq!(handoff.completed.len(), 1);
        assert_eq!(handoff.completed[0].description, "Stripe webhook");
        assert_eq!(
            handoff.completed[0].detail.as_deref(),
            Some("Verified signatures")
        );

        assert_eq!(handoff.in_progress.len(), 1);
        assert_eq!(handoff.in_progress[0].description, "Refund API");

        assert_eq!(handoff.pending.len(), 1);
        assert_eq!(handoff.pending[0].description, "Unit tests");

        assert_eq!(handoff.decisions.len(), 1);
        assert_eq!(handoff.decisions[0].what, "Webhooks");
        assert_eq!(handoff.decisions[0].why, "Use HMAC verification");

        assert_eq!(handoff.blockers, vec!["Waiting for merchant keys"]);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_extract_handoff_with_string_content() {
        let temp_dir = std::env::temp_dir().join(format!(
            "codex_test_string_content_{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(67890)
        ));
        fs::create_dir_all(&temp_dir).expect("Failed to create temp dir");

        let rollout_file = temp_dir.join("rollout-2026-09-04T15-00-00-33333333.jsonl");
        let content = r#"{"type":"response_item","payload":{"role":"user","content":"Simple user message with a pending task:\n- [ ] Deploy container"}}
"#;
        fs::write(&rollout_file, content).expect("Failed to write rollout");

        let adapter = CodexAdapter::new();
        let handoff = adapter
            .extract_handoff_from_dir(&temp_dir, Path::new("/workspace/test_app"))
            .expect("Extraction must succeed");

        assert_eq!(handoff.project_name, "test_app");
        assert!(handoff.summary.contains("Simple user message"));
        assert_eq!(handoff.pending.len(), 1);
        assert_eq!(handoff.pending[0].description, "Deploy container");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_extract_handoff_empty_and_missing_directory() {
        let adapter = CodexAdapter::new();

        // Non-existent directory
        let missing = Path::new("/non/existent/path/for/codex/test");
        let res = adapter.extract_handoff_from_dir(missing, Path::new("/test"));
        assert!(matches!(res, Err(AdapterError::SessionDirectoryNotFound(_))));

        // Empty directory
        let temp_dir = std::env::temp_dir().join(format!(
            "codex_test_empty_{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(99999)
        ));
        fs::create_dir_all(&temp_dir).expect("Failed to create temp dir");

        let res_empty = adapter.extract_handoff_from_dir(&temp_dir, Path::new("/test"));
        assert!(matches!(res_empty, Err(AdapterError::NoRolloutFiles(_))));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_detect_installed() {
        let temp_dir = std::env::temp_dir().join(format!(
            "codex_test_install_{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(54321)
        ));
        fs::create_dir_all(&temp_dir).expect("Failed to create temp dir");

        let adapter = CodexAdapter::with_codex_home(&temp_dir);
        assert!(adapter.detect_installed());

        let non_existent = Path::new("/non/existent/dir/for/codex/home");
        let adapter_missing = CodexAdapter::with_codex_home(non_existent);
        // If binary is not in PATH, detect_installed should be false; if in PATH, true
        let expected = is_binary_in_path("codex");
        assert_eq!(adapter_missing.detect_installed(), expected);

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
