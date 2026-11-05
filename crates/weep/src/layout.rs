//! The state a worksace was left with: which panels are open and how wide,
//! the recent files (what Fleet keeps as `RecentNavigationEntity` rows) and
//! whether the Goto preview is hidden (`GotoPanelViewState.isPreviewHidden`),
//! kept per workspace root in `~/.config/weep/layout.json` (Fleet keeps the
//! same in its local storage). The roots in the file, by when they were last
//! opened, are the recent workspaces.
//!
//! The workspace's breakpoints (M8) live in the same entry under
//! `breakpoints`, written on their own ([`Layout::save_breakpoints`]) and
//! kept when the rest of the entry is saved; so do its exception
//! breakpoints under `exceptionBreakpoints` ([`Layout::save_exceptions`]).

use crate::breakpoints::ExceptionBreakpoint;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelLayout {
    pub open: bool,
    pub size: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub left: PanelLayout,
    pub bottom: PanelLayout,
    pub right: PanelLayout,
    /// Most recent first.
    pub recent: Vec<PathBuf>,
    pub goto_preview_hidden: bool,
    /// The run configurations started, by name, most recent first (the Run
    /// & Debug popup's Recent section, M6).
    pub recent_runs: Vec<String>,
    /// Smart Mode (M7): on unless the lightning button turned it off for
    /// this workspace (Fleet keeps the choice per workspace too).
    pub smart_mode: bool,
}

/// A breakpoint as `layout.json` keeps it (M8): the file relative to the
/// workspace root, the 0-based line, and what the user set on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedBreakpoint {
    pub path: PathBuf,
    pub line: usize,
    pub enabled: bool,
    pub condition: Option<String>,
    pub log_message: Option<String>,
    /// DAP's `hitCondition`, kept as `hitCondition`.
    pub hit_condition: Option<String>,
}

/// The entry's `breakpoints` array.
fn breakpoints_to_json(list: &[SavedBreakpoint]) -> Value {
    Value::Array(
        list.iter()
            .map(|b| {
                let mut o = Map::new();
                // Forward slashes, so a workspace on a shared disk reads the
                // same on every platform.
                let path = b
                    .path
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                o.insert("path".into(), Value::String(path));
                o.insert("line".into(), json!(b.line));
                o.insert("enabled".into(), Value::Bool(b.enabled));
                if let Some(c) = &b.condition {
                    o.insert("condition".into(), Value::String(c.clone()));
                }
                if let Some(l) = &b.log_message {
                    o.insert("logMessage".into(), Value::String(l.clone()));
                }
                if let Some(h) = &b.hit_condition {
                    o.insert("hitCondition".into(), Value::String(h.clone()));
                }
                Value::Object(o)
            })
            .collect(),
    )
}

fn breakpoints_from_json(value: Option<&Value>) -> Vec<SavedBreakpoint> {
    let Some(items) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_string);
    items
        .iter()
        .filter_map(|v| {
            Some(SavedBreakpoint {
                path: PathBuf::from(v.get("path")?.as_str()?),
                line: v.get("line")?.as_u64()? as usize,
                enabled: v.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                condition: text(v, "condition"),
                log_message: text(v, "logMessage"),
                hit_condition: text(v, "hitCondition"),
            })
        })
        .collect()
}

/// The entry's `exceptionBreakpoints` array: `filter`, `label`, `enabled`.
fn exceptions_to_json(list: &[ExceptionBreakpoint]) -> Value {
    Value::Array(
        list.iter()
            .map(|e| json!({"filter": e.filter, "label": e.label, "enabled": e.enabled}))
            .collect(),
    )
}

fn exceptions_from_json(value: Option<&Value>) -> Vec<ExceptionBreakpoint> {
    let Some(items) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|v| {
            let filter = v.get("filter")?.as_str()?.to_string();
            Some(ExceptionBreakpoint {
                label: v
                    .get("label")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| filter.clone()),
                enabled: v.get("enabled").and_then(Value::as_bool).unwrap_or(false),
                filter,
            })
        })
        .collect()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Layout {
    /// A fresh Fleet window: the Files island open at 360, the others closed,
    /// the Goto preview shown (Fleet's stored default).
    pub fn fresh() -> Layout {
        use crate::theme::metrics::*;
        Layout {
            left: PanelLayout {
                open: true,
                size: LEFT_PANEL_WIDTH,
            },
            bottom: PanelLayout {
                open: false,
                size: BOTTOM_PANEL_HEIGHT,
            },
            right: PanelLayout {
                open: false,
                size: RIGHT_PANEL_WIDTH,
            },
            recent: Vec::new(),
            goto_preview_hidden: false,
            recent_runs: Vec::new(),
            smart_mode: true,
        }
    }

    fn file() -> Option<PathBuf> {
        weep_settings::weep_config_dir().map(|d| d.join("layout.json"))
    }

    fn read_all() -> Map<String, Value> {
        Self::file()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| match v {
                Value::Object(m) => Some(m),
                _ => None,
            })
            .unwrap_or_default()
    }

    pub fn load(root: &Path) -> Layout {
        let all = Self::read_all();
        let Some(entry) = all.get(&root.display().to_string()) else {
            return Layout::fresh();
        };
        let fresh = Layout::fresh();
        let panel = |name: &str, default: PanelLayout| -> PanelLayout {
            let Some(v) = entry.get(name) else {
                return default;
            };
            PanelLayout {
                open: v
                    .get("open")
                    .and_then(Value::as_bool)
                    .unwrap_or(default.open),
                size: v
                    .get("size")
                    .and_then(Value::as_f64)
                    .map(|s| s as f32)
                    .unwrap_or(default.size),
            }
        };
        let recent = entry
            .get("recent")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str().map(PathBuf::from))
                    .collect()
            })
            .unwrap_or_default();
        Layout {
            left: panel("left", fresh.left),
            bottom: panel("bottom", fresh.bottom),
            right: panel("right", fresh.right),
            recent,
            goto_preview_hidden: entry
                .get("gotoPreviewHidden")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            recent_runs: entry
                .get("recentRuns")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            smart_mode: entry
                .get("smartMode")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        }
    }

    pub fn save(&self, root: &Path) {
        let Some(path) = Self::file() else { return };
        let mut all = Self::read_all();
        let entry = self.entry_json(all.get(&root.display().to_string()));
        all.insert(root.display().to_string(), entry);
        Self::write_all(&path, all);
    }

    /// The workspace's entry, keeping what `previous` holds that this
    /// struct does not (the breakpoints and exception breakpoints, M8).
    fn entry_json(&self, previous: Option<&Value>) -> Value {
        let panel = |p: PanelLayout| json!({"open": p.open, "size": p.size});
        let recent: Vec<Value> = self
            .recent
            .iter()
            .map(|p| Value::String(p.display().to_string()))
            .collect();
        let mut entry = json!({
            "left": panel(self.left),
            "bottom": panel(self.bottom),
            "right": panel(self.right),
            "recent": recent,
            "gotoPreviewHidden": self.goto_preview_hidden,
            "recentRuns": self.recent_runs,
            "smartMode": self.smart_mode,
            "opened": now_secs(),
        });
        for key in ["breakpoints", "exceptionBreakpoints"] {
            if let Some(b) = previous.and_then(|p| p.get(key)) {
                entry[key] = b.clone();
            }
        }
        entry
    }

    /// The breakpoints `layout.json` holds for `root` (M8).
    pub fn load_breakpoints(root: &Path) -> Vec<SavedBreakpoint> {
        let all = Self::read_all();
        breakpoints_from_json(
            all.get(&root.display().to_string())
                .and_then(|e| e.get("breakpoints")),
        )
    }

    /// Writes `root`'s breakpoints, leaving the rest of its entry (a root
    /// the file did not know gets a fresh layout beside them).
    pub fn save_breakpoints(root: &Path, list: &[SavedBreakpoint]) {
        Self::save_key(root, "breakpoints", breakpoints_to_json(list));
    }

    /// The exception breakpoints `layout.json` holds for `root` (M8).
    pub fn load_exceptions(root: &Path) -> Vec<ExceptionBreakpoint> {
        let all = Self::read_all();
        exceptions_from_json(
            all.get(&root.display().to_string())
                .and_then(|e| e.get("exceptionBreakpoints")),
        )
    }

    /// Writes `root`'s exception breakpoints, as [`Self::save_breakpoints`].
    pub fn save_exceptions(root: &Path, list: &[ExceptionBreakpoint]) {
        Self::save_key(root, "exceptionBreakpoints", exceptions_to_json(list));
    }

    fn save_key(root: &Path, name: &str, value: Value) {
        let Some(path) = Self::file() else { return };
        let mut all = Self::read_all();
        let key = root.display().to_string();
        let mut entry = match all.remove(&key) {
            Some(Value::Object(m)) => Value::Object(m),
            _ => Layout::fresh().entry_json(None),
        };
        entry[name] = value;
        all.insert(key, entry);
        Self::write_all(&path, all);
    }

    fn write_all(path: &Path, all: Map<String, Value>) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(&Value::Object(all)) {
            if let Err(e) = std::fs::write(path, text + "\n") {
                eprintln!("weep: cannot save {}: {e}", path.display());
            }
        }
    }

    /// Every workspace root the file knows, most recently opened first,
    /// those still on disk.
    pub fn recent_workspaces() -> Vec<PathBuf> {
        let all = Self::read_all();
        let mut roots: Vec<(u64, PathBuf)> = all
            .iter()
            .map(|(root, v)| {
                let opened = v.get("opened").and_then(Value::as_u64).unwrap_or(0);
                (opened, PathBuf::from(root))
            })
            .filter(|(_, p)| p.is_dir())
            .collect();
        roots.sort_by_key(|a| std::cmp::Reverse(a.0));
        roots.into_iter().map(|(_, p)| p).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved() -> Vec<SavedBreakpoint> {
        vec![
            SavedBreakpoint {
                path: PathBuf::from("src").join("main.rs"),
                line: 0,
                enabled: true,
                condition: None,
                log_message: None,
                hit_condition: None,
            },
            SavedBreakpoint {
                path: PathBuf::from("Cargo.toml"),
                line: 41,
                enabled: false,
                condition: Some("i > 3".into()),
                log_message: Some("i = {i}".into()),
                hit_condition: Some("3".into()),
            },
        ]
    }

    #[test]
    fn breakpoints_round_trip() {
        let json = breakpoints_to_json(&saved());
        assert_eq!(json[0]["path"], "src/main.rs");
        assert_eq!(json[1]["logMessage"], "i = {i}");
        assert_eq!(json[1]["hitCondition"], "3");
        assert!(json[0].get("condition").is_none());
        assert!(json[0].get("hitCondition").is_none());
        let text = serde_json::to_string(&json).unwrap();
        let back: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(breakpoints_from_json(Some(&back)), saved());
    }

    #[test]
    fn unreadable_breakpoints_are_skipped() {
        let json = json!([{"path": "a.rs"}, {"line": 2}, {"path": "b.rs", "line": 2}]);
        let back = breakpoints_from_json(Some(&json));
        assert_eq!(back.len(), 1);
        assert!(back[0].enabled);
        assert!(breakpoints_from_json(None).is_empty());
    }

    fn exceptions() -> Vec<ExceptionBreakpoint> {
        vec![
            ExceptionBreakpoint {
                filter: "rust_panic".into(),
                label: "Rust: on panic".into(),
                enabled: true,
            },
            ExceptionBreakpoint {
                filter: "cpp_throw".into(),
                label: "C++: on throw".into(),
                enabled: false,
            },
        ]
    }

    #[test]
    fn exception_breakpoints_round_trip() {
        let json = exceptions_to_json(&exceptions());
        assert_eq!(json[0]["filter"], "rust_panic");
        let text = serde_json::to_string(&json).unwrap();
        let back: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(exceptions_from_json(Some(&back)), exceptions());
        // A row without a label shows its filter; one without a filter is skipped.
        let back = exceptions_from_json(Some(&json!([{"filter": "x"}, {"label": "y"}])));
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].label, "x");
        assert!(!back[0].enabled);
    }

    #[test]
    fn saving_the_layout_keeps_the_breakpoints() {
        let previous = json!({
            "left": {},
            "breakpoints": breakpoints_to_json(&saved()),
            "exceptionBreakpoints": exceptions_to_json(&exceptions()),
        });
        let entry = Layout::fresh().entry_json(Some(&previous));
        assert_eq!(breakpoints_from_json(entry.get("breakpoints")), saved());
        assert_eq!(
            exceptions_from_json(entry.get("exceptionBreakpoints")),
            exceptions()
        );
        assert!(Layout::fresh()
            .entry_json(None)
            .get("breakpoints")
            .is_none());
    }
}
