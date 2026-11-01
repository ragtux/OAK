//! The editor's text model, kept free of any UI dependency so it can be
//! tested on its own: a rope buffer ([`Buffer`]), the cursor and selection,
//! undo grouping, and every edit command the editor exposes
//! ([`TextEditor`]). Positions are char offsets into the rope; the UI turns
//! them into byte offsets per line when it shapes text.

mod buffer;
mod editor;

pub use buffer::{Buffer, Point};
pub use editor::{Selection, TextEditor};

/// Spaces per tab stop, the Fleet default ("Indent: 4 spaces").
pub const TAB_WIDTH: usize = 4;
