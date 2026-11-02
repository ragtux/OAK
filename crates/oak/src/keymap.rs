//! Binds the keymap: Fleet's chords for the actions Oak has, read from the
//! bundled `assets/keymap/default.json` (Fleet's own format and defaults)
//! and then from the user's `~/.config/oak/keymap.json` if there is one.

use crate::actions::*;
use gpui::{App, KeyBinding};
use oak_keymap::{Keymap, Platform};

const EDITOR: Option<&str> = Some("Editor");
const WORKSPACE: Option<&str> = Some("Workspace");

/// The gpui binding for a Fleet action id, or `None` for one Oak lacks.
fn binding(keystroke: &str, action: &str) -> Option<KeyBinding> {
    macro_rules! b {
        ($action:expr, $ctx:expr) => {
            Some(KeyBinding::new(keystroke, $action, $ctx))
        };
    }
    match action {
        "save-file" => b!(Save, WORKSPACE),
        "save-all" => b!(SaveAll, WORKSPACE),
        "close-tab" | "tabs/close-tab" => b!(CloseTab, WORKSPACE),
        "tabs/close-other-tabs" => b!(CloseOtherTabs, WORKSPACE),
        "tabs/close-tabs-to-the-right" => b!(CloseTabsToTheRight, WORKSPACE),
        "tabs/close-saved-tabs" => b!(CloseSavedTabs, WORKSPACE),
        "tabs/reopen-closed-tab" => b!(ReopenClosedTab, WORKSPACE),
        "close-window" => b!(CloseWindow, WORKSPACE),
        "goto/go-to-next-tab" | "next-tab" | "tabs/next-tab" | "switch-to-next-file" => {
            b!(NextTab, WORKSPACE)
        }
        "goto/go-to-previous-tab"
        | "prev-tab"
        | "tabs/previous-tab"
        | "switch-to-previous-file" => b!(PrevTab, WORKSPACE),
        "toggle-left-panel" => b!(ToggleLeftPanel, WORKSPACE),
        "toggle-bottom-panel" | "terminal/toggle" => b!(ToggleBottomPanel, WORKSPACE),
        "toggle-right-panel" => b!(ToggleRightPanel, WORKSPACE),
        "hide-all-panels" => b!(HideAllPanels, WORKSPACE),
        "hide-active-panel" => b!(HideActivePanel, WORKSPACE),
        "focus-editor" => b!(FocusEditor, WORKSPACE),
        "new-file" => b!(NewFile, WORKSPACE),
        "split-right" => b!(SplitRight, WORKSPACE),
        "unsplit" => b!(Unsplit, WORKSPACE),
        "focus-next-split" | "focus-right-split" => b!(FocusNextSplit, WORKSPACE),
        "focus-previous-split" | "focus-left-split" => b!(FocusPreviousSplit, WORKSPACE),
        "focus-first-split" => b!(FocusFirstSplit, WORKSPACE),
        "focus-last-split" => b!(FocusLastSplit, WORKSPACE),
        "move-tab-to-next-split" | "move-tab-to-right-split" => b!(MoveTabToNextSplit, WORKSPACE),
        "move-tab-to-previous-split" | "move-tab-to-left-split" => {
            b!(MoveTabToPreviousSplit, WORKSPACE)
        }
        "move-tab-to-first-split" => b!(MoveTabToFirstSplit, WORKSPACE),
        "move-tab-to-last-split" => b!(MoveTabToLastSplit, WORKSPACE),
        "zoom-in" => b!(ZoomIn, WORKSPACE),
        "zoom-out" => b!(ZoomOut, WORKSPACE),
        "reset-zoom" => b!(ResetZoom, WORKSPACE),
        "toggle-full-screen" => b!(ToggleFullScreen, WORKSPACE),
        "minimize-window" => b!(MinimizeWindow, WORKSPACE),
        "edit-settings-file" => b!(EditSettingsFile, WORKSPACE),
        "edit-keymap-file" => b!(EditKeymapFile, WORKSPACE),
        "quit" => b!(Quit, WORKSPACE),
        "find" => b!(Find, WORKSPACE),
        "replace" => b!(Replace, WORKSPACE),
        "replace-one" => b!(ReplaceOne, WORKSPACE),
        "replace-all" => b!(ReplaceAll, WORKSPACE),
        "next-occurrence" => b!(NextOccurrence, WORKSPACE),
        "prev-occurrence" => b!(PrevOccurrence, WORKSPACE),
        "goto/line" => b!(GotoLine, WORKSPACE),
        "files/rename" => b!(RenameFile, WORKSPACE),
        "files/delete" => b!(DeleteFile, WORKSPACE),
        "editor/change-line-endings" => b!(ChangeLineEndings, WORKSPACE),
        "editor/paste-from-history" => b!(PasteFromHistory, WORKSPACE),

        "arrow-up" => b!(MoveUp, EDITOR),
        "arrow-down" => b!(MoveDown, EDITOR),
        "arrow-left" => b!(MoveLeft, EDITOR),
        "arrow-right" => b!(MoveRight, EDITOR),
        "shift-arrow-up" => b!(SelectUp, EDITOR),
        "shift-arrow-down" => b!(SelectDown, EDITOR),
        "shift-arrow-left" => b!(SelectLeft, EDITOR),
        "shift-arrow-right" => b!(SelectRight, EDITOR),
        "home" => b!(Home, EDITOR),
        "end" => b!(End, EDITOR),
        "shift-home" => b!(SelectHome, EDITOR),
        "shift-end" => b!(SelectEnd, EDITOR),
        "page-up" => b!(PageUp, EDITOR),
        "page-down" => b!(PageDown, EDITOR),
        "shift-page-up" => b!(SelectPageUp, EDITOR),
        "shift-page-down" => b!(SelectPageDown, EDITOR),
        "editor/move-caret-to-start-of-document" => b!(DocStart, EDITOR),
        "editor/move-caret-to-end-of-document" => b!(DocEnd, EDITOR),
        "editor/move-caret-to-start-of-document-with-selection" => b!(SelectDocStart, EDITOR),
        "editor/move-caret-to-end-of-document-with-selection" => b!(SelectDocEnd, EDITOR),
        "editor/move-caret-to-previous-word" => b!(WordLeft, EDITOR),
        "editor/move-caret-to-next-word" => b!(WordRight, EDITOR),
        "editor/move-caret-to-previous-word-with-selection" => b!(SelectWordLeft, EDITOR),
        "editor/move-caret-to-next-word-with-selection" => b!(SelectWordRight, EDITOR),
        "delete-backward" => b!(Backspace, EDITOR),
        "delete-forward" => b!(Delete, EDITOR),
        "delete-word-backward" => b!(DeleteWordBackward, EDITOR),
        "delete-word-forward" => b!(DeleteWordForward, EDITOR),
        "editor/delete-line" => b!(DeleteLine, EDITOR),
        "editor/duplicate-line" => b!(DuplicateLine, EDITOR),
        "move-line-up" => b!(MoveLineUp, EDITOR),
        "move-line-down" => b!(MoveLineDown, EDITOR),
        "confirm" | "insert-newline-character" | "wrap-line" => b!(Newline, EDITOR),
        "editor/add-caret-above" => b!(AddCaretAbove, EDITOR),
        "editor/add-caret-below" => b!(AddCaretBelow, EDITOR),
        "editor/add-carets-to-ends-of-selected-lines" => b!(AddCaretsToLineEnds, EDITOR),
        "select-next-occurrence" => b!(SelectNextOccurrence, EDITOR),
        "select-all-occurrences" => b!(SelectAllOccurrences, EDITOR),
        "unselect-occurrence" => b!(UnselectOccurrence, EDITOR),
        "expand-selection" => b!(ExpandSelection, EDITOR),
        "shrink-selection" => b!(ShrinkSelection, EDITOR),
        "editor/move-caret-to-matching-bracket" => b!(MoveToMatchingBracket, EDITOR),
        "editor/expand-folding" => b!(ExpandFolding, EDITOR),
        "editor/collapse-folding" => b!(CollapseFolding, EDITOR),
        "toggle-case" => b!(ToggleCase, EDITOR),
        "comment-block" => b!(CommentBlock, EDITOR),
        "editor/insert-new-line-below" => b!(NewlineBelow, EDITOR),
        "editor/insert-new-line-above" => b!(NewlineAbove, EDITOR),
        "tab" => b!(Tab, EDITOR),
        "undo" => b!(Undo, EDITOR),
        "redo" => b!(Redo, EDITOR),
        "copy" => b!(Copy, EDITOR),
        "cut" => b!(Cut, EDITOR),
        "paste" => b!(Paste, EDITOR),
        "select-all" => b!(SelectAll, EDITOR),
        "comment-line" => b!(CommentLine, EDITOR),
        "join-lines" => b!(JoinLines, EDITOR),
        "cancel" => b!(Cancel, EDITOR),
        _ => None,
    }
}

/// The keymap in effect: the bundled Fleet defaults under the user's file.
pub fn load() -> Keymap {
    let mut keymap = match Keymap::parse(crate::assets::DEFAULT_KEYMAP) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("oak: the bundled keymap does not parse: {e:#}");
            Keymap::default()
        }
    };
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

/// The chord bound to a Fleet action id on this platform, spelled for a
/// tooltip ("Alt+1"), or none.
pub fn hint(keymap: &Keymap, action: &str) -> Option<String> {
    keymap
        .resolved(Platform::current())
        .into_iter()
        .find(|r| r.action == action)
        .map(|r| {
            r.chord
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
        })
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
