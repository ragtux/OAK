//! Git for Oak, M4's first piece: the branch the toolbar shows and the
//! per-file status the Files tree colours (`editor.gitDiff.text.*`,
//! `icon.background.*`, `docs/fleet-probe.md`, eighth pass), a file's
//! changed lines for the gutter, its history and its blame, the index for
//! the Git tool (stage, unstage, commit), push, pull and revert, and the
//! line diff behind the Changed Files tab (ninth pass). Everything
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
        let out = git(dir, &["rev-parse", "--show-toplevel", "--show-prefix"]).ok()?;
        let text = String::from_utf8_lossy(&out);
        let mut lines = text.lines();
        let top = lines.next()?.trim();
        if top.is_empty() {
            return None;
        }
        let prefix = lines.next().unwrap_or("").trim();
        Some(Repo {
            root: root_spelled_like(dir, top, prefix),
        })
    }
}

/// The work tree's top level spelled the way `dir` is: git prints a mapped
/// network drive as its UNC path (`//server/share/repo`), which never equals
/// the workspace's `Z:\repo\...` paths, so the statuses keyed by it would
/// colour no row. `prefix` is `dir` relative to the top level (git's
/// `--show-prefix`), so the root is `dir` less that many components; `top`
/// itself when they do not line up.
fn root_spelled_like(dir: &Path, top: &str, prefix: &str) -> PathBuf {
    let depth = Path::new(prefix)
        .components()
        .filter(|c| matches!(c, std::path::Component::Normal(_)))
        .count();
    let dir = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(dir))
            .unwrap_or_else(|_| dir.to_path_buf())
    };
    let same_name = |a: &Path, b: &Path| {
        a.file_name()
            .zip(b.file_name())
            .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))
    };
    match dir.ancestors().nth(depth) {
        Some(root) if same_name(root, Path::new(top)) => root.to_path_buf(),
        _ => PathBuf::from(top),
    }
}

impl Repo {
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

impl Repo {
    /// A changed file's hunks against HEAD for the Changed Files tab: as
    /// [`Repo::line_status`], but a rename is followed to its old path so
    /// the diff is of the contents, not a whole file added.
    pub fn file_diff(&self, entry: &StatusEntry) -> Result<Vec<Hunk>> {
        let rel = self.relative(&entry.path);
        let mut args = vec![
            "diff",
            "-U0",
            "--no-color",
            "--no-ext-diff",
            "-M",
            "HEAD",
            "--",
            &rel,
        ];
        let old;
        if let Some(old_path) = &entry.old_path {
            old = self.relative(old_path);
            args.push(&old);
        }
        let out = git(&self.root, &args)?;
        Ok(parse_hunks(&String::from_utf8_lossy(&out)))
    }
}

/// The rows of a side-by-side view from git's own hunks, so the pairing
/// is git's (and Fleet's) rather than a second diff's; None when the hunks
/// do not fit texts of these lengths.
pub fn ops_from_hunks(hunks: &[Hunk], old_len: usize, new_len: usize) -> Option<Vec<LineOp>> {
    let mut ops = Vec::new();
    let (mut o, mut n) = (0, 0);
    for h in hunks {
        while n < h.row {
            if o >= old_len || n >= new_len {
                return None;
            }
            ops.push(LineOp::Keep(o, n));
            o += 1;
            n += 1;
        }
        if n != h.row {
            return None;
        }
        for _ in 0..h.old.len() {
            if o >= old_len {
                return None;
            }
            ops.push(LineOp::Remove(o));
            o += 1;
        }
        for _ in 0..h.rows {
            if n >= new_len {
                return None;
            }
            ops.push(LineOp::Add(n));
            n += 1;
        }
    }
    while n < new_len {
        if o >= old_len {
            return None;
        }
        ops.push(LineOp::Keep(o, n));
        o += 1;
        n += 1;
    }
    (o == old_len).then_some(ops)
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
    /// `author-time`: unix seconds as parsed; `Repo::blame` replaces it
    /// with the local day (`2025-12-31`).
    pub date: String,
    /// `author-tz`: `+0100`.
    pub tz: String,
    pub summary: String,
    /// An uncommitted line (the all-zero commit).
    pub uncommitted: bool,
}

impl BlameLine {
    /// The column's label: `Dec 31, 2025  Author`; nothing for an uncommitted line.
    pub fn label(&self) -> Option<String> {
        if self.uncommitted {
            return None;
        }
        // `author-time` seconds in the author's zone until `Repo::blame`
        // swaps in the local day.
        let day = match self.date.parse::<i64>() {
            Ok(t) => format_day(t, &self.tz),
            Err(_) => format_iso(&self.date),
        };
        Some(format!("{day}  {}", self.author))
    }
}

/// Five NUL-ended fields; the date in the machine's zone, as Fleet shows it
/// (the fixture's `2026-01-01T00:00:00Z` commit reads `18:00 Dec 31, 2025`
/// in Fleet's rows, ninth pass).
const LOG_FORMAT: &str = "--format=%H%x00%h%x00%an%x00%ad%x00%s%x00";
const LOG_DATE: &str = "--date=format-local:%Y-%m-%dT%H:%M:%S";

impl Repo {
    /// The newest `limit` commits of the repository, or of one file.
    pub fn history(&self, path: Option<&Path>, limit: usize) -> Result<Vec<Commit>> {
        self.history_of(None, path, limit)
    }

    /// The same on another branch (the History tool's branch chip); None
    /// is HEAD.
    pub fn history_of(
        &self,
        branch: Option<&str>,
        path: Option<&Path>,
        limit: usize,
    ) -> Result<Vec<Commit>> {
        let limit = format!("-n{limit}");
        let mut args = vec!["log", LOG_FORMAT, LOG_DATE, &limit];
        if let Some(branch) = branch {
            args.push(branch);
        }
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

    /// One entry per line of the file as it is on disk, each commit's day
    /// in the machine's zone.
    pub fn blame(&self, path: &Path) -> Result<Vec<BlameLine>> {
        let rel = self.relative(path);
        let out = git(&self.root, &["blame", "--line-porcelain", "--", &rel])?;
        let mut lines = parse_blame(&String::from_utf8_lossy(&out));
        let mut ids: Vec<String> = lines
            .iter()
            .filter(|l| !l.uncommitted)
            .map(|l| l.short_id.clone())
            .collect();
        ids.sort();
        ids.dedup();
        if !ids.is_empty() {
            let mut args = vec![
                "log",
                "--no-walk=unsorted",
                "--format=%h%x00%ad%x00",
                "--date=format-local:%Y-%m-%d",
            ];
            args.extend(ids.iter().map(String::as_str));
            if let Ok(out) = git(&self.root, &args) {
                let text = String::from_utf8_lossy(&out);
                let mut fields = text.split('\0').map(|s| s.trim_start_matches('\n'));
                let mut days: HashMap<String, String> = HashMap::new();
                while let (Some(id), Some(day)) = (fields.next(), fields.next()) {
                    if id.is_empty() {
                        break;
                    }
                    days.insert(id.to_string(), day.to_string());
                }
                for line in &mut lines {
                    if let Some(day) = days.get(&line.short_id) {
                        line.date = day.clone();
                    }
                }
            }
        }
        Ok(lines)
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

    /// The upstream branch's short name (`origin/main`), if HEAD has one.
    pub fn upstream(&self) -> Option<String> {
        let out = git(
            &self.root,
            &[
                "rev-parse",
                "--abbrev-ref",
                "--symbolic-full-name",
                "@{upstream}",
            ],
        )
        .ok()?;
        let name = String::from_utf8_lossy(&out).trim().to_string();
        (!name.is_empty()).then_some(name)
    }

    /// `git/push`: to the upstream, or with `--set-upstream origin <branch>`
    /// when there is none yet (the Git tool's "Push N commits to a new
    /// remote branch"). Returns what git printed.
    pub fn push(&self) -> Result<String> {
        if self.upstream().is_some() {
            return git_report(&self.root, &["push"]);
        }
        let branch = self
            .branch()
            .ok_or_else(|| anyhow::anyhow!("no branch to push"))?;
        git_report(&self.root, &["push", "--set-upstream", "origin", &branch])
    }

    /// `git/pull`. Returns what git printed.
    pub fn pull(&self) -> Result<String> {
        git_report(&self.root, &["pull"])
    }

    /// The file's text in HEAD, or None when HEAD does not have the path
    /// (an added or untracked file).
    pub fn head_text(&self, path: &Path) -> Option<String> {
        let spec = format!("HEAD:{}", self.relative(path));
        let out = git(&self.root, &["show", &spec]).ok()?;
        Some(String::from_utf8_lossy(&out).into_owned())
    }

    /// `git/revert` on one changed file: HEAD's version comes back for a
    /// tracked file (index and work tree alike), a rename is undone, and a
    /// new file leaves the index and goes to the system trash (Fleet deletes
    /// without a dialog elsewhere; the trash keeps Oak's revert recoverable).
    pub fn revert(&self, entry: &StatusEntry) -> Result<()> {
        let rel = self.relative(&entry.path);
        match entry.status {
            Status::Untracked => trash_file(&entry.path),
            Status::Added => {
                git(
                    &self.root,
                    &["rm", "--quiet", "--cached", "--force", "--", &rel],
                )?;
                trash_file(&entry.path)
            }
            Status::Renamed => {
                let old = entry
                    .old_path
                    .as_ref()
                    .map(|p| self.relative(p))
                    .ok_or_else(|| anyhow::anyhow!("a rename without its old path"))?;
                git(&self.root, &["reset", "--quiet", "--", &old, &rel])?;
                git(&self.root, &["checkout", "--quiet", "HEAD", "--", &old])?;
                if entry.path.exists() {
                    trash_file(&entry.path)?;
                }
                Ok(())
            }
            Status::Modified | Status::Deleted | Status::Conflict => {
                let _ = git(&self.root, &["reset", "--quiet", "--", &rel]);
                git(&self.root, &["checkout", "--quiet", "HEAD", "--", &rel]).map(|_| ())
            }
            Status::Ignored => Ok(()),
        }
    }
}

fn trash_file(path: &Path) -> Result<()> {
    trash::delete(path).with_context(|| format!("trash {}", path.display()))
}

/// Runs git for its report (push and pull write theirs to stderr), failing
/// with that report when the command does.
fn git_report(dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .with_context(|| format!("git {}", args.join(" ")))?;
    let mut text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !err.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&err);
    }
    if output.status.success() {
        Ok(text)
    } else {
        anyhow::bail!("git {}: {text}", args.join(" "))
    }
}

// ---- line diffs, for the Changed Files tab -----------------------------------------

/// One row of a line diff: both texts keep the line, or one side alone has it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineOp {
    /// The old row and the new row that match.
    Keep(usize, usize),
    /// A row only the old text has.
    Remove(usize),
    /// A row only the new text has.
    Add(usize),
}

/// Past this many edits the two texts count as wholly replaced.
const MAX_EDITS: usize = 1000;

/// A line diff of `old` against `new` (Myers' O(ND) algorithm, the edit
/// script git itself prints), in row order.
pub fn diff_lines<T: PartialEq>(old: &[T], new: &[T]) -> Vec<LineOp> {
    let n = old.len() as isize;
    let m = new.len() as isize;
    let max = (n + m) as usize;
    if max == 0 {
        return Vec::new();
    }
    // k runs -d..=d with d ≤ max; one spare slot each side keeps k ± 1 in range.
    let offset = max as isize + 1;
    let mut v = vec![0isize; 2 * max + 3];
    // For each round d, the frontier before it over k in -d-1..=d+1.
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let mut reached = None;
    'rounds: for d in 0..=max.min(MAX_EDITS) {
        let d = d as isize;
        let lo = (offset - d - 1) as usize;
        let hi = (offset + d + 1) as usize;
        trace.push(v[lo..=hi].to_vec());
        let mut k = -d;
        while k <= d {
            let idx = (k + offset) as usize;
            let mut x = if k == -d || (k != d && v[idx - 1] < v[idx + 1]) {
                v[idx + 1]
            } else {
                v[idx - 1] + 1
            };
            let mut y = x - k;
            while x < n && y < m && old[x as usize] == new[y as usize] {
                x += 1;
                y += 1;
            }
            v[idx] = x;
            if x >= n && y >= m {
                reached = Some(d);
                break 'rounds;
            }
            k += 2;
        }
    }
    let Some(last) = reached else {
        let mut ops: Vec<LineOp> = (0..old.len()).map(LineOp::Remove).collect();
        ops.extend((0..new.len()).map(LineOp::Add));
        return ops;
    };
    let mut ops = Vec::new();
    let (mut x, mut y) = (n, m);
    for d in (0..=last).rev() {
        let v = &trace[d as usize];
        let at = |k: isize| v[(k + d + 1) as usize];
        let k = x - y;
        let prev_k = if k == -d || (k != d && at(k - 1) < at(k + 1)) {
            k + 1
        } else {
            k - 1
        };
        let (prev_x, prev_y) = if d == 0 {
            (0, 0)
        } else {
            let px = at(prev_k);
            (px, px - prev_k)
        };
        while x > prev_x && y > prev_y {
            x -= 1;
            y -= 1;
            ops.push(LineOp::Keep(x as usize, y as usize));
        }
        if d > 0 {
            if x == prev_x {
                ops.push(LineOp::Add(prev_y as usize));
            } else {
                ops.push(LineOp::Remove(prev_x as usize));
            }
            x = prev_x;
            y = prev_y;
        }
    }
    ops.reverse();
    ops
}

// ---- dates, as Fleet prints them ---------------------------------------------------

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `+0100` → seconds east of UTC.
fn tz_seconds(tz: &str) -> i64 {
    let (sign, digits) = match tz.chars().next() {
        Some('-') => (-1, &tz[1..]),
        Some('+') => (1, &tz[1..]),
        _ => (1, tz),
    };
    let hours: i64 = digits.get(..2).and_then(|h| h.parse().ok()).unwrap_or(0);
    let minutes: i64 = digits.get(2..4).and_then(|m| m.parse().ok()).unwrap_or(0);
    sign * (hours * 3600 + minutes * 60)
}

/// A unix time in the author's zone as the blame column shows it: `Dec 31, 2025`.
pub fn format_day(unix: i64, tz: &str) -> String {
    let local = unix + tz_seconds(tz);
    let (y, m, d) = civil_from_days(local.div_euclid(86_400));
    format!("{} {d}, {y}", MONTHS[(m - 1) as usize])
}

/// An ISO 8601 date (`%aI`) as the commit rows show it: `18:00 Dec 31, 2025`.
pub fn format_iso(iso: &str) -> String {
    let (date, rest) = iso.split_once('T').unwrap_or((iso, ""));
    let mut parts = date.split('-');
    let (Some(y), Some(m), Some(d)) = (parts.next(), parts.next(), parts.next()) else {
        return iso.to_string();
    };
    let month = m
        .parse::<usize>()
        .ok()
        .and_then(|m| MONTHS.get(m.wrapping_sub(1)))
        .copied()
        .unwrap_or(m);
    let day = d.trim_start_matches('0');
    let time = rest.get(..5).unwrap_or("");
    if time.is_empty() {
        format!("{month} {day}, {y}")
    } else {
        format!("{time} {month} {day}, {y}")
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
                    tz: "+0000".to_string(),
                    summary: String::new(),
                    uncommitted: sha.bytes().all(|b| b == b'0'),
                });
            }
            Some(entry) => {
                if let Some(v) = line.strip_prefix("author ") {
                    entry.author = v.to_string();
                } else if let Some(v) = line.strip_prefix("author-time ") {
                    entry.date = v.to_string();
                } else if let Some(v) = line.strip_prefix("author-tz ") {
                    entry.tz = v.to_string();
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
    fn root_keeps_the_callers_spelling() {
        // A mapped drive: git prints the UNC path, the workspace is `Z:`.
        let dir = Path::new(if cfg!(windows) {
            "Z:\\repos\\oak"
        } else {
            "/mnt/z/repos/oak"
        });
        assert_eq!(root_spelled_like(dir, "//server/share/repos/oak", ""), dir);
        let sub = dir.join("crates").join("oak");
        assert_eq!(
            root_spelled_like(&sub, "//server/share/repos/oak", "crates/oak/"),
            dir
        );
        // Components that do not line up fall back to git's answer.
        assert_eq!(
            root_spelled_like(dir, "//server/share/elsewhere", ""),
            PathBuf::from("//server/share/elsewhere")
        );
        assert_eq!(
            root_spelled_like(dir, "//server/share/repos/oak", "deeper/than/dir/"),
            PathBuf::from("//server/share/repos/oak")
        );
    }

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
    fn rows_from_hunks() {
        let hunks = vec![
            Hunk {
                row: 1,
                rows: 1,
                kind: HunkKind::Modified,
                old: vec!["b".to_string()],
            },
            Hunk {
                row: 3,
                rows: 0,
                kind: HunkKind::Deleted,
                old: vec!["d".to_string()],
            },
            Hunk {
                row: 3,
                rows: 2,
                kind: HunkKind::Added,
                old: vec![],
            },
        ];
        // old: a b c d e; new: a x c y z e
        assert_eq!(
            ops_from_hunks(&hunks, 5, 6),
            Some(vec![
                LineOp::Keep(0, 0),
                LineOp::Remove(1),
                LineOp::Add(1),
                LineOp::Keep(2, 2),
                LineOp::Remove(3),
                LineOp::Add(3),
                LineOp::Add(4),
                LineOp::Keep(4, 5),
            ])
        );
        assert_eq!(ops_from_hunks(&hunks, 4, 6), None, "the old text is short");
        assert_eq!(
            ops_from_hunks(&[], 2, 2),
            Some(vec![LineOp::Keep(0, 0), LineOp::Keep(1, 1)])
        );
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

        let blame = "abc123def4567890 1 1 1\nauthor Ada\nauthor-mail <a>\nauthor-time 1762074000\nauthor-tz +0100\nsummary First\nfilename f\n\tline one\n0000000000000000000000000000000000000000 2 2 1\nauthor Not Committed Yet\nauthor-time 1762074001\nsummary Version of f from f\nfilename f\n\tline two\n";
        let lines = parse_blame(blame);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].short_id, "abc123d");
        assert_eq!(lines[0].author, "Ada");
        assert_eq!(lines[0].date, "1762074000");
        assert_eq!(lines[0].tz, "+0100");
        assert_eq!(lines[0].summary, "First");
        assert!(!lines[0].uncommitted);
        assert_eq!(lines[0].label().as_deref(), Some("Nov 2, 2025  Ada"));
        assert!(lines[1].uncommitted);
        assert_eq!(lines[1].label(), None);
    }

    #[test]
    fn formats_dates() {
        // 2025-12-31T18:00:00+01:00 is 17:00 UTC: 1767200400.
        assert_eq!(format_day(1_767_200_400, "+0100"), "Dec 31, 2025");
        assert_eq!(format_day(1_767_200_400, "+0000"), "Dec 31, 2025");
        // Just before midnight in Tokyo is the next day there.
        assert_eq!(format_day(1_767_200_400, "+0900"), "Jan 1, 2026");
        assert_eq!(format_day(0, "+0000"), "Jan 1, 1970");
        assert_eq!(format_day(-1, "+0000"), "Dec 31, 1969");
        assert_eq!(format_day(951_782_400, "+0000"), "Feb 29, 2000");
        assert_eq!(
            format_iso("2025-12-31T18:00:00+01:00"),
            "18:00 Dec 31, 2025"
        );
        assert_eq!(format_iso("2026-03-05"), "Mar 5, 2026");
        assert_eq!(format_iso("soon"), "soon");
    }

    #[test]
    fn diffs_lines() {
        let old = ["a", "b", "c", "d"];
        let new = ["a", "x", "c", "d", "e"];
        assert_eq!(
            diff_lines(&old, &new),
            vec![
                LineOp::Keep(0, 0),
                LineOp::Remove(1),
                LineOp::Add(1),
                LineOp::Keep(2, 2),
                LineOp::Keep(3, 3),
                LineOp::Add(4),
            ]
        );
        assert_eq!(diff_lines(&["a"], &["a"]), vec![LineOp::Keep(0, 0)]);
        assert_eq!(diff_lines::<&str>(&[], &[]), vec![]);
        assert_eq!(diff_lines(&[], &["a"]), vec![LineOp::Add(0)]);
        assert_eq!(diff_lines(&["a"], &[]), vec![LineOp::Remove(0)]);
        assert_eq!(
            diff_lines(&["a", "b"], &["b", "a"]),
            vec![LineOp::Remove(0), LineOp::Keep(1, 0), LineOp::Add(1)]
        );
        // Every op accounts for every row of both sides, in order.
        let old: Vec<String> = (0..300).map(|i| format!("l{}", i * 3 % 17)).collect();
        let new: Vec<String> = (0..320).map(|i| format!("l{}", i * 5 % 13)).collect();
        let ops = diff_lines(&old, &new);
        let (mut o, mut n) = (0, 0);
        for op in &ops {
            match *op {
                LineOp::Keep(a, b) => {
                    assert_eq!((a, b), (o, n));
                    assert_eq!(old[a], new[b]);
                    o += 1;
                    n += 1;
                }
                LineOp::Remove(a) => {
                    assert_eq!(a, o);
                    o += 1;
                }
                LineOp::Add(b) => {
                    assert_eq!(b, n);
                    n += 1;
                }
            }
        }
        assert_eq!((o, n), (old.len(), new.len()));
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
        // Windows installs set `core.autocrlf`; the test compares bytes.
        run(&["config", "core.autocrlf", "false"]);
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
        assert_eq!(
            repo.head_text(&repo.root.join("kept.txt")).as_deref(),
            Some("a\n")
        );
        assert_eq!(repo.head_text(&repo.root.join("new.txt")), None);
        assert_eq!(repo.upstream(), None);
        // Revert: the modified file takes HEAD's text back.
        let kept = repo.root.join("kept.txt");
        let entry = repo
            .status_entries()
            .unwrap()
            .into_iter()
            .find(|e| e.path == kept)
            .unwrap();
        repo.revert(&entry).unwrap();
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), "a\n");
        assert!(!repo.status().unwrap().contains_key(&kept));
        std::fs::write(&kept, "b\n").unwrap();
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
        assert_eq!(clone.upstream().as_deref(), Some("origin/trunk"));
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
