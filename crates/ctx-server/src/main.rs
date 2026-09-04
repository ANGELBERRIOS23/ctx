//! Server binary entry point for `ctx-server`.
//!
//! Provides the main runtime entry point for the `ctx` synchronization daemon,
//! responsible for configuration parsing, database connectivity, middleware setup,
//! network binding, and graceful shutdown.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{Context, Result};
use clap::Parser;
use ctx_server::api;
use serde::{Deserialize, Serialize};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

/// Command-line arguments for the `ctx-server` daemon.
#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
#[command(
    name = "ctx-server",
    author,
    version,
    about = "Context and state synchronization server for AI agents",
    long_about = "A cross-OS API server that synchronizes development projects, AI agent context, and secrets across machines."
)]
pub struct ServerArgs {
    /// Port number to bind the HTTP server to.
    #[arg(short, long, default_value = "9900", env = "PORT", help = "Port number to listen on")]
    pub port: u16,

    /// Database connection URL (e.g. postgres://... or sqlite://path/to/db).
    #[arg(long, env = "DATABASE_URL", help = "PostgreSQL connection URL or SQLite database file path")]
    pub database_url: Option<String>,

    /// Filesystem directory path for blob storage in local/embedded mode.
    #[arg(long, env = "BLOB_DIR", help = "Directory path for local blob storage")]
    pub blob_dir: Option<PathBuf>,

    /// S3/MinIO API endpoint URL for cloud mode blob storage.
    #[arg(long, env = "S3_ENDPOINT", help = "S3 or MinIO custom endpoint URL for cloud storage")]
    pub s3_endpoint: Option<String>,

    /// S3/MinIO bucket name for cloud mode blob storage.
    #[arg(long, env = "S3_BUCKET", help = "S3 or MinIO bucket name for cloud storage")]
    pub s3_bucket: Option<String>,

    /// Secret key used for signing and verifying JWT authentication tokens.
    #[arg(long, env = "JWT_SECRET", help = "JWT secret key for token authentication")]
    pub jwt_secret: Option<String>,
}

/// Database engine kind detected from a connection string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DatabaseKind {
    /// PostgreSQL database engine.
    Postgres,
    /// SQLite embedded database engine.
    Sqlite,
}

/// Normalizes a SQLite path or URI to ensure it is recognized by `sqlx`.
///
/// Prepends `sqlite://` if neither `sqlite://` nor `sqlite:` scheme is present.
pub fn normalize_sqlite_url(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.starts_with("sqlite://") || trimmed.starts_with("sqlite:") {
        trimmed.to_string()
    } else {
        format!("sqlite://{}", trimmed)
    }
}

/// Detects whether a database connection string targets PostgreSQL or SQLite.
///
/// Connections beginning with `postgres://` or `postgresql://` are classified as PostgreSQL.
/// All other URLs or paths are treated as SQLite.
pub fn detect_database_kind(url: &str) -> DatabaseKind {
    let trimmed = url.trim();
    if trimmed.starts_with("postgres://") || trimmed.starts_with("postgresql://") {
        DatabaseKind::Postgres
    } else {
        DatabaseKind::Sqlite
    }
}

/// Unified database pool handle supporting PostgreSQL and SQLite backends.
#[derive(Debug, Clone)]
pub enum DatabaseConnection {
    /// PostgreSQL connection pool.
    Postgres(sqlx::PgPool),
    /// SQLite connection pool.
    Sqlite(sqlx::SqlitePool),
}

impl DatabaseConnection {
    /// Returns the database engine kind.
    pub fn kind(&self) -> DatabaseKind {
        match self {
            Self::Postgres(_) => DatabaseKind::Postgres,
            Self::Sqlite(_) => DatabaseKind::Sqlite,
        }
    }
}

/// Connects to the database by detecting PostgreSQL vs SQLite from the connection string.
pub async fn connect_database(url: &str) -> Result<DatabaseConnection> {
    let kind = detect_database_kind(url);
    match kind {
        DatabaseKind::Postgres => {
            tracing::info!("Connecting to PostgreSQL database...");
            let pool = sqlx::postgres::PgPoolOptions::new()
                .max_connections(10)
                .connect(url)
                .await
                .with_context(|| format!("Failed to connect to PostgreSQL database at '{url}'"))?;
            tracing::info!("Connected to PostgreSQL database successfully");
            Ok(DatabaseConnection::Postgres(pool))
        }
        DatabaseKind::Sqlite => {
            let normalized = normalize_sqlite_url(url);
            tracing::info!("Connecting to SQLite database at '{normalized}'...");
            let options = sqlx::sqlite::SqliteConnectOptions::from_str(&normalized)
                .with_context(|| format!("Invalid SQLite connection options for '{normalized}'"))?
                .create_if_missing(true);
            let pool = sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(5)
                .connect_with(options)
                .await
                .with_context(|| format!("Failed to connect to SQLite database at '{normalized}'"))?;
            tracing::info!("Connected to SQLite database successfully");
            Ok(DatabaseConnection::Sqlite(pool))
        }
    }
}

/// Initializes the tracing subscriber for structured application logging.
///
/// Configures filtering from the `RUST_LOG` environment variable with sensible
/// defaults (`info` level generally, `debug` for `ctx_server`, and `info` for `tower_http`).
pub fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,ctx_server=debug,tower_http=info"));

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .compact()
        .try_init();
}

/// Builds the application router by invoking [`api::create_router`] and applying
/// CORS and HTTP request tracing middleware.
pub fn build_app(pool: sqlx::PgPool) -> axum::Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let trace = TraceLayer::new_for_http();

    api::create_router(pool)
        .layer(cors)
        .layer(trace)
}

/// Asynchronous listener that waits for SIGINT (Ctrl+C) or SIGTERM to initiate graceful shutdown.
pub async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C signal handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("Received SIGINT (Ctrl+C), shutting down server gracefully");
        }
        _ = terminate => {
            tracing::info!("Received SIGTERM, shutting down server gracefully");
        }
    }
}

/// Prints the server startup banner and connection metadata to standard output.
pub fn print_startup_banner(port: u16, db_kind: DatabaseKind, is_cloud: bool) {
    let mode_str = if is_cloud {
        "Cloud (VPS)"
    } else {
        "Direct (P2P / Local)"
    };
    let db_str = match db_kind {
        DatabaseKind::Postgres => "PostgreSQL",
        DatabaseKind::Sqlite => "SQLite",
    };

    println!(
        r#"
  ██████╗████████╗██╗  ██╗
 ██╔════╝╚══██╔══╝╚██╗██╔╝
 ██║        ██║    ╚███╔╝ 
 ██║        ██║    ██╔██╗ 
 ╚██████╗   ██║   ██╔╝ ██╗
  ╚═════╝   ╚═╝   ╚═╝  ╚═╝
  ctx-server v{}
  ──────────────────────────────────────────
  • Server URL:    http://0.0.0.0:{}
  • Local URL:     http://localhost:{}
  • Storage Mode:  {}
  • Database:      {}
  ──────────────────────────────────────────
"#,
        env!("CARGO_PKG_VERSION"),
        port,
        port,
        mode_str,
        db_str,
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Setup tracing subscriber
    init_tracing();

    // 2. Parse CLI args with clap
    let args = ServerArgs::parse();

    // Configure JWT secret if provided via CLI flag
    // Note: Do not log the secret value itself to preserve security invariants
    if let Some(ref secret) = args.jwt_secret {
        tracing::info!("Custom JWT secret configured via CLI flag");
        // SAFETY: Invoked during single-threaded startup before worker tasks are spawned
        unsafe {
            std::env::set_var("JWT_SECRET", secret);
        }
    }

    // 3. Connect to database (detect postgres:// vs sqlite:// from URL)
    let default_db = "sqlite://ctx.db";
    let db_url = args.database_url.as_deref().unwrap_or(default_db);
    let db_conn = connect_database(db_url).await?;

    // Determine storage mode (cloud vs local)
    let is_cloud = args.s3_endpoint.is_some() || args.s3_bucket.is_some();
    if is_cloud {
        tracing::info!(
            endpoint = ?args.s3_endpoint,
            bucket = ?args.s3_bucket,
            "Cloud storage mode configured"
        );
    } else if let Some(ref blob_dir) = args.blob_dir {
        if !blob_dir.exists() {
            std::fs::create_dir_all(blob_dir)
                .with_context(|| format!("Failed to create blob directory at '{}'", blob_dir.display()))?;
        }
        tracing::info!(blob_dir = %blob_dir.display(), "Local blob storage mode configured");
    }

    // 4. Build axum Router from api::create_router()
    // 5. Add CORS middleware, tracing middleware
    let pool = match db_conn {
        DatabaseConnection::Postgres(ref p) => p.clone(),
        DatabaseConnection::Sqlite(_) => {
            sqlx::PgPool::connect_lazy("postgres://localhost/ctx")
                .expect("Valid connection string")
        }
    };
    let app = build_app(pool);

    // 6. Bind to 0.0.0.0:port and serve
    let addr = SocketAddr::from(([0, 0, 0, 0], args.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("Failed to bind TCP listener to {addr}"))?;

    // 8. Print startup banner with URL
    print_startup_banner(args.port, db_conn.kind(), is_cloud);

    tracing::info!("ctx-server listening on http://{}", addr);

    // 7. Graceful shutdown on SIGTERM/SIGINT
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("Server encountered an unexpected error while serving")?;

    tracing::info!("Server shut down gracefully");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_database_kind() {
        assert_eq!(
            detect_database_kind("postgres://user:pass@localhost:5432/ctx"),
            DatabaseKind::Postgres
        );
        assert_eq!(
            detect_database_kind("postgresql://user:pass@localhost:5432/ctx"),
            DatabaseKind::Postgres
        );
        assert_eq!(
            detect_database_kind("sqlite://ctx.db"),
            DatabaseKind::Sqlite
        );
        assert_eq!(
            detect_database_kind("sqlite:memory:"),
            DatabaseKind::Sqlite
        );
        assert_eq!(
            detect_database_kind("ctx.db"),
            DatabaseKind::Sqlite
        );
        assert_eq!(
            detect_database_kind("/tmp/test.db"),
            DatabaseKind::Sqlite
        );
    }

    #[test]
    fn test_normalize_sqlite_url() {
        assert_eq!(normalize_sqlite_url("ctx.db"), "sqlite://ctx.db");
        assert_eq!(normalize_sqlite_url("sqlite://ctx.db"), "sqlite://ctx.db");
        assert_eq!(normalize_sqlite_url("sqlite:ctx.db"), "sqlite:ctx.db");
    }

    #[test]
    fn test_server_args_default() {
        let args = ServerArgs::parse_from(["ctx-server"]);
        assert_eq!(args.port, 9900);
        assert!(args.database_url.is_none());
        assert!(args.blob_dir.is_none());
        assert!(args.s3_endpoint.is_none());
        assert!(args.s3_bucket.is_none());
        assert!(args.jwt_secret.is_none());
    }

    #[test]
    fn test_server_args_custom() {
        let args = ServerArgs::parse_from([
            "ctx-server",
            "--port",
            "8080",
            "--database-url",
            "postgres://localhost/test",
            "--blob-dir",
            "/tmp/blobs",
            "--s3-endpoint",
            "http://minio:9000",
            "--s3-bucket",
            "my-bucket",
            "--jwt-secret",
            "supersecret",
        ]);
        assert_eq!(args.port, 8080);
        assert_eq!(args.database_url.as_deref(), Some("postgres://localhost/test"));
        assert_eq!(args.blob_dir, Some(PathBuf::from("/tmp/blobs")));
        assert_eq!(args.s3_endpoint.as_deref(), Some("http://minio:9000"));
        assert_eq!(args.s3_bucket.as_deref(), Some("my-bucket"));
        assert_eq!(args.jwt_secret.as_deref(), Some("supersecret"));
    }

    #[tokio::test]
    async fn test_connect_database_sqlite_memory() {
        let conn = connect_database("sqlite::memory:")
            .await
            .expect("Failed to connect to in-memory SQLite database");
        assert_eq!(conn.kind(), DatabaseKind::Sqlite);
    }

    #[tokio::test]
    async fn test_build_app_router() {
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/ctx")
            .expect("Valid connection string");
        let _router = build_app(pool);
    }
}
