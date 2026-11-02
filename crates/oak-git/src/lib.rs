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

/// One changed file as the Git tool lists it: its status, whether the index
/// holds it (the row's checkbox) and, for a rename, the old path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusEntry {
    pub path: PathBuf,
    pub status: Status,
    pub staged: bool,
    pub old_path: Option<PathBuf>,
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub row: usize,
    pub rows: usize,
    pub kind: HunkKind,
    /// The lines HEAD had there (what Revert puts back).
    pub old: Vec<String>,
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
    let mut hunks: Vec<Hunk> = Vec::new();
    for line in diff.lines() {
        let Some(rest) = line.strip_prefix("@@ -") else {
            if let Some(old) = line.strip_prefix('-') {
                if !line.starts_with("---") {
                    if let Some(h) = hunks.last_mut() {
                        h.old.push(old.to_string());
                    }
                }
            }
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
        hunks.push(Hunk {
            row,
            rows,
            kind,
            old: Vec::new(),
        });
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

/// A local branch (`git branch`), for the branch tag's menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub current: bool,
    /// Ahead of and behind its upstream, when it has one.
    pub upstream: Option<AheadBehind>,
}

impl Repo {
    /// The local branches, the checked-out one marked.
    pub fn branches(&self) -> Result<Vec<Branch>> {
        let out = git(
            &self.root,
            &[
                "branch",
                "--format=%(refname:short)%00%(HEAD)%00%(upstream:short)",
            ],
        )?;
        let text = String::from_utf8_lossy(&out);
        let mut branches = Vec::new();
        for line in text.lines() {
            let mut parts = line.split('\0');
            let (Some(name), Some(head), Some(upstream)) =
                (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            if name.is_empty() {
                continue;
            }
            let upstream = if upstream.is_empty() {
                None
            } else {
                let range = format!("{name}...{upstream}");
                git(&self.root, &["rev-list", "--left-right", "--count", &range])
                    .ok()
                    .and_then(|o| parse_ahead_behind(&String::from_utf8_lossy(&o)))
            };
            branches.push(Branch {
                name: name.to_string(),
                current: head == "*",
                upstream,
            });
        }
        Ok(branches)
    }

    /// `git checkout <branch>`.
    pub fn checkout(&self, branch: &str) -> Result<()> {
        git(&self.root, &["checkout", "--quiet", branch]).map(|_| ())
    }
}

/// Commits ahead of and behind the upstream branch (`git.autoFetch` feeds
/// this: "incoming/outgoing commits in the Git panel").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AheadBehind {
    pub ahead: usize,
    pub behind: usize,
}

impl Repo {
    /// `git fetch` from every remote, quietly; the work of `git.autoFetch`.
    pub fn fetch(&self) -> Result<()> {
        git(&self.root, &["fetch", "--all", "--quiet"]).map(|_| ())
    }

    /// How far HEAD is from its upstream; None without an upstream.
    pub fn ahead_behind(&self) -> Option<AheadBehind> {
        let out = git(
            &self.root,
            &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
        )
        .ok()?;
        parse_ahead_behind(&String::from_utf8_lossy(&out))
    }
}

/// Parses `git rev-list --left-right --count HEAD...@{upstream}`: `ahead\tbehind`.
pub fn parse_ahead_behind(out: &str) -> Option<AheadBehind> {
    let mut parts = out.split_whitespace();
    let ahead = parts.next()?.parse().ok()?;
    let behind = parts.next()?.parse().ok()?;
    Some(AheadBehind { ahead, behind })
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
    parse_status_entries(out, root)
        .into_iter()
        .map(|e| (e.path, e.status))
        .collect()
}

/// The same, keeping the index column and a rename's old path.
pub fn parse_status_entries(out: &[u8], root: &Path) -> Vec<StatusEntry> {
    let mut entries = Vec::new();
    let mut fields = out.split(|&b| b == 0);
    while let Some(entry) = fields.next() {
        if entry.len() < 4 {
            continue;
        }
        let (x, y) = (entry[0], entry[1]);
        let path = String::from_utf8_lossy(&entry[3..]).to_string();
        let old_path = if matches!(x, b'R' | b'C') {
            fields
                .next()
                .map(|old| root.join(String::from_utf8_lossy(old).to_string()))
        } else {
            None
        };
        if let Some(status) = Status::from_xy(x, y) {
            // Staged: the index column holds a change (not `?` or `!`).
            let staged = !matches!(x, b' ' | b'?' | b'!');
            entries.push(StatusEntry {
                path: root.join(path),
                status,
                staged,
                old_path,
            });
        }
    }
    entries
}

impl Repo {
    /// Every changed file with its index state, for the Git tool.
    pub fn status_entries(&self) -> Result<Vec<StatusEntry>> {
        let out = git(
            &self.root,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        )?;
        Ok(parse_status_entries(&out, &self.root))
    }

    /// `git add` on the paths (the Git tool's checkboxes).
    pub fn stage(&self, paths: &[PathBuf]) -> Result<()> {
        let rels: Vec<String> = paths.iter().map(|p| self.relative(p)).collect();
        let mut args = vec!["add", "--"];
        args.extend(rels.iter().map(String::as_str));
        git(&self.root, &args).map(|_| ())
    }

    /// `git reset` on the paths.
    pub fn unstage(&self, paths: &[PathBuf]) -> Result<()> {
        let rels: Vec<String> = paths.iter().map(|p| self.relative(p)).collect();
        let mut args = vec!["reset", "--quiet", "--"];
        args.extend(rels.iter().map(String::as_str));
        git(&self.root, &args).map(|_| ())
    }

    /// `git commit -m` with what the index holds.
    pub fn commit(&self, message: &str) -> Result<()> {
        git(&self.root, &["commit", "--quiet", "-m", message]).map(|_| ())
    }

    /// How many commits HEAD has (the "unsynced" count without an upstream).
    pub fn commit_count(&self) -> usize {
        git(&self.root, &["rev-list", "--count", "HEAD"])
            .ok()
            .and_then(|o| String::from_utf8_lossy(&o).trim().parse().ok())
            .unwrap_or(0)
    }
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
        let entries = parse_status_entries(out, root);
        let readme = entries
            .iter()
            .find(|e| e.path == root.join("README.md"))
            .unwrap();
        assert!(!readme.staged);
        let added = entries
            .iter()
            .find(|e| e.path == root.join("src/added.rs"))
            .unwrap();
        assert!(added.staged);
        let renamed = entries
            .iter()
            .find(|e| e.path == root.join("src/renamed.rs"))
            .unwrap();
        assert_eq!(renamed.old_path, Some(root.join("src/old.rs")));
        assert!(
            !entries
                .iter()
                .find(|e| e.path == root.join("notes.txt"))
                .unwrap()
                .staged
        );
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
                    kind: HunkKind::Modified,
                    old: vec!["a".to_string()],
                },
                Hunk {
                    row: 3,
                    rows: 2,
                    kind: HunkKind::Added,
                    old: vec![],
                },
                Hunk {
                    row: 10,
                    rows: 0,
                    kind: HunkKind::Deleted,
                    old: vec!["p".to_string(), "q".to_string()],
                },
                Hunk {
                    row: 11,
                    rows: 4,
                    kind: HunkKind::Modified,
                    old: vec![],
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

    #[test]
    fn parses_ahead_behind() {
        assert_eq!(
            parse_ahead_behind("2\t5\n"),
            Some(AheadBehind {
                ahead: 2,
                behind: 5
            })
        );
        assert_eq!(parse_ahead_behind(""), None);
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
        assert_eq!(repo.commit_count(), 1);
        repo.stage(&[repo.root.join("new.txt")]).unwrap();
        let entries = repo.status_entries().unwrap();
        assert!(entries
            .iter()
            .any(|e| e.path.ends_with("new.txt") && e.staged));
        repo.unstage(&[repo.root.join("new.txt")]).unwrap();
        let entries = repo.status_entries().unwrap();
        assert!(entries
            .iter()
            .any(|e| e.path.ends_with("new.txt") && !e.staged));
        let hunks = repo.line_status(&repo.root.join("kept.txt")).unwrap();
        assert_eq!(
            hunks,
            vec![Hunk {
                row: 0,
                rows: 1,
                kind: HunkKind::Modified,
                old: vec!["a".to_string()],
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
        assert_eq!(repo.ahead_behind(), None, "no upstream yet");
        let branches = repo.branches().unwrap();
        assert_eq!(branches.len(), 1);
        assert_eq!(branches[0].name, "trunk");
        assert!(branches[0].current);
        assert_eq!(branches[0].upstream, None);

        // A clone has an upstream: one commit behind after the origin moves on.
        let clone_dir = tempfile::tempdir().unwrap();
        let clone_path = clone_dir.path().join("clone");
        let ok = Command::new("git")
            .args([
                "clone",
                "-q",
                &dir.path().to_string_lossy(),
                &clone_path.to_string_lossy(),
            ])
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git clone");
        std::fs::write(dir.path().join("kept.txt"), "c\n").unwrap();
        run(&["commit", "-q", "-am", "two"]);
        let clone = Repo::discover(&clone_path).unwrap();
        clone.fetch().unwrap();
        assert_eq!(
            clone.ahead_behind(),
            Some(AheadBehind {
                ahead: 0,
                behind: 1
            })
        );
        let branches = clone.branches().unwrap();
        assert_eq!(
            branches[0].upstream,
            Some(AheadBehind {
                ahead: 0,
                behind: 1
            })
        );
    }
}
