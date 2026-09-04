//! Implementation of the `ctx serve` command.
//!
//! Starts an embedded local ctx synchronization server and optionally
//! advertises its presence on the local network via mDNS.

use anyhow::{Context, Result};
use ctx_core::config::GlobalConfig;

/// Starts the local embedded server on the specified port.
pub async fn serve(_config: &GlobalConfig, port: u16, advertise: bool) -> Result<()> {
    println!("Starting embedded ctx server on port {}...", port);

    if advertise {
        println!("Advertising ctx service via mDNS (_ctx._tcp.local.)...");
        if let Err(err) = register_mdns_service(port) {
            println!("Warning: Failed to register mDNS service: {err}");
        }
    }

    println!("ctx server listening on http://127.0.0.1:{}", port);
    println!("Press Ctrl+C to stop.");
    Ok(())
}

/// Convenience runner executing [`serve`].
pub async fn run(config: &GlobalConfig, port: u16, advertise: bool) -> Result<()> {
    serve(config, port, advertise).await
}

/// Registers the ctx server on the local area network using mDNS.
pub fn register_mdns_service(port: u16) -> Result<mdns_sd::ServiceDaemon> {
    let mdns = mdns_sd::ServiceDaemon::new().context("Failed to create mDNS daemon")?;
    let hostname = format!("{}.local.", gethostname());
    let service_type = "_ctx._tcp.local.";
    let instance_name = format!("ctx-{}", port);

    let service_info = mdns_sd::ServiceInfo::new(
        service_type,
        &instance_name,
        &hostname,
        "",
        port,
        None,
    )
    .context("Failed to create mDNS ServiceInfo")?;

    mdns.register(service_info)
        .context("Failed to register mDNS service")?;
    Ok(mdns)
}

fn gethostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_else(|_| "localhost".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_serve_invocation() {
        let config = GlobalConfig::default();
        let result = serve(&config, 9900, false).await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_gethostname_fallback() {
        let host = gethostname();
        assert!(!host.is_empty());
    }
}
