//! Authentication commands (`login` and `logout`) for the `ctx` CLI.
//!
//! Provides interactive authentication prompts, server communication via HTTP,
//! secure credential and JWT storage inside the operating system keychain via
//! the [`keyring`] crate, and session invalidation.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use console::style;
use ctx_core::config::ProjectConfig;
use serde::{Deserialize, Serialize};

/// Service name identifier used for OS keychain storage.
pub const KEYRING_SERVICE: &str = "ctx";

/// Key name used for storing the JWT access token in the OS keychain.
pub const KEYRING_TOKEN_ACCOUNT: &str = "token";

/// Key name used for storing the authenticated user's email in the OS keychain.
pub const KEYRING_USER_ACCOUNT: &str = "email";

/// Default server URL used when no explicit server URL or config is provided.
pub const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:9900";

/// Request payload submitted to the server login endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginRequest {
    /// User email address.
    pub email: String,
    /// Plaintext user password.
    pub password: String,
}

impl LoginRequest {
    /// Creates a new [`LoginRequest`].
    pub fn new(email: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            email: email.into(),
            password: password.into(),
        }
    }
}

/// Authentication response returned by the server upon successful login.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthResponse {
    /// JWT access token.
    pub access_token: String,
    /// JWT refresh token.
    pub refresh_token: String,
    /// Token lifetime in seconds.
    pub expires_in: u64,
}

impl AuthResponse {
    /// Creates a new [`AuthResponse`].
    pub fn new(
        access_token: impl Into<String>,
        refresh_token: impl Into<String>,
        expires_in: u64,
    ) -> Self {
        Self {
            access_token: access_token.into(),
            refresh_token: refresh_token.into(),
            expires_in,
        }
    }
}

/// Normalizes a server URL by ensuring an `http://` or `https://` scheme and stripping trailing slashes.
pub fn normalize_server_url(url: &str) -> String {
    let trimmed = url.trim();
    let with_scheme = if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        format!("http://{}", trimmed)
    } else {
        trimmed.to_string()
    };
    with_scheme.trim_end_matches('/').to_string()
}

/// Resolves the server URL from the provided option, environment variable, local project config, or default.
pub fn resolve_server_url(server_url: Option<String>) -> String {
    if let Some(url) = server_url {
        let trimmed = url.trim();
        if !trimmed.is_empty() {
            return normalize_server_url(trimmed);
        }
    }

    if let Ok(env_url) = std::env::var("CTX_SERVER_URL") {
        let trimmed = env_url.trim();
        if !trimmed.is_empty() {
            return normalize_server_url(trimmed);
        }
    }

    if let Ok(current_dir) = std::env::current_dir()
        && let Ok(project_config) = ProjectConfig::load(&current_dir) {
            let server = project_config.project.server.trim();
            if !server.is_empty() {
                return normalize_server_url(server);
            }
        }

    DEFAULT_SERVER_URL.to_string()
}

/// Stores authentication credentials (user email and JWT access token) securely in the OS keychain.
pub fn store_credentials(email: &str, token: &str) -> Result<()> {
    let token_entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_TOKEN_ACCOUNT)
        .context("Failed to open system keychain entry for JWT access token")?;
    token_entry
        .set_password(token)
        .context("Failed to store JWT access token in OS keychain")?;

    let user_entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER_ACCOUNT)
        .context("Failed to open system keychain entry for user email")?;
    user_entry
        .set_password(email)
        .context("Failed to store user email in OS keychain")?;

    Ok(())
}

/// Retrieves the stored JWT access token from the OS keychain.
pub fn get_stored_token() -> Result<String> {
    let token_entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_TOKEN_ACCOUNT)
        .context("Failed to open system keychain entry for JWT access token")?;
    token_entry
        .get_password()
        .context("No active authentication token found. Run 'ctx login' to authenticate.")
}

/// Retrieves the stored user email from the OS keychain.
pub fn get_stored_email() -> Result<String> {
    let user_entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER_ACCOUNT)
        .context("Failed to open system keychain entry for user email")?;
    user_entry
        .get_password()
        .context("No stored user email found in OS keychain.")
}

/// Removes stored authentication credentials from the OS keychain.
pub fn delete_stored_credentials() -> Result<()> {
    if let Ok(token_entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_TOKEN_ACCOUNT) {
        let _ = token_entry.delete_credential();
    }
    if let Ok(user_entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER_ACCOUNT) {
        let _ = user_entry.delete_credential();
    }
    Ok(())
}

/// Executes a non-interactive HTTP login request against the server and stores the returned JWT.
pub async fn login_with_credentials(server_url: &str, email: &str, password: &str) -> Result<AuthResponse> {
    let email_trimmed = email.trim();
    if email_trimmed.is_empty() {
        bail!("Email cannot be empty.");
    }
    if password.is_empty() {
        bail!("Password cannot be empty.");
    }

    let normalized_base = normalize_server_url(server_url);
    let endpoint = format!("{}/api/auth/login", normalized_base);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .context("Failed to build HTTP client")?;

    let payload = LoginRequest::new(email_trimmed, password);

    let response = client
        .post(&endpoint)
        .json(&payload)
        .send()
        .await
        .with_context(|| format!("Failed to connect to ctx server at {}", endpoint))?;

    let status = response.status();
    if status.is_success() {
        let auth_response: AuthResponse = response
            .json()
            .await
            .context("Failed to deserialize authentication response from server")?;

        store_credentials(email_trimmed, &auth_response.access_token)?;
        Ok(auth_response)
    } else if status == reqwest::StatusCode::UNAUTHORIZED {
        // Auto-register: ask user if they want to create an account
        println!(
            "{}",
            style("  Account not found. Creating new account...").yellow()
        );
        let register_endpoint = format!("{}/api/auth/register", normalized_base);
        let reg_response = client
            .post(&register_endpoint)
            .json(&payload)
            .send()
            .await
            .with_context(|| "Failed to register new account")?;

        if reg_response.status().is_success() {
            let auth_response: AuthResponse = reg_response
                .json()
                .await
                .context("Failed to deserialize registration response")?;
            store_credentials(email_trimmed, &auth_response.access_token)?;
            println!(
                "  {} New account created and authenticated.",
                style("✓").green().bold()
            );
            Ok(auth_response)
        } else {
            let err = reg_response.text().await.unwrap_or_default();
            bail!("Registration failed: {}", err.trim());
        }
    } else if status == reqwest::StatusCode::BAD_REQUEST {
        bail!("Authentication failed: invalid email format or empty credentials.");
    } else {
        let error_body = response.text().await.unwrap_or_default();
        bail!(
            "Server returned error status {}: {}",
            status,
            error_body.trim()
        );
    }
}

/// Executes the interactive login command.
///
/// Prompts the user for email and password via [`dialoguer`], submits them to
/// `/api/auth/login` on the target ctx server, stores the returned JWT in the
/// OS keychain via [`keyring`], and prints a success message.
pub async fn run(server_url: Option<String>) -> Result<()> {
    let resolved_server = resolve_server_url(server_url);

    println!(
        "{} Logging in to ctx server at {}",
        style("→").cyan().bold(),
        style(&resolved_server).bold()
    );

    let email: String = dialoguer::Input::new()
        .with_prompt("Email")
        .interact_text()
        .context("Failed to read email input")?;

    let password = dialoguer::Password::new()
        .with_prompt("Password")
        .interact()
        .context("Failed to read password input")?;

    login_with_credentials(&resolved_server, &email, &password).await?;

    println!(
        "{} Successfully authenticated as {}",
        style("✓").green().bold(),
        style(&email).cyan().bold()
    );
    println!("  Server:   {}", style(&resolved_server).dim());
    println!("  Keychain: JWT token saved securely in OS credential store.");

    Ok(())
}

/// Executes the logout command, removing credentials from the OS keychain.
pub async fn run_logout() -> Result<()> {
    let email_opt = get_stored_email().ok();
    delete_stored_credentials()?;

    if let Some(email) = email_opt {
        println!(
            "{} Successfully logged out ({}) and cleared OS keychain credentials.",
            style("✓").green().bold(),
            style(email).cyan()
        );
    } else {
        println!(
            "{} Successfully logged out and cleared OS keychain credentials.",
            style("✓").green().bold()
        );
    }

    Ok(())
}

/// Convenience function executing [`run`], accepting a [`ctx_core::config::GlobalConfig`] reference.
pub async fn login(_config: &ctx_core::config::GlobalConfig, server_url: Option<String>) -> Result<()> {
    run(server_url).await
}

/// Convenience function executing [`run_logout`], accepting a [`ctx_core::config::GlobalConfig`] reference.
pub async fn logout(_config: &ctx_core::config::GlobalConfig) -> Result<()> {
    run_logout().await
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_server_url() {
        assert_eq!(normalize_server_url("localhost:9900"), "http://localhost:9900");
        assert_eq!(
            normalize_server_url("http://127.0.0.1:9900/"),
            "http://127.0.0.1:9900"
        );
        assert_eq!(
            normalize_server_url("https://ctx.example.com/api///"),
            "https://ctx.example.com/api"
        );
        assert_eq!(
            normalize_server_url("   http://test.local:8080   "),
            "http://test.local:8080"
        );
    }

    #[test]
    fn test_resolve_server_url_explicit_and_default() {
        let explicit = Some("https://custom.server.io".to_string());
        assert_eq!(
            resolve_server_url(explicit),
            "https://custom.server.io"
        );

        let empty = Some("   ".to_string());
        // Empty string should fall back to env or default
        let resolved = resolve_server_url(empty);
        assert!(!resolved.is_empty());
    }

    #[test]
    fn test_login_and_auth_models_serde() {
        let req = LoginRequest::new("dev@example.com", "secretpass");
        let json_req = serde_json::to_string(&req).expect("Serialize LoginRequest");
        let deser_req: LoginRequest =
            serde_json::from_str(&json_req).expect("Deserialize LoginRequest");
        assert_eq!(req, deser_req);

        let resp = AuthResponse::new("jwt_access_123", "jwt_refresh_456", 900);
        let json_resp = serde_json::to_string(&resp).expect("Serialize AuthResponse");
        let deser_resp: AuthResponse =
            serde_json::from_str(&json_resp).expect("Deserialize AuthResponse");
        assert_eq!(resp, deser_resp);
    }

    #[tokio::test]
    async fn test_login_with_empty_credentials_rejected() {
        let result = login_with_credentials("http://localhost:9900", "", "password").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Email cannot be empty"));

        let result_pass = login_with_credentials("http://localhost:9900", "user@example.com", "").await;
        assert!(result_pass.is_err());
        assert!(result_pass.unwrap_err().to_string().contains("Password cannot be empty"));
    }
}
