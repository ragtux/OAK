//! A language server client, free of gpui: what Fleet's Smart Mode is when
//! the IntelliJ backend is replaced by a server speaking the Language Server
//! Protocol (`docs/roadmap.md`, M7). The app owns one [`Client`] per server
//! (rust-analyzer first), feeds it the open documents and asks it for
//! completions, hovers, definitions, references, renames, formatting, code
//! actions and semantic tokens; the answers arrive through [`Client::poll`]
//! as [`Event`]s, like a run configuration's output does in `oak-run`.
//!
//! - `transport`: JSON-RPC 2.0 framed with `Content-Length` over the
//!   server's pipes, on two threads.
//! - `client`: the handshake, the request ids, the server's own requests
//!   (progress, configuration, `workspace/applyEdit`), shutdown.
//! - `position`: LSP positions (UTF-8, UTF-16 or UTF-32 columns, as the
//!   server negotiated) against Oak's char offsets.
//! - `edits`: text edits and workspace edits as char-range replacements.
//! - `snippet`: an LSP snippet as plain text with the first tabstop.
//! - `semantic`: semantic tokens decoded and mapped onto Fleet's
//!   `textAttributes` keys.
//! - `servers`: which server runs for which language, and where it is.
//! - `markup`: Markdown from hovers and documentation as plain lines.

pub mod client;
pub mod edits;
pub mod markup;
pub mod position;
pub mod semantic;
pub mod servers;
pub mod snippet;
pub mod transport;

pub use client::{Client, Event, MessageLevel, RequestId, ResponseError};
pub use edits::{apply, changes, workspace_edit_files, FileEdits, TextChange};
pub use lsp_types;
pub use position::{column_from_lsp, column_to_lsp, Encoding};
pub use servers::{language_id, ServerCommand};

use std::path::{Path, PathBuf};

/// The `file:` URI of an absolute path (a relative one is made absolute
/// against the current directory, as a server would not know it).
pub fn uri(path: &Path) -> Option<lsp_types::Url> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    lsp_types::Url::from_file_path(absolute).ok()
}

/// The path of a `file:` URI, or `None` for any other scheme.
pub fn path(url: &lsp_types::Url) -> Option<PathBuf> {
    url.to_file_path().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_round_trip() {
        let dir = std::env::temp_dir()
            .join("oak-lsp uri test")
            .join("main.rs");
        let url = uri(&dir).expect("a file uri");
        assert!(url.as_str().starts_with("file://"));
        assert_eq!(path(&url).as_deref(), Some(dir.as_path()));
    }
}
