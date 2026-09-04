//! SQLite implementation of [`MetadataStore`] for embedded and P2P operation.
//!
//! [`SqliteMetadataStore`] manages users, projects, machines, and session locks
//! inside a local SQLite database using [`sqlx::SqlitePool`].

use chrono::{DateTime, Utc};
use ctx_core::protocol::{MachineInfo, ProjectInfo, SessionLock};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use super::{MetadataStore, Result, StorageError, User};

/// Relational metadata store implementation powered by SQLite.
#[derive(Debug, Clone)]
pub struct SqliteMetadataStore {
    pool: SqlitePool,
}

impl SqliteMetadataStore {
    /// Creates a new [`SqliteMetadataStore`] backed by the provided [`SqlitePool`].
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Returns a reference to the underlying [`SqlitePool`].
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Initializes and migrates database tables if they do not already exist.
    pub async fn create_tables(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS users (
                id TEXT PRIMARY KEY,
                email TEXT NOT NULL UNIQUE,
                password_hash TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS projects (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                git_remote TEXT NOT NULL,
                git_branch TEXT NOT NULL,
                git_commit TEXT NOT NULL,
                active_machine TEXT,
                active_agent TEXT,
                claimed_at TEXT
            );

            CREATE TABLE IF NOT EXISTS machines (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                os TEXT NOT NULL,
                fingerprint TEXT NOT NULL,
                last_seen TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS session_locks (
                project_id TEXT PRIMARY KEY,
                machine_id TEXT NOT NULL,
                locked_at TEXT NOT NULL,
                heartbeat TEXT NOT NULL
            );
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(StorageError::Database)?;

        Ok(())
    }
}

fn parse_project_row(row: &sqlx::sqlite::SqliteRow) -> Result<ProjectInfo> {
    let id_str: String = row.try_get("id").map_err(StorageError::Database)?;
    let id = Uuid::parse_str(&id_str)
        .map_err(|e| StorageError::Serialization(format!("Invalid project UUID: {e}")))?;
    let name: String = row.try_get("name").map_err(StorageError::Database)?;
    let git_remote: String = row.try_get("git_remote").map_err(StorageError::Database)?;
    let git_branch: String = row.try_get("git_branch").map_err(StorageError::Database)?;
    let git_commit: String = row.try_get("git_commit").map_err(StorageError::Database)?;
    let active_machine: Option<String> = row
        .try_get("active_machine")
        .map_err(StorageError::Database)?;
    let active_machine = active_machine
        .map(|s| Uuid::parse_str(&s))
        .transpose()
        .map_err(|e| StorageError::Serialization(format!("Invalid active machine UUID: {e}")))?;
    let active_agent: Option<String> =
        row.try_get("active_agent").map_err(StorageError::Database)?;
    let claimed_at_str: Option<String> =
        row.try_get("claimed_at").map_err(StorageError::Database)?;
    let claimed_at = claimed_at_str
        .map(|s| DateTime::parse_from_rfc3339(&s).map(|dt| dt.with_timezone(&Utc)))
        .transpose()
        .map_err(|e| StorageError::Serialization(format!("Invalid claimed_at timestamp: {e}")))?;

    Ok(ProjectInfo {
        id,
        name,
        git_remote,
        git_branch,
        git_commit,
        active_machine,
        active_agent,
        claimed_at,
    })
}

impl MetadataStore for SqliteMetadataStore {
    async fn create_user(&self, user: &User) -> Result<User> {
        let res = sqlx::query(
            "INSERT INTO users (id, email, password_hash, created_at) VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(user.id.to_string())
        .bind(&user.email)
        .bind(&user.password_hash)
        .bind(user.created_at.to_rfc3339())
        .execute(&self.pool)
        .await;

        match res {
            Ok(_) => Ok(user.clone()),
            Err(err) => {
                if err
                    .as_database_error()
                    .is_some_and(|db_err| db_err.is_unique_violation())
                {
                    return Err(StorageError::Conflict(format!(
                        "User with email '{}' already exists",
                        user.email
                    )));
                }
                Err(StorageError::Database(err))
            }
        }
    }

    async fn get_user_by_email(&self, email: &str) -> Result<Option<User>> {
        let row = sqlx::query(
            "SELECT id, email, password_hash, created_at FROM users WHERE email = ?1",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .map_err(StorageError::Database)?;

        match row {
            Some(row) => {
                let id_str: String = row.try_get("id").map_err(StorageError::Database)?;
                let id = Uuid::parse_str(&id_str).map_err(|e| {
                    StorageError::Serialization(format!("Invalid user UUID in DB: {e}"))
                })?;
                let email: String = row.try_get("email").map_err(StorageError::Database)?;
                let password_hash: String = row
                    .try_get("password_hash")
                    .map_err(StorageError::Database)?;
                let created_at_str: String =
                    row.try_get("created_at").map_err(StorageError::Database)?;
                let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .map_err(|e| {
                        StorageError::Serialization(format!("Invalid created_at timestamp: {e}"))
                    })?;

                Ok(Some(User {
                    id,
                    email,
                    password_hash,
                    created_at,
                }))
            }
            None => Ok(None),
        }
    }

    async fn create_project(&self, project: &ProjectInfo) -> Result<ProjectInfo> {
        let active_machine_str = project.active_machine.map(|id| id.to_string());
        let claimed_at_str = project.claimed_at.map(|dt| dt.to_rfc3339());

        let res = sqlx::query(
            "INSERT INTO projects (id, name, git_remote, git_branch, git_commit, active_machine, active_agent, claimed_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind(project.id.to_string())
        .bind(&project.name)
        .bind(&project.git_remote)
        .bind(&project.git_branch)
        .bind(&project.git_commit)
        .bind(active_machine_str)
        .bind(&project.active_agent)
        .bind(claimed_at_str)
        .execute(&self.pool)
        .await;

        match res {
            Ok(_) => Ok(project.clone()),
            Err(err) => {
                if err
                    .as_database_error()
                    .is_some_and(|db_err| db_err.is_unique_violation())
                {
                    return Err(StorageError::Conflict(format!(
                        "Project with ID '{}' already exists",
                        project.id
                    )));
                }
                Err(StorageError::Database(err))
            }
        }
    }

    async fn get_project(&self, id: Uuid) -> Result<Option<ProjectInfo>> {
        let row = sqlx::query(
            "SELECT id, name, git_remote, git_branch, git_commit, active_machine, active_agent, claimed_at \
             FROM projects WHERE id = ?1",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(StorageError::Database)?;

        row.as_ref().map(parse_project_row).transpose()
    }

    async fn list_projects(&self) -> Result<Vec<ProjectInfo>> {
        let rows = sqlx::query(
            "SELECT id, name, git_remote, git_branch, git_commit, active_machine, active_agent, claimed_at \
             FROM projects ORDER BY name ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(StorageError::Database)?;

        rows.iter().map(parse_project_row).collect()
    }

    async fn update_project(&self, project: &ProjectInfo) -> Result<ProjectInfo> {
        let active_machine_str = project.active_machine.map(|id| id.to_string());
        let claimed_at_str = project.claimed_at.map(|dt| dt.to_rfc3339());

        let res = sqlx::query(
            "UPDATE projects SET name = ?2, git_remote = ?3, git_branch = ?4, git_commit = ?5, \
             active_machine = ?6, active_agent = ?7, claimed_at = ?8 WHERE id = ?1",
        )
        .bind(project.id.to_string())
        .bind(&project.name)
        .bind(&project.git_remote)
        .bind(&project.git_branch)
        .bind(&project.git_commit)
        .bind(active_machine_str)
        .bind(&project.active_agent)
        .bind(claimed_at_str)
        .execute(&self.pool)
        .await
        .map_err(StorageError::Database)?;

        if res.rows_affected() == 0 {
            return Err(StorageError::NotFound(format!(
                "Project with ID '{}' not found",
                project.id
            )));
        }

        Ok(project.clone())
    }

    async fn create_machine(&self, machine: &MachineInfo) -> Result<MachineInfo> {
        let res = sqlx::query(
            "INSERT INTO machines (id, name, os, fingerprint, last_seen) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(machine.id.to_string())
        .bind(&machine.name)
        .bind(&machine.os)
        .bind(&machine.fingerprint)
        .bind(machine.last_seen.to_rfc3339())
        .execute(&self.pool)
        .await;

        match res {
            Ok(_) => Ok(machine.clone()),
            Err(err) => {
                if err
                    .as_database_error()
                    .is_some_and(|db_err| db_err.is_unique_violation())
                {
                    return Err(StorageError::Conflict(format!(
                        "Machine with ID '{}' already exists",
                        machine.id
                    )));
                }
                Err(StorageError::Database(err))
            }
        }
    }

    async fn list_machines(&self) -> Result<Vec<MachineInfo>> {
        let rows = sqlx::query(
            "SELECT id, name, os, fingerprint, last_seen FROM machines ORDER BY last_seen DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(StorageError::Database)?;

        let mut machines = Vec::with_capacity(rows.len());
        for row in rows {
            let id_str: String = row.try_get("id").map_err(StorageError::Database)?;
            let id = Uuid::parse_str(&id_str)
                .map_err(|e| StorageError::Serialization(format!("Invalid machine UUID: {e}")))?;
            let name: String = row.try_get("name").map_err(StorageError::Database)?;
            let os: String = row.try_get("os").map_err(StorageError::Database)?;
            let fingerprint: String =
                row.try_get("fingerprint").map_err(StorageError::Database)?;
            let last_seen_str: String =
                row.try_get("last_seen").map_err(StorageError::Database)?;
            let last_seen = DateTime::parse_from_rfc3339(&last_seen_str)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| {
                    StorageError::Serialization(format!("Invalid last_seen timestamp: {e}"))
                })?;

            machines.push(MachineInfo {
                id,
                name,
                os,
                fingerprint,
                last_seen,
            });
        }
        Ok(machines)
    }

    async fn create_session_lock(&self, lock: &SessionLock) -> Result<SessionLock> {
        let res = sqlx::query(
            "INSERT INTO session_locks (project_id, machine_id, locked_at, heartbeat) \
             VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(lock.project_id.to_string())
        .bind(lock.machine_id.to_string())
        .bind(lock.locked_at.to_rfc3339())
        .bind(lock.heartbeat.to_rfc3339())
        .execute(&self.pool)
        .await;

        match res {
            Ok(_) => Ok(lock.clone()),
            Err(err) => {
                if err
                    .as_database_error()
                    .is_some_and(|db_err| db_err.is_unique_violation())
                {
                    if let Ok(Some(existing)) = self.get_session_lock(lock.project_id).await {
                        return Err(StorageError::LockConflict {
                            project_id: lock.project_id,
                            held_by: existing.machine_id,
                        });
                    }
                    return Err(StorageError::Conflict(format!(
                        "Lock already exists for project '{}'",
                        lock.project_id
                    )));
                }
                Err(StorageError::Database(err))
            }
        }
    }

    async fn get_session_lock(&self, project_id: Uuid) -> Result<Option<SessionLock>> {
        let row = sqlx::query(
            "SELECT project_id, machine_id, locked_at, heartbeat FROM session_locks WHERE project_id = ?1",
        )
        .bind(project_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(StorageError::Database)?;

        match row {
            Some(row) => {
                let proj_str: String =
                    row.try_get("project_id").map_err(StorageError::Database)?;
                let project_id = Uuid::parse_str(&proj_str).map_err(|e| {
                    StorageError::Serialization(format!("Invalid lock project UUID: {e}"))
                })?;
                let mach_str: String =
                    row.try_get("machine_id").map_err(StorageError::Database)?;
                let machine_id = Uuid::parse_str(&mach_str).map_err(|e| {
                    StorageError::Serialization(format!("Invalid lock machine UUID: {e}"))
                })?;
                let locked_at_str: String =
                    row.try_get("locked_at").map_err(StorageError::Database)?;
                let locked_at = DateTime::parse_from_rfc3339(&locked_at_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .map_err(|e| {
                        StorageError::Serialization(format!("Invalid locked_at timestamp: {e}"))
                    })?;
                let heartbeat_str: String =
                    row.try_get("heartbeat").map_err(StorageError::Database)?;
                let heartbeat = DateTime::parse_from_rfc3339(&heartbeat_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .map_err(|e| {
                        StorageError::Serialization(format!("Invalid heartbeat timestamp: {e}"))
                    })?;

                Ok(Some(SessionLock {
                    project_id,
                    machine_id,
                    locked_at,
                    heartbeat,
                }))
            }
            None => Ok(None),
        }
    }

    async fn update_heartbeat(&self, project_id: Uuid, heartbeat: DateTime<Utc>) -> Result<()> {
        let res =
            sqlx::query("UPDATE session_locks SET heartbeat = ?2 WHERE project_id = ?1")
                .bind(project_id.to_string())
                .bind(heartbeat.to_rfc3339())
                .execute(&self.pool)
                .await
                .map_err(StorageError::Database)?;

        if res.rows_affected() == 0 {
            return Err(StorageError::NotFound(format!(
                "Session lock for project '{project_id}' not found"
            )));
        }

        Ok(())
    }

    async fn delete_session_lock(&self, project_id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM session_locks WHERE project_id = ?1")
            .bind(project_id.to_string())
            .execute(&self.pool)
            .await
            .map_err(StorageError::Database)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_test_sqlite_store() -> SqliteMetadataStore {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("Failed to connect to in-memory sqlite");
        let store = SqliteMetadataStore::new(pool);
        store.create_tables().await.expect("Failed to create tables");
        store
    }

    #[tokio::test]
    async fn test_sqlite_user_lifecycle() {
        let store = setup_test_sqlite_store().await;

        let user = User::new("dev@example.com", "$argon2id$mock_hash_123");
        let created = store.create_user(&user).await.expect("Create user");
        assert_eq!(created.email, "dev@example.com");

        // Fetch user
        let fetched = store
            .get_user_by_email("dev@example.com")
            .await
            .expect("Get user")
            .expect("User should exist");
        assert_eq!(fetched.id, user.id);
        assert_eq!(fetched.email, "dev@example.com");

        // Duplicate user conflict
        let err = store.create_user(&user).await.expect_err("Duplicate email");
        assert!(matches!(err, StorageError::Conflict(_)));

        // Missing user
        let missing = store
            .get_user_by_email("nobody@example.com")
            .await
            .expect("Get non-existent user");
        assert_eq!(missing, None);
    }

    #[tokio::test]
    async fn test_sqlite_project_lifecycle() {
        let store = setup_test_sqlite_store().await;

        let id = Uuid::new_v4();
        let project = ProjectInfo::new(
            id,
            "ctx-core",
            "git@github.com:example/ctx.git",
            "main",
            "abc1234",
        );

        // Create
        store.create_project(&project).await.expect("Create project");

        // Get
        let fetched = store
            .get_project(id)
            .await
            .expect("Get project")
            .expect("Project must exist");
        assert_eq!(fetched.name, "ctx-core");
        assert_eq!(fetched.git_remote, "git@github.com:example/ctx.git");

        // List
        let list = store.list_projects().await.expect("List projects");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id);

        // Update
        let mut updated = fetched;
        let machine_id = Uuid::new_v4();
        updated.claim(machine_id, Some("claude-code".to_string()), Utc::now());
        store.update_project(&updated).await.expect("Update project");

        let fetched_after_update = store
            .get_project(id)
            .await
            .expect("Get updated")
            .expect("Must exist");
        assert_eq!(fetched_after_update.active_machine, Some(machine_id));
        assert_eq!(
            fetched_after_update.active_agent.as_deref(),
            Some("claude-code")
        );
    }

    #[tokio::test]
    async fn test_sqlite_machine_and_session_lock() {
        let store = setup_test_sqlite_store().await;

        let machine = MachineInfo::new(Uuid::new_v4(), "mac-studio", "macos", "fp-5566");
        store.create_machine(&machine).await.expect("Create machine");

        let machines = store.list_machines().await.expect("List machines");
        assert_eq!(machines.len(), 1);
        assert_eq!(machines[0].name, "mac-studio");

        // Session lock
        let project_id = Uuid::new_v4();
        let lock = SessionLock::new(project_id, machine.id);
        store.create_session_lock(&lock).await.expect("Create lock");

        let fetched_lock = store
            .get_session_lock(project_id)
            .await
            .expect("Get lock")
            .expect("Lock must exist");
        assert_eq!(fetched_lock.machine_id, machine.id);

        // Conflict when attempting to create another lock on same project
        let other_machine = Uuid::new_v4();
        let conflicting_lock = SessionLock::new(project_id, other_machine);
        let err = store
            .create_session_lock(&conflicting_lock)
            .await
            .expect_err("Conflicting lock");
        match err {
            StorageError::LockConflict {
                project_id: p,
                held_by,
            } => {
                assert_eq!(p, project_id);
                assert_eq!(held_by, machine.id);
            }
            other => panic!("Expected LockConflict, got {other:?}"),
        }

        // Heartbeat update
        let new_heartbeat = Utc::now();
        store
            .update_heartbeat(project_id, new_heartbeat)
            .await
            .expect("Update heartbeat");

        // Delete lock
        store
            .delete_session_lock(project_id)
            .await
            .expect("Delete lock");
        let after_delete = store
            .get_session_lock(project_id)
            .await
            .expect("Get after delete");
        assert_eq!(after_delete, None);
    }
}
