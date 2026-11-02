//! Binds the keymap: Fleet's chods for the actions Oak has, read from the
//! bundled `assets/keymap/default.json` (Fleet's own format and defaults),
//! or from a preset the `keymap` setting names (one of Fleet's eight preset
//! files copied to `~/.config/oak/keymaps/<name>.json`), and then from the
//! user's `~/.config/oak/keymap.json` if there is one.
//!
//! The same table serves `goto/action`: every Fleet action id Oak answers
//! to, with the gpui action it dispatches and the chord to show beside it.

use crate::actions::*;
use gpui::{Action, App, Global, KeyBinding};
use oak_keymap::{Keymap, Platform};
use std::collections::HashMap;

const EDITOR: Option<&str> = Some("Editor");
const WORKSPACE: Option<&str> = Some("Workspace");
/// Alt+2 is also `toggle-bottom-panel`; the Goto preview toggle applies only while
/// a popup has the focus, as Fleet resolves it by context.
const POPUP: Option<&str> = Some("Popup");

/// One table, three readers: the key bindings, the boxed action for an id
/// (menus and the Goto panel dispatch it), and the list of canonical ids
/// (the first spelling of each arm) the Goto panel offers.
macro_rules! action_table {
    ($( $first:literal $(| $alias:literal)* => $action:ident, $ctx:ident ;)*) => {
        /// The gpui binding for a Fleet action id, or `None` for one Oak lacks.
        fn binding(keystroke: &str, action: &str) -> Option<KeyBinding> {
            match action {
                $( $first $(| $alias)* => Some(KeyBinding::new(keystroke, $action, $ctx)), )*
                _ => None,
            }
        }

        /// The action a Fleet id stands for, boxed for `Window::dispatch_action`.
        pub fn action_for(action: &str) -> Option<Box<dyn Action>> {
            match action {
                $( $first $(| $alias)* => Some(Box::new($action)), )*
                _ => None,
            }
        }

        /// Whether the id's action runs in the editor (and so needs one focused).
        pub fn is_editor_action(action: &str) -> bool {
            match action {
                $( $first $(| $alias)* => $ctx == EDITOR, )*
                _ => false,
            }
        }

        /// Every Fleet action id Oak answers to, one spelling per action.
        pub const ACTION_IDS: &[&str] = &[ $( $first, )* ];
    };
}

action_table! {
    "save-file" => Save, WORKSPACE;
    "save-all" => SaveAll, WORKSPACE;
    "close-tab" | "tabs/close-tab" => CloseTab, WORKSPACE;
    "tabs/close-other-tabs" => CloseOtherTabs, WORKSPACE;
    "tabs/close-tabs-to-the-right" => CloseTabsToTheRight, WORKSPACE;
    "tabs/close-saved-tabs" => CloseSavedTabs, WORKSPACE;
    "tabs/reopen-closed-tab" => ReopenClosedTab, WORKSPACE;
    "close-window" => CloseWindow, WORKSPACE;
    "goto/go-to-next-tab" | "next-tab" | "tabs/next-tab" | "switch-to-next-file" => NextTab, WORKSPACE;
    "goto/go-to-previous-tab" | "prev-tab" | "tabs/previous-tab" | "switch-to-previous-file" => PrevTab, WORKSPACE;
    "toggle-left-panel" => ToggleLeftPanel, WORKSPACE;
    "toggle-bottom-panel" | "terminal/toggle" => ToggleBottomPanel, WORKSPACE;
    "toggle-right-panel" => ToggleRightPanel, WORKSPACE;
    "hide-all-panels" => HideAllPanels, WORKSPACE;
    "hide-active-panel" => HideActivePanel, WORKSPACE;
    "focus-editor" => FocusEditor, WORKSPACE;
    "new-file" => NewFile, WORKSPACE;
    "split-right" => SplitRight, WORKSPACE;
    // Oak's own id for Fleet's tree-menu row, which has none.
    "open-in-split" => OpenInSplit, WORKSPACE;
    "unsplit" => Unsplit, WORKSPACE;
    "focus-next-split" | "focus-right-split" => FocusNextSplit, WORKSPACE;
    "focus-previous-split" | "focus-left-split" => FocusPreviousSplit, WORKSPACE;
    "focus-first-split" => FocusFirstSplit, WORKSPACE;
    "focus-last-split" => FocusLastSplit, WORKSPACE;
    "move-tab-to-next-split" | "move-tab-to-right-split" => MoveTabToNextSplit, WORKSPACE;
    "move-tab-to-previous-split" | "move-tab-to-left-split" => MoveTabToPreviousSplit, WORKSPACE;
    "move-tab-to-first-split" => MoveTabToFirstSplit, WORKSPACE;
    "move-tab-to-last-split" => MoveTabToLastSplit, WORKSPACE;
    "zoom-in" => ZoomIn, WORKSPACE;
    "zoom-out" => ZoomOut, WORKSPACE;
    "reset-zoom" => ResetZoom, WORKSPACE;
    "toggle-full-screen" => ToggleFullScreen, WORKSPACE;
    "minimize-window" => MinimizeWindow, WORKSPACE;
    "edit-settings-file" => EditSettingsFile, WORKSPACE;
    "edit-keymap-file" => EditKeymapFile, WORKSPACE;
    "open-settings" => OpenSettings, WORKSPACE;
    "quit" => Quit, WORKSPACE;
    "find" => Find, WORKSPACE;
    "replace" => Replace, WORKSPACE;
    "replace-one" => ReplaceOne, WORKSPACE;
    "replace-all" => ReplaceAll, WORKSPACE;
    "next-occurrence" => NextOccurrence, WORKSPACE;
    "prev-occurrence" => PrevOccurrence, WORKSPACE;
    "find-in-path" => FindInPath, WORKSPACE;
    "replace-in-path" | "search/replace" => ReplaceInPath, WORKSPACE;
    "search/jump-to-tool" | "search/new-tool" => SearchJumpToTool, WORKSPACE;
    "files/jump-to-tool" | "files/new-tool" => FilesJumpToTool, WORKSPACE;
    "goto/line" => GotoLine, WORKSPACE;
    "goto/everything" => GotoEverything, WORKSPACE;
    "goto/file" => GotoFile, WORKSPACE;
    "goto/symbol" | "goto/symbol-in-file" => GotoSymbol, WORKSPACE;
    "goto/action" => GotoAction, WORKSPACE;
    "goto/tool" => GotoTool, WORKSPACE;
    "goto/recent-file" => GotoRecentFile, WORKSPACE;
    "show-file-symbols" => ShowFileSymbols, WORKSPACE;
    "toggle-goto-preview" => ToggleGotoPreview, POPUP;
    "files/rename" => RenameFile, WORKSPACE;
    "files/delete" => DeleteFile, WORKSPACE;
    "files/new-folder" => NewFolder, WORKSPACE;
    "toggle-hidden-files" => ToggleHiddenFiles, WORKSPACE;
    "open-in-associated-application" => OpenInAssociatedApplication, WORKSPACE;
    "files/show-in-file-manager" => ShowInFileManager, WORKSPACE;
    "copy-path" | "files/copy-path" => CopyPath, WORKSPACE;
    "show-context-menu" => ShowContextMenu, WORKSPACE;
    "show-main-menu" => ShowMainMenu, WORKSPACE;
    "show-recent-workspaces" => ShowRecentWorkspaces, WORKSPACE;
    "notifications/toggle" => ToggleNotifications, WORKSPACE;
    "notifications/clear-all" => ClearNotifications, WORKSPACE;
    "local-history/make-snapshot" => MakeSnapshot, WORKSPACE;
    "tabs/close-all-tabs" => CloseAllTabs, WORKSPACE;
    "copy-relative-path" => CopyRelativePath, WORKSPACE;
    "about" => About, WORKSPACE;
    "editor/change-line-endings" => ChangeLineEndings, WORKSPACE;
    "editor/paste-from-history" => PasteFromHistory, WORKSPACE;

    "arrow-up" => MoveUp, EDITOR;
    "arrow-down" => MoveDown, EDITOR;
    "arrow-left" => MoveLeft, EDITOR;
    "arrow-right" => MoveRight, EDITOR;
    "shift-arrow-up" => SelectUp, EDITOR;
    "shift-arrow-down" => SelectDown, EDITOR;
    "shift-arrow-left" => SelectLeft, EDITOR;
    "shift-arrow-right" => SelectRight, EDITOR;
    "home" => Home, EDITOR;
    "end" => End, EDITOR;
    "shift-home" => SelectHome, EDITOR;
    "shift-end" => SelectEnd, EDITOR;
    "page-up" => PageUp, EDITOR;
    "page-down" => PageDown, EDITOR;
    "shift-page-up" => SelectPageUp, EDITOR;
    "shift-page-down" => SelectPageDown, EDITOR;
    "editor/move-caret-to-start-of-document" => DocStart, EDITOR;
    "editor/move-caret-to-end-of-document" => DocEnd, EDITOR;
    "editor/move-caret-to-start-of-document-with-selection" => SelectDocStart, EDITOR;
    "editor/move-caret-to-end-of-document-with-selection" => SelectDocEnd, EDITOR;
    "editor/move-caret-to-previous-word" => WordLeft, EDITOR;
    "editor/move-caret-to-next-word" => WordRight, EDITOR;
    "editor/move-caret-to-previous-word-with-selection" => SelectWordLeft, EDITOR;
    "editor/move-caret-to-next-word-with-selection" => SelectWordRight, EDITOR;
    "delete-backward" => Backspace, EDITOR;
    "delete-forward" => Delete, EDITOR;
    "delete-word-backward" => DeleteWordBackward, EDITOR;
    "delete-word-forward" => DeleteWordForward, EDITOR;
    "editor/delete-line" => DeleteLine, EDITOR;
    "editor/duplicate-line" => DuplicateLine, EDITOR;
    "move-line-up" => MoveLineUp, EDITOR;
    "move-line-down" => MoveLineDown, EDITOR;
    "confirm" | "insert-newline-character" | "wrap-line" => Newline, EDITOR;
    "editor/add-caret-above" => AddCaretAbove, EDITOR;
    "editor/add-caret-below" => AddCaretBelow, EDITOR;
    "editor/add-carets-to-ends-of-selected-lines" => AddCaretsToLineEnds, EDITOR;
    "select-next-occurrence" => SelectNextOccurrence, EDITOR;
    "select-all-occurrences" => SelectAllOccurrences, EDITOR;
    "unselect-occurrence" => UnselectOccurrence, EDITOR;
    "expand-selection" => ExpandSelection, EDITOR;
    "shrink-selection" => ShrinkSelection, EDITOR;
    "editor/move-caret-to-matching-bracket" => MoveToMatchingBracket, EDITOR;
    "editor/expand-folding" => ExpandFolding, EDITOR;
    "editor/collapse-folding" => CollapseFolding, EDITOR;
    "toggle-case" => ToggleCase, EDITOR;
    "comment-block" => CommentBlock, EDITOR;
    "editor/insert-new-line-below" => NewlineBelow, EDITOR;
    "editor/insert-new-line-above" => NewlineAbove, EDITOR;
    "tab" => Tab, EDITOR;
    "undo" => Undo, EDITOR;
    "redo" => Redo, EDITOR;
    "copy" => Copy, EDITOR;
    "cut" => Cut, EDITOR;
    "paste" => Paste, EDITOR;
    "select-all" => SelectAll, EDITOR;
    "comment-line" => CommentLine, EDITOR;
    "join-lines" => JoinLines, EDITOR;
    "cancel" => Cancel, EDITOR;
}

/// Ids that are plain key names rather than commands; the Goto panel's
/// Actions list leaves them out, as Fleet's does not list "Up" or "Tab".
pub fn is_plain_key(id: &str) -> bool {
    matches!(
        id,
        "arrow-up"
            | "arrow-down"
            | "arrow-left"
            | "arrow-right"
            | "shift-arrow-up"
            | "shift-arrow-down"
            | "shift-arrow-left"
            | "shift-arrow-right"
            | "home"
            | "end"
            | "shift-home"
            | "shift-end"
            | "page-up"
            | "page-down"
            | "shift-page-up"
            | "shift-page-down"
            | "tab"
            | "confirm"
            | "cancel"
            | "delete-backward"
            | "delete-forward"
    )
}

/// Fleet's presentable name for an action id, as its Goto panel and menus
/// spell it: the last path segment, hyphens to spaces, each word capitalised,
/// with the few ids whose words are not plain English given by hand.
pub fn action_title(id: &str) -> String {
    let special = match id {
        "goto/everything" => Some("Go to Everything"),
        "goto/file" => Some("Go to File"),
        "goto/symbol" => Some("Go to Symbol"),
        "goto/action" => Some("Go to Action"),
        "goto/tool" => Some("Go to Tool"),
        "goto/line" => Some("Go to Line:Column"),
        "goto/recent-file" => Some("Go to Recent File"),
        "goto/go-to-next-tab" => Some("Next Tab"),
        "goto/go-to-previous-tab" => Some("Previous Tab"),
        "find-in-path" => Some("Find in Files"),
        "replace-in-path" => Some("Replace in Files"),
        "search/jump-to-tool" => Some("Search"),
        "files/jump-to-tool" => Some("Files"),
        "files/new-folder" => Some("New Folder"),
        "files/rename" => Some("Rename"),
        "files/delete" => Some("Delete"),
        "files/show-in-file-manager" => Some("Show in File Manager"),
        "copy-path" => Some("Copy Path"),
        "copy-relative-path" => Some("Copy Relative Path"),
        "tabs/close-all-tabs" => Some("Close All Tabs"),
        "about" => Some("About Oak"),
        "open-in-associated-application" => Some("Open in Associated Application"),
        "edit-settings-file" => Some("Edit Settings File"),
        "edit-keymap-file" => Some("Edit Keymap File"),
        "open-settings" => Some("Settings"),
        "editor/paste-from-history" => Some("Paste from History"),
        "editor/change-line-endings" => Some("Change Line Endings"),
        "editor/move-caret-to-start-of-document" => Some("Move Caret to Start of Document"),
        "editor/move-caret-to-end-of-document" => Some("Move Caret to End of Document"),
        "editor/add-carets-to-ends-of-selected-lines" => {
            Some("Add Carets to Ends of Selected Lines")
        }
        "notifications/toggle" => Some("Notifications"),
        "notifications/clear-all" => Some("Clear All Notifications"),
        "local-history/make-snapshot" => Some("Make Local History Snapshot"),
        "confirm" => Some("Enter"),
        "arrow-up" => Some("Up"),
        "arrow-down" => Some("Down"),
        "arrow-left" => Some("Left"),
        "arrow-right" => Some("Right"),
        _ => None,
    };
    if let Some(s) = special {
        return s.to_string();
    }
    let tail = id.rsplit('/').next().unwrap_or(id);
    tail.split('-')
        .filter(|w| !w.is_empty())
        .map(|w| match w {
            "to" | "of" | "in" | "the" | "from" | "with" | "and" | "or" | "a" => w.to_string(),
            _ => {
                let mut c = w.chars();
                match c.next() {
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                    None => String::new(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The keymap in effect: the preset the `keymap` setting names (a file in
/// `~/.config/oak/keymaps/`, else the bundled Fleet defaults) under the
/// user's own file.
pub fn load(preset: &str) -> Keymap {
    let mut keymap = match Keymap::parse(crate::assets::DEFAULT_KEYMAP) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("oak: the bundled keymap does not parse: {e:#}");
            Keymap::default()
        }
    };
    if !preset.is_empty() && preset != "fleet" {
        let file = oak_settings::oak_config_dir()
            .map(|d| d.join("keymaps").join(format!("{preset}.json")));
        match file.as_ref().and_then(|p| std::fs::read_to_string(p).ok()) {
            Some(text) => match Keymap::parse(&text) {
                // A preset replaces Fleet's chords for the actions it names.
                Ok(p) => keymap.merge(p),
                Err(e) => eprintln!("oak: keymap preset {preset}: {e:#}"),
            },
            None => eprintln!(
                "oak: keymap preset {preset:?} not found; copy Fleet's {preset}.json to {}",
                file.map(|p| p.display().to_string()).unwrap_or_default()
            ),
        }
    }
    if let Some(path) = crate::settings::keymap_file() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            match Keymap::parse(&text) {
                Ok(user) => keymap.merge(user),
                Err(e) => eprintln!("oak: {} does not parse: {e:#}", path.display()),
            }
        }
    }
    keymap
}

/// A chord in Fleet's tooltip spelling ("Ctrl+Shift+F").
fn presentable(chord: &str) -> String {
    chord
        .split('-')
        .map(|part| {
            let mut c = part.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join("+")
        .replace("Digit", "")
        .replace("Cmd", "⌘")
}

/// Every action's first chord, for the menus and the Goto panel; a global
/// so a view can read it in `render`.
#[derive(Clone, Debug, Default)]
pub struct Hints(HashMap<String, String>);

impl Global for Hints {}

impl Hints {
    pub fn from_keymap(keymap: &Keymap) -> Hints {
        let mut map = HashMap::new();
        for r in keymap.resolved(Platform::current()) {
            map.entry(r.action.clone())
                .or_insert_with(|| presentable(&r.chord));
        }
        Hints(map)
    }

    pub fn get(&self, action: &str) -> Option<&str> {
        self.0.get(action).map(String::as_str)
    }
}

/// The hint for an action id from the global table.
pub fn hint_for(cx: &App, action: &str) -> Option<String> {
    cx.try_global::<Hints>()
        .and_then(|h| h.get(action).map(str::to_string))
}

pub fn bind(keymap: &Keymap, cx: &mut App) {
    let mut bindings = Vec::new();
    for r in keymap.resolved(Platform::current()) {
        // gpui's parser refuses a few spellings; a bad chord must not take the app down.
        let parsed = std::panic::catch_unwind(|| binding(&r.keystroke, &r.action));
        match parsed {
            Ok(Some(b)) => bindings.push(b),
            Ok(None) => {}
            Err(_) => eprintln!("oak: cannot bind {:?} ({})", r.chord, r.action),
        }
    }
    cx.bind_keys(bindings);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles() {
        assert_eq!(action_title("tabs/close-other-tabs"), "Close Other Tabs");
        assert_eq!(action_title("goto/file"), "Go to File");
        assert_eq!(action_title("editor/duplicate-line"), "Duplicate Line");
        assert_eq!(
            action_title("move-tab-to-next-split"),
            "Move Tab to Next Split"
        );
    }

    #[test]
    fn table_is_consistent() {
        for id in ACTION_IDS {
            assert!(action_for(id).is_some(), "{id}");
        }
        assert!(is_editor_action("copy"));
        assert!(!is_editor_action("save-file"));
    }
}
