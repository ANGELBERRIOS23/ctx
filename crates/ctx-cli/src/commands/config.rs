//! Implementation of the `ctx config` command.
//!
//! Gets, sets, or updates persistent global configuration keys in `~/.ctx/config.yaml`.

use anyhow::Result;
use ctx_core::config::{GlobalConfig, SyncMode};

/// Updates a global configuration key-value pair and persists it to disk.
pub async fn config(current_config: &GlobalConfig, key: &str, value: &str) -> Result<()> {
    let mut updated = current_config.clone();

    match key.to_ascii_lowercase().as_str() {
        "sync_mode" => match value.to_ascii_lowercase().as_str() {
            "auto" => updated.sync_mode = SyncMode::Auto,
            "selective" => updated.sync_mode = SyncMode::Selective,
            other => anyhow::bail!("Invalid sync_mode '{}': expected 'auto' or 'selective'", other),
        },
        "interval" => {
            let parsed: u64 = value
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid interval value '{}': must be a positive integer", value))?;
            updated.interval = parsed;
        }
        "auto_save_on_agent_exit" => {
            let parsed: bool = value
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid boolean value '{}': expected 'true' or 'false'", value))?;
            updated.auto_save_on_agent_exit = parsed;
        }
        unknown => anyhow::bail!(
            "Unknown configuration key '{}'. Valid keys: sync_mode, interval, auto_save_on_agent_exit",
            unknown
        ),
    }

    if let Err(err) = updated.save_default() {
        println!("Warning: Could not write to default config file: {err}");
    }

    println!("Successfully set configuration '{}' = '{}'.", key, value);
    Ok(())
}

/// Convenience runner executing [`config`].
pub async fn run(current_config: &GlobalConfig, key: &str, value: &str) -> Result<()> {
    config(current_config, key, value).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_set_valid_keys() {
        let global = GlobalConfig::default();
        let res_mode = config(&global, "sync_mode", "selective").await;
        assert!(res_mode.is_ok());

        let res_interval = config(&global, "interval", "120").await;
        assert!(res_interval.is_ok());

        let res_auto_save = config(&global, "auto_save_on_agent_exit", "false").await;
        assert!(res_auto_save.is_ok());
    }

    #[tokio::test]
    async fn test_set_invalid_key_fails() {
        let global = GlobalConfig::default();
        let res_invalid_key = config(&global, "nonexistent_key", "123").await;
        assert!(res_invalid_key.is_err());

        let res_invalid_val = config(&global, "sync_mode", "invalid_mode").await;
        assert!(res_invalid_val.is_err());
    }
}
