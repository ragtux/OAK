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
        "close-tab" => b!(CloseTab, WORKSPACE),
        "close-window" => b!(CloseWindow, WORKSPACE),
        "goto/go-to-next-tab" | "next-tab" | "switch-to-next-file" => b!(NextTab, WORKSPACE),
        "goto/go-to-previous-tab" | "prev-tab" | "switch-to-previous-file" => {
            b!(PrevTab, WORKSPACE)
        }
        "toggle-left-panel" => b!(ToggleLeftPanel, WORKSPACE),
        "toggle-bottom-panel" | "terminal/toggle" => b!(ToggleBottomPanel, WORKSPACE),
        "toggle-right-panel" => b!(ToggleRightPanel, WORKSPACE),
        "hide-all-panels" => b!(HideAllPanels, WORKSPACE),
        "focus-editor" => b!(FocusEditor, WORKSPACE),
        "new-file" => b!(NewFile, WORKSPACE),
        "quit" => b!(Quit, WORKSPACE),

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
        "confirm" | "insert-newline-character" => b!(Newline, EDITOR),
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

fn user_keymap_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config"))
        })?;
    Some(base.join("oak/keymap.json"))
}

pub fn bind(cx: &mut App) {
    let mut keymap = match Keymap::parse(crate::assets::DEFAULT_KEYMAP) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("oak: the bundled keymap does not parse: {e:#}");
            Keymap::default()
        }
    };
    if let Some(path) = user_keymap_path() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            match Keymap::parse(&text) {
                Ok(user) => keymap.merge(user),
                Err(e) => eprintln!("oak: {} does not parse: {e:#}", path.display()),
            }
        }
    }
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
