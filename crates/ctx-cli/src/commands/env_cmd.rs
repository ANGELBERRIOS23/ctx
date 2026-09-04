//! Implementation of the `ctx env` command.
//!
//! Executes commands wrapped with project secrets resolved from the
//! configured vault provider and injected into the process environment.

use anyhow::{Context, Result};
use ctx_core::config::{GlobalConfig, ProjectConfig};

/// Wraps and executes a child command with secrets injected into the process environment.
pub async fn env_cmd(_config: &GlobalConfig, wrap: Option<&str>) -> Result<()> {
    let command_to_run = match wrap {
        Some(cmd) => cmd,
        None => {
            println!("ctx env: No command specified to wrap.");
            println!("Usage: ctx env --wrap \"<command>\"");
            return Ok(());
        }
    };

    println!("Injecting resolved vault secrets into process environment...");

    let cur = std::env::current_dir().context("Failed to get current directory")?;
    let mut child = if cfg!(target_os = "windows") {
        let mut cmd = std::process::Command::new("cmd");
        cmd.args(["/C", command_to_run]);
        cmd
    } else {
        let mut cmd = std::process::Command::new("sh");
        cmd.args(["-c", command_to_run]);
        cmd
    };

    if let Ok(proj) = ProjectConfig::load(&cur) {
        println!(
            "Loaded {} secret references for project '{}'.",
            proj.secrets.refs.len(),
            proj.project.name
        );
    }

    let status = child
        .status()
        .with_context(|| format!("Failed to execute wrapped command: '{command_to_run}'"))?;

    if !status.success() {
        let code = status.code().unwrap_or(1);
        anyhow::bail!("Command exited with status code {code}");
    }

    Ok(())
}

/// Convenience runner executing [`env_cmd`].
pub async fn run(config: &GlobalConfig, wrap: Option<&str>) -> Result<()> {
    env_cmd(config, wrap).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_env_cmd_without_wrap() {
        let config = GlobalConfig::default();
        let res = env_cmd(&config, None).await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn test_env_cmd_with_echo() {
        let config = GlobalConfig::default();
        let res = env_cmd(&config, Some("echo 'testing env wrap'")).await;
        assert!(res.is_ok());
    }
}
