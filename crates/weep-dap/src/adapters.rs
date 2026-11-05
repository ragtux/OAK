//! Where the debug adapter is on this machine, and what it needs to show
//! Rust well. lldb-dap (LLVM's adapter, `lldb-vscode` before LLVM 18)
//! debugs C, C++ and Rust through LLDB; Rust's own value formatters ship
//! with the toolchain (`rust-lldb` loads them) and are loaded here through
//! the launch's `initCommands`.

use crate::AdapterCommand;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

impl AdapterCommand {
    /// lldb-dap: on `PATH`, then `xcrun -f lldb-dap` on macOS, then the
    /// versioned LLVM installs on Linux (`lldb-dap-NN` on `PATH`,
    /// `/usr/lib/llvm-NN/bin/lldb-dap`), then `lldb-vscode` (its old name)
    /// the same ways. `None` when none is installed.
    pub fn find_lldb() -> Option<AdapterCommand> {
        let program = lldb_candidates().into_iter().find(|p| p.is_file())?;
        Some(AdapterCommand {
            name: "lldb-dap".into(),
            program,
            args: Vec::new(),
        })
    }
}

fn lldb_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for name in ["lldb-dap", "lldb-vscode"] {
        out.extend(find_on_path(name));
        if cfg!(target_os = "macos") {
            out.extend(xcrun_find(name));
        }
        if cfg!(target_os = "linux") {
            out.extend(versioned_on_path(name));
            out.extend(versioned_llvm(name));
        }
    }
    out
}

/// `xcrun -f <name>`: Xcode's (or the command line tools') copy.
fn xcrun_find(name: &str) -> Option<PathBuf> {
    let out = std::process::Command::new("xcrun")
        .args(["-f", name])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    path.is_file().then_some(path)
}

/// `<name>-NN` anywhere on `PATH` (Debian's and Ubuntu's packages), newest
/// version first.
fn versioned_on_path(name: &str) -> Vec<PathBuf> {
    let Some(path) = std::env::var_os("PATH") else {
        return Vec::new();
    };
    let mut found: Vec<(u32, PathBuf)> = Vec::new();
    for dir in std::env::split_paths(&path) {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let file = entry.file_name();
            let file = file.to_string_lossy();
            if let Some(v) = version_suffix(&file, name) {
                found.push((v, entry.path()));
            }
        }
    }
    found.sort_by_key(|f| std::cmp::Reverse(f.0));
    found.into_iter().map(|(_, p)| p).collect()
}

/// `/usr/lib/llvm-NN/bin/<name>`, newest version first.
fn versioned_llvm(name: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir("/usr/lib") else {
        return Vec::new();
    };
    let mut found: Vec<(u32, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let file = e.file_name();
            let v = version_suffix(&file.to_string_lossy(), "llvm")?;
            Some((v, e.path().join("bin").join(name)))
        })
        .collect();
    found.sort_by_key(|f| std::cmp::Reverse(f.0));
    found.into_iter().map(|(_, p)| p).collect()
}

/// `NN` out of `<name>-NN`.
fn version_suffix(file: &str, name: &str) -> Option<u32> {
    file.strip_prefix(name)?.strip_prefix('-')?.parse().ok()
}

/// The first `name` (with the platform's executable suffix) on `PATH`.
pub fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    std::env::split_paths(&path)
        .map(|dir| dir.join(&file))
        .find(|candidate| candidate.is_file())
}

/// The LLDB commands that teach LLDB Rust's types (what `rust-lldb` does):
/// the toolchain's `lldb_lookup.py` (with `lldb_commands` on toolchains
/// that still ship it) and, when the `rust-src` component is installed, a
/// source map from the `/rustc/<commit>` paths in std's debug info to it.
/// Empty without rustup's `rustc`, or for a toolchain without the files.
/// Computed once per process.
pub fn rust_init_commands() -> Vec<String> {
    static COMMANDS: OnceLock<Vec<String>> = OnceLock::new();
    COMMANDS
        .get_or_init(|| {
            let Some(sysroot) = rustc_line(&["--print", "sysroot"]).map(PathBuf::from) else {
                return Vec::new();
            };
            let commit = rustc_commit();
            rust_init_commands_for(&sysroot, commit.as_deref())
        })
        .clone()
}

/// [`rust_init_commands`] for a given sysroot and rustc commit hash.
pub fn rust_init_commands_for(sysroot: &Path, commit: Option<&str>) -> Vec<String> {
    let etc = sysroot.join("lib/rustlib/etc");
    let lookup = etc.join("lldb_lookup.py");
    let mut out = Vec::new();
    if !lookup.is_file() {
        return out;
    }
    out.push(format!("command script import {}", quoted(&lookup)));
    let commands = etc.join("lldb_commands");
    if commands.is_file() {
        out.push(format!("command source -s true {}", quoted(&commands)));
    }
    let src = sysroot.join("lib/rustlib/src/rust");
    if let Some(commit) = commit.filter(|c| !c.is_empty()) {
        if src.is_dir() {
            out.push(format!(
                "settings append target.source-map /rustc/{commit} {}",
                quoted(&src)
            ));
        }
    }
    out
}

fn quoted(path: &Path) -> String {
    format!("\"{}\"", path.display().to_string().replace('"', "\\\""))
}

fn rustc_output(args: &[&str]) -> Option<String> {
    let mut cmd = std::process::Command::new("rustc");
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(crate::client::CREATE_NO_WINDOW);
    }
    let out = cmd.output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn rustc_line(args: &[&str]) -> Option<String> {
    let text = rustc_output(args)?;
    let line = text.lines().next()?.trim().to_string();
    (!line.is_empty()).then_some(line)
}

/// `commit-hash:` out of `rustc -vV`.
fn rustc_commit() -> Option<String> {
    let text = rustc_output(&["-vV"])?;
    commit_hash(&text)
}

fn commit_hash(verbose_version: &str) -> Option<String> {
    verbose_version
        .lines()
        .find_map(|l| l.strip_prefix("commit-hash:"))
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty() && h != "unknown")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_read_off_the_name() {
        assert_eq!(version_suffix("lldb-dap-18", "lldb-dap"), Some(18));
        assert_eq!(version_suffix("llvm-17", "llvm"), Some(17));
        assert_eq!(version_suffix("lldb-dap", "lldb-dap"), None);
        assert_eq!(version_suffix("lldb-dapper", "lldb-dap"), None);
    }

    #[test]
    fn the_commit_hash_comes_from_rustc_vv() {
        let text = "rustc 1.98.1 (48a229cea 2026-09-01)\nbinary: rustc\ncommit-hash: 48a229ceaffe\nhost: aarch64-apple-darwin\n";
        assert_eq!(commit_hash(text).as_deref(), Some("48a229ceaffe"));
        assert_eq!(commit_hash("commit-hash: unknown\n"), None);
    }

    #[test]
    fn rust_formatters_load_from_the_sysroot() {
        let dir = tempfile::tempdir().unwrap();
        assert!(rust_init_commands_for(dir.path(), Some("abc")).is_empty());
        let etc = dir.path().join("lib/rustlib/etc");
        std::fs::create_dir_all(&etc).unwrap();
        std::fs::write(etc.join("lldb_lookup.py"), "").unwrap();
        let got = rust_init_commands_for(dir.path(), Some("abc"));
        assert_eq!(got.len(), 1);
        assert!(got[0].starts_with("command script import \""));
        assert!(got[0].ends_with("lldb_lookup.py\""));
        std::fs::write(etc.join("lldb_commands"), "").unwrap();
        std::fs::create_dir_all(dir.path().join("lib/rustlib/src/rust")).unwrap();
        let got = rust_init_commands_for(dir.path(), Some("abc"));
        assert_eq!(got.len(), 3);
        assert!(got[1].starts_with("command source -s true \""));
        assert!(got[2].starts_with("settings append target.source-map /rustc/abc \""));
    }

    #[test]
    fn the_path_lookup_finds_a_program() {
        // The test runs under cargo, so cargo is there.
        assert!(find_on_path("cargo").is_some());
        assert!(find_on_path("weep-no-such-program").is_none());
    }
}
