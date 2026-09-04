//! Implementation of the `ctx connect` command.
//!
//! Connects to a remote ctx server or discovers active peer instances
//! on the local network via mDNS.

use std::time::Duration;

use anyhow::{Context, Result};
use ctx_core::config::GlobalConfig;

/// Connects to a server or browses for available servers on the local network.
pub async fn connect(_config: &GlobalConfig, url: Option<&str>, discover: bool) -> Result<()> {
    if discover {
        println!("Discovering ctx servers on local network via mDNS (_ctx._tcp.local.)...");
        let found = discover_servers(Duration::from_millis(500))?;
        if found.is_empty() {
            println!("No ctx servers discovered on local network.");
        } else {
            println!("Discovered {} ctx server(s):", found.len());
            for s in found {
                println!("  • {}", s);
            }
        }
        return Ok(());
    }

    let target_url = url.unwrap_or("http://127.0.0.1:9900");
    println!("Connecting to ctx server at {}...", target_url);

    let health_endpoint = format!("{}/health", target_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    match client.get(&health_endpoint).send().await {
        Ok(resp) if resp.status().is_success() => {
            println!("Successfully connected to ctx server at {}.", target_url);
            Ok(())
        }
        Ok(resp) => {
            println!(
                "Connected to server, but health check returned status {}.",
                resp.status()
            );
            Ok(())
        }
        Err(err) => {
            println!("Could not connect to {}: {}", target_url, err);
            Ok(())
        }
    }
}

/// Convenience runner executing [`connect`].
pub async fn run(config: &GlobalConfig, url: Option<&str>, discover: bool) -> Result<()> {
    connect(config, url, discover).await
}

/// Browses for `_ctx._tcp.local.` services on the LAN for up to `timeout`.
pub fn discover_servers(timeout: Duration) -> Result<Vec<String>> {
    let mdns = mdns_sd::ServiceDaemon::new().context("Failed to start mDNS service daemon")?;
    let receiver = mdns
        .browse("_ctx._tcp.local.")
        .context("Failed to browse mDNS services")?;

    let mut discovered = Vec::new();
    let start = std::time::Instant::now();

    while start.elapsed() < timeout {
        if let Ok(event) = receiver.recv_timeout(Duration::from_millis(50))
            && let mdns_sd::ServiceEvent::ServiceResolved(info) = event {
                discovered.push(format!(
                    "{} ({}:{})",
                    info.get_fullname(),
                    info.get_hostname(),
                    info.get_port()
                ));
            }
    }

    Ok(discovered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_connect_with_explicit_url() {
        let config = GlobalConfig::default();
        let res = connect(&config, Some("http://127.0.0.1:9900"), false).await;
        assert!(res.is_ok());
    }

    #[test]
    fn test_discover_servers_runs_without_panic() {
        let res = discover_servers(Duration::from_millis(50));
        assert!(res.is_ok());
    }
}
