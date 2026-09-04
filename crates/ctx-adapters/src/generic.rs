//! Fallback adapter for unknown AI coding agents or manual usage, and universal search helpers.
//!
//! Provides [`GenericAdapter`] which implements [`AgentAdapter`] for fallback/generic
//! scenarios. Uses `CONTEXT.md` as the instruction file and reads handoff state
//! from `.ctx/handoff.md`.
//!
//! Also provides universal multi-agent search utilities:
//! - [`search_all_agents`]: Searches across all agent adapters and ranks by relevance.
//! - [`list_all_recent`]: Aggregates recent sessions across all agent adapters.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use ctx_core::handoff::{Decision, Handoff, TaskItem, TaskStatus};

use crate::adapter::{AdapterError, AgentAdapter, Result};

/// Represents a matched session from an AI coding agent's history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionMatch {
    /// Identifier of the session (e.g., filename, session ID, or hash).
    pub session_id: String,
    /// Identifier or name of the agent adapter that produced or recorded this session.
    pub agent: String,
    /// Name of the project associated with this session, if known.
    pub project_name: Option<String>,
    /// Filesystem path to the session file or directory, if available.
    pub path: Option<PathBuf>,
    /// Summary or brief description of the session.
    pub summary: Option<String>,
    /// Contextual text snippet matching the query.
    pub snippet: Option<String>,
    /// Timestamp when the session was created or last updated.
    pub timestamp: Option<DateTime<Utc>>,
    /// Relevance score for ranking search results (higher values indicate greater relevance).
    pub relevance_score: f64,
}

impl SessionMatch {
    /// Creates a new [`SessionMatch`] with the given session ID, agent name, and relevance score.
    pub fn new(session_id: impl Into<String>, agent: impl Into<String>, relevance_score: f64) -> Self {
        Self {
            session_id: session_id.into(),
            agent: agent.into(),
            project_name: None,
            path: None,
            summary: None,
            snippet: None,
            timestamp: None,
            relevance_score,
        }
    }

    /// Sets the associated project name.
    pub fn with_project_name(mut self, project_name: impl Into<String>) -> Self {
        self.project_name = Some(project_name.into());
        self
    }

    /// Sets the session file or directory path.
    pub fn with_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Sets the session summary.
    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    /// Sets the matched snippet.
    pub fn with_snippet(mut self, snippet: impl Into<String>) -> Self {
        self.snippet = Some(snippet.into());
        self
    }

    /// Sets the session timestamp.
    pub fn with_timestamp(mut self, timestamp: DateTime<Utc>) -> Self {
        self.timestamp = Some(timestamp);
        self
    }
}

/// Fallback agent adapter for unknown agents or manual use.
///
/// This adapter always reports as installed, uses `CONTEXT.md` as its instruction path,
/// generates generic markdown instructions, extracts handoffs from `.ctx/handoff.md`
/// if present, and returns empty launch commands and session searches.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenericAdapter;

impl GenericAdapter {
    /// Creates a new [`GenericAdapter`].
    pub fn new() -> Self {
        Self
    }

    /// Parses handoff markdown content into a [`Handoff`] snapshot.
    pub fn parse_handoff_markdown(content: &str, fallback_project_name: &str) -> Result<Handoff> {
        // Fallback: If content is JSON formatted, parse directly.
        if let Ok(handoff) = serde_json::from_str::<Handoff>(content) {
            return Ok(handoff);
        }

        let mut handoff = Handoff::new();
        handoff.project_name = fallback_project_name.to_string();

        #[derive(Debug, PartialEq, Eq)]
        enum Section {
            None,
            Summary,
            Tasks(Option<TaskStatus>),
            Decisions,
            Blockers,
            FilesModified,
            EnvironmentHints,
            Notes,
            Ignored,
        }

        let mut current_section = Section::None;
        let mut summary_lines = Vec::new();
        let mut notes_lines = Vec::new();

        for raw_line in content.lines() {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }

            // Top-level document heading: "# Handoff: project_name" or "# Project: project_name"
            if let Some(heading) = line.strip_prefix("# ") {
                let heading = heading.trim();
                if let Some((_, name)) = heading.split_once(':') {
                    let trimmed = name.trim();
                    if !trimmed.is_empty() {
                        handoff.project_name = trimmed.to_string();
                    }
                } else if !heading.is_empty()
                    && !heading.eq_ignore_ascii_case("Project Handoff")
                    && !heading.eq_ignore_ascii_case("Project Context & Handoff")
                    && !heading.eq_ignore_ascii_case("Project")
                {
                    handoff.project_name = heading.to_string();
                }
                continue;
            }

            // Section headings: "## Section Title"
            if let Some(heading) = line.strip_prefix("## ") {
                let h = heading.trim().to_lowercase();
                if h.contains("current state") || h.contains("summary") {
                    current_section = Section::Summary;
                } else if h.contains("completed") {
                    current_section = Section::Tasks(Some(TaskStatus::Done));
                } else if h.contains("in progress") {
                    current_section = Section::Tasks(Some(TaskStatus::Partial));
                } else if h.contains("pending") {
                    current_section = Section::Tasks(Some(TaskStatus::Pending));
                } else if h.contains("task") {
                    current_section = Section::Tasks(None);
                } else if h.contains("decision") {
                    current_section = Section::Decisions;
                } else if h.contains("blocker") {
                    current_section = Section::Blockers;
                } else if h.contains("file") {
                    current_section = Section::FilesModified;
                } else if h.contains("environment") || h.contains("hint") {
                    current_section = Section::EnvironmentHints;
                } else if h.contains("note") {
                    current_section = Section::Notes;
                } else if h.contains("security") || h.contains("metadata") {
                    current_section = Section::Ignored;
                } else {
                    current_section = Section::None;
                }
                continue;
            }

            // Subheadings: "### Tasks Sub-category"
            if let Some(subheading) = line.strip_prefix("### ") {
                let sh = subheading.trim().to_lowercase();
                if sh.contains("completed") {
                    current_section = Section::Tasks(Some(TaskStatus::Done));
                } else if sh.contains("in progress") {
                    current_section = Section::Tasks(Some(TaskStatus::Partial));
                } else if sh.contains("pending") {
                    current_section = Section::Tasks(Some(TaskStatus::Pending));
                }
                continue;
            }

            // Metadata items outside sections or in None/Ignored sections
            if matches!(current_section, Section::None | Section::Ignored) {
                if let Some(val) = extract_metadata_bullet(line, "Created")
                    .or_else(|| extract_metadata_bullet(line, "Timestamp"))
                {
                    if let Ok(ts) = DateTime::parse_from_rfc3339(val) {
                        handoff.created_at = ts.with_timezone(&Utc);
                    }
                    continue;
                }
                if let Some(val) = extract_metadata_bullet(line, "Source Agent")
                    .or_else(|| extract_metadata_bullet(line, "Agent"))
                {
                    handoff.source_agent = val.to_string();
                    continue;
                }
                if let Some(val) = extract_metadata_bullet(line, "Source Machine")
                    .or_else(|| extract_metadata_bullet(line, "Machine"))
                {
                    handoff.source_machine = val.to_string();
                    continue;
                }
                if let Some(val) = extract_metadata_bullet(line, "Git Branch") {
                    handoff.git_branch = val.to_string();
                    continue;
                }
                if let Some(val) = extract_metadata_bullet(line, "Git Commit") {
                    handoff.git_commit = val.to_string();
                    continue;
                }
                if let Some(val) = extract_metadata_bullet(line, "Git") {
                    if let Some((branch, commit_part)) = val.split_once('(') {
                        handoff.git_branch = branch.trim().to_string();
                        let commit = commit_part.trim_end_matches(')').trim();
                        handoff.git_commit = commit.to_string();
                    } else {
                        handoff.git_branch = val.to_string();
                    }
                    continue;
                }
            }

            // Content within specific sections
            match &current_section {
                Section::Summary => {
                    summary_lines.push(line);
                }
                Section::Tasks(default_status) => {
                    if let Some(task) = parse_task_item(line, *default_status) {
                        match task.status {
                            TaskStatus::Done => handoff.add_completed(task),
                            TaskStatus::Partial => handoff.add_in_progress(task),
                            TaskStatus::Pending => handoff.add_pending(task),
                        }
                    }
                }
                Section::Decisions => {
                    if let Some(decision) = parse_decision_item(line) {
                        handoff.add_decision(decision);
                    }
                }
                Section::Blockers => {
                    if let Some(blocker) = strip_markdown_bullet(line) {
                        handoff.add_blocker(blocker);
                    }
                }
                Section::FilesModified => {
                    if let Some(file) = strip_markdown_bullet(line) {
                        let cleaned = file.trim().trim_matches('`').trim();
                        if !cleaned.is_empty() {
                            handoff.add_file_modified(cleaned);
                        }
                    }
                }
                Section::EnvironmentHints => {
                    if let Some(hint) = strip_markdown_bullet(line) {
                        handoff.add_environment_hint(hint);
                    }
                }
                Section::Notes => {
                    notes_lines.push(line);
                }
                Section::None | Section::Ignored => {}
            }
        }

        if !summary_lines.is_empty() {
            let combined = summary_lines.join("\n");
            if combined != "No summary provided." {
                handoff.summary = combined;
            }
        }

        if !notes_lines.is_empty() {
            handoff.notes = Some(notes_lines.join("\n"));
        }

        Ok(handoff)
    }
}

impl AgentAdapter for GenericAdapter {
    fn name(&self) -> &str {
        "generic"
    }

    fn detect_installed(&self) -> bool {
        true
    }

    fn instruction_path(&self, project_dir: &Path) -> PathBuf {
        project_dir.join("CONTEXT.md")
    }

    fn generate_instructions(&self, handoff: &Handoff) -> String {
        let mut md = String::new();

        // 1. Title
        if handoff.project_name.trim().is_empty() {
            md.push_str("# Project Context & Handoff\n\n");
        } else {
            md.push_str(&format!(
                "# Project Context & Handoff: {}\n\n",
                handoff.project_name.trim()
            ));
        }

        // 2. Session Metadata
        md.push_str("## Session Metadata\n\n");
        md.push_str(&format!(
            "- **Timestamp:** {}\n",
            handoff.created_at.to_rfc3339()
        ));
        if !handoff.source_agent.trim().is_empty() {
            md.push_str(&format!("- **Source Agent:** {}\n", handoff.source_agent.trim()));
        }
        if !handoff.source_machine.trim().is_empty() {
            md.push_str(&format!(
                "- **Source Machine:** {}\n",
                handoff.source_machine.trim()
            ));
        }
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

        // 4. Tasks
        let has_tasks = !handoff.completed.is_empty()
            || !handoff.in_progress.is_empty()
            || !handoff.pending.is_empty();

        if has_tasks {
            md.push_str("## Tasks\n\n");

            if !handoff.completed.is_empty() {
                md.push_str("### Completed\n\n");
                for item in &handoff.completed {
                    md.push_str(&item.to_markdown());
                    md.push('\n');
                }
                md.push('\n');
            }

            if !handoff.in_progress.is_empty() {
                md.push_str("### In Progress\n\n");
                for item in &handoff.in_progress {
                    md.push_str(&item.to_markdown());
                    md.push('\n');
                }
                md.push('\n');
            }

            if !handoff.pending.is_empty() {
                md.push_str("### Pending\n\n");
                for item in &handoff.pending {
                    md.push_str(&item.to_markdown());
                    md.push('\n');
                }
                md.push('\n');
            }
        }

        // 5. Decisions
        if !handoff.decisions.is_empty() {
            md.push_str("## Decisions\n\n");
            for decision in &handoff.decisions {
                md.push_str(&decision.to_markdown());
                md.push('\n');
            }
            md.push('\n');
        }

        // 6. Blockers
        if !handoff.blockers.is_empty() {
            md.push_str("## Blockers\n\n");
            for blocker in &handoff.blockers {
                md.push_str(&format!("- {}\n", blocker));
            }
            md.push('\n');
        }

        // 7. Files Modified
        if !handoff.files_modified.is_empty() {
            md.push_str("## Files Modified\n\n");
            for file in &handoff.files_modified {
                md.push_str(&format!("- `{}`\n", file));
            }
            md.push('\n');
        }

        // 8. Environment Hints
        if !handoff.environment_hints.is_empty() {
            md.push_str("## Environment Hints\n\n");
            for hint in &handoff.environment_hints {
                md.push_str(&format!("- {}\n", hint));
            }
            md.push('\n');
        }

        // 9. Notes
        if let Some(notes) = handoff.notes.as_deref().filter(|n| !n.trim().is_empty()) {
            md.push_str("## Notes\n\n");
            md.push_str(notes.trim());
            md.push_str("\n\n");
        }

        // 10. Security Directive
        md.push_str("## Security Directive\n\n");
        md.push_str(
            "- Do NOT store, commit, or hardcode secrets, tokens, or credentials in project files.\n",
        );
        md.push_str(
            "- All sensitive credentials must be accessed strictly through environment variables provided by the vault.\n",
        );
        md.push_str("- Never print decrypted secret values into session transcripts or logs.\n");

        md
    }

    fn extract_handoff(&self, project_dir: &Path) -> Result<Handoff> {
        let handoff_path = project_dir.join(".ctx").join("handoff.md");
        let fallback_name = project_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unnamed");

        if handoff_path.is_file() {
            let content = fs::read_to_string(&handoff_path).map_err(AdapterError::Io)?;
            Self::parse_handoff_markdown(&content, fallback_name)
        } else {
            Ok(Handoff::for_project(fallback_name))
        }
    }

    fn search_sessions(&self, _query: &str) -> Vec<SessionMatch> {
        Vec::new()
    }

    fn list_recent_sessions(&self, _days: u32) -> Vec<SessionMatch> {
        Vec::new()
    }

    fn launch_command(&self) -> &str {
        ""
    }
}

/// Searches all provided agent adapters for sessions matching the query.
///
/// Calls [`AgentAdapter::search_sessions`] on each adapter, collects and aggregates
/// results, sorts them by [`SessionMatch::relevance_score`] descending (with timestamp
/// as secondary tiebreaker), and returns the unified list of matches.
pub async fn search_all_agents(
    query: &str,
    adapters: &[Box<dyn AgentAdapter>],
) -> Vec<SessionMatch> {
    let mut results = Vec::new();
    for adapter in adapters {
        results.extend(adapter.search_sessions(query));
    }
    results.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.timestamp.cmp(&a.timestamp))
    });
    results
}

/// Lists recent sessions across all provided agent adapters within the specified days.
///
/// Calls [`AgentAdapter::list_recent_sessions`] on each adapter, collects and aggregates
/// results, sorts them by [`SessionMatch::relevance_score`] descending (with timestamp
/// as secondary tiebreaker), and returns the unified list of matches.
pub async fn list_all_recent(
    days: u32,
    adapters: &[Box<dyn AgentAdapter>],
) -> Vec<SessionMatch> {
    let mut results = Vec::new();
    for adapter in adapters {
        results.extend(adapter.list_recent_sessions(days));
    }
    results.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.timestamp.cmp(&a.timestamp))
    });
    results
}

/// Strips `- ` or `* ` list prefixes from markdown lines.
fn strip_markdown_bullet(line: &str) -> Option<&str> {
    if let Some(rest) = line.strip_prefix("- ") {
        Some(rest.trim())
    } else if let Some(rest) = line.strip_prefix("* ") {
        Some(rest.trim())
    } else {
        None
    }
}

/// Extracts a metadata value from a bullet line like `- **Key:** value`.
fn extract_metadata_bullet<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let line = strip_markdown_bullet(line)?;
    let line = line.strip_prefix("**")?;
    let (k, rest) = line.split_once("**")?;
    let k_clean = k.trim().trim_end_matches(':');
    let key_clean = key.trim().trim_end_matches(':');

    if k_clean.eq_ignore_ascii_case(key_clean) {
        let val = rest.strip_prefix(':').unwrap_or(rest).trim();
        return Some(val);
    }
    None
}

/// Parses a task list item like `- [x] description - detail` or `- description`.
fn parse_task_item(line: &str, default_status: Option<TaskStatus>) -> Option<TaskItem> {
    let line = strip_markdown_bullet(line)?;

    let (status, text) = if let Some(rest) = line.strip_prefix("[x] ").or_else(|| line.strip_prefix("[X] ")) {
        (TaskStatus::Done, rest.trim())
    } else if let Some(rest) = line.strip_prefix("[-] ") {
        (TaskStatus::Partial, rest.trim())
    } else if let Some(rest) = line.strip_prefix("[ ] ") {
        (TaskStatus::Pending, rest.trim())
    } else if let Some(st) = default_status {
        (st, line)
    } else {
        return None;
    };

    if text.is_empty() {
        return None;
    }

    if let Some((desc, detail)) = text.split_once(" - ") {
        let desc = desc.trim();
        let detail = detail.trim();
        if !detail.is_empty() {
            return Some(TaskItem::new(desc, status).with_detail(detail));
        }
        Some(TaskItem::new(desc, status))
    } else {
        Some(TaskItem::new(text, status))
    }
}

/// Parses a decision bullet like `- **what**: why *(when: 2026-09-04T15:00:00Z)*`.
fn parse_decision_item(line: &str) -> Option<Decision> {
    let line = strip_markdown_bullet(line)?;

    if let Some(rest) = line.strip_prefix("**") {
        if let Some((what, remainder)) = rest.split_once("**") {
            let remainder = remainder.strip_prefix(':').unwrap_or(remainder).trim();
            let (why, when) = if let Some((w, when_part)) = remainder.split_once("*(when:") {
                let when_str = when_part
                    .trim()
                    .trim_end_matches('*')
                    .trim_end_matches(')')
                    .trim();
                let when_dt = DateTime::parse_from_rfc3339(when_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                (w.trim(), when_dt)
            } else {
                (remainder, Utc::now())
            };
            return Some(Decision::new(what.trim(), why, when));
        }
    }

    if let Some((what, why)) = line.split_once(':') {
        Some(Decision::now(what.trim(), why.trim()))
    } else {
        Some(Decision::now(line, ""))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn unique_test_dir(prefix: &str) -> PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let count = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("{prefix}_{nanos}_{count}"))
    }

    struct MockAgent {
        name: &'static str,
        search_results: Vec<SessionMatch>,
        recent_results: Vec<SessionMatch>,
    }

    impl AgentAdapter for MockAgent {
        fn name(&self) -> &str {
            self.name
        }

        fn detect_installed(&self) -> bool {
            true
        }

        fn instruction_path(&self, project_dir: &Path) -> PathBuf {
            project_dir.join("MOCK.md")
        }

        fn generate_instructions(&self, handoff: &Handoff) -> String {
            format!("# {}", handoff.project_name)
        }

        fn extract_handoff(&self, _project_dir: &Path) -> Result<Handoff> {
            Ok(Handoff::new())
        }

        fn launch_command(&self) -> &str {
            "mock"
        }

        fn search_sessions(&self, _query: &str) -> Vec<SessionMatch> {
            self.search_results.clone()
        }

        fn list_recent_sessions(&self, _days: u32) -> Vec<SessionMatch> {
            self.recent_results.clone()
        }
    }

    #[test]
    fn test_generic_adapter_methods() {
        let adapter = GenericAdapter::new();
        assert_eq!(adapter.name(), "generic");
        assert!(adapter.detect_installed());
        assert_eq!(
            adapter.instruction_path(Path::new("/workspace/sample")),
            PathBuf::from("/workspace/sample/CONTEXT.md")
        );
        assert_eq!(adapter.launch_command(), "");
        assert!(adapter.search_sessions("test").is_empty());
        assert!(adapter.list_recent_sessions(7).is_empty());
    }

    #[test]
    fn test_generate_instructions_formatting() {
        let adapter = GenericAdapter::new();
        let mut handoff = Handoff::for_project("ctx-workspace")
            .with_summary("Refactored generic agent adapter")
            .with_source("manual", "workstation")
            .with_git("feature/generic", "c0ffee")
            .with_notes("Run cargo check before committing");

        handoff.add_completed(TaskItem::done("Define SessionMatch").with_detail("Includes tests"));
        handoff.add_in_progress(TaskItem::partial("Implement search_all_agents"));
        handoff.add_pending(TaskItem::pending("CLI integration"));
        handoff.add_decision(Decision::new(
            "Use CONTEXT.md",
            "Consistent fallback document",
            Utc::now(),
        ));
        handoff.add_blocker("Waiting on API tokens");
        handoff.add_file_modified("crates/ctx-adapters/src/generic.rs");
        handoff.add_environment_hint("RUST_LOG=info");

        let md = adapter.generate_instructions(&handoff);

        assert!(md.contains("# Project Context & Handoff: ctx-workspace"));
        assert!(md.contains("**Source Agent:** manual"));
        assert!(md.contains("**Source Machine:** workstation"));
        assert!(md.contains("**Git Branch:** feature/generic"));
        assert!(md.contains("**Git Commit:** c0ffee"));
        assert!(md.contains("## Current State"));
        assert!(md.contains("Refactored generic agent adapter"));
        assert!(md.contains("### Completed"));
        assert!(md.contains("- [x] Define SessionMatch - Includes tests"));
        assert!(md.contains("### In Progress"));
        assert!(md.contains("- [-] Implement search_all_agents"));
        assert!(md.contains("### Pending"));
        assert!(md.contains("- [ ] CLI integration"));
        assert!(md.contains("## Decisions"));
        assert!(md.contains("Use CONTEXT.md"));
        assert!(md.contains("## Blockers"));
        assert!(md.contains("- Waiting on API tokens"));
        assert!(md.contains("## Files Modified"));
        assert!(md.contains("- `crates/ctx-adapters/src/generic.rs`"));
        assert!(md.contains("## Environment Hints"));
        assert!(md.contains("- RUST_LOG=info"));
        assert!(md.contains("## Notes"));
        assert!(md.contains("Run cargo check before committing"));
        assert!(md.contains("## Security Directive"));
    }

    #[test]
    fn test_extract_handoff_from_existing_file() {
        let adapter = GenericAdapter::new();
        let temp_dir = unique_test_dir("ctx_test_generic_extract");
        let ctx_dir = temp_dir.join(".ctx");
        fs::create_dir_all(&ctx_dir).expect("Failed to create .ctx directory");

        let handoff_content = r#"# Handoff: test-project
- **Created:** 2026-09-04T12:00:00Z
- **Source Agent:** custom-bot
- **Source Machine:** dev-laptop
- **Git Branch:** main
- **Git Commit:** 1234567

## Current State
Extracting handoff state from file.

## Tasks
### Completed
- [x] Initial setup - finished successfully
### In Progress
- [-] Writing tests
### Pending
- [ ] Documentation

## Decisions
- **Architecture**: Modular crates *(when: 2026-09-04T12:00:00Z)*

## Blockers
- Missing network access

## Files Modified
- `crates/ctx-adapters/src/generic.rs`

## Environment Hints
- export RUST_BACKTRACE=1

## Notes
Check edge cases thoroughly.
"#;

        let handoff_path = ctx_dir.join("handoff.md");
        fs::write(&handoff_path, handoff_content).expect("Failed to write handoff.md");

        let handoff = adapter
            .extract_handoff(&temp_dir)
            .expect("Handoff extraction should succeed");

        assert_eq!(handoff.project_name, "test-project");
        assert_eq!(handoff.source_agent, "custom-bot");
        assert_eq!(handoff.source_machine, "dev-laptop");
        assert_eq!(handoff.git_branch, "main");
        assert_eq!(handoff.git_commit, "1234567");
        assert_eq!(handoff.summary, "Extracting handoff state from file.");
        assert_eq!(handoff.completed.len(), 1);
        assert_eq!(handoff.completed[0].description, "Initial setup");
        assert_eq!(
            handoff.completed[0].detail.as_deref(),
            Some("finished successfully")
        );
        assert_eq!(handoff.in_progress.len(), 1);
        assert_eq!(handoff.in_progress[0].description, "Writing tests");
        assert_eq!(handoff.pending.len(), 1);
        assert_eq!(handoff.pending[0].description, "Documentation");
        assert_eq!(handoff.decisions.len(), 1);
        assert_eq!(handoff.decisions[0].what, "Architecture");
        assert_eq!(handoff.blockers, vec!["Missing network access"]);
        assert_eq!(
            handoff.files_modified,
            vec!["crates/ctx-adapters/src/generic.rs"]
        );
        assert_eq!(
            handoff.environment_hints,
            vec!["export RUST_BACKTRACE=1"]
        );
        assert_eq!(
            handoff.notes.as_deref(),
            Some("Check edge cases thoroughly.")
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_extract_handoff_missing_file() {
        let adapter = GenericAdapter::new();
        let temp_dir = unique_test_dir("ctx_test_generic_missing");
        fs::create_dir_all(&temp_dir).expect("Failed to create temp directory");

        let handoff = adapter
            .extract_handoff(&temp_dir)
            .expect("Extraction on missing file should return default project handoff");

        let expected_name = temp_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unnamed");
        assert_eq!(handoff.project_name, expected_name);
        assert!(handoff.is_empty());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_extract_handoff_json_fallback() {
        let adapter = GenericAdapter::new();
        let temp_dir = unique_test_dir("ctx_test_generic_json");
        let ctx_dir = temp_dir.join(".ctx");
        fs::create_dir_all(&ctx_dir).expect("Failed to create .ctx directory");

        let original = Handoff::for_project("json-proj")
            .with_summary("JSON encoded handoff")
            .with_source("agent-json", "box");
        let json_str = serde_json::to_string(&original).expect("JSON serialization must succeed");

        fs::write(ctx_dir.join("handoff.md"), json_str).expect("Failed to write handoff.md");

        let extracted = adapter
            .extract_handoff(&temp_dir)
            .expect("Extraction from JSON formatted handoff.md must succeed");
        assert_eq!(extracted.project_name, "json-proj");
        assert_eq!(extracted.summary, "JSON encoded handoff");
        assert_eq!(extracted.source_agent, "agent-json");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_search_all_agents_sorting() {
        let agent1: Box<dyn AgentAdapter> = Box::new(MockAgent {
            name: "agent1",
            search_results: vec![
                SessionMatch::new("s1", "agent1", 0.3).with_summary("Match low"),
                SessionMatch::new("s2", "agent1", 0.9).with_summary("Match high"),
            ],
            recent_results: Vec::new(),
        });

        let agent2: Box<dyn AgentAdapter> = Box::new(MockAgent {
            name: "agent2",
            search_results: vec![
                SessionMatch::new("s3", "agent2", 0.6).with_summary("Match mid"),
            ],
            recent_results: Vec::new(),
        });

        let adapters = vec![agent1, agent2];
        let matches = search_all_agents("test", &adapters).await;

        assert_eq!(matches.len(), 3);
        assert_eq!(matches[0].session_id, "s2");
        assert_eq!(matches[0].relevance_score, 0.9);
        assert_eq!(matches[1].session_id, "s3");
        assert_eq!(matches[1].relevance_score, 0.6);
        assert_eq!(matches[2].session_id, "s1");
        assert_eq!(matches[2].relevance_score, 0.3);
    }

    #[tokio::test]
    async fn test_list_all_recent_sorting() {
        let agent1: Box<dyn AgentAdapter> = Box::new(MockAgent {
            name: "agent1",
            search_results: Vec::new(),
            recent_results: vec![
                SessionMatch::new("r1", "agent1", 0.4),
                SessionMatch::new("r2", "agent1", 0.8),
            ],
        });

        let agent2: Box<dyn AgentAdapter> = Box::new(MockAgent {
            name: "agent2",
            search_results: Vec::new(),
            recent_results: vec![
                SessionMatch::new("r3", "agent2", 0.95),
            ],
        });

        let adapters = vec![agent1, agent2];
        let recent = list_all_recent(7, &adapters).await;

        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].session_id, "r3");
        assert_eq!(recent[0].relevance_score, 0.95);
        assert_eq!(recent[1].session_id, "r2");
        assert_eq!(recent[1].relevance_score, 0.8);
        assert_eq!(recent[2].session_id, "r1");
        assert_eq!(recent[2].relevance_score, 0.4);
    }

    #[test]
    fn test_session_match_builder_and_serde() {
        let now = Utc::now();
        let sm = SessionMatch::new("sess-100", "generic", 0.85)
            .with_project_name("my-app")
            .with_path(PathBuf::from("/sessions/sess-100.json"))
            .with_summary("A sample session")
            .with_snippet("fn main() { ... }")
            .with_timestamp(now);

        assert_eq!(sm.session_id, "sess-100");
        assert_eq!(sm.agent, "generic");
        assert_eq!(sm.relevance_score, 0.85);
        assert_eq!(sm.project_name.as_deref(), Some("my-app"));
        assert_eq!(
            sm.path.as_deref(),
            Some(Path::new("/sessions/sess-100.json"))
        );
        assert_eq!(sm.summary.as_deref(), Some("A sample session"));
        assert_eq!(sm.snippet.as_deref(), Some("fn main() { ... }"));
        assert_eq!(sm.timestamp, Some(now));

        let json = serde_json::to_string(&sm).expect("Serialization must succeed");
        let deserialized: SessionMatch =
            serde_json::from_str(&json).expect("Deserialization must succeed");
        assert_eq!(sm, deserialized);
    }
}
