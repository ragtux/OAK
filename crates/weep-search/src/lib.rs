//! Searching a workspace, kept free of gpui so it can be tested on its own.
//!
//! - [`fuzzy`]: the matcher behind the Goto panel's lists (files, symbols,
//!   actions, tools): a subsequence match scored the way Fleet's Goto ranks
//!   its rows, as far as the probes can tell (`docs/fleet-probe.md`, batch 6).
//! - [`files`]: the workspace's file list for `goto/file`, walked with the
//!   `ignore` crate so `.gitignore` and `files.exclude` apply as in Fleet.
//! - [`grep`]: `find-in-path` and `replace-in-path` over the same walk, with
//!   the Search tool's toggles (match case, words, regex), its file mask and
//!   path scope, and its result limit (Fleet stores 100).

pub mod files;
pub mod fuzzy;
pub mod grep;

pub use files::FileIndex;
pub use fuzzy::{fuzzy_match, FuzzyMatch};
pub use grep::{FileMatches, GrepOptions, GrepQuery, LineMatch};
