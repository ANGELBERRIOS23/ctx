-- ==============================================================================
-- Migration: 001_initial.sql
-- Description: Initial schema definition for ctx-server
-- Entities: users, machines, projects, sync_snapshots, secret_refs, session_locks
-- ==============================================================================

-- Enable UUID extension for cryptographically random identifiers
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE EXTENSION IF NOT EXISTS "pgcrypto";

-- ==============================================================================
-- 1. USERS TABLE
-- Developer and operator user accounts for authentication and ownership
-- ==============================================================================
CREATE TABLE IF NOT EXISTS users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_users_email ON users(email);
CREATE INDEX IF NOT EXISTS idx_users_created_at ON users(created_at DESC);

-- ==============================================================================
-- 2. MACHINES TABLE
-- Registered nodes, laptops, workstations, and remote development environments
-- ==============================================================================
CREATE TABLE IF NOT EXISTS machines (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    name TEXT NOT NULL,
    os TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    last_seen TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_machines_user_id ON machines(user_id);
CREATE INDEX IF NOT EXISTS idx_machines_fingerprint ON machines(fingerprint);
CREATE INDEX IF NOT EXISTS idx_machines_last_seen ON machines(last_seen DESC);

-- ==============================================================================
-- 3. PROJECTS TABLE
-- Synchronized git repositories, tracking metadata, and active claims
-- ==============================================================================
CREATE TABLE IF NOT EXISTS projects (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    name TEXT NOT NULL,
    git_remote TEXT NOT NULL,
    git_branch TEXT NOT NULL,
    git_commit TEXT NOT NULL,
    active_machine UUID REFERENCES machines(id) ON DELETE SET NULL,
    active_agent TEXT,
    claimed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_projects_name ON projects(name);
CREATE INDEX IF NOT EXISTS idx_projects_user_id ON projects(user_id);
CREATE INDEX IF NOT EXISTS idx_projects_active_machine ON projects(active_machine);
CREATE INDEX IF NOT EXISTS idx_projects_claimed_at ON projects(claimed_at);

-- ==============================================================================
-- 4. SESSION LOCKS TABLE
-- Distributed mutual exclusion locks for preventing concurrent project writes
-- ==============================================================================
CREATE TABLE IF NOT EXISTS session_locks (
    project_id UUID PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    machine_id UUID NOT NULL REFERENCES machines(id) ON DELETE CASCADE,
    locked_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    heartbeat TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_session_locks_machine_id ON session_locks(machine_id);
CREATE INDEX IF NOT EXISTS idx_session_locks_heartbeat ON session_locks(heartbeat);

-- ==============================================================================
-- 5. SYNC SNAPSHOTS TABLE
-- Point-in-time handoffs, encrypted memory states, and sync metadata
-- ==============================================================================
CREATE TABLE IF NOT EXISTS sync_snapshots (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    machine_id UUID NOT NULL REFERENCES machines(id) ON DELETE CASCADE,
    snapshot_type TEXT NOT NULL,
    git_commit TEXT NOT NULL,
    handoff_blob BYTEA NOT NULL,
    memory_blob BYTEA,
    state_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_sync_snapshots_project_id ON sync_snapshots(project_id);
CREATE INDEX IF NOT EXISTS idx_sync_snapshots_project_created ON sync_snapshots(project_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_sync_snapshots_machine_id ON sync_snapshots(machine_id);
CREATE INDEX IF NOT EXISTS idx_sync_snapshots_snapshot_type ON sync_snapshots(snapshot_type);

-- ==============================================================================
-- 6. SECRET REFERENCES TABLE
-- Vault URI references and variable identifiers (never stores raw secrets)
-- ==============================================================================
CREATE TABLE IF NOT EXISTS secret_refs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    key_name TEXT NOT NULL,
    vault_uri TEXT NOT NULL,
    required BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_secret_refs_project_key UNIQUE (project_id, key_name)
);

CREATE INDEX IF NOT EXISTS idx_secret_refs_project_id ON secret_refs(project_id);
CREATE INDEX IF NOT EXISTS idx_secret_refs_key_name ON secret_refs(project_id, key_name);

-- Audit log for tracking all sync/auth activity
CREATE TABLE IF NOT EXISTS audit_log (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     uuid REFERENCES users(id),
    project_id  uuid,
    machine_name text,
    action      text NOT NULL,
    detail      text,
    ip_address  text,
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_audit_log_project ON audit_log(project_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_audit_log_user ON audit_log(user_id, created_at DESC);
