//! Tree-sitter smart context compactor: repo-map, terminal truncation, conversation summarization.

use anyhow::Result;
use chrono::Utc;
use ignore::WalkBuilder;
use regex::Regex;
use source_core::context::{
    CompactionResult, ContextBudget, ConversationTurn, RepoMap, RepoSymbol, TerminalLogSlice,
};
use std::path::{Path, PathBuf};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Parser, Query, QueryCursor};

pub struct ContextCompactor {
    budget: ContextBudget,
}

impl Default for ContextCompactor {
    fn default() -> Self {
        Self {
            budget: ContextBudget::default(),
        }
    }
}

impl ContextCompactor {
    pub fn new(budget: ContextBudget) -> Self {
        Self { budget }
    }

    pub fn budget(&self) -> &ContextBudget {
        &self.budget
    }

    /// Build a repo-map of signatures using tree-sitter for supported languages.
    pub fn build_repo_map(&self, root: &Path, max_symbols: Option<usize>) -> Result<RepoMap> {
        let max_symbols = max_symbols.unwrap_or(self.budget.repo_map_max_symbols);
        let mut symbols = Vec::new();
        let mut file_count = 0usize;

        let walker = WalkBuilder::new(root)
            .hidden(false)
            .git_ignore(true)
            .git_global(true)
            .build();

        for entry in walker.flatten() {
            let path = entry.path();
            if !entry
                .file_type()
                .map(|ft| ft.is_file())
                .unwrap_or(false)
            {
                continue;
            }
            let Some(lang_name) = language_for_path(path) else {
                continue;
            };
            file_count += 1;
            if let Ok(src) = std::fs::read_to_string(path) {
                if src.len() > 512_000 {
                    continue;
                }
                match extract_symbols(path, &src, lang_name) {
                    Ok(mut found) => symbols.append(&mut found),
                    Err(err) => {
                        tracing::debug!(path = %path.display(), error = %err, "symbol extract skipped");
                    }
                }
            }
            if symbols.len() >= max_symbols {
                symbols.truncate(max_symbols);
                break;
            }
        }

        Ok(RepoMap {
            root: root.to_path_buf(),
            symbols,
            file_count,
            generated_at_unix_ms: Utc::now().timestamp_millis(),
        })
    }

    /// Truncate terminal logs while preserving stack traces and exit codes.
    pub fn compact_terminal(
        &self,
        log: &str,
        max_chars: Option<usize>,
        exit_code: Option<i32>,
    ) -> TerminalLogSlice {
        let max_chars = max_chars.unwrap_or(self.budget.terminal_max_chars);
        let exit = exit_code.or_else(|| detect_exit_code(log));
        let stack_lines = extract_stack_lines(log);
        let preserved_stack_frames = stack_lines.len();

        if log.chars().count() <= max_chars {
            return TerminalLogSlice {
                original_chars: log.chars().count(),
                compacted: log.to_string(),
                preserved_exit_code: exit,
                preserved_stack_frames,
                truncated: false,
            };
        }

        // Keep head + stack frames + tail with exit line.
        let head_budget = max_chars / 3;
        let stack_budget = max_chars / 3;
        let tail_budget = max_chars - head_budget - stack_budget;

        let head = take_chars(log, head_budget);
        let stack_block = {
            let joined = stack_lines.join("\n");
            if joined.chars().count() > stack_budget {
                take_chars_end(&joined, stack_budget)
            } else {
                joined
            }
        };
        let tail = take_chars_end(log, tail_budget);
        let mut compacted = String::new();
        compacted.push_str(&head);
        compacted.push_str("\n\n… [truncated by Source context compactor] …\n\n");
        if !stack_block.is_empty() {
            compacted.push_str("--- preserved stack frames ---\n");
            compacted.push_str(&stack_block);
            compacted.push('\n');
        }
        compacted.push_str(&tail);
        if let Some(code) = exit {
            if !compacted.contains(&format!("exit code")) && !compacted.contains(&format!("{code}"))
            {
                compacted.push_str(&format!("\n[exit_code={code}]\n"));
            }
        }

        // Hard clamp
        if compacted.chars().count() > max_chars {
            compacted = take_chars(&compacted, max_chars);
        }

        TerminalLogSlice {
            original_chars: log.chars().count(),
            compacted,
            preserved_exit_code: exit,
            preserved_stack_frames,
            truncated: true,
        }
    }

    /// Summarize older conversation turns when utilization crosses threshold.
    pub fn compact_conversation(
        &self,
        turns: Vec<ConversationTurn>,
        used_tokens: u32,
        budget: Option<ContextBudget>,
    ) -> CompactionResult {
        let budget = budget.unwrap_or_else(|| self.budget.clone());
        let window = budget.context_window_tokens.max(1) as f32;
        let utilization_before = (used_tokens as f32 / window).min(2.0);

        if utilization_before < budget.summarize_threshold || turns.len() < 4 {
            let after_tokens: u32 = turns.iter().map(|t| t.token_estimate).sum();
            return CompactionResult {
                utilization_before,
                utilization_after: (after_tokens as f32 / window).min(2.0),
                summarized: false,
                summary: None,
                retained_turns: turns,
                dropped_turns: 0,
            };
        }

        // Keep system (if first) + last 3 turns; summarize the middle.
        let (prefix, rest) = if turns
            .first()
            .map(|t| t.role.eq_ignore_ascii_case("system"))
            .unwrap_or(false)
        {
            turns.split_at(1)
        } else {
            turns.split_at(0)
        };

        let keep_tail = 3.min(rest.len());
        let split_at = rest.len() - keep_tail;
        let (middle, tail) = rest.split_at(split_at);

        let summary = summarize_turns(middle);
        let summary_turn = ConversationTurn {
            role: "system".into(),
            content: format!("[Source conversation summary]\n{summary}"),
            token_estimate: estimate_tokens(&summary) + 8,
        };

        let mut retained = Vec::new();
        retained.extend_from_slice(prefix);
        retained.push(summary_turn.clone());
        retained.extend_from_slice(tail);

        let after_tokens: u32 = retained.iter().map(|t| t.token_estimate).sum();
        CompactionResult {
            utilization_before,
            utilization_after: (after_tokens as f32 / window).min(2.0),
            summarized: true,
            summary: Some(summary_turn.content),
            retained_turns: retained,
            dropped_turns: middle.len(),
        }
    }
}

fn language_for_path(path: &Path) -> Option<&'static str> {
    match path.extension().and_then(|e| e.to_str())? {
        "rs" => Some("rust"),
        "py" => Some("python"),
        "js" | "jsx" | "mjs" | "cjs" => Some("javascript"),
        "ts" => Some("typescript"),
        "tsx" => Some("tsx"),
        "go" => Some("go"),
        _ => None,
    }
}

fn language_object(name: &str) -> Option<Language> {
    Some(match name {
        "rust" => tree_sitter_rust::LANGUAGE.into(),
        "python" => tree_sitter_python::LANGUAGE.into(),
        "javascript" => tree_sitter_javascript::LANGUAGE.into(),
        "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "go" => tree_sitter_go::LANGUAGE.into(),
        _ => return None,
    })
}

fn query_source(lang: &str) -> Option<&'static str> {
    Some(match lang {
        "rust" => r#"
(function_item name: (identifier) @name) @item
(struct_item name: (type_identifier) @name) @item
(enum_item name: (type_identifier) @name) @item
(trait_item name: (type_identifier) @name) @item
(impl_item) @item
(mod_item name: (identifier) @name) @item
"#,
        "python" => r#"
(function_definition name: (identifier) @name) @item
(class_definition name: (identifier) @name) @item
"#,
        "javascript" | "typescript" | "tsx" => r#"
(function_declaration name: (identifier) @name) @item
(class_declaration name: (type_identifier) @name) @item
(lexical_declaration) @item
(export_statement) @item
"#,
        "go" => r#"
(function_declaration name: (identifier) @name) @item
(method_declaration name: (field_identifier) @name) @item
(type_declaration) @item
"#,
        _ => return None,
    })
}

fn extract_symbols(path: &Path, source: &str, lang_name: &str) -> Result<Vec<RepoSymbol>> {
    let language = language_object(lang_name)
        .ok_or_else(|| anyhow::anyhow!("unsupported language {lang_name}"))?;
    let query_src = query_source(lang_name)
        .ok_or_else(|| anyhow::anyhow!("no query for {lang_name}"))?;

    let mut parser = Parser::new();
    parser.set_language(&language)?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| anyhow::anyhow!("parse failed"))?;
    let query = Query::new(&language, query_src)?;
    let mut cursor = QueryCursor::new();
    let mut out = Vec::new();
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    while let Some(m) = matches.next() {
        let mut name = String::from("<anon>");
        let mut node = m.captures.first().map(|c| c.node);
        for cap in m.captures {
            let capture_name = query.capture_names()[cap.index as usize];
            if capture_name == "name" {
                name = cap
                    .node
                    .utf8_text(source.as_bytes())
                    .unwrap_or("<anon>")
                    .to_string();
            }
            if capture_name == "item" {
                node = Some(cap.node);
            }
        }
        let Some(n) = node else { continue };
        let start = n.start_position().row as u32 + 1;
        let end = n.end_position().row as u32 + 1;
        let signature = first_line_signature(source, n.start_byte(), n.end_byte());
        let kind = n.kind().to_string();
        out.push(RepoSymbol {
            path: relative_display(path),
            kind,
            name,
            signature,
            start_line: start,
            end_line: end,
            language: lang_name.to_string(),
        });
    }
    Ok(out)
}

fn relative_display(path: &Path) -> PathBuf {
    path.to_path_buf()
}

fn first_line_signature(source: &str, start: usize, end: usize) -> String {
    let slice = &source[start.min(source.len())..end.min(source.len())];
    let line = slice.lines().next().unwrap_or("").trim();
    if line.chars().count() > 200 {
        format!("{}…", take_chars(line, 200))
    } else {
        line.to_string()
    }
}

fn detect_exit_code(log: &str) -> Option<i32> {
    let patterns = [
        r"(?i)exit(?:\s+code)?[=:\s]+(-?\d+)",
        r"(?i)exited with (?:code|status)\s+(-?\d+)",
        r"(?i)process exited[:\s]+(-?\d+)",
        r"\[exit_code=(-?\d+)\]",
    ];
    for pat in patterns {
        if let Ok(re) = Regex::new(pat) {
            if let Some(c) = re.captures(log) {
                if let Some(m) = c.get(1) {
                    if let Ok(code) = m.as_str().parse::<i32>() {
                        return Some(code);
                    }
                }
            }
        }
    }
    None
}

fn extract_stack_lines(log: &str) -> Vec<String> {
    let mut out = Vec::new();
    let rust_frame = Regex::new(r"^\s*(?:\d+:\s+)?(?:at\s+)?[\w:<>]+.+\n?\s+at\s+.+:\d+").ok();
    for line in log.lines() {
        let trimmed = line.trim_end();
        let is_stack = trimmed.contains("Stack backtrace")
            || trimmed.starts_with("   at ")
            || trimmed.starts_with("\tat ")
            || trimmed.contains(".rs:")
                && (trimmed.contains(" at ") || trimmed.trim_start().starts_with("at "))
            || Regex::new(r#"^\s*File \".+\", line \d+"#)
                .ok()
                .map(|re| re.is_match(trimmed))
                .unwrap_or(false)
            || Regex::new(r"^\s+at [\w./]+ \([\w./]+:\d+:\d+\)")
                .ok()
                .map(|re| re.is_match(trimmed))
                .unwrap_or(false)
            || trimmed.starts_with("panic!")
            || trimmed.contains("thread '") && trimmed.contains("panicked at");
        if is_stack {
            out.push(trimmed.to_string());
        } else if let Some(re) = &rust_frame {
            if re.is_match(trimmed) {
                out.push(trimmed.to_string());
            }
        }
    }
    // Also keep contiguous traceback blocks from Python
    let mut in_tb = false;
    for line in log.lines() {
        if line.starts_with("Traceback (most recent call last)") {
            in_tb = true;
        }
        if in_tb {
            if !out.iter().any(|e| e == line) {
                out.push(line.to_string());
            }
            if line.starts_with("Error") || line.starts_with("Exception") || line.is_empty() {
                if line.starts_with("Error") || line.starts_with("Exception") {
                    in_tb = false;
                }
            }
        }
    }
    out
}

fn summarize_turns(turns: &[ConversationTurn]) -> String {
    if turns.is_empty() {
        return "No prior turns.".into();
    }
    let mut parts = Vec::new();
    for (i, t) in turns.iter().enumerate() {
        let excerpt = take_chars(t.content.trim(), 160);
        parts.push(format!("{}. [{}] {}", i + 1, t.role, excerpt));
    }
    format!(
        "Summarized {} earlier turn(s):\n{}",
        turns.len(),
        parts.join("\n")
    )
}

fn estimate_tokens(s: &str) -> u32 {
    // Rough heuristic: ~4 chars/token
    ((s.chars().count() + 3) / 4) as u32
}

fn take_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn take_chars_end(s: &str, n: usize) -> String {
    let total = s.chars().count();
    if total <= n {
        return s.to_string();
    }
    s.chars().skip(total - n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_terminal_keeps_exit() {
        let c = ContextCompactor::default();
        let mut log = "line\n".repeat(5000);
        log.push_str("thread 'main' panicked at src/main.rs:10:5\n");
        log.push_str("stack backtrace:\n   at foo\n");
        log.push_str("exit code: 1\n");
        let slice = c.compact_terminal(&log, Some(800), None);
        assert!(slice.truncated);
        assert_eq!(slice.preserved_exit_code, Some(1));
        assert!(slice.preserved_stack_frames >= 1);
    }

    #[test]
    fn summarizes_when_over_threshold() {
        let c = ContextCompactor::new(ContextBudget {
            summarize_threshold: 0.5,
            context_window_tokens: 100,
            ..Default::default()
        });
        let turns: Vec<_> = (0..8)
            .map(|i| ConversationTurn {
                role: if i == 0 { "system".into() } else { "user".into() },
                content: format!("turn {i} {}", "x".repeat(40)),
                token_estimate: 20,
            })
            .collect();
        let result = c.compact_conversation(turns, 80, None);
        assert!(result.summarized);
        assert!(result.dropped_turns > 0);
    }
}