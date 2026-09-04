//! Implementation of the `ctx doctor` command.
//!
//! Performs diagnostic health checks on the local environment, verifying
//! required tools, directories, and configuration settings.

use anyhow::Result;
use ctx_core::config::{GlobalConfig, ProjectConfig};

/// Individual diagnostic check result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticCheck {
    /// Name of the check performed.
    pub name: String,
    /// Whether the check passed successfully.
    pub passed: bool,
    /// Diagnostic description or failure reason.
    pub message: String,
}

impl DiagnosticCheck {
    /// Creates a new passed diagnostic check.
    pub fn pass(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            passed: true,
            message: message.into(),
        }
    }

    /// Creates a new failed diagnostic check.
    pub fn fail(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            passed: false,
            message: message.into(),
        }
    }
}

/// Runs all diagnostic checks and prints the results to stdout.
pub async fn doctor(config: &GlobalConfig) -> Result<()> {
    let checks = run_diagnostics(config);
    let mut all_ok = true;

    println!("ctx system diagnostics:");
    println!("──────────────────────────────────────────");
    for check in &checks {
        let icon = if check.passed { "✓" } else { "✗" };
        println!("  {} {}: {}", icon, check.name, check.message);
        if !check.passed {
            all_ok = false;
        }
    }
    println!("──────────────────────────────────────────");

    if all_ok {
        println!("All diagnostic checks passed.");
    } else {
        println!("Some diagnostic checks reported warnings or errors.");
    }

    Ok(())
}

/// Convenience runner executing [`doctor`].
pub async fn run(config: &GlobalConfig) -> Result<()> {
    doctor(config).await
}

/// Runs diagnostic evaluation returning a list of [`DiagnosticCheck`].
pub fn run_diagnostics(_config: &GlobalConfig) -> Vec<DiagnosticCheck> {
    let mut checks = Vec::new();

    // 1. Home directory check
    match dirs::home_dir() {
        Some(home) => checks.push(DiagnosticCheck::pass(
            "User home directory",
            format!("Found at {}", home.display()),
        )),
        None => checks.push(DiagnosticCheck::fail(
            "User home directory",
            "Could not determine user home directory",
        )),
    }

    // 2. Global ctx directory check
    match GlobalConfig::global_dir() {
        Ok(dir) => {
            let exists = dir.exists();
            checks.push(DiagnosticCheck::pass(
                "Global ctx directory",
                format!(
                    "{} (exists: {})",
                    dir.display(),
                    if exists { "yes" } else { "no (will create on demand)" }
                ),
            ));
        }
        Err(err) => checks.push(DiagnosticCheck::fail(
            "Global ctx directory",
            format!("Failed to determine global path: {err}"),
        )),
    }

    // 3. Git executable check
    match std::process::Command::new("git").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            checks.push(DiagnosticCheck::pass("Git executable", version));
        }
        Ok(_) => checks.push(DiagnosticCheck::fail(
            "Git executable",
            "Git returned non-zero exit status",
        )),
        Err(err) => checks.push(DiagnosticCheck::fail(
            "Git executable",
            format!("Git not found or executable failed: {err}"),
        )),
    }

    // 4. Current project check
    if let Ok(cur) = std::env::current_dir() {
        match ProjectConfig::load(&cur) {
            Ok(proj) => checks.push(DiagnosticCheck::pass(
                "Current project",
                format!("Active ctx project '{}' ({})", proj.project.name, proj.project.id),
            )),
            Err(_) => checks.push(DiagnosticCheck::pass(
                "Current project",
                "Not inside a ctx project workspace (run `ctx init <name>` to initialize)",
            )),
        }
    }

    checks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_diagnostics_returns_results() {
        let config = GlobalConfig::default();
        let checks = run_diagnostics(&config);
        assert!(!checks.is_empty());
        assert!(checks.iter().any(|c| c.name == "User home directory"));
        assert!(checks.iter().any(|c| c.name == "Global ctx directory"));
    }

    #[test]
    fn test_diagnostic_check_constructors() {
        let pass = DiagnosticCheck::pass("Check1", "Working");
        assert!(pass.passed);
        assert_eq!(pass.name, "Check1");
        assert_eq!(pass.message, "Working");

        let fail = DiagnosticCheck::fail("Check2", "Broken");
        assert!(!fail.passed);
        assert_eq!(fail.name, "Check2");
        assert_eq!(fail.message, "Broken");
    }
}
