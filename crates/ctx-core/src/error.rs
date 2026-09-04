//! Error types and result aliases for the `ctx-core` crate.
//!
//! This module defines [`CtxError`], the central error enum used throughout
//! `ctx-core`, along with the [`Result`] type alias and [`From`] implementations
//! for standard I/O and serialization error types.

use std::io;
use thiserror::Error;

/// A specialized `Result` type for `ctx-core` operations.
pub type Result<T> = std::result::Result<T, CtxError>;

/// Central error enum representing all failure modes in `ctx-core`.
#[derive(Debug, Error)]
pub enum CtxError {
    /// Configuration parsing, validation, or loading error.
    #[error("Configuration error: {0}")]
    Config(String),

    /// Cryptographic operation error (encryption, decryption, key derivation).
    #[error("Crypto error: {0}")]
    Crypto(String),

    /// Standard I/O error.
    #[error("I/O error: {0}")]
    Io(#[source] io::Error),

    /// Vault or secret storage error.
    #[error("Vault error: {0}")]
    Vault(String),

    /// Sync engine or network synchronization error.
    #[error("Sync error: {0}")]
    Sync(String),

    /// Authentication or authorization error.
    #[error("Auth error: {0}")]
    Auth(String),

    /// Agent handoff bundle error.
    #[error("Handoff error: {0}")]
    Handoff(String),

    /// Requested resource or entity was not found.
    #[error("Not found: {0}")]
    NotFound(String),

    /// Session lock conflict between machines or processes.
    #[error("Lock conflict: {0}")]
    LockConflict(String),

    /// State machine invariant or lifecycle violation.
    #[error("Invalid state: {0}")]
    InvalidState(String),
}

impl CtxError {
    /// Creates a new [`CtxError::Config`] error with the provided message.
    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config(msg.into())
    }

    /// Creates a new [`CtxError::Crypto`] error with the provided message.
    pub fn crypto(msg: impl Into<String>) -> Self {
        Self::Crypto(msg.into())
    }

    /// Creates a new [`CtxError::Vault`] error with the provided message.
    pub fn vault(msg: impl Into<String>) -> Self {
        Self::Vault(msg.into())
    }

    /// Creates a new [`CtxError::Sync`] error with the provided message.
    pub fn sync(msg: impl Into<String>) -> Self {
        Self::Sync(msg.into())
    }

    /// Creates a new [`CtxError::Auth`] error with the provided message.
    pub fn auth(msg: impl Into<String>) -> Self {
        Self::Auth(msg.into())
    }

    /// Creates a new [`CtxError::Handoff`] error with the provided message.
    pub fn handoff(msg: impl Into<String>) -> Self {
        Self::Handoff(msg.into())
    }

    /// Creates a new [`CtxError::NotFound`] error with the provided message.
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::NotFound(msg.into())
    }

    /// Creates a new [`CtxError::LockConflict`] error with the provided message.
    pub fn lock_conflict(msg: impl Into<String>) -> Self {
        Self::LockConflict(msg.into())
    }

    /// Creates a new [`CtxError::InvalidState`] error with the provided message.
    pub fn invalid_state(msg: impl Into<String>) -> Self {
        Self::InvalidState(msg.into())
    }
}

impl From<io::Error> for CtxError {
    /// Converts a [`std::io::Error`] into a [`CtxError::Io`].
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<serde_json::Error> for CtxError {
    /// Converts a [`serde_json::Error`] into a [`CtxError::Config`].
    fn from(err: serde_json::Error) -> Self {
        Self::Config(err.to_string())
    }
}

impl From<serde_yaml::Error> for CtxError {
    /// Converts a [`serde_yaml::Error`] into a [`CtxError::Config`].
    fn from(err: serde_yaml::Error) -> Self {
        Self::Config(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;
    use std::io::{Error as IoError, ErrorKind};

    #[test]
    fn test_error_display_variants() {
        let cases = [
            (
                CtxError::Config("invalid syntax".to_string()),
                "Configuration error: invalid syntax",
            ),
            (
                CtxError::Crypto("decryption failed".to_string()),
                "Crypto error: decryption failed",
            ),
            (
                CtxError::Io(IoError::new(ErrorKind::NotFound, "file missing")),
                "I/O error: file missing",
            ),
            (
                CtxError::Vault("keychain denied".to_string()),
                "Vault error: keychain denied",
            ),
            (
                CtxError::Sync("sync conflict".to_string()),
                "Sync error: sync conflict",
            ),
            (
                CtxError::Auth("unauthorized token".to_string()),
                "Auth error: unauthorized token",
            ),
            (
                CtxError::Handoff("missing summary".to_string()),
                "Handoff error: missing summary",
            ),
            (
                CtxError::NotFound("project not found".to_string()),
                "Not found: project not found",
            ),
            (
                CtxError::LockConflict("locked by node-1".to_string()),
                "Lock conflict: locked by node-1",
            ),
            (
                CtxError::InvalidState("already running".to_string()),
                "Invalid state: already running",
            ),
        ];

        for (err, expected) in cases {
            assert_eq!(err.to_string(), expected);
        }
    }

    #[test]
    fn test_from_io_error() {
        let io_err = IoError::new(ErrorKind::PermissionDenied, "access denied");
        let ctx_err: CtxError = io_err.into();

        match ctx_err {
            CtxError::Io(ref inner) => {
                assert_eq!(inner.kind(), ErrorKind::PermissionDenied);
                assert_eq!(inner.to_string(), "access denied");
            }
            _ => panic!("Expected CtxError::Io variant"),
        }

        assert!(ctx_err.source().is_some());
    }

    #[test]
    fn test_from_serde_json_error() {
        let json_res: std::result::Result<serde_json::Value, serde_json::Error> =
            serde_json::from_str("{ invalid json }");
        let json_err = json_res.expect_err("Expected invalid JSON syntax error");
        let ctx_err: CtxError = json_err.into();

        match ctx_err {
            CtxError::Config(msg) => {
                assert!(!msg.is_empty());
            }
            _ => panic!("Expected CtxError::Config variant for serde_json::Error"),
        }
    }

    #[test]
    fn test_from_serde_yaml_error() {
        let yaml_res: std::result::Result<serde_yaml::Value, serde_yaml::Error> =
            serde_yaml::from_str(":\n  - bad yaml");
        let yaml_err = yaml_res.expect_err("Expected invalid YAML syntax error");
        let ctx_err: CtxError = yaml_err.into();

        match ctx_err {
            CtxError::Config(msg) => {
                assert!(!msg.is_empty());
            }
            _ => panic!("Expected CtxError::Config variant for serde_yaml::Error"),
        }
    }

    #[test]
    fn test_helper_constructors() {
        assert_eq!(
            CtxError::config("bad config").to_string(),
            "Configuration error: bad config"
        );
        assert_eq!(
            CtxError::crypto("key error").to_string(),
            "Crypto error: key error"
        );
        assert_eq!(
            CtxError::vault("missing key").to_string(),
            "Vault error: missing key"
        );
        assert_eq!(
            CtxError::sync("network timeout").to_string(),
            "Sync error: network timeout"
        );
        assert_eq!(
            CtxError::auth("invalid token").to_string(),
            "Auth error: invalid token"
        );
        assert_eq!(
            CtxError::handoff("corrupted packet").to_string(),
            "Handoff error: corrupted packet"
        );
        assert_eq!(
            CtxError::not_found("item 42").to_string(),
            "Not found: item 42"
        );
        assert_eq!(
            CtxError::lock_conflict("worker A").to_string(),
            "Lock conflict: worker A"
        );
        assert_eq!(
            CtxError::invalid_state("uninitialized").to_string(),
            "Invalid state: uninitialized"
        );
    }
}
