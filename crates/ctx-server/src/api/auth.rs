//! Authentication API handlers, JWT token management, and password hashing for `ctx-server`.
//!
//! This module provides axum HTTP route handlers for user registration and login,
//! JSON Web Token (JWT) creation and verification using [`jsonwebtoken`], and
//! secure password hashing and verification using [`argon2`].

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    extract::{FromRequestParts, Request, State},
    http::{request::Parts, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use thiserror::Error;
use uuid::Uuid;

/// Default lifetime for access tokens in seconds (15 minutes).
pub const ACCESS_TOKEN_EXPIRATION_SECS: u64 = 15 * 60;

/// Default lifetime for refresh tokens in seconds (7 days).
pub const REFRESH_TOKEN_EXPIRATION_SECS: u64 = 7 * 24 * 60 * 60;

/// Default fallback secret for development environments if `JWT_SECRET` is not set.
pub const DEFAULT_JWT_SECRET: &str = "ctx-default-jwt-secret-change-in-production";

/// Errors that can occur during authentication and token operations.
#[derive(Debug, Error)]
pub enum AuthError {
    /// Error encoding or decoding a JSON Web Token.
    #[error("JWT error: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),

    /// Error hashing or verifying a password with Argon2.
    #[error("Password error: {0}")]
    Password(String),

    /// Database query or connection error.
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    /// Attempted to register a user that already exists.
    #[error("User already exists")]
    UserAlreadyExists,

    /// Provided email or password does not match any registered account.
    #[error("Invalid credentials")]
    InvalidCredentials,

    /// Token is invalid or expired.
    #[error("Invalid token: {0}")]
    InvalidToken(String),

    /// An internal server error occurred.
    #[error("Internal server error: {0}")]
    Internal(String),
}

impl From<AuthError> for StatusCode {
    fn from(err: AuthError) -> Self {
        match err {
            AuthError::UserAlreadyExists => StatusCode::CONFLICT,
            AuthError::InvalidCredentials => StatusCode::UNAUTHORIZED,
            AuthError::InvalidToken(_) => StatusCode::UNAUTHORIZED,
            AuthError::Jwt(_) => StatusCode::UNAUTHORIZED,
            AuthError::Password(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AuthError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AuthError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> axum::response::Response {
        let status: StatusCode = StatusCode::from(self);
        status.into_response()
    }
}

/// A specialized [`Result`] type for authentication operations.
pub type Result<T, E = AuthError> = std::result::Result<T, E>;

/// Payload submitted to the registration endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterRequest {
    /// User email address.
    pub email: String,
    /// Plaintext password to be securely hashed.
    pub password: String,
}

impl RegisterRequest {
    /// Creates a new [`RegisterRequest`] with the given email and password.
    pub fn new(email: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            email: email.into(),
            password: password.into(),
        }
    }
}

/// Payload submitted to the login endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginRequest {
    /// User email address.
    pub email: String,
    /// Plaintext password for authentication.
    pub password: String,
}

impl LoginRequest {
    /// Creates a new [`LoginRequest`] with the given email and password.
    pub fn new(email: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            email: email.into(),
            password: password.into(),
        }
    }
}

/// Response returned upon successful registration or login.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthResponse {
    /// Signed JWT access token with 15-minute validity.
    pub access_token: String,
    /// Signed JWT refresh token with 7-day validity.
    pub refresh_token: String,
    /// Access token validity duration in seconds (900 seconds).
    pub expires_in: u64,
}

impl AuthResponse {
    /// Creates a new [`AuthResponse`] with the provided access and refresh tokens.
    pub fn new(access_token: impl Into<String>, refresh_token: impl Into<String>, expires_in: u64) -> Self {
        Self {
            access_token: access_token.into(),
            refresh_token: refresh_token.into(),
            expires_in,
        }
    }
}

/// JWT claims payload containing subject identity and lifecycle timestamps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    /// Subject identifier, corresponding to the user's UUID.
    pub sub: Uuid,
    /// Expiration timestamp in seconds since Unix epoch.
    pub exp: usize,
    /// Issued-at timestamp in seconds since Unix epoch.
    pub iat: usize,
}

impl Claims {
    /// Creates a new [`Claims`] struct for the given user ID with expiration based on `duration_secs`.
    pub fn new(user_id: Uuid, duration_secs: u64) -> Self {
        let now = Utc::now().timestamp();
        let iat = if now > 0 { now as usize } else { 0 };
        let exp = iat.saturating_add(duration_secs as usize);
        Self {
            sub: user_id,
            exp,
            iat,
        }
    }

    /// Creates a [`Claims`] struct with explicit expiration and issued-at timestamps.
    pub fn with_timestamps(user_id: Uuid, exp: usize, iat: usize) -> Self {
        Self {
            sub: user_id,
            exp,
            iat,
        }
    }
}

impl<S> FromRequestParts<S> for Claims
where
    S: Send + Sync,
{
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // If Claims was already placed into request extensions by auth_middleware, return it
        if let Some(claims) = parts.extensions.get::<Claims>() {
            return Ok(claims.clone());
        }

        // Fallback to reading Authorization header directly
        let auth_header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or(StatusCode::UNAUTHORIZED)?;

        let token = auth_header
            .strip_prefix("Bearer ")
            .or_else(|| auth_header.strip_prefix("bearer "))
            .ok_or(StatusCode::UNAUTHORIZED)?
            .trim();

        let secret = get_jwt_secret();
        verify_jwt(token, &secret).map_err(|_| StatusCode::UNAUTHORIZED)
    }
}

/// Axum middleware that validates Bearer JWT tokens on incoming requests.
///
/// If valid, the extracted [`Claims`] are inserted into the request extensions.
/// If invalid or absent, responds immediately with [`StatusCode::UNAUTHORIZED`].
pub async fn auth_middleware(
    mut request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let auth_header = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .or_else(|| auth_header.strip_prefix("bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?
        .trim();

    let secret = get_jwt_secret();
    let claims = verify_jwt(token, &secret).map_err(|_| StatusCode::UNAUTHORIZED)?;

    request.extensions_mut().insert(claims);
    Ok(next.run(request).await)
}

/// Hashes a plaintext password using Argon2id with a cryptographically secure random salt.
pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| AuthError::Password(e.to_string()))
}

/// Verifies a plaintext password against an Argon2 password hash.
pub fn verify_password(password: &str, password_hash: &str) -> Result<bool> {
    let parsed_hash = PasswordHash::new(password_hash)
        .map_err(|e| AuthError::Password(e.to_string()))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}

/// Resolves the JWT secret from the `JWT_SECRET` environment variable, falling back to a default.
pub fn get_jwt_secret() -> String {
    std::env::var("JWT_SECRET").unwrap_or_else(|_| DEFAULT_JWT_SECRET.to_string())
}

/// Creates a signed JWT with a custom validity duration in seconds.
pub fn create_jwt_with_expiry(user_id: Uuid, secret: &str, duration_secs: u64) -> Result<String> {
    let claims = Claims::new(user_id, duration_secs);
    let header = Header::default();
    let encoding_key = EncodingKey::from_secret(secret.as_bytes());
    encode(&header, &claims, &encoding_key).map_err(AuthError::from)
}

/// Creates a signed 15-minute access JWT for the given user ID.
pub fn create_jwt(user_id: Uuid, secret: &str) -> Result<String> {
    create_jwt_with_expiry(user_id, secret, ACCESS_TOKEN_EXPIRATION_SECS)
}

/// Creates a signed 7-day refresh JWT for the given user ID.
pub fn create_refresh_jwt(user_id: Uuid, secret: &str) -> Result<String> {
    create_jwt_with_expiry(user_id, secret, REFRESH_TOKEN_EXPIRATION_SECS)
}

/// Verifies a signed JWT using the provided secret and returns the contained [`Claims`].
pub fn verify_jwt(token: &str, secret: &str) -> Result<Claims> {
    verify_jwt_with_validation(token, secret, &Validation::default())
}

/// Verifies a signed JWT using the provided secret and custom [`Validation`] configuration.
pub fn verify_jwt_with_validation(token: &str, secret: &str, validation: &Validation) -> Result<Claims> {
    let decoding_key = DecodingKey::from_secret(secret.as_bytes());
    let token_data = decode::<Claims>(token, &decoding_key, validation)?;
    Ok(token_data.claims)
}

/// Registers a new user account, securely hashing the password with Argon2.
///
/// Inserts the new user into the PostgreSQL database and returns an [`AuthResponse`]
/// containing 15-minute access and 7-day refresh JWTs on success.
/// Returns [`StatusCode::CONFLICT`] if the email is already registered,
/// [`StatusCode::BAD_REQUEST`] on invalid inputs, or [`StatusCode::INTERNAL_SERVER_ERROR`] on failures.
pub async fn register(
    State(pool): State<PgPool>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<AuthResponse>, StatusCode> {
    let email = req.email.trim();
    if email.is_empty() || req.password.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let password_hash = hash_password(&req.password).map_err(|err| {
        tracing::error!("Failed to hash password during registration: {err}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let user_id = Uuid::new_v4();
    let now = Utc::now();

    let query_result = sqlx::query(
        "INSERT INTO users (id, email, password_hash, created_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(user_id)
    .bind(email)
    .bind(&password_hash)
    .bind(now)
    .execute(&pool)
    .await;

    match query_result {
        Ok(_) => {
            let secret = get_jwt_secret();
            let access_token = create_jwt(user_id, &secret).map_err(|err| {
                tracing::error!("Failed to generate access token during registration: {err}");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
            let refresh_token = create_refresh_jwt(user_id, &secret).map_err(|err| {
                tracing::error!("Failed to generate refresh token during registration: {err}");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

            Ok(Json(AuthResponse {
                access_token,
                refresh_token,
                expires_in: ACCESS_TOKEN_EXPIRATION_SECS,
            }))
        }
        Err(err) => {
            if err.as_database_error().is_some_and(|db_err| db_err.is_unique_violation()) {
                tracing::warn!("Registration attempt with existing email: {email}");
                return Err(StatusCode::CONFLICT);
            }
            tracing::error!("Database error during user registration: {err}");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Authenticates an existing user account by verifying the password against its stored Argon2 hash.
///
/// Returns an [`AuthResponse`] containing 15-minute access and 7-day refresh JWTs on success.
/// Returns [`StatusCode::UNAUTHORIZED`] if credentials are invalid,
/// [`StatusCode::BAD_REQUEST`] on empty inputs, or [`StatusCode::INTERNAL_SERVER_ERROR`] on database failure.
pub async fn login(
    State(pool): State<PgPool>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, StatusCode> {
    let email = req.email.trim();
    if email.is_empty() || req.password.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let row = sqlx::query("SELECT id, password_hash FROM users WHERE email = $1")
        .bind(email)
        .fetch_optional(&pool)
        .await
        .map_err(|err| {
            tracing::error!("Database query error during login: {err}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let row = match row {
        Some(r) => r,
        None => return Err(StatusCode::UNAUTHORIZED),
    };

    let user_id: Uuid = row.try_get("id").map_err(|err| {
        tracing::error!("Failed to parse user id from database row: {err}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let password_hash: String = row.try_get("password_hash").map_err(|err| {
        tracing::error!("Failed to parse password_hash from database row: {err}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let is_valid = verify_password(&req.password, &password_hash).map_err(|err| {
        tracing::error!("Argon2 password verification failed: {err}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    if !is_valid {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let secret = get_jwt_secret();
    let access_token = create_jwt(user_id, &secret).map_err(|err| {
        tracing::error!("Failed to generate access token on login: {err}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let refresh_token = create_refresh_jwt(user_id, &secret).map_err(|err| {
        tracing::error!("Failed to generate refresh token on login: {err}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(AuthResponse {
        access_token,
        refresh_token,
        expires_in: ACCESS_TOKEN_EXPIRATION_SECS,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_verify_access_jwt() {
        let user_id = Uuid::new_v4();
        let secret = "test-secret-key-12345678901234567890";

        let token = create_jwt(user_id, secret).expect("Failed to create access JWT");
        let claims = verify_jwt(&token, secret).expect("Failed to verify access JWT");

        assert_eq!(claims.sub, user_id);
        assert!(claims.exp > claims.iat);
        assert_eq!(claims.exp - claims.iat, ACCESS_TOKEN_EXPIRATION_SECS as usize);
    }

    #[test]
    fn test_create_and_verify_refresh_jwt() {
        let user_id = Uuid::new_v4();
        let secret = "test-refresh-secret-1234567890123456";

        let token = create_refresh_jwt(user_id, secret).expect("Failed to create refresh JWT");
        let claims = verify_jwt(&token, secret).expect("Failed to verify refresh JWT");

        assert_eq!(claims.sub, user_id);
        assert!(claims.exp > claims.iat);
        assert_eq!(claims.exp - claims.iat, REFRESH_TOKEN_EXPIRATION_SECS as usize);
    }

    #[test]
    fn test_verify_jwt_with_invalid_secret() {
        let user_id = Uuid::new_v4();
        let token = create_jwt(user_id, "correct-secret-1234567890").expect("Failed to create JWT");

        let err = verify_jwt(&token, "wrong-secret-0987654321").expect_err("Verification must fail with wrong secret");
        assert!(matches!(err, AuthError::Jwt(_)));
    }

    #[test]
    fn test_verify_jwt_tampered_token() {
        let user_id = Uuid::new_v4();
        let secret = "valid-secret-1234567890";
        let mut token = create_jwt(user_id, secret).expect("Failed to create JWT");

        // Tamper with the signature portion
        token.push('x');

        let err = verify_jwt(&token, secret).expect_err("Verification must fail for tampered token");
        assert!(matches!(err, AuthError::Jwt(_)));
    }

    #[test]
    fn test_verify_jwt_expired() {
        let user_id = Uuid::new_v4();
        let secret = "test-secret-1234567890";

        // Create expired claims: exp in 1970
        let claims = Claims::with_timestamps(user_id, 100, 50);
        let header = Header::default();
        let encoding_key = EncodingKey::from_secret(secret.as_bytes());
        let token = encode(&header, &claims, &encoding_key).expect("Failed to encode expired token");

        let err = verify_jwt(&token, secret).expect_err("Expired token must fail verification");
        assert!(matches!(err, AuthError::Jwt(_)));
    }

    #[test]
    fn test_argon2_password_hashing_and_verification() {
        let password = "correct_horse_battery_staple_42";
        let hash = hash_password(password).expect("Hashing password should succeed");

        assert_ne!(hash, password);
        assert!(hash.starts_with("$argon2id$"));

        // Verify with correct password
        let is_valid = verify_password(password, &hash).expect("Verification should succeed");
        assert!(is_valid);

        // Verify with wrong password
        let is_wrong_valid = verify_password("incorrect_password", &hash).expect("Verification should execute");
        assert!(!is_wrong_valid);
    }

    #[test]
    fn test_request_and_response_serialization() {
        let reg_req = RegisterRequest::new("dev@example.com", "my_secure_pass");
        let json_reg = serde_json::to_string(&reg_req).expect("Failed to serialize RegisterRequest");
        let deser_reg: RegisterRequest = serde_json::from_str(&json_reg).expect("Failed to deserialize RegisterRequest");
        assert_eq!(reg_req, deser_reg);

        let login_req = LoginRequest::new("dev@example.com", "my_secure_pass");
        let json_login = serde_json::to_string(&login_req).expect("Failed to serialize LoginRequest");
        let deser_login: LoginRequest = serde_json::from_str(&json_login).expect("Failed to deserialize LoginRequest");
        assert_eq!(login_req, deser_login);

        let auth_resp = AuthResponse::new("access_token_123", "refresh_token_456", ACCESS_TOKEN_EXPIRATION_SECS);
        let json_resp = serde_json::to_string(&auth_resp).expect("Failed to serialize AuthResponse");
        let deser_resp: AuthResponse = serde_json::from_str(&json_resp).expect("Failed to deserialize AuthResponse");
        assert_eq!(auth_resp, deser_resp);
    }

    #[test]
    fn test_claims_new_and_timestamps() {
        let user_id = Uuid::new_v4();
        let claims = Claims::new(user_id, 300);

        assert_eq!(claims.sub, user_id);
        assert_eq!(claims.exp - claims.iat, 300);

        let explicit = Claims::with_timestamps(user_id, 2000, 1000);
        assert_eq!(explicit.sub, user_id);
        assert_eq!(explicit.exp, 2000);
        assert_eq!(explicit.iat, 1000);
    }

    #[test]
    fn test_auth_error_status_code_mapping() {
        assert_eq!(StatusCode::from(AuthError::UserAlreadyExists), StatusCode::CONFLICT);
        assert_eq!(StatusCode::from(AuthError::InvalidCredentials), StatusCode::UNAUTHORIZED);
        assert_eq!(StatusCode::from(AuthError::Password("fail".into())), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(StatusCode::from(AuthError::Internal("error".into())), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(StatusCode::from(AuthError::InvalidToken("bad".into())), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_claims_from_request_parts_with_auth_header() {
        let user_id = Uuid::new_v4();
        let secret = get_jwt_secret();
        let token = create_jwt(user_id, &secret).expect("token creation");

        let req = axum::http::Request::builder()
            .header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
            .body(())
            .expect("request creation");
        let (mut parts, _) = req.into_parts();

        let claims = Claims::from_request_parts(&mut parts, &())
            .await
            .expect("claims extraction");
        assert_eq!(claims.sub, user_id);
    }

    #[tokio::test]
    async fn test_claims_from_request_parts_missing_header() {
        let req = axum::http::Request::builder()
            .body(())
            .expect("request creation");
        let (mut parts, _) = req.into_parts();

        let err = Claims::from_request_parts(&mut parts, &())
            .await
            .expect_err("should fail without header");
        assert_eq!(err, StatusCode::UNAUTHORIZED);
    }
}
