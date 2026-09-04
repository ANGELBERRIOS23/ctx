//! Implementation of the `ctx machines` command.
//!
//! Lists registered machines participating in the synchronization mesh.

use anyhow::Result;
use ctx_core::config::GlobalConfig;
use ctx_core::protocol::MachineInfo;
use uuid::Uuid;

/// Lists registered machines and displays the current machine identity.
pub async fn machines(_config: &GlobalConfig) -> Result<()> {
    let hostname = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_else(|_| "localhost".to_string());

    let current = MachineInfo::current(Uuid::new_v4(), &hostname, "local-fingerprint");

    println!("Registered synchronization machines:");
    println!("──────────────────────────────────────────");
    println!(
        "  • {} (current)\n    ID:   {}\n    OS:   {}\n    Last: {}",
        current.name,
        current.id,
        current.os,
        current.last_seen.to_rfc3339()
    );
    println!("──────────────────────────────────────────");
    Ok(())
}

/// Convenience runner executing [`machines`].
pub async fn run(config: &GlobalConfig) -> Result<()> {
    machines(config).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_machines_command_callable() {
        let config = GlobalConfig::default();
        let res = machines(&config).await;
        assert!(res.is_ok());
    }

    #[test]
    fn test_current_machine_metadata() {
        let info = MachineInfo::current(Uuid::new_v4(), "my-laptop", "fp-1");
        assert_eq!(info.name, "my-laptop");
        assert_eq!(info.fingerprint, "fp-1");
    }
}
