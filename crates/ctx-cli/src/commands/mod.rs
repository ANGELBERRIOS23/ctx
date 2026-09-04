//! Command implementations and handlers for the `ctx` CLI.
//!
//! Re-exports all individual command modules and their entry functions.

pub mod claim;
pub mod config;
pub mod connect;
pub mod doctor;
pub mod env_cmd;
pub mod find;
pub mod init;
pub mod login;
pub mod logout;
pub mod machines;
pub mod projects;
pub mod pull;
pub mod push;
pub mod release;
pub mod resume;
pub mod save;
pub mod secrets;
pub mod serve;
pub mod status;
pub mod sync_cmd;

pub use claim::claim;
pub use config::config;
pub use connect::connect;
pub use doctor::doctor;
pub use env_cmd::env_cmd;
pub use find::find;
pub use init::init;
pub use login::login;
pub use logout::logout;
pub use machines::machines;
pub use projects::projects;
pub use pull::pull;
pub use push::push;
pub use release::release;
pub use resume::resume;
pub use save::save;
pub use secrets::{secrets, SecretsCmd};
pub use serve::serve;
pub use status::status;
pub use sync_cmd::{sync_cmd, SyncCmd};

#[cfg(test)]
mod tests {
    use super::*;
    use ctx_core::config::GlobalConfig;

    #[tokio::test]
    async fn test_reexported_command_signatures() {
        let config = GlobalConfig::default();
        assert!(doctor(&config).await.is_ok());
        assert!(status(&config).await.is_ok());
    }

    #[test]
    fn test_sync_cmd_and_secrets_cmd_reexports() {
        let sync = SyncCmd::Status;
        let sec = SecretsCmd::List;
        let _ = (sync, sec);
    }
}
