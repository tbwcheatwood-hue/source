ALTER TABLE tool_registry ADD COLUMN offline_capable INTEGER NOT NULL DEFAULT 1;
ALTER TABLE tool_registry ADD COLUMN requires_wan INTEGER NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS rag_chunks (
    id TEXT PRIMARY KEY NOT NULL,
    source_path TEXT NOT NULL,
    chunk_index INTEGER NOT NULL,
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    embedding BLOB NOT NULL,
    token_estimate INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    UNIQUE(source_path, chunk_index)
);

CREATE INDEX IF NOT EXISTS idx_rag_chunks_path ON rag_chunks(source_path);
CREATE INDEX IF NOT EXISTS idx_rag_chunks_hash ON rag_chunks(content_hash);

CREATE TABLE IF NOT EXISTS mcp_extensions (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    transport TEXT NOT NULL CHECK (transport IN ('stdio', 'sse', 'streamable_http')),
    command_json TEXT,
    url TEXT,
    offline_capable INTEGER NOT NULL DEFAULT 1,
    requires_wan INTEGER NOT NULL DEFAULT 0,
    enabled INTEGER NOT NULL DEFAULT 1,
    manifest_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL
);
