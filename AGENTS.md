# AGENTS.md — Instructions for AI coding agents working on ctx

## Project: ctx

A cross-OS CLI + server that syncs development projects, AI agent context, and
secrets across machines. Two network modes: cloud (VPS) and direct (P2P/LAN).
Two sync modes: auto (all projects) and selective (per-project).

## Architecture

Rust workspace with 4 crates:
- `ctx-core`: shared library (config, crypto, handoff model, protocol types, state machine, vault abstraction)
- `ctx-server`: API server (axum) with auth, sync engine, session locks. Supports PostgreSQL+MinIO (cloud) and SQLite+filesystem (embedded/P2P)
- `ctx-cli`: CLI binary with commands, background daemon, mDNS discovery
- `ctx-adapters`: agent adapters (Claude Code, Codex, OpenCode, Cursor, generic)

## Rules

1. **Write idiomatic Rust.** Use `thiserror` for library errors, `anyhow` for binaries. Derive `serde::Serialize`/`Deserialize` on all data types.
2. **Every public function has a doc comment.** No exceptions.
3. **Use workspace dependencies.** Never add a dep to a crate Cargo.toml that is not in the workspace Cargo.toml.
4. **Errors must be typed.** No `.unwrap()` in library code. `.expect()` only with a message explaining the invariant.
5. **Tests go in the same file** as `#[cfg(test)] mod tests { ... }` for unit tests. Integration tests go in `tests/` directory.
6. **No placeholder code.** Every function must have a real implementation or return `todo!()` with a clear message of what it will do.
7. **Security:** Never store secrets in plaintext. Never log secret values. Encrypt before network transit.
8. **Cross-OS:** Use `std::path::PathBuf`, `dirs` crate for home dir, `cfg(target_os)` when needed. No hardcoded `/` paths.
9. **File names use snake_case.** Module structure matches the directory layout described above.

## Dependencies

All dependencies are declared in the workspace root `Cargo.toml`. Use them as:
```toml
[dependencies]
tokio = { workspace = true }
serde = { workspace = true }
```

## Key data types (defined in ctx-core)

- `Handoff`: project state for agent handoff (summary, completed/pending tasks, decisions, blockers)
- `ProjectConfig`: parsed from `.ctx/config.yaml`
- `SyncSnapshot`: encrypted package of handoff + memory + state for transit
- `SecretRef`: reference to a vault entry (never the value)
- `SessionLock`: which machine is active on which project

## When generating files

- Write the FULL file content. Do not use ellipsis or "rest of implementation".
- Include all `use` imports at the top.
- Include `#[cfg(test)] mod tests` with at least 2 unit tests per file.
- Make sure `cargo check` would pass (correct types, lifetimes, trait bounds).
