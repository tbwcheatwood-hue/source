use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ContextBudget {
    /// Soft utilization threshold (0.0–1.0) that triggers summarization.
    pub summarize_threshold: f32,
    /// Maximum characters retained for a terminal log slice after compaction.
    pub terminal_max_chars: usize,
    /// Maximum symbols retained in a repo map.
    pub repo_map_max_symbols: usize,
    /// Estimated model context window in tokens.
    pub context_window_tokens: u32,
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            summarize_threshold: 0.75,
            terminal_max_chars: 8_192,
            repo_map_max_symbols: 2_048,
            context_window_tokens: 32_768,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RepoSymbol {
    pub path: PathBuf,
    pub kind: String,
    pub name: String,
    pub signature: String,
    pub start_line: u32,
    pub end_line: u32,
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct RepoMap {
    pub root: PathBuf,
    pub symbols: Vec<RepoSymbol>,
    pub file_count: usize,
    pub generated_at_unix_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TerminalLogSlice {
    pub original_chars: usize,
    pub compacted: String,
    pub preserved_exit_code: Option<i32>,
    pub preserved_stack_frames: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConversationTurn {
    pub role: String,
    pub content: String,
    pub token_estimate: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CompactionResult {
    pub utilization_before: f32,
    pub utilization_after: f32,
    pub summarized: bool,
    pub summary: Option<String>,
    pub retained_turns: Vec<ConversationTurn>,
    pub dropped_turns: usize,
}