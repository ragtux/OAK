//! Debugging a `cargo` row (M8): the row's `cargoArgs` become a build that
//! prints cargo's JSON messages (`--message-format=json-render-diagnostics`:
//! the compiler's diagnostics still rendered as text on stderr, the
//! artifacts as JSON on stdout), and the build's artifacts name the
//! executable the debugger launches: a `cargo run` row's binary, a
//! `cargo test` row's test executable (`cargo test --no-run`). The program's
//! arguments are what the row passes after `--` (and a test row's name
//! filters). Pure: the app runs the build and feeds the lines in.

use crate::config::Command;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// What the row's subcommand runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CargoTarget {
    /// `cargo run` (and `cargo build`): a binary or an example.
    Run,
    /// `cargo test`: a test executable.
    Test,
}

/// The target selectors in the row's arguments, which narrow the artifacts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selector {
    pub package: Option<String>,
    pub bin: Option<String>,
    pub example: Option<String>,
    pub test: Option<String>,
    pub bench: Option<String>,
    pub lib: bool,
}

/// The build that precedes a debug session of a `cargo` row.
#[derive(Clone, Debug, PartialEq)]
pub struct DebugBuild {
    /// `cargo build ...` or `cargo test --no-run ...`, in the row's directory
    /// and environment.
    pub command: Command,
    pub target: CargoTarget,
    pub selector: Selector,
    /// The debuggee's arguments.
    pub program_args: Vec<String>,
}

/// The cargo flags that take a value (`--bin x`; `--bin=x` is one word).
const VALUE_FLAGS: &[&str] = &[
    "-p",
    "--package",
    "--bin",
    "--example",
    "--test",
    "--bench",
    "-F",
    "--features",
    "--target",
    "--target-dir",
    "--profile",
    "-j",
    "--jobs",
    "--manifest-path",
    "--color",
    "--config",
    "-Z",
    "-C",
    "--exclude",
    "--lockfile-path",
    "--message-format",
];

fn takes_value(flag: &str) -> bool {
    VALUE_FLAGS.contains(&flag)
}

impl DebugBuild {
    /// The build for a row whose resolved command is `cargo <cargo_args>`
    /// (`command` carries the row's directory and environment).
    pub fn from_cargo(command: &Command) -> Result<DebugBuild, String> {
        let args = &command.args;
        let mut pre = Vec::new();
        let mut i = 0;
        // A toolchain (`+nightly`) and global flags before the subcommand.
        while i < args.len() && (args[i].starts_with('+') || args[i].starts_with('-')) {
            pre.push(args[i].clone());
            if takes_value(&args[i]) && i + 1 < args.len() {
                pre.push(args[i + 1].clone());
                i += 1;
            }
            i += 1;
        }
        let target = match args.get(i).map(String::as_str) {
            Some("run" | "r" | "build" | "b") => CargoTarget::Run,
            Some("test" | "t") => CargoTarget::Test,
            Some(other) => return Err(format!("'cargo {other}' cannot be debugged")),
            None => return Err("the row has no cargo subcommand".into()),
        };
        i += 1;
        let mut flags = Vec::new();
        let mut filters = Vec::new();
        let mut program_args = Vec::new();
        let mut selector = Selector::default();
        while i < args.len() {
            let arg = &args[i];
            if arg == "--" {
                program_args.extend(args[i + 1..].iter().cloned());
                break;
            }
            if arg.starts_with('-') {
                let (flag, inline) = match arg.split_once('=') {
                    Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
                    _ => (arg.clone(), None),
                };
                let value = match inline {
                    Some(v) => Some(v),
                    None if takes_value(&flag) && i + 1 < args.len() => {
                        i += 1;
                        Some(args[i].clone())
                    }
                    None => None,
                };
                match (flag.as_str(), &value) {
                    // The build prints its own format.
                    ("--message-format", _) => {
                        i += 1;
                        continue;
                    }
                    ("-p" | "--package", Some(v)) => selector.package = Some(v.clone()),
                    ("--bin", Some(v)) => selector.bin = Some(v.clone()),
                    ("--example", Some(v)) => selector.example = Some(v.clone()),
                    ("--test", Some(v)) => selector.test = Some(v.clone()),
                    ("--bench", Some(v)) => selector.bench = Some(v.clone()),
                    ("--lib", None) => selector.lib = true,
                    _ => {}
                }
                flags.push(arg.clone());
                if value.is_some() && !arg.contains('=') {
                    flags.push(args[i].clone());
                }
            } else if target == CargoTarget::Test {
                // `cargo test name`: a filter the test executable gets.
                filters.push(arg.clone());
            } else {
                flags.push(arg.clone());
            }
            i += 1;
        }
        filters.extend(program_args);
        let program_args = filters;
        let mut build_args = pre;
        match target {
            CargoTarget::Run => build_args.push("build".into()),
            CargoTarget::Test => {
                build_args.push("test".into());
                build_args.push("--no-run".into());
            }
        }
        build_args.extend(flags);
        build_args.push("--message-format=json-render-diagnostics".into());
        let display = std::iter::once(command.program.as_str())
            .chain(build_args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        Ok(DebugBuild {
            command: Command {
                program: command.program.clone(),
                args: build_args,
                cwd: command.cwd.clone(),
                env: command.env.clone(),
                display,
            },
            target,
            selector,
            program_args,
        })
    }
}

/// One `compiler-artifact` message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artifact {
    pub package_id: String,
    pub manifest_path: PathBuf,
    pub target_name: String,
    /// The target's kinds: `bin`, `lib`, `test`, `example`, ...
    pub kinds: Vec<String>,
    /// Built with the test harness (`profile.test`).
    pub test: bool,
    pub executable: Option<PathBuf>,
}

impl Artifact {
    /// The package's name, from its id (`path+file:///w/a#name@0.1.0`,
    /// `path+file:///w/name#0.1.0`, or the old `name 0.1.0 (path+file://...)`).
    pub fn package_name(&self) -> String {
        package_name(&self.package_id)
    }

    fn is_kind(&self, kind: &str) -> bool {
        self.kinds.iter().any(|k| k == kind)
    }

    fn is_lib(&self) -> bool {
        self.kinds.iter().any(|k| {
            matches!(
                k.as_str(),
                "lib" | "rlib" | "dylib" | "cdylib" | "staticlib" | "proc-macro"
            )
        })
    }

    /// How the debug console names it: `name (kind)`.
    pub fn label(&self) -> String {
        format!(
            "{} ({})",
            self.target_name,
            self.kinds.first().map(String::as_str).unwrap_or("?")
        )
    }
}

fn package_name(id: &str) -> String {
    if let Some((_, frag)) = id.rsplit_once('#') {
        // `#name@version`, or `#version` with the name the path's last part.
        if let Some((name, _)) = frag.split_once('@') {
            return name.to_string();
        }
        let path = id[..id.len() - frag.len() - 1].trim_end_matches('/');
        return path.rsplit('/').next().unwrap_or(path).to_string();
    }
    id.split_whitespace().next().unwrap_or(id).to_string()
}

/// A line of the build's output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    Artifact(Artifact),
    Finished {
        success: bool,
    },
    /// Another JSON message (`build-script-executed`, ...), shown nowhere.
    Other,
}

/// The line as one of cargo's JSON messages; `None` for a line of text (a
/// rendered diagnostic, "Compiling ..."), which the console shows.
pub fn parse_message(line: &str) -> Option<Message> {
    let line = line.trim();
    if !line.starts_with('{') {
        return None;
    }
    let value: Value = serde_json::from_str(line).ok()?;
    let reason = value.get("reason")?.as_str()?;
    Some(match reason {
        "compiler-artifact" => {
            let target = value.get("target");
            let strings = |v: Option<&Value>| -> Vec<String> {
                v.and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|k| k.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            Message::Artifact(Artifact {
                package_id: value
                    .get("package_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                manifest_path: value
                    .get("manifest_path")
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
                    .unwrap_or_default(),
                target_name: target
                    .and_then(|t| t.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                kinds: strings(target.and_then(|t| t.get("kind"))),
                test: value
                    .get("profile")
                    .and_then(|p| p.get("test"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                executable: value
                    .get("executable")
                    .and_then(Value::as_str)
                    .map(PathBuf::from),
            })
        }
        "build-finished" => Message::Finished {
            success: value
                .get("success")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        },
        _ => Message::Other,
    })
}

/// The executable a debug session launches, and the other candidates it
/// passed over (a `cargo test` row builds one per target; the debugger
/// runs one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picked {
    pub artifact: Artifact,
    pub executable: PathBuf,
    pub others: Vec<String>,
}

/// Picks the debuggee among the build's artifacts, as `cargo run` and
/// `cargo test` would pick what they run.
pub fn pick(
    artifacts: &[Artifact],
    target: CargoTarget,
    selector: &Selector,
) -> Result<Picked, String> {
    let named =
        |a: &Artifact, name: &Option<String>| name.as_ref().is_none_or(|n| &a.target_name == n);
    let in_package = |a: &Artifact| {
        selector
            .package
            .as_ref()
            .is_none_or(|p| &a.package_name() == p)
    };
    let mut candidates: Vec<&Artifact> = artifacts
        .iter()
        .filter(|a| a.executable.is_some() && in_package(a))
        .filter(|a| match target {
            CargoTarget::Run => {
                !a.test
                    && if selector.example.is_some() {
                        a.is_kind("example") && named(a, &selector.example)
                    } else {
                        a.is_kind("bin") && named(a, &selector.bin)
                    }
            }
            CargoTarget::Test => {
                a.test
                    && if selector.test.is_some() {
                        a.is_kind("test") && named(a, &selector.test)
                    } else if selector.bin.is_some() {
                        a.is_kind("bin") && named(a, &selector.bin)
                    } else if selector.example.is_some() {
                        a.is_kind("example") && named(a, &selector.example)
                    } else if selector.bench.is_some() {
                        a.is_kind("bench") && named(a, &selector.bench)
                    } else if selector.lib {
                        a.is_lib()
                    } else {
                        true
                    }
            }
        })
        .collect();
    // The same unit may be reported twice (a rebuild); keep the last.
    let mut seen = Vec::new();
    candidates.reverse();
    candidates.retain(|a| {
        let key = (a.executable.clone(), a.target_name.clone());
        if seen.contains(&key) {
            false
        } else {
            seen.push(key);
            true
        }
    });
    candidates.reverse();
    if candidates.is_empty() {
        return Err(match target {
            CargoTarget::Run => "the build made no binary to debug".into(),
            CargoTarget::Test => "the build made no test executable to debug".into(),
        });
    }
    let chosen = match target {
        CargoTarget::Run if candidates.len() > 1 => {
            // `cargo run` asks which one unless the package's own name is a bin.
            let by_name: Vec<&&Artifact> = candidates
                .iter()
                .filter(|a| a.target_name == a.package_name())
                .collect();
            if by_name.len() == 1 {
                (*by_name[0]).clone()
            } else {
                let names: Vec<String> = candidates.iter().map(|a| a.target_name.clone()).collect();
                return Err(format!(
                    "the row builds several binaries ({}): add --bin to its cargoArgs",
                    names.join(", ")
                ));
            }
        }
        CargoTarget::Run => candidates[0].clone(),
        CargoTarget::Test => {
            // The unit tests first (lib, then bin), then the integration tests.
            let rank = |a: &Artifact| {
                if a.is_lib() {
                    0
                } else if a.is_kind("bin") {
                    1
                } else {
                    2
                }
            };
            let best = candidates
                .iter()
                .min_by_key(|a| rank(a))
                .map(|a| (*a).clone())
                .unwrap_or_else(|| candidates[0].clone());
            best
        }
    };
    let others = candidates
        .iter()
        .filter(|a| a.executable != chosen.executable)
        .map(|a| a.label())
        .collect();
    Ok(Picked {
        executable: chosen.executable.clone().unwrap_or_default(),
        artifact: chosen,
        others,
    })
}

/// Where and with what the debuggee runs, as cargo would run it: a test
/// executable in its package's directory (`cargo test` does so), a binary in
/// the row's; both with cargo's `CARGO_MANIFEST_DIR` and `CARGO_PKG_NAME`
/// under the row's environment.
pub fn launch_place(build: &DebugBuild, picked: &Picked) -> (PathBuf, Vec<(String, String)>) {
    let manifest_dir = picked
        .artifact
        .manifest_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| build.command.cwd.clone());
    let cwd = match build.target {
        CargoTarget::Test => manifest_dir.clone(),
        CargoTarget::Run => build.command.cwd.clone(),
    };
    let mut env = vec![
        (
            "CARGO_MANIFEST_DIR".to_string(),
            manifest_dir.display().to_string(),
        ),
        ("CARGO_PKG_NAME".to_string(), picked.artifact.package_name()),
    ];
    for (k, v) in &build.command.env {
        env.retain(|(ek, _)| ek != k);
        env.push((k.clone(), v.clone()));
    }
    (cwd, env)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cargo(args: &[&str]) -> Command {
        Command {
            program: "cargo".into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            cwd: PathBuf::from("/w"),
            env: vec![("A".into(), "1".into())],
            display: String::new(),
        }
    }

    #[test]
    fn a_run_row_builds_with_json_messages() {
        let b = DebugBuild::from_cargo(&cargo(&[
            "run",
            "--bin",
            "app",
            "--release",
            "--",
            "x",
            "--y",
        ]))
        .unwrap();
        assert_eq!(b.target, CargoTarget::Run);
        assert_eq!(
            b.command.args,
            vec![
                "build",
                "--bin",
                "app",
                "--release",
                "--message-format=json-render-diagnostics"
            ]
        );
        assert_eq!(b.program_args, vec!["x", "--y"]);
        assert_eq!(b.selector.bin.as_deref(), Some("app"));
        assert_eq!(b.command.cwd, PathBuf::from("/w"));
        assert_eq!(b.command.env, vec![("A".to_string(), "1".to_string())]);
    }

    #[test]
    fn a_test_row_builds_without_running_and_keeps_its_filters() {
        let b = DebugBuild::from_cargo(&cargo(&[
            "+nightly",
            "test",
            "--package=core",
            "--test",
            "it",
            "parses",
            "--message-format",
            "short",
            "--",
            "--exact",
        ]))
        .unwrap();
        assert_eq!(b.target, CargoTarget::Test);
        assert_eq!(
            b.command.args,
            vec![
                "+nightly",
                "test",
                "--no-run",
                "--package=core",
                "--test",
                "it",
                "--message-format=json-render-diagnostics"
            ]
        );
        assert_eq!(b.program_args, vec!["parses", "--exact"]);
        assert_eq!(b.selector.test.as_deref(), Some("it"));
        assert_eq!(b.selector.package.as_deref(), Some("core"));
        let b = DebugBuild::from_cargo(&cargo(&["t", "-p", "core", "--lib"])).unwrap();
        assert_eq!(b.selector.package.as_deref(), Some("core"));
        assert!(b.selector.lib);
        assert!(DebugBuild::from_cargo(&cargo(&["clippy"]))
            .unwrap_err()
            .contains("clippy"));
        assert!(DebugBuild::from_cargo(&cargo(&[])).is_err());
    }

    // Captured from `cargo build --message-format=json-render-diagnostics`
    // and `cargo test --no-run ...` (cargo 1.8x), trimmed to the fields read.
    const LINES: &[&str] = &[
        r#"{"reason":"compiler-artifact","package_id":"registry+https://github.com/rust-lang/crates.io-index#serde@1.0.210","manifest_path":"/home/u/.cargo/registry/src/serde-1.0.210/Cargo.toml","target":{"kind":["lib"],"crate_types":["lib"],"name":"serde","src_path":"/x/lib.rs","edition":"2018","doc":true,"doctest":true,"test":true},"profile":{"opt_level":"0","debuginfo":2,"debug_assertions":true,"overflow_checks":true,"test":false},"features":[],"filenames":["/w/target/debug/deps/libserde-1.rlib"],"executable":null,"fresh":true}"#,
        r#"{"reason":"build-script-executed","package_id":"path+file:///w#app@0.1.0","linked_libs":[],"linked_paths":[],"cfgs":[],"env":[],"out_dir":"/w/target/debug/build/app-1/out"}"#,
        r#"{"reason":"compiler-artifact","package_id":"path+file:///w#app@0.1.0","manifest_path":"/w/Cargo.toml","target":{"kind":["lib"],"crate_types":["lib"],"name":"app","src_path":"/w/src/lib.rs","edition":"2021","doc":true,"doctest":true,"test":true},"profile":{"opt_level":"0","debuginfo":2,"debug_assertions":true,"overflow_checks":true,"test":true},"features":[],"filenames":["/w/target/debug/deps/app-0f3c"],"executable":"/w/target/debug/deps/app-0f3c","fresh":false}"#,
        r#"{"reason":"compiler-artifact","package_id":"path+file:///w#app@0.1.0","manifest_path":"/w/Cargo.toml","target":{"kind":["bin"],"crate_types":["bin"],"name":"app","src_path":"/w/src/main.rs","edition":"2021","doc":true,"doctest":false,"test":true},"profile":{"opt_level":"0","debuginfo":2,"debug_assertions":true,"overflow_checks":true,"test":false},"features":[],"filenames":["/w/target/debug/app"],"executable":"/w/target/debug/app","fresh":false}"#,
        r#"{"reason":"compiler-artifact","package_id":"path+file:///w#app@0.1.0","manifest_path":"/w/Cargo.toml","target":{"kind":["bin"],"crate_types":["bin"],"name":"tool","src_path":"/w/src/bin/tool.rs","edition":"2021","doc":true,"doctest":false,"test":true},"profile":{"opt_level":"0","debuginfo":2,"debug_assertions":true,"overflow_checks":true,"test":false},"features":[],"filenames":["/w/target/debug/tool"],"executable":"/w/target/debug/tool","fresh":false}"#,
        r#"{"reason":"compiler-artifact","package_id":"path+file:///w#app@0.1.0","manifest_path":"/w/Cargo.toml","target":{"kind":["test"],"crate_types":["bin"],"name":"it","src_path":"/w/tests/it.rs","edition":"2021","doc":false,"doctest":false,"test":true},"profile":{"opt_level":"0","debuginfo":2,"debug_assertions":true,"overflow_checks":true,"test":true},"features":[],"filenames":["/w/target/debug/deps/it-9a1b"],"executable":"/w/target/debug/deps/it-9a1b","fresh":false}"#,
        r#"{"reason":"build-finished","success":true}"#,
    ];

    fn artifacts() -> Vec<Artifact> {
        LINES
            .iter()
            .filter_map(|l| match parse_message(l) {
                Some(Message::Artifact(a)) => Some(a),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn messages_parse_and_text_lines_do_not() {
        assert_eq!(parse_message("   Compiling app v0.1.0 (/w)"), None);
        assert_eq!(parse_message("{not json"), None);
        assert_eq!(parse_message(LINES[1]), Some(Message::Other));
        assert_eq!(
            parse_message(LINES[6]),
            Some(Message::Finished { success: true })
        );
        let a = artifacts();
        assert_eq!(a.len(), 5);
        assert_eq!(a[0].executable, None);
        assert_eq!(a[1].kinds, vec!["lib"]);
        assert!(a[1].test);
        assert_eq!(
            a[2].executable.as_deref(),
            Some(Path::new("/w/target/debug/app"))
        );
        assert_eq!(a[2].package_name(), "app");
        assert_eq!(a[0].package_name(), "serde");
    }

    #[test]
    fn package_names_from_every_id_spelling() {
        assert_eq!(package_name("path+file:///w/app#0.1.0"), "app");
        assert_eq!(package_name("path+file:///w#app@0.1.0"), "app");
        assert_eq!(package_name("app 0.1.0 (path+file:///w)"), "app");
    }

    #[test]
    fn a_run_row_picks_its_binary() {
        let a = artifacts();
        let run = CargoTarget::Run;
        // Two bins: the package's own name wins, as cargo's default would.
        let p = pick(&a, run, &Selector::default()).unwrap();
        assert_eq!(p.executable, PathBuf::from("/w/target/debug/app"));
        assert_eq!(p.others, vec!["tool (bin)"]);
        let sel = Selector {
            bin: Some("tool".into()),
            ..Selector::default()
        };
        assert_eq!(
            pick(&a, run, &sel).unwrap().executable,
            PathBuf::from("/w/target/debug/tool")
        );
        let none = Selector {
            bin: Some("nope".into()),
            ..Selector::default()
        };
        assert!(pick(&a, run, &none).is_err());
        let two: Vec<Artifact> = a
            .iter()
            .filter(|x| x.target_name == "tool")
            .cloned()
            .chain(
                a.iter()
                    .filter(|x| x.target_name == "tool")
                    .map(|x| Artifact {
                        target_name: "other".into(),
                        executable: Some("/w/target/debug/other".into()),
                        ..x.clone()
                    }),
            )
            .collect();
        assert!(pick(&two, run, &Selector::default())
            .unwrap_err()
            .contains("--bin"));
    }

    #[test]
    fn a_test_row_picks_a_test_executable() {
        let a = artifacts();
        let test = CargoTarget::Test;
        let p = pick(&a, test, &Selector::default()).unwrap();
        assert_eq!(p.executable, PathBuf::from("/w/target/debug/deps/app-0f3c"));
        assert_eq!(p.others, vec!["it (test)"]);
        let sel = Selector {
            test: Some("it".into()),
            ..Selector::default()
        };
        let p = pick(&a, test, &sel).unwrap();
        assert_eq!(p.executable, PathBuf::from("/w/target/debug/deps/it-9a1b"));
        assert!(p.others.is_empty());
        let other_pkg = Selector {
            package: Some("serde".into()),
            ..Selector::default()
        };
        assert!(pick(&a, test, &other_pkg).is_err());
    }

    #[test]
    fn tests_run_in_their_package_and_binaries_where_the_row_says() {
        let a = artifacts();
        let b = DebugBuild::from_cargo(&cargo(&["test"])).unwrap();
        let p = pick(&a, b.target, &b.selector).unwrap();
        let (cwd, env) = launch_place(&b, &p);
        assert_eq!(cwd, PathBuf::from("/w"));
        assert_eq!(env[0], ("CARGO_MANIFEST_DIR".into(), "/w".into()));
        assert_eq!(env.last(), Some(&("A".to_string(), "1".to_string())));
        let mut run = cargo(&["run"]);
        run.cwd = PathBuf::from("/w/sub");
        let b = DebugBuild::from_cargo(&run).unwrap();
        let p = pick(&a, b.target, &b.selector).unwrap();
        assert_eq!(launch_place(&b, &p).0, PathBuf::from("/w/sub"));
    }
}
