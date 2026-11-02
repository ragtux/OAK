//! Shell profiles: Fleet's `terminal.profiles` rows and the shells a machine
//! offers without any.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One row of `terminal.profiles`, as Fleet's `Profile` class stores it:
/// a name for the chooser, the program and its arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
}

impl Profile {
    pub fn new(name: impl Into<String>, program: impl Into<String>, args: Vec<String>) -> Profile {
        Profile {
            name: name.into(),
            program: program.into(),
            args,
        }
    }

    /// The rows of the setting's value (an array of `{name, program, args}`
    /// objects); rows that do not parse are skipped, a missing value is none.
    pub fn from_setting(value: Option<&serde_json::Value>) -> Vec<Profile> {
        match value {
            Some(serde_json::Value::Array(rows)) => rows
                .iter()
                .filter_map(|row| serde_json::from_value::<Profile>(row.clone()).ok())
                .filter(|p| !p.program.is_empty())
                .map(|mut p| {
                    if p.name.is_empty() {
                        p.name = program_name(&p.program);
                    }
                    p
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The shells on this machine, named as Fleet names its built-in
    /// profiles (`pwsh`, `powershell`, `cmd` on Windows; `$SHELL` first,
    /// then `bash`, `zsh`, `fish` and `sh` elsewhere).
    pub fn discover() -> Vec<Profile> {
        let path: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        let shell = std::env::var("SHELL").ok();
        Self::discover_in(&path, shell.as_deref(), cfg!(windows))
    }

    /// [`discover`](Self::discover) against a given `PATH` and `$SHELL`.
    pub fn discover_in(path: &[PathBuf], shell: Option<&str>, windows: bool) -> Vec<Profile> {
        let mut found: Vec<Profile> = Vec::new();
        let mut add = |name: &str, program: String| {
            if !found.iter().any(|p| p.name == name) {
                found.push(Profile::new(name, program, Vec::new()));
            }
        };
        if windows {
            for name in ["pwsh", "powershell", "cmd"] {
                if let Some(p) = find_program(name, path, true) {
                    add(name, p.display().to_string());
                }
            }
        } else {
            if let Some(shell) = shell.filter(|s| !s.is_empty()) {
                add(&program_name(shell), shell.to_string());
            }
            for name in ["bash", "zsh", "fish", "sh"] {
                if let Some(p) = find_program(name, path, false) {
                    add(name, p.display().to_string());
                }
            }
        }
        found
    }

    /// The profile `terminal.profile.default` names, else the first one.
    pub fn default_of<'a>(name: Option<&str>, profiles: &'a [Profile]) -> Option<&'a Profile> {
        name.filter(|n| !n.is_empty())
            .and_then(|n| profiles.iter().find(|p| p.name.eq_ignore_ascii_case(n)))
            .or_else(|| profiles.first())
    }
}

/// `pwsh` for `C:\Program Files\PowerShell\7\pwsh.exe`, `zsh` for `/bin/zsh`.
pub fn program_name(program: &str) -> String {
    let name = program
        .rsplit(['/', '\\'])
        .next()
        .filter(|n| !n.is_empty())
        .unwrap_or(program)
        .to_string();
    match name
        .strip_suffix(".exe")
        .or_else(|| name.strip_suffix(".EXE"))
    {
        Some(stem) => stem.to_string(),
        None => name,
    }
}

/// The first `name` (plus `.exe` on Windows) on `path`.
pub fn find_program(name: &str, path: &[PathBuf], windows: bool) -> Option<PathBuf> {
    let candidates: Vec<String> = if windows {
        vec![format!("{name}.exe"), name.to_string()]
    } else {
        vec![name.to_string()]
    };
    path.iter()
        .flat_map(|dir| candidates.iter().map(move |c| dir.join(c)))
        .find(|p: &PathBuf| Path::is_file(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fleet_rows() {
        let value = serde_json::json!([
            {"name": "zsh", "program": "/bin/zsh", "args": ["-l"]},
            {"name": "", "program": "/usr/bin/fish"},
            {"program": ""},
            "not a row"
        ]);
        let profiles = Profile::from_setting(Some(&value));
        assert_eq!(
            profiles,
            vec![
                Profile::new("zsh", "/bin/zsh", vec!["-l".into()]),
                Profile::new("fish", "/usr/bin/fish", vec![]),
            ]
        );
        assert!(Profile::from_setting(None).is_empty());
    }

    #[test]
    fn names_programs() {
        assert_eq!(
            program_name(r"C:\Program Files\PowerShell\7\pwsh.exe"),
            "pwsh"
        );
        assert_eq!(program_name("/bin/zsh"), "zsh");
        assert_eq!(program_name("bash"), "bash");
    }

    #[test]
    fn picks_the_default_by_name() {
        let all = vec![
            Profile::new("pwsh", "pwsh.exe", vec![]),
            Profile::new("cmd", "cmd.exe", vec![]),
        ];
        assert_eq!(Profile::default_of(Some("CMD"), &all).unwrap().name, "cmd");
        assert_eq!(
            Profile::default_of(Some("fish"), &all).unwrap().name,
            "pwsh"
        );
        assert_eq!(Profile::default_of(None, &all).unwrap().name, "pwsh");
        assert!(Profile::default_of(None, &[]).is_none());
    }

    #[test]
    fn discovers_shells_on_a_path() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["bash", "zsh.exe", "pwsh.exe"] {
            std::fs::write(dir.path().join(name), "").unwrap();
        }
        let path = vec![dir.path().to_path_buf()];
        let unix = Profile::discover_in(&path, Some("/usr/bin/fish"), false);
        assert_eq!(
            unix.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["fish", "bash"]
        );
        let windows = Profile::discover_in(&path, None, true);
        assert_eq!(
            windows.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["pwsh"]
        );
    }
}
