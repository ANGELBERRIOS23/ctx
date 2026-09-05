//! Activity history command — displays audit log of syncs, claims, and logins for a project.

use anyhow::{Context, Result};
use console::style;
use ctx_core::config::ProjectConfig;

/// Audit entry received from the server.
#[derive(Debug, serde::Deserialize)]
struct AuditEntry {
    action: String,
    machine_name: Option<String>,
    detail: Option<String>,
    created_at: String,
}

/// Displays the activity log for the current or specified project.
pub async fn run(_config: &ctx_core::config::GlobalConfig, project: Option<&str>) -> Result<()> {
    let project_dir = std::env::current_dir()?;
    let cfg = ProjectConfig::load(&project_dir)
        .context("No .ctx/config.yaml found. Run 'ctx init' first.")?;

    let server = cfg.project.server.trim_end_matches('/');
    if server.is_empty() {
        anyhow::bail!("No server configured. Run 'ctx login --server <url>' first.");
    }

    let project_id = if let Some(_name) = project {
        // TODO: resolve project by name
        cfg.project.id
    } else {
        cfg.project.id
    };

    let token = crate::commands::push::get_auth_token();
    let client = reqwest::Client::new();
    let url = format!("{}/api/audit/{}", server, project_id);

    let mut req = client.get(&url);
    if let Some(ref t) = token {
        req = req.header("Authorization", format!("Bearer {}", t));
    }

    let resp = req.send().await.context("Failed to connect to server")?;

    if !resp.status().is_success() {
        anyhow::bail!("Server returned {}", resp.status());
    }

    let entries: Vec<AuditEntry> = resp.json().await.context("Failed to parse audit log")?;

    if entries.is_empty() {
        println!("  No activity recorded yet for project '{}'.", cfg.project.name);
        return Ok(());
    }

    println!(
        "\n{} Activity log for {} (last {} entries)\n",
        style("---").dim(),
        style(&cfg.project.name).cyan().bold(),
        entries.len()
    );
    println!(
        "  {:<22} {:<14} {:<10} {}",
        style("Date").bold().underlined(),
        style("Machine").bold().underlined(),
        style("Action").bold().underlined(),
        style("Detail").bold().underlined(),
    );

    for e in &entries {
        let date = &e.created_at[..19]; // trim timezone
        let machine = e.machine_name.as_deref().unwrap_or("-");
        let detail = e.detail.as_deref().unwrap_or("");
        let action_styled = match e.action.as_str() {
            "push" => style(&e.action).green(),
            "pull" => style(&e.action).cyan(),
            "claim" => style(&e.action).yellow(),
            "login" => style(&e.action).blue(),
            _ => style(&e.action).dim(),
        };
        println!("  {:<22} {:<14} {:<10} {}", date, machine, action_styled, detail);
    }
    println!();

    Ok(())
}
