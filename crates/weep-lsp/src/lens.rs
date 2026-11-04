//! Code lenses as Fleet's code vision (`editor.showCodeVision`): the short
//! rows above a declaration ("4 implementations" over a struct, fifteenth
//! pass). Fleet shows a server's runnables as the gutter's run triangle, not
//! as text, so a lens whose command runs or debugs is left out here; the
//! others are drawn by their command's title and, clicked, list the
//! locations their command carries.

use lsp_types::{CodeLens, Command, Location};

/// Whether a lens stands for a runnable (rust-analyzer's `▶︎ Run`,
/// `Debug`, `▶︎ Run Tests`): the run icons cover those.
pub fn is_runnable(command: &Command) -> bool {
    let id = command.command.as_str();
    id.ends_with("runSingle")
        || id.ends_with("debugSingle")
        || id.ends_with(".run")
        || id.ends_with(".debug")
        || command.title.starts_with('▶')
}

/// Whether a lens is a code-vision row's: a resolved one is unless it
/// runs; an unresolved one (no command yet) is kept until it resolves.
pub fn shows(lens: &CodeLens) -> bool {
    lens.command.as_ref().is_none_or(|c| !is_runnable(c))
}

/// The locations a lens's command lists: the `showReferences` shape most
/// servers use (rust-analyzer's implementations and references lenses,
/// VS Code's `editor.action.showReferences`): the document, the position,
/// then the locations.
pub fn locations(command: &Command) -> Vec<Location> {
    let Some(args) = &command.arguments else {
        return Vec::new();
    };
    args.iter()
        .rev()
        .find_map(|a| serde_json::from_value::<Vec<Location>>(a.clone()).ok())
        .unwrap_or_default()
}

/// The word a lens's list is headed with: its title without the count,
/// capitalised and plural ("4 implementations" and "1 implementation" are
/// both "Implementations").
pub fn heading(title: &str) -> String {
    let words = title
        .trim_start_matches(|c: char| c.is_ascii_digit())
        .trim();
    let mut out: String = words.chars().take(1).flat_map(char::to_uppercase).collect();
    out.extend(words.chars().skip(1));
    if !out.is_empty() && !out.ends_with('s') {
        out.push('s');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn command(id: &str, title: &str, arguments: Option<Vec<serde_json::Value>>) -> Command {
        Command {
            title: title.into(),
            command: id.into(),
            arguments,
        }
    }

    #[test]
    fn runnables_are_left_to_the_gutter() {
        assert!(is_runnable(&command(
            "rust-analyzer.runSingle",
            "▶︎ Run",
            None
        )));
        assert!(is_runnable(&command(
            "rust-analyzer.debugSingle",
            "Debug",
            None
        )));
        assert!(!is_runnable(&command(
            "rust-analyzer.showReferences",
            "4 implementations",
            None
        )));
        let unresolved = CodeLens {
            range: Default::default(),
            command: None,
            data: Some(json!({"impls": 1})),
        };
        assert!(shows(&unresolved));
    }

    #[test]
    fn headings_drop_the_count() {
        assert_eq!(heading("4 implementations"), "Implementations");
        assert_eq!(heading("1 implementation"), "Implementations");
        assert_eq!(heading("12 references"), "References");
    }

    #[test]
    fn locations_come_from_the_last_list_argument() {
        let loc = json!({
            "uri": "file:///tmp/main.rs",
            "range": {"start": {"line": 12, "character": 0}, "end": {"line": 12, "character": 11}}
        });
        let c = command(
            "rust-analyzer.showReferences",
            "1 implementation",
            Some(vec![
                json!("file:///tmp/main.rs"),
                json!({"line": 6, "character": 11}),
                json!([loc]),
            ]),
        );
        let found = locations(&c);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].range.start.line, 12);
        assert!(locations(&command("x", "y", None)).is_empty());
    }
}
