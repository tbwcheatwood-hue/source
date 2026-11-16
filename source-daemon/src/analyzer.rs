//! Multi-file tree-sitter AST / symbol search (extends Phase 2 repo-map).

use anyhow::Result;
use source_core::context::{RepoMap, RepoSymbol};
use std::path::Path;

use crate::context::ContextCompactor;

#[derive(Debug, Clone)]
pub struct SymbolSearchQuery {
    pub query: String,
    pub kind: Option<String>,
    pub language: Option<String>,
    pub path_glob: Option<String>,
    pub max_results: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SymbolHit {
    pub symbol: RepoSymbol,
    pub score: f32,
    pub match_kind: String,
}

pub struct TreeSitterAnalyzer {
    compactor: ContextCompactor,
}

impl Default for TreeSitterAnalyzer {
    fn default() -> Self {
        Self {
            compactor: ContextCompactor::default(),
        }
    }
}

impl TreeSitterAnalyzer {
    pub fn new(compactor: ContextCompactor) -> Self {
        Self { compactor }
    }

    pub fn build_map(&self, root: &Path, max_symbols: Option<usize>) -> Result<RepoMap> {
        self.compactor.build_repo_map(root, max_symbols)
    }

    /// Search symbols across a workspace using an in-memory Phase-2 map, scored by name/signature.
    pub fn search(&self, root: &Path, q: SymbolSearchQuery) -> Result<Vec<SymbolHit>> {
        let map = self.compactor.build_repo_map(root, Some(8_192))?;
        Ok(search_map(&map, &q))
    }

    pub fn search_map(map: &RepoMap, q: &SymbolSearchQuery) -> Vec<SymbolHit> {
        search_map(map, q)
    }
}

fn search_map(map: &RepoMap, q: &SymbolSearchQuery) -> Vec<SymbolHit> {
    let needle = q.query.to_ascii_lowercase();
    let mut hits = Vec::new();

    for sym in &map.symbols {
        if let Some(kind) = &q.kind {
            if !sym.kind.eq_ignore_ascii_case(kind) && !sym.kind.contains(kind) {
                continue;
            }
        }
        if let Some(lang) = &q.language {
            if !sym.language.eq_ignore_ascii_case(lang) {
                continue;
            }
        }
        if let Some(glob) = &q.path_glob {
            if !path_matches(sym.path.to_string_lossy().as_ref(), glob) {
                continue;
            }
        }

        let name_l = sym.name.to_ascii_lowercase();
        let sig_l = sym.signature.to_ascii_lowercase();
        let path_l = sym.path.to_string_lossy().to_ascii_lowercase();

        let (score, match_kind) = if name_l == needle {
            (1.0, "exact_name")
        } else if name_l.starts_with(&needle) {
            (0.9, "prefix_name")
        } else if name_l.contains(&needle) {
            (0.75, "substring_name")
        } else if sig_l.contains(&needle) {
            (0.55, "signature")
        } else if path_l.contains(&needle) {
            (0.35, "path")
        } else {
            continue;
        };

        hits.push(SymbolHit {
            symbol: sym.clone(),
            score,
            match_kind: match_kind.into(),
        });
    }

    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.symbol.name.cmp(&b.symbol.name))
    });
    hits.truncate(q.max_results.max(1));
    hits
}

fn path_matches(path: &str, glob: &str) -> bool {
    // Simple glob: `*` and `**` / substring with optional `*` wildcards.
    if glob.contains('*') {
        let parts: Vec<&str> = glob.split('*').filter(|p| !p.is_empty()).collect();
        if parts.is_empty() {
            return true;
        }
        let mut idx = 0usize;
        if !glob.starts_with('*') {
            if !path.starts_with(parts[0]) {
                return false;
            }
            idx = parts[0].len();
        }
        for (i, part) in parts.iter().enumerate() {
            if i == 0 && !glob.starts_with('*') {
                continue;
            }
            if let Some(found) = path[idx..].find(part) {
                idx += found + part.len();
            } else {
                return false;
            }
        }
        if !glob.ends_with('*') {
            return path.ends_with(parts.last().copied().unwrap_or(""));
        }
        true
    } else {
        path.contains(glob)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn finds_rust_fn_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "pub fn greet_user() {{}}").unwrap();
        writeln!(f, "pub struct Widget;").unwrap();

        let analyzer = TreeSitterAnalyzer::default();
        let hits = analyzer
            .search(
                dir.path(),
                SymbolSearchQuery {
                    query: "greet".into(),
                    kind: None,
                    language: Some("rust".into()),
                    path_glob: None,
                    max_results: 10,
                },
            )
            .unwrap();
        assert!(!hits.is_empty());
        assert!(hits.iter().any(|h| h.symbol.name.contains("greet")));
    }
}
