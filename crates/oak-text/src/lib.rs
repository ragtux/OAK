//! The editor's text model, kept free of any UI dependency so it can be
//! tested on its own: a rope buffer ([`Buffer`]) that several views may
//! share, and per view a [`Cursor`] holding the selection and the typing
//! group, with every edit command the editor exposes. [`TextEditor`] pairs
//! one buffer with one cursor for the single-view case and the tests.
//! Positions are char offsets into the rope; the UI turns them into byte
//! offsets per line when it shapes text.

pub mod analysis;
mod buffer;
mod editor;
pub mod indent;
pub mod search;

pub use buffer::{Buffer, Edit, Point};
pub use editor::{Cursor, Selection, TextEditor};
pub use indent::Indent;

/// Spaces per tab stop, the Fleet default ("Indent: 4 spaces").
pub const TAB_WIDTH: usize = 4;
