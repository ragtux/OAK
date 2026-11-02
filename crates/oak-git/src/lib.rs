//! Git for Oak, M4's first piece: the branch the toolbar shows and the
//! per-file status the Files tree colours (`editor.gitDiff.text.*`,
//! `icon.background.*`, `docs/fleet-probe.md`, eighth pass), and a file's
//! changed lines for the gutter, its history and its blame (painted once
//! batch 10 shows them). Everything
//! shells out to the `git` on the path, as Fleet does
//! (`git.discoverGitManually`); when there is no git, there is no repository
//! and nothing is coloured.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// What `git status` says about a file, folded to the states Fleet colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Status {
    /// Staged as new (`A`).
    Added,
    /// Changed in the index or the work tree (`M`, `T`).
    Modified,
    /// Staged as renamed or copied (`R`, `C`); the new path carries it.
    Renamed,
    /// Deleted in the index or the work tree.
    Deleted,
    /// Not tracked (`??`).
    Untracked,
    /// An unmerged path (`U` on either side, `AA`, `DD`).
    Conflict,
    /// Ignored (`!!`, only with `--ignored`).
    Ignored,
}

impl Status {
    /// The two columns of a porcelain v1 entry.
    pub fn from_xy(x: u8, y: u8) -> Option<Status> {
        Some(match (x, y) {
            (b'?', b'?') => Status::Untracked,
            (b'!', b'!') => Status::Ignored,
            (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D') => Status::Conflict,
            (b'R', _) | (b'C', _) => Status::Renamed,
            (b'A', _) => Status::Added,
            (b'D', _) | (_, b'D') => Status::Deleted,
            (b'M', _) | (b'T', _) | (_, b'M') | (_, b'T') => Status::Modified,
            _ => return None,
        })
    }
}

/// A repository found above a workspace folder.
#[derive(Clone, Debug)]
pub struct Repo {
    /// The work tree's top level, absolute.
    pub root: PathBuf,
}

fn git(dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .with_context(|| format!("git {}", args.join(" ")))?;
    if !output.status.success() {
        anyhow::bail!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

impl Repo {
    /// The repository holding `dir`, if git is on the path and `dir` is in
    /// a work tree.
    pub fn discover(dir: &Path) -> Option<Repo> {
        let out = git(dir, &["rev-parse", "--show-toplevel"]).ok()?;
        let top = String::from_utf8_lossy(&out).trim().to_string();
        if top.is_empty() {
            return None;
        }
        Some(Repo {
            root: PathBuf::from(top),
        })
    }

    /// The checked-out branch, or the short commit id when detached.
    pub fn branch(&self) -> Option<String> {
        let out = git(&self.root, &["rev-parse", "--abbrev-ref", "HEAD"]).ok()?;
        let name = String::from_utf8_lossy(&out).trim().to_string();
        if name.is_empty() {
            return None;
        }
        if name == "HEAD" {
            let out = git(&self.root, &["rev-parse", "--short", "HEAD"]).ok()?;
            return Some(String::from_utf8_lossy(&out).trim().to_string());
        }
        Some(name)
    }

    /// Every file with a status, by absolute path. Folders are not listed;
    /// Fleet colours only files.
    pub fn status(&self) -> Result<HashMap<PathBuf, Status>> {
        let out = git(
            &self.root,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        )?;
        Ok(parse_status(&out, &self.root))
    }
}

/// One changed run of lines in a file against HEAD, for the gutter's line
/// status (`editor.showLineStatus`). Rows are zero-based in the current
/// text; a deletion is the row the removed lines sat before, with no rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub row: usize,
    pub rows: usize,
    pub kind: HunkKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HunkKind {
    Added,
    Modified,
    Deleted,
}

impl Repo {
    /// The file's changed lines against HEAD (index and work tree together,
    /// as Fleet's gutter marks them); an untracked file has none.
    pub fn line_status(&self, path: &Path) -> Result<Vec<Hunk>> {
        let rel = path
            .strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let out = git(
            &self.root,
            &[
                "diff",
                "-U0",
                "--no-color",
                "--no-ext-diff",
                "HEAD",
                "--",
                &rel,
            ],
        )?;
        Ok(parse_hunks(&String::from_utf8_lossy(&out)))
    }
}

/// Parses the `@@ -a,b +c,d @@` lines of a unified diff with no context.
pub fn parse_hunks(diff: &str) -> Vec<Hunk> {
    let mut hunks = Vec::new();
    for line in diff.lines() {
        let Some(rest) = line.strip_prefix("@@ -") else {
            continue;
        };
        let Some((old, rest)) = rest.split_once(" +") else {
            continue;
        };
        let Some((new, _)) = rest.split_once(" @@") else {
            continue;
        };
        let range = |s: &str| -> Option<(usize, usize)> {
            match s.split_once(',') {
                Some((start, len)) => Some((start.parse().ok()?, len.parse().ok()?)),
                None => Some((s.parse().ok()?, 1)),
            }
        };
        let (Some((_, old_len)), Some((new_start, new_len))) = (range(old), range(new)) else {
            continue;
        };
        let (kind, row, rows) = if new_len == 0 {
            // `+c,0`: the lines were removed after line c (c is 0 at the top).
            (HunkKind::Deleted, new_start, 0)
        } else if old_len == 0 {
            (HunkKind::Added, new_start - 1, new_len)
        } else {
            (HunkKind::Modified, new_start - 1, new_len)
        };
        hunks.push(Hunk { row, rows, kind });
    }
    hunks
}

/// One commit of a history (`git/history/*`, `git/file-history`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub id: String,
    pub short_id: String,
    pub author: String,
    /// ISO 8601, as `%aI` prints it.
    pub date: String,
    pub summary: String,
}

/// Who last changed one line (`git/toggle-blame`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlameLine {
    pub short_id: String,
    pub author: String,
    pub date: String,
    pub summary: String,
    /// An uncommitted line (the all-zero commit).
    pub uncommitted: bool,
}

const LOG_FORMAT: &str = "--format=%H%x00%h%x00%an%x00%aI%x00%s%x00";

impl Repo {
    /// The newest `limit` commits of the repository, or of one file.
    pub fn history(&self, path: Option<&Path>, limit: usize) -> Result<Vec<Commit>> {
        let limit = format!("-n{limit}");
        let mut args = vec!["log", LOG_FORMAT, &limit];
        let rel;
        if let Some(path) = path {
            rel = self.relative(path);
            args.push("--follow");
            args.push("--");
            args.push(&rel);
        }
        let out = git(&self.root, &args)?;
        Ok(parse_log(&String::from_utf8_lossy(&out)))
    }

    /// One entry per line of the file as it is on disk.
    pub fn blame(&self, path: &Path) -> Result<Vec<BlameLine>> {
        let rel = self.relative(path);
        let out = git(&self.root, &["blame", "--line-porcelain", "--", &rel])?;
        Ok(parse_blame(&String::from_utf8_lossy(&out)))
    }

    fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }
}

/// Parses `git log` printed with [`LOG_FORMAT`]: five NUL-ended fields per commit.
pub fn parse_log(out: &str) -> Vec<Commit> {
    let mut fields = out.split('\0').map(|s| s.trim_start_matches('\n'));
    let mut commits = Vec::new();
    while let (Some(id), Some(short_id), Some(author), Some(date), Some(summary)) = (
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
    ) {
        if id.is_empty() {
            break;
        }
        commits.push(Commit {
            id: id.to_string(),
            short_id: short_id.to_string(),
            author: author.to_string(),
            date: date.to_string(),
            summary: summary.to_string(),
        });
    }
    commits
}

/// Parses `git blame --line-porcelain`: a header line `<sha> <orig> <final>
/// [<count>]`, then `key value` lines, then the line itself after a tab.
pub fn parse_blame(out: &str) -> Vec<BlameLine> {
    let mut lines = Vec::new();
    let mut current: Option<BlameLine> = None;
    for line in out.lines() {
        if let Some(text) = line.strip_prefix('\t') {
            let _ = text;
            if let Some(entry) = current.take() {
                lines.push(entry);
            }
            continue;
        }
        match &mut current {
            None => {
                let sha = line.split(' ').next().unwrap_or("");
                if sha.len() < 7 {
                    continue;
                }
                current = Some(BlameLine {
                    short_id: sha[..7].to_string(),
                    author: String::new(),
                    date: String::new(),
                    summary: String::new(),
                    uncommitted: sha.bytes().all(|b| b == b'0'),
                });
            }
            Some(entry) => {
                if let Some(v) = line.strip_prefix("author ") {
                    entry.author = v.to_string();
                } else if let Some(v) = line.strip_prefix("author-time ") {
                    entry.date = v.to_string();
                } else if let Some(v) = line.strip_prefix("summary ") {
                    entry.summary = v.to_string();
                }
            }
        }
    }
    lines
}

/// Parses `git status --porcelain=v1 -z`: `XY path\0`, and for a rename or
/// copy `XY new\0old\0`.
pub fn parse_status(out: &[u8], root: &Path) -> HashMap<PathBuf, Status> {
    let mut statuses = HashMap::new();
    let mut fields = out.split(|&b| b == 0);
    while let Some(entry) = fields.next() {
        if entry.len() < 4 {
            continue;
        }
        let (x, y) = (entry[0], entry[1]);
        let path = String::from_utf8_lossy(&entry[3..]).to_string();
        if matches!(x, b'R' | b'C') {
            // The old name follows; nothing to show for it.
            fields.next();
        }
        if let Some(status) = Status::from_xy(x, y) {
            statuses.insert(root.join(path), status);
        }
    }
    statuses
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_entries() {
        let out = b" M README.md\0A  src/added.rs\0R  src/renamed.rs\0src/old.rs\0?? notes.txt\0UU both.rs\0D  gone.rs\0";
        let root = Path::new("/w");
        let s = parse_status(out, root);
        assert_eq!(s[&root.join("README.md")], Status::Modified);
        assert_eq!(s[&root.join("src/added.rs")], Status::Added);
        assert_eq!(s[&root.join("src/renamed.rs")], Status::Renamed);
        assert!(!s.contains_key(&root.join("src/old.rs")));
        assert_eq!(s[&root.join("notes.txt")], Status::Untracked);
        assert_eq!(s[&root.join("both.rs")], Status::Conflict);
        assert_eq!(s[&root.join("gone.rs")], Status::Deleted);
        assert_eq!(s.len(), 6);
    }

    #[test]
    fn folds_the_columns() {
        assert_eq!(Status::from_xy(b'M', b' '), Some(Status::Modified));
        assert_eq!(Status::from_xy(b' ', b'T'), Some(Status::Modified));
        assert_eq!(Status::from_xy(b'A', b'M'), Some(Status::Added));
        assert_eq!(Status::from_xy(b' ', b'D'), Some(Status::Deleted));
        assert_eq!(Status::from_xy(b'!', b'!'), Some(Status::Ignored));
        assert_eq!(Status::from_xy(b' ', b' '), None);
    }

    #[test]
    fn parses_hunk_headers() {
        let diff = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n+b\n@@ -3,0 +4,2 @@\n+x\n+y\n@@ -8,2 +10,0 @@\n-p\n-q\n@@ -12,3 +12,4 @@\n";
        let hunks = parse_hunks(diff);
        assert_eq!(
            hunks,
            vec![
                Hunk {
                    row: 0,
                    rows: 1,
                    kind: HunkKind::Modified
                },
                Hunk {
                    row: 3,
                    rows: 2,
                    kind: HunkKind::Added
                },
                Hunk {
                    row: 10,
                    rows: 0,
                    kind: HunkKind::Deleted
                },
                Hunk {
                    row: 11,
                    rows: 4,
                    kind: HunkKind::Modified
                },
            ]
        );
        assert!(parse_hunks("").is_empty());
    }

    #[test]
    fn parses_log_and_blame() {
        let log = "abc123def\x00abc123d\x00Ada\x002026-11-02T10:00:00+01:00\x00First\x00\n0f0f0f0f0\x000f0f0f0\x00Bo\x002026-11-01T09:00:00+01:00\x00Second line\x00\n";
        let commits = parse_log(log);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].short_id, "abc123d");
        assert_eq!(commits[0].author, "Ada");
        assert_eq!(commits[1].summary, "Second line");
        assert!(parse_log("").is_empty());

        let blame = "abc123def4567890 1 1 1\nauthor Ada\nauthor-mail <a>\nauthor-time 1762074000\nsummary First\nfilename f\n\tline one\n0000000000000000000000000000000000000000 2 2 1\nauthor Not Committed Yet\nauthor-time 1762074001\nsummary Version of f from f\nfilename f\n\tline two\n";
        let lines = parse_blame(blame);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].short_id, "abc123d");
        assert_eq!(lines[0].author, "Ada");
        assert_eq!(lines[0].date, "1762074000");
        assert_eq!(lines[0].summary, "First");
        assert!(!lines[0].uncommitted);
        assert!(lines[1].uncommitted);
    }

    /// Against the real git, when there is one.
    #[test]
    fn reads_a_repository() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            let ok = Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .output()
                .unwrap()
                .status
                .success();
            assert!(ok, "git {}", args.join(" "));
        };
        run(&["init", "-q", "-b", "trunk"]);
        run(&["config", "user.email", "oak@example.com"]);
        run(&["config", "user.name", "Oak"]);
        std::fs::write(dir.path().join("kept.txt"), "a\n").unwrap();
        run(&["add", "kept.txt"]);
        run(&["commit", "-q", "-m", "one"]);
        std::fs::write(dir.path().join("kept.txt"), "b\n").unwrap();
        std::fs::write(dir.path().join("new.txt"), "n\n").unwrap();
        let nested = dir.path().join("sub");
        std::fs::create_dir(&nested).unwrap();
        let repo = Repo::discover(&nested).expect("a repository above the folder");
        assert_eq!(
            repo.root.canonicalize().unwrap(),
            dir.path().canonicalize().unwrap()
        );
        assert_eq!(repo.branch().as_deref(), Some("trunk"));
        let s = repo.status().unwrap();
        assert_eq!(s[&repo.root.join("kept.txt")], Status::Modified);
        assert_eq!(s[&repo.root.join("new.txt")], Status::Untracked);
        let hunks = repo.line_status(&repo.root.join("kept.txt")).unwrap();
        assert_eq!(
            hunks,
            vec![Hunk {
                row: 0,
                rows: 1,
                kind: HunkKind::Modified
            }]
        );
        assert!(repo
            .line_status(&repo.root.join("new.txt"))
            .unwrap()
            .is_empty());
        let history = repo.history(None, 10).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].summary, "one");
        assert_eq!(history[0].author, "Oak");
        let for_file = repo.history(Some(&repo.root.join("kept.txt")), 10).unwrap();
        assert_eq!(for_file, history);
        let blame = repo.blame(&repo.root.join("kept.txt")).unwrap();
        assert_eq!(blame.len(), 1);
        assert!(
            blame[0].uncommitted,
            "the work tree's line is not committed"
        );
    }
}
