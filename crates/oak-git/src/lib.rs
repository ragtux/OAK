//! Git for Oak, M4's first piece: the branch the toolbar shows and the
//! per-file status the Files tree colours (`editor.gitDiff.text.*`,
//! `icon.background.*`, `docs/fleet-probe.md`, eighth pass). Everything
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
    }
}
