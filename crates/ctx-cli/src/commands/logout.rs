//! Implementation of the `ctx logout` command.
//!
//! Clears authentication credentials from the operating system keychain.

use anyhow::Result;
use ctx_core::config::GlobalConfig;

/// Logs out the user and clears credentials from the OS keychain.
pub async fn logout(config: &GlobalConfig) -> Result<()> {
    crate::commands::login::logout(config).await
}

/// Convenience runner executing [`logout`].
pub async fn run(config: &GlobalConfig) -> Result<()> {
    logout(config).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_logout_command_callable() {
        let config = GlobalConfig::default();
        // Verifies callable signature without panic
        let _ = logout(&config).await;
    }

    #[test]
    fn test_logout_runner_structure() {
        let config = GlobalConfig::default();
        let _ = config.interval;
    }
}
