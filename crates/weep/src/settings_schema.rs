//! What Fleet's Settings tool shows, page by page, read off its pages on
//! 2026-11-02 (batch 8, `docs/fleet-probe.md` seventh pass): the sections
//! and groups of its left tree, each row's label, the `settings.json` key it
//! writes (from `documentation.yaml`), the kind of control and Fleet's
//! default as the page displayed it. Rows whose key Fleet's documentation
//! does not name (the AI rows, inlay hints, the scrollbar toggles, the shell
//! integration rows, the collaboration username) are left out rather than
//! bound to a guessed key.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// A list of (JSON value, label) pairs.
    Dropdown(&'static [(&'static str, &'static str)]),
    Text,
    Number,
    Checkbox,
    /// "Edit in settings.json": the value has no control.
    Link,
}

#[derive(Clone, Copy, Debug)]
pub struct Item {
    pub label: &'static str,
    pub key: &'static str,
    pub kind: Kind,
    /// Fleet's default, as a JSON literal.
    pub default: &'static str,
    /// The secondary line under a checkbox.
    pub description: Option<&'static str>,
    /// Drawn a step in (Fleet indents "Tooltip delay" under its checkbox).
    pub indent: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Group {
    /// None for a section with a single, untitled group (General).
    pub title: Option<&'static str>,
    pub items: &'static [Item],
}

#[derive(Clone, Copy, Debug)]
pub struct Section {
    pub title: &'static str,
    pub groups: &'static [Group],
}

const fn item(label: &'static str, key: &'static str, kind: Kind, default: &'static str) -> Item {
    Item {
        label,
        key,
        kind,
        default,
        description: None,
        indent: false,
    }
}

const fn described(
    label: &'static str,
    key: &'static str,
    default: &'static str,
    description: &'static str,
) -> Item {
    Item {
        label,
        key,
        kind: Kind::Checkbox,
        default,
        description: Some(description),
        indent: false,
    }
}

const fn check(label: &'static str, key: &'static str, default: &'static str) -> Item {
    item(label, key, Kind::Checkbox, default)
}

const fn link(label: &'static str, key: &'static str) -> Item {
    item(label, key, Kind::Link, "null")
}

const SKINS: &[(&str, &str)] = &[("\"fleet\"", "Fleet"), ("\"tui\"", "Terminal (TUI)")];
const THEMES: &[(&str, &str)] = &[
    ("\"dark\"", "Fleet Dark"),
    ("\"light\"", "Fleet Light"),
    ("\"dark_purple\"", "Fleet Dark Purple"),
];
/// Fleet's zoom steps are not documented; these are the values the probe
/// has seen it accept (estimate).
const ZOOMS: &[(&str, &str)] = &[
    ("0.7", "70%"),
    ("0.8", "80%"),
    ("0.9", "90%"),
    ("1.0", "100%"),
    ("1.1", "110%"),
    ("1.25", "125%"),
    ("1.5", "150%"),
    ("1.75", "175%"),
    ("2.0", "200%"),
];
const KEYMAPS: &[(&str, &str)] = &[
    ("\"fleet\"", "Fleet"),
    ("\"intellij\"", "IntelliJ IDEA"),
    ("\"ij-classic\"", "IntelliJ IDEA Classic"),
    ("\"visual-studio\"", "Visual Studio"),
    ("\"vscode\"", "VS Code"),
    ("\"sublimeText\"", "Sublime Text"),
    ("\"eclipse\"", "Eclipse"),
    ("\"emacs\"", "Emacs"),
];
const AUTO_SAVE: &[(&str, &str)] = &[
    ("\"Off\"", "Off"),
    ("\"OnFocus\"", "On focus change"),
    ("\"AfterDelay\"", "After delay"),
];
const FONTS: &[(&str, &str)] = &[
    ("\"JetBrains Mono\"", "JetBrains Mono"),
    ("\"Cascadia Mono\"", "Cascadia Mono"),
    ("\"Consolas\"", "Consolas"),
    ("\"Courier New\"", "Courier New"),
];
const WHITESPACE: &[(&str, &str)] = &[("false", "None"), ("true", "All")];
const LINE_NUMBERS: &[(&str, &str)] = &[
    ("\"On\"", "On"),
    ("\"Off\"", "Off"),
    ("\"Relative\"", "Relative"),
];
const INDENT: &[(&str, &str)] = &[("\"Spaces\"", "Spaces"), ("\"Tabs\"", "Tabs")];
// Fleet's values as its Settings dropdown writes them (sixteenth pass): Block,
// Outline, Underline, Vertical (the default) and VerticalThin; WEEP draws the
// three listed.
const CARETS: &[(&str, &str)] = &[
    ("\"Vertical\"", "Vertical"),
    ("\"Block\"", "Block"),
    ("\"Underline\"", "Underline"),
];
const CLICKS: &[(&str, &str)] = &[
    ("\"Single\"", "Single click"),
    ("\"Double\"", "Double click"),
];
const TERMINAL_CARETS: &[(&str, &str)] = &[
    ("null", "Same as in the editor"),
    ("\"Vertical\"", "Vertical"),
    ("\"Block\"", "Block"),
    ("\"Underline\"", "Underline"),
];

pub const SECTIONS: &[Section] = &[
    Section {
        title: "General",
        groups: &[Group {
            title: None,
            items: &[
                item("Theme", "theme", Kind::Dropdown(THEMES), "\"dark\""),
                // WEEP's own row: Fleet has no skin (`weep.skin`).
                item("Skin", "weep.skin", Kind::Dropdown(SKINS), "\"fleet\""),
                item("Zoom", "zoomLevel", Kind::Dropdown(ZOOMS), "1.0"),
                item("Keymap", "keymap", Kind::Dropdown(KEYMAPS), "\"fleet\""),
                item("Auto save", "autoSave", Kind::Dropdown(AUTO_SAVE), "\"OnFocus\""),
            ],
        }],
    },
    Section {
        title: "Editor",
        groups: &[
            Group {
                title: Some("Font"),
                items: &[
                    item("Font", "editor.fontFamily", Kind::Dropdown(FONTS), "\"JetBrains Mono\""),
                    item("Font size", "editor.fontSize", Kind::Number, "13.0"),
                    item("Line height", "editor.lineSpacing", Kind::Number, "1.7"),
                    check("Enable ligatures", "editor.ligatures", "true"),
                ],
            },
            Group {
                title: Some("Appearance"),
                items: &[
                    check("Enable soft wraps", "editor.softWrapEnabled", "false"),
                    item("Show whitespace", "editor.renderWhitespace", Kind::Dropdown(WHITESPACE), "false"),
                    check("Show indent guides", "editor.showIndentGuides", "true"),
                    described(
                        "Show code interlines",
                        "editor.showCodeVision",
                        "true",
                        "Show markers in the code with the number of usages and implementations (code vision)",
                    ),
                    described(
                        "Show sticky lines",
                        "editor.showStickyLines",
                        "true",
                        "Show the nested scopes at the top of the editor while scrolling",
                    ),
                    check("Show inspections status in the top-right corner", "editor.showFileStatus", "true"),
                    check(
                        "Show Gradle / Maven project auto-import widget in the top-right corner",
                        "editor.showAutoImportWidget",
                        "true",
                    ),
                    check("Show run icons in the gutter", "editor.renderRunInGutter", "true"),
                    item("Line numbers", "editor.lineNumbers", Kind::Dropdown(LINE_NUMBERS), "\"On\""),
                    check("Show visual guides", "editor.showVisualGuides", "true"),
                    link("Visual guides position", "editor.guides"),
                    check(
                        "Show problems and documentation tooltip on hover",
                        "editor.showTooltipOnHover",
                        "true",
                    ),
                    Item {
                        label: "Tooltip delay",
                        key: "editor.hoverTooltipDelayMs",
                        kind: Kind::Number,
                        default: "500",
                        description: None,
                        indent: true,
                    },
                ],
            },
            Group {
                title: Some("Code"),
                items: &[
                    item("Indent using", "editor.indentStyle", Kind::Dropdown(INDENT), "\"Spaces\""),
                    item("Tab size", "editor.tabSize", Kind::Number, "4"),
                    described(
                        "Automatically detect indentation based on file content",
                        "editor.autoDetectIndentation",
                        "true",
                        "Overrides tab size and indentation style settings",
                    ),
                    check("Auto-indent on code editing", "enableIndenter", "true"),
                    check("Reformat code on save", "editor.formatOnSave", "false"),
                    check("Remove trailing spaces on save", "trimTrailingSpacesOnSave", "true"),
                    check("Preselect symbol name on rename refactoring", "rename.selectOnRename", "true"),
                    described(
                        "Honor \"CamelHumps\" and \"snake_case\"",
                        "editor.honorCamelHumps",
                        "false",
                        "Treat word parts as separate words in camel- and snake-case identifiers and literals",
                    ),
                ],
            },
            // M7: the Smart Mode rows. Fleet's page for them was not shot
            // (batch 12); the labels are estimates, the keys and defaults
            // from `documentation.yaml`.
            Group {
                title: Some("Smart Mode"),
                items: &[
                    described(
                        "Semantic highlighting",
                        "editor.semanticHighlighting",
                        "true",
                        "Highlight code based on its semantics, as the language server resolves it",
                    ),
                    item("Completion auto popup delay", "completion.autoPopupDelay", Kind::Number, "100"),
                    check("Rename symbol usages automatically when typing", "rename.autoRenameOnTyping", "true"),
                ],
            },
            Group {
                title: Some("Tabs"),
                items: &[
                    check("Show editor tabs", "editor.showTabs", "true"),
                    item("Tab limit", "editor.tabLimit", Kind::Number, "10"),
                    check("Enable preview tab", "editor.previewTab", "true"),
                ],
            },
            Group {
                title: Some("Caret"),
                items: &[
                    check("Enable caret blinking", "editor.caretBlinkingEnabled", "true"),
                    check("Enable smooth caret animation", "editor.caretSmoothAnimation", "true"),
                    item("Caret shape", "editor.caretShape", Kind::Dropdown(CARETS), "\"Vertical\""),
                ],
            },
            Group {
                title: Some("New File"),
                items: &[
                    check(
                        "Create temporary file using New File action",
                        "createScratchOnNewFile",
                        "true",
                    ),
                    check(
                        "Suggest file templates for new files",
                        "editor.showFileTemplatesSuggestion",
                        "true",
                    ),
                ],
            },
        ],
    },
    Section {
        title: "Tools",
        groups: &[
            Group {
                title: Some("Gradle"),
                items: &[check(
                    "Automatically import project model when build script is saved",
                    "gradle.autoImportOnSave",
                    "true",
                )],
            },
            Group {
                title: Some("Files"),
                items: &[
                    described(
                        "Enable compact folders",
                        "files.compactDirectories",
                        "true",
                        "Display nested subfolders in a compact, single-line view to save vertical space",
                    ),
                    check("Select open file in Files tool", "files.selectOpenedFile", "true"),
                    item("Open file on", "files.openFileMode", Kind::Dropdown(CLICKS), "\"Single\""),
                    item("Expand folder on", "files.toggleFolderMode", Kind::Dropdown(CLICKS), "\"Double\""),
                    link("Exclude", "files.exclude"),
                ],
            },
            Group {
                title: Some("Run and Debug"),
                items: &[check(
                    "Show floating debug toolbar",
                    "editor.showFloatingDebugPanel",
                    "false",
                )],
            },
            Group {
                title: Some("Git"),
                items: &[
                    check("Auto-fetch repositories", "git.autoFetch", "true"),
                    Item {
                        label: "Auto-fetch interval",
                        key: "git.fetchInterval",
                        kind: Kind::Number,
                        default: "15.0",
                        description: None,
                        indent: true,
                    },
                    check("Show changes in the gutter", "editor.showLineStatus", "true"),
                    check("Show confirmation on push", "git.push.confirm", "true"),
                    check("Show confirmation on revert", "git.revert.confirm", "true"),
                ],
            },
            Group {
                title: Some("Terminal"),
                items: &[
                    item("Font", "terminal.fontFamily", Kind::Dropdown(FONTS), "\"JetBrains Mono\""),
                    item("Font size", "terminal.fontSize", Kind::Number, "13.0"),
                    item("Line height", "terminal.lineSpacing", Kind::Number, "1.7"),
                    item("Caret shape", "terminal.caretShape", Kind::Dropdown(TERMINAL_CARETS), "null"),
                    check("Clear terminal on ⌘K", "terminal.clearTerminalOnCmdK", "false"),
                    check("Focus editor on Esc", "terminal.enableFocusEditorOnEsc", "false"),
                    check("Copy to clipboard on selection", "terminal.copyOnSelection", "false"),
                    check("Terminal bell", "terminal.bell", "true"),
                    check("Launch terminal in new workspaces", "launchTerminalOnOpen", "false"),
                    item("Default profile", "terminal.profile.default", Kind::Text, "\"pwsh\""),
                    link("Terminal profiles", "terminal.profiles"),
                ],
            },
            Group {
                title: Some("Docker"),
                items: &[item(
                    "Docker executable",
                    "docker.executable.path",
                    Kind::Text,
                    "\"docker\"",
                )],
            },
        ],
    },
    Section {
        title: "Application",
        groups: &[
            Group {
                title: Some("Appearance"),
                items: &[
                    check("Enable UI animations", "animationEnabled", "true"),
                    check("Show key combinations when using shortcuts", "showShortcuts", "false"),
                ],
            },
            Group {
                title: Some("Status Bar"),
                items: &[
                    check(
                        "Show file language, encoding, and current line",
                        "editor.showFileDetails",
                        "true",
                    ),
                    check("Show navigation breadcrumbs", "editor.showNavBar", "true"),
                ],
            },
            Group {
                title: Some("Window"),
                items: &[
                    check("Reopen workspaces on startup", "restoreWorkspacesOnStartup", "true"),
                    check(
                        "Show startup page when closing the last window",
                        "showEmptyStateOnLastClosedWindow",
                        "true",
                    ),
                    check("Confirm before quitting", "warn.before.quitting", "false"),
                ],
            },
        ],
    },
];

/// The label of a dropdown option for a JSON value, or the value's own text.
pub fn option_label(options: &[(&str, &str)], value: &serde_json::Value) -> String {
    for (json, label) in options {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(json) {
            if values_equal(&v, value) {
                return (*label).to_string();
            }
        }
    }
    match value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Equality that treats `1` and `1.0` as the same number and compares
/// strings without case.
pub fn values_equal(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    use serde_json::Value::*;
    match (a, b) {
        (Number(x), Number(y)) => {
            (x.as_f64().unwrap_or(f64::NAN) - y.as_f64().unwrap_or(f64::NAN)).abs() < 1e-9
        }
        (String(x), String(y)) => x.eq_ignore_ascii_case(y),
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_parse_and_keys_are_unique() {
        let mut keys = Vec::new();
        for s in SECTIONS {
            for g in s.groups {
                for i in g.items {
                    serde_json::from_str::<serde_json::Value>(i.default)
                        .unwrap_or_else(|e| panic!("{}: {e}", i.key));
                    assert!(!keys.contains(&i.key), "duplicate {}", i.key);
                    keys.push(i.key);
                    if let Kind::Dropdown(options) = i.kind {
                        let default: serde_json::Value = serde_json::from_str(i.default).unwrap();
                        assert!(
                            options.iter().any(|(json, _)| {
                                values_equal(&serde_json::from_str(json).unwrap(), &default)
                            }),
                            "{}: default not among its options",
                            i.key
                        );
                    }
                }
            }
        }
        assert_eq!(option_label(ZOOMS, &serde_json::json!(1)), "100%");
    }
}
