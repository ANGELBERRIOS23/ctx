# ctx

[![Rust](https://img.shields.io/badge/rust-1.85%2B-blue.svg)](https://www.rust-lang.org)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![CI](https://img.shields.io/badge/CI-passing-brightgreen.svg)](https://github.com/example-org/ctx/actions)

Cross-OS CLI and server that synchronizes development projects, AI agent context, and secrets across machines.

---

## Features

- Cross-OS Synchronization: Native support for macOS, Linux, and Windows with path abstraction and cross-platform state portability.
- Dual Network Modes: Cloud mode using a centralized VPS with Docker Compose, or Direct mode using peer-to-peer LAN discovery via mDNS.
- Dual Sync Modes: Auto mode for background continuous synchronization, or Selective mode for explicit per-project control.
- Vault-Agnostic Secret Management: Resolves credentials dynamically at runtime across Bitwarden, 1Password, HashiCorp Vault, AWS Secrets Manager, SOPS, and environment variables. Secrets are never stored in plaintext or synced unencrypted.
- Multi-Agent Context Continuity: Built-in adapters for Claude Code, Codex, OpenCode, Cursor, Antigravity, Command Code, and generic LLM tools to persist and transfer task states, conversation summaries, and pending objectives.
- Universal Session Search (`ctx find`): Search and query historical transcripts, decisions, and instructions across all local and synced agent sessions.
- Distributed Session Locking: Prevents conflicting edits and race conditions across multiple active development workstations.
- End-to-End Cryptography: Client-side payload encryption using age (X25519), Argon2id key derivation, and SHA-256 checksum verification.

---

## Quick Start

### 1. Install the CLI

#### From Source

Ensure Rust 1.85 or newer is installed:

```bash
cargo install --path crates/ctx-cli
```

Verify the installation:

```bash
ctx --version
ctx doctor
```

### 2. Set Up a Server

#### Option A: Direct / P2P Mode (Local Machine or LAN)

Start an embedded local server on the primary machine:

```bash
ctx serve --port 9900 --advertise
```

On a secondary machine on the same LAN, connect automatically:

```bash
ctx connect --discover
```

Or connect directly by IP or hostname:

```bash
ctx connect --url http://192.168.1.50:9900
```

#### Option B: Cloud Mode (Self-Hosted VPS)

Connect to an existing self-hosted cloud instance:

```bash
ctx connect --url https://ctx.example.com
ctx login
```

### 3. Initialize Your First Project

Navigate to an existing code repository on your primary workstation:

```bash
cd ~/projects/my-api
ctx init my-api
```

Configure a secret reference for the project:

```bash
ctx secrets add DATABASE_URL
```

Acquire a session lock and push initial project state and agent context:

```bash
ctx claim
ctx push
```

On your secondary machine, pull the project and resume your AI coding session:

```bash
ctx pull --project my-api
cd ~/projects/my-api
ctx claim
ctx resume --agent claude
```

---

## Commands

| Command | Description | Example |
| :--- | :--- | :--- |
| `ctx init <name>` | Initialize a `.ctx` configuration in the current directory | `ctx init my-api` |
| `ctx status` | Display synchronization status, active locks, and tracked state | `ctx status` |
| `ctx pull` | Pull project snapshot from the server (`--all` for all projects) | `ctx pull --project my-api` |
| `ctx push` | Push local state and handoff snapshots to the server | `ctx push --all` |
| `ctx claim` | Acquire exclusive session lock for a project | `ctx claim --project my-api` |
| `ctx release` | Release active session lock for a project | `ctx release --project my-api` |
| `ctx resume` | Restore AI agent instructions and state from latest handoff | `ctx resume --agent cursor` |
| `ctx save` | Snapshot current agent state and generate handoff data | `ctx save -m "Refactored auth"` |
| `ctx find <query>` | Search sessions and transcripts across all AI agent tools | `ctx find "sql migration" --recent 5` |
| `ctx secrets setup` | Configure the secret vault provider backend | `ctx secrets setup` |
| `ctx secrets add <key>` | Track a new secret reference in the project configuration | `ctx secrets add STRIPE_KEY` |
| `ctx secrets list` | List all configured secret references for current project | `ctx secrets list` |
| `ctx secrets check` | Validate connectivity and resolution against the vault | `ctx secrets check` |
| `ctx env` | Execute a command with decrypted secrets injected into environment | `ctx env --wrap "cargo test"` |
| `ctx sync enable <proj>` | Enable synchronization for a specific project | `ctx sync enable project-1` |
| `ctx sync disable <proj>` | Exclude a project from synchronization | `ctx sync disable project-1` |
| `ctx sync status` | View background synchronization status and daemon intervals | `ctx sync status` |
| `ctx sync now` | Trigger an immediate manual synchronization cycle | `ctx sync now` |
| `ctx serve` | Start local embedded server with SQLite and local storage | `ctx serve --port 9900 --advertise` |
| `ctx connect` | Connect to remote server or discover LAN instances via mDNS | `ctx connect --discover` |
| `ctx login` | Authenticate with the server and store credentials in keychain | `ctx login` |
| `ctx logout` | Invalidate credentials and remove auth tokens from keychain | `ctx logout` |
| `ctx projects` | List all tracked development projects | `ctx projects` |
| `ctx machines` | List all registered machines in the synchronization network | `ctx machines` |
| `ctx config <k> <v>` | Set global configuration values in `~/.ctx/config.yaml` | `ctx config sync_mode selective` |
| `ctx doctor` | Check environment, dependencies, and system health | `ctx doctor` |

---

## Network Modes

`ctx` supports two network modes tailored for different infrastructure constraints:

| Capability | Cloud Mode (VPS) | Direct Mode (P2P / LAN) |
| :--- | :--- | :--- |
| Topology | Centralized client-server | Peer-to-peer or local embedded server |
| Target Environment | VPS, dedicated servers, cloud VMs | Local network, home lab, Tailscale, air-gapped |
| Metadata Backend | PostgreSQL | SQLite (embedded, zero configuration) |
| Snapshot Blob Storage | MinIO or AWS S3 | Local filesystem directory |
| Service Discovery | DNS / static public URL | mDNS (`_ctx._tcp.local`) broadcast |
| Authentication | JWT with Argon2 password verification | Local machine pairing and pre-shared keys |
| Connectivity | Always accessible across WAN/Internet | Accessible on local subnet or mesh VPN |

### Cloud Mode

Cloud mode connects multiple workstations across the internet through a centralized `ctx-server` instance. Metadata and locks are coordinated in PostgreSQL, while encrypted snapshot archives are stored in S3/MinIO.

```bash
ctx connect --url https://ctx.example.com
ctx login
```

### Direct Mode

Direct mode requires no cloud infrastructure. A developer workstation runs an embedded server, advertising itself on the local network via mDNS. Secondary workstations discover and connect to the host automatically.

```bash
# On primary workstation (Host)
ctx serve --port 9900 --advertise

# On secondary workstation (Peer)
ctx connect --discover
```

---

## Sync Modes

Global synchronization behavior is configured in `~/.ctx/config.yaml` via `sync_mode`:

```yaml
sync_mode: auto
interval: 300
auto_save_on_agent_exit: true
```

### Auto Mode (`sync_mode: auto`)

- Continuous Synchronization: The background daemon monitors file changes, git state, and agent handoffs across all projects listed in `~/.ctx/config.yaml`.
- Periodic Sync: Snapshots are pushed and pulled automatically at the configured interval (default: 300 seconds).
- Agent Exit Hooks: When `auto_save_on_agent_exit` is set to `true`, the daemon automatically captures handoff state when an agent process exits.

### Selective Mode (`sync_mode: selective`)

- Granular Control: Only projects explicitly enabled using `ctx sync enable <project>` participate in synchronization cycles.
- Low Bandwidth / High Isolation: Ideal for environments where specific large repositories or sensitive client projects must remain strictly local.
- Manual Triggering: Pulls and pushes can be triggered on demand via `ctx pull`, `ctx push`, and `ctx sync now`.

---

## Secrets Setup

`ctx` enforces zero-plaintext secret storage. Secret references are stored in `.ctx/config.yaml`, while plaintext values remain exclusively inside external vaults and are resolved in memory at process execution time.

### Supported Vault Backends

- Bitwarden (`bw` CLI and self-hosted Vaultwarden instances)
- 1Password (`op` CLI)
- HashiCorp Vault (CLI and HTTP API)
- AWS Secrets Manager
- Mozilla SOPS (encrypted files)
- Manual (environment variable fallback)

### Configuration Example

A project `.ctx/config.yaml` declares references to secrets:

```yaml
secrets:
  provider: bitwarden
  refs:
    DATABASE_URL: "vault://production/db-primary"
    STRIPE_SECRET_KEY: "vault://billing/stripe-api"
    OPENAI_API_KEY: "bw://items/openai-key"
```

### Managing Secrets

```bash
# Configure the provider for the current project
ctx secrets setup

# Add a reference (never prompts for or stores plaintext)
ctx secrets add DATABASE_URL

# Validate that all references can be resolved from the vault
ctx secrets check

# Run a process with resolved secrets injected into its environment
ctx env --wrap "cargo test"
ctx env --wrap "npm run start"
```

---

## Agent Adapters

`ctx-adapters` bridges AI coding assistants by standardizing session history, prompts, context files, and task completion records into a unified `Handoff` model.

### Supported Agents

| Agent | Instruction File | Context Extraction Source |
| :--- | :--- | :--- |
| Claude Code | `CLAUDE.md` | `~/.claude/sessions` and transcript logs |
| OpenAI Codex | `CODEX.md` | Session rollouts and task logs |
| OpenCode | `OPENCODE.md` | OpenCode workspace cache and registry |
| Cursor | `.cursorrules` / `CURSOR.md` | Cursor Composer SQLite database |
| Antigravity | `ANTIGRAVITY.md` | Trajectory traces and task artifacts |
| Command Code | `COMMANDCODE.md` | Terminal session buffers and state |
| Generic | `CTX_HANDOFF.md` | Standard markdown format for any LLM CLI |

### Lifecycle Workflow

1. Work with an AI agent (for example, Claude Code or Cursor).
2. Save progress:
   ```bash
   ctx save -m "Completed database schema migration and added user models"
   ```
3. Push state to the sync server:
   ```bash
   ctx push
   ```
4. Switch to another computer, pull state, and resume:
   ```bash
   ctx pull
   ctx resume --agent cursor
   ```
   `ctx` automatically extracts the previous session summary, remaining tasks, and technical decisions, generating the appropriate instruction file for the target agent.

---

## Universal Session Search (`ctx find`)

Search across conversation history, prompt runs, and transcripts generated by any supported AI coding agent on any connected machine.

```bash
# Search across all agent sessions for keyword matches
ctx find "jwt authentication"

# Scope search to a specific agent platform
ctx find "postgres migration" --platform claude

# Limit search results to recent sessions
ctx find "docker compose" --recent 5
```

Search output includes the originating agent platform, session identifier, associated project, and relevant snippet matches.

---

## Architecture

```text
+-----------------------------------------------------------------------+
|                           Workstation A                               |
|                                                                       |
|  +--------------------+   +-------------------+   +----------------+  |
|  |    Claude Code     |   |      Cursor       |   |  Antigravity   |  |
|  +---------+----------+   +---------+---------+   +-------+--------+  |
|            |                        |                     |           |
|            +------------------------+---------------------+           |
|                                     |                                 |
|                        +------------v-----------+                     |
|                        |      ctx-adapters      |                     |
|                        +------------+-----------+                     |
|                                     |                                 |
|                        +------------v-----------+                     |
|                        |        ctx-cli         |                     |
|                        +------------+-----------+                     |
|                                     |                                 |
|  +----------------------------------v------------------------------+  |
|  |                             ctx-core                            |  |
|  |  +-----------------+  +-----------------+  +-----------------+  |  |
|  |  | Crypto (age/X25519) | State Machine  |  | Vault Resolver  |  |  |
|  |  +-----------------+  +-----------------+  +--------+--------+  |  |
|  +-----------------------------------------------------|-----------+  |
+--------------------------------------------------------|--------------+
                       |                                 |
                       | (Encrypted Sync Payload)        | (Secret References)
                       |                                 v
                       |                    +------------------------+
                       |                    | Secret Vault Providers |
                       |                    | (Bitwarden, 1Password, |
                       |                    |  HashiCorp, SOPS, AWS) |
                       |                    +------------------------+
        +--------------+--------------+
        |                             |
        v                             v
+---------------------------+   +---------------------------+
|        Cloud Mode         |   |        Direct Mode        |
|       (Central VPS)       |   |       (Peer-to-Peer)      |
|                           |   |                           |
|  +---------------------+  |   |  +---------------------+  |
|  |     ctx-server      |  |   |  | ctx-server embedded |  |
|  |      (axum API)     |  |   |  |     (axum / LAN)    |  |
|  +----------+----------+  |   |  +----------+----------+  |
|             |             |   |             |             |
|      +------+------+      |   |      +------+------+      |
|      |             |      |   |      |             |      |
|      v             v      |   |      v             v      |
| +----------+ +----------+ |   | +----------+ +----------+ |
| |PostgreSQL| | MinIO/S3 | |   | |  SQLite  | |Filesystem| |
| |(Metadata)| | (Blobs)  | |   | |(Metadata)| | (Blobs)  | |
| +----------+ +----------+ |   | +----------+ +----------+ |
+---------------------------+   +---------------------------+
```

---

## Security Model

- End-to-End Payload Encryption: All project snapshots, state summaries, and handoffs are encrypted client-side using age (X25519 identities) prior to transit. The server stores ciphertext and cannot inspect context data.
- Zero-Knowledge Secret Transport: Secrets are never sent to the sync server. Only provider URIs (such as `vault://items/12345`) are synchronized. Each machine resolves secrets locally from its own authenticated vault session.
- Secure Key Derivation: Passwords and master keys use Argon2id with salt generation compliant with modern cryptographic standards.
- Tamper-Evident Hashing: Every snapshot payload includes a SHA-256 digest to verify integrity before unpacking on a destination machine.
- Safe Execution Wrapper: `ctx env` injects secrets into the target process memory without writing intermediate files or unencrypted `.env` files to the filesystem.
- Distributed Session Locks: Mutual exclusion locks prevent simultaneous write conflicts when multiple machines track the same project repository.

---

## Self-Hosting Guide

Deploy a centralized `ctx-server` instance using Docker Compose.

### 1. Create `docker-compose.yml`

```yaml
services:
  ctx-server:
    image: ghcr.io/example-org/ctx-server:latest
    container_name: ctx-server
    restart: unless-stopped
    ports:
      - "9900:9900"
    environment:
      PORT: "9900"
      DATABASE_URL: "postgres://ctx_user:ctx_password@postgres:5432/ctx_db"
      S3_ENDPOINT: "http://minio:9000"
      S3_BUCKET: "ctx-snapshots"
      AWS_ACCESS_KEY_ID: "minio_admin"
      AWS_SECRET_ACCESS_KEY: "minio_strong_password"
      JWT_SECRET: "replace_with_a_secure_random_64_char_secret_key"
    depends_on:
      - postgres
      - minio

  postgres:
    image: postgres:16-alpine
    container_name: ctx-postgres
    restart: unless-stopped
    environment:
      POSTGRES_USER: "ctx_user"
      POSTGRES_PASSWORD: "ctx_password"
      POSTGRES_DB: "ctx_db"
    volumes:
      - postgres_data:/var/lib/postgresql/data

  minio:
    image: minio/minio:latest
    container_name: ctx-minio
    restart: unless-stopped
    command: server /data --console-address ":9001"
    environment:
      MINIO_ROOT_USER: "minio_admin"
      MINIO_ROOT_PASSWORD: "minio_strong_password"
    volumes:
      - minio_data:/data

volumes:
  postgres_data:
  minio_data:
```

### 2. Start Services

```bash
docker compose up -d
```

### 3. Verify Server Status

```bash
curl http://localhost:9900/health
```

### 4. Connect Clients

```bash
ctx connect --url http://your-vps-ip:9900
ctx login
```

---

## Contributing

Contributions to `ctx` are welcome. Please adhere to the following workflow:

1. Requirements:
   - Rust 1.85+ (2024 edition)
   - Docker (for database integration tests)
2. Setup and Validation:
   ```bash
   git clone https://github.com/example-org/ctx.git
   cd ctx
   cargo check --workspace
   cargo test --workspace
   cargo clippy --workspace -- -D warnings
   cargo fmt --check
   ```
3. Architecture Rules:
   - All shared dependencies must be declared in the root `Cargo.toml`.
   - Library code in `ctx-core` and `ctx-adapters` must use typed errors via `thiserror`. Do not use `.unwrap()` in library code.
   - All public functions, structs, and traits require complete Rustdoc documentation.
   - Unit tests must be placed in the corresponding source file using `#[cfg(test)] mod tests`.

---

## License

This project is licensed under the Apache License, Version 2.0. See the [LICENSE](LICENSE) file for details.
