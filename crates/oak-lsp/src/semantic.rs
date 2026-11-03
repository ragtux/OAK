//! Semantic tokens (`textDocument/semanticTokens/full`) decoded from the
//! protocol's relative encoding and mapped onto Fleet's `textAttributes`
//! keys, which is what `editor.semanticHighlighting` means in Oak: the
//! server's token colours a span the way Fleet's backend would, over the
//! tree-sitter colour it had.

use lsp_types::{SemanticToken, SemanticTokensLegend};

/// One token: the row, the start column and the length in the server's
/// position encoding, with the legend's names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub line: u32,
    pub start: u32,
    pub len: u32,
    pub kind: String,
    pub modifiers: Vec<String>,
}

/// Resolves the deltas and the legend.
pub fn decode(data: &[SemanticToken], legend: &SemanticTokensLegend) -> Vec<Token> {
    let mut out = Vec::with_capacity(data.len());
    let mut line = 0u32;
    let mut start = 0u32;
    for t in data {
        if t.delta_line > 0 {
            line += t.delta_line;
            start = t.delta_start;
        } else {
            start += t.delta_start;
        }
        let kind = legend
            .token_types
            .get(t.token_type as usize)
            .map(|k| k.as_str().to_string())
            .unwrap_or_default();
        let modifiers = legend
            .token_modifiers
            .iter()
            .enumerate()
            .filter(|(bit, _)| t.token_modifiers_bitset & (1 << bit) != 0)
            .map(|(_, m)| m.as_str().to_string())
            .collect();
        out.push(Token {
            line,
            start,
            len: t.length,
            kind,
            modifiers,
        });
    }
    out
}

/// The Fleet attribute key a token paints as, or `None` to leave the
/// tree-sitter colour (an unresolved reference, a token Fleet has no key
/// for). The names are rust-analyzer's (the standard set plus its own),
/// the keys Fleet's theme files' (`tools/fleet-probe/out/theme-text-attributes.txt`).
pub fn fleet_attribute(kind: &str, modifiers: &[String]) -> Option<&'static str> {
    let has = |m: &str| modifiers.iter().any(|x| x == m);
    let declared = has("declaration") || has("definition");
    Some(match kind {
        "namespace" => "identifier",
        "struct" | "union" => "identifier.type.struct",
        "enum" => "identifier.type.enum",
        "class" => "identifier.type.class",
        "type" | "typeAlias" | "builtinType" => "identifier.type",
        "interface" => "identifier.interface",
        "typeParameter" => "identifier.typeParameter",
        "enumMember" | "const" => "identifier.constant",
        "parameter" => "identifier.parameter",
        "variable" => {
            if has("constant") || has("static") {
                "identifier.constant"
            } else if has("mutable") {
                "identifier.variable.mutable"
            } else {
                "identifier.variable"
            }
        }
        "property" => {
            if has("static") {
                "identifier.field.static"
            } else {
                "identifier.field"
            }
        }
        "function" | "procMacro" => {
            if declared {
                "identifier.function.declaration"
            } else {
                "identifier.function.call"
            }
        }
        "method" => {
            if has("static") {
                "identifier.method.static"
            } else if declared {
                "identifier.function.declaration"
            } else {
                "identifier.function.call"
            }
        }
        "macro" => "identifier.function.call",
        "keyword" => "keyword",
        "selfKeyword" | "selfTypeKeyword" => "identifier.this",
        "attribute" | "attributeBracket" | "builtinAttribute" | "decorator" | "derive"
        | "deriveHelper" | "toolModule" => "metadata",
        "lifetime" | "label" => "identifier.constant",
        "comment" => {
            if has("documentation") {
                "comment.doc"
            } else {
                "comment"
            }
        }
        "string" | "character" => "string",
        "escapeSequence" => "string.escape",
        "formatSpecifier" => "string.formatItem",
        "number" => "number",
        "boolean" => "boolean",
        "operator" | "arithmetic" | "bitwise" | "comparison" | "logical" => "punctuation.operator",
        "punctuation" | "bracket" | "brace" | "parenthesis" | "angle" | "comma" | "colon"
        | "semicolon" | "dot" => "punctuation",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::{SemanticTokenModifier, SemanticTokenType};

    fn legend() -> SemanticTokensLegend {
        SemanticTokensLegend {
            token_types: vec![
                SemanticTokenType::FUNCTION,
                SemanticTokenType::VARIABLE,
                SemanticTokenType::KEYWORD,
            ],
            token_modifiers: vec![
                SemanticTokenModifier::DECLARATION,
                SemanticTokenModifier::new("mutable"),
            ],
        }
    }

    #[test]
    fn deltas_resolve_to_rows_and_columns() {
        let data = vec![
            SemanticToken {
                delta_line: 0,
                delta_start: 3,
                length: 4,
                token_type: 0,
                token_modifiers_bitset: 1,
            },
            SemanticToken {
                delta_line: 0,
                delta_start: 7,
                length: 1,
                token_type: 1,
                token_modifiers_bitset: 2,
            },
            SemanticToken {
                delta_line: 2,
                delta_start: 4,
                length: 3,
                token_type: 2,
                token_modifiers_bitset: 0,
            },
        ];
        let tokens = decode(&data, &legend());
        assert_eq!(
            tokens,
            vec![
                Token {
                    line: 0,
                    start: 3,
                    len: 4,
                    kind: "function".into(),
                    modifiers: vec!["declaration".into()]
                },
                Token {
                    line: 0,
                    start: 10,
                    len: 1,
                    kind: "variable".into(),
                    modifiers: vec!["mutable".into()]
                },
                Token {
                    line: 2,
                    start: 4,
                    len: 3,
                    kind: "keyword".into(),
                    modifiers: vec![]
                },
            ]
        );
    }

    #[test]
    fn fleet_keys() {
        let m = |s: &str| vec![s.to_string()];
        assert_eq!(
            fleet_attribute("function", &[]),
            Some("identifier.function.call")
        );
        assert_eq!(
            fleet_attribute("function", &m("declaration")),
            Some("identifier.function.declaration")
        );
        assert_eq!(
            fleet_attribute("variable", &m("mutable")),
            Some("identifier.variable.mutable")
        );
        assert_eq!(
            fleet_attribute("variable", &m("constant")),
            Some("identifier.constant")
        );
        assert_eq!(
            fleet_attribute("struct", &[]),
            Some("identifier.type.struct")
        );
        assert_eq!(
            fleet_attribute("comment", &m("documentation")),
            Some("comment.doc")
        );
        assert_eq!(fleet_attribute("unresolvedReference", &[]), None);
    }
}
