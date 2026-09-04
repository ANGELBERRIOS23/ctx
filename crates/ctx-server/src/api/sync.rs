//! Synchronization API handlers for `ctx-server`.
//!
//! Provides axum HTTP route handlers for pushing encrypted snapshots,
//! pulling the most recent snapshot for a project, and retrieving snapshot
//! history. Integrates with PostgreSQL metadata and blob storage.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use ctx_core::protocol::{ParseSnapshotTypeError, SnapshotType, SyncSnapshot};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use thiserror::Error;
use uuid::Uuid;

use crate::api::auth::Claims;

/// Errors that can occur during synchronization API operations.
#[derive(Debug, Error)]
pub enum SyncApiError {
    /// Database query or connection failure.
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    /// Snapshot not found for the requested project.
    #[error("Snapshot not found for project: {0}")]
    NotFound(Uuid),

    /// Bad request or malformed input payload.
    #[error("Bad request: {0}")]
    BadRequest(String),

    /// Caller is not authorized to access this resource.
    #[error("Unauthorized access")]
    Unauthorized,
}

impl From<&SyncApiError> for StatusCode {
    fn from(err: &SyncApiError) -> Self {
        match err {
            SyncApiError::NotFound(_) => StatusCode::NOT_FOUND,
            SyncApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            SyncApiError::Unauthorized => StatusCode::UNAUTHORIZED,
            SyncApiError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<SyncApiError> for StatusCode {
    fn from(err: SyncApiError) -> Self {
        StatusCode::from(&err)
    }
}

impl IntoResponse for SyncApiError {
    fn into_response(self) -> axum::response::Response {
        let status = StatusCode::from(&self);
        let body = serde_json::json!({
            "error": self.to_string(),
            "status": status.as_u16(),
        });
        (status, Json(body)).into_response()
    }
}

/// A specialized [`Result`] type for synchronization API operations.
pub type Result<T, E = SyncApiError> = std::result::Result<T, E>;

/// Response payload returned upon successfully storing a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushSnapshotResponse {
    /// Unique identifier of the stored snapshot.
    pub snapshot_id: Uuid,
    /// Identifier of the project this snapshot belongs to.
    pub project_id: Uuid,
    /// Confirmation message.
    pub message: String,
}

impl PushSnapshotResponse {
    /// Creates a new [`PushSnapshotResponse`].
    pub fn new(snapshot_id: Uuid, project_id: Uuid, message: impl Into<String>) -> Self {
        Self {
            snapshot_id,
            project_id,
            message: message.into(),
        }
    }
}

/// Helper function to parse a SQL row into a [`SyncSnapshot`].
fn row_to_snapshot(row: &sqlx::postgres::PgRow) -> Result<SyncSnapshot> {
    let id: Uuid = row.try_get("id")?;
    let project_id: Uuid = row.try_get("project_id")?;
    let machine_id: Uuid = row.try_get("machine_id")?;
    let snapshot_type_str: String = row.try_get("snapshot_type")?;
    let snapshot_type: SnapshotType = snapshot_type_str
        .parse()
        .map_err(|e: ParseSnapshotTypeError| SyncApiError::BadRequest(e.0))?;
    let git_commit: String = row.try_get("git_commit")?;
    let handoff_blob: Vec<u8> = row.try_get("handoff_blob")?;
    let memory_blob: Option<Vec<u8>> = row.try_get("memory_blob")?;
    let state_json: serde_json::Value = row.try_get("state_json")?;
    let created_at: DateTime<Utc> = row.try_get("created_at")?;

    let mut snapshot = SyncSnapshot::new(
        project_id,
        machine_id,
        snapshot_type,
        git_commit,
        handoff_blob,
    )
    .with_optional_memory_blob(memory_blob)
    .with_state_json(state_json);

    snapshot.id = id;
    snapshot.created_at = created_at;

    Ok(snapshot)
}

/// Pushes a new encrypted [`SyncSnapshot`] to the server.
///
/// Stores snapshot metadata and binary payloads (`handoff_blob`, `memory_blob`)
/// into the PostgreSQL database. Also updates the project's recorded git commit.
///
/// Route: `POST /api/sync/push`
pub async fn push_snapshot(
    State(pool): State<PgPool>,
    _claims: Claims,
    Json(snapshot): Json<SyncSnapshot>,
) -> Result<(StatusCode, Json<PushSnapshotResponse>)> {
    if snapshot.handoff_blob.is_empty() {
        return Err(SyncApiError::BadRequest(
            "Snapshot handoff blob cannot be empty".to_string(),
        ));
    }

    let snapshot_type_str = snapshot.snapshot_type.as_str();

    // Insert snapshot into PostgreSQL (storing both metadata and blobs)
    sqlx::query(
        r#"
        INSERT INTO snapshots (
            id, project_id, machine_id, snapshot_type, git_commit,
            handoff_blob, memory_blob, state_json, created_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        ON CONFLICT (id) DO UPDATE SET
            snapshot_type = EXCLUDED.snapshot_type,
            git_commit = EXCLUDED.git_commit,
            handoff_blob = EXCLUDED.handoff_blob,
            memory_blob = EXCLUDED.memory_blob,
            state_json = EXCLUDED.state_json,
            created_at = EXCLUDED.created_at
        "#,
    )
    .bind(snapshot.id)
    .bind(snapshot.project_id)
    .bind(snapshot.machine_id)
    .bind(snapshot_type_str)
    .bind(&snapshot.git_commit)
    .bind(&snapshot.handoff_blob)
    .bind(&snapshot.memory_blob)
    .bind(&snapshot.state_json)
    .bind(snapshot.created_at)
    .execute(&pool)
    .await?;

    // Optionally update project git commit and updated_at timestamp
    let now = Utc::now();
    sqlx::query(
        r#"
        UPDATE projects
        SET git_commit = $1, updated_at = $2
        WHERE id = $3
        "#,
    )
    .bind(&snapshot.git_commit)
    .bind(now)
    .bind(snapshot.project_id)
    .execute(&pool)
    .await
    .ok();

    let response = PushSnapshotResponse::new(
        snapshot.id,
        snapshot.project_id,
        "Snapshot stored successfully",
    );

    Ok((StatusCode::CREATED, Json(response)))
}

/// Retrieves the latest [`SyncSnapshot`] for a specific project.
///
/// Route: `GET /api/sync/latest/{project_id}`
pub async fn pull_latest(
    State(pool): State<PgPool>,
    _claims: Claims,
    Path(project_id): Path<Uuid>,
) -> Result<Json<SyncSnapshot>> {
    let row = sqlx::query(
        r#"
        SELECT id, project_id, machine_id, snapshot_type, git_commit,
               handoff_blob, memory_blob, state_json, created_at
        FROM snapshots
        WHERE project_id = $1
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(project_id)
    .fetch_optional(&pool)
    .await?;

    match row {
        Some(ref r) => {
            let snapshot = row_to_snapshot(r)?;
            Ok(Json(snapshot))
        }
        None => Err(SyncApiError::NotFound(project_id)),
    }
}

/// Lists historical [`SyncSnapshot`] records for a given project, newest first.
///
/// Route: `GET /api/sync/{project_id}/history`
pub async fn list_snapshots(
    State(pool): State<PgPool>,
    _claims: Claims,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Vec<SyncSnapshot>>> {
    let rows = sqlx::query(
        r#"
        SELECT id, project_id, machine_id, snapshot_type, git_commit,
               handoff_blob, memory_blob, state_json, created_at
        FROM snapshots
        WHERE project_id = $1
        ORDER BY created_at DESC
        LIMIT 100
        "#,
    )
    .bind(project_id)
    .fetch_all(&pool)
    .await?;

    let mut snapshots = Vec::with_capacity(rows.len());
    for r in &rows {
        snapshots.push(row_to_snapshot(r)?);
    }

    Ok(Json(snapshots))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_snapshot_response_serde() {
        let snap_id = Uuid::new_v4();
        let proj_id = Uuid::new_v4();
        let resp = PushSnapshotResponse::new(snap_id, proj_id, "All good");

        assert_eq!(resp.snapshot_id, snap_id);
        assert_eq!(resp.project_id, proj_id);
        assert_eq!(resp.message, "All good");

        let json = serde_json::to_string(&resp).expect("Failed to serialize PushSnapshotResponse");
        let deser: PushSnapshotResponse =
            serde_json::from_str(&json).expect("Failed to deserialize PushSnapshotResponse");
        assert_eq!(resp, deser);
    }

    #[test]
    fn test_sync_api_error_status_code() {
        let proj_id = Uuid::new_v4();
        let not_found = SyncApiError::NotFound(proj_id);
        assert_eq!(StatusCode::from(&not_found), StatusCode::NOT_FOUND);

        let bad_request = SyncApiError::BadRequest("empty blob".to_string());
        assert_eq!(StatusCode::from(&bad_request), StatusCode::BAD_REQUEST);

        let unauth = SyncApiError::Unauthorized;
        assert_eq!(StatusCode::from(&unauth), StatusCode::UNAUTHORIZED);

        // Verify IntoResponse generates correct HTTP status
        let resp = not_found.into_response();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_sync_snapshot_construction_and_validation() {
        let proj_id = Uuid::new_v4();
        let machine_id = Uuid::new_v4();
        let handoff = vec![1, 2, 3, 4];
        let memory = vec![5, 6, 7];

        let snapshot = SyncSnapshot::new(
            proj_id,
            machine_id,
            SnapshotType::Manual,
            "commit-sha-test",
            handoff.clone(),
        )
        .with_memory_blob(memory.clone());

        assert_eq!(snapshot.project_id, proj_id);
        assert_eq!(snapshot.machine_id, machine_id);
        assert_eq!(snapshot.snapshot_type, SnapshotType::Manual);
        assert_eq!(snapshot.git_commit, "commit-sha-test");
        assert_eq!(snapshot.handoff_blob, handoff);
        assert_eq!(snapshot.memory_blob, Some(memory));
    }
}
