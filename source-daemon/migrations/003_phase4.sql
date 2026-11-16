-- Phase 4: mesh peer keys extensions + remote task tracking + node identity

ALTER TABLE peer_keys ADD COLUMN address TEXT;
ALTER TABLE peer_keys ADD COLUMN port INTEGER;
ALTER TABLE peer_keys ADD COLUMN fingerprint TEXT;
ALTER TABLE peer_keys ADD COLUMN source TEXT NOT NULL DEFAULT 'manual';
ALTER TABLE peer_keys ADD COLUMN role TEXT NOT NULL DEFAULT 'hybrid';
ALTER TABLE peer_keys ADD COLUMN capabilities_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE peer_keys ADD COLUMN last_seen_at TEXT;
ALTER TABLE peer_keys ADD COLUMN paired_at TEXT;
ALTER TABLE peer_keys ADD COLUMN session_secret TEXT;

CREATE TABLE IF NOT EXISTS mesh_tasks (
    id TEXT PRIMARY KEY NOT NULL,
    parent_id TEXT,
    kind TEXT NOT NULL,
    label TEXT NOT NULL,
    state TEXT NOT NULL,
    assigned_peer_id TEXT,
    required_model TEXT,
    min_vram_mb INTEGER,
    payload_json TEXT NOT NULL DEFAULT '{}',
    result_json TEXT,
    error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    completed_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_mesh_tasks_state ON mesh_tasks(state);
CREATE INDEX IF NOT EXISTS idx_mesh_tasks_peer ON mesh_tasks(assigned_peer_id);

CREATE TABLE IF NOT EXISTS node_identity (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    node_id TEXT NOT NULL,
    public_key_hex TEXT NOT NULL,
    private_key_hex TEXT NOT NULL,
    label TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'hybrid',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
