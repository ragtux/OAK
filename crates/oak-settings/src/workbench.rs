//! The keys the M3 workbench tools read: the Goto panel, the Search tool,
//! the Files tool's navigation mode, keymap presets, windows and quitting.
//! Each accessor names its Fleet key; defaults marked "estimate" have not
//! been read off Fleet's Settings tool yet (`docs/probe-plan.md`, batch 8).

use crate::Settings;

/// How the arrow keys move in the Files tree (`files.keyboardNavigation`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyboardNavigation {
    /// The arrows move the selection.
    Select,
    /// The arrows move a cursor rectangle; Space selects.
    Highlight,
}

impl Settings {
    /// `goto_panel.showShortcuts`: "Display helper keyboard shortcuts in the
    /// Goto panel". Default true (estimate).
    pub fn goto_panel_show_shortcuts(&self) -> bool {
        self.bool("goto_panel.showShortcuts").unwrap_or(true)
    }

    /// `ff.previewAndPrioritiseFileSymbolsInGotoEverywhere`: "file symbols
    /// will be displayed at the top of the Goto panel, even when the query is
    /// empty". Default false (a feature flag).
    pub fn ff_preview_file_symbols_in_goto(&self) -> bool {
        self.bool("ff.previewAndPrioritiseFileSymbolsInGotoEverywhere")
            .unwrap_or(false)
    }

    /// `ff.gotoSymbolByExactName`: "Match only member names instead of their
    /// entire signatures when searching for symbols in a file". Default false.
    pub fn ff_goto_symbol_by_exact_name(&self) -> bool {
        self.bool("ff.gotoSymbolByExactName").unwrap_or(false)
    }

    /// `ff.searchInBigUnindexedFiles`: "Enable searching in files that were
    /// excluded from the text indices due to their size". Default false.
    pub fn ff_search_in_big_unindexed_files(&self) -> bool {
        self.bool("ff.searchInBigUnindexedFiles").unwrap_or(false)
    }

    /// `files.keyboardNavigation`: "select" (the arrows move the selection)
    /// or "highlight" (the arrows move a cursor rectangle; Space selects).
    /// Default select (estimate: the documentation calls it the basic mode).
    pub fn files_keyboard_navigation(&self) -> KeyboardNavigation {
        let value = self
            .str("files.keyboardNavigation")
            .map(|s| s.to_ascii_lowercase());
        match value.as_deref() {
            Some("highlight") => KeyboardNavigation::Highlight,
            _ => KeyboardNavigation::Select,
        }
    }

    /// `keymap`: "The keymap preset, which defines the mapping between
    /// triggers and key combinations" (fleet, intellij, ij-classic,
    /// visual-studio, vscode, sublimeText, eclipse, emacs). Default fleet.
    pub fn keymap(&self) -> &str {
        self.str("keymap").unwrap_or("fleet")
    }

    /// `inputSequenceTimeoutMilliseconds`: "Maximum time between inputs in a
    /// sequence of key combinations". Default 1000 (estimate).
    pub fn input_sequence_timeout_ms(&self) -> u64 {
        self.usize("inputSequenceTimeoutMilliseconds")
            .map(|n| n as u64)
            .unwrap_or(1000)
    }

    /// `showShortcuts`: the shortcut overlay on the toolbar's buttons.
    /// Default false (estimate).
    pub fn show_shortcuts(&self) -> bool {
        self.bool("showShortcuts").unwrap_or(false)
    }

    /// `warn.before.quitting`: "Asks confirmation before quitting when using
    /// a shortcut". Default false (estimate).
    pub fn warn_before_quitting(&self) -> bool {
        self.bool("warn.before.quitting").unwrap_or(false)
    }

    /// `restoreWorkspacesOnStartup`: "Reopen workspaces from the previous
    /// session on Fleet startup". Default true (estimate).
    pub fn restore_workspaces_on_startup(&self) -> bool {
        self.bool("restoreWorkspacesOnStartup").unwrap_or(true)
    }

    /// `showEmptyStateOnLastClosedWindow`: "Show startup page when closing
    /// the last remaining workspace". Default true (estimate).
    pub fn show_empty_state_on_last_closed_window(&self) -> bool {
        self.bool("showEmptyStateOnLastClosedWindow")
            .unwrap_or(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workbench_defaults() {
        let s = Settings::parse("{}").unwrap();
        assert!(s.goto_panel_show_shortcuts());
        assert_eq!(s.files_keyboard_navigation(), KeyboardNavigation::Select);
        assert_eq!(s.keymap(), "fleet");
        assert!(!s.warn_before_quitting());
        let s = Settings::parse(
            r#"{"files.keyboardNavigation": "Highlight", "keymap": "vscode", "inputSequenceTimeoutMilliseconds": 500}"#,
        )
        .unwrap();
        assert_eq!(s.files_keyboard_navigation(), KeyboardNavigation::Highlight);
        assert_eq!(s.keymap(), "vscode");
        assert_eq!(s.input_sequence_timeout_ms(), 500);
    }
}
