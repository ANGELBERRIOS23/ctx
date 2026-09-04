//! Distributed session lock API handlers for `ctx-server`.
//!
//! Manages exclusive write claims by machines on projects, enforcing
//! staleness timeouts (heartbeats older than 120s allow takeovers),
//! lock release, and periodic heartbeat refreshes.

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use ctx_core::protocol::SessionLock;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use thiserror::Error;
use uuid::Uuid;

use crate::api::auth::Claims;

/// Time in seconds after which a session lock without a heartbeat is considered stale.
pub const SESSION_LOCK_TIMEOUT_SECS: i64 = 120;

/// Errors that can occur during session lock operations.
#[derive(Debug, Error)]
pub enum SessionApiError {
    /// Database query or connection failure.
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    /// Project is currently locked by another active machine.
    #[error("Session is actively locked by machine {0} (expires in {1}s)")]
    Conflict(Uuid, i64),

    /// No active session lock was found for the requested project.
    #[error("Session lock not found for project: {0}")]
    NotFound(Uuid),

    /// Machine is forbidden from modifying a lock held by another machine.
    #[error("Forbidden: lock is held by machine {0}, requested by machine {1}")]
    Forbidden(Uuid, Uuid),

    /// Bad request or malformed parameters.
    #[error("Bad request: {0}")]
    BadRequest(String),

    /// Caller is not authorized.
    #[error("Unauthorized access")]
    Unauthorized,
}

impl From<&SessionApiError> for StatusCode {
    fn from(err: &SessionApiError) -> Self {
        match err {
            SessionApiError::Conflict(_, _) => StatusCode::CONFLICT,
            SessionApiError::NotFound(_) => StatusCode::NOT_FOUND,
            SessionApiError::Forbidden(_, _) => StatusCode::FORBIDDEN,
            SessionApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            SessionApiError::Unauthorized => StatusCode::UNAUTHORIZED,
            SessionApiError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<SessionApiError> for StatusCode {
    fn from(err: SessionApiError) -> Self {
        StatusCode::from(&err)
    }
}

impl IntoResponse for SessionApiError {
    fn into_response(self) -> axum::response::Response {
        let status = StatusCode::from(&self);
        let body = serde_json::json!({
            "error": self.to_string(),
            "status": status.as_u16(),
        });
        (status, Json(body)).into_response()
    }
}

/// A specialized [`Result`] type for session lock operations.
pub type Result<T, E = SessionApiError> = std::result::Result<T, E>;

/// Payload submitted to claim an exclusive session lock on a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimSessionRequest {
    /// Identifier of the project to lock.
    pub project_id: Uuid,
    /// Identifier of the machine claiming the lock.
    pub machine_id: Uuid,
    /// Optional AI agent identifier holding the active session.
    pub agent_name: Option<String>,
}

impl ClaimSessionRequest {
    /// Creates a new [`ClaimSessionRequest`].
    pub fn new(project_id: Uuid, machine_id: Uuid, agent_name: Option<String>) -> Self {
        Self {
            project_id,
            machine_id,
            agent_name,
        }
    }
}

/// Payload submitted to release an active session lock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseSessionRequest {
    /// Identifier of the project whose lock should be released.
    pub project_id: Uuid,
    /// Identifier of the machine releasing the lock.
    pub machine_id: Uuid,
}

impl ReleaseSessionRequest {
    /// Creates a new [`ReleaseSessionRequest`].
    pub fn new(project_id: Uuid, machine_id: Uuid) -> Self {
        Self {
            project_id,
            machine_id,
        }
    }
}

/// Response returned when a session lock is released.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseSessionResponse {
    /// Project UUID whose lock was released.
    pub project_id: Uuid,
    /// Whether the lock was successfully released.
    pub released: bool,
    /// Informational message.
    pub message: String,
}

/// Payload submitted to refresh a session lock heartbeat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartbeatRequest {
    /// Identifier of the locked project.
    pub project_id: Uuid,
    /// Identifier of the machine holding the lock.
    pub machine_id: Uuid,
}

impl HeartbeatRequest {
    /// Creates a new [`HeartbeatRequest`].
    pub fn new(project_id: Uuid, machine_id: Uuid) -> Self {
        Self {
            project_id,
            machine_id,
        }
    }
}

/// Claims an exclusive session lock on a project for a machine.
///
/// If an existing lock is held by another machine:
/// - If heartbeat is older than 120 seconds, the stale lock is taken over.
/// - If heartbeat is within 120 seconds, returns [`StatusCode::CONFLICT`].
/// If held by the same machine, the heartbeat is refreshed.
///
/// Route: `POST /api/session/claim`
pub async fn claim_session(
    State(pool): State<PgPool>,
    _claims: Claims,
    Json(req): Json<ClaimSessionRequest>,
) -> Result<(StatusCode, Json<SessionLock>)> {
    let now = Utc::now();

    let existing_lock_row = sqlx::query(
        r#"
        SELECT project_id, machine_id, locked_at, heartbeat
        FROM session_locks
        WHERE project_id = $1
        "#,
    )
    .bind(req.project_id)
    .fetch_optional(&pool)
    .await?;

    let (status, lock) = match existing_lock_row {
        Some(r) => {
            let existing_machine_id: Uuid = r.try_get("machine_id")?;
            let existing_locked_at: DateTime<Utc> = r.try_get("locked_at")?;
            let existing_heartbeat: DateTime<Utc> = r.try_get("heartbeat")?;

            let elapsed_secs = (now - existing_heartbeat).num_seconds();
            let is_stale = elapsed_secs > SESSION_LOCK_TIMEOUT_SECS;

            if existing_machine_id == req.machine_id {
                // Same machine: refresh heartbeat
                sqlx::query(
                    r#"
                    UPDATE session_locks
                    SET heartbeat = $1
                    WHERE project_id = $2
                    "#,
                )
                .bind(now)
                .bind(req.project_id)
                .execute(&pool)
                .await?;

                // Keep project record synchronized
                sqlx::query(
                    r#"
                    UPDATE projects
                    SET active_machine = $1,
                        active_agent = COALESCE($2, active_agent),
                        claimed_at = $3
                    WHERE id = $4
                    "#,
                )
                .bind(req.machine_id)
                .bind(&req.agent_name)
                .bind(now)
                .bind(req.project_id)
                .execute(&pool)
                .await
                .ok();

                (
                    StatusCode::OK,
                    SessionLock::with_timestamps(req.project_id, req.machine_id, existing_locked_at, now),
                )
            } else if !is_stale {
                // Active lock held by another machine
                let remaining_secs = SESSION_LOCK_TIMEOUT_SECS.saturating_sub(elapsed_secs);
                return Err(SessionApiError::Conflict(existing_machine_id, remaining_secs));
            } else {
                // Stale lock (>120s): takeover lock
                sqlx::query(
                    r#"
                    UPDATE session_locks
                    SET machine_id = $1, locked_at = $2, heartbeat = $3
                    WHERE project_id = $4
                    "#,
                )
                .bind(req.machine_id)
                .bind(now)
                .bind(now)
                .bind(req.project_id)
                .execute(&pool)
                .await?;

                sqlx::query(
                    r#"
                    UPDATE projects
                    SET active_machine = $1, active_agent = $2, claimed_at = $3
                    WHERE id = $4
                    "#,
                )
                .bind(req.machine_id)
                .bind(&req.agent_name)
                .bind(now)
                .bind(req.project_id)
                .execute(&pool)
                .await
                .ok();

                (
                    StatusCode::OK,
                    SessionLock::with_timestamps(req.project_id, req.machine_id, now, now),
                )
            }
        }
        None => {
            // No lock exists: insert new session lock
            sqlx::query(
                r#"
                INSERT INTO session_locks (project_id, machine_id, locked_at, heartbeat)
                VALUES ($1, $2, $3, $4)
                ON CONFLICT (project_id) DO UPDATE SET
                    machine_id = EXCLUDED.machine_id,
                    locked_at = EXCLUDED.locked_at,
                    heartbeat = EXCLUDED.heartbeat
                "#,
            )
            .bind(req.project_id)
            .bind(req.machine_id)
            .bind(now)
            .bind(now)
            .execute(&pool)
            .await?;

            sqlx::query(
                r#"
                UPDATE projects
                SET active_machine = $1, active_agent = $2, claimed_at = $3
                WHERE id = $4
                "#,
            )
            .bind(req.machine_id)
            .bind(&req.agent_name)
            .bind(now)
            .bind(req.project_id)
            .execute(&pool)
            .await
            .ok();

            (
                StatusCode::CREATED,
                SessionLock::with_timestamps(req.project_id, req.machine_id, now, now),
            )
        }
    };

    Ok((status, Json(lock)))
}

/// Releases an active session lock for a project.
///
/// Only the holding machine (or a caller taking over a stale lock) can release it.
///
/// Route: `POST /api/session/release`
pub async fn release_session(
    State(pool): State<PgPool>,
    _claims: Claims,
    Json(req): Json<ReleaseSessionRequest>,
) -> Result<Json<ReleaseSessionResponse>> {
    let row = sqlx::query(
        r#"
        SELECT machine_id, heartbeat
        FROM session_locks
        WHERE project_id = $1
        "#,
    )
    .bind(req.project_id)
    .fetch_optional(&pool)
    .await?;

    if let Some(r) = row {
        let existing_machine: Uuid = r.try_get("machine_id")?;
        let existing_heartbeat: DateTime<Utc> = r.try_get("heartbeat")?;
        let now = Utc::now();
        let is_stale = (now - existing_heartbeat).num_seconds() > SESSION_LOCK_TIMEOUT_SECS;

        if existing_machine != req.machine_id && !is_stale {
            return Err(SessionApiError::Forbidden(existing_machine, req.machine_id));
        }

        sqlx::query(
            r#"
            DELETE FROM session_locks
            WHERE project_id = $1
            "#,
        )
        .bind(req.project_id)
        .execute(&pool)
        .await?;

        sqlx::query(
            r#"
            UPDATE projects
            SET active_machine = NULL, active_agent = NULL, claimed_at = NULL
            WHERE id = $1
            "#,
        )
        .bind(req.project_id)
        .execute(&pool)
        .await
        .ok();
    }

    Ok(Json(ReleaseSessionResponse {
        project_id: req.project_id,
        released: true,
        message: "Session lock released successfully".to_string(),
    }))
}

/// Refreshes the heartbeat timestamp of an active session lock.
///
/// Fails with [`StatusCode::NOT_FOUND`] if no lock exists, or
/// [`StatusCode::FORBIDDEN`] if held by another machine.
///
/// Route: `POST /api/session/heartbeat`
pub async fn heartbeat(
    State(pool): State<PgPool>,
    _claims: Claims,
    Json(req): Json<HeartbeatRequest>,
) -> Result<Json<SessionLock>> {
    let row = sqlx::query(
        r#"
        SELECT machine_id, locked_at
        FROM session_locks
        WHERE project_id = $1
        "#,
    )
    .bind(req.project_id)
    .fetch_optional(&pool)
    .await?;

    let row = match row {
        Some(r) => r,
        None => return Err(SessionApiError::NotFound(req.project_id)),
    };

    let existing_machine: Uuid = row.try_get("machine_id")?;
    let locked_at: DateTime<Utc> = row.try_get("locked_at")?;

    if existing_machine != req.machine_id {
        return Err(SessionApiError::Forbidden(existing_machine, req.machine_id));
    }

    let now = Utc::now();
    sqlx::query(
        r#"
        UPDATE session_locks
        SET heartbeat = $1
        WHERE project_id = $2 AND machine_id = $3
        "#,
    )
    .bind(now)
    .bind(req.project_id)
    .bind(req.machine_id)
    .execute(&pool)
    .await?;

    let updated_lock = SessionLock::with_timestamps(req.project_id, req.machine_id, locked_at, now);
    Ok(Json(updated_lock))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn test_claim_request_and_response_serde() {
        let proj_id = Uuid::new_v4();
        let mach_id = Uuid::new_v4();
        let req = ClaimSessionRequest::new(proj_id, mach_id, Some("codex".to_string()));

        assert_eq!(req.project_id, proj_id);
        assert_eq!(req.machine_id, mach_id);
        assert_eq!(req.agent_name.as_deref(), Some("codex"));

        let json = serde_json::to_string(&req).expect("Serialize claim request");
        let deser: ClaimSessionRequest =
            serde_json::from_str(&json).expect("Deserialize claim request");
        assert_eq!(req, deser);

        let release_resp = ReleaseSessionResponse {
            project_id: proj_id,
            released: true,
            message: "Released".to_string(),
        };
        let rel_json = serde_json::to_string(&release_resp).expect("Serialize release response");
        let deser_rel: ReleaseSessionResponse =
            serde_json::from_str(&rel_json).expect("Deserialize release response");
        assert_eq!(release_resp, deser_rel);
    }

    #[test]
    fn test_session_lock_staleness_calculation() {
        let proj_id = Uuid::new_v4();
        let mach_id = Uuid::new_v4();
        let now = Utc::now();

        // Fresh lock: heartbeat 30 seconds ago
        let fresh_heartbeat = now - Duration::seconds(30);
        let fresh_lock = SessionLock::with_timestamps(proj_id, mach_id, now, fresh_heartbeat);
        assert!(!fresh_lock.is_expired(Duration::seconds(SESSION_LOCK_TIMEOUT_SECS), now));

        // Stale lock: heartbeat 121 seconds ago (>120s)
        let stale_heartbeat = now - Duration::seconds(121);
        let stale_lock = SessionLock::with_timestamps(proj_id, mach_id, now, stale_heartbeat);
        assert!(stale_lock.is_expired(Duration::seconds(SESSION_LOCK_TIMEOUT_SECS), now));
    }

    #[test]
    fn test_session_api_error_status_codes() {
        let mach1 = Uuid::new_v4();
        let mach2 = Uuid::new_v4();
        let proj_id = Uuid::new_v4();

        let conflict = SessionApiError::Conflict(mach1, 45);
        assert_eq!(StatusCode::from(&conflict), StatusCode::CONFLICT);

        let not_found = SessionApiError::NotFound(proj_id);
        assert_eq!(StatusCode::from(&not_found), StatusCode::NOT_FOUND);

        let forbidden = SessionApiError::Forbidden(mach1, mach2);
        assert_eq!(StatusCode::from(&forbidden), StatusCode::FORBIDDEN);

        let resp = conflict.into_response();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }
}
