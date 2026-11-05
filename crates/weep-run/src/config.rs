//! `.fleet/run.json`: `{"configurations": [{"type": ..., "name": ..., ...}]}`.
//! Parsed with serde (unknown keys kept, so a rewrite loses nothing) and
//! scanned once more for the line each configuration's `"type"` key sits on,
//! which is where Fleet puts the gutter's run icon (batch 11).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The file Fleet writes for "Edit Run Configurations..." when there is none
/// (estimate: an empty list).
pub const TEMPLATE: &str = "{\n  \"configurations\": [\n  ]\n}\n";

/// The relative path of the file in a workspace.
pub const FILE: &str = ".fleet/run.json";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunConfig {
    /// `name`: "Allows to refer to the configuration and distinguish it from others."
    #[serde(default)]
    pub name: String,
    /// `type`: `command`, `cargo`, `gradle`, ... WEEP runs the first two.
    #[serde(rename = "type", default)]
    pub kind: String,
    /// `command`: "Name of the shell program which will be executed."
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    /// `command`: "List of the args passed to program."
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// `cargo`: the arguments after `cargo`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cargo_args: Vec<String>,
    /// "Specify working directory for this run configuration." Relative to
    /// the workspace when not absolute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    /// "Specify environment variables which will be passed to configuration
    /// on run. They will override variables obtained from 'environmentFile'."
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub environment: BTreeMap<String, String>,
    /// "Specify a file to read environment variables from for this run
    /// configuration."
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_file: Option<String>,
    /// "List of names of other configurations which will be executed before
    /// this configuration."
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    /// "If true - more than one instance of this configurations can be run
    /// in parallel."
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub allow_parallel_run: bool,
    /// Every other key, kept for a rewrite.
    #[serde(flatten)]
    pub rest: serde_json::Map<String, serde_json::Value>,
    /// The 0-based line of the configuration's `"type"` key (its first key
    /// when it has none), where the gutter's run icon goes. Not in the file.
    #[serde(skip)]
    pub line: Option<usize>,
}

/// The whole file: the configurations and whatever else it holds.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RunFile {
    #[serde(default)]
    pub configurations: Vec<RunConfig>,
    #[serde(flatten)]
    pub rest: serde_json::Map<String, serde_json::Value>,
}

impl RunFile {
    /// Parses the file's text; the error is serde's message.
    pub fn parse(text: &str) -> Result<RunFile, String> {
        let mut file: RunFile = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let lines = config_lines(text);
        for (config, line) in file.configurations.iter_mut().zip(lines) {
            config.line = line;
        }
        Ok(file)
    }

    /// The file's text with 2-space indentation, as Fleet writes it.
    pub fn to_text(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).unwrap_or_default();
        text.push('\n');
        text
    }

    /// The file without configuration `index` (`run/delete-configuration`).
    pub fn without(&self, index: usize) -> RunFile {
        let mut file = self.clone();
        if index < file.configurations.len() {
            file.configurations.remove(index);
        }
        file
    }
}

/// The line of each configuration object's `"type"` key (the object's first
/// line when it has none), in file order: a scan of the text that tracks
/// strings and nesting, so a file serde rejects still gets icons where it can.
pub fn config_lines(text: &str) -> Vec<Option<usize>> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut line = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut in_configurations = false;
    let mut array_depth = 0usize;
    // The object currently open at the configuration level and whether its
    // `"type"` was seen.
    let mut current: Option<(usize, bool)> = None;
    let mut string_start = 0usize;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                in_string = false;
                let key = &text[string_start..i];
                // A key is a string followed by a colon.
                let rest = text[i + 1..].trim_start();
                let is_key = rest.starts_with(':');
                if is_key {
                    if depth == 1 && key == "configurations" {
                        in_configurations = true;
                    } else if let Some((_, seen)) = &mut current {
                        if depth == array_depth + 1 && key == "type" && !*seen {
                            *seen = true;
                            if let Some(last) = out.last_mut() {
                                *last = Some(line);
                            }
                        }
                    }
                }
            }
            i += 1;
            continue;
        }
        match c {
            b'"' => {
                in_string = true;
                string_start = i + 1;
            }
            b'\n' => line += 1,
            b'[' => {
                depth += 1;
                if in_configurations && array_depth == 0 && depth == 2 {
                    array_depth = depth;
                }
            }
            b']' => {
                if depth == array_depth && array_depth > 0 {
                    in_configurations = false;
                    array_depth = 0;
                }
                depth = depth.saturating_sub(1);
            }
            b'{' => {
                depth += 1;
                if array_depth > 0 && depth == array_depth + 1 {
                    current = Some((line, false));
                    out.push(Some(line));
                }
            }
            b'}' => {
                if array_depth > 0 && depth == array_depth + 1 {
                    current = None;
                }
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// What a configuration runs: the program, its arguments, the directory,
/// the environment on top of the inherited one, and the line Fleet prints
/// at the top of the output (`cmd /c echo hello from the fixture`).
#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub display: String,
}

impl Command {
    /// A command line typed into the Run & Debug popup, run through the
    /// platform's shell.
    pub fn shell(text: &str, root: &Path) -> Command {
        let (program, flag) = if cfg!(windows) {
            ("cmd", "/c")
        } else {
            ("sh", "-c")
        };
        Command {
            program: program.to_string(),
            args: vec![flag.to_string(), text.to_string()],
            cwd: root.to_path_buf(),
            env: Vec::new(),
            display: text.to_string(),
        }
    }
}

impl Command {
    /// The program as a path a debug adapter can launch (M8): a path with a
    /// directory part against `cwd`, a bare name looked up in `path` (the
    /// `PATH` value), else the name itself.
    pub fn program_path(&self, path: Option<&std::ffi::OsStr>) -> PathBuf {
        resolve_program(&self.program, &self.cwd, path)
    }
}

/// [`Command::program_path`] on its own.
pub fn resolve_program(program: &str, cwd: &Path, path: Option<&std::ffi::OsStr>) -> PathBuf {
    let p = Path::new(program);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    if p.components().count() > 1 {
        return cwd.join(p);
    }
    if let Some(path) = path {
        for dir in std::env::split_paths(path) {
            let candidate = dir.join(program);
            if candidate.is_file() {
                return candidate;
            }
            if cfg!(windows) {
                let exe = dir.join(format!("{program}.exe"));
                if exe.is_file() {
                    return exe;
                }
            }
        }
    }
    p.to_path_buf()
}

/// `KEY=VALUE` lines of an environment file; `#` lines and blank ones skipped.
pub fn parse_env_file(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            let v = v.trim().trim_matches('"').trim_matches('\'');
            Some((k.trim().to_string(), v.to_string()))
        })
        .collect()
}

impl RunConfig {
    /// The command for this configuration in workspace `root`, with
    /// `default_env_file` (`run.environmentFile`) read when the row names
    /// none of its own. Types WEEP cannot run give an error naming them.
    pub fn command(&self, root: &Path, default_env_file: Option<&Path>) -> Result<Command, String> {
        let (program, args) = match self.kind.as_str() {
            "command" => {
                let program = self
                    .program
                    .clone()
                    .filter(|p| !p.is_empty())
                    .ok_or_else(|| format!("'{}' has no program", self.name))?;
                (program, self.args.clone())
            }
            "cargo" => ("cargo".to_string(), self.cargo_args.clone()),
            "" => {
                return Err(format!("'{}' has no type", self.name));
            }
            other => {
                return Err(format!(
                    "WEEP cannot run '{}' configurations ('{}')",
                    other, self.name
                ))
            }
        };
        let cwd = match &self.working_dir {
            Some(dir) if !dir.is_empty() => {
                let p = Path::new(dir);
                if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    root.join(p)
                }
            }
            _ => root.to_path_buf(),
        };
        let mut env = Vec::new();
        let env_file = self
            .environment_file
            .as_deref()
            .filter(|f| !f.is_empty())
            .map(|f| {
                let p = Path::new(f);
                if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    root.join(p)
                }
            })
            .or_else(|| default_env_file.map(Path::to_path_buf));
        if let Some(file) = env_file {
            if let Ok(text) = std::fs::read_to_string(&file) {
                env.extend(parse_env_file(&text));
            }
        }
        for (k, v) in &self.environment {
            env.retain(|(ek, _)| ek != k);
            env.push((k.clone(), v.clone()));
        }
        let display = std::iter::once(program.as_str())
            .chain(args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        Ok(Command {
            program,
            args,
            cwd,
            env,
            display,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
  "configurations": [
    {
      "type": "command",
      "name": "echo hello",
      "program": "cmd",
      "args": ["/c", "echo hello from the fixture"]
    },
    {
      "name": "fail",
      "type": "command",
      "program": "cmd",
      "args": ["/c", "exit 3"],
      "workingDir": "sub",
      "environment": {"A": "1"}
    },
    {
      "type": "cargo",
      "name": "run fixture",
      "cargoArgs": ["run"]
    }
  ]
}
"#;

    #[test]
    fn parses_fleets_fields_and_keeps_the_rest() {
        let file = RunFile::parse(FIXTURE).unwrap();
        assert_eq!(file.configurations.len(), 3);
        let first = &file.configurations[0];
        assert_eq!(first.name, "echo hello");
        assert_eq!(first.kind, "command");
        assert_eq!(first.program.as_deref(), Some("cmd"));
        assert_eq!(first.args, vec!["/c", "echo hello from the fixture"]);
        let second = &file.configurations[1];
        assert_eq!(second.working_dir.as_deref(), Some("sub"));
        assert_eq!(second.environment.get("A").map(String::as_str), Some("1"));
        let third = &file.configurations[2];
        assert_eq!(third.cargo_args, vec!["run"]);
        assert!(third.rest.is_empty());
    }

    #[test]
    fn type_lines_are_where_fleet_puts_the_icons() {
        let file = RunFile::parse(FIXTURE).unwrap();
        let lines: Vec<_> = file.configurations.iter().map(|c| c.line).collect();
        // Lines are 0-based: the fixture's "type" keys are on lines 4, 11 and 18 (1-based).
        assert_eq!(lines, vec![Some(3), Some(10), Some(17)]);
    }

    #[test]
    fn a_configuration_without_a_type_gets_its_first_line() {
        let text = "{\n \"configurations\": [\n  {\n   \"name\": \"x\"\n  }\n ]\n}";
        assert_eq!(config_lines(text), vec![Some(2)]);
        let broken =
            "{\"configurations\": [{\"type\": \"a\", \"args\": [\"{\"]}, {\"type\": \"b\"}";
        assert_eq!(config_lines(broken), vec![Some(0), Some(0)]);
    }

    #[test]
    fn commands_resolve_per_type() {
        let file = RunFile::parse(FIXTURE).unwrap();
        let root = Path::new("/work");
        let c = file.configurations[0].command(root, None).unwrap();
        assert_eq!(c.program, "cmd");
        assert_eq!(c.display, "cmd /c echo hello from the fixture");
        assert_eq!(c.cwd, root);
        let c = file.configurations[1].command(root, None).unwrap();
        assert_eq!(c.cwd, root.join("sub"));
        assert_eq!(c.env, vec![("A".to_string(), "1".to_string())]);
        let c = file.configurations[2].command(root, None).unwrap();
        assert_eq!(
            (c.program.as_str(), c.display.as_str()),
            ("cargo", "cargo run")
        );
        let gradle = RunConfig {
            kind: "gradle".into(),
            name: "build".into(),
            ..file.configurations[2].clone()
        };
        assert!(gradle.command(root, None).unwrap_err().contains("gradle"));
    }

    #[test]
    fn environment_file_then_environment() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".env"), "# comment\nA=file\nB=\"two\"\n").unwrap();
        let mut config = RunConfig {
            name: "x".into(),
            kind: "command".into(),
            program: Some("p".into()),
            args: Vec::new(),
            cargo_args: Vec::new(),
            working_dir: None,
            environment: BTreeMap::new(),
            environment_file: None,
            depends_on: Vec::new(),
            allow_parallel_run: false,
            rest: Default::default(),
            line: None,
        };
        let c = config
            .command(dir.path(), Some(&dir.path().join(".env")))
            .unwrap();
        assert_eq!(
            c.env,
            vec![("A".into(), "file".into()), ("B".into(), "two".into())]
        );
        config.environment.insert("A".into(), "row".into());
        let c = config
            .command(dir.path(), Some(&dir.path().join(".env")))
            .unwrap();
        assert_eq!(
            c.env,
            vec![("B".into(), "two".into()), ("A".into(), "row".into())]
        );
    }

    #[test]
    fn without_rewrites_the_file() {
        let file = RunFile::parse(FIXTURE).unwrap();
        let text = file.without(1).to_text();
        let again = RunFile::parse(&text).unwrap();
        let names: Vec<_> = again
            .configurations
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(names, vec!["echo hello", "run fixture"]);
        assert!(
            text.contains(
                "\"cargoArgs\": [
        \"run\"
      ]"
            ),
            "{text}"
        );
        assert!(!text.contains("allowParallelRun"));
    }

    #[test]
    fn programs_resolve_for_the_debugger() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tool"), "").unwrap();
        let cwd = Path::new("/w");
        assert_eq!(
            resolve_program("/bin/x", cwd, None),
            PathBuf::from("/bin/x")
        );
        assert_eq!(
            resolve_program("target/debug/app", cwd, None),
            PathBuf::from("/w/target/debug/app")
        );
        let path = std::env::join_paths([dir.path()]).unwrap();
        assert_eq!(
            resolve_program("tool", cwd, Some(&path)),
            dir.path().join("tool")
        );
        assert_eq!(
            resolve_program("nope", cwd, Some(&path)),
            PathBuf::from("nope")
        );
    }

    #[test]
    fn shell_commands_and_the_template() {
        let c = Command::shell("echo hi", Path::new("/w"));
        assert_eq!(c.display, "echo hi");
        assert_eq!(c.args.len(), 2);
        assert!(RunFile::parse(TEMPLATE).unwrap().configurations.is_empty());
    }
}
