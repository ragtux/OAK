//! The symbols a file declares, for `goto/symbol` and `show-file-symbols`:
//! read off the tree-sitter tree without a language server. Rust items,
//! JSON keys, TOML tables and keys, Markdown headings. Fleet's own list
//! comes from its backend; which rows it shows for these four languages
//! without Smart Mode is not observable, so this is the stated set. Fleet's
//! Symbols in File list shows each item's signature (`new(id: u32, name:
//! &str) -> Self`, `id: u32`), which `signature` carries.

use crate::structure::SyntaxTree;
use crate::Language;
use std::ops::Range;
use tree_sitter::Node;

/// What kind of thing a symbol is; names follow Fleet's `icons.nodes.*`
/// (`class`, `function`, `field`, ...) so the row's icon follows directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Function,
    Method,
    Struct,
    Enum,
    Trait,
    Impl,
    Module,
    Constant,
    Static,
    Type,
    Macro,
    Field,
    Variant,
    Key,
    Table,
    Heading,
}

impl SymbolKind {
    /// The expui icon under `assets/icons/expui/nodes` that stands for it.
    pub fn icon(self) -> &'static str {
        match self {
            SymbolKind::Function => "expui/nodes/function",
            SymbolKind::Method => "expui/nodes/method",
            SymbolKind::Struct => "expui/nodes/class",
            SymbolKind::Enum => "expui/nodes/enum",
            SymbolKind::Trait => "expui/nodes/interface",
            SymbolKind::Impl => "expui/nodes/class",
            SymbolKind::Module => "expui/nodes/module",
            SymbolKind::Constant | SymbolKind::Static => "expui/nodes/constant",
            SymbolKind::Type => "expui/nodes/class",
            SymbolKind::Macro => "expui/nodes/function",
            SymbolKind::Field => "expui/nodes/field",
            SymbolKind::Variant => "expui/nodes/constant",
            SymbolKind::Key => "expui/nodes/property",
            SymbolKind::Table => "expui/nodes/package",
            SymbolKind::Heading => "expui/nodes/tag",
        }
    }

    /// Whether items of this kind hold members (the outline draws a chevron).
    pub fn is_container(self) -> bool {
        matches!(
            self,
            SymbolKind::Struct
                | SymbolKind::Enum
                | SymbolKind::Trait
                | SymbolKind::Impl
                | SymbolKind::Module
                | SymbolKind::Table
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    /// What the outline shows: the name with a field's type or a function's
    /// parameters and return type; the name alone for the rest.
    pub signature: String,
    pub kind: SymbolKind,
    /// The byte range of the name (what Goto selects), inside the item.
    pub range: Range<usize>,
    /// The 0-based row the item starts on.
    pub row: usize,
    /// Nesting depth, for an indented outline.
    pub depth: usize,
    /// For a method or field: the item it belongs to.
    pub container: Option<String>,
}

fn text_of(node: Node<'_>, text: &str) -> String {
    text.get(node.start_byte()..node.end_byte())
        .unwrap_or("")
        .to_string()
}

fn rust_kind(kind: &str) -> Option<SymbolKind> {
    Some(match kind {
        "function_item" | "function_signature_item" => SymbolKind::Function,
        "struct_item" | "union_item" => SymbolKind::Struct,
        "enum_item" => SymbolKind::Enum,
        "trait_item" => SymbolKind::Trait,
        "impl_item" => SymbolKind::Impl,
        "mod_item" => SymbolKind::Module,
        "const_item" => SymbolKind::Constant,
        "static_item" => SymbolKind::Static,
        "type_item" => SymbolKind::Type,
        "macro_definition" => SymbolKind::Macro,
        "field_declaration" => SymbolKind::Field,
        "enum_variant" => SymbolKind::Variant,
        _ => return None,
    })
}

#[allow(clippy::too_many_arguments)]
fn push_symbol(
    out: &mut Vec<Symbol>,
    name_node: Node<'_>,
    name: String,
    signature: Option<String>,
    kind: SymbolKind,
    item: Node<'_>,
    depth: usize,
    container: Option<&str>,
) {
    if name.is_empty() {
        return;
    }
    out.push(Symbol {
        signature: signature.unwrap_or_else(|| name.clone()),
        name,
        kind,
        range: name_node.start_byte()..name_node.end_byte(),
        row: item.start_position().row,
        depth,
        container: container.map(str::to_string),
    });
}

/// A function's `name(params) -> Ret` or a field's `name: Type`, as the
/// outline shows them; whitespace inside collapsed to single spaces.
fn rust_signature(item: Node<'_>, name: &str, kind: SymbolKind, text: &str) -> Option<String> {
    let squeeze = |s: String| s.split_whitespace().collect::<Vec<_>>().join(" ");
    match kind {
        SymbolKind::Function | SymbolKind::Method => {
            let params = item
                .child_by_field_name("parameters")
                .map(|p| squeeze(text_of(p, text)))
                .unwrap_or_else(|| "()".into());
            let ret = item
                .child_by_field_name("return_type")
                .map(|r| format!(" -> {}", squeeze(text_of(r, text))))
                .unwrap_or_default();
            Some(format!("{name}{params}{ret}"))
        }
        SymbolKind::Field => item
            .child_by_field_name("type")
            .map(|t| format!("{name}: {}", squeeze(text_of(t, text)))),
        SymbolKind::Constant | SymbolKind::Static => item
            .child_by_field_name("type")
            .map(|t| format!("{name}: {}", squeeze(text_of(t, text)))),
        _ => None,
    }
}

fn walk_rust(
    node: Node<'_>,
    text: &str,
    depth: usize,
    container: Option<&str>,
    out: &mut Vec<Symbol>,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let Some(kind) = rust_kind(child.kind()) else {
            // Declaration lists and bodies carry the items; descend through them.
            if matches!(
                child.kind(),
                "declaration_list" | "field_declaration_list" | "enum_variant_list" | "source_file"
            ) {
                walk_rust(child, text, depth, container, out);
            }
            continue;
        };
        let (name_node, name) = match kind {
            SymbolKind::Impl => {
                let ty = child.child_by_field_name("type");
                let tr = child.child_by_field_name("trait");
                match (ty, tr) {
                    (Some(ty), Some(tr)) => (
                        ty,
                        format!("impl {} for {}", text_of(tr, text), text_of(ty, text)),
                    ),
                    (Some(ty), None) => (ty, format!("impl {}", text_of(ty, text))),
                    _ => continue,
                }
            }
            _ => match child.child_by_field_name("name") {
                Some(n) => (n, text_of(n, text)),
                None => continue,
            },
        };
        let kind = if kind == SymbolKind::Function && container.is_some() {
            SymbolKind::Method
        } else {
            kind
        };
        // Fleet's outline names an impl block by its type alone.
        let shown = match kind {
            SymbolKind::Impl => child
                .child_by_field_name("type")
                .map(|t| text_of(t, text))
                .unwrap_or_else(|| name.clone()),
            _ => name.clone(),
        };
        let signature = rust_signature(child, &shown, kind, text).or(Some(shown.clone()));
        push_symbol(
            out,
            name_node,
            name.clone(),
            signature,
            kind,
            child,
            depth,
            container,
        );
        // Members of the item.
        let inner_container = match kind {
            SymbolKind::Impl => shown,
            _ => name,
        };
        if matches!(
            kind,
            SymbolKind::Impl
                | SymbolKind::Trait
                | SymbolKind::Module
                | SymbolKind::Struct
                | SymbolKind::Enum
        ) {
            walk_rust(child, text, depth + 1, Some(&inner_container), out);
        }
    }
}

fn walk_json(node: Node<'_>, text: &str, depth: usize, out: &mut Vec<Symbol>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "pair" => {
                if let Some(key) = child.child_by_field_name("key") {
                    let name = text_of(key, text).trim_matches('"').to_string();
                    push_symbol(out, key, name, None, SymbolKind::Key, child, depth, None);
                    if let Some(value) = child.child_by_field_name("value") {
                        if value.kind() == "object" && depth < 2 {
                            walk_json(value, text, depth + 1, out);
                        }
                    }
                }
            }
            "object" | "document" | "array" if depth == 0 => walk_json(child, text, depth, out),
            _ => {}
        }
    }
}

fn walk_toml(node: Node<'_>, text: &str, out: &mut Vec<Symbol>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "table" | "table_array_element" => {
                // The header's key is the first named child that is not a pair.
                let mut c2 = child.walk();
                let header = child
                    .named_children(&mut c2)
                    .find(|n| n.kind() != "pair" && n.kind() != "comment");
                if let Some(h) = header {
                    push_symbol(
                        out,
                        h,
                        text_of(h, text),
                        None,
                        SymbolKind::Table,
                        child,
                        0,
                        None,
                    );
                }
                let container = header.map(|h| text_of(h, text));
                let mut c3 = child.walk();
                for pair in child.named_children(&mut c3).filter(|n| n.kind() == "pair") {
                    if let Some(key) = pair.named_child(0) {
                        push_symbol(
                            out,
                            key,
                            text_of(key, text),
                            None,
                            SymbolKind::Key,
                            pair,
                            1,
                            container.as_deref(),
                        );
                    }
                }
            }
            "pair" => {
                if let Some(key) = child.named_child(0) {
                    push_symbol(
                        out,
                        key,
                        text_of(key, text),
                        None,
                        SymbolKind::Key,
                        child,
                        0,
                        None,
                    );
                }
            }
            "document" => walk_toml(child, text, out),
            _ => {}
        }
    }
}

fn walk_markdown(node: Node<'_>, text: &str, out: &mut Vec<Symbol>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "atx_heading" | "setext_heading" => {
                let line = text
                    .get(child.start_byte()..child.end_byte())
                    .unwrap_or("")
                    .lines()
                    .next()
                    .unwrap_or("");
                let level = line.chars().take_while(|c| *c == '#').count();
                let name = line.trim_start_matches('#').trim().to_string();
                let depth = if child.kind() == "setext_heading" {
                    0
                } else {
                    level.saturating_sub(1)
                };
                let start = child.start_byte()
                    + (line.len() - line.trim_start_matches('#').trim_start().len());
                if !name.is_empty() {
                    out.push(Symbol {
                        signature: name.clone(),
                        name,
                        kind: SymbolKind::Heading,
                        range: start..start + line.trim_start_matches('#').trim().len(),
                        row: child.start_position().row,
                        depth,
                        container: None,
                    });
                }
            }
            "section" | "document" => walk_markdown(child, text, out),
            _ => {}
        }
    }
}

impl SyntaxTree {
    /// The symbols the file declares, in source order.
    pub fn symbols(&self, text: &str) -> Vec<Symbol> {
        let root = self.tree().root_node();
        let mut out = Vec::new();
        match self.language() {
            Language::Rust => walk_rust(root, text, 0, None, &mut out),
            Language::Json => walk_json(root, text, 0, &mut out),
            Language::Toml => walk_toml(root, text, &mut out),
            Language::Markdown => walk_markdown(root, text, &mut out),
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(language: Language, text: &str) -> Vec<(String, SymbolKind, usize)> {
        SyntaxTree::parse(language, text)
            .unwrap()
            .symbols(text)
            .into_iter()
            .map(|s| (s.name, s.kind, s.depth))
            .collect()
    }

    #[test]
    fn rust_items() {
        let text = "pub struct Record { id: u32 }\nimpl Record {\n    fn new() -> Self { todo!() }\n}\nenum E { A, B }\nconst N: u32 = 1;\nmod m { fn inner() {} }\n";
        let got = names(Language::Rust, text);
        assert_eq!(got[0], ("Record".into(), SymbolKind::Struct, 0));
        assert_eq!(got[1], ("id".into(), SymbolKind::Field, 1));
        assert_eq!(got[2], ("impl Record".into(), SymbolKind::Impl, 0));
        assert_eq!(got[3], ("new".into(), SymbolKind::Method, 1));
        assert_eq!(got[4], ("E".into(), SymbolKind::Enum, 0));
        assert_eq!(got[5], ("A".into(), SymbolKind::Variant, 1));
        assert!(got.contains(&("N".into(), SymbolKind::Constant, 0)));
        assert!(got.contains(&("inner".into(), SymbolKind::Method, 1)));
        let symbols = SyntaxTree::parse(Language::Rust, text)
            .unwrap()
            .symbols(text);
        let s = &symbols[3];
        assert_eq!(s.container.as_deref(), Some("Record"));
        assert_eq!(&text[s.range.clone()], "new");
        assert_eq!(s.signature, "new() -> Self");
        assert_eq!(symbols[1].signature, "id: u32");
        assert_eq!(symbols[2].signature, "Record");
        let n = symbols.iter().find(|s| s.name == "N").unwrap();
        assert_eq!(n.signature, "N: u32");
    }

    #[test]
    fn json_toml_markdown() {
        let got = names(Language::Json, "{\"a\": 1, \"b\": {\"c\": 2}}");
        assert_eq!(got[0].0, "a");
        assert_eq!(got[1].0, "b");
        assert_eq!(got[2], ("c".into(), SymbolKind::Key, 1));
        let got = names(
            Language::Toml,
            "top = 1\n[package]\nname = \"x\"\n[[bin]]\npath = \"a\"\n",
        );
        assert_eq!(got[0], ("top".into(), SymbolKind::Key, 0));
        assert_eq!(got[1], ("package".into(), SymbolKind::Table, 0));
        assert_eq!(got[2], ("name".into(), SymbolKind::Key, 1));
        assert_eq!(got[3].0, "bin");
        let got = names(Language::Markdown, "# Title\n\ntext\n\n## Part\n");
        assert_eq!(got[0], ("Title".into(), SymbolKind::Heading, 0));
        assert_eq!(got[1], ("Part".into(), SymbolKind::Heading, 1));
    }
}
