//! Vault abstraction layer for secret management and resolution in `ctx-core`.
//!
//! This module provides a unified interface across diverse secret backends
//! (e.g. Bitwarden, 1Password, HashiCorp Vault, AWS Secrets Manager, SOPS, and manual entry)
//! allowing development projects and AI agents to resolve secret references securely at runtime
//! without committing sensitive credentials to source control or synchronizing plaintext tokens.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};

pub use crate::config::SecretRef;
use crate::error::{CtxError, Result};

/// Pinned, heap-allocated, thread-safe future alias for async trait dispatch.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Supported secret vault and password manager providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultProvider {
    /// Bitwarden cloud service (`vault.bitwarden.com`).
    Bitwarden,
    /// Self-hosted Bitwarden instance.
    BitwardenSelfHosted,
    /// 1Password CLI (`op`).
    #[serde(alias = "1password", alias = "onepassword")]
    OnePassword,
    /// HashiCorp Vault.
    #[serde(alias = "vault")]
    Hashicorp,
    /// AWS Secrets Manager.
    #[serde(alias = "aws", alias = "aws_secrets")]
    AwsSecretsManager,
    /// Mozilla SOPS (Secrets OPerationS).
    Sops,
    /// Manual secret resolution or local environment variable fallback.
    Manual,
}

impl VaultProvider {
    /// Returns the static string representation of the vault provider.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Bitwarden => "bitwarden",
            Self::BitwardenSelfHosted => "bitwarden_self_hosted",
            Self::OnePassword => "one_password",
            Self::Hashicorp => "hashicorp",
            Self::AwsSecretsManager => "aws_secrets_manager",
            Self::Sops => "sops",
            Self::Manual => "manual",
        }
    }
}

impl fmt::Display for VaultProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Configuration for secret vault integration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultConfig {
    /// The secret vault provider backend.
    pub provider: VaultProvider,
    /// Optional server or endpoint URL (primarily for self-hosted instances).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_url: Option<String>,
}

impl VaultConfig {
    /// Creates a new vault configuration with no custom server URL.
    pub fn new(provider: VaultProvider) -> Self {
        Self {
            provider,
            server_url: None,
        }
    }

    /// Creates a new vault configuration with a custom server URL.
    pub fn with_server_url(provider: VaultProvider, server_url: impl Into<String>) -> Self {
        Self {
            provider,
            server_url: Some(server_url.into()),
        }
    }
}

/// A resolved secret mapping a variable key identifier to its plaintext value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretResolution {
    /// The variable key name (e.g. `OPENAI_API_KEY`, `DATABASE_URL`).
    pub key_name: String,
    /// The resolved plaintext secret value.
    pub value: String,
}

impl SecretResolution {
    /// Creates a new resolved secret pair.
    pub fn new(key_name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key_name: key_name.into(),
            value: value.into(),
        }
    }
}

/// Trait defining secret vault resolution and availability checking.
pub trait VaultResolver: Send + Sync {
    /// Resolves a collection of secret references into their resolved plaintext values.
    fn resolve<'a>(
        &'a self,
        refs: &'a [SecretRef],
    ) -> BoxFuture<'a, Result<Vec<SecretResolution>>>;

    /// Checks whether the vault provider CLI or service is available and reachable.
    fn check_available<'a>(&'a self) -> BoxFuture<'a, Result<bool>>;
}

/// Extracts the item ID or search name from a vault URI.
///
/// Strips common URI schemes such as `vault://`, `bitwarden://`, or `bw://`.
/// If the resulting ID is empty, falls back to the provided `fallback` string.
pub fn extract_item_id<'a>(uri: &'a str, fallback: &'a str) -> &'a str {
    let trimmed = uri.trim();
    let stripped = if let Some(rest) = trimmed.strip_prefix("vault://") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("bitwarden://") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("bw://") {
        rest
    } else {
        trimmed
    };

    let stripped = stripped.trim_matches('/');
    if stripped.is_empty() {
        fallback.trim()
    } else {
        stripped
    }
}

/// Parses the JSON stdout output of `bw get item <id>` to extract the secret value.
///
/// Searches for secret values in the following order:
/// 1. Custom fields with a name matching `key_name` (case-insensitive).
/// 2. `login.password` if present and non-empty.
/// 3. Custom fields with common secret names (`"password"`, `"secret"`, `"token"`, `"api_key"`, `"key"`).
/// 4. `notes` field if present and non-empty (common for Secure Notes).
/// 5. Single field in `fields` if only one custom field is present.
/// 6. Top-level `"value"` field if present.
/// 7. Raw trimmed non-JSON text output.
pub fn parse_bitwarden_item_output(output: &str, key_name: &str) -> Option<String> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(obj) = json.as_object() {
            // 1. Check custom fields matching key_name
            if let Some(fields) = obj.get("fields").and_then(|f| f.as_array()) {
                let matching = fields.iter().find_map(|field| {
                    let name = field.get("name")?.as_str()?;
                    if name.eq_ignore_ascii_case(key_name) {
                        let val = field.get("value")?.as_str()?;
                        if !val.is_empty() {
                            return Some(val.to_string());
                        }
                    }
                    None
                });
                if let Some(val) = matching {
                    return Some(val);
                }
            }

            // 2. Check login.password
            let login_password = obj
                .get("login")
                .and_then(|l| l.get("password"))
                .and_then(|p| p.as_str())
                .filter(|p| !p.is_empty());
            if let Some(password) = login_password {
                return Some(password.to_string());
            }

            // 3. Check custom fields with common secret names
            if let Some(fields) = obj.get("fields").and_then(|f| f.as_array()) {
                let common_names = [
                    "password",
                    "secret",
                    "token",
                    "api_key",
                    "apikey",
                    "key",
                    "credential",
                ];
                let common_field = fields.iter().find_map(|field| {
                    let name = field.get("name")?.as_str()?.to_ascii_lowercase();
                    if common_names.contains(&name.as_str()) {
                        let val = field.get("value")?.as_str()?;
                        if !val.is_empty() {
                            return Some(val.to_string());
                        }
                    }
                    None
                });
                if let Some(val) = common_field {
                    return Some(val);
                }

                // 4. Single field in fields
                if fields.len() == 1 {
                    let single_val = fields[0]
                        .get("value")
                        .and_then(|v| v.as_str())
                        .filter(|v| !v.is_empty());
                    if let Some(val) = single_val {
                        return Some(val.to_string());
                    }
                }
            }

            // 5. Check notes (Secure Notes)
            let notes = obj
                .get("notes")
                .and_then(|n| n.as_str())
                .map(str::trim)
                .filter(|n| !n.is_empty());
            if let Some(n) = notes {
                return Some(n.to_string());
            }

            // 6. Top-level "value" field
            let top_value = obj
                .get("value")
                .and_then(|v| v.as_str())
                .filter(|v| !v.is_empty());
            if let Some(val) = top_value {
                return Some(val.to_string());
            }
        }
    } else if !trimmed.is_empty() && !trimmed.starts_with('{') {
        // Raw non-JSON text output
        return Some(trimmed.to_string());
    }

    None
}

/// Resolver for Bitwarden vaults using the `bw` command-line interface.
#[derive(Debug, Clone)]
pub struct BitwardenResolver {
    /// Path to the `bw` CLI binary (defaults to `"bw"`).
    cli_path: String,
    /// Optional self-hosted server URL.
    server_url: Option<String>,
    /// Optional Bitwarden session token (`BW_SESSION`).
    session_token: Option<String>,
}

impl BitwardenResolver {
    /// Creates a new Bitwarden resolver using the standard `bw` binary.
    pub fn new(server_url: Option<String>) -> Self {
        Self {
            cli_path: "bw".to_string(),
            server_url,
            session_token: None,
        }
    }

    /// Creates a new Bitwarden resolver with a custom CLI executable path.
    pub fn with_cli_path(cli_path: impl Into<String>, server_url: Option<String>) -> Self {
        Self {
            cli_path: cli_path.into(),
            server_url,
            session_token: None,
        }
    }

    /// Sets an optional session token for unlocked vault queries.
    pub fn with_session(mut self, session_token: impl Into<String>) -> Self {
        self.session_token = Some(session_token.into());
        self
    }

    /// Returns the path to the configured Bitwarden CLI binary.
    pub fn cli_path(&self) -> &str {
        &self.cli_path
    }

    /// Returns the configured server URL, if any.
    pub fn server_url(&self) -> Option<&str> {
        self.server_url.as_deref()
    }

    /// Returns the configured session token, if any.
    pub fn session_token(&self) -> Option<&str> {
        self.session_token.as_deref()
    }
}

impl VaultResolver for BitwardenResolver {
    fn resolve<'a>(
        &'a self,
        refs: &'a [SecretRef],
    ) -> BoxFuture<'a, Result<Vec<SecretResolution>>> {
        Box::pin(async move {
            let mut resolutions = Vec::with_capacity(refs.len());

            for secret_ref in refs {
                let item_id = extract_item_id(&secret_ref.vault_uri, &secret_ref.key_name);
                let mut cmd = tokio::process::Command::new(&self.cli_path);
                cmd.arg("get").arg("item").arg(item_id);

                if let Some(ref url) = self.server_url {
                    cmd.env("BW_SERVER_URL", url);
                }
                if let Some(ref token) = self.session_token {
                    cmd.env("BW_SESSION", token);
                }

                match cmd.output().await {
                    Ok(output) => {
                        if output.status.success() {
                            let stdout = String::from_utf8_lossy(&output.stdout);
                            match parse_bitwarden_item_output(&stdout, &secret_ref.key_name) {
                                Some(val) => {
                                    resolutions.push(SecretResolution::new(
                                        &secret_ref.key_name,
                                        val,
                                    ));
                                }
                                None => {
                                    if secret_ref.required {
                                        return Err(CtxError::vault(format!(
                                            "Could not extract secret value for required key '{}' from Bitwarden item '{}'",
                                            secret_ref.key_name, item_id
                                        )));
                                    }
                                    tracing::warn!(
                                        "Could not extract secret value for optional key '{}' from Bitwarden item '{}'",
                                        secret_ref.key_name,
                                        item_id
                                    );
                                }
                            }
                        } else {
                            let stderr = String::from_utf8_lossy(&output.stderr);
                            let err_msg = stderr.trim();
                            if secret_ref.required {
                                return Err(CtxError::vault(format!(
                                    "Bitwarden CLI failed to get item '{}' for key '{}': {}",
                                    item_id, secret_ref.key_name, err_msg
                                )));
                            }
                            tracing::warn!(
                                "Failed to retrieve optional secret '{}' (item '{}') from Bitwarden: {}",
                                secret_ref.key_name,
                                item_id,
                                err_msg
                            );
                        }
                    }
                    Err(err) => {
                        if secret_ref.required {
                            return Err(CtxError::vault(format!(
                                "Failed to execute Bitwarden CLI '{}' for key '{}': {}",
                                self.cli_path, secret_ref.key_name, err
                            )));
                        }
                        tracing::warn!(
                            "Failed to execute Bitwarden CLI for optional secret '{}': {}",
                            secret_ref.key_name,
                            err
                        );
                    }
                }
            }

            Ok(resolutions)
        })
    }

    fn check_available<'a>(&'a self) -> BoxFuture<'a, Result<bool>> {
        Box::pin(async move {
            let mut cmd = tokio::process::Command::new(&self.cli_path);
            cmd.arg("status");

            if let Some(ref url) = self.server_url {
                cmd.env("BW_SERVER_URL", url);
            }
            if let Some(ref token) = self.session_token {
                cmd.env("BW_SESSION", token);
            }

            match cmd.output().await {
                Ok(output) => {
                    if output.status.success() {
                        Ok(true)
                    } else {
                        // Some versions of bw output status JSON even on locked states
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        let has_status = serde_json::from_str::<serde_json::Value>(&stdout)
                            .ok()
                            .and_then(|val| val.get("status").cloned())
                            .is_some();
                        Ok(has_status)
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(err) => Err(CtxError::from(err)),
            }
        })
    }
}

/// Resolver for manual secrets and local environment variables.
#[derive(Debug, Clone, Default)]
pub struct ManualResolver;

impl ManualResolver {
    /// Creates a new manual secret resolver.
    pub fn new() -> Self {
        Self
    }
}

impl VaultResolver for ManualResolver {
    fn resolve<'a>(
        &'a self,
        refs: &'a [SecretRef],
    ) -> BoxFuture<'a, Result<Vec<SecretResolution>>> {
        Box::pin(async move {
            let mut resolutions = Vec::with_capacity(refs.len());

            for secret_ref in refs {
                match std::env::var(&secret_ref.key_name) {
                    Ok(val) => {
                        resolutions.push(SecretResolution::new(&secret_ref.key_name, val));
                    }
                    Err(_) => {
                        if secret_ref.required {
                            return Err(CtxError::vault(format!(
                                "Required secret '{}' not found in environment for manual resolver",
                                secret_ref.key_name
                            )));
                        }
                        tracing::warn!(
                            "Optional secret '{}' not found in environment for manual resolver",
                            secret_ref.key_name
                        );
                    }
                }
            }

            Ok(resolutions)
        })
    }

    fn check_available<'a>(&'a self) -> BoxFuture<'a, Result<bool>> {
        Box::pin(async move { Ok(true) })
    }
}

/// Factory function to create a boxed [`VaultResolver`] corresponding to the provided [`VaultConfig`].
pub fn create_resolver(config: &VaultConfig) -> Box<dyn VaultResolver> {
    match config.provider {
        VaultProvider::Bitwarden => Box::new(BitwardenResolver::new(None)),
        VaultProvider::BitwardenSelfHosted => {
            Box::new(BitwardenResolver::new(config.server_url.clone()))
        }
        VaultProvider::Manual => Box::new(ManualResolver::new()),
        VaultProvider::OnePassword => {
            todo!("Implement OnePasswordResolver integration with 1Password `op` CLI")
        }
        VaultProvider::Hashicorp => {
            todo!("Implement HashicorpResolver integration with HashiCorp Vault API or CLI")
        }
        VaultProvider::AwsSecretsManager => {
            todo!("Implement AwsSecretsManagerResolver integration with AWS Secrets Manager SDK")
        }
        VaultProvider::Sops => {
            todo!("Implement SopsResolver integration with Mozilla `sops` CLI")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vault_provider_serialization_and_display() {
        let providers = [
            (VaultProvider::Bitwarden, "\"bitwarden\"", "bitwarden"),
            (
                VaultProvider::BitwardenSelfHosted,
                "\"bitwarden_self_hosted\"",
                "bitwarden_self_hosted",
            ),
            (
                VaultProvider::OnePassword,
                "\"one_password\"",
                "one_password",
            ),
            (VaultProvider::Hashicorp, "\"hashicorp\"", "hashicorp"),
            (
                VaultProvider::AwsSecretsManager,
                "\"aws_secrets_manager\"",
                "aws_secrets_manager",
            ),
            (VaultProvider::Sops, "\"sops\"", "sops"),
            (VaultProvider::Manual, "\"manual\"", "manual"),
        ];

        for (provider, json_str, display_str) in providers {
            assert_eq!(provider.to_string(), display_str);
            assert_eq!(provider.as_str(), display_str);

            let serialized = serde_json::to_string(&provider).expect("Serialize provider");
            assert_eq!(serialized, json_str);

            let deserialized: VaultProvider =
                serde_json::from_str(&serialized).expect("Deserialize provider");
            assert_eq!(deserialized, provider);
        }

        // Test serde aliases
        let op_alias: VaultProvider =
            serde_json::from_str("\"1password\"").expect("Deserialize 1password alias");
        assert_eq!(op_alias, VaultProvider::OnePassword);

        let vault_alias: VaultProvider =
            serde_json::from_str("\"vault\"").expect("Deserialize vault alias");
        assert_eq!(vault_alias, VaultProvider::Hashicorp);
    }

    #[test]
    fn test_vault_config_and_secret_resolution() {
        let config = VaultConfig::new(VaultProvider::Bitwarden);
        assert_eq!(config.provider, VaultProvider::Bitwarden);
        assert_eq!(config.server_url, None);

        let hosted_config = VaultConfig::with_server_url(
            VaultProvider::BitwardenSelfHosted,
            "https://vault.internal.net",
        );
        assert_eq!(hosted_config.provider, VaultProvider::BitwardenSelfHosted);
        assert_eq!(
            hosted_config.server_url.as_deref(),
            Some("https://vault.internal.net")
        );

        let resolution = SecretResolution::new("STRIPE_KEY", "sk_test_12345");
        assert_eq!(resolution.key_name, "STRIPE_KEY");
        assert_eq!(resolution.value, "sk_test_12345");

        let res_json = serde_json::to_string(&resolution).expect("Serialize resolution");
        let res_de: SecretResolution =
            serde_json::from_str(&res_json).expect("Deserialize resolution");
        assert_eq!(res_de, resolution);
    }

    #[test]
    fn test_extract_item_id() {
        assert_eq!(
            extract_item_id("vault://my-secret-id", "FALLBACK"),
            "my-secret-id"
        );
        assert_eq!(
            extract_item_id("bitwarden://0123-4567-89ab", "FALLBACK"),
            "0123-4567-89ab"
        );
        assert_eq!(extract_item_id("bw://item_name", "FALLBACK"), "item_name");
        assert_eq!(extract_item_id("plain_id", "FALLBACK"), "plain_id");
        assert_eq!(extract_item_id("", "FALLBACK"), "FALLBACK");
        assert_eq!(extract_item_id("vault://", "FALLBACK"), "FALLBACK");
        assert_eq!(
            extract_item_id("vault:///nested/key/", "FALLBACK"),
            "nested/key"
        );
    }

    #[test]
    fn test_parse_bitwarden_item_login_password() {
        let json = r#"{
            "object": "item",
            "id": "1111-2222",
            "name": "Database Prod",
            "login": {
                "username": "admin",
                "password": "super_secret_db_password"
            }
        }"#;

        let parsed = parse_bitwarden_item_output(json, "DB_PASS");
        assert_eq!(parsed.as_deref(), Some("super_secret_db_password"));
    }

    #[test]
    fn test_parse_bitwarden_item_custom_fields_matching() {
        let json = r#"{
            "object": "item",
            "id": "3333-4444",
            "name": "API Service",
            "fields": [
                {
                    "name": "OTHER_KEY",
                    "value": "ignore_me"
                },
                {
                    "name": "OPENAI_API_KEY",
                    "value": "sk-proj-99887766"
                }
            ],
            "login": {
                "password": "login_pass"
            }
        }"#;

        // Custom field matching key_name takes priority
        let parsed = parse_bitwarden_item_output(json, "OPENAI_API_KEY");
        assert_eq!(parsed.as_deref(), Some("sk-proj-99887766"));

        // If key_name does not match a specific field name, fall back to login password
        let parsed_fallback = parse_bitwarden_item_output(json, "UNMATCHED_KEY");
        assert_eq!(parsed_fallback.as_deref(), Some("login_pass"));
    }

    #[test]
    fn test_parse_bitwarden_item_notes_and_raw_fallback() {
        // Secure note item
        let note_json = r#"{
            "object": "item",
            "id": "5555-6666",
            "name": "Private RSA Key",
            "notes": "-----BEGIN RSA PRIVATE KEY-----\nMIIE...\n-----END RSA PRIVATE KEY-----",
            "secureNote": {
                "type": 0
            }
        }"#;

        let parsed_note = parse_bitwarden_item_output(note_json, "RSA_KEY");
        assert!(parsed_note
            .as_deref()
            .unwrap_or("")
            .contains("BEGIN RSA PRIVATE KEY"));

        // Raw plain-text output
        let raw_text = "sk_live_pure_string_secret";
        let parsed_raw = parse_bitwarden_item_output(raw_text, "API_KEY");
        assert_eq!(parsed_raw.as_deref(), Some("sk_live_pure_string_secret"));

        // Empty string
        let parsed_empty = parse_bitwarden_item_output("", "ANY");
        assert_eq!(parsed_empty, None);
    }

    #[tokio::test]
    async fn test_manual_resolver_success_and_missing() {
        let resolver = ManualResolver::new();
        assert!(resolver.check_available().await.expect("Manual check"));

        // Test resolving an existing environment variable
        let path_val = std::env::var("PATH").expect("PATH env var exists in test environment");

        let refs = vec![
            SecretRef::new("PATH", "vault://env", true),
            SecretRef::new("CTX_TEST_OPTIONAL_MISSING_VAR", "vault://env", false),
        ];

        let resolved = resolver.resolve(&refs).await.expect("Resolve manual");
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].key_name, "PATH");
        assert_eq!(resolved[0].value, path_val);

        // Required missing variable fails
        let missing_required = vec![SecretRef::new(
            "CTX_TEST_DEFINITELY_MISSING_VAR_XYZ",
            "vault://env",
            true,
        )];
        let err = resolver
            .resolve(&missing_required)
            .await
            .expect_err("Expected missing required secret error");
        assert!(err.to_string().contains("not found in environment"));
    }

    #[tokio::test]
    async fn test_bitwarden_resolver_missing_cli() {
        let resolver =
            BitwardenResolver::with_cli_path("/nonexistent/path/to/bw_missing_cli", None);
        assert_eq!(resolver.cli_path(), "/nonexistent/path/to/bw_missing_cli");
        assert_eq!(resolver.server_url(), None);

        // Missing CLI should report unavailable
        let available = resolver
            .check_available()
            .await
            .expect("check_available missing cli");
        assert!(!available);

        // Resolving required secret with missing CLI produces error
        let refs = vec![SecretRef::new("KEY", "vault://id", true)];
        let err = resolver
            .resolve(&refs)
            .await
            .expect_err("Expected resolve failure for missing CLI");
        assert!(err.to_string().contains("Failed to execute Bitwarden CLI"));

        // Resolving optional secret with missing CLI skips without error
        let optional_refs = vec![SecretRef::new("KEY", "vault://id", false)];
        let res = resolver
            .resolve(&optional_refs)
            .await
            .expect("Optional secret with missing CLI should resolve empty");
        assert!(res.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_bitwarden_resolver_mock_cli() {
        use std::os::unix::fs::PermissionsExt;

        let script_name = format!("mock_bw_{}.sh", uuid::Uuid::new_v4());
        let script_path = std::env::temp_dir().join(script_name);

        let script_content = r#"#!/bin/sh
if [ "$1" = "status" ]; then
    echo '{"serverUrl":"https://vault.bitwarden.com","status":"unlocked"}'
    exit 0
fi

if [ "$1" = "get" ] && [ "$2" = "item" ]; then
    if [ "$3" = "found-item" ]; then
        echo '{"object":"item","id":"found-item","name":"Found","login":{"password":"mocked_secret_42"}}'
        exit 0
    else
        echo "Not found" >&2
        exit 1
    fi
fi

exit 2
"#;

        std::fs::write(&script_path, script_content).expect("Write mock script");
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))
            .expect("Chmod mock script");

        let resolver = BitwardenResolver::with_cli_path(
            script_path
                .to_str()
                .expect("Valid utf8 path for mock script"),
            Some("https://vault.bitwarden.com".to_string()),
        )
        .with_session("mock_session_key");

        assert_eq!(resolver.server_url(), Some("https://vault.bitwarden.com"));
        assert_eq!(resolver.session_token(), Some("mock_session_key"));

        // Test status check
        let is_available = resolver.check_available().await.expect("Mock bw status");
        assert!(is_available);

        // Test resolving found item
        let refs = vec![
            SecretRef::new("API_TOKEN", "vault://found-item", true),
            SecretRef::new("OPTIONAL_MISSING", "vault://missing-item", false),
        ];

        let results = resolver.resolve(&refs).await.expect("Resolve mock");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].key_name, "API_TOKEN");
        assert_eq!(results[0].value, "mocked_secret_42");

        // Test resolving required missing item
        let failing_refs = vec![SecretRef::new(
            "FAIL_REQUIRED",
            "vault://missing-item",
            true,
        )];
        let err = resolver
            .resolve(&failing_refs)
            .await
            .expect_err("Expected resolution error");
        assert!(err.to_string().contains("Bitwarden CLI failed to get item"));

        let _ = std::fs::remove_file(&script_path);
    }

    #[test]
    fn test_create_resolver_factory() {
        let bw_config = VaultConfig::new(VaultProvider::Bitwarden);
        let bw_resolver = create_resolver(&bw_config);
        // Trait object dynamically dispatched
        assert!(format!("{:?}", std::any::type_name_of_val(&bw_resolver)).contains("VaultResolver"));

        let hosted_config = VaultConfig::with_server_url(
            VaultProvider::BitwardenSelfHosted,
            "https://bw.example.com",
        );
        let _hosted_resolver = create_resolver(&hosted_config);

        let manual_config = VaultConfig::new(VaultProvider::Manual);
        let _manual_resolver = create_resolver(&manual_config);
    }
}


