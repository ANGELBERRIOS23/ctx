//! Project handoff data models and serialization for `ctx`.
//!
//! A handoff captures the state of an agent's work on a project, including
//! completed tasks, in-progress work, pending items, architectural decisions,
//! blockers, notes, and environment hints. It facilitates seamless context
//! transfers across agents and machines.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The execution status of a specific task item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// The task has been completed.
    #[serde(alias = "Done", alias = "DONE")]
    Done,
    /// The task is partially completed or currently being worked on.
    #[serde(alias = "Partial", alias = "PARTIAL")]
    Partial,
    /// The task is queued and has not yet been started.
    #[serde(alias = "Pending", alias = "PENDING")]
    Pending,
}

impl TaskStatus {
    /// Returns the markdown checkbox representation for this status.
    ///
    /// - `Done` -> `"[x]"`
    /// - `Partial` -> `"[-]"`
    /// - `Pending` -> `"[ ]"`
    pub fn as_checkbox(&self) -> &'static str {
        match self {
            TaskStatus::Done => "[x]",
            TaskStatus::Partial => "[-]",
            TaskStatus::Pending => "[ ]",
        }
    }
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskStatus::Done => write!(f, "done"),
            TaskStatus::Partial => write!(f, "partial"),
            TaskStatus::Pending => write!(f, "pending"),
        }
    }
}

/// An individual task entry within a handoff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskItem {
    /// Brief description of the task.
    pub description: String,
    /// Current completion status of the task.
    pub status: TaskStatus,
    /// Optional additional details, context, or next steps for the task.
    pub detail: Option<String>,
}

impl TaskItem {
    /// Creates a new [`TaskItem`] with the given description and status.
    pub fn new(description: impl Into<String>, status: TaskStatus) -> Self {
        Self {
            description: description.into(),
            status,
            detail: None,
        }
    }

    /// Attaches supplementary detail to the task item.
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Creates a new completed task item ([`TaskStatus::Done`]).
    pub fn done(description: impl Into<String>) -> Self {
        Self::new(description, TaskStatus::Done)
    }

    /// Creates a new partially completed task item ([`TaskStatus::Partial`]).
    pub fn partial(description: impl Into<String>) -> Self {
        Self::new(description, TaskStatus::Partial)
    }

    /// Creates a new pending task item ([`TaskStatus::Pending`]).
    pub fn pending(description: impl Into<String>) -> Self {
        Self::new(description, TaskStatus::Pending)
    }

    /// Formats this task item as a markdown list entry with its checkbox.
    pub fn to_markdown(&self) -> String {
        match &self.detail {
            Some(detail) if !detail.trim().is_empty() => {
                format!(
                    "- {} {} - {}",
                    self.status.as_checkbox(),
                    self.description,
                    detail.trim()
                )
            }
            _ => format!("- {} {}", self.status.as_checkbox(), self.description),
        }
    }
}

/// Record of an architectural or implementation decision made during the session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    /// Description of the decision that was made.
    pub what: String,
    /// Rationale explaining why the decision was made.
    pub why: String,
    /// UTC timestamp when the decision was recorded.
    pub when: DateTime<Utc>,
}

impl Decision {
    /// Creates a new [`Decision`] record with an explicit timestamp.
    pub fn new(what: impl Into<String>, why: impl Into<String>, when: DateTime<Utc>) -> Self {
        Self {
            what: what.into(),
            why: why.into(),
            when,
        }
    }

    /// Creates a new [`Decision`] record timestamped with the current UTC time.
    pub fn now(what: impl Into<String>, why: impl Into<String>) -> Self {
        Self::new(what, why, Utc::now())
    }

    /// Formats the decision as a markdown list entry.
    pub fn to_markdown(&self) -> String {
        format!(
            "- **{}**: {} *(when: {})*",
            self.what,
            self.why,
            self.when.to_rfc3339()
        )
    }
}

/// Comprehensive project state snapshot captured for agent and machine handoffs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Handoff {
    /// Schema version for compatibility across different releases.
    pub version: u32,
    /// Name of the project associated with this handoff.
    pub project_name: String,
    /// Timestamp when this handoff snapshot was created in UTC.
    pub created_at: DateTime<Utc>,
    /// Identifier or name of the AI agent that produced this handoff.
    pub source_agent: String,
    /// Hostname or machine identifier where the handoff was generated.
    pub source_machine: String,
    /// Active git branch during this session.
    pub git_branch: String,
    /// Active git commit hash during this session.
    pub git_commit: String,
    /// High-level summary of work performed and current project state.
    pub summary: String,
    /// List of tasks completed during this session.
    pub completed: Vec<TaskItem>,
    /// List of tasks currently in progress.
    pub in_progress: Vec<TaskItem>,
    /// List of tasks pending future execution.
    pub pending: Vec<TaskItem>,
    /// Architectural or technical decisions made during the session.
    pub decisions: Vec<Decision>,
    /// Active issues or external impediments blocking progress.
    pub blockers: Vec<String>,
    /// Additional unstructured notes, context, or instructions.
    pub notes: Option<String>,
    /// Relative or workspace file paths modified during this session.
    pub files_modified: Vec<String>,
    /// Environmental prerequisites, setup hints, or runtime notes.
    pub environment_hints: Vec<String>,
}

impl Default for Handoff {
    fn default() -> Self {
        Self::new()
    }
}

impl Handoff {
    /// Creates a new empty [`Handoff`] instance with default values.
    pub fn new() -> Self {
        Self {
            version: 1,
            project_name: String::new(),
            created_at: Utc::now(),
            source_agent: String::new(),
            source_machine: String::new(),
            git_branch: String::new(),
            git_commit: String::new(),
            summary: String::new(),
            completed: Vec::new(),
            in_progress: Vec::new(),
            pending: Vec::new(),
            decisions: Vec::new(),
            blockers: Vec::new(),
            notes: None,
            files_modified: Vec::new(),
            environment_hints: Vec::new(),
        }
    }

    /// Creates a new [`Handoff`] initialized with a specific project name.
    pub fn for_project(project_name: impl Into<String>) -> Self {
        let mut handoff = Self::new();
        handoff.project_name = project_name.into();
        handoff
    }

    /// Sets the summary field.
    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = summary.into();
        self
    }

    /// Sets the source agent and source machine metadata.
    pub fn with_source(
        mut self,
        source_agent: impl Into<String>,
        source_machine: impl Into<String>,
    ) -> Self {
        self.source_agent = source_agent.into();
        self.source_machine = source_machine.into();
        self
    }

    /// Sets the git branch and commit metadata.
    pub fn with_git(mut self, branch: impl Into<String>, commit: impl Into<String>) -> Self {
        self.git_branch = branch.into();
        self.git_commit = commit.into();
        self
    }

    /// Sets the unstructured notes field.
    pub fn with_notes(mut self, notes: impl Into<String>) -> Self {
        self.notes = Some(notes.into());
        self
    }

    /// Adds a completed task item to the handoff.
    pub fn add_completed(&mut self, item: TaskItem) {
        self.completed.push(item);
    }

    /// Adds an in-progress task item to the handoff.
    pub fn add_in_progress(&mut self, item: TaskItem) {
        self.in_progress.push(item);
    }

    /// Adds a pending task item to the handoff.
    pub fn add_pending(&mut self, item: TaskItem) {
        self.pending.push(item);
    }

    /// Records a technical decision in the handoff.
    pub fn add_decision(&mut self, decision: Decision) {
        self.decisions.push(decision);
    }

    /// Records a blocker in the handoff.
    pub fn add_blocker(&mut self, blocker: impl Into<String>) {
        self.blockers.push(blocker.into());
    }

    /// Records a modified file path in the handoff.
    pub fn add_file_modified(&mut self, file: impl Into<String>) {
        self.files_modified.push(file.into());
    }

    /// Records an environment hint in the handoff.
    pub fn add_environment_hint(&mut self, hint: impl Into<String>) {
        self.environment_hints.push(hint.into());
    }

    /// Checks whether the handoff has any substantive content.
    ///
    /// Returns `true` if all task lists, summary, decisions, blockers,
    /// notes, modified files, and environment hints are empty or whitespace-only.
    pub fn is_empty(&self) -> bool {
        self.summary.trim().is_empty()
            && self.completed.is_empty()
            && self.in_progress.is_empty()
            && self.pending.is_empty()
            && self.decisions.is_empty()
            && self.blockers.is_empty()
            && self.notes.as_deref().unwrap_or("").trim().is_empty()
            && self.files_modified.is_empty()
            && self.environment_hints.is_empty()
    }

    /// Serializes this handoff to a structured Markdown document.
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();

        if self.project_name.trim().is_empty() {
            md.push_str("# Project Handoff\n\n");
        } else {
            md.push_str(&format!("# Handoff: {}\n\n", self.project_name));
        }

        // Metadata
        md.push_str(&format!(
            "- **Created:** {}\n",
            self.created_at.to_rfc3339()
        ));
        if !self.source_agent.trim().is_empty() {
            md.push_str(&format!("- **Source Agent:** {}\n", self.source_agent));
        }
        if !self.source_machine.trim().is_empty() {
            md.push_str(&format!("- **Source Machine:** {}\n", self.source_machine));
        }
        if !self.git_branch.trim().is_empty() {
            md.push_str(&format!("- **Git Branch:** {}\n", self.git_branch));
        }
        if !self.git_commit.trim().is_empty() {
            md.push_str(&format!("- **Git Commit:** {}\n", self.git_commit));
        }
        md.push('\n');

        // Summary
        if !self.summary.trim().is_empty() {
            md.push_str("## Summary\n\n");
            md.push_str(self.summary.trim());
            md.push_str("\n\n");
        }

        // Tasks
        let has_tasks =
            !self.completed.is_empty() || !self.in_progress.is_empty() || !self.pending.is_empty();

        if has_tasks {
            md.push_str("## Tasks\n\n");

            if !self.completed.is_empty() {
                md.push_str("### Completed\n\n");
                for item in &self.completed {
                    md.push_str(&item.to_markdown());
                    md.push('\n');
                }
                md.push('\n');
            }

            if !self.in_progress.is_empty() {
                md.push_str("### In Progress\n\n");
                for item in &self.in_progress {
                    md.push_str(&item.to_markdown());
                    md.push('\n');
                }
                md.push('\n');
            }

            if !self.pending.is_empty() {
                md.push_str("### Pending\n\n");
                for item in &self.pending {
                    md.push_str(&item.to_markdown());
                    md.push('\n');
                }
                md.push('\n');
            }
        }

        // Decisions
        if !self.decisions.is_empty() {
            md.push_str("## Decisions\n\n");
            for decision in &self.decisions {
                md.push_str(&decision.to_markdown());
                md.push('\n');
            }
            md.push('\n');
        }

        // Blockers
        if !self.blockers.is_empty() {
            md.push_str("## Blockers\n\n");
            for blocker in &self.blockers {
                md.push_str(&format!("- {}\n", blocker));
            }
            md.push('\n');
        }

        // Files Modified
        if !self.files_modified.is_empty() {
            md.push_str("## Files Modified\n\n");
            for file in &self.files_modified {
                md.push_str(&format!("- `{}`\n", file));
            }
            md.push('\n');
        }

        // Environment Hints
        if !self.environment_hints.is_empty() {
            md.push_str("## Environment Hints\n\n");
            for hint in &self.environment_hints {
                md.push_str(&format!("- {}\n", hint));
            }
            md.push('\n');
        }

        // Notes
        if let Some(notes) = self.notes.as_deref().filter(|n| !n.trim().is_empty()) {
            md.push_str("## Notes\n\n");
            md.push_str(notes.trim());
            md.push_str("\n\n");
        }

        md
    }

    /// Serializes this handoff into a JSON string.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserializes a [`Handoff`] from a JSON string.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_and_is_empty() {
        let mut handoff = Handoff::new();
        assert_eq!(handoff.version, 1);
        assert!(handoff.is_empty());

        handoff.project_name = "ctx".to_string();
        assert!(handoff.is_empty());

        handoff.summary = "Initial handoff".to_string();
        assert!(!handoff.is_empty());

        handoff.summary.clear();
        assert!(handoff.is_empty());

        handoff.add_completed(TaskItem::done("Setup repo"));
        assert!(!handoff.is_empty());
    }

    #[test]
    fn test_to_markdown() {
        let mut handoff = Handoff::for_project("ctx")
            .with_summary("Refactoring core types")
            .with_source("Claude Code", "dev-box")
            .with_git("main", "a1b2c3d")
            .with_notes("Remember to run cargo test.");

        handoff.add_completed(
            TaskItem::done("Implement Handoff struct").with_detail("Includes tests"),
        );
        handoff.add_in_progress(TaskItem::partial("Write server handlers"));
        handoff.add_pending(TaskItem::pending("CLI integration"));
        handoff.add_decision(Decision::new(
            "Use workspace Cargo.toml",
            "Keep dependencies centralized",
            Utc::now(),
        ));
        handoff.add_blocker("Need MinIO credentials for cloud sync tests");
        handoff.add_file_modified("crates/ctx-core/src/handoff.rs");
        handoff.add_environment_hint("RUST_LOG=debug cargo test");

        let md = handoff.to_markdown();

        assert!(md.contains("# Handoff: ctx"));
        assert!(md.contains("**Source Agent:** Claude Code"));
        assert!(md.contains("**Source Machine:** dev-box"));
        assert!(md.contains("**Git Branch:** main"));
        assert!(md.contains("**Git Commit:** a1b2c3d"));
        assert!(md.contains("## Summary"));
        assert!(md.contains("Refactoring core types"));
        assert!(md.contains("## Tasks"));
        assert!(md.contains("### Completed"));
        assert!(md.contains("- [x] Implement Handoff struct - Includes tests"));
        assert!(md.contains("### In Progress"));
        assert!(md.contains("- [-] Write server handlers"));
        assert!(md.contains("### Pending"));
        assert!(md.contains("- [ ] CLI integration"));
        assert!(md.contains("## Decisions"));
        assert!(md.contains("Use workspace Cargo.toml"));
        assert!(md.contains("Keep dependencies centralized"));
        assert!(md.contains("## Blockers"));
        assert!(md.contains("Need MinIO credentials for cloud sync tests"));
        assert!(md.contains("## Files Modified"));
        assert!(md.contains("`crates/ctx-core/src/handoff.rs`"));
        assert!(md.contains("## Environment Hints"));
        assert!(md.contains("RUST_LOG=debug cargo test"));
        assert!(md.contains("## Notes"));
        assert!(md.contains("Remember to run cargo test."));
    }

    #[test]
    fn test_serde_roundtrip_and_defaults() {
        let mut original = Handoff::for_project("ctx")
            .with_summary("Testing serialization")
            .with_source("Codex", "linux-laptop");
        original.add_completed(TaskItem::done("Serialize handoff"));
        original.add_decision(Decision::new(
            "Format as JSON",
            "Standard interoperability",
            Utc::now(),
        ));

        let json = original.to_json().expect("Serialization must succeed");
        let deserialized = Handoff::from_json(&json).expect("Deserialization must succeed");

        assert_eq!(original, deserialized);

        // Test deserialization with minimal JSON (verifying #[serde(default)])
        let minimal_json = r#"{"project_name": "minimal"}"#;
        let minimal_handoff: Handoff =
            serde_json::from_str(minimal_json).expect("Minimal JSON should deserialize");
        assert_eq!(minimal_handoff.project_name, "minimal");
        assert_eq!(minimal_handoff.version, 1);
        assert!(minimal_handoff.completed.is_empty());
        assert!(minimal_handoff.is_empty());
    }

    #[test]
    fn test_task_item_and_status() {
        let done_item = TaskItem::done("Finished task");
        assert_eq!(done_item.status, TaskStatus::Done);
        assert_eq!(done_item.to_markdown(), "- [x] Finished task");

        let partial_item = TaskItem::partial("Working on task").with_detail("50% done");
        assert_eq!(partial_item.status, TaskStatus::Partial);
        assert_eq!(
            partial_item.to_markdown(),
            "- [-] Working on task - 50% done"
        );

        let pending_item = TaskItem::pending("Queued task");
        assert_eq!(pending_item.status, TaskStatus::Pending);
        assert_eq!(pending_item.to_markdown(), "- [ ] Queued task");

        assert_eq!(TaskStatus::Done.as_checkbox(), "[x]");
        assert_eq!(TaskStatus::Partial.as_checkbox(), "[-]");
        assert_eq!(TaskStatus::Pending.as_checkbox(), "[ ]");

        assert_eq!(TaskStatus::Done.to_string(), "done");
        assert_eq!(TaskStatus::Partial.to_string(), "partial");
        assert_eq!(TaskStatus::Pending.to_string(), "pending");
    }

    #[test]
    fn test_decision_to_markdown() {
        let timestamp = Utc::now();
        let decision = Decision::new(
            "Adopt Axum 0.8",
            "Better ergonomics and async support",
            timestamp,
        );
        let md = decision.to_markdown();
        assert!(md.contains("Adopt Axum 0.8"));
        assert!(md.contains("Better ergonomics and async support"));
        assert!(md.contains(&timestamp.to_rfc3339()));
    }
}
