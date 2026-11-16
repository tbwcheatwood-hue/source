//! Offline RAG: auto-index `~/.source/offline-docs/` with SQLite vectors + Tantivy BM25 hybrid.
//! Local embedding path uses deterministic feature-hashing (no WAN / no model download).

use anyhow::{Context, Result};
use chrono::Utc;
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{Schema, STORED, TEXT, Value};
use tantivy::{doc, Index, IndexWriter, ReloadPolicy, TantivyDocument};
use uuid::Uuid;

pub const EMBED_DIM: usize = 384;
pub const EMBEDDING_MODEL: &str = "source-hash-v1";

#[derive(Clone)]
pub struct OfflineRag {
    docs_root: PathBuf,
    rag_dir: PathBuf,
    pool: SqlitePool,
    index: Arc<parking_lot::RwLock<Option<IndexHandle>>>,
}

struct IndexHandle {
    index: Index,
    schema: Schema,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagStatus {
    pub docs_root: PathBuf,
    pub index_path: PathBuf,
    pub embedding_model: String,
    pub chunk_count: i64,
    pub file_count: i64,
    pub status: String,
    pub last_built_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagHit {
    pub source_path: String,
    pub chunk_index: i64,
    pub content: String,
    pub score: f32,
    pub bm25_score: f32,
    pub vector_score: f32,
}

impl OfflineRag {
    pub async fn open(docs_root: PathBuf, rag_dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&docs_root)?;
        fs::create_dir_all(&rag_dir)?;
        let db_path = rag_dir.join("rag-vectors.db");
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", db_path.display()))?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal);
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .with_context(|| format!("open rag db {}", db_path.display()))?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS rag_chunks (
                id TEXT PRIMARY KEY NOT NULL,
                source_path TEXT NOT NULL,
                chunk_index INTEGER NOT NULL,
                content TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                embedding BLOB NOT NULL,
                token_estimate INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                UNIQUE(source_path, chunk_index)
            )",
        )
        .execute(&pool)
        .await?;

        let this = Self {
            docs_root,
            rag_dir,
            pool,
            index: Arc::new(parking_lot::RwLock::new(None)),
        };
        // Best-effort open existing tantivy index.
        let _ = this.open_tantivy();
        Ok(this)
    }

    pub fn docs_root(&self) -> &Path {
        &self.docs_root
    }

    fn tantivy_path(&self) -> PathBuf {
        self.rag_dir.join("tantivy")
    }

    fn build_schema() -> Schema {
        let mut sb = Schema::builder();
        sb.add_text_field("id", TEXT | STORED);
        sb.add_text_field("path", TEXT | STORED);
        sb.add_text_field("body", TEXT | STORED);
        sb.add_i64_field("chunk_index", tantivy::schema::INDEXED | STORED);
        sb.build()
    }

    fn open_tantivy(&self) -> Result<()> {
        let path = self.tantivy_path();
        if path.exists() {
            let index = Index::open_in_dir(&path)?;
            let schema = index.schema();
            *self.index.write() = Some(IndexHandle { index, schema });
        }
        Ok(())
    }

    pub async fn status(&self) -> Result<RagStatus> {
        let row = sqlx::query("SELECT COUNT(*) AS c, COUNT(DISTINCT source_path) AS f FROM rag_chunks")
            .fetch_one(&self.pool)
            .await?;
        let chunk_count: i64 = row.try_get("c")?;
        let file_count: i64 = row.try_get("f")?;
        let last: Option<String> = sqlx::query("SELECT created_at FROM rag_chunks ORDER BY created_at DESC LIMIT 1")
            .fetch_optional(&self.pool)
            .await?
            .map(|r| r.try_get("created_at"))
            .transpose()?;
        Ok(RagStatus {
            docs_root: self.docs_root.clone(),
            index_path: self.rag_dir.clone(),
            embedding_model: EMBEDDING_MODEL.into(),
            chunk_count,
            file_count,
            status: if chunk_count > 0 { "ready".into() } else { "idle".into() },
            last_built_at: last,
        })
    }

    /// Ingest MD / PDF / docsets under docs_root (or override) into SQLite vectors + Tantivy.
    pub async fn index_docs(&self, docs_root: Option<PathBuf>, force: bool) -> Result<RagStatus> {
        let root = docs_root.unwrap_or_else(|| self.docs_root.clone());
        fs::create_dir_all(&root)?;

        if force {
            sqlx::query("DELETE FROM rag_chunks").execute(&self.pool).await?;
            let tp = self.tantivy_path();
            if tp.exists() {
                fs::remove_dir_all(&tp)?;
            }
        }

        let schema = Self::build_schema();
        let tantivy_path = self.tantivy_path();
        fs::create_dir_all(&tantivy_path)?;
        let meta_path = tantivy_path.join("meta.json");
        let index = if meta_path.exists() && !force {
            Index::open_in_dir(&tantivy_path)?
        } else {
            if force && tantivy_path.exists() {
                let _ = fs::remove_dir_all(&tantivy_path);
                fs::create_dir_all(&tantivy_path)?;
            }
            Index::create_in_dir(&tantivy_path, schema.clone())?
        };
        let mut writer: IndexWriter = index.writer(50_000_000)?;
        let id_f = schema.get_field("id").unwrap();
        let path_f = schema.get_field("path").unwrap();
        let body_f = schema.get_field("body").unwrap();
        let chunk_f = schema.get_field("chunk_index").unwrap();

        let walker = WalkBuilder::new(&root)
            .hidden(false)
            .git_ignore(true)
            .build();

        for entry in walker.flatten() {
            let path = entry.path();
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let Some(text) = extract_document_text(path) else {
                continue;
            };
            if text.trim().is_empty() {
                continue;
            }
            let rel = path
                .strip_prefix(&root)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();
            let chunks = chunk_text(&text, 800, 120);
            for (i, chunk) in chunks.into_iter().enumerate() {
                let hash = hex::encode(Sha256::digest(chunk.as_bytes()));
                if !force {
                    let exists = sqlx::query(
                        "SELECT id FROM rag_chunks WHERE source_path = ? AND chunk_index = ? AND content_hash = ?",
                    )
                    .bind(&rel)
                    .bind(i as i64)
                    .bind(&hash)
                    .fetch_optional(&self.pool)
                    .await?;
                    if exists.is_some() {
                        continue;
                    }
                }
                let emb = embed_text(&chunk);
                let emb_bytes = f32_slice_to_bytes(&emb);
                let id = Uuid::new_v4().to_string();
                sqlx::query(
                    "INSERT INTO rag_chunks (id, source_path, chunk_index, content, content_hash, embedding, token_estimate, created_at)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                     ON CONFLICT(source_path, chunk_index) DO UPDATE SET
                       content=excluded.content,
                       content_hash=excluded.content_hash,
                       embedding=excluded.embedding,
                       token_estimate=excluded.token_estimate,
                       created_at=excluded.created_at,
                       id=excluded.id",
                )
                .bind(&id)
                .bind(&rel)
                .bind(i as i64)
                .bind(&chunk)
                .bind(&hash)
                .bind(&emb_bytes)
                .bind(((chunk.chars().count() + 3) / 4) as i64)
                .bind(Utc::now().to_rfc3339())
                .execute(&self.pool)
                .await?;

                writer.add_document(doc!(
                    id_f => id.as_str(),
                    path_f => rel.as_str(),
                    body_f => chunk.as_str(),
                    chunk_f => i as i64
                ))?;
            }
        }

        writer.commit()?;
        *self.index.write() = Some(IndexHandle { index, schema });
        self.status().await
    }

    pub async fn query(
        &self,
        query: &str,
        top_k: usize,
        bm25_weight: f32,
        vector_weight: f32,
    ) -> Result<Vec<RagHit>> {
        if query.trim().is_empty() {
            return Ok(vec![]);
        }
        let top_k = top_k.max(1);
        let q_emb = embed_text(query);

        // Vector candidates from SQLite (full scan — fine for local offline corpora).
        let rows = sqlx::query(
            "SELECT source_path, chunk_index, content, embedding FROM rag_chunks",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut vector_scores: HashMap<(String, i64), (f32, String)> = HashMap::new();
        for row in rows {
            let path: String = row.try_get("source_path")?;
            let idx: i64 = row.try_get("chunk_index")?;
            let content: String = row.try_get("content")?;
            let blob: Vec<u8> = row.try_get("embedding")?;
            let emb = bytes_to_f32_vec(&blob);
            let score = cosine_similarity(&q_emb, &emb);
            vector_scores.insert((path, idx), (score, content));
        }

        // BM25 via Tantivy
        let mut bm25_scores: HashMap<(String, i64), f32> = HashMap::new();
        if let Some(handle) = self.index.read().as_ref() {
            let reader = handle
                .index
                .reader_builder()
                .reload_policy(ReloadPolicy::Manual)
                .try_into()?;
            let searcher = reader.searcher();
            let body = handle.schema.get_field("body").unwrap();
            let path_f = handle.schema.get_field("path").unwrap();
            let chunk_f = handle.schema.get_field("chunk_index").unwrap();
            let parser = QueryParser::for_index(&handle.index, vec![body]);
            if let Ok(tq) = parser.parse_query(query) {
                let top = searcher.search(&tq, &TopDocs::with_limit(top_k * 4))?;
                for (score, addr) in top {
                    let doc: TantivyDocument = searcher.doc(addr)?;
                    let path = doc
                        .get_first(path_f)
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let idx = doc
                        .get_first(chunk_f)
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    bm25_scores.insert((path, idx), score);
                }
            }
        }

        let mut keys: Vec<(String, i64)> = vector_scores.keys().cloned().collect();
        for k in bm25_scores.keys() {
            if !keys.contains(k) {
                keys.push(k.clone());
            }
        }

        // Normalize BM25 roughly into 0..1
        let max_bm25 = bm25_scores.values().cloned().fold(0.0_f32, f32::max).max(1e-6);

        let mut hits = Vec::new();
        for key in keys {
            let (vscore, content) = vector_scores
                .get(&key)
                .cloned()
                .unwrap_or((0.0, String::new()));
            let bscore = bm25_scores.get(&key).copied().unwrap_or(0.0) / max_bm25;
            let score = bm25_weight * bscore + vector_weight * vscore;
            if score <= 0.0 && content.is_empty() {
                continue;
            }
            let content = if content.is_empty() {
                // fetch from db if only BM25 hit
                sqlx::query("SELECT content FROM rag_chunks WHERE source_path = ? AND chunk_index = ?")
                    .bind(&key.0)
                    .bind(key.1)
                    .fetch_optional(&self.pool)
                    .await?
                    .map(|r| r.try_get::<String, _>("content"))
                    .transpose()?
                    .unwrap_or_default()
            } else {
                content
            };
            hits.push(RagHit {
                source_path: key.0,
                chunk_index: key.1,
                content,
                score,
                bm25_score: bscore,
                vector_score: vscore,
            });
        }
        hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        hits.truncate(top_k);
        Ok(hits)
    }
}

pub fn embed_text(text: &str) -> Vec<f32> {
    // Deterministic offline hashing-trick embedding (bag of character n-grams + tokens).
    let mut v = vec![0.0_f32; EMBED_DIM];
    let lower = text.to_ascii_lowercase();
    for token in lower.split(|c: char| !c.is_alphanumeric()) {
        if token.is_empty() {
            continue;
        }
        let h = hash_to_dim(token);
        v[h] += 1.0;
        if token.len() >= 3 {
            for i in 0..token.len().saturating_sub(2) {
                let tri = &token[i..i + 3];
                let h2 = hash_to_dim(tri);
                v[h2] += 0.5;
            }
        }
    }
    // L2 normalize
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-8);
    for x in &mut v {
        *x /= norm;
    }
    v
}

fn hash_to_dim(s: &str) -> usize {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    let bytes = hasher.finalize();
    let n = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    (n as usize) % EMBED_DIM
}

fn f32_slice_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

fn bytes_to_f32_vec(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    if n == 0 {
        return 0.0;
    }
    let mut dot = 0.0;
    let mut na = 0.0;
    let mut nb = 0.0;
    for i in 0..n {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom < 1e-8 {
        0.0
    } else {
        dot / denom
    }
}

fn chunk_text(text: &str, size: usize, overlap: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return vec![];
    }
    let mut out = Vec::new();
    let mut start = 0usize;
    while start < chars.len() {
        let end = (start + size).min(chars.len());
        out.push(chars[start..end].iter().collect());
        if end == chars.len() {
            break;
        }
        start = end.saturating_sub(overlap).max(start + 1);
    }
    out
}

fn extract_document_text(path: &Path) -> Option<String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "md" | "markdown" | "txt" | "rst" | "adoc" | "html" | "htm" | "json" | "yaml" | "yml"
        | "toml" | "csv" => fs::read_to_string(path).ok(),
        "pdf" => pdf_extract::extract_text(path).ok(),
        // Apple/Dash-style docsets are directories; individual HTML pages handled by walk.
        _ => {
            // Treat extension-less README-like files
            let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
            if name == "readme" || name.ends_with(".docset") {
                fs::read_to_string(path).ok()
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn index_and_hybrid_query() {
        let dir = tempfile::tempdir().unwrap();
        let docs = dir.path().join("docs");
        let rag = dir.path().join("rag");
        fs::create_dir_all(&docs).unwrap();
        fs::write(
            docs.join("guide.md"),
            "# Offline RAG\n\nSource indexes markdown for local retrieval with hybrid BM25 and vectors.\n",
        )
        .unwrap();
        let engine = OfflineRag::open(docs.clone(), rag).await.unwrap();
        let status = engine.index_docs(None, true).await.unwrap();
        assert!(status.chunk_count >= 1);
        let hits = engine
            .query("hybrid BM25 retrieval", 5, 0.5, 0.5)
            .await
            .unwrap();
        assert!(!hits.is_empty());
        assert!(hits[0].content.to_ascii_lowercase().contains("rag")
            || hits[0].content.to_ascii_lowercase().contains("hybrid"));
    }

    #[test]
    fn embedding_is_deterministic() {
        let a = embed_text("hello world");
        let b = embed_text("hello world");
        assert_eq!(a, b);
        assert_eq!(a.len(), EMBED_DIM);
    }
}
