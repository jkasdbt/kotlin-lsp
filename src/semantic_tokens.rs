//! Semantic token classification for `textDocument/semanticTokens/full` and
//! `textDocument/semanticTokens/range`.
//!
//! Tokens are derived purely from the tree-sitter CST — no cross-file type
//! resolution.  This is sufficient to distinguish classes from functions,
//! parameters from properties, `val` (readonly) from `var`, etc.
//!
//! # Encoding
//! LSP semantic tokens are delta-encoded: each token stores the line *delta*
//! from the previous token and the column *delta* (from the previous token on
//! the same line, or from column 0 on a new line).  Tokens must be sorted by
//! (line, col) before encoding.

use tower_lsp::lsp_types::{
    Range, SemanticToken, SemanticTokenModifier, SemanticTokenType, SemanticTokens,
    SemanticTokensLegend,
};
use tree_sitter::Node;

use crate::indexer::LiveDoc;
use crate::queries::{
    KIND_ANNOTATION_TYPE_DECL, KIND_CLASS_DECL, KIND_COMPANION_OBJ, KIND_ENUM_CONSTANT,
    KIND_FIELD_DECL, KIND_FUN_DECL, KIND_IDENTIFIER, KIND_INTERFACE_DECL, KIND_METHOD_DECL,
    KIND_MODIFIERS, KIND_MULTI_VAR_DECL, KIND_OBJECT_DECL, KIND_PROP_DECL, KIND_RECORD_DECL,
    KIND_SIMPLE_IDENT, KIND_TYPE_IDENT, KIND_TYPE_PARAM, KIND_VAR_DECL,
};
use crate::Language;

// ─── Legend ──────────────────────────────────────────────────────────────────

/// Ordered list of token types — index == LSP token type id.
pub(crate) const TOKEN_TYPES: &[SemanticTokenType] = &[
    SemanticTokenType::CLASS,          // 0
    SemanticTokenType::INTERFACE,      // 1
    SemanticTokenType::ENUM,           // 2
    SemanticTokenType::TYPE_PARAMETER, // 3
    SemanticTokenType::FUNCTION,       // 4
    SemanticTokenType::METHOD,         // 5
    SemanticTokenType::PROPERTY,       // 6
    SemanticTokenType::VARIABLE,       // 7
    SemanticTokenType::PARAMETER,      // 8
    SemanticTokenType::ENUM_MEMBER,    // 9
    SemanticTokenType::DECORATOR,      // 10  (annotations)
    SemanticTokenType::NAMESPACE,      // 11  (objects / companion objects used as namespaces)
];

/// Ordered list of modifiers — bit position == modifier id.
pub(crate) const TOKEN_MODIFIERS: &[SemanticTokenModifier] = &[
    SemanticTokenModifier::DECLARATION, // bit 0
    SemanticTokenModifier::READONLY,    // bit 1
    SemanticTokenModifier::STATIC,      // bit 2  (companion object members)
    SemanticTokenModifier::ABSTRACT,    // bit 3
    SemanticTokenModifier::ASYNC,       // bit 4  (suspend funs)
    SemanticTokenModifier::DEPRECATED,  // bit 5
];

fn type_index(t: &SemanticTokenType) -> u32 {
    TOKEN_TYPES
        .iter()
        .position(|x| x == t)
        .expect("token type not in legend") as u32
}

fn modifier_bit(m: &SemanticTokenModifier) -> u32 {
    1 << TOKEN_MODIFIERS
        .iter()
        .position(|x| x == m)
        .expect("modifier not in legend") as u32
}

pub(crate) fn legend() -> SemanticTokensLegend {
    SemanticTokensLegend {
        token_types: TOKEN_TYPES.to_vec(),
        token_modifiers: TOKEN_MODIFIERS.to_vec(),
    }
}

// ─── Classification ───────────────────────────────────────────────────────────

#[derive(Clone)]
struct RawToken {
    line: u32,
    col: u32,  // UTF-16 column
    length: u32,
    token_type: u32,
    token_modifiers_bitset: u32,
}

/// Collect semantic tokens for `doc`, for the given `language`.
/// Returns delta-encoded `SemanticToken` values ready for the LSP response.
/// Filtered to `range` when `Some`.
pub(crate) fn collect_tokens(
    doc: &LiveDoc,
    language: Language,
    range: Option<&Range>,
) -> Vec<SemanticToken> {
    let mut raw: Vec<RawToken> = Vec::new();

    match language {
        Language::Kotlin => walk_kotlin(doc.tree.root_node(), &doc.bytes, &mut raw),
        Language::Java => walk_java(doc.tree.root_node(), &doc.bytes, &mut raw),
        _ => {}
    }

    // Sort by (line, col) — tree walk is depth-first so usually already sorted,
    // but not guaranteed for all node orderings.
    raw.sort_by_key(|t| (t.line, t.col));

    // Apply range filter before delta-encoding.
    if let Some(r) = range {
        let start_line = r.start.line;
        let end_line = r.end.line;
        raw.retain(|t| t.line >= start_line && t.line <= end_line);
    }

    delta_encode(raw)
}

// ─── Kotlin walker ────────────────────────────────────────────────────────────

fn walk_kotlin(node: Node<'_>, src: &[u8], out: &mut Vec<RawToken>) {
    classify_kotlin(node, src, out);
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            walk_kotlin(cursor.node(), src, out);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

fn classify_kotlin(node: Node<'_>, src: &[u8], out: &mut Vec<RawToken>) {
    let kind = node.kind();
    match kind {
        k if k == KIND_CLASS_DECL => kotlin_class_token(node, src, out),
        k if k == KIND_OBJECT_DECL => kotlin_object_token(node, src, out),
        k if k == KIND_COMPANION_OBJ => kotlin_companion_token(node, src, out),
        k if k == KIND_FUN_DECL => kotlin_fun_token(node, src, out),
        k if k == KIND_PROP_DECL => kotlin_prop_token(node, src, out),
        k if k == KIND_TYPE_PARAM => kotlin_type_param_token(node, src, out),
        "parameter" => {
            let mods = modifier_bit(&SemanticTokenModifier::DECLARATION);
            if let Some(name) = child_ident(node, src) {
                push_token(name, type_index(&SemanticTokenType::PARAMETER), mods, src, out);
            }
        }
        "enum_entry" => {
            let mods = modifier_bit(&SemanticTokenModifier::DECLARATION)
                | modifier_bit(&SemanticTokenModifier::READONLY);
            if let Some(name) = child_ident(node, src) {
                push_token(name, type_index(&SemanticTokenType::ENUM_MEMBER), mods, src, out);
            }
        }
        "annotation" | "multi_annotation" => {
            if let Some(ident) = first_child_of_kind(node, KIND_TYPE_IDENT)
                .or_else(|| first_child_of_kind(node, KIND_SIMPLE_IDENT))
            {
                push_token(ident, type_index(&SemanticTokenType::DECORATOR), 0, src, out);
            }
        }
        _ => {}
    }
}

fn kotlin_class_token(node: Node<'_>, src: &[u8], out: &mut Vec<RawToken>) {
    let token_type = if has_keyword_child(node, "interface") {
        type_index(&SemanticTokenType::INTERFACE)
    } else if has_keyword_child(node, "enum") {
        type_index(&SemanticTokenType::ENUM)
    } else {
        type_index(&SemanticTokenType::CLASS)
    };
    let mut mods = modifier_bit(&SemanticTokenModifier::DECLARATION);
    if has_modifier(node, src, "abstract") {
        mods |= modifier_bit(&SemanticTokenModifier::ABSTRACT);
    }
    if let Some(name) = child_ident(node, src) {
        push_token(name, token_type, mods, src, out);
    }
}

fn kotlin_object_token(node: Node<'_>, src: &[u8], out: &mut Vec<RawToken>) {
    let mods = modifier_bit(&SemanticTokenModifier::DECLARATION);
    if let Some(name) = child_ident(node, src) {
        push_token(name, type_index(&SemanticTokenType::NAMESPACE), mods, src, out);
    }
}

fn kotlin_companion_token(node: Node<'_>, src: &[u8], out: &mut Vec<RawToken>) {
    let mods = modifier_bit(&SemanticTokenModifier::DECLARATION)
        | modifier_bit(&SemanticTokenModifier::STATIC);
    let ns_type = type_index(&SemanticTokenType::NAMESPACE);
    if let Some(name) = child_ident(node, src) {
        push_token(name, ns_type, mods, src, out);
    } else if let Some(obj_kw) = first_child_of_kind(node, "object") {
        push_token(obj_kw, ns_type, mods, src, out);
    }
}

fn kotlin_fun_token(node: Node<'_>, src: &[u8], out: &mut Vec<RawToken>) {
    let token_type = if is_inside_class_body(node) {
        type_index(&SemanticTokenType::METHOD)
    } else {
        type_index(&SemanticTokenType::FUNCTION)
    };
    let mut mods = modifier_bit(&SemanticTokenModifier::DECLARATION);
    if has_modifier(node, src, "suspend") {
        mods |= modifier_bit(&SemanticTokenModifier::ASYNC);
    }
    if has_modifier(node, src, "abstract") {
        mods |= modifier_bit(&SemanticTokenModifier::ABSTRACT);
    }
    if let Some(name) = child_ident(node, src) {
        push_token(name, token_type, mods, src, out);
    }
}

fn kotlin_prop_token(node: Node<'_>, src: &[u8], out: &mut Vec<RawToken>) {
    let is_val = first_child_of_kind(node, "binding_pattern_kind")
        .map(|bpk| has_keyword_child(bpk, "val"))
        .unwrap_or_else(|| has_keyword_child(node, "val"));
    let token_type = if is_inside_class_body(node) {
        type_index(&SemanticTokenType::PROPERTY)
    } else {
        type_index(&SemanticTokenType::VARIABLE)
    };
    let mut mods = modifier_bit(&SemanticTokenModifier::DECLARATION);
    if is_val {
        mods |= modifier_bit(&SemanticTokenModifier::READONLY);
    }
    if let Some(var_decl) = first_child_of_kind(node, KIND_VAR_DECL) {
        if let Some(name) = child_ident(var_decl, src) {
            push_token(name, token_type, mods, src, out);
        }
    } else if let Some(multi) = first_child_of_kind(node, KIND_MULTI_VAR_DECL) {
        for i in 0..multi.named_child_count() {
            if let Some(vd) = multi.named_child(i) {
                if let Some(name) = child_ident(vd, src) {
                    push_token(name, token_type, mods, src, out);
                }
            }
        }
    }
}

fn kotlin_type_param_token(node: Node<'_>, src: &[u8], out: &mut Vec<RawToken>) {
    let mods = modifier_bit(&SemanticTokenModifier::DECLARATION);
    if let Some(ident) = first_child_of_kind(node, KIND_TYPE_IDENT)
        .or_else(|| first_child_of_kind(node, KIND_SIMPLE_IDENT))
    {
        push_token(ident, type_index(&SemanticTokenType::TYPE_PARAMETER), mods, src, out);
    }
}

// ─── Java walker ─────────────────────────────────────────────────────────────

fn walk_java(node: Node<'_>, src: &[u8], out: &mut Vec<RawToken>) {
    classify_java(node, src, out);
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            walk_java(cursor.node(), src, out);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

fn classify_java(node: Node<'_>, src: &[u8], out: &mut Vec<RawToken>) {
    let kind = node.kind();

    match kind {
        k if k == KIND_CLASS_DECL || k == KIND_RECORD_DECL => {
            let mut mods = modifier_bit(&SemanticTokenModifier::DECLARATION);
            if has_java_modifier(node, src, "abstract") {
                mods |= modifier_bit(&SemanticTokenModifier::ABSTRACT);
            }
            if let Some(name) = first_child_of_kind(node, KIND_IDENTIFIER) {
                push_token(name, type_index(&SemanticTokenType::CLASS), mods, src, out);
            }
        }

        k if k == KIND_INTERFACE_DECL || k == KIND_ANNOTATION_TYPE_DECL => {
            let token_type = if k == KIND_ANNOTATION_TYPE_DECL {
                type_index(&SemanticTokenType::DECORATOR)
            } else {
                type_index(&SemanticTokenType::INTERFACE)
            };
            let mods = modifier_bit(&SemanticTokenModifier::DECLARATION);
            if let Some(name) = first_child_of_kind(node, KIND_IDENTIFIER) {
                push_token(name, token_type, mods, src, out);
            }
        }

        "enum_declaration" => {
            let mods = modifier_bit(&SemanticTokenModifier::DECLARATION);
            if let Some(name) = first_child_of_kind(node, KIND_IDENTIFIER) {
                push_token(name, type_index(&SemanticTokenType::ENUM), mods, src, out);
            }
        }

        k if k == KIND_METHOD_DECL => {
            let mut mods = modifier_bit(&SemanticTokenModifier::DECLARATION);
            if has_java_modifier(node, src, "abstract") {
                mods |= modifier_bit(&SemanticTokenModifier::ABSTRACT);
            }
            if let Some(name) = first_child_of_kind(node, KIND_IDENTIFIER) {
                push_token(name, type_index(&SemanticTokenType::METHOD), mods, src, out);
            }
        }

        k if k == KIND_FIELD_DECL => {
            let is_final = has_java_modifier(node, src, "final");
            let mut mods = modifier_bit(&SemanticTokenModifier::DECLARATION);
            if is_final {
                mods |= modifier_bit(&SemanticTokenModifier::READONLY);
            }
            if has_java_modifier(node, src, "static") {
                mods |= modifier_bit(&SemanticTokenModifier::STATIC);
            }
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "variable_declarator" {
                        if let Some(name) = first_child_of_kind(child, KIND_IDENTIFIER) {
                            push_token(name, type_index(&SemanticTokenType::PROPERTY), mods, src, out);
                        }
                    }
                }
            }
        }

        "formal_parameter" | "spread_parameter" => {
            let mods = modifier_bit(&SemanticTokenModifier::DECLARATION);
            if let Some(name) = first_child_of_kind(node, KIND_IDENTIFIER) {
                push_token(name, type_index(&SemanticTokenType::PARAMETER), mods, src, out);
            }
        }

        k if k == KIND_ENUM_CONSTANT => {
            let mods = modifier_bit(&SemanticTokenModifier::DECLARATION)
                | modifier_bit(&SemanticTokenModifier::READONLY);
            if let Some(name) = first_child_of_kind(node, KIND_IDENTIFIER) {
                push_token(name, type_index(&SemanticTokenType::ENUM_MEMBER), mods, src, out);
            }
        }

        k if k == KIND_TYPE_PARAM => {
            let mods = modifier_bit(&SemanticTokenModifier::DECLARATION);
            // Java type_parameter has type_identifier child; Kotlin uses type_identifier too
            if let Some(name) = first_child_of_kind(node, KIND_TYPE_IDENT)
                .or_else(|| first_child_of_kind(node, KIND_IDENTIFIER))
            {
                push_token(name, type_index(&SemanticTokenType::TYPE_PARAMETER), mods, src, out);
            }
        }

        "marker_annotation" | "annotation" => {
            // @Override, @SuppressWarnings("...") etc.
            if let Some(name) = first_child_of_kind(node, KIND_IDENTIFIER) {
                push_token(name, type_index(&SemanticTokenType::DECORATOR), 0, src, out);
            }
        }

        _ => {}
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Find the first direct child with a name identifier (simple_identifier or identifier).
fn child_ident<'a>(node: Node<'a>, _src: &[u8]) -> Option<Node<'a>> {
    for i in 0..node.child_count() {
        let child = node.child(i)?;
        if child.kind() == KIND_SIMPLE_IDENT
            || child.kind() == KIND_IDENTIFIER
            || child.kind() == KIND_TYPE_IDENT
        {
            return Some(child);
        }
    }
    None
}

fn first_child_of_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == kind {
                return Some(child);
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

fn has_keyword_child(node: Node<'_>, keyword: &str) -> bool {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            if cursor.node().kind() == keyword {
                return true;
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    false
}

/// Check whether a Kotlin node has a modifier keyword (e.g. "suspend", "abstract").
fn has_modifier(node: Node<'_>, src: &[u8], keyword: &str) -> bool {
    // Modifiers are either direct keyword children or inside a `modifiers` node.
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == KIND_MODIFIERS
                && node_text(child, src).split_whitespace().any(|w| w == keyword)
            {
                return true;
            }
            if child.kind() == keyword {
                return true;
            }
        }
    }
    false
}

/// Check whether a Java node has a modifier keyword inside a `modifiers` node.
fn has_java_modifier(node: Node<'_>, src: &[u8], keyword: &str) -> bool {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "modifiers"
                && node_text(child, src).split_whitespace().any(|w| w == keyword)
            {
                return true;
            }
        }
    }
    false
}

/// True when this node is a direct child of a class/interface/enum body.
fn is_inside_class_body(node: Node<'_>) -> bool {
    let Some(parent) = node.parent() else { return false };
    matches!(
        parent.kind(),
        "class_body" | "interface_body" | "enum_class_body" | "object_body"
    )
}

fn node_text<'a>(node: Node<'_>, src: &'a [u8]) -> &'a str {
    node.utf8_text(src).unwrap_or("")
}

fn push_token(node: Node<'_>, token_type: u32, token_modifiers_bitset: u32, src: &[u8], out: &mut Vec<RawToken>) {
    let start = node.start_position();
    let text = node_text(node, src);
    let length = text.encode_utf16().count() as u32;
    if length == 0 {
        return;
    }
    out.push(RawToken {
        line: start.row as u32,
        col: byte_col_to_utf16(src, start.row, start.column),
        length,
        token_type,
        token_modifiers_bitset,
    });
}

/// Convert a tree-sitter byte-column to a UTF-16 column for LSP.
fn byte_col_to_utf16(src: &[u8], row: usize, byte_col: usize) -> u32 {
    let line_start = src
        .iter()
        .enumerate()
        .filter(|(_, &b)| b == b'\n')
        .nth(row.saturating_sub(1))
        .map(|(i, _)| i + 1)
        .unwrap_or(0);
    let line_bytes = &src[line_start..];
    let prefix = if byte_col <= line_bytes.len() {
        &line_bytes[..byte_col]
    } else {
        line_bytes
    };
    std::str::from_utf8(prefix)
        .map(|s| s.encode_utf16().count() as u32)
        .unwrap_or(byte_col as u32)
}

// ─── Delta encoding ───────────────────────────────────────────────────────────

fn delta_encode(sorted: Vec<RawToken>) -> Vec<SemanticToken> {
    let mut result = Vec::with_capacity(sorted.len());
    let mut prev_line = 0u32;
    let mut prev_start = 0u32;

    for tok in sorted {
        let delta_line = tok.line - prev_line;
        let delta_start = if delta_line == 0 {
            tok.col - prev_start
        } else {
            tok.col
        };
        result.push(SemanticToken {
            delta_line,
            delta_start,
            length: tok.length,
            token_type: tok.token_type,
            token_modifiers_bitset: tok.token_modifiers_bitset,
        });
        prev_line = tok.line;
        prev_start = tok.col;
    }
    result
}

// ─── Public API ──────────────────────────────────────────────────────────────

pub(crate) fn full_tokens(doc: &LiveDoc, language: Language) -> SemanticTokens {
    SemanticTokens {
        result_id: None,
        data: collect_tokens(doc, language, None),
    }
}

pub(crate) fn range_tokens(doc: &LiveDoc, language: Language, range: &Range) -> SemanticTokens {
    SemanticTokens {
        result_id: None,
        data: collect_tokens(doc, language, Some(range)),
    }
}

#[cfg(test)]
#[path = "semantic_tokens_tests.rs"]
mod tests;
