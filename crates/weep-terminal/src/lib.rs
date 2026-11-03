//! WEEP's terminal, free of gpui: a shell on a PTY and the grid it draws
//! into, through `alacritty_terminal` (the parser, the grid, the scrollback
//! and the selection are Alacritty's). The app paints the grid from a
//! [`Snapshot`] and sends keys through [`keys::encode`].
//!
//! Fleet's settings are the API's vocabulary: `terminal.profiles` rows are
//! [`Profile`]s (Fleet's `Profile(name, program, args)`), `terminal.lineCapacity`
//! is the scrollback and `terminal.caretShape` the default cursor.

pub mod colors;
pub mod keys;
pub mod profile;
mod terminal;

pub use profile::Profile;
pub use terminal::{
    Cell, CellFlags, Color, Cursor, CursorShape, Modes, SelectionKind, Size, Snapshot, Terminal,
};
