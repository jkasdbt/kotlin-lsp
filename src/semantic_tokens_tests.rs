use tower_lsp::lsp_types::{Position, Range, SemanticTokenType};
use tree_sitter_kotlin;

use crate::indexer::live_tree::parse_live;
use crate::Language;
use crate::semantic_tokens::{full_tokens, range_tokens, TOKEN_TYPES};

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn parse_kotlin(src: &str) -> crate::indexer::LiveDoc {
    parse_live(src, tree_sitter_kotlin::language()).expect("parse failed")
}

fn parse_java(src: &str) -> crate::indexer::LiveDoc {
    parse_live(src, tree_sitter_java::language()).expect("parse failed")
}

/// Find token type id by name in the legend.
fn type_id(t: &SemanticTokenType) -> u32 {
    TOKEN_TYPES.iter().position(|x| x == t).unwrap() as u32
}

/// Decode one SemanticToken from the delta-encoded stream.
/// Returns (line, col, len, token_type, modifiers) for the token at
/// absolute position `index` in the stream.
fn decode_all(
    doc: &crate::indexer::LiveDoc,
    language: Language,
) -> Vec<(u32, u32, u32, u32, u32)> {
    let tokens = full_tokens(doc, language);
    let mut line = 0u32;
    let mut col = 0u32;
    tokens
        .data
        .iter()
        .map(|t| {
            line += t.delta_line;
            if t.delta_line > 0 {
                col = t.delta_start;
            } else {
                col += t.delta_start;
            }
            (line, col, t.length, t.token_type, t.token_modifiers_bitset)
        })
        .collect()
}

// ─── Kotlin tests ─────────────────────────────────────────────────────────────

#[test]
fn kotlin_class_decl() {
    let src = "class Foo";
    let doc = parse_kotlin(src);
    let tokens = decode_all(&doc, Language::Kotlin);
    let class_id = type_id(&SemanticTokenType::CLASS);
    let found = tokens.iter().any(|&(_, _, _, tt, _)| tt == class_id);
    assert!(found, "expected CLASS token for 'class Foo', got: {tokens:?}");
}

#[test]
fn kotlin_interface_decl() {
    let src = "interface Runnable";
    let doc = parse_kotlin(src);
    let tokens = decode_all(&doc, Language::Kotlin);
    let iface_id = type_id(&SemanticTokenType::INTERFACE);
    assert!(
        tokens.iter().any(|&(_, _, _, tt, _)| tt == iface_id),
        "expected INTERFACE token, got: {tokens:?}"
    );
}

#[test]
fn kotlin_enum_class() {
    let src = "enum class Color { RED, GREEN, BLUE }";
    let doc = parse_kotlin(src);
    let tokens = decode_all(&doc, Language::Kotlin);
    let enum_id = type_id(&SemanticTokenType::ENUM);
    let member_id = type_id(&SemanticTokenType::ENUM_MEMBER);
    assert!(
        tokens.iter().any(|&(_, _, _, tt, _)| tt == enum_id),
        "expected ENUM token, got: {tokens:?}"
    );
    assert!(
        tokens.iter().any(|&(_, _, _, tt, _)| tt == member_id),
        "expected ENUM_MEMBER token, got: {tokens:?}"
    );
}

#[test]
fn kotlin_top_level_function() {
    let src = "fun greet() {}";
    let doc = parse_kotlin(src);
    let tokens = decode_all(&doc, Language::Kotlin);
    let fun_id = type_id(&SemanticTokenType::FUNCTION);
    assert!(
        tokens.iter().any(|&(_, _, _, tt, _)| tt == fun_id),
        "expected FUNCTION token for top-level fun, got: {tokens:?}"
    );
}

#[test]
fn kotlin_method_vs_function() {
    let src = r#"
class Foo {
    fun bar() {}
}
fun topLevel() {}
"#;
    let doc = parse_kotlin(src);
    let tokens = decode_all(&doc, Language::Kotlin);
    let method_id = type_id(&SemanticTokenType::METHOD);
    let fun_id = type_id(&SemanticTokenType::FUNCTION);
    assert!(
        tokens.iter().any(|&(_, _, _, tt, _)| tt == method_id),
        "expected METHOD token for class member fun, got: {tokens:?}"
    );
    assert!(
        tokens.iter().any(|&(_, _, _, tt, _)| tt == fun_id),
        "expected FUNCTION token for top-level fun, got: {tokens:?}"
    );
}

#[test]
fn kotlin_val_is_readonly() {
    let src = "val x: Int = 1";
    let doc = parse_kotlin(src);
    let tokens = decode_all(&doc, Language::Kotlin);
    // READONLY modifier bit = 1
    let readonly_bit = 1u32 << 1;
    assert!(
        tokens.iter().any(|&(_, _, _, _, mods)| mods & readonly_bit != 0),
        "expected READONLY modifier for val, got: {tokens:?}"
    );
}

#[test]
fn kotlin_var_not_readonly() {
    let src = "var x: Int = 1";
    let doc = parse_kotlin(src);
    let tokens = decode_all(&doc, Language::Kotlin);
    let readonly_bit = 1u32 << 1;
    assert!(
        !tokens.iter().all(|&(_, _, _, _, mods)| mods & readonly_bit != 0),
        "var should not always have READONLY modifier"
    );
}

#[test]
fn kotlin_type_parameter() {
    let src = "fun <T> identity(x: T): T = x";
    let doc = parse_kotlin(src);
    let tokens = decode_all(&doc, Language::Kotlin);
    let tp_id = type_id(&SemanticTokenType::TYPE_PARAMETER);
    assert!(
        tokens.iter().any(|&(_, _, _, tt, _)| tt == tp_id),
        "expected TYPE_PARAMETER token, got: {tokens:?}"
    );
}

#[test]
fn kotlin_companion_object() {
    let src = r#"
class Foo {
    companion object {
        val CONST = 1
    }
}
"#;
    let doc = parse_kotlin(src);
    let tokens = decode_all(&doc, Language::Kotlin);
    let ns_id = type_id(&SemanticTokenType::NAMESPACE);
    assert!(
        tokens.iter().any(|&(_, _, _, tt, _)| tt == ns_id),
        "expected NAMESPACE token for companion object, got: {tokens:?}"
    );
}

#[test]
fn kotlin_object_decl() {
    let src = "object Singleton { val x = 1 }";
    let doc = parse_kotlin(src);
    let tokens = decode_all(&doc, Language::Kotlin);
    let ns_id = type_id(&SemanticTokenType::NAMESPACE);
    assert!(
        tokens.iter().any(|&(_, _, _, tt, _)| tt == ns_id),
        "expected NAMESPACE token for object decl, got: {tokens:?}"
    );
}

#[test]
fn kotlin_range_restricts_to_range() {
    let src = r#"
class Foo
fun bar() {}
"#;
    let doc = parse_kotlin(src);
    // Only request line 1 (0-indexed) — "class Foo"
    let range = Range {
        start: Position { line: 1, character: 0 },
        end: Position { line: 1, character: 9 },
    };
    let tokens = range_tokens(&doc, Language::Kotlin, &range);
    // Should have tokens only on line 1
    let decoded: Vec<_> = {
        let mut line = 0u32;
        let mut col = 0u32;
        tokens
            .data
            .iter()
            .map(|t| {
                line += t.delta_line;
                if t.delta_line > 0 {
                    col = t.delta_start;
                } else {
                    col += t.delta_start;
                }
                line
            })
            .collect()
    };
    assert!(
        decoded.iter().all(|&l| l == 1),
        "range_tokens should only return tokens on line 1, got lines: {decoded:?}"
    );
}

// ─── Java tests ───────────────────────────────────────────────────────────────

#[test]
fn java_class_decl() {
    let src = "class Foo {}";
    let doc = parse_java(src);
    let tokens = decode_all(&doc, Language::Java);
    let class_id = type_id(&SemanticTokenType::CLASS);
    assert!(
        tokens.iter().any(|&(_, _, _, tt, _)| tt == class_id),
        "expected CLASS token for Java class, got: {tokens:?}"
    );
}

#[test]
fn java_interface_decl() {
    let src = "interface Runnable { void run(); }";
    let doc = parse_java(src);
    let tokens = decode_all(&doc, Language::Java);
    let iface_id = type_id(&SemanticTokenType::INTERFACE);
    assert!(
        tokens.iter().any(|&(_, _, _, tt, _)| tt == iface_id),
        "expected INTERFACE token for Java interface, got: {tokens:?}"
    );
}

#[test]
fn java_method_decl() {
    let src = r#"
class Foo {
    void bar() {}
}
"#;
    let doc = parse_java(src);
    let tokens = decode_all(&doc, Language::Java);
    let method_id = type_id(&SemanticTokenType::METHOD);
    assert!(
        tokens.iter().any(|&(_, _, _, tt, _)| tt == method_id),
        "expected METHOD token for Java method, got: {tokens:?}"
    );
}

#[test]
fn java_enum_constant() {
    let src = "enum Color { RED, GREEN, BLUE }";
    let doc = parse_java(src);
    let tokens = decode_all(&doc, Language::Java);
    let enum_id = type_id(&SemanticTokenType::ENUM);
    let member_id = type_id(&SemanticTokenType::ENUM_MEMBER);
    assert!(
        tokens.iter().any(|&(_, _, _, tt, _)| tt == enum_id),
        "expected ENUM token for Java enum, got: {tokens:?}"
    );
    assert!(
        tokens.iter().any(|&(_, _, _, tt, _)| tt == member_id),
        "expected ENUM_MEMBER token for Java enum constant, got: {tokens:?}"
    );
}

#[test]
fn java_type_parameter() {
    let src = r#"
class Box<T> {
    T value;
}
"#;
    let doc = parse_java(src);
    let tokens = decode_all(&doc, Language::Java);
    let tp_id = type_id(&SemanticTokenType::TYPE_PARAMETER);
    assert!(
        tokens.iter().any(|&(_, _, _, tt, _)| tt == tp_id),
        "expected TYPE_PARAMETER token for Java generic, got: {tokens:?}"
    );
}

#[test]
fn delta_encoding_is_sorted() {
    let src = r#"
class Foo {
    val x: Int = 1
    fun bar(): Int = x
}
"#;
    let doc = parse_kotlin(src);
    let tokens = full_tokens(&doc, Language::Kotlin);
    // delta_line must never be negative (tokens must be in order)
    for t in &tokens.data {
        assert!(
            t.delta_line < 0x8000_0000,
            "delta_line overflow suggests unsorted tokens: {t:?}"
        );
    }
}

#[test]
fn empty_file_returns_no_tokens() {
    let doc = parse_kotlin("");
    let tokens = full_tokens(&doc, Language::Kotlin);
    assert!(tokens.data.is_empty(), "empty file should produce no tokens");
}

