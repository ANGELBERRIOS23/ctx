//! Storage abstraction layer for `ctx-server`.
//!
//! This module defines the core async traits and error types for server persistence:
//! - [`StorageBackend`]: Handles binary blob and snapshot storage (S3/MinIO for cloud, filesystem for P2P/local).
//! - [`MetadataStore`]: Handles relational metadata persistence for users, projects, machines, and session locks
//!   (PostgreSQL for cloud, SQLite for P2P/embedded).
//!
//! Submodules provide concrete implementations for each supported backend.

pub mod local;
pub mod postgres;
pub mod s3;
pub mod sqlite;

// Submodules will export concrete stores when implemented:
// pub use local::LocalBlobStore;
// pub use postgres::PgMetadataStore;
// pub use s3::S3BlobStore;
// pub use sqlite::SqliteMetadataStore;

use chrono::{DateTime, Utc};
use ctx_core::protocol::{MachineInfo, ProjectInfo, SessionLock, SyncSnapshot};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Errors that can occur during storage and database operations in `ctx-server`.
#[derive(Debug, Error)]
pub enum StorageError {
    /// Relational database query or connection failure.
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    /// Local filesystem I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Object storage / S3 service error.
    #[error("S3 error: {0}")]
    S3(String),

    /// Serialization or deserialization error.
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Requested resource or entity was not found.
    #[error("Not found: {0}")]
    NotFound(String),

    /// Conflict when creating or updating a record (e.g., duplicate unique key).
    #[error("Conflict: {0}")]
    Conflict(String),

    /// Session lock acquisition conflict between machines.
    #[error("Lock conflict for project {project_id}: currently held by machine {held_by}")]
    LockConflict {
        /// Project UUID that is locked.
        project_id: Uuid,
        /// Machine UUID currently holding the lock.
        held_by: Uuid,
    },

    /// Internal error or invariant violation.
    #[error("Internal storage error: {0}")]
    Internal(String),
}

/// A specialized [`Result`] type for storage operations.
pub type Result<T, E = StorageError> = std::result::Result<T, E>;

/// User entity representing a registered developer account in `ctx-server`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    /// Unique identifier of the user account.
    pub id: Uuid,
    /// Unique email address used for login.
    pub email: String,
    /// Securely hashed password (Argon2id).
    pub password_hash: String,
    /// UTC timestamp when the user registered.
    pub created_at: DateTime<Utc>,
}

impl User {
    /// Creates a new [`User`] with a random UUID and the current UTC timestamp.
    pub fn new(email: impl Into<String>, password_hash: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            email: email.into(),
            password_hash: password_hash.into(),
            created_at: Utc::now(),
        }
    }

    /// Creates a [`User`] with explicit field values.
    pub fn with_details(
        id: Uuid,
        email: impl Into<String>,
        password_hash: impl Into<String>,
        created_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            email: email.into(),
            password_hash: password_hash.into(),
            created_at,
        }
    }
}

/// Asynchronous storage backend trait for binary payloads and project snapshots.
///
/// Implemented by [`S3BlobStore`] for cloud/VPS deployments and [`LocalBlobStore`]
/// for embedded or peer-to-peer deployments. Provides default implementations for
/// snapshot management using the underlying blob storage methods.
#[allow(async_fn_in_trait)]
pub trait StorageBackend: Send + Sync {
    /// Persists a project synchronization snapshot.
    ///
    /// The default implementation serializes the snapshot to JSON, stores it under
    /// `snapshots/{project_id}/{snapshot_id}.json`, updates the `latest.json` pointer,
    /// and appends the snapshot ID to the project's snapshot index.
    async fn save_snapshot(&self, snapshot: &SyncSnapshot) -> Result<()> {
        let key = format!("snapshots/{}/{}.json", snapshot.project_id, snapshot.id);
        let bytes = serde_json::to_vec(snapshot)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        self.save_blob(&key, &bytes).await?;

        // Update latest pointer
        let latest_key = format!("snapshots/{}/latest.json", snapshot.project_id);
        self.save_blob(&latest_key, &bytes).await?;

        // Update project snapshot index
        let index_key = format!("snapshots/{}/index.json", snapshot.project_id);
        let mut index: Vec<Uuid> = match self.get_blob(&index_key).await? {
            Some(data) => serde_json::from_slice(&data).unwrap_or_default(),
            None => Vec::new(),
        };
        index.retain(|id| id != &snapshot.id);
        index.insert(0, snapshot.id);
        let index_bytes = serde_json::to_vec(&index)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        self.save_blob(&index_key, &index_bytes).await?;

        Ok(())
    }

    /// Retrieves the most recent snapshot for the specified project, if one exists.
    async fn get_latest_snapshot(&self, project_id: Uuid) -> Result<Option<SyncSnapshot>> {
        let latest_key = format!("snapshots/{}/latest.json", project_id);
        match self.get_blob(&latest_key).await? {
            Some(data) => {
                let snapshot = serde_json::from_slice(&data)
                    .map_err(|e| StorageError::Serialization(e.to_string()))?;
                Ok(Some(snapshot))
            }
            None => Ok(None),
        }
    }

    /// Lists up to `limit` snapshots for the specified project, ordered newest first.
    async fn list_snapshots(&self, project_id: Uuid, limit: usize) -> Result<Vec<SyncSnapshot>> {
        let index_key = format!("snapshots/{}/index.json", project_id);
        let index: Vec<Uuid> = match self.get_blob(&index_key).await? {
            Some(data) => serde_json::from_slice(&data).unwrap_or_default(),
            None => Vec::new(),
        };

        let mut snapshots = Vec::new();
        for id in index.into_iter().take(limit) {
            let key = format!("snapshots/{}/{}.json", project_id, id);
            if let Some(data) = self.get_blob(&key).await?
                && let Ok(snapshot) = serde_json::from_slice::<SyncSnapshot>(&data) {
                    snapshots.push(snapshot);
                }
        }
        Ok(snapshots)
    }

    /// Stores binary data blob under the given key.
    async fn save_blob(&self, key: &str, data: &[u8]) -> Result<()>;

    /// Retrieves a binary data blob by key. Returns `None` if the blob does not exist.
    async fn get_blob(&self, key: &str) -> Result<Option<Vec<u8>>>;

    /// Deletes a binary data blob by key. Returns `Ok(())` even if the blob was already absent.
    async fn delete_blob(&self, key: &str) -> Result<()>;
}

/// Asynchronous relational metadata store trait.
///
/// Implemented by [`PgMetadataStore`] (PostgreSQL) for cloud mode and
/// [`SqliteMetadataStore`] (SQLite) for embedded/P2P mode.
#[allow(async_fn_in_trait)]
pub trait MetadataStore: Send + Sync {
    /// Inserts a new user account into the store.
    async fn create_user(&self, user: &User) -> Result<User>;

    /// Fetches a user record by email address.
    async fn get_user_by_email(&self, email: &str) -> Result<Option<User>>;

    /// Creates a new project record.
    async fn create_project(&self, project: &ProjectInfo) -> Result<ProjectInfo>;

    /// Fetches a project by its unique identifier.
    async fn get_project(&self, id: Uuid) -> Result<Option<ProjectInfo>>;

    /// Lists all projects in the store, ordered by name.
    async fn list_projects(&self) -> Result<Vec<ProjectInfo>>;

    /// Updates project metadata, tracking branches, git commits, or active session claims.
    async fn update_project(&self, project: &ProjectInfo) -> Result<ProjectInfo>;

    /// Registers a new machine or updates an existing machine entry.
    async fn create_machine(&self, machine: &MachineInfo) -> Result<MachineInfo>;

    /// Lists all registered machines, ordered by last seen timestamp descending.
    async fn list_machines(&self) -> Result<Vec<MachineInfo>>;

    /// Acquires an exclusive session lock on a project for a machine.
    async fn create_session_lock(&self, lock: &SessionLock) -> Result<SessionLock>;

    /// Fetches the current session lock for a project, if locked.
    async fn get_session_lock(&self, project_id: Uuid) -> Result<Option<SessionLock>>;

    /// Updates the heartbeat timestamp for an active project session lock.
    async fn update_heartbeat(&self, project_id: Uuid, heartbeat: DateTime<Utc>) -> Result<()>;

    /// Deletes/releases the session lock held on a project.
    async fn delete_session_lock(&self, project_id: Uuid) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_creation_and_serialization() {
        let user = User::new("test@example.com", "$argon2id$mockhash");
        assert_eq!(user.email, "test@example.com");
        assert_eq!(user.password_hash, "$argon2id$mockhash");

        let json = serde_json::to_string(&user).expect("Serialize user");
        let deser: User = serde_json::from_str(&json).expect("Deserialize user");
        assert_eq!(user, deser);
    }

    #[test]
    fn test_user_with_details() {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let user = User::with_details(id, "admin@ctx.dev", "hash_secret", now);

        assert_eq!(user.id, id);
        assert_eq!(user.email, "admin@ctx.dev");
        assert_eq!(user.password_hash, "hash_secret");
        assert_eq!(user.created_at, now);
    }

    #[test]
    fn test_storage_error_display() {
        let not_found = StorageError::NotFound("project-1".to_string());
        assert_eq!(not_found.to_string(), "Not found: project-1");

        let conflict = StorageError::Conflict("duplicate key".to_string());
        assert_eq!(conflict.to_string(), "Conflict: duplicate key");

        let proj_id = Uuid::new_v4();
        let mach_id = Uuid::new_v4();
        let lock_conflict = StorageError::LockConflict {
            project_id: proj_id,
            held_by: mach_id,
        };
        assert!(lock_conflict.to_string().contains(&proj_id.to_string()));
        assert!(lock_conflict.to_string().contains(&mach_id.to_string()));
    }
}
