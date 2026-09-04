//! Main executable entry point for the `ctx` CLI.
//!
//! A cross-OS developer tool that synchronizes projects, AI agent context,
//! and secrets across machines.

pub mod commands;

use clap::{Parser, Subcommand};
use ctx_core::config::GlobalConfig;
use serde::{Deserialize, Serialize};

pub use commands::{SecretsCmd, SyncCmd};

/// Subcommands available in the `ctx` CLI.
#[derive(Subcommand, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Commands {
    /// Authenticate with the ctx server and save credentials.
    Login {
        /// Server URL (e.g. http://52.6.216.78:9900). Saved for future use.
        #[arg(short, long)]
        server: Option<String>,
        /// Connect directly to another machine (P2P mode).
        #[arg(short, long)]
        direct: Option<String>,
    },
    /// Log out and clear saved credentials from the OS keychain.
    Logout,
    /// Initialize a new ctx project in the workspace.
    Init {
        /// Name of the project to initialize.
        name: String,
    },
    /// Diagnose system health and environment dependencies.
    Doctor,
    /// Display synchronization and session lock status.
    Status,
    /// Pull the latest project snapshot from the server.
    Pull {
        /// Project name to pull (defaults to current project).
        #[arg(short, long)]
        project: Option<String>,
        /// Pull all tracked projects.
        #[arg(short, long, default_value_t = false)]
        all: bool,
    },
    /// Push current project state and context to the server.
    Push {
        /// Push all tracked projects.
        #[arg(short, long, default_value_t = false)]
        all: bool,
    },
    /// Acquire an exclusive session lock for a project.
    Claim {
        /// Project name to claim (defaults to current project).
        #[arg(short, long)]
        project: Option<String>,
    },
    /// Release the active session lock for a project.
    Release {
        /// Project name to release (defaults to current project).
        #[arg(short, long)]
        project: Option<String>,
    },
    /// Resume AI agent context and restore state.
    Resume {
        /// Agent adapter name (e.g., "claude", "cursor", "codex").
        #[arg(short, long)]
        agent: Option<String>,
    },
    /// Save current agent context and create a handoff snapshot.
    Save {
        /// Optional message describing current progress.
        #[arg(short, long)]
        message: Option<String>,
    },
    /// Start the embedded local ctx server.
    Serve {
        /// Port number to listen on.
        #[arg(short, long, default_value_t = 9900)]
        port: u16,
        /// Advertise service on LAN via mDNS.
        #[arg(short, long, default_value_t = false)]
        advertise: bool,
    },
    /// Connect to a remote or local ctx server.
    Connect {
        /// Server URL to connect to.
        #[arg(short, long)]
        url: Option<String>,
        /// Discover servers on local network via mDNS.
        #[arg(short, long, default_value_t = false)]
        discover: bool,
    },
    /// Search across AI agent sessions and handoffs.
    Find {
        /// Search query string.
        query: String,
        /// Filter by platform or agent name.
        #[arg(short, long)]
        platform: Option<String>,
        /// Limit results to most recent N sessions.
        #[arg(short, long)]
        recent: Option<u32>,
    },
    /// Manage synchronization configuration and manual syncs.
    #[command(subcommand)]
    Sync(SyncCmd),
    /// Manage project secrets and vault integration.
    #[command(subcommand)]
    Secrets(SecretsCmd),
    /// Run a command wrapped with decrypted vault secrets in the environment.
    Env {
        /// Command to execute with injected secrets.
        #[arg(short, long)]
        wrap: Option<String>,
    },
    /// List all tracked development projects.
    Projects,
    /// List all registered machines in the sync network.
    Machines,
    /// Set or update a global configuration key-value pair.
    Config {
        /// Configuration key (e.g., "sync_mode", "interval").
        key: String,
        /// Configuration value to set.
        value: String,
    },
}

/// Command-line arguments parser for the `ctx` CLI.
#[derive(Parser, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[command(
    name = "ctx",
    author,
    version,
    about = "Cross-OS AI agent context and project synchronizer",
    long_about = "A cross-OS CLI that synchronizes development projects, AI agent context, and secrets across machines."
)]
pub struct Cli {
    /// Subcommand to execute.
    #[command(subcommand)]
    pub command: Commands,
}

/// Initializes the tracing subscriber for CLI logging output.
pub fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,ctx_cli=debug"));

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .compact()
        .try_init();
}

/// Dispatches the parsed subcommand to its corresponding handler.
pub async fn dispatch(command: Commands, config: &GlobalConfig) -> anyhow::Result<()> {
    match command {
        Commands::Login { server, direct } => commands::login(config, server.or(direct)).await,
        Commands::Logout => commands::logout(config).await,
        Commands::Init { name } => commands::init(config, &name).await,
        Commands::Doctor => commands::doctor(config).await,
        Commands::Status => commands::status(config).await,
        Commands::Pull { project, all } => commands::pull(config, project.as_deref(), all).await,
        Commands::Push { all } => commands::push(config, all).await,
        Commands::Claim { project } => commands::claim(config, project.as_deref()).await,
        Commands::Release { project } => commands::release(config, project.as_deref()).await,
        Commands::Resume { agent } => commands::resume(config, agent.as_deref()).await,
        Commands::Save { message } => commands::save(config, message.as_deref()).await,
        Commands::Serve { port, advertise } => commands::serve(config, port, advertise).await,
        Commands::Connect { url, discover } => {
            commands::connect(config, url.as_deref(), discover).await
        }
        Commands::Find {
            query,
            platform,
            recent,
        } => commands::find(config, &query, platform.as_deref(), recent).await,
        Commands::Sync(cmd) => commands::sync_cmd(config, &cmd).await,
        Commands::Secrets(cmd) => commands::secrets(config, &cmd).await,
        Commands::Env { wrap } => commands::env_cmd(config, wrap.as_deref()).await,
        Commands::Projects => commands::projects(config).await,
        Commands::Machines => commands::machines(config).await,
        Commands::Config { key, value } => commands::config(config, &key, &value).await,
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let cli = Cli::parse();

    let config = match GlobalConfig::load_default() {
        Ok(cfg) => cfg,
        Err(ctx_core::config::ConfigError::NotFound(_)) => {
            tracing::debug!("Global config not found, using default configuration");
            GlobalConfig::default()
        }
        Err(err) => {
            tracing::warn!("Failed to load global config: {err}, falling back to defaults");
            GlobalConfig::default()
        }
    };

    dispatch(cli.command, &config).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parse_subcommands() {
        // Init
        let cli = Cli::try_parse_from(["ctx", "init", "backend-service"])
            .expect("Parse init subcommand");
        assert_eq!(
            cli.command,
            Commands::Init {
                name: "backend-service".to_string()
            }
        );

        // Doctor
        let cli = Cli::try_parse_from(["ctx", "doctor"]).expect("Parse doctor subcommand");
        assert_eq!(cli.command, Commands::Doctor);

        // Status
        let cli = Cli::try_parse_from(["ctx", "status"]).expect("Parse status subcommand");
        assert_eq!(cli.command, Commands::Status);

        // Pull
        let cli = Cli::try_parse_from(["ctx", "pull", "--project", "api", "--all"])
            .expect("Parse pull subcommand");
        assert_eq!(
            cli.command,
            Commands::Pull {
                project: Some("api".to_string()),
                all: true,
            }
        );

        // Push
        let cli = Cli::try_parse_from(["ctx", "push", "--all"]).expect("Parse push subcommand");
        assert_eq!(cli.command, Commands::Push { all: true });

        // Claim
        let cli = Cli::try_parse_from(["ctx", "claim", "-p", "api"])
            .expect("Parse claim subcommand");
        assert_eq!(
            cli.command,
            Commands::Claim {
                project: Some("api".to_string())
            }
        );

        // Release
        let cli = Cli::try_parse_from(["ctx", "release", "-p", "api"])
            .expect("Parse release subcommand");
        assert_eq!(
            cli.command,
            Commands::Release {
                project: Some("api".to_string())
            }
        );

        // Resume
        let cli = Cli::try_parse_from(["ctx", "resume", "--agent", "claude"])
            .expect("Parse resume subcommand");
        assert_eq!(
            cli.command,
            Commands::Resume {
                agent: Some("claude".to_string())
            }
        );

        // Save
        let cli = Cli::try_parse_from(["ctx", "save", "-m", "completed auth refactor"])
            .expect("Parse save subcommand");
        assert_eq!(
            cli.command,
            Commands::Save {
                message: Some("completed auth refactor".to_string())
            }
        );

        // Serve
        let cli = Cli::try_parse_from(["ctx", "serve", "--port", "8080", "--advertise"])
            .expect("Parse serve subcommand");
        assert_eq!(
            cli.command,
            Commands::Serve {
                port: 8080,
                advertise: true,
            }
        );

        // Connect
        let cli = Cli::try_parse_from(["ctx", "connect", "--url", "http://10.0.0.1:9900", "--discover"])
            .expect("Parse connect subcommand");
        assert_eq!(
            cli.command,
            Commands::Connect {
                url: Some("http://10.0.0.1:9900".to_string()),
                discover: true,
            }
        );

        // Find
        let cli = Cli::try_parse_from([
            "ctx", "find", "search query", "--platform", "codex", "--recent", "10",
        ])
        .expect("Parse find subcommand");
        assert_eq!(
            cli.command,
            Commands::Find {
                query: "search query".to_string(),
                platform: Some("codex".to_string()),
                recent: Some(10),
            }
        );

        // Sync Enable / Disable / Status / Now
        let cli = Cli::try_parse_from(["ctx", "sync", "enable", "frontend"])
            .expect("Parse sync enable");
        assert_eq!(
            cli.command,
            Commands::Sync(SyncCmd::Enable {
                project: "frontend".to_string()
            })
        );

        let cli = Cli::try_parse_from(["ctx", "sync", "status"]).expect("Parse sync status");
        assert_eq!(cli.command, Commands::Sync(SyncCmd::Status));

        let cli = Cli::try_parse_from(["ctx", "sync", "now"]).expect("Parse sync now");
        assert_eq!(cli.command, Commands::Sync(SyncCmd::Now));

        // Secrets Setup / Add / List / Check
        let cli = Cli::try_parse_from(["ctx", "secrets", "setup"]).expect("Parse secrets setup");
        assert_eq!(cli.command, Commands::Secrets(SecretsCmd::Setup));

        let cli = Cli::try_parse_from(["ctx", "secrets", "add", "OPENAI_API_KEY"])
            .expect("Parse secrets add");
        assert_eq!(
            cli.command,
            Commands::Secrets(SecretsCmd::Add {
                name: "OPENAI_API_KEY".to_string()
            })
        );

        let cli = Cli::try_parse_from(["ctx", "secrets", "list"]).expect("Parse secrets list");
        assert_eq!(cli.command, Commands::Secrets(SecretsCmd::List));

        let cli = Cli::try_parse_from(["ctx", "secrets", "check"]).expect("Parse secrets check");
        assert_eq!(cli.command, Commands::Secrets(SecretsCmd::Check));

        // Env
        let cli = Cli::try_parse_from(["ctx", "env", "--wrap", "cargo test"])
            .expect("Parse env subcommand");
        assert_eq!(
            cli.command,
            Commands::Env {
                wrap: Some("cargo test".to_string())
            }
        );

        // Projects
        let cli = Cli::try_parse_from(["ctx", "projects"]).expect("Parse projects subcommand");
        assert_eq!(cli.command, Commands::Projects);

        // Machines
        let cli = Cli::try_parse_from(["ctx", "machines"]).expect("Parse machines subcommand");
        assert_eq!(cli.command, Commands::Machines);

        // Config
        let cli = Cli::try_parse_from(["ctx", "config", "sync_mode", "auto"])
            .expect("Parse config subcommand");
        assert_eq!(
            cli.command,
            Commands::Config {
                key: "sync_mode".to_string(),
                value: "auto".to_string(),
            }
        );
    }

    #[test]
    fn test_cli_serde_roundtrip() {
        let cmd = Commands::Doctor;
        let json = serde_json::to_string(&cmd).expect("Serialize command");
        let deser: Commands = serde_json::from_str(&json).expect("Deserialize command");
        assert_eq!(cmd, deser);
    }

    #[tokio::test]
    async fn test_dispatch_doctor() {
        let config = GlobalConfig::default();
        let res = dispatch(Commands::Doctor, &config).await;
        assert!(res.is_ok());
    }
}
