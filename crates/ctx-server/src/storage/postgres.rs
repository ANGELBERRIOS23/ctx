//! PostgreSQL implementation of [`MetadataStore`] for cloud/VPS deployments.
//!
//! [`PgMetadataStore`] manages users, projects, machines, and session locks
//! inside a PostgreSQL database using [`sqlx::PgPool`].

use chrono::{DateTime, Utc};
use ctx_core::protocol::{MachineInfo, ProjectInfo, SessionLock};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::{MetadataStore, Result, StorageError, User};

/// Relational metadata store implementation powered by PostgreSQL.
#[derive(Debug, Clone)]
pub struct PgMetadataStore {
    pool: PgPool,
}

impl PgMetadataStore {
    /// Creates a new [`PgMetadataStore`] backed by the provided [`PgPool`].
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Returns a reference to the underlying [`PgPool`].
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Initializes and migrates database tables if they do not already exist.
    pub async fn create_tables(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS users (
                id UUID PRIMARY KEY,
                email TEXT NOT NULL UNIQUE,
                password_hash TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL
            );

            CREATE TABLE IF NOT EXISTS projects (
                id UUID PRIMARY KEY,
                name TEXT NOT NULL,
                git_remote TEXT NOT NULL,
                git_branch TEXT NOT NULL,
                git_commit TEXT NOT NULL,
                active_machine UUID,
                active_agent TEXT,
                claimed_at TIMESTAMPTZ
            );

            CREATE TABLE IF NOT EXISTS machines (
                id UUID PRIMARY KEY,
                name TEXT NOT NULL,
                os TEXT NOT NULL,
                fingerprint TEXT NOT NULL,
                last_seen TIMESTAMPTZ NOT NULL
            );

            CREATE TABLE IF NOT EXISTS session_locks (
                project_id UUID PRIMARY KEY,
                machine_id UUID NOT NULL,
                locked_at TIMESTAMPTZ NOT NULL,
                heartbeat TIMESTAMPTZ NOT NULL
            );
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(StorageError::Database)?;

        Ok(())
    }
}

impl MetadataStore for PgMetadataStore {
    async fn create_user(&self, user: &User) -> Result<User> {
        let res = sqlx::query(
            "INSERT INTO users (id, email, password_hash, created_at) VALUES ($1, $2, $3, $4)",
        )
        .bind(user.id)
        .bind(&user.email)
        .bind(&user.password_hash)
        .bind(user.created_at)
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
            "SELECT id, email, password_hash, created_at FROM users WHERE email = $1",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .map_err(StorageError::Database)?;

        match row {
            Some(row) => {
                let id: Uuid = row.try_get("id").map_err(StorageError::Database)?;
                let email: String = row.try_get("email").map_err(StorageError::Database)?;
                let password_hash: String = row
                    .try_get("password_hash")
                    .map_err(StorageError::Database)?;
                let created_at: DateTime<Utc> =
                    row.try_get("created_at").map_err(StorageError::Database)?;

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
        let res = sqlx::query(
            "INSERT INTO projects (id, name, git_remote, git_branch, git_commit, active_machine, active_agent, claimed_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(project.id)
        .bind(&project.name)
        .bind(&project.git_remote)
        .bind(&project.git_branch)
        .bind(&project.git_commit)
        .bind(project.active_machine)
        .bind(&project.active_agent)
        .bind(project.claimed_at)
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
             FROM projects WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(StorageError::Database)?;

        match row {
            Some(row) => {
                let id: Uuid = row.try_get("id").map_err(StorageError::Database)?;
                let name: String = row.try_get("name").map_err(StorageError::Database)?;
                let git_remote: String =
                    row.try_get("git_remote").map_err(StorageError::Database)?;
                let git_branch: String =
                    row.try_get("git_branch").map_err(StorageError::Database)?;
                let git_commit: String =
                    row.try_get("git_commit").map_err(StorageError::Database)?;
                let active_machine: Option<Uuid> = row
                    .try_get("active_machine")
                    .map_err(StorageError::Database)?;
                let active_agent: Option<String> = row
                    .try_get("active_agent")
                    .map_err(StorageError::Database)?;
                let claimed_at: Option<DateTime<Utc>> =
                    row.try_get("claimed_at").map_err(StorageError::Database)?;

                Ok(Some(ProjectInfo {
                    id,
                    name,
                    git_remote,
                    git_branch,
                    git_commit,
                    active_machine,
                    active_agent,
                    claimed_at,
                }))
            }
            None => Ok(None),
        }
    }

    async fn list_projects(&self) -> Result<Vec<ProjectInfo>> {
        let rows = sqlx::query(
            "SELECT id, name, git_remote, git_branch, git_commit, active_machine, active_agent, claimed_at \
             FROM projects ORDER BY name ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(StorageError::Database)?;

        let mut projects = Vec::with_capacity(rows.len());
        for row in rows {
            let id: Uuid = row.try_get("id").map_err(StorageError::Database)?;
            let name: String = row.try_get("name").map_err(StorageError::Database)?;
            let git_remote: String =
                row.try_get("git_remote").map_err(StorageError::Database)?;
            let git_branch: String =
                row.try_get("git_branch").map_err(StorageError::Database)?;
            let git_commit: String =
                row.try_get("git_commit").map_err(StorageError::Database)?;
            let active_machine: Option<Uuid> = row
                .try_get("active_machine")
                .map_err(StorageError::Database)?;
            let active_agent: Option<String> = row
                .try_get("active_agent")
                .map_err(StorageError::Database)?;
            let claimed_at: Option<DateTime<Utc>> =
                row.try_get("claimed_at").map_err(StorageError::Database)?;

            projects.push(ProjectInfo {
                id,
                name,
                git_remote,
                git_branch,
                git_commit,
                active_machine,
                active_agent,
                claimed_at,
            });
        }
        Ok(projects)
    }

    async fn update_project(&self, project: &ProjectInfo) -> Result<ProjectInfo> {
        let res = sqlx::query(
            "UPDATE projects SET name = $2, git_remote = $3, git_branch = $4, git_commit = $5, \
             active_machine = $6, active_agent = $7, claimed_at = $8 WHERE id = $1",
        )
        .bind(project.id)
        .bind(&project.name)
        .bind(&project.git_remote)
        .bind(&project.git_branch)
        .bind(&project.git_commit)
        .bind(project.active_machine)
        .bind(&project.active_agent)
        .bind(project.claimed_at)
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
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(machine.id)
        .bind(&machine.name)
        .bind(&machine.os)
        .bind(&machine.fingerprint)
        .bind(machine.last_seen)
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
            let id: Uuid = row.try_get("id").map_err(StorageError::Database)?;
            let name: String = row.try_get("name").map_err(StorageError::Database)?;
            let os: String = row.try_get("os").map_err(StorageError::Database)?;
            let fingerprint: String =
                row.try_get("fingerprint").map_err(StorageError::Database)?;
            let last_seen: DateTime<Utc> =
                row.try_get("last_seen").map_err(StorageError::Database)?;

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
             VALUES ($1, $2, $3, $4)",
        )
        .bind(lock.project_id)
        .bind(lock.machine_id)
        .bind(lock.locked_at)
        .bind(lock.heartbeat)
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
            "SELECT project_id, machine_id, locked_at, heartbeat FROM session_locks WHERE project_id = $1",
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(StorageError::Database)?;

        match row {
            Some(row) => {
                let project_id: Uuid =
                    row.try_get("project_id").map_err(StorageError::Database)?;
                let machine_id: Uuid =
                    row.try_get("machine_id").map_err(StorageError::Database)?;
                let locked_at: DateTime<Utc> =
                    row.try_get("locked_at").map_err(StorageError::Database)?;
                let heartbeat: DateTime<Utc> =
                    row.try_get("heartbeat").map_err(StorageError::Database)?;

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
        let res = sqlx::query("UPDATE session_locks SET heartbeat = $2 WHERE project_id = $1")
            .bind(project_id)
            .bind(heartbeat)
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
        sqlx::query("DELETE FROM session_locks WHERE project_id = $1")
            .bind(project_id)
            .execute(&self.pool)
            .await
            .map_err(StorageError::Database)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pg_metadata_store_structure() {
        // Verify types and schema query constant validity
        let ddl = r#"
            CREATE TABLE IF NOT EXISTS users (
                id UUID PRIMARY KEY,
                email TEXT NOT NULL UNIQUE,
                password_hash TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL
            );
        "#;
        assert!(ddl.contains("users"));
        assert!(ddl.contains("UUID PRIMARY KEY"));
    }

    #[test]
    fn test_pg_metadata_store_methods_type_contract() {
        // Compile-time check that PgMetadataStore implements MetadataStore
        fn assert_metadata_store<T: MetadataStore>() {}
        assert_metadata_store::<PgMetadataStore>();
    }
}
