//! Which language server runs for which language, and where it is on this
//! machine. Fleet's `toolchains` setting names the toolchains Smart Mode
//! may use; WEEP reads it but, with rust-analyzer the only server so far,
//! finds the server on `PATH` and, failing that, through rustup.

use std::path::{Path, PathBuf};

/// A server to start: its presentable name, the program and its arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerCommand {
    pub name: String,
    pub program: PathBuf,
    pub args: Vec<String>,
}

/// The LSP language id of one of WEEP's languages (`weep_syntax::Language`
/// is named after the file type; the id is what servers expect).
pub fn language_id(language: &str) -> &'static str {
    match language.to_ascii_lowercase().as_str() {
        "rust" => "rust",
        "json" => "json",
        "toml" => "toml",
        "markdown" => "markdown",
        _ => "plaintext",
    }
}

/// The server for a language id, when this machine has one.
pub fn for_language(language_id: &str) -> Option<ServerCommand> {
    match language_id {
        "rust" => rust_analyzer(),
        _ => None,
    }
}

/// rust-analyzer: on `PATH`, else the toolchain's component through
/// `rustup which` (rustup's own `rust-analyzer` proxy on `PATH` fails when
/// the component is missing, so the proxy is checked by running it).
pub fn rust_analyzer() -> Option<ServerCommand> {
    let candidates = find_on_path("rust-analyzer")
        .into_iter()
        .chain(rustup_which("rust-analyzer"));
    for program in candidates {
        if runs(&program) {
            return Some(ServerCommand {
                name: "rust-analyzer".into(),
                program,
                args: Vec::new(),
            });
        }
    }
    None
}

/// `program --version` exits 0: the binary exists and is not a dead proxy.
fn runs(program: &Path) -> bool {
    let mut cmd = std::process::Command::new(program);
    cmd.arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.status().is_ok_and(|s| s.success())
}

fn rustup_which(name: &str) -> Option<PathBuf> {
    let mut cmd = std::process::Command::new("rustup");
    cmd.args(["which", name])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let path = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    path.is_file().then_some(path)
}

/// The first `name` (with the platform's executable suffixes) on `PATH`.
pub fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let names: Vec<String> = if cfg!(windows) {
        vec![
            format!("{name}.exe"),
            format!("{name}.cmd"),
            name.to_string(),
        ]
    } else {
        vec![name.to_string()]
    };
    for dir in std::env::split_paths(&path) {
        for n in &names {
            let candidate = dir.join(n);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(windows)]
pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_ids() {
        assert_eq!(language_id("Rust"), "rust");
        assert_eq!(language_id("Markdown"), "markdown");
        assert_eq!(language_id("Text"), "plaintext");
    }

    #[test]
    fn cargo_is_on_the_path() {
        // The test runs under cargo, so cargo is there; the lookup must find it.
        assert!(find_on_path("cargo").is_some());
        assert!(find_on_path("weep-no-such-program").is_none());
    }
}
