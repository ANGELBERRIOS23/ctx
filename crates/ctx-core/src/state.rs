//! Project state machine and local state persistence for `ctx`.
//!
//! This module defines the state machine for project lifecycles across machines
//! and AI agents:
//! - [`ProjectState`]: Represents the current status of a project (uninitialized,
//!   synced, active, stale, or error).
//! - [`StateTransition`]: Actions that trigger state changes (pull, claim, release,
//!   push, save, timeout, or fail).
//! - [`InvalidTransition`]: Typed error returned when an illegal transition is attempted.
//! - [`StateError`]: Error enum covering invalid transitions, I/O errors, and JSON
//!   serialization failures.
//! - [`LocalState`]: In-memory and on-disk representation stored at `.ctx/state.json`.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::error::CtxError;

/// Error returned when an invalid state transition is attempted on a [`ProjectState`].
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
#[error("invalid transition from state '{from}' via action '{action}': {reason}")]
pub struct InvalidTransition {
    /// The state of the project before the attempted transition.
    pub from: ProjectState,
    /// The transition action that was attempted.
    pub action: StateTransition,
    /// Human-readable explanation of why this transition is invalid.
    pub reason: String,
}

impl InvalidTransition {
    /// Creates a new [`InvalidTransition`] error.
    pub fn new(from: ProjectState, action: StateTransition, reason: impl Into<String>) -> Self {
        Self {
            from,
            action,
            reason: reason.into(),
        }
    }
}

/// Errors that can occur during state machine transitions or state persistence.
#[derive(Debug, Error)]
pub enum StateError {
    /// An invalid state transition was attempted.
    #[error(transparent)]
    InvalidTransition(#[from] InvalidTransition),

    /// An I/O error occurred while interacting with a state file or directory.
    #[error("I/O error at {path}: {source}")]
    Io {
        /// Path where the I/O error occurred.
        path: PathBuf,
        /// Underlying standard I/O error.
        #[source]
        source: std::io::Error,
    },

    /// Failed to parse the JSON state file.
    #[error("Failed to parse JSON state file at {path}: {source}")]
    JsonParse {
        /// Path of the file that failed parsing.
        path: PathBuf,
        /// Underlying JSON deserialization error.
        #[source]
        source: serde_json::Error,
    },

    /// Failed to serialize local state to JSON.
    #[error("Failed to serialize state to JSON: {0}")]
    JsonSerialize(#[source] serde_json::Error),

    /// The state file was not found at the expected path.
    #[error("State file not found at: {0}")]
    NotFound(PathBuf),
}

impl StateError {
    /// Creates a new [`StateError::InvalidTransition`].
    pub fn invalid_transition(
        from: ProjectState,
        action: StateTransition,
        reason: impl Into<String>,
    ) -> Self {
        Self::InvalidTransition(InvalidTransition::new(from, action, reason))
    }

    /// Returns a reference to the inner [`InvalidTransition`] if this is an invalid transition error.
    pub fn as_invalid_transition(&self) -> Option<&InvalidTransition> {
        match self {
            Self::InvalidTransition(inner) => Some(inner),
            _ => None,
        }
    }
}

impl From<InvalidTransition> for CtxError {
    fn from(err: InvalidTransition) -> Self {
        Self::InvalidState(err.to_string())
    }
}

impl From<StateError> for CtxError {
    fn from(err: StateError) -> Self {
        match err {
            StateError::InvalidTransition(e) => Self::InvalidState(e.to_string()),
            StateError::Io { source, .. } => Self::Io(source),
            StateError::JsonParse { source, .. } => Self::Config(source.to_string()),
            StateError::JsonSerialize(source) => Self::Config(source.to_string()),
            StateError::NotFound(path) => Self::NotFound(path.display().to_string()),
        }
    }
}

/// A specialized `Result` type for state operations.
pub type Result<T> = std::result::Result<T, StateError>;

/// Represents the lifecycle state of a project within `ctx`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectState {
    /// The project is newly registered or has not yet synchronized with the remote.
    Uninitialized,
    /// The project is in sync with the remote and no agent is actively working on it.
    Synced,
    /// An agent holds an active session lock on the project and is performing work.
    Active,
    /// The local project state has fallen behind remote changes or a session lock expired.
    Stale,
    /// The project encountered an unrecoverable error or synchronization failure.
    Error(String),
}

impl std::fmt::Display for ProjectState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectState::Uninitialized => write!(f, "uninitialized"),
            ProjectState::Synced => write!(f, "synced"),
            ProjectState::Active => write!(f, "active"),
            ProjectState::Stale => write!(f, "stale"),
            ProjectState::Error(msg) => write!(f, "error: {}", msg),
        }
    }
}

/// Actions or events that trigger a state transition in the project state machine.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateTransition {
    /// Fetch and apply the latest project state from the remote server or peers.
    Pull,
    /// Acquire an exclusive session lock to begin active work with an AI agent.
    Claim,
    /// Release the active session lock without pushing changes to remote.
    Release,
    /// Push committed handoff snapshots and state to the remote server or peers.
    Push,
    /// Save a local checkpoint or handoff snapshot to disk without releasing the session lock.
    Save,
    /// Mark the project as stale due to session lock expiration, heartbeat timeout, or remote drift.
    Timeout,
    /// Transition the project into an error state with a diagnostic message.
    Fail(String),
}

impl std::fmt::Display for StateTransition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StateTransition::Pull => write!(f, "pull"),
            StateTransition::Claim => write!(f, "claim"),
            StateTransition::Release => write!(f, "release"),
            StateTransition::Push => write!(f, "push"),
            StateTransition::Save => write!(f, "save"),
            StateTransition::Timeout => write!(f, "timeout"),
            StateTransition::Fail(msg) => write!(f, "fail: {}", msg),
        }
    }
}

impl ProjectState {
    /// Evaluates a state transition and returns the new [`ProjectState`], or an
    /// [`InvalidTransition`] error if the transition is illegal from the current state.
    ///
    /// # Transition Rules
    /// - **`Uninitialized`**:
    ///   - `Pull` -> `Synced`
    ///   - `Fail(msg)` -> `Error(msg)`
    /// - **`Synced`**:
    ///   - `Pull` -> `Synced` (re-sync)
    ///   - `Claim` -> `Active` (acquire lock)
    ///   - `Timeout` -> `Stale` (remote drift or idle timeout)
    ///   - `Fail(msg)` -> `Error(msg)`
    /// - **`Active`**:
    ///   - `Save` -> `Active` (local checkpoint)
    ///   - `Push` -> `Synced` (sync to remote, release lock)
    ///   - `Release` -> `Synced` (yield lock)
    ///   - `Timeout` -> `Stale` (session heartbeat expired)
    ///   - `Fail(msg)` -> `Error(msg)`
    /// - **`Stale`**:
    ///   - `Pull` -> `Synced` (refresh remote state)
    ///   - `Fail(msg)` -> `Error(msg)`
    /// - **`Error(String)`**:
    ///   - `Pull` -> `Synced` (recover by pulling clean state)
    ///   - `Fail(msg)` -> `Error(msg)` (record new error)
    pub fn transition(&self, action: StateTransition) -> Result<ProjectState> {
        match (self, &action) {
            // Uninitialized transitions
            (ProjectState::Uninitialized, StateTransition::Pull) => Ok(ProjectState::Synced),
            (ProjectState::Uninitialized, StateTransition::Claim) => {
                Err(StateError::invalid_transition(
                    self.clone(),
                    action,
                    "cannot claim an uninitialized project; pull remote state first",
                ))
            }
            (ProjectState::Uninitialized, StateTransition::Release) => {
                Err(StateError::invalid_transition(
                    self.clone(),
                    action,
                    "cannot release an uninitialized project; no active session held",
                ))
            }
            (ProjectState::Uninitialized, StateTransition::Push) => {
                Err(StateError::invalid_transition(
                    self.clone(),
                    action,
                    "cannot push an uninitialized project; can only push from active state",
                ))
            }
            (ProjectState::Uninitialized, StateTransition::Save) => {
                Err(StateError::invalid_transition(
                    self.clone(),
                    action,
                    "cannot save an uninitialized project; can only save from active state",
                ))
            }
            (ProjectState::Uninitialized, StateTransition::Timeout) => {
                Err(StateError::invalid_transition(
                    self.clone(),
                    action,
                    "cannot timeout an uninitialized project",
                ))
            }

            // Synced transitions
            (ProjectState::Synced, StateTransition::Pull) => Ok(ProjectState::Synced),
            (ProjectState::Synced, StateTransition::Claim) => Ok(ProjectState::Active),
            (ProjectState::Synced, StateTransition::Release) => {
                Err(StateError::invalid_transition(
                    self.clone(),
                    action,
                    "cannot release a synced project; no active session held",
                ))
            }
            (ProjectState::Synced, StateTransition::Push) => {
                Err(StateError::invalid_transition(
                    self.clone(),
                    action,
                    "cannot push a synced project; can only push from active state",
                ))
            }
            (ProjectState::Synced, StateTransition::Save) => {
                Err(StateError::invalid_transition(
                    self.clone(),
                    action,
                    "cannot save a synced project; can only save from active state",
                ))
            }
            (ProjectState::Synced, StateTransition::Timeout) => Ok(ProjectState::Stale),

            // Active transitions
            (ProjectState::Active, StateTransition::Pull) => {
                Err(StateError::invalid_transition(
                    self.clone(),
                    action,
                    "cannot pull while project is active; release or push first",
                ))
            }
            (ProjectState::Active, StateTransition::Claim) => {
                Err(StateError::invalid_transition(
                    self.clone(),
                    action,
                    "cannot claim an active project; session is already held",
                ))
            }
            (ProjectState::Active, StateTransition::Release) => Ok(ProjectState::Synced),
            (ProjectState::Active, StateTransition::Push) => Ok(ProjectState::Synced),
            (ProjectState::Active, StateTransition::Save) => Ok(ProjectState::Active),
            (ProjectState::Active, StateTransition::Timeout) => Ok(ProjectState::Stale),

            // Stale transitions
            (ProjectState::Stale, StateTransition::Pull) => Ok(ProjectState::Synced),
            (ProjectState::Stale, StateTransition::Claim) => {
                Err(StateError::invalid_transition(
                    self.clone(),
                    action,
                    "cannot claim a stale project; pull remote state first",
                ))
            }
            (ProjectState::Stale, StateTransition::Release) => {
                Err(StateError::invalid_transition(
                    self.clone(),
                    action,
                    "cannot release a stale project; no active session held",
                ))
            }
            (ProjectState::Stale, StateTransition::Push) => {
                Err(StateError::invalid_transition(
                    self.clone(),
                    action,
                    "cannot push a stale project; can only push from active state",
                ))
            }
            (ProjectState::Stale, StateTransition::Save) => {
                Err(StateError::invalid_transition(
                    self.clone(),
                    action,
                    "cannot save a stale project; can only save from active state",
                ))
            }
            (ProjectState::Stale, StateTransition::Timeout) => {
                Err(StateError::invalid_transition(
                    self.clone(),
                    action,
                    "project is already stale",
                ))
            }

            // Error transitions
            (ProjectState::Error(_), StateTransition::Pull) => Ok(ProjectState::Synced),
            (ProjectState::Error(_), StateTransition::Claim) => {
                Err(StateError::invalid_transition(
                    self.clone(),
                    action,
                    "cannot claim a project in error state; pull remote state first",
                ))
            }
            (ProjectState::Error(_), StateTransition::Release) => {
                Err(StateError::invalid_transition(
                    self.clone(),
                    action,
                    "cannot release a project in error state; no active session held",
                ))
            }
            (ProjectState::Error(_), StateTransition::Push) => {
                Err(StateError::invalid_transition(
                    self.clone(),
                    action,
                    "cannot push a project in error state; can only push from active state",
                ))
            }
            (ProjectState::Error(_), StateTransition::Save) => {
                Err(StateError::invalid_transition(
                    self.clone(),
                    action,
                    "cannot save a project in error state; can only save from active state",
                ))
            }
            (ProjectState::Error(_), StateTransition::Timeout) => {
                Err(StateError::invalid_transition(
                    self.clone(),
                    action,
                    "cannot timeout a project in error state",
                ))
            }

            // Fail transition is valid from any state
            (_, StateTransition::Fail(err)) => Ok(ProjectState::Error(err.clone())),
        }
    }

    /// Returns `true` if the project is in the [`ProjectState::Uninitialized`] state.
    pub fn is_uninitialized(&self) -> bool {
        matches!(self, Self::Uninitialized)
    }

    /// Returns `true` if the project is in the [`ProjectState::Synced`] state.
    pub fn is_synced(&self) -> bool {
        matches!(self, Self::Synced)
    }

    /// Returns `true` if the project is in the [`ProjectState::Active`] state.
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active)
    }

    /// Returns `true` if the project is in the [`ProjectState::Stale`] state.
    pub fn is_stale(&self) -> bool {
        matches!(self, Self::Stale)
    }

    /// Returns `true` if the project is in the [`ProjectState::Error`] state.
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }

    /// Returns the inner error message if this state is [`ProjectState::Error`].
    pub fn error_message(&self) -> Option<&str> {
        match self {
            Self::Error(msg) => Some(msg.as_str()),
            _ => None,
        }
    }
}

/// Represents the persisted local project state stored in `.ctx/state.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalState {
    /// Unique identifier of the project.
    pub project_id: Uuid,
    /// Current state in the project state machine.
    pub state: ProjectState,
    /// Timestamp of the last successful synchronization with the remote.
    pub last_sync: DateTime<Utc>,
    /// Timestamp of the last local save or checkpoint, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_save: Option<DateTime<Utc>>,
    /// Name or identifier of the AI agent currently holding the session lock, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_agent: Option<String>,
}

impl LocalState {
    /// Creates a new `LocalState` initialized to [`ProjectState::Uninitialized`].
    pub fn new(project_id: Uuid) -> Self {
        Self {
            project_id,
            state: ProjectState::Uninitialized,
            last_sync: Utc::now(),
            last_save: None,
            active_agent: None,
        }
    }

    /// Creates a new `LocalState` with explicit values for all fields.
    pub fn with_details(
        project_id: Uuid,
        state: ProjectState,
        last_sync: DateTime<Utc>,
        last_save: Option<DateTime<Utc>>,
        active_agent: Option<String>,
    ) -> Self {
        Self {
            project_id,
            state,
            last_sync,
            last_save,
            active_agent,
        }
    }

    /// Returns the path to the `.ctx` directory inside the given project directory.
    pub fn ctx_dir(dir: &Path) -> PathBuf {
        dir.join(".ctx")
    }

    /// Returns the path to the `state.json` file inside the given project directory.
    pub fn state_path(dir: &Path) -> PathBuf {
        Self::ctx_dir(dir).join("state.json")
    }

    /// Loads the local state from `<dir>/.ctx/state.json`.
    pub fn load(dir: &Path) -> Result<Self> {
        let path = Self::state_path(dir);
        if !path.exists() {
            return Err(StateError::NotFound(path));
        }
        let content = fs::read_to_string(&path).map_err(|source| StateError::Io {
            path: path.clone(),
            source,
        })?;
        let state: Self = serde_json::from_str(&content)
            .map_err(|source| StateError::JsonParse { path, source })?;
        Ok(state)
    }

    /// Saves the local state to `<dir>/.ctx/state.json`.
    ///
    /// Creates the `.ctx` directory if it does not already exist.
    pub fn save(&self, dir: &Path) -> Result<()> {
        let ctx_dir = Self::ctx_dir(dir);
        if !ctx_dir.exists() {
            fs::create_dir_all(&ctx_dir).map_err(|source| StateError::Io {
                path: ctx_dir.clone(),
                source,
            })?;
        }
        let path = Self::state_path(dir);
        let json_str = serde_json::to_string_pretty(self).map_err(StateError::JsonSerialize)?;
        fs::write(&path, json_str).map_err(|source| StateError::Io { path, source })?;
        Ok(())
    }

    /// Applies a state transition to this local state, updating the state and metadata.
    pub fn apply_transition(&mut self, action: StateTransition) -> Result<ProjectState> {
        let new_state = self.state.transition(action.clone())?;
        match &action {
            StateTransition::Pull => {
                self.last_sync = Utc::now();
                self.active_agent = None;
            }
            StateTransition::Claim => {}
            StateTransition::Release => {
                self.active_agent = None;
            }
            StateTransition::Push => {
                self.last_sync = Utc::now();
                self.active_agent = None;
            }
            StateTransition::Save => {
                self.last_save = Some(Utc::now());
            }
            StateTransition::Timeout => {
                self.active_agent = None;
            }
            StateTransition::Fail(_) => {}
        }
        self.state = new_state.clone();
        Ok(new_state)
    }

    /// Claims this project for an active agent, transitioning to [`ProjectState::Active`].
    pub fn claim(&mut self, agent: impl Into<String>) -> Result<()> {
        self.state = self.state.transition(StateTransition::Claim)?;
        self.active_agent = Some(agent.into());
        Ok(())
    }

    /// Releases the session lock, transitioning back to [`ProjectState::Synced`].
    pub fn release(&mut self) -> Result<()> {
        self.state = self.state.transition(StateTransition::Release)?;
        self.active_agent = None;
        Ok(())
    }

    /// Pushes active work, updating `last_sync` and transitioning to [`ProjectState::Synced`].
    pub fn push(&mut self) -> Result<()> {
        self.state = self.state.transition(StateTransition::Push)?;
        self.last_sync = Utc::now();
        self.active_agent = None;
        Ok(())
    }

    /// Pulls remote state, updating `last_sync` and transitioning to [`ProjectState::Synced`].
    pub fn pull(&mut self) -> Result<()> {
        self.state = self.state.transition(StateTransition::Pull)?;
        self.last_sync = Utc::now();
        self.active_agent = None;
        Ok(())
    }

    /// Records a local save checkpoint, updating `last_save`.
    pub fn save_checkpoint(&mut self) -> Result<()> {
        self.state = self.state.transition(StateTransition::Save)?;
        self.last_save = Some(Utc::now());
        Ok(())
    }

    /// Marks the project as timed out, transitioning to [`ProjectState::Stale`].
    pub fn timeout(&mut self) -> Result<()> {
        self.state = self.state.transition(StateTransition::Timeout)?;
        self.active_agent = None;
        Ok(())
    }

    /// Transitions the project to [`ProjectState::Error`].
    pub fn fail(&mut self, error: impl Into<String>) -> Result<()> {
        let err_str = error.into();
        self.state = self.state.transition(StateTransition::Fail(err_str))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDirGuard(PathBuf);

    impl TempDirGuard {
        fn new() -> Self {
            let unique = format!("ctx_state_test_{}_{}", Uuid::new_v4(), std::process::id());
            let path = std::env::temp_dir().join(unique);
            fs::create_dir_all(&path).expect("Failed to create temporary test directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn test_valid_transitions_uninitialized() {
        let state = ProjectState::Uninitialized;
        assert_eq!(
            state.transition(StateTransition::Pull).expect("Pull should succeed"),
            ProjectState::Synced
        );

        let fail_action = StateTransition::Fail("disk error".to_string());
        assert_eq!(
            state.transition(fail_action).expect("Fail should succeed"),
            ProjectState::Error("disk error".to_string())
        );
    }

    #[test]
    fn test_invalid_transitions_uninitialized() {
        let state = ProjectState::Uninitialized;

        let invalid_actions = [
            StateTransition::Claim,
            StateTransition::Release,
            StateTransition::Push,
            StateTransition::Save,
            StateTransition::Timeout,
        ];

        for action in invalid_actions {
            let err = state
                .transition(action.clone())
                .expect_err(&format!("Action {action:?} should fail on Uninitialized"));
            let invalid = err
                .as_invalid_transition()
                .expect("Should be InvalidTransition");
            assert_eq!(invalid.from, ProjectState::Uninitialized);
            assert_eq!(invalid.action, action);
        }
    }

    #[test]
    fn test_valid_transitions_synced() {
        let state = ProjectState::Synced;

        assert_eq!(
            state.transition(StateTransition::Pull).expect("Pull should succeed"),
            ProjectState::Synced
        );

        assert_eq!(
            state.transition(StateTransition::Claim).expect("Claim should succeed"),
            ProjectState::Active
        );

        assert_eq!(
            state.transition(StateTransition::Timeout).expect("Timeout should succeed"),
            ProjectState::Stale
        );

        assert_eq!(
            state
                .transition(StateTransition::Fail("auth failed".to_string()))
                .expect("Fail should succeed"),
            ProjectState::Error("auth failed".to_string())
        );
    }

    #[test]
    fn test_invalid_transitions_synced() {
        let state = ProjectState::Synced;

        let invalid_actions = [
            StateTransition::Release,
            StateTransition::Push,
            StateTransition::Save,
        ];

        for action in invalid_actions {
            let err = state
                .transition(action.clone())
                .expect_err(&format!("Action {action:?} should fail on Synced"));
            let invalid = err
                .as_invalid_transition()
                .expect("Should be InvalidTransition");
            assert_eq!(invalid.from, ProjectState::Synced);
            assert_eq!(invalid.action, action);
        }
    }

    #[test]
    fn test_valid_transitions_active() {
        let state = ProjectState::Active;

        assert_eq!(
            state.transition(StateTransition::Save).expect("Save should succeed"),
            ProjectState::Active
        );

        assert_eq!(
            state.transition(StateTransition::Push).expect("Push should succeed"),
            ProjectState::Synced
        );

        assert_eq!(
            state.transition(StateTransition::Release).expect("Release should succeed"),
            ProjectState::Synced
        );

        assert_eq!(
            state.transition(StateTransition::Timeout).expect("Timeout should succeed"),
            ProjectState::Stale
        );

        assert_eq!(
            state
                .transition(StateTransition::Fail("crashed".to_string()))
                .expect("Fail should succeed"),
            ProjectState::Error("crashed".to_string())
        );
    }

    #[test]
    fn test_invalid_transitions_active() {
        let state = ProjectState::Active;

        let invalid_actions = [StateTransition::Pull, StateTransition::Claim];

        for action in invalid_actions {
            let err = state
                .transition(action.clone())
                .expect_err(&format!("Action {action:?} should fail on Active"));
            let invalid = err
                .as_invalid_transition()
                .expect("Should be InvalidTransition");
            assert_eq!(invalid.from, ProjectState::Active);
            assert_eq!(invalid.action, action);
        }
    }

    #[test]
    fn test_valid_transitions_stale() {
        let state = ProjectState::Stale;

        assert_eq!(
            state.transition(StateTransition::Pull).expect("Pull should succeed"),
            ProjectState::Synced
        );

        assert_eq!(
            state
                .transition(StateTransition::Fail("net unreachable".to_string()))
                .expect("Fail should succeed"),
            ProjectState::Error("net unreachable".to_string())
        );
    }

    #[test]
    fn test_invalid_transitions_stale() {
        let state = ProjectState::Stale;

        let invalid_actions = [
            StateTransition::Claim,
            StateTransition::Release,
            StateTransition::Push,
            StateTransition::Save,
            StateTransition::Timeout,
        ];

        for action in invalid_actions {
            let err = state
                .transition(action.clone())
                .expect_err(&format!("Action {action:?} should fail on Stale"));
            let invalid = err
                .as_invalid_transition()
                .expect("Should be InvalidTransition");
            assert_eq!(invalid.from, ProjectState::Stale);
            assert_eq!(invalid.action, action);
        }
    }

    #[test]
    fn test_valid_transitions_error() {
        let state = ProjectState::Error("previous issue".to_string());

        assert_eq!(
            state.transition(StateTransition::Pull).expect("Pull should succeed"),
            ProjectState::Synced
        );

        assert_eq!(
            state
                .transition(StateTransition::Fail("new issue".to_string()))
                .expect("Fail should succeed"),
            ProjectState::Error("new issue".to_string())
        );
    }

    #[test]
    fn test_invalid_transitions_error() {
        let state = ProjectState::Error("some error".to_string());

        let invalid_actions = [
            StateTransition::Claim,
            StateTransition::Release,
            StateTransition::Push,
            StateTransition::Save,
            StateTransition::Timeout,
        ];

        for action in invalid_actions {
            let err = state
                .transition(action.clone())
                .expect_err(&format!("Action {action:?} should fail on Error"));
            let invalid = err
                .as_invalid_transition()
                .expect("Should be InvalidTransition");
            assert_eq!(invalid.from, state);
            assert_eq!(invalid.action, action);
        }
    }

    #[test]
    fn test_project_state_predicates_and_display() {
        let uninit = ProjectState::Uninitialized;
        assert!(uninit.is_uninitialized());
        assert!(!uninit.is_synced());
        assert_eq!(uninit.to_string(), "uninitialized");

        let synced = ProjectState::Synced;
        assert!(synced.is_synced());
        assert!(!synced.is_active());
        assert_eq!(synced.to_string(), "synced");

        let active = ProjectState::Active;
        assert!(active.is_active());
        assert!(!active.is_stale());
        assert_eq!(active.to_string(), "active");

        let stale = ProjectState::Stale;
        assert!(stale.is_stale());
        assert!(!stale.is_error());
        assert_eq!(stale.to_string(), "stale");

        let err = ProjectState::Error("lock timeout".to_string());
        assert!(err.is_error());
        assert_eq!(err.error_message(), Some("lock timeout"));
        assert_eq!(err.to_string(), "error: lock timeout");
    }

    #[test]
    fn test_state_transition_display() {
        assert_eq!(StateTransition::Pull.to_string(), "pull");
        assert_eq!(StateTransition::Claim.to_string(), "claim");
        assert_eq!(StateTransition::Release.to_string(), "release");
        assert_eq!(StateTransition::Push.to_string(), "push");
        assert_eq!(StateTransition::Save.to_string(), "save");
        assert_eq!(StateTransition::Timeout.to_string(), "timeout");
        assert_eq!(
            StateTransition::Fail("connection reset".to_string()).to_string(),
            "fail: connection reset"
        );
    }

    #[test]
    fn test_local_state_new_and_with_details() {
        let project_id = Uuid::new_v4();
        let state = LocalState::new(project_id);

        assert_eq!(state.project_id, project_id);
        assert_eq!(state.state, ProjectState::Uninitialized);
        assert!(state.last_save.is_none());
        assert!(state.active_agent.is_none());

        let now = Utc::now();
        let custom = LocalState::with_details(
            project_id,
            ProjectState::Active,
            now,
            Some(now),
            Some("claude-code".to_string()),
        );
        assert_eq!(custom.project_id, project_id);
        assert_eq!(custom.state, ProjectState::Active);
        assert_eq!(custom.last_sync, now);
        assert_eq!(custom.last_save, Some(now));
        assert_eq!(custom.active_agent.as_deref(), Some("claude-code"));
    }

    #[test]
    fn test_local_state_save_and_load_roundtrip() {
        let guard = TempDirGuard::new();
        let project_id = Uuid::new_v4();
        let now = Utc::now();

        let initial_state = LocalState::with_details(
            project_id,
            ProjectState::Active,
            now,
            Some(now),
            Some("agent-42".to_string()),
        );

        initial_state
            .save(guard.path())
            .expect("Failed to save LocalState");

        let loaded_state =
            LocalState::load(guard.path()).expect("Failed to load LocalState from disk");

        assert_eq!(initial_state, loaded_state);
        assert_eq!(loaded_state.project_id, project_id);
        assert_eq!(loaded_state.state, ProjectState::Active);
        assert_eq!(loaded_state.active_agent.as_deref(), Some("agent-42"));
    }

    #[test]
    fn test_local_state_load_not_found() {
        let guard = TempDirGuard::new();
        let res = LocalState::load(guard.path());
        assert!(matches!(res, Err(StateError::NotFound(_))));
    }

    #[test]
    fn test_local_state_load_invalid_json() {
        let guard = TempDirGuard::new();
        let ctx_dir = LocalState::ctx_dir(guard.path());
        fs::create_dir_all(&ctx_dir).expect("Failed to create .ctx directory");
        fs::write(LocalState::state_path(guard.path()), "{ corrupt json }")
            .expect("Failed to write corrupt JSON");

        let res = LocalState::load(guard.path());
        assert!(matches!(res, Err(StateError::JsonParse { .. })));
    }

    #[test]
    fn test_local_state_methods_lifecycle() {
        let project_id = Uuid::new_v4();
        let mut local = LocalState::new(project_id);

        assert_eq!(local.state, ProjectState::Uninitialized);

        // Pull to sync
        local.pull().expect("Pull failed");
        assert_eq!(local.state, ProjectState::Synced);
        assert!(local.active_agent.is_none());

        // Claim by agent
        local.claim("cursor-agent").expect("Claim failed");
        assert_eq!(local.state, ProjectState::Active);
        assert_eq!(local.active_agent.as_deref(), Some("cursor-agent"));

        // Checkpoint save
        local.save_checkpoint().expect("Save checkpoint failed");
        assert_eq!(local.state, ProjectState::Active);
        assert!(local.last_save.is_some());
        assert_eq!(local.active_agent.as_deref(), Some("cursor-agent"));

        // Push work
        local.push().expect("Push failed");
        assert_eq!(local.state, ProjectState::Synced);
        assert!(local.active_agent.is_none());

        // Re-claim and release
        local.claim("codex-agent").expect("Claim failed");
        assert_eq!(local.state, ProjectState::Active);
        assert_eq!(local.active_agent.as_deref(), Some("codex-agent"));

        local.release().expect("Release failed");
        assert_eq!(local.state, ProjectState::Synced);
        assert!(local.active_agent.is_none());

        // Timeout
        local.timeout().expect("Timeout failed");
        assert_eq!(local.state, ProjectState::Stale);

        // Fail
        local.fail("network split").expect("Fail failed");
        assert_eq!(
            local.state,
            ProjectState::Error("network split".to_string())
        );

        // Recover with pull
        local.pull().expect("Pull recovery failed");
        assert_eq!(local.state, ProjectState::Synced);
    }

    #[test]
    fn test_serde_roundtrip() {
        let states = vec![
            ProjectState::Uninitialized,
            ProjectState::Synced,
            ProjectState::Active,
            ProjectState::Stale,
            ProjectState::Error("failed lock".to_string()),
        ];

        for state in states {
            let json = serde_json::to_string(&state).expect("Serialization failed");
            let deserialized: ProjectState =
                serde_json::from_str(&json).expect("Deserialization failed");
            assert_eq!(state, deserialized);
        }

        let transitions = vec![
            StateTransition::Pull,
            StateTransition::Claim,
            StateTransition::Release,
            StateTransition::Push,
            StateTransition::Save,
            StateTransition::Timeout,
            StateTransition::Fail("fatal error".to_string()),
        ];

        for action in transitions {
            let json = serde_json::to_string(&action).expect("Serialization failed");
            let deserialized: StateTransition =
                serde_json::from_str(&json).expect("Deserialization failed");
            assert_eq!(action, deserialized);
        }
    }

    #[test]
    fn test_ctx_error_conversions() {
        let invalid = InvalidTransition::new(
            ProjectState::Synced,
            StateTransition::Push,
            "cannot push from synced",
        );
        let ctx_err: CtxError = invalid.clone().into();
        assert!(matches!(ctx_err, CtxError::InvalidState(_)));
        assert!(ctx_err.to_string().contains("cannot push from synced"));

        let state_err = StateError::InvalidTransition(invalid);
        let ctx_err2: CtxError = state_err.into();
        assert!(matches!(ctx_err2, CtxError::InvalidState(_)));
    }
}
