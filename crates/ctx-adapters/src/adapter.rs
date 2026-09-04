//! Base adapter trait and error types for AI coding agent integrations.
//!
//! This module defines the [`AgentAdapter`] trait which standardizes interactions
//! with different AI coding assistants (such as OpenAI Codex, Claude Code, Cursor, etc.),
//! along with the [`AdapterError`] enum.

use std::path::{Path, PathBuf};
use ctx_core::error::CtxError;
use ctx_core::handoff::Handoff;
use thiserror::Error;

pub use crate::generic::SessionMatch;

/// Error type for adapter operations.
#[derive(Debug, Error)]
pub enum AdapterError {
    /// An I/O error occurred while interacting with the filesystem.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A JSON serialization or deserialization error occurred.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// The agent session directory could not be found.
    #[error("Session directory not found: {0}")]
    SessionDirectoryNotFound(PathBuf),

    /// No rollout or session files were found in the session directory.
    #[error("No rollout session files found in: {0}")]
    NoRolloutFiles(PathBuf),

    /// No session history or log files were found for this agent.
    #[error("No session found in: {0}")]
    NoSessionFound(PathBuf),

    /// Failed to extract or synthesize handoff state from session logs.
    #[error("Failed to extract handoff: {0}")]
    ExtractionFailed(String),

    /// The user's home directory could not be resolved.
    #[error("Failed to determine user home directory")]
    MissingHomeDir,

    /// Parsing session data failed.
    #[error("Failed to parse session data: {0}")]
    Parse(String),

    /// General adapter error.
    #[error("Adapter error: {0}")]
    Other(String),
}

/// A specialized Result type for adapter operations.
pub type Result<T> = std::result::Result<T, AdapterError>;

impl From<CtxError> for AdapterError {
    fn from(err: CtxError) -> Self {
        Self::Other(err.to_string())
    }
}

impl From<AdapterError> for CtxError {
    fn from(err: AdapterError) -> Self {
        match err {
            AdapterError::Io(source) => CtxError::Io(source),
            AdapterError::Json(source) => CtxError::Config(source.to_string()),
            AdapterError::ExtractionFailed(msg) | AdapterError::Parse(msg) => {
                CtxError::Handoff(msg)
            }
            AdapterError::NoSessionFound(path)
            | AdapterError::SessionDirectoryNotFound(path)
            | AdapterError::NoRolloutFiles(path) => {
                CtxError::NotFound(format!("Session not found: {}", path.display()))
            }
            AdapterError::MissingHomeDir => {
                CtxError::Config("Missing home directory".to_string())
            }
            AdapterError::Other(msg) => CtxError::Handoff(msg),
        }
    }
}

/// Common trait implemented by all AI coding agent adapters.
///
/// Implementors provide agent-specific installation detection, instruction path
/// formatting, instruction file generation, session handoff extraction, and launch commands.
pub trait AgentAdapter: Send + Sync {
    /// Returns the unique identifier name of this agent adapter (e.g., `"codex"` or `"claude"`).
    fn name(&self) -> &str;

    /// Checks whether the agent's binary or runtime environment is installed locally.
    fn detect_installed(&self) -> bool;

    /// Returns the path to the instructions file within the target project directory.
    fn instruction_path(&self, project_dir: &Path) -> PathBuf;

    /// Formats handoff state into instructions tailored for this agent.
    fn generate_instructions(&self, handoff: &Handoff) -> String;

    /// Extracts project handoff state from the agent's local session history.
    fn extract_handoff(&self, project_dir: &Path) -> Result<Handoff>;

    /// Returns the command line executable used to launch this agent.
    fn launch_command(&self) -> &str;

    /// Searches past sessions for this agent matching the query string.
    fn search_sessions(&self, _query: &str) -> Vec<SessionMatch> {
        Vec::new()
    }

    /// Lists recent sessions for this agent within the specified number of days.
    fn list_recent_sessions(&self, _days: u32) -> Vec<SessionMatch> {
        Vec::new()
    }
}

/// Creates an agent adapter instance based on the provided agent identifier name.
///
/// Supported agent names:
/// - `"claude"`, `"claude-code"`, `"claudecode"` -> [`crate::claude::ClaudeAdapter`]
/// - `"codex"`, `"openai-codex"` -> [`crate::codex::CodexAdapter`]
/// - `"cursor"` -> [`crate::cursor::CursorAdapter`]
/// - `"opencode"` -> [`crate::opencode::OpenCodeAdapter`]
/// - Any other name -> [`crate::generic::GenericAdapter`]
pub fn create_adapter(name: &str) -> Box<dyn AgentAdapter> {
    match name.to_ascii_lowercase().as_str() {
        "claude" | "claude-code" | "claudecode" => Box::new(crate::claude::ClaudeAdapter::new()),
        "codex" | "openai-codex" => Box::new(crate::codex::CodexAdapter::new()),
        "cursor" => Box::new(crate::cursor::CursorAdapter::new()),
        "opencode" => Box::new(crate::opencode::OpenCodeAdapter::new()),
        _ => Box::new(crate::generic::GenericAdapter::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockAdapter;

    impl AgentAdapter for MockAdapter {
        fn name(&self) -> &str {
            "mock"
        }

        fn detect_installed(&self) -> bool {
            true
        }

        fn instruction_path(&self, project_dir: &Path) -> PathBuf {
            project_dir.join("MOCK.md")
        }

        fn generate_instructions(&self, handoff: &Handoff) -> String {
            format!("# Instructions for {}", handoff.project_name)
        }

        fn extract_handoff(&self, project_dir: &Path) -> Result<Handoff> {
            let mut h = Handoff::new();
            h.project_name = project_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("default")
                .to_string();
            Ok(h)
        }

        fn launch_command(&self) -> &str {
            "mock-agent"
        }
    }

    #[test]
    fn test_adapter_trait_methods() {
        let adapter = MockAdapter;
        assert_eq!(adapter.name(), "mock");
        assert!(adapter.detect_installed());
        assert_eq!(
            adapter.instruction_path(Path::new("/tmp/test")),
            PathBuf::from("/tmp/test/MOCK.md")
        );
        let handoff = Handoff::for_project("my-project");
        let instructions = adapter.generate_instructions(&handoff);
        assert_eq!(instructions, "# Instructions for my-project");
        assert_eq!(adapter.launch_command(), "mock-agent");

        let extracted = adapter
            .extract_handoff(Path::new("/tmp/sample-dir"))
            .expect("Mock extract must succeed");
        assert_eq!(extracted.project_name, "sample-dir");
    }

    #[test]
    fn test_adapter_error_display() {
        let not_found_err = AdapterError::SessionDirectoryNotFound(PathBuf::from("/non/existent"));
        assert!(not_found_err.to_string().contains("/non/existent"));

        let no_rollouts_err = AdapterError::NoRolloutFiles(PathBuf::from("/empty/sessions"));
        assert!(no_rollouts_err.to_string().contains("/empty/sessions"));

        let parse_err = AdapterError::Parse("corrupted token".to_string());
        assert_eq!(
            parse_err.to_string(),
            "Failed to parse session data: corrupted token"
        );

        let other_err = AdapterError::Other("custom error".to_string());
        assert_eq!(other_err.to_string(), "Adapter error: custom error");

        let missing_home = AdapterError::MissingHomeDir;
        assert_eq!(
            missing_home.to_string(),
            "Failed to determine user home directory"
        );

        let extraction = AdapterError::ExtractionFailed("corrupted session".to_string());
        assert_eq!(
            extraction.to_string(),
            "Failed to extract handoff: corrupted session"
        );

        let ctx_err: CtxError = extraction.into();
        match ctx_err {
            CtxError::Handoff(msg) => assert_eq!(msg, "corrupted session"),
            _ => panic!("Expected CtxError::Handoff"),
        }
    }

    #[test]
    fn test_create_adapter() {
        assert_eq!(create_adapter("claude").name(), "claude");
        assert_eq!(create_adapter("claude-code").name(), "claude");
        assert_eq!(create_adapter("codex").name(), "codex");
        assert_eq!(create_adapter("cursor").name(), "cursor");
        assert_eq!(create_adapter("opencode").name(), "opencode");
        assert_eq!(create_adapter("unknown").name(), "generic");
    }
}
