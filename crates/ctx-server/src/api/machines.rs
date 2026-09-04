//! Machine registration and inventory API handlers for `ctx-server`.
//!
//! Handles registering machine fingerprints, listing machines registered by
//! the authenticated user, and retrieving machine details.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use ctx_core::protocol::MachineInfo;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use thiserror::Error;
use uuid::Uuid;

use crate::api::auth::Claims;

/// Errors that can occur during machine API operations.
#[derive(Debug, Error)]
pub enum MachineApiError {
    /// Database query or connection failure.
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    /// Requested machine was not found.
    #[error("Machine not found: {0}")]
    NotFound(Uuid),

    /// Bad request or validation failure.
    #[error("Bad request: {0}")]
    BadRequest(String),

    /// Caller is not authorized.
    #[error("Unauthorized access")]
    Unauthorized,
}

impl From<&MachineApiError> for StatusCode {
    fn from(err: &MachineApiError) -> Self {
        match err {
            MachineApiError::NotFound(_) => StatusCode::NOT_FOUND,
            MachineApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            MachineApiError::Unauthorized => StatusCode::UNAUTHORIZED,
            MachineApiError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<MachineApiError> for StatusCode {
    fn from(err: MachineApiError) -> Self {
        StatusCode::from(&err)
    }
}

impl IntoResponse for MachineApiError {
    fn into_response(self) -> axum::response::Response {
        let status = StatusCode::from(&self);
        let body = serde_json::json!({
            "error": self.to_string(),
            "status": status.as_u16(),
        });
        (status, Json(body)).into_response()
    }
}

/// A specialized [`Result`] type for machine API operations.
pub type Result<T, E = MachineApiError> = std::result::Result<T, E>;

/// Payload submitted to register or update a machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterMachineRequest {
    /// Optional machine ID (generated automatically if omitted).
    pub id: Option<Uuid>,
    /// Human-readable hostname or display name.
    pub name: String,
    /// Operating system identifier (e.g. "macos", "linux", "windows").
    pub os: Option<String>,
    /// Unique hardware/host fingerprint for machine identification.
    pub fingerprint: String,
}

impl RegisterMachineRequest {
    /// Creates a new [`RegisterMachineRequest`].
    pub fn new(name: impl Into<String>, fingerprint: impl Into<String>) -> Self {
        Self {
            id: None,
            name: name.into(),
            os: None,
            fingerprint: fingerprint.into(),
        }
    }

    /// Sets the machine ID explicitly.
    pub fn with_id(mut self, id: Uuid) -> Self {
        self.id = Some(id);
        self
    }

    /// Sets the operating system identifier explicitly.
    pub fn with_os(mut self, os: impl Into<String>) -> Self {
        self.os = Some(os.into());
        self
    }
}

/// Helper function to parse a PostgreSQL row into a [`MachineInfo`].
fn row_to_machine_info(row: &sqlx::postgres::PgRow) -> Result<MachineInfo> {
    let id: Uuid = row.try_get("id")?;
    let name: String = row.try_get("name")?;
    let os: String = row.try_get("os")?;
    let fingerprint: String = row.try_get("fingerprint")?;
    let last_seen: DateTime<Utc> = row.try_get("last_seen")?;

    let mut machine = MachineInfo::new(id, name, os, fingerprint);
    machine.last_seen = last_seen;
    Ok(machine)
}

/// Registers a machine by hardware fingerprint, or updates its last-seen status.
///
/// Route: `POST /api/machines` or `POST /api/machines/register`
pub async fn register_machine(
    State(pool): State<PgPool>,
    claims: Claims,
    Json(req): Json<RegisterMachineRequest>,
) -> Result<(StatusCode, Json<MachineInfo>)> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err(MachineApiError::BadRequest(
            "Machine name cannot be empty".to_string(),
        ));
    }

    let fingerprint = req.fingerprint.trim();
    if fingerprint.is_empty() {
        return Err(MachineApiError::BadRequest(
            "Machine fingerprint cannot be empty".to_string(),
        ));
    }

    let os = req
        .os
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| std::env::consts::OS.to_string());

    let now = Utc::now();

    // Check if machine with this fingerprint already exists for user
    let existing = sqlx::query(
        r#"
        SELECT id, name, os, fingerprint, last_seen
        FROM machines
        WHERE user_id = $1 AND fingerprint = $2
        "#,
    )
    .bind(claims.sub)
    .bind(fingerprint)
    .fetch_optional(&pool)
    .await?;

    if let Some(r) = existing {
        let existing_id: Uuid = r.try_get("id")?;
        let updated_row = sqlx::query(
            r#"
            UPDATE machines
            SET name = $1, os = $2, last_seen = $3
            WHERE id = $4
            RETURNING id, name, os, fingerprint, last_seen
            "#,
        )
        .bind(name)
        .bind(&os)
        .bind(now)
        .bind(existing_id)
        .fetch_one(&pool)
        .await?;

        let machine = row_to_machine_info(&updated_row)?;
        return Ok((StatusCode::OK, Json(machine)));
    }

    let id = req.id.unwrap_or_else(Uuid::new_v4);

    sqlx::query(
        r#"
        INSERT INTO machines (id, user_id, name, os, fingerprint, last_seen, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#,
    )
    .bind(id)
    .bind(claims.sub)
    .bind(name)
    .bind(&os)
    .bind(fingerprint)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await?;

    let machine = MachineInfo::new(id, name, os, fingerprint);
    Ok((StatusCode::CREATED, Json(machine)))
}

/// Lists all machines registered to the authenticated user, ordered by last seen descending.
///
/// Route: `GET /api/machines`
pub async fn list_machines(
    State(pool): State<PgPool>,
    claims: Claims,
) -> Result<Json<Vec<MachineInfo>>> {
    let rows = sqlx::query(
        r#"
        SELECT id, name, os, fingerprint, last_seen
        FROM machines
        WHERE user_id = $1
        ORDER BY last_seen DESC
        "#,
    )
    .bind(claims.sub)
    .fetch_all(&pool)
    .await?;

    let mut machines = Vec::with_capacity(rows.len());
    for r in &rows {
        machines.push(row_to_machine_info(r)?);
    }

    Ok(Json(machines))
}

/// Retrieves details for a specific machine by ID.
///
/// Route: `GET /api/machines/{id}`
pub async fn get_machine(
    State(pool): State<PgPool>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> Result<Json<MachineInfo>> {
    let row = sqlx::query(
        r#"
        SELECT id, name, os, fingerprint, last_seen
        FROM machines
        WHERE id = $1 AND user_id = $2
        "#,
    )
    .bind(id)
    .bind(claims.sub)
    .fetch_optional(&pool)
    .await?;

    match row {
        Some(ref r) => Ok(Json(row_to_machine_info(r)?)),
        None => Err(MachineApiError::NotFound(id)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn test_register_machine_request_serde_and_builders() {
        let req = RegisterMachineRequest::new("dev-box", "fp-abcdef-123456")
            .with_os("linux");

        assert_eq!(req.name, "dev-box");
        assert_eq!(req.fingerprint, "fp-abcdef-123456");
        assert_eq!(req.os.as_deref(), Some("linux"));
        assert!(req.id.is_none());

        let custom_id = Uuid::new_v4();
        let req_with_id = req.with_id(custom_id);
        assert_eq!(req_with_id.id, Some(custom_id));

        let json = serde_json::to_string(&req_with_id).expect("Serialize RegisterMachineRequest");
        let deser: RegisterMachineRequest =
            serde_json::from_str(&json).expect("Deserialize RegisterMachineRequest");
        assert_eq!(req_with_id, deser);
    }

    #[test]
    fn test_machine_api_error_status_codes() {
        let mach_id = Uuid::new_v4();
        let not_found = MachineApiError::NotFound(mach_id);
        assert_eq!(StatusCode::from(&not_found), StatusCode::NOT_FOUND);

        let bad_request = MachineApiError::BadRequest("empty fingerprint".to_string());
        assert_eq!(StatusCode::from(&bad_request), StatusCode::BAD_REQUEST);

        let unauth = MachineApiError::Unauthorized;
        assert_eq!(StatusCode::from(&unauth), StatusCode::UNAUTHORIZED);

        let resp = not_found.into_response();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_machine_info_activity() {
        let id = Uuid::new_v4();
        let machine = MachineInfo::new(id, "mac-mini", "macos", "fp-9988");
        let now = machine.last_seen;
        let timeout = Duration::seconds(60);

        assert!(machine.is_active(timeout, now));
        assert!(machine.is_active(timeout, now + Duration::seconds(30)));
        assert!(!machine.is_active(timeout, now + Duration::seconds(61)));
    }
}
