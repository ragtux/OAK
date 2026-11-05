//! Binds the keymap: Fleet's chods for the actions WEEP has, read from the
//! bundled `assets/keymap/default.json` (Fleet's own format and defaults),
//! or from a preset the `keymap` setting names (one of Fleet's eight preset
//! files copied to `~/.config/weep/keymaps/<name>.json`), and then from the
//! user's `~/.config/weep/keymap.json` if there is one.
//!
//! The same table serves `goto/action`: every Fleet action id WEEP answers
//! to, with the gpui action it dispatches and the chord to show beside it.

use crate::actions::*;
use gpui::{Action, App, Global, KeyBinding};
use std::collections::HashMap;
use weep_keymap::{Keymap, Platform};

const EDITOR: Option<&str> = Some("Editor");
const WORKSPACE: Option<&str> = Some("Workspace");
/// Alt+2 is also `toggle-bottom-panel`; the Goto preview toggle applies only while
/// a popup has the focus, as Fleet resolves it by context.
const POPUP: Option<&str> = Some("Popup");
/// A focused terminal's own copy, paste, select-all and clear (M5).
const TERMINAL: Option<&str> = Some("Terminal");
/// M8: the debugger tools' own actions (`debug_tool.rs` sets these contexts):
/// the watches in the Debug tool, the breakpoint list's rows. Fleet's
/// Backspace and Delete for them would delete text anywhere else.
const DEBUG_TOOL: Option<&str> = Some("DebugTool");
const BREAKPOINTS_TOOL: Option<&str> = Some("BreakpointsTool");

/// One table, three readers: the key bindings, the boxed action for an id
/// (menus and the Goto panel dispatch it), and the list of canonical ids
/// (the first spelling of each arm) the Goto panel offers. A row may name
/// more contexts after the first (`EDITOR + BREAKPOINTS_TOOL`): its chords
/// bind in each; the first decides `is_editor_action`.
macro_rules! action_table {
    ($( $first:literal $(| $alias:literal)* => $action:ident, $ctx:ident $(+ $more:ident)* ;)*) => {
        /// The gpui bindings for a Fleet action id (one per context), empty
        /// for one WEEP lacks.
        fn bindings(keystroke: &str, action: &str) -> Vec<KeyBinding> {
            match action {
                $( $first $(| $alias)* => vec![
                    KeyBinding::new(keystroke, $action, $ctx)
                    $(, KeyBinding::new(keystroke, $action, $more))*
                ], )*
                _ => Vec::new(),
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

        /// Every Fleet action id WEEP answers to, one spelling per action.
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
    "toggle-bottom-panel" => ToggleBottomPanel, WORKSPACE;
    "toggle-right-panel" => ToggleRightPanel, WORKSPACE;
    "hide-all-panels" => HideAllPanels, WORKSPACE;
    "hide-active-panel" => HideActivePanel, WORKSPACE;
    "focus-editor" => FocusEditor, WORKSPACE;
    "new-file" => NewFile, WORKSPACE;
    "split-right" => SplitRight, WORKSPACE;
    // WEEP's own id for Fleet's tree-menu row, which has none.
    "open-in-split" => OpenInSplit, WORKSPACE;
    "git/commit/jump-to-tool" | "git/commit/new-tool" => GitCommitJumpToTool, WORKSPACE;
    "git/file-history" => GitFileHistory, WORKSPACE;
    "git/history/jump-to-tool" | "git/history/new-tool" => GitHistoryJumpToTool, WORKSPACE;
    "git/toggle-blame" => GitToggleBlame, WORKSPACE;
    "git/push" => GitPush, WORKSPACE;
    "git/pull" => GitPull, WORKSPACE;
    "git/revert" => GitRevert, WORKSPACE;
    "git/interline-toolbar/show-diff" => GitShowDiff, WORKSPACE;
    "new-diff-tab-tool" => NewDiffTab, WORKSPACE;
    "diff/two-files" => DiffTwoFiles, WORKSPACE;
    "diff/with-clipboard" => DiffWithClipboard, WORKSPACE;
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
    "run-and-debug" => RunAndDebug, WORKSPACE;
    "run/run-in-context" => RunInContext, WORKSPACE;
    "run/rerun" => Rerun, WORKSPACE;
    "stop-run-configurations" => StopRunConfigurations, WORKSPACE;
    "run/stop" => RunStop, WORKSPACE;
    "run/edit-configurations" => RunEditConfigurations, WORKSPACE;
    "run/delete-configuration" => RunDeleteConfiguration, POPUP;
    "editor/go-to-next-problem" => GoToNextProblem, EDITOR;
    "editor/go-to-previous-problem" => GoToPreviousProblem, EDITOR;
    "terminal/new-tool" => TerminalNewTool, WORKSPACE;
    "terminal/toggle" => TerminalToggle, WORKSPACE;
    "terminal/jump-to-tool" => TerminalJumpToTool, WORKSPACE;
    "terminal/new-with-profile" => TerminalNewWithProfile, WORKSPACE;
    "terminal/select-default-profile" => TerminalSelectDefaultProfile, WORKSPACE;
    "terminal/edit-profiles" => TerminalEditProfiles, WORKSPACE;
    "terminal/clear" => TerminalClear, TERMINAL;
    "terminal/copy" => TerminalCopy, TERMINAL;
    "terminal/paste" => TerminalPaste, TERMINAL;
    "select-all-terminal" => SelectAllTerminal, TERMINAL;
    "local-history/make-snapshot" => MakeSnapshot, WORKSPACE;
    "tabs/close-all-tabs" => CloseAllTabs, WORKSPACE;
    "copy-relative-path" => CopyRelativePath, WORKSPACE;
    "about" => About, WORKSPACE;
    "editor/change-line-endings" => ChangeLineEndings, WORKSPACE;
    "editor/paste-from-history" => PasteFromHistory, WORKSPACE;
    // M7: Smart Mode. Run by the workspace, which owns the language servers,
    // on the focused editor.
    "completion" => Completion, EDITOR;
    "smart-completion" => SmartCompletion, EDITOR;
    "show-parameter-info" => ShowParameterInfo, EDITOR;
    "show-documentation" => ShowDocumentation, EDITOR;
    "go-to-declaration" => GoToDeclaration, EDITOR;
    "go-to-type-declaration" => GoToTypeDeclaration, EDITOR;
    "find-usages" => FindUsages, EDITOR;
    "rename" => RenameSymbol, EDITOR;
    "quick-fixes" => QuickFixes, EDITOR;
    "apply-primary-quick-fix" => ApplyPrimaryQuickFix, EDITOR;
    "reformat-code" => ReformatCode, EDITOR;
    "optimize-imports" => OptimizeImports, EDITOR;
    "back" => Back, WORKSPACE;
    "forward" => Forward, WORKSPACE;
    // M8: the debugger. Sessions, steps and the tools run by the workspace
    // (`workspace/debug.rs`); breakpoints and quick evaluate in the editor;
    // the watches and the breakpoint list in their tools.
    "debug/debug-in-context" => DebugInContext, WORKSPACE;
    "debug/resume" => DebugResume, WORKSPACE;
    "debug/pause" => DebugPause, WORKSPACE;
    "debug/step-over" => DebugStepOver, WORKSPACE;
    "debug/step-into" => DebugStepInto, WORKSPACE;
    "debug/force-step-into" => DebugForceStepInto, WORKSPACE;
    "debug/step-out" => DebugStepOut, WORKSPACE;
    "debug/stop" => DebugStop, WORKSPACE;
    "debug/toggle-breakpoint" => DebugToggleBreakpoint, EDITOR;
    "debug/toggle-enabled-breakpoint" => DebugToggleEnabledBreakpoint, EDITOR + BREAKPOINTS_TOOL;
    "debug/remove-breakpoint" => DebugRemoveBreakpoint, BREAKPOINTS_TOOL;
    "debug/add-watch" => DebugAddWatch, DEBUG_TOOL;
    "debug/edit-watch" => DebugEditWatch, DEBUG_TOOL;
    "debug/delete-watch" => DebugDeleteWatch, DEBUG_TOOL;
    "debug/quick-evaluate-expression" => DebugQuickEvaluateExpression, EDITOR;
    "debug/jump-to-tool" => DebugJumpToTool, WORKSPACE;
    "debug/new-tool" => DebugNewTool, WORKSPACE;
    "debug/breakpoints/jump-to-tool" => DebugBreakpointsJumpToTool, WORKSPACE;
    "new-breakpoints-tool" => NewBreakpointsTool, WORKSPACE;
    "debug/console/jump-to-tool" => DebugConsoleJumpToTool, WORKSPACE;
    "debug/console/new-tool" => DebugConsoleNewTool, WORKSPACE;
    "debugger/attach-to-process" => DebuggerAttachToProcess, WORKSPACE;

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
        "about" => Some("About WEEP"),
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
        "git/commit/jump-to-tool" => Some("Commit"),
        "git/commit/new-tool" => Some("New Commit Tool"),
        "git/history/jump-to-tool" => Some("History"),
        "git/history/new-tool" => Some("New History Tool"),
        "git/file-history" => Some("File History"),
        "git/interline-toolbar/show-diff" => Some("Show Diff"),
        "new-diff-tab-tool" => Some("Changed Files"),
        "diff/two-files" => Some("Compare Two Files"),
        "diff/with-clipboard" => Some("Compare with Clipboard"),
        "local-history/make-snapshot" => Some("Make Local History Snapshot"),
        // M5: the first six as Fleet's terminal menu spells them (batch 9),
        // the rest estimates.
        // M6: as the Run & Debug popup and its tooltips spell them (batch 11),
        // the rest estimates.
        "run-and-debug" => Some("Run & Debug"),
        "run/run-in-context" => Some("Run in Context"),
        "run/rerun" => Some("Rerun"),
        "stop-run-configurations" => Some("Stop Run Configurations"),
        "run/stop" => Some("Stop"),
        "run/edit-configurations" => Some("Edit Run Configurations..."),
        "run/delete-configuration" => Some("Delete Configuration"),
        "editor/go-to-next-problem" => Some("Next Problem"),
        "editor/go-to-previous-problem" => Some("Previous Problem"),
        "terminal/new-tool" => Some("New Terminal"),
        "terminal/new-with-profile" => Some("New Terminal With Profile..."),
        "terminal/copy" => Some("Copy"),
        "terminal/paste" => Some("Paste"),
        "select-all-terminal" => Some("Select All"),
        "terminal/clear" => Some("Clear"),
        "terminal/toggle" => Some("Toggle Terminal"),
        "terminal/jump-to-tool" => Some("Terminal"),
        "terminal/select-default-profile" => Some("Select Default Terminal Profile"),
        "terminal/edit-profiles" => Some("Edit Terminal Profiles"),
        // M7: as Fleet's editor menu spells them (batch 7 caught Quick
        // Fixes..., Reformat Code, Go To and Refactor); the rest estimates.
        "completion" => Some("Code Completion"),
        "smart-completion" => Some("Smart Code Completion"),
        "show-parameter-info" => Some("Show Parameter Info"),
        "show-documentation" => Some("Show Documentation"),
        "go-to-declaration" => Some("Go to Declaration"),
        "go-to-type-declaration" => Some("Go to Type Declaration"),
        "find-usages" => Some("Find Usages"),
        "rename" => Some("Rename..."),
        "quick-fixes" => Some("Quick Fixes..."),
        "apply-primary-quick-fix" => Some("Apply Primary Quick Fix"),
        "reformat-code" => Some("Reformat Code"),
        "optimize-imports" => Some("Optimize Imports"),
        "back" => Some("Back"),
        "forward" => Some("Forward"),
        // M8: estimates (no Fleet menu or tooltip of the debugger captured
        // yet), in IntelliJ's words where Fleet's family shares them.
        "debug/debug-in-context" => Some("Debug in Context"),
        "debug/resume" => Some("Resume"),
        "debug/pause" => Some("Pause"),
        "debug/step-over" => Some("Step Over"),
        "debug/step-into" => Some("Step Into"),
        "debug/force-step-into" => Some("Force Step Into"),
        "debug/step-out" => Some("Step Out"),
        "debug/stop" => Some("Stop Debugging"),
        "debug/toggle-breakpoint" => Some("Toggle Line Breakpoint"),
        "debug/toggle-enabled-breakpoint" => Some("Enable/Disable Breakpoint"),
        "debug/remove-breakpoint" => Some("Remove Breakpoint"),
        "debug/add-watch" => Some("Add Watch"),
        "debug/edit-watch" => Some("Edit Watch"),
        "debug/delete-watch" => Some("Delete Watch"),
        "debug/quick-evaluate-expression" => Some("Quick Evaluate Expression"),
        "debug/jump-to-tool" => Some("Debug"),
        "debug/new-tool" => Some("New Debug Tool"),
        "debug/breakpoints/jump-to-tool" => Some("Breakpoints"),
        "new-breakpoints-tool" => Some("New Breakpoints Tool"),
        "debug/console/jump-to-tool" => Some("Debugger Console"),
        "debug/console/new-tool" => Some("New Debugger Console Tool"),
        "debugger/attach-to-process" => Some("Attach to Process..."),
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
/// `~/.config/weep/keymaps/`, else the bundled Fleet defaults) under the
/// user's own file.
pub fn load(preset: &str) -> Keymap {
    let mut keymap = match Keymap::parse(crate::assets::DEFAULT_KEYMAP) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("weep: the bundled keymap does not parse: {e:#}");
            Keymap::default()
        }
    };
    if !preset.is_empty() && preset != "fleet" {
        let file = weep_settings::weep_config_dir()
            .map(|d| d.join("keymaps").join(format!("{preset}.json")));
        match file.as_ref().and_then(|p| std::fs::read_to_string(p).ok()) {
            Some(text) => match Keymap::parse(&text) {
                // A preset replaces Fleet's chords for the actions it names.
                Ok(p) => keymap.merge(p),
                Err(e) => eprintln!("weep: keymap preset {preset}: {e:#}"),
            },
            None => eprintln!(
                "weep: keymap preset {preset:?} not found; copy Fleet's {preset}.json to {}",
                file.map(|p| p.display().to_string()).unwrap_or_default()
            ),
        }
    }
    if let Some(path) = crate::settings::keymap_file() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            match Keymap::parse(&text) {
                Ok(user) => keymap.merge(user),
                Err(e) => eprintln!("weep: {} does not parse: {e:#}", path.display()),
            }
        }
    }
    keymap
}

/// A chord in Fleet's tooltip spelling for this platform ("Ctrl+Shift+F" on
/// Windows and Linux, "⇧⌘F" on macOS; a sequence as "Ctrl+Shift+G, Ctrl+Shift+A").
pub fn presentable(chord: &str) -> String {
    presentable_on(chord, Platform::current())
}

/// [`presentable`] for a platform. macOS spells a chord the system's way:
/// the modifier glyphs in the order ⌃⌥⇧⌘ with no separators, then the key,
/// as the menu bar does (fourteenth pass: an estimate, Fleet's own macOS
/// tooltips not yet captured).
pub fn presentable_on(chord: &str, platform: Platform) -> String {
    chord
        .split_whitespace()
        .map(|chord| match platform {
            Platform::Mac => mac_chord(chord),
            _ => chord
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
                .replace("Cmd", "⌘"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// A chord written into Fleet's own text (a placeholder, a menu row): Fleet's
/// words as measured on Windows and Linux, the chord in macOS's spelling
/// on a Mac.
pub fn chord_text(fleet_text: &str, mac_chord: &str) -> String {
    if cfg!(target_os = "macos") {
        presentable_on(mac_chord, Platform::Mac)
    } else {
        fleet_text.to_string()
    }
}

/// One chord in macOS's spelling: `cmd-shift-f` → `⇧⌘F`.
fn mac_chord(chord: &str) -> String {
    let mut parts: Vec<&str> = chord.split('-').collect();
    // A chord ending in the "-" key itself splits into an empty last part.
    if parts.len() >= 2 && parts.last() == Some(&"") {
        parts.pop();
        let n = parts.len();
        parts[n - 1] = "minus";
    }
    let Some((key, mods)) = parts.split_last() else {
        return String::new();
    };
    let has = |names: &[&str]| {
        mods.iter()
            .any(|m| names.contains(&m.to_ascii_lowercase().as_str()))
    };
    let mut out = String::new();
    for (names, glyph) in [
        (&["ctrl", "control"][..], "⌃"),
        (&["alt", "option"][..], "⌥"),
        (&["shift"][..], "⇧"),
        (&["cmd", "command", "meta", "super", "win"][..], "⌘"),
    ] {
        if has(names) {
            out.push_str(glyph);
        }
    }
    let k = key.to_ascii_lowercase();
    let name = match k.as_str() {
        "return" | "enter" | "numpadenter" => "↩".to_string(),
        "backspace" => "⌫".to_string(),
        "delete" | "del" => "⌦".to_string(),
        "escape" | "esc" => "⎋".to_string(),
        "tab" => "⇥".to_string(),
        "space" => "Space".to_string(),
        "up" => "↑".to_string(),
        "down" => "↓".to_string(),
        "left" => "←".to_string(),
        "right" => "→".to_string(),
        "home" => "↖".to_string(),
        "end" => "↘".to_string(),
        "pageup" | "page_up" => "⇞".to_string(),
        "pagedown" | "page_down" => "⇟".to_string(),
        "period" | "numpaddecimal" => ".".to_string(),
        "comma" => ",".to_string(),
        "slash" | "numpaddivide" => "/".to_string(),
        "backslash" => "\\".to_string(),
        "semicolon" => ";".to_string(),
        "quote" | "apostrophe" => "'".to_string(),
        "backquote" | "grave" | "graveaccent" | "backtick" => "`".to_string(),
        "minus" | "numpadsubtract" => "-".to_string(),
        "equals" | "equal" => "=".to_string(),
        "plus" | "numpadadd" => "+".to_string(),
        "leftbracket" | "openbracket" => "[".to_string(),
        "rightbracket" | "closebracket" => "]".to_string(),
        _ => {
            let k = k
                .strip_prefix("digit")
                .or_else(|| k.strip_prefix("numpad"))
                .unwrap_or(&k);
            k.to_uppercase()
        }
    };
    out + &name
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
        let parsed = std::panic::catch_unwind(|| self::bindings(&r.keystroke, &r.action));
        match parsed {
            Ok(b) => bindings.extend(b),
            Err(_) => eprintln!("weep: cannot bind {:?} ({})", r.chord, r.action),
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
    fn chords_per_platform() {
        assert_eq!(
            presentable_on("ctrl-shift-f", Platform::Windows),
            "Ctrl+Shift+F"
        );
        assert_eq!(
            presentable_on("ctrl-shift-g ctrl-shift-a", Platform::Linux),
            "Ctrl+Shift+G, Ctrl+Shift+A"
        );
        assert_eq!(presentable_on("alt-digit1", Platform::Windows), "Alt+1");
        assert_eq!(presentable_on("cmd-n", Platform::Mac), "⌘N");
        assert_eq!(presentable_on("shift-cmd-f", Platform::Mac), "⇧⌘F");
        assert_eq!(
            presentable_on("cmd-alt-ctrl-shift-l", Platform::Mac),
            "⌃⌥⇧⌘L"
        );
        assert_eq!(presentable_on("cmd-comma", Platform::Mac), "⌘,");
        assert_eq!(presentable_on("cmd-digit1", Platform::Mac), "⌘1");
        assert_eq!(presentable_on("alt-up", Platform::Mac), "⌥↑");
        assert_eq!(presentable_on("cmd-minus", Platform::Mac), "⌘-");
        assert_eq!(presentable_on("cmd--", Platform::Mac), "⌘-");
        assert_eq!(presentable_on("f4", Platform::Mac), "F4");
        assert_eq!(presentable_on("cmd-k cmd-s", Platform::Mac), "⌘K, ⌘S");
    }

    #[test]
    fn table_is_consistent() {
        for id in ACTION_IDS {
            assert!(action_for(id).is_some(), "{id}");
        }
        assert!(is_editor_action("copy"));
        assert!(!is_editor_action("save-file"));
        // M8: one id in two contexts binds twice.
        assert_eq!(bindings("f9", "debug/toggle-enabled-breakpoint").len(), 2);
        assert!(is_editor_action("debug/toggle-breakpoint"));
        assert!(bindings("f9", "debug/drop-frame").is_empty());
    }
}
