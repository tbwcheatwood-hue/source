-- Source daemon schema (Phase 1–2)
-- sessions, checkpoints, cryptographic audit logs, peer keys, tool registry, RAG index metadata

PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL,
    mode TEXT NOT NULL CHECK (mode IN ('plan', 'act')),
    workspace_root TEXT NOT NULL,
    network_tier TEXT NOT NULL,
    provider_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS checkpoints (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    step_index INTEGER NOT NULL,
    git_commit TEXT NOT NULL,
    label TEXT NOT NULL,
    created_at TEXT NOT NULL,
    audit_event_id TEXT,
    UNIQUE(session_id, step_index)
);

CREATE INDEX IF NOT EXISTS idx_checkpoints_session ON checkpoints(session_id);

CREATE TABLE IF NOT EXISTS audit_logs (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT,
    kind TEXT NOT NULL,
    summary TEXT NOT NULL,
    detail_json TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    prev_hash TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_audit_session ON audit_logs(session_id);
CREATE INDEX IF NOT EXISTS idx_audit_created ON audit_logs(created_at);

CREATE TABLE IF NOT EXISTS peer_keys (
    peer_id TEXT PRIMARY KEY NOT NULL,
    public_key TEXT NOT NULL,
    label TEXT,
    trusted INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS tool_registry (
    name TEXT PRIMARY KEY NOT NULL,
    description TEXT NOT NULL,
    input_schema_json TEXT NOT NULL,
    categories_json TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS rag_index_meta (
    id TEXT PRIMARY KEY NOT NULL,
    workspace_root TEXT NOT NULL,
    index_path TEXT NOT NULL,
    embedding_model TEXT,
    chunk_count INTEGER NOT NULL DEFAULT 0,
    file_count INTEGER NOT NULL DEFAULT 0,
    last_built_at TEXT,
    status TEXT NOT NULL DEFAULT 'idle',
    meta_json TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_rag_workspace ON rag_index_meta(workspace_root);

CREATE TABLE IF NOT EXISTS providers (
    id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL,
    display_name TEXT NOT NULL,
    base_url TEXT NOT NULL,
    model TEXT NOT NULL,
    api_key_env TEXT,
    enabled INTEGER NOT NULL DEFAULT 0,
    extra_headers_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS approvals (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT,
    category TEXT NOT NULL,
    summary TEXT NOT NULL,
    detail_json TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'approved', 'denied', 'deferred')),
    created_at TEXT NOT NULL,
    resolved_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_approvals_status ON approvals(status);

CREATE TABLE IF NOT EXISTS network_policy (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    tier TEXT NOT NULL,
    allowlist_json TEXT NOT NULL DEFAULT '[]',
    denylist_json TEXT NOT NULL DEFAULT '[]',
    require_approval_for_public INTEGER NOT NULL DEFAULT 1,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS kv_state (
    key TEXT PRIMARY KEY NOT NULL,
    value_json TEXT NOT NULL,
    updated_at TEXT NOT NULL
);