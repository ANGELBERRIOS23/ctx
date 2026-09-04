//! Synchronization protocol types for the `ctx` platform.
//!
//! This module defines the core protocol data structures exchanged between
//! `ctx-cli`, `ctx-server`, and peer nodes during synchronization operations.
//! These structures include snapshots ([`SyncSnapshot`]), session locks
//! ([`SessionLock`]), registered machine metadata ([`MachineInfo`]), and
//! project tracking information ([`ProjectInfo`]).

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::error::CtxError;

/// Error returned when parsing an invalid [`SnapshotType`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("Invalid snapshot type '{0}': expected 'auto', 'manual', or 'agent_end'")]
pub struct ParseSnapshotTypeError(pub String);

impl From<ParseSnapshotTypeError> for CtxError {
    fn from(err: ParseSnapshotTypeError) -> Self {
        CtxError::Sync(err.to_string())
    }
}

/// The trigger or lifecycle event that caused a snapshot to be captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotType {
    /// Periodic or automatic background snapshot.
    #[serde(alias = "Auto", alias = "AUTO")]
    Auto,
    /// Explicitly triggered manual snapshot by a user command.
    #[serde(alias = "Manual", alias = "MANUAL")]
    Manual,
    /// Snapshot generated upon the conclusion of an AI agent session.
    #[serde(alias = "AgentEnd", alias = "agent_end", alias = "AGENT_END")]
    AgentEnd,
}

impl SnapshotType {
    /// Returns the static string representation of this snapshot type.
    ///
    /// - `Auto` -> `"auto"`
    /// - `Manual` -> `"manual"`
    /// - `AgentEnd` -> `"agent_end"`
    pub fn as_str(&self) -> &'static str {
        match self {
            SnapshotType::Auto => "auto",
            SnapshotType::Manual => "manual",
            SnapshotType::AgentEnd => "agent_end",
        }
    }
}

impl fmt::Display for SnapshotType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for SnapshotType {
    type Err = ParseSnapshotTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(SnapshotType::Auto),
            "manual" => Ok(SnapshotType::Manual),
            "agent_end" | "agentend" => Ok(SnapshotType::AgentEnd),
            _ => Err(ParseSnapshotTypeError(s.to_string())),
        }
    }
}

/// An encrypted snapshot of project state, handoff, and optional memory for sync transit.
///
/// Encapsulates the encrypted handoff payload, optional encrypted agent memory,
/// state metadata as structured JSON, and Git commit reference for a given project
/// captured on a specific machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncSnapshot {
    /// Unique identifier of this snapshot.
    pub id: Uuid,
    /// Unique identifier of the project this snapshot belongs to.
    pub project_id: Uuid,
    /// Unique identifier of the machine that captured this snapshot.
    pub machine_id: Uuid,
    /// The trigger type that produced this snapshot.
    pub snapshot_type: SnapshotType,
    /// Git commit SHA at the point the snapshot was captured.
    pub git_commit: String,
    /// Encrypted binary payload of the project handoff.
    pub handoff_blob: Vec<u8>,
    /// Optional encrypted binary payload of agent memory.
    pub memory_blob: Option<Vec<u8>>,
    /// Additional project or machine state encoded as arbitrary JSON metadata.
    pub state_json: serde_json::Value,
    /// UTC timestamp when this snapshot was created.
    pub created_at: DateTime<Utc>,
}

impl SyncSnapshot {
    /// Creates a new [`SyncSnapshot`] with a randomly generated UUID and current timestamp.
    ///
    /// Memory blob defaults to `None`, and `state_json` defaults to an empty JSON object (`{}`).
    pub fn new(
        project_id: Uuid,
        machine_id: Uuid,
        snapshot_type: SnapshotType,
        git_commit: impl Into<String>,
        handoff_blob: Vec<u8>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            project_id,
            machine_id,
            snapshot_type,
            git_commit: git_commit.into(),
            handoff_blob,
            memory_blob: None,
            state_json: serde_json::json!({}),
            created_at: Utc::now(),
        }
    }

    /// Sets the optional encrypted memory payload for this snapshot.
    pub fn with_memory_blob(mut self, memory_blob: Vec<u8>) -> Self {
        self.memory_blob = Some(memory_blob);
        self
    }

    /// Sets or clears the optional encrypted memory payload for this snapshot.
    pub fn with_optional_memory_blob(mut self, memory_blob: Option<Vec<u8>>) -> Self {
        self.memory_blob = memory_blob;
        self
    }

    /// Sets the state JSON value for this snapshot.
    pub fn with_state_json(mut self, state_json: serde_json::Value) -> Self {
        self.state_json = state_json;
        self
    }

    /// Returns `true` if this snapshot includes an encrypted memory blob.
    pub fn has_memory(&self) -> bool {
        self.memory_blob.is_some()
    }

    /// Calculates the total size in bytes of the binary blobs contained in this snapshot.
    pub fn payload_size_bytes(&self) -> usize {
        self.handoff_blob.len() + self.memory_blob.as_ref().map_or(0, |b| b.len())
    }
}

/// Distributed session lock representing an exclusive write claim by a machine on a project.
///
/// Locks prevent concurrent write conflicts across machines. They are acquired with
/// a timestamp and kept alive through periodic heartbeats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionLock {
    /// Unique identifier of the project being locked.
    pub project_id: Uuid,
    /// Unique identifier of the machine currently holding the lock.
    pub machine_id: Uuid,
    /// UTC timestamp when the lock was originally acquired.
    pub locked_at: DateTime<Utc>,
    /// UTC timestamp of the most recent heartbeat from the lock holder.
    pub heartbeat: DateTime<Utc>,
}

impl SessionLock {
    /// Creates a new [`SessionLock`] with the current UTC time for both `locked_at` and `heartbeat`.
    pub fn new(project_id: Uuid, machine_id: Uuid) -> Self {
        let now = Utc::now();
        Self {
            project_id,
            machine_id,
            locked_at: now,
            heartbeat: now,
        }
    }

    /// Creates a [`SessionLock`] with explicit timestamps.
    pub fn with_timestamps(
        project_id: Uuid,
        machine_id: Uuid,
        locked_at: DateTime<Utc>,
        heartbeat: DateTime<Utc>,
    ) -> Self {
        Self {
            project_id,
            machine_id,
            locked_at,
            heartbeat,
        }
    }

    /// Updates the heartbeat timestamp of this session lock to `now`.
    pub fn refresh_heartbeat(&mut self, now: DateTime<Utc>) {
        self.heartbeat = now;
    }

    /// Checks whether this session lock has expired given a time-to-live (`ttl`) and reference time (`now`).
    pub fn is_expired(&self, ttl: Duration, now: DateTime<Utc>) -> bool {
        now.signed_duration_since(self.heartbeat) > ttl
    }

    /// Returns `true` if the lock is held by the specified machine.
    pub fn is_held_by(&self, machine: &Uuid) -> bool {
        self.machine_id == *machine
    }
}

/// Information about a registered machine participating in the synchronization network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineInfo {
    /// Unique identifier of the machine.
    pub id: Uuid,
    /// Human-readable name or hostname of the machine.
    pub name: String,
    /// Operating system identifier (e.g., "macos", "linux", "windows").
    pub os: String,
    /// Hardware or host fingerprint used for identity verification.
    pub fingerprint: String,
    /// UTC timestamp when the machine was last active or seen by the sync network.
    pub last_seen: DateTime<Utc>,
}

impl MachineInfo {
    /// Creates a new [`MachineInfo`] instance with `last_seen` set to the current UTC time.
    pub fn new(
        id: Uuid,
        name: impl Into<String>,
        os: impl Into<String>,
        fingerprint: impl Into<String>,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            os: os.into(),
            fingerprint: fingerprint.into(),
            last_seen: Utc::now(),
        }
    }

    /// Constructs a [`MachineInfo`] using the current platform's OS name and current UTC time.
    pub fn current(
        id: Uuid,
        name: impl Into<String>,
        fingerprint: impl Into<String>,
    ) -> Self {
        Self::new(id, name, std::env::consts::OS, fingerprint)
    }

    /// Updates the `last_seen` timestamp to the specified time.
    pub fn update_last_seen(&mut self, now: DateTime<Utc>) {
        self.last_seen = now;
    }

    /// Checks whether the machine has been seen recently within the specified timeout duration.
    pub fn is_active(&self, timeout: Duration, now: DateTime<Utc>) -> bool {
        now.signed_duration_since(self.last_seen) <= timeout
    }
}

/// Metadata and active session tracking information for a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectInfo {
    /// Unique identifier of the project.
    pub id: Uuid,
    /// Display name of the project.
    pub name: String,
    /// Remote Git repository URL or remote specifier.
    pub git_remote: String,
    /// Active or default Git branch name.
    pub git_branch: String,
    /// Latest known Git commit SHA.
    pub git_commit: String,
    /// Unique identifier of the machine currently holding an active session lock, if any.
    pub active_machine: Option<Uuid>,
    /// Identifier of the AI agent currently active on the project, if any.
    pub active_agent: Option<String>,
    /// UTC timestamp when the project session was claimed, if currently claimed.
    pub claimed_at: Option<DateTime<Utc>>,
}

impl ProjectInfo {
    /// Creates a new [`ProjectInfo`] with no active machine or agent claim.
    pub fn new(
        id: Uuid,
        name: impl Into<String>,
        git_remote: impl Into<String>,
        git_branch: impl Into<String>,
        git_commit: impl Into<String>,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            git_remote: git_remote.into(),
            git_branch: git_branch.into(),
            git_commit: git_commit.into(),
            active_machine: None,
            active_agent: None,
            claimed_at: None,
        }
    }

    /// Returns `true` if the project is currently claimed by an active machine session.
    pub fn is_claimed(&self) -> bool {
        self.active_machine.is_some()
    }

    /// Claims the project session for a given machine and optional agent.
    pub fn claim(
        &mut self,
        machine_id: Uuid,
        agent_name: Option<String>,
        now: DateTime<Utc>,
    ) {
        self.active_machine = Some(machine_id);
        self.active_agent = agent_name;
        self.claimed_at = Some(now);
    }

    /// Releases any active machine and agent claim on this project.
    pub fn release(&mut self) {
        self.active_machine = None;
        self.active_agent = None;
        self.claimed_at = None;
    }

    /// Updates the Git branch and commit reference for the project.
    pub fn update_git_revision(
        &mut self,
        branch: impl Into<String>,
        commit: impl Into<String>,
    ) {
        self.git_branch = branch.into();
        self.git_commit = commit.into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshot_type_string_and_display() {
        assert_eq!(SnapshotType::Auto.as_str(), "auto");
        assert_eq!(SnapshotType::Manual.as_str(), "manual");
        assert_eq!(SnapshotType::AgentEnd.as_str(), "agent_end");

        assert_eq!(format!("{}", SnapshotType::Auto), "auto");
        assert_eq!(format!("{}", SnapshotType::Manual), "manual");
        assert_eq!(format!("{}", SnapshotType::AgentEnd), "agent_end");
    }

    #[test]
    fn test_snapshot_type_from_str() {
        assert_eq!("auto".parse::<SnapshotType>().unwrap(), SnapshotType::Auto);
        assert_eq!("Auto".parse::<SnapshotType>().unwrap(), SnapshotType::Auto);
        assert_eq!("AUTO".parse::<SnapshotType>().unwrap(), SnapshotType::Auto);
        assert_eq!("manual".parse::<SnapshotType>().unwrap(), SnapshotType::Manual);
        assert_eq!("agent_end".parse::<SnapshotType>().unwrap(), SnapshotType::AgentEnd);
        assert_eq!("agentend".parse::<SnapshotType>().unwrap(), SnapshotType::AgentEnd);

        let err = "unknown".parse::<SnapshotType>().unwrap_err();
        assert_eq!(err, ParseSnapshotTypeError("unknown".to_string()));

        // Test From conversion to CtxError
        let ctx_err: CtxError = err.into();
        assert!(matches!(ctx_err, CtxError::Sync(_)));
    }

    #[test]
    fn test_snapshot_type_serde() {
        let json_auto = serde_json::to_string(&SnapshotType::Auto).unwrap();
        assert_eq!(json_auto, "\"auto\"");
        let parsed: SnapshotType = serde_json::from_str("\"auto\"").unwrap();
        assert_eq!(parsed, SnapshotType::Auto);

        // Aliases
        let parsed_alias: SnapshotType = serde_json::from_str("\"Auto\"").unwrap();
        assert_eq!(parsed_alias, SnapshotType::Auto);
        let parsed_agent_end: SnapshotType = serde_json::from_str("\"AgentEnd\"").unwrap();
        assert_eq!(parsed_agent_end, SnapshotType::AgentEnd);
    }

    #[test]
    fn test_sync_snapshot_creation_and_methods() {
        let project_id = Uuid::new_v4();
        let machine_id = Uuid::new_v4();
        let handoff = vec![1, 2, 3, 4, 5];
        let memory = vec![10, 20, 30];

        let snapshot = SyncSnapshot::new(
            project_id,
            machine_id,
            SnapshotType::Manual,
            "abc1234",
            handoff.clone(),
        )
        .with_memory_blob(memory.clone())
        .with_state_json(serde_json::json!({"synced": true}));

        assert_eq!(snapshot.project_id, project_id);
        assert_eq!(snapshot.machine_id, machine_id);
        assert_eq!(snapshot.snapshot_type, SnapshotType::Manual);
        assert_eq!(snapshot.git_commit, "abc1234");
        assert_eq!(snapshot.handoff_blob, handoff);
        assert_eq!(snapshot.memory_blob, Some(memory));
        assert_eq!(snapshot.state_json["synced"], true);
        assert!(snapshot.has_memory());
        assert_eq!(snapshot.payload_size_bytes(), 8);

        // Serde roundtrip
        let serialized = serde_json::to_string(&snapshot).unwrap();
        let deserialized: SyncSnapshot = serde_json::from_str(&serialized).unwrap();
        assert_eq!(snapshot, deserialized);
    }

    #[test]
    fn test_sync_snapshot_without_memory() {
        let project_id = Uuid::new_v4();
        let machine_id = Uuid::new_v4();
        let handoff = vec![42; 16];

        let snapshot = SyncSnapshot::new(
            project_id,
            machine_id,
            SnapshotType::Auto,
            "commit123",
            handoff,
        );

        assert!(!snapshot.has_memory());
        assert_eq!(snapshot.payload_size_bytes(), 16);
        assert_eq!(snapshot.state_json, serde_json::json!({}));
    }

    #[test]
    fn test_session_lock_lifecycle_and_expiry() {
        let project_id = Uuid::new_v4();
        let machine_id = Uuid::new_v4();
        let other_machine = Uuid::new_v4();

        let mut lock = SessionLock::new(project_id, machine_id);
        assert!(lock.is_held_by(&machine_id));
        assert!(!lock.is_held_by(&other_machine));

        let now = lock.heartbeat;
        let ttl = Duration::seconds(30);

        // Immediately, not expired
        assert!(!lock.is_expired(ttl, now));

        // 20 seconds later, not expired
        assert!(!lock.is_expired(ttl, now + Duration::seconds(20)));

        // 31 seconds later, expired
        assert!(lock.is_expired(ttl, now + Duration::seconds(31)));

        // Refresh heartbeat
        let refreshed_time = now + Duration::seconds(25);
        lock.refresh_heartbeat(refreshed_time);
        assert_eq!(lock.heartbeat, refreshed_time);
        assert!(!lock.is_expired(ttl, now + Duration::seconds(31)));
        assert!(lock.is_expired(ttl, refreshed_time + Duration::seconds(35)));

        // Serde roundtrip
        let serialized = serde_json::to_string(&lock).unwrap();
        let deserialized: SessionLock = serde_json::from_str(&serialized).unwrap();
        assert_eq!(lock, deserialized);
    }

    #[test]
    fn test_machine_info_creation_and_activity() {
        let id = Uuid::new_v4();
        let mut machine = MachineInfo::new(id, "macbook-pro", "macos", "fp-998877");

        assert_eq!(machine.id, id);
        assert_eq!(machine.name, "macbook-pro");
        assert_eq!(machine.os, "macos");
        assert_eq!(machine.fingerprint, "fp-998877");

        let now = machine.last_seen;
        let timeout = Duration::seconds(60);

        assert!(machine.is_active(timeout, now));
        assert!(machine.is_active(timeout, now + Duration::seconds(50)));
        assert!(!machine.is_active(timeout, now + Duration::seconds(61)));

        // Update last seen
        let updated_time = now + Duration::seconds(50);
        machine.update_last_seen(updated_time);
        assert_eq!(machine.last_seen, updated_time);
        assert!(machine.is_active(timeout, now + Duration::seconds(61)));

        // Test current() constructor
        let current_machine = MachineInfo::current(Uuid::new_v4(), "current-host", "fp-123");
        assert_eq!(current_machine.os, std::env::consts::OS);

        // Serde roundtrip
        let serialized = serde_json::to_string(&machine).unwrap();
        let deserialized: MachineInfo = serde_json::from_str(&serialized).unwrap();
        assert_eq!(machine, deserialized);
    }

    #[test]
    fn test_project_info_claim_and_release() {
        let id = Uuid::new_v4();
        let mut project = ProjectInfo::new(
            id,
            "ctx",
            "git@github.com:user/ctx.git",
            "main",
            "sha111",
        );

        assert_eq!(project.id, id);
        assert_eq!(project.name, "ctx");
        assert_eq!(project.git_remote, "git@github.com:user/ctx.git");
        assert_eq!(project.git_branch, "main");
        assert_eq!(project.git_commit, "sha111");
        assert!(!project.is_claimed());
        assert_eq!(project.active_machine, None);
        assert_eq!(project.active_agent, None);
        assert_eq!(project.claimed_at, None);

        // Claim project
        let machine_id = Uuid::new_v4();
        let claim_time = Utc::now();
        project.claim(machine_id, Some("claude-code".to_string()), claim_time);

        assert!(project.is_claimed());
        assert_eq!(project.active_machine, Some(machine_id));
        assert_eq!(project.active_agent, Some("claude-code".to_string()));
        assert_eq!(project.claimed_at, Some(claim_time));

        // Update git revision
        project.update_git_revision("feature/sync", "sha222");
        assert_eq!(project.git_branch, "feature/sync");
        assert_eq!(project.git_commit, "sha222");

        // Release project
        project.release();
        assert!(!project.is_claimed());
        assert_eq!(project.active_machine, None);
        assert_eq!(project.active_agent, None);
        assert_eq!(project.claimed_at, None);

        // Serde roundtrip
        let serialized = serde_json::to_string(&project).unwrap();
        let deserialized: ProjectInfo = serde_json::from_str(&serialized).unwrap();
        assert_eq!(project, deserialized);
    }
}
