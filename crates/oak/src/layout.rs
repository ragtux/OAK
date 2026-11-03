//! The state a worksace was left with: which panels are open and how wide,
//! the recent files (what Fleet keeps as `RecentNavigationEntity` rows) and
//! whether the Goto preview is hidden (`GotoPanelViewState.isPreviewHidden`),
//! kept per workspace root in `~/.config/oak/layout.json` (Fleet keeps the
//! same in its local storage). The roots in the file, by when they were last
//! opened, are the recent workspaces.

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
        oak_settings::oak_config_dir().map(|d| d.join("layout.json"))
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
        let panel = |p: PanelLayout| json!({"open": p.open, "size": p.size});
        let recent: Vec<Value> = self
            .recent
            .iter()
            .map(|p| Value::String(p.display().to_string()))
            .collect();
        all.insert(
            root.display().to_string(),
            json!({
                "left": panel(self.left),
                "bottom": panel(self.bottom),
                "right": panel(self.right),
                "recent": recent,
                "gotoPreviewHidden": self.goto_preview_hidden,
                "recentRuns": self.recent_runs,
                "smartMode": self.smart_mode,
                "opened": now_secs(),
            }),
        );
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(&Value::Object(all)) {
            if let Err(e) = std::fs::write(&path, text + "\n") {
                eprintln!("oak: cannot save {}: {e}", path.display());
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
