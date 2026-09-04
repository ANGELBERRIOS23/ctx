//! Implementation of the `ctx find` command.
//!
//! Searches across AI agent session histories, transcripts, and handoffs
//! using keyword matching and relevance ranking.

use anyhow::Result;
use ctx_core::config::GlobalConfig;

/// Searches across agent sessions matching the given query, platform, and limit.
pub async fn find(
    _config: &GlobalConfig,
    query: &str,
    platform: Option<&str>,
    recent: Option<u32>,
) -> Result<()> {
    println!("Searching agent sessions for query: '{}'...", query);

    let adapters: Vec<Box<dyn ctx_adapters::AgentAdapter>> = vec![
        Box::new(ctx_adapters::ClaudeAdapter::new()),
        Box::new(ctx_adapters::CursorAdapter::new()),
        Box::new(ctx_adapters::CodexAdapter::new()),
        Box::new(ctx_adapters::GenericAdapter::new()),
    ];
    let mut matches = ctx_adapters::search_all_agents(query, &adapters).await;

    if let Some(plat) = platform {
        let plat_lower = plat.to_ascii_lowercase();
        matches.retain(|m| m.agent.to_ascii_lowercase().contains(&plat_lower));
    }

    if let Some(limit) = recent {
        matches.truncate(limit as usize);
    }

    if matches.is_empty() {
        println!("No matching agent sessions found.");
    } else {
        println!("Found {} matching session(s):", matches.len());
        for m in &matches {
            let proj_str = m.project_name.as_deref().unwrap_or("<unknown>");
            println!("  • [{}] {} (project: {})", m.agent, m.session_id, proj_str);
            if let Some(ref snippet) = m.snippet {
                println!("    Snippet: {}", snippet);
            }
        }
    }

    Ok(())
}

/// Convenience runner executing [`find`].
pub async fn run(
    config: &GlobalConfig,
    query: &str,
    platform: Option<&str>,
    recent: Option<u32>,
) -> Result<()> {
    find(config, query, platform, recent).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_find_with_dummy_query() {
        let config = GlobalConfig::default();
        let res = find(&config, "ctx test search query", None, Some(5)).await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn test_find_with_platform_filter() {
        let config = GlobalConfig::default();
        let res = find(&config, "token", Some("claude"), None).await;
        assert!(res.is_ok());
    }
}
