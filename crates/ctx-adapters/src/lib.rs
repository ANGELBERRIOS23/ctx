//! AI coding agent adapters for `ctx`.
//!
//! Provides integration adapters for various AI coding agents (Claude Code,
//! Codex, OpenCode, Cursor, and generic agents).

pub mod adapter;
pub mod claude;
pub mod codex;
pub mod cursor;
pub mod generic;
pub mod opencode;

pub use adapter::{create_adapter, AdapterError, AgentAdapter, Result};
pub use claude::ClaudeAdapter;
pub use codex::CodexAdapter;
pub use cursor::{ComposerMessage, ComposerSession, CursorAdapter};
pub use generic::{list_all_recent, search_all_agents, GenericAdapter, SessionMatch};
pub use opencode::OpenCodeAdapter;

