//! Every action Oak binds, named after the Fleet action it stands in for
//! (see `assets/keymap/default.json` and `keymap.rs` for the mapping).

use gpui::actions;

actions!(
    workspace,
    [
        Quit,
        Save,
        SaveAll,
        CloseTab,
        CloseWindow,
        NextTab,
        PrevTab,
        ToggleLeftPanel,
        ToggleBottomPanel,
        ToggleRightPanel,
        HideAllPanels,
        FocusEditor,
        NewFile,
    ]
);

actions!(
    editor,
    [
        MoveUp,
        MoveDown,
        MoveLeft,
        MoveRight,
        SelectUp,
        SelectDown,
        SelectLeft,
        SelectRight,
        Home,
        End,
        SelectHome,
        SelectEnd,
        PageUp,
        PageDown,
        SelectPageUp,
        SelectPageDown,
        DocStart,
        DocEnd,
        SelectDocStart,
        SelectDocEnd,
        WordLeft,
        WordRight,
        SelectWordLeft,
        SelectWordRight,
        Backspace,
        Delete,
        DeleteWordBackward,
        DeleteWordForward,
        DeleteLine,
        DuplicateLine,
        MoveLineUp,
        MoveLineDown,
        Newline,
        NewlineBelow,
        NewlineAbove,
        Tab,
        Outdent,
        Undo,
        Redo,
        Copy,
        Cut,
        Paste,
        SelectAll,
        CommentLine,
        JoinLines,
        Cancel,
    ]
);

actions!(files, [TreeUp, TreeDown, TreeLeft, TreeRight, TreeOpen]);
