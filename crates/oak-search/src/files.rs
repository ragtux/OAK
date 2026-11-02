//! The workspace's files for `goto/file`: every file under the root that
//! `.gitignore` does not hide and `files.exclude` does not name, as paths
//! relative to the root with `/` separators (what Fleet's Goto rows show),
//! ranked against a query with the fuzzy matcher.

use crate::fuzzy::{fuzzy_match_path, FuzzyMatch};
use ignore::WalkBuilder;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default)]
pub struct FileIndex {
    root: PathBuf,
    /// Relative paths with `/` separators, sorted.
    entries: Vec<String>,
}

/// A `files.exclude` pattern against a path component: a literal name,
/// `*.ext`, or a `**/name` prefix (Fleet's own defaults use these three).
pub fn excluded_by(patterns: &[String], name: &str) -> bool {
    patterns.iter().any(|pat| {
        let pat = pat.strip_prefix("**/").unwrap_or(pat);
        if let Some(suffix) = pat.strip_prefix('*') {
            name.ends_with(suffix)
        } else {
            name == pat
        }
    })
}

/// A walk over the workspace honouring `.gitignore`, skipping `.git` and
/// anything `exclude` names (`files.exclude`); hidden files are included,
/// as the Files tool shows them when the hidden toggle is on.
pub fn walk(root: &Path, exclude: &[String]) -> ignore::Walk {
    let exclude: Vec<String> = exclude.to_vec();
    WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .follow_links(false)
        .filter_entry(move |entry| {
            let name = entry.file_name().to_string_lossy();
            name != ".git" && !excluded_by(&exclude, &name)
        })
        .build()
}

impl FileIndex {
    /// Reads the tree once; the caller re-indexes when the disk changes.
    pub fn build(root: &Path, exclude: &[String]) -> FileIndex {
        let mut entries = Vec::new();
        for entry in walk(root, exclude).flatten() {
            if entry.file_type().is_some_and(|t| t.is_file()) {
                if let Ok(rel) = entry.path().strip_prefix(root) {
                    entries.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
        entries.sort();
        FileIndex {
            root: root.to_path_buf(),
            entries,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    /// The absolute path of a relative entry.
    pub fn absolute(&self, rel: &str) -> PathBuf {
        let mut p = self.root.clone();
        for part in rel.split('/') {
            p.push(part);
        }
        p
    }

    /// The best `limit` matches for `query`, best first; an empty query gives
    /// the first `limit` entries in path order.
    pub fn matches(&self, query: &str, limit: usize) -> Vec<(&str, FuzzyMatch)> {
        let mut scored: Vec<(&str, FuzzyMatch)> = self
            .entries
            .iter()
            .filter_map(|e| fuzzy_match_path(query, e).map(|m| (e.as_str(), m)))
            .collect();
        scored.sort_by(|a, b| {
            b.1.score
                .cmp(&a.1.score)
                .then_with(|| a.0.len().cmp(&b.0.len()))
                .then_with(|| a.0.cmp(b.0))
        });
        scored.truncate(limit);
        scored
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn indexes_and_ranks() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("target/debug")).unwrap();
        fs::create_dir_all(root.join("node_modules")).unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();
        fs::write(root.join("src/lib.rs"), "").unwrap();
        fs::write(root.join("README.md"), "").unwrap();
        fs::write(root.join(".hidden"), "").unwrap();
        fs::write(root.join("target/debug/out.o"), "").unwrap();
        fs::write(root.join("node_modules/x.js"), "").unwrap();
        fs::write(root.join(".gitignore"), "target/\n").unwrap();
        let index = FileIndex::build(root, &["**/node_modules".to_string()]);
        let entries = index.entries();
        assert!(entries.contains(&"src/main.rs".to_string()));
        assert!(entries.contains(&".hidden".to_string()));
        // `.gitignore` applies only inside a git repository; without one the
        // walk keeps target/, so only the exclude list is asserted here.
        assert!(!entries.iter().any(|e| e.starts_with("node_modules")));
        let m = index.matches("main", 5);
        assert_eq!(m[0].0, "src/main.rs");
        let all = index.matches("", 100);
        assert_eq!(all.len(), entries.len());
    }
}
