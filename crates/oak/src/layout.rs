//! The panel layout a workspace was left with: which panels are open and how
//! wide, kept per workspace root in `~/.config/oak/layout.json` (Fleet keeps
//! the same in its local storage).

use serde_json::{json, Map, Value};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelLayout {
    pub open: bool,
    pub size: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub left: PanelLayout,
    pub bottom: PanelLayout,
    pub right: PanelLayout,
}

impl Layout {
    /// A fresh Fleet window: the Files island open at 360, the others closed.
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
        }
    }

    fn file() -> Option<std::path::PathBuf> {
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
        Layout {
            left: panel("left", fresh.left),
            bottom: panel("bottom", fresh.bottom),
            right: panel("right", fresh.right),
        }
    }

    pub fn save(&self, root: &Path) {
        let Some(path) = Self::file() else { return };
        let mut all = Self::read_all();
        let panel = |p: PanelLayout| json!({"open": p.open, "size": p.size});
        all.insert(
            root.display().to_string(),
            json!({"left": panel(self.left), "bottom": panel(self.bottom), "right": panel(self.right)}),
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
}
