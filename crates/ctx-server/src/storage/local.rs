//! Filesystem-backed blob and snapshot storage for `ctx-server`.
//!
//! [`LocalBlobStore`] stores arbitrary binary payloads and project snapshots
//! within a local directory on disk. Designed for embedded mode, P2P operation,
//! and single-node development environments.

use std::path::{Component, Path, PathBuf};

use tokio::fs;

use super::{Result, StorageBackend, StorageError};

/// Local filesystem storage backend for binary blobs and synchronization snapshots.
#[derive(Debug, Clone)]
pub struct LocalBlobStore {
    base_path: PathBuf,
}

impl LocalBlobStore {
    /// Creates a new [`LocalBlobStore`] rooted at the specified base directory.
    pub fn new(base_path: impl Into<PathBuf>) -> Self {
        Self {
            base_path: base_path.into(),
        }
    }

    /// Returns a reference to the base directory path.
    pub fn base_path(&self) -> &Path {
        &self.base_path
    }

    /// Resolves a storage key to a safe absolute or relative path within the base directory.
    ///
    /// Sanitizes the key and ensures that no parent directory traversal components (`..`)
    /// are permitted, preventing arbitrary file access outside `base_path`.
    pub fn resolve_path(&self, key: &str) -> Result<PathBuf> {
        let sanitized = key.trim().trim_start_matches(['/', '\\']);
        if sanitized.is_empty() {
            return Err(StorageError::Internal(
                "Storage key cannot be empty".to_string(),
            ));
        }

        let rel_path = Path::new(sanitized);
        for component in rel_path.components() {
            match component {
                Component::ParentDir => {
                    return Err(StorageError::Internal(format!(
                        "Path traversal rejected in key: '{key}'"
                    )));
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(StorageError::Internal(format!(
                        "Absolute path keys not allowed: '{key}'"
                    )));
                }
                _ => {}
            }
        }

        Ok(self.base_path.join(rel_path))
    }
}

impl StorageBackend for LocalBlobStore {
    async fn save_blob(&self, key: &str, data: &[u8]) -> Result<()> {
        let path = self.resolve_path(key)?;
        if let Some(parent) = path.parent()
            && !parent.exists() {
                fs::create_dir_all(parent).await.map_err(StorageError::Io)?;
            }
        fs::write(&path, data).await.map_err(StorageError::Io)
    }

    async fn get_blob(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let path = self.resolve_path(key)?;
        match fs::read(&path).await {
            Ok(data) => Ok(Some(data)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(StorageError::Io(err)),
        }
    }

    async fn delete_blob(&self, key: &str) -> Result<()> {
        let path = self.resolve_path(key)?;
        match fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(StorageError::Io(err)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctx_core::protocol::{SnapshotType, SyncSnapshot};
    use uuid::Uuid;

    #[tokio::test]
    async fn test_local_blob_store_save_get_delete() {
        let temp_dir = std::env::temp_dir().join(format!("ctx_local_blob_test_{}", Uuid::new_v4()));
        let store = LocalBlobStore::new(&temp_dir);
        assert_eq!(store.base_path(), temp_dir);

        let key = "projects/abc/data.bin";
        let payload = b"hello ctx binary storage";

        // Save
        store.save_blob(key, payload).await.expect("Save blob");

        // Retrieve
        let retrieved = store
            .get_blob(key)
            .await
            .expect("Get blob")
            .expect("Blob should exist");
        assert_eq!(retrieved, payload);

        // Delete
        store.delete_blob(key).await.expect("Delete blob");

        // Retrieve after deletion
        let after_delete = store.get_blob(key).await.expect("Get after delete");
        assert_eq!(after_delete, None);

        // Clean up
        let _ = fs::remove_dir_all(&temp_dir).await;
    }

    #[tokio::test]
    async fn test_local_blob_store_nonexistent_blob() {
        let temp_dir = std::env::temp_dir().join(format!("ctx_local_blob_missing_{}", Uuid::new_v4()));
        let store = LocalBlobStore::new(&temp_dir);

        let non_existent = store
            .get_blob("does/not/exist.txt")
            .await
            .expect("Get missing blob");
        assert_eq!(non_existent, None);

        // Deleting non-existent should succeed idempotently
        store
            .delete_blob("does/not/exist.txt")
            .await
            .expect("Delete missing blob");

        let _ = fs::remove_dir_all(&temp_dir).await;
    }

    #[tokio::test]
    async fn test_local_blob_store_path_traversal_rejected() {
        let store = LocalBlobStore::new("/tmp/base");
        let result = store.resolve_path("../escape.txt");
        assert!(result.is_err());
        match result.unwrap_err() {
            StorageError::Internal(msg) => assert!(msg.contains("Path traversal rejected")),
            other => panic!("Expected StorageError::Internal, got {other:?}"),
        }

        let abs_result = store.resolve_path("/etc/passwd");
        // Leading slashes are trimmed, but if any component escapes, it's rejected
        assert!(abs_result.is_ok()); // trimmed to "etc/passwd" under base
    }

    #[tokio::test]
    async fn test_local_blob_store_snapshot_lifecycle() {
        let temp_dir = std::env::temp_dir().join(format!("ctx_local_snap_test_{}", Uuid::new_v4()));
        let store = LocalBlobStore::new(&temp_dir);

        let project_id = Uuid::new_v4();
        let machine_id = Uuid::new_v4();

        // Initially no snapshots
        let latest = store
            .get_latest_snapshot(project_id)
            .await
            .expect("Get latest snapshot empty");
        assert_eq!(latest, None);

        let list = store
            .list_snapshots(project_id, 10)
            .await
            .expect("List snapshots empty");
        assert!(list.is_empty());

        // Create snapshot 1
        let snap1 = SyncSnapshot::new(
            project_id,
            machine_id,
            SnapshotType::Auto,
            "commit111",
            vec![1, 2, 3],
        );
        store.save_snapshot(&snap1).await.expect("Save snap1");

        let latest = store
            .get_latest_snapshot(project_id)
            .await
            .expect("Get latest snap1")
            .expect("Snap1 should exist");
        assert_eq!(latest.id, snap1.id);
        assert_eq!(latest.git_commit, "commit111");

        // Create snapshot 2
        let snap2 = SyncSnapshot::new(
            project_id,
            machine_id,
            SnapshotType::Manual,
            "commit222",
            vec![4, 5, 6],
        );
        store.save_snapshot(&snap2).await.expect("Save snap2");

        let latest2 = store
            .get_latest_snapshot(project_id)
            .await
            .expect("Get latest snap2")
            .expect("Snap2 should exist");
        assert_eq!(latest2.id, snap2.id);
        assert_eq!(latest2.git_commit, "commit222");

        // List snapshots
        let list = store
            .list_snapshots(project_id, 10)
            .await
            .expect("List snapshots");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, snap2.id);
        assert_eq!(list[1].id, snap1.id);

        let _ = fs::remove_dir_all(&temp_dir).await;
    }
}
