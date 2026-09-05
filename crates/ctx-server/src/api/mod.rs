//! HTTP API modules and router construction for `ctx-server`.
//!
//! Re-exports authentication, synchronization, session locking, project
//! management, and machine registration handlers. Provides the top-level
//! [`create_router`] function to configure public and JWT-protected routes.

pub mod auth;
pub mod machines;
pub mod projects;
pub mod session;
pub mod sync;

pub use auth::{
    auth_middleware, create_jwt, create_jwt_with_expiry, create_refresh_jwt, get_jwt_secret,
    hash_password, login, register, verify_jwt, verify_jwt_with_validation, verify_password,
    AuthError, AuthResponse, Claims, LoginRequest, RegisterRequest, ACCESS_TOKEN_EXPIRATION_SECS,
    DEFAULT_JWT_SECRET, REFRESH_TOKEN_EXPIRATION_SECS,
};
pub use machines::{
    get_machine, list_machines, register_machine, MachineApiError, RegisterMachineRequest,
};
pub use projects::{
    create_project, get_project, list_projects, update_project, CreateProjectRequest,
    ProjectApiError, UpdateProjectRequest,
};
pub use session::{
    claim_session, heartbeat, release_session, ClaimSessionRequest, HeartbeatRequest,
    ReleaseSessionRequest, ReleaseSessionResponse, SessionApiError, SESSION_LOCK_TIMEOUT_SECS,
};
pub use sync::{
    list_snapshots, pull_latest, push_snapshot, PushSnapshotResponse, SyncApiError,
};

use axum::{
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use sqlx::PgPool;

/// Health check handler returning basic server status and version information.
pub async fn health_check() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "service": "ctx-server",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// Builds the unstateful [`Router<PgPool>`] containing all API routes and JWT middleware.
pub fn create_routes() -> Router<PgPool> {
    let protected_routes = Router::new()
        // Sync routes
        .route("/api/sync/push", post(sync::push_snapshot))
        .route("/api/sync/latest/{project_id}", get(sync::pull_latest))
        .route("/api/sync/{project_id}/history", get(sync::list_snapshots))
        // Audit log
        .route("/api/audit/{project_id}", get(sync::get_audit_log))
        // Session lock routes
        .route("/api/session/claim", post(session::claim_session))
        .route("/api/session/release", post(session::release_session))
        .route("/api/session/heartbeat", post(session::heartbeat))
        // Project routes
        .route(
            "/api/projects",
            post(projects::create_project).get(projects::list_projects),
        )
        .route(
            "/api/projects/{id}",
            get(projects::get_project)
                .put(projects::update_project)
                .patch(projects::update_project),
        )
        // Machine routes
        .route(
            "/api/machines",
            post(machines::register_machine).get(machines::list_machines),
        )
        .route("/api/machines/register", post(machines::register_machine))
        .route("/api/machines/{id}", get(machines::get_machine))
        .route_layer(axum::middleware::from_fn(auth::auth_middleware));

    let public_routes = Router::new()
        .route("/health", get(health_check))
        .route("/api/v1/health", get(health_check))
        .route("/api/auth/register", post(auth::register))
        .route("/api/auth/login", post(auth::login));

    Router::new().merge(public_routes).merge(protected_routes)
}

/// Builds the complete Axum router containing all API routes for `ctx-server` with the provided PgPool.
///
/// Routes under `/api/sync/*`, `/api/session/*`, `/api/projects/*`, and `/api/machines/*`
/// are protected by JWT Bearer authentication middleware.
pub fn create_router(pool: PgPool) -> Router {
    create_routes().with_state(pool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_health_check_handler() {
        let response = health_check().await;
        assert_eq!(response.0["status"], "ok");
        assert_eq!(response.0["service"], "ctx-server");
    }

    #[tokio::test]
    async fn test_create_routes_and_create_router() {
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/ctx")
            .expect("Lazy pool creation must succeed with valid connection string");

        let unstateful_routes = create_routes();
        let _router: Router = unstateful_routes.with_state(pool.clone());
        let _complete_router: Router = create_router(pool);
    }
}
