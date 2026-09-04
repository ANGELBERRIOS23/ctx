//! Project management API handlers for `ctx-server`.
//!
//! Provides axum HTTP route handlers for CRUD operations on development projects:
//! creating new projects, listing projects for the authenticated user,
//! fetching project details by ID, and updating project metadata and Git revisions.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use ctx_core::protocol::ProjectInfo;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use thiserror::Error;
use uuid::Uuid;

use crate::api::auth::Claims;

/// Errors that can occur during project API operations.
#[derive(Debug, Error)]
pub enum ProjectApiError {
    /// Database query or connection failure.
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    /// Requested project was not found.
    #[error("Project not found: {0}")]
    NotFound(Uuid),

    /// Bad request or validation failure.
    #[error("Bad request: {0}")]
    BadRequest(String),

    /// Caller is not authorized.
    #[error("Unauthorized access")]
    Unauthorized,
}

impl From<&ProjectApiError> for StatusCode {
    fn from(err: &ProjectApiError) -> Self {
        match err {
            ProjectApiError::NotFound(_) => StatusCode::NOT_FOUND,
            ProjectApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ProjectApiError::Unauthorized => StatusCode::UNAUTHORIZED,
            ProjectApiError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<ProjectApiError> for StatusCode {
    fn from(err: ProjectApiError) -> Self {
        StatusCode::from(&err)
    }
}

impl IntoResponse for ProjectApiError {
    fn into_response(self) -> axum::response::Response {
        let status = StatusCode::from(&self);
        let body = serde_json::json!({
            "error": self.to_string(),
            "status": status.as_u16(),
        });
        (status, Json(body)).into_response()
    }
}

/// A specialized [`Result`] type for project API operations.
pub type Result<T, E = ProjectApiError> = std::result::Result<T, E>;

/// Payload submitted to create a new project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateProjectRequest {
    /// Optional project ID (generated automatically if omitted).
    pub id: Option<Uuid>,
    /// Display or repository name of the project.
    pub name: String,
    /// Remote Git repository URL or remote specifier.
    pub git_remote: String,
    /// Default Git branch (defaults to `"main"` if omitted).
    pub git_branch: Option<String>,
    /// Latest known Git commit SHA.
    pub git_commit: Option<String>,
}

impl CreateProjectRequest {
    /// Creates a new [`CreateProjectRequest`].
    pub fn new(name: impl Into<String>, git_remote: impl Into<String>) -> Self {
        Self {
            id: None,
            name: name.into(),
            git_remote: git_remote.into(),
            git_branch: None,
            git_commit: None,
        }
    }

    /// Sets the project ID explicitly.
    pub fn with_id(mut self, id: Uuid) -> Self {
        self.id = Some(id);
        self
    }

    /// Sets the Git branch and commit reference.
    pub fn with_git_revision(mut self, branch: impl Into<String>, commit: impl Into<String>) -> Self {
        self.git_branch = Some(branch.into());
        self.git_commit = Some(commit.into());
        self
    }
}

/// Payload submitted to update existing project metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateProjectRequest {
    /// Optional updated project display name.
    pub name: Option<String>,
    /// Optional updated remote Git repository URL.
    pub git_remote: Option<String>,
    /// Optional updated Git branch.
    pub git_branch: Option<String>,
    /// Optional updated Git commit SHA.
    pub git_commit: Option<String>,
}

impl UpdateProjectRequest {
    /// Creates an empty [`UpdateProjectRequest`].
    pub fn new() -> Self {
        Self {
            name: None,
            git_remote: None,
            git_branch: None,
            git_commit: None,
        }
    }
}

impl Default for UpdateProjectRequest {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper function to parse a PostgreSQL row into a [`ProjectInfo`].
fn row_to_project_info(row: &sqlx::postgres::PgRow) -> Result<ProjectInfo> {
    let id: Uuid = row.try_get("id")?;
    let name: String = row.try_get("name")?;
    let git_remote: String = row.try_get("git_remote")?;
    let git_branch: String = row.try_get("git_branch")?;
    let git_commit: String = row.try_get("git_commit")?;
    let active_machine: Option<Uuid> = row.try_get("active_machine")?;
    let active_agent: Option<String> = row.try_get("active_agent")?;
    let claimed_at: Option<DateTime<Utc>> = row.try_get("claimed_at")?;

    let mut project = ProjectInfo::new(id, name, git_remote, git_branch, git_commit);
    project.active_machine = active_machine;
    project.active_agent = active_agent;
    project.claimed_at = claimed_at;
    Ok(project)
}

/// Creates a new project in the system.
///
/// Route: `POST /api/projects`
pub async fn create_project(
    State(pool): State<PgPool>,
    claims: Claims,
    Json(req): Json<CreateProjectRequest>,
) -> Result<(StatusCode, Json<ProjectInfo>)> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ProjectApiError::BadRequest(
            "Project name cannot be empty".to_string(),
        ));
    }

    let git_remote = req.git_remote.trim();
    if git_remote.is_empty() {
        return Err(ProjectApiError::BadRequest(
            "Git remote URL cannot be empty".to_string(),
        ));
    }

    let id = req.id.unwrap_or_else(Uuid::new_v4);
    let git_branch = req
        .git_branch
        .map(|b| b.trim().to_string())
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| "main".to_string());
    let git_commit = req.git_commit.unwrap_or_default();
    let now = Utc::now();

    sqlx::query(
        r#"
        INSERT INTO projects (
            id, user_id, name, git_remote, git_branch, git_commit,
            active_machine, active_agent, claimed_at, created_at, updated_at
        ) VALUES ($1, $2, $3, $4, $5, $6, NULL, NULL, NULL, $7, $8)
        "#,
    )
    .bind(id)
    .bind(claims.sub)
    .bind(name)
    .bind(git_remote)
    .bind(&git_branch)
    .bind(&git_commit)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await?;

    let project = ProjectInfo::new(id, name, git_remote, git_branch, git_commit);
    Ok((StatusCode::CREATED, Json(project)))
}

/// Lists all projects accessible by the authenticated user.
///
/// Route: `GET /api/projects`
pub async fn list_projects(
    State(pool): State<PgPool>,
    claims: Claims,
) -> Result<Json<Vec<ProjectInfo>>> {
    let rows = sqlx::query(
        r#"
        SELECT id, name, git_remote, git_branch, git_commit,
               active_machine, active_agent, claimed_at
        FROM projects
        WHERE user_id = $1
        ORDER BY name ASC
        "#,
    )
    .bind(claims.sub)
    .fetch_all(&pool)
    .await?;

    let mut projects = Vec::with_capacity(rows.len());
    for r in &rows {
        projects.push(row_to_project_info(r)?);
    }

    Ok(Json(projects))
}

/// Retrieves the metadata and active session state of a single project by ID.
///
/// Route: `GET /api/projects/{id}`
pub async fn get_project(
    State(pool): State<PgPool>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> Result<Json<ProjectInfo>> {
    let row = sqlx::query(
        r#"
        SELECT id, name, git_remote, git_branch, git_commit,
               active_machine, active_agent, claimed_at
        FROM projects
        WHERE id = $1 AND user_id = $2
        "#,
    )
    .bind(id)
    .bind(claims.sub)
    .fetch_optional(&pool)
    .await?;

    match row {
        Some(ref r) => Ok(Json(row_to_project_info(r)?)),
        None => Err(ProjectApiError::NotFound(id)),
    }
}

/// Updates project metadata, tracking branch, or current Git commit.
///
/// Route: `PUT /api/projects/{id}` or `PATCH /api/projects/{id}`
pub async fn update_project(
    State(pool): State<PgPool>,
    claims: Claims,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateProjectRequest>,
) -> Result<Json<ProjectInfo>> {
    let now = Utc::now();

    let row = sqlx::query(
        r#"
        UPDATE projects
        SET name = COALESCE($1, name),
            git_remote = COALESCE($2, git_remote),
            git_branch = COALESCE($3, git_branch),
            git_commit = COALESCE($4, git_commit),
            updated_at = $5
        WHERE id = $6 AND user_id = $7
        RETURNING id, name, git_remote, git_branch, git_commit,
                  active_machine, active_agent, claimed_at
        "#,
    )
    .bind(req.name.as_deref())
    .bind(req.git_remote.as_deref())
    .bind(req.git_branch.as_deref())
    .bind(req.git_commit.as_deref())
    .bind(now)
    .bind(id)
    .bind(claims.sub)
    .fetch_optional(&pool)
    .await?;

    match row {
        Some(ref r) => Ok(Json(row_to_project_info(r)?)),
        None => Err(ProjectApiError::NotFound(id)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_project_request_serde_and_builders() {
        let req = CreateProjectRequest::new("ctx", "git@github.com:user/ctx.git")
            .with_git_revision("feature/branch", "abc1234");

        assert_eq!(req.name, "ctx");
        assert_eq!(req.git_remote, "git@github.com:user/ctx.git");
        assert_eq!(req.git_branch.as_deref(), Some("feature/branch"));
        assert_eq!(req.git_commit.as_deref(), Some("abc1234"));

        let json = serde_json::to_string(&req).expect("Serialize CreateProjectRequest");
        let deser: CreateProjectRequest =
            serde_json::from_str(&json).expect("Deserialize CreateProjectRequest");
        assert_eq!(req, deser);
    }

    #[test]
    fn test_update_project_request_serde() {
        let mut req = UpdateProjectRequest::new();
        req.name = Some("updated-ctx".to_string());
        req.git_branch = Some("main".to_string());

        let json = serde_json::to_string(&req).expect("Serialize UpdateProjectRequest");
        let deser: UpdateProjectRequest =
            serde_json::from_str(&json).expect("Deserialize UpdateProjectRequest");
        assert_eq!(req, deser);
    }

    #[test]
    fn test_project_api_error_status_codes() {
        let proj_id = Uuid::new_v4();
        let not_found = ProjectApiError::NotFound(proj_id);
        assert_eq!(StatusCode::from(&not_found), StatusCode::NOT_FOUND);

        let bad_request = ProjectApiError::BadRequest("empty name".to_string());
        assert_eq!(StatusCode::from(&bad_request), StatusCode::BAD_REQUEST);

        let unauth = ProjectApiError::Unauthorized;
        assert_eq!(StatusCode::from(&unauth), StatusCode::UNAUTHORIZED);

        let resp = not_found.into_response();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
