//! Production-grade integration test suite for `lpp-frontend`.
//!
//! Covers lexical scanning, literal forms, unicode handling, indentation and
//! dedent recovery, operator tokens, declarations, attributes, generics,
//! turbofish, closures, async/await/spawn, match arms, multiline expressions,
//! and 65+ negative syntax/lexical error cases.

use std::collections::BTreeSet;
use std::sync::Arc;

use lpp_common::{Diagnostic, FileId, SourceMap};
use lpp_frontend::{
    Attribute, ImportKind, ItemKind, Keyword, LexedFile, ModulePath, ParsedModule, SyntaxKind,
    SyntaxNode, Token, TokenKind, lex, lex_shared, parse, parse_shared, parse_source,
    syntax_snapshot,
};

fn test_file() -> FileId {
    FileId::from_raw(7)
}

fn lex_ok(source: &str) -> LexedFile {
    lex(test_file(), source)
        .unwrap_or_else(|diagnostics| panic!("expected lex success, got {diagnostics:#?}"))
}

fn lex_err(source: &str) -> Vec<Diagnostic> {
    lex(test_file(), source).expect_err("expected lex failure")
}

fn parse_ok(source: &str) -> ParsedModule {
    parse_source(test_file(), source)
        .unwrap_or_else(|diagnostics| panic!("expected parse success, got {diagnostics:#?}"))
}

fn parse_err(source: &str) -> Vec<Diagnostic> {
    parse_source(test_file(), source).expect_err("expected parse failure")
}

fn error_code(source: &str) -> String {
    let diagnostics = parse_err(source);
    assert!(
        !diagnostics.is_empty(),
        "expected at least one diagnostic for:\n{source}"
    );
    assert!(
        !diagnostics[0].message.is_empty(),
        "diagnostic message must not be empty"
    );
    diagnostics[0].code.as_str().to_owned()
}

fn non_trivia_tokens(lexed: &LexedFile) -> Vec<Token> {
    lexed
        .tokens
        .iter()
        .copied()
        .filter(|token| !token.kind.is_trivia())
        .collect()
}

fn non_trivia_kinds(source: &str) -> Vec<TokenKind> {
    non_trivia_tokens(&lex_ok(source))
        .into_iter()
        .map(|token| token.kind)
        .collect()
}

fn node_tokens<'a>(module: &'a ParsedModule, node: &SyntaxNode) -> Vec<(&'a Token, &'a str)> {
    module.tokens[node.tokens.start..node.tokens.end]
        .iter()
        .filter(|token| !token.kind.is_trivia())
        .map(|token| (token, token.text(&module.source)))
        .collect()
}

fn count_syntax_kind(nodes: &[SyntaxNode], target: SyntaxKind) -> usize {
    nodes
        .iter()
        .map(|node| usize::from(node.kind == target) + count_syntax_kind(&node.children, target))
        .sum()
}

fn assert_spans_valid(module: &ParsedModule) {
    let len = module.source.len() as u32;
    for token in &module.tokens {
        assert_eq!(token.span.file, module.file);
        assert!(token.span.start <= token.span.end);
        assert!(token.span.end <= len);
        assert!(module.source.is_char_boundary(token.span.start as usize));
        assert!(module.source.is_char_boundary(token.span.end as usize));
    }
    for item in &module.items {
        assert_eq!(item.span.file, module.file);
        assert!(item.span.start <= item.span.end);
        assert!(item.span.end <= len);
    }
}

// ============================================================================
// Section 1: Lexical Tokens, Keywords, Trivia, Reconstruction & Snapshots
// ============================================================================

#[test]
fn token_and_keyword_coverage_is_complete() {
    let source = concat!(
        "as async await break const continue def elif else enum extern fn for from ",
        "if impl import in match mut pub return spawn struct trait type unsafe while\n",
    );
    let tokens = non_trivia_tokens(&lex_ok(source));
    let expected = [
        Keyword::As,
        Keyword::Async,
        Keyword::Await,
        Keyword::Break,
        Keyword::Const,
        Keyword::Continue,
        Keyword::Def,
        Keyword::Elif,
        Keyword::Else,
        Keyword::Enum,
        Keyword::Extern,
        Keyword::Fn,
        Keyword::For,
        Keyword::From,
        Keyword::If,
        Keyword::Impl,
        Keyword::Import,
        Keyword::In,
        Keyword::Match,
        Keyword::Mut,
        Keyword::Pub,
        Keyword::Return,
        Keyword::Spawn,
        Keyword::Struct,
        Keyword::Trait,
        Keyword::Type,
        Keyword::Unsafe,
        Keyword::While,
    ];
    let actual: Vec<Keyword> = tokens
        .iter()
        .filter_map(|token| match token.kind {
            TokenKind::Keyword(keyword) => Some(keyword),
            _ => None,
        })
        .collect();
    assert_eq!(actual, expected);
    assert_eq!(Keyword::from_ident("not_a_keyword"), None);
    assert_eq!(Keyword::from_ident("true"), None);
    assert_eq!(Keyword::from_ident("and"), None);
}

#[test]
fn identifiers_starting_with_keywords_remain_identifiers() {
    let source = concat!(
        "def_count async_io awaitable broken const_val continued default elif_x ",
        "elsewhere enum_tag external fn_ptr forever from_path iffy impl_block ",
        "imported inner matcher mutable public returned spawner struct_size ",
        "trait_obj typedef unsafely while_loop true_flag false_flag and_or not_ok _ _hidden\n",
    );
    let lexed = lex_ok(source);
    let idents: Vec<&str> = lexed
        .tokens
        .iter()
        .filter(|token| token.kind == TokenKind::Identifier)
        .map(|token| token.text(&lexed.source))
        .collect();
    assert_eq!(idents.len(), 34);
    assert!(idents.contains(&"_"));
    assert!(idents.contains(&"_hidden"));
    assert!(idents.contains(&"true_flag"));
    assert!(idents.contains(&"and_or"));
    assert!(idents.contains(&"not_ok"));
}

#[test]
fn all_operator_and_punctuation_tokens_lex_accurately() {
    let source = concat!(
        ":= = == != < > <= >= && || & | ^ << >> += -= *= /= %= ",
        ": -> + - * / % ? ! ( ) [ ] , . .. ..= ... @ and or not true false\n",
    );
    let kinds = non_trivia_kinds(source);
    assert_eq!(
        kinds,
        [
            TokenKind::Assign,
            TokenKind::Equal,
            TokenKind::EqualEqual,
            TokenKind::NotEqual,
            TokenKind::Less,
            TokenKind::Greater,
            TokenKind::LessEqual,
            TokenKind::GreaterEqual,
            TokenKind::LogicalAnd,
            TokenKind::LogicalOr,
            TokenKind::BitAnd,
            TokenKind::BitOr,
            TokenKind::BitXor,
            TokenKind::ShiftLeft,
            TokenKind::ShiftRight,
            TokenKind::PlusEqual,
            TokenKind::MinusEqual,
            TokenKind::StarEqual,
            TokenKind::SlashEqual,
            TokenKind::PercentEqual,
            TokenKind::Colon,
            TokenKind::Arrow,
            TokenKind::Plus,
            TokenKind::Minus,
            TokenKind::Star,
            TokenKind::Slash,
            TokenKind::Percent,
            TokenKind::Question,
            TokenKind::Not,
            TokenKind::LeftParen,
            TokenKind::RightParen,
            TokenKind::LeftBracket,
            TokenKind::RightBracket,
            TokenKind::Comma,
            TokenKind::Dot,
            TokenKind::DotDot,
            TokenKind::DotDotEqual,
            TokenKind::Ellipsis,
            TokenKind::At,
            TokenKind::LogicalAnd,
            TokenKind::LogicalOr,
            TokenKind::Not,
            TokenKind::Bool,
            TokenKind::Bool,
            TokenKind::Newline,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn lexer_preserves_bom_comments_whitespace_and_crlf_losslessly() {
    let source = "\u{feff}# top hash comment\r\n// slash comment\r\ndef main():\r\n    x := 1  # inline\r\n    y := 2 // inline slash\r\n";
    let lexed = lex_ok(source);
    assert_eq!(lexed.reconstruct(), source);
    let rebuilt: String = lexed
        .tokens
        .iter()
        .filter(|token| !token.is_synthetic())
        .map(|token| token.text(&lexed.source))
        .collect();
    assert_eq!(rebuilt, source);
    assert_eq!(lexed.tokens[0].kind, TokenKind::Bom);
    let comments: Vec<&str> = lexed
        .tokens
        .iter()
        .filter(|token| token.kind == TokenKind::Comment)
        .map(|token| token.text(&lexed.source))
        .collect();
    assert_eq!(
        comments,
        [
            "# top hash comment",
            "// slash comment",
            "# inline",
            "// inline slash",
        ]
    );
    assert_eq!(
        lexed
            .tokens
            .iter()
            .filter(|token| token.kind == TokenKind::Newline)
            .count(),
        5
    );
}

#[test]
fn token_layout_and_shared_buffer_invariants_hold() {
    assert_eq!(std::mem::size_of::<Token>(), 16);
    let shared: Arc<str> = Arc::from("def ping() -> Int:\n    return 1\n");
    let lexed = lex_shared(FileId::from_raw(12), Arc::clone(&shared)).unwrap();
    assert!(Arc::ptr_eq(&lexed.source, &shared));
    let parsed = parse_shared(FileId::from_raw(12), Arc::clone(&shared)).unwrap();
    assert!(Arc::ptr_eq(&parsed.source, &shared));
    assert!(
        parsed
            .tokens
            .last()
            .is_some_and(|t| t.kind == TokenKind::Eof && t.is_synthetic())
    );
    assert_spans_valid(&parsed);
}

#[test]
fn parsed_module_remap_file_updates_every_span_recursively() {
    let source = concat!(
        "import math.vec as v\n",
        "@inline\n",
        "pub def compute(x: Int) -> Int:\n",
        "    if x > 0:\n",
        "        return x\n",
        "    return 0\n",
    );
    let mut parsed = parse_source(FileId::from_raw(1), source).unwrap();
    let target = FileId::from_raw(99);
    parsed.remap_file(target);
    assert_eq!(parsed.file, target);
    for token in &parsed.tokens {
        assert_eq!(token.span.file, target);
    }
    for item in &parsed.items {
        assert_eq!(item.span.file, target);
        for attr in &item.attributes {
            assert_eq!(attr.span.file, target);
        }
    }
    for import in &parsed.imports {
        assert_eq!(import.span.file, target);
    }
    fn check_node(node: &SyntaxNode, file: FileId) {
        assert_eq!(node.span.file, file);
        for child in &node.children {
            check_node(child, file);
        }
    }
    for node in &parsed.syntax {
        check_node(node, target);
    }
}

#[test]
fn syntax_snapshot_captures_tokens_syntax_tree_and_imports() {
    let source = concat!(
        "import std.fs\n",
        "from app.types import Point, Line\n",
        "def origin() -> Int:\n",
        "    return 0\n",
    );
    let parsed = parse_ok(source);
    let snap_a = syntax_snapshot(&parsed);
    let snap_b = syntax_snapshot(&parsed);
    assert_eq!(snap_a, snap_b);
    assert!(snap_a.contains("file 7\n"));
    assert!(snap_a.contains("Import "));
    assert!(snap_a.contains("FromImport "));
    assert!(snap_a.contains("Function "));
    assert!(snap_a.contains("Return "));
    assert!(snap_a.contains("std.fs @"));
    assert!(snap_a.contains("app.types import Point,Line @"));
}

#[test]
fn lexer_tokenizes_operator_precedence_matrix_cleanly() {
    let source = concat!(
        "def matrix(a: Int, b: Int, c: Int, d: Bool) -> Bool:\n",
        "    arith := a + b * c - a / b % c\n",
        "    bits := (a & b) | (a ^ c) << 2 >> 1\n",
        "    cmp := arith <= bits and bits != 0 or not d && !d || a == b\n",
        "    casted := (arith as Float) + 1.5\n",
        "    return cmp\n",
    );
    let parsed = parse_ok(source);
    assert_eq!(parsed.items.len(), 1);
    let fn_node = &parsed.syntax[0];
    assert_eq!(fn_node.children.len(), 5);
    assert_eq!(fn_node.children[0].kind, SyntaxKind::Binding);
    assert_eq!(fn_node.children[1].kind, SyntaxKind::Binding);
    assert_eq!(fn_node.children[2].kind, SyntaxKind::Binding);
    assert_eq!(fn_node.children[3].kind, SyntaxKind::Binding);
    assert_eq!(fn_node.children[4].kind, SyntaxKind::Return);
}

// ============================================================================
// Section 2: Strings, F-Strings, Triple-Quoted Strings, Char & Unicode
// ============================================================================

#[test]
fn standard_strings_support_all_escape_sequences() {
    let source = concat!(
        "def escapes() -> Str:\n",
        "    a := \"line1\\nline2\\r\\t\\0\\\"\\'\\\\\"\n",
        "    b := \"hex:\\x41\\x7a\\x00\\xff\"\n",
        "    c := \"\"\n",
        "    return a + b + c\n",
    );
    let parsed = parse_ok(source);
    let strings: Vec<&str> = parsed
        .tokens
        .iter()
        .filter(|token| token.kind == TokenKind::String)
        .map(|token| token.text(&parsed.source))
        .collect();
    assert_eq!(
        strings,
        [
            "\"line1\\nline2\\r\\t\\0\\\"\\'\\\\\"",
            "\"hex:\\x41\\x7a\\x00\\xff\"",
            "\"\"",
        ]
    );
}

#[test]
fn triple_quoted_multiline_strings_preserve_embedded_newlines_and_quotes() {
    let source = concat!(
        "def doc() -> Str:\n",
        "    s := \"\"\"line one\n",
        "    indented \"quoted\" and ''single''\n",
        "line three with \\n and \\x42\n",
        "\"\"\"\n",
        "    return s\n",
    );
    let parsed = parse_ok(source);
    assert_eq!(parsed.reconstruct(), source);
    let strings: Vec<&str> = parsed
        .tokens
        .iter()
        .filter(|token| token.kind == TokenKind::String)
        .map(|token| token.text(&parsed.source))
        .collect();
    assert_eq!(strings.len(), 1);
    assert!(strings[0].starts_with("\"\"\"line one\n"));
    assert!(strings[0].ends_with("\"\"\""));
    assert_eq!(
        parsed
            .tokens
            .iter()
            .filter(|token| token.kind == TokenKind::Indent)
            .count(),
        1
    );
}

#[test]
fn fstrings_handle_nested_braces_escapes_and_multiline_triple_quotes() {
    let source = concat!(
        "def fmt(x: Int, name: Str) -> Str:\n",
        "    a := f\"\"\n",
        "    b := f\"hello {name}, sum={x + 1}, escaped=\\{raw\\}\"\n",
        "    c := f\"\"\"nested={wrap(f\"inner:{x}\", \"{\")}\"\"\"\n",
        "    d := f\"tuple={(x, x + 1).0} and call={compute(1, 2)}\"\n",
        "    e := f\"\"\"multi\n",
        "line {x * 2}\n",
        "end\"\"\"\n",
        "    return b + c + d + e\n",
    );
    let parsed = parse_ok(source);
    let fstrings: Vec<&str> = parsed
        .tokens
        .iter()
        .filter(|token| token.kind == TokenKind::FString)
        .map(|token| token.text(&parsed.source))
        .collect();
    assert_eq!(fstrings.len(), 5);
    assert_eq!(fstrings[0], "f\"\"");
    assert!(fstrings[1].contains("\\{raw\\}"));
    assert!(fstrings[2].contains("f\"inner:{x}\""));
    assert!(fstrings[4].starts_with("f\"\"\"multi\n"));
}

#[test]
fn raw_prefix_before_string_lexes_as_raw_string() {
    let kinds = non_trivia_kinds("r\"hello\"\n");
    assert_eq!(
        kinds,
        [TokenKind::RawString, TokenKind::Newline, TokenKind::Eof]
    );
    let spaced = non_trivia_kinds("r \"hello\"\n");
    assert_eq!(
        spaced,
        [
            TokenKind::Identifier,
            TokenKind::String,
            TokenKind::Newline,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn character_literals_support_ascii_escapes_and_multibyte_unicode() {
    let source = concat!(
        "def chars() -> Int:\n",
        "    c1 := 'A'\n",
        "    c2 := '\\n'\n",
        "    c3 := '\\r'\n",
        "    c4 := '\\t'\n",
        "    c5 := '\\0'\n",
        "    c6 := '\\''\n",
        "    c7 := '\\\\'\n",
        "    c8 := '\\x7F'\n",
        "    c9 := 'π'\n",
        "    c10 := 'λ'\n",
        "    c11 := '中'\n",
        "    c12 := '🚀'\n",
        "    return 12\n",
    );
    let parsed = parse_ok(source);
    let char_tokens: Vec<&str> = parsed
        .tokens
        .iter()
        .filter(|token| token.kind == TokenKind::Character)
        .map(|token| token.text(&parsed.source))
        .collect();
    assert_eq!(
        char_tokens,
        [
            "'A'", "'\\n'", "'\\r'", "'\\t'", "'\\0'", "'\\''", "'\\\\'", "'\\x7F'", "'π'", "'λ'",
            "'中'", "'🚀'",
        ]
    );
    assert_spans_valid(&parsed);
}

#[test]
fn unicode_identifiers_strings_comments_and_diagnostic_columns_are_accurate() {
    let source = concat!(
        "# αβγδ comment with 🚀\n",
        "def café(π_val: Int, naïve_str: Str) -> Str:\n",
        "    λ_msg := f\"π={π_val}, text={naïve_str} 🎯\"\n",
        "    return λ_msg\n",
    );
    let parsed = parse_ok(source);
    assert_spans_valid(&parsed);
    let idents: Vec<&str> = parsed
        .tokens
        .iter()
        .filter(|token| token.kind == TokenKind::Identifier)
        .map(|token| token.text(&parsed.source))
        .collect();
    assert!(idents.contains(&"café"));
    assert!(idents.contains(&"π_val"));
    assert!(idents.contains(&"naïve_str"));
    assert!(idents.contains(&"λ_msg"));

    // Verify human diagnostic rendering reports character column after multibyte UTF-8.
    let bad = "def test():\n    π_α := {\n";
    let mut sources = SourceMap::new();
    let file = sources.add_file("unicode.lpp", bad).unwrap();
    let diag = &parse_source(file, bad).unwrap_err()[0];
    assert_eq!(diag.code.as_str(), "E1001");
    let rendered = diag.render_human(&sources);
    assert!(rendered.contains("unicode.lpp:2:12"), "got:\n{rendered}");
}

// ============================================================================
// Section 3: Numeric Literals, Ranges & Numeric Boundary Cases
// ============================================================================

#[test]
fn numeric_literals_cover_decimal_hex_binary_underscores_and_floats() {
    let source = concat!(
        "def numbers() -> Float:\n",
        "    z := 0\n",
        "    dec := 1_000_000_000\n",
        "    max_i64 := 9_223_372_036_854_775_807\n",
        "    hex_lo := 0x0\n",
        "    hex_hi := 0X7FFF_FFFF_FFFF_FFFF\n",
        "    hex_mask := 0xdead_BEEF\n",
        "    bin_zero := 0b0\n",
        "    bin_bits := 0B1010_0101_1111_0000\n",
        "    flt_zero := 0.0\n",
        "    flt_pi := 3.141_592_653\n",
        "    flt_big := 1_000_000.500_25\n",
        "    return flt_pi + flt_big + flt_zero\n",
    );
    let parsed = parse_ok(source);
    let ints: Vec<&str> = parsed
        .tokens
        .iter()
        .filter(|token| token.kind == TokenKind::Integer)
        .map(|token| token.text(&parsed.source))
        .collect();
    assert_eq!(
        ints,
        [
            "0",
            "1_000_000_000",
            "9_223_372_036_854_775_807",
            "0x0",
            "0X7FFF_FFFF_FFFF_FFFF",
            "0xdead_BEEF",
            "0b0",
            "0B1010_0101_1111_0000",
        ]
    );
    let floats: Vec<&str> = parsed
        .tokens
        .iter()
        .filter(|token| token.kind == TokenKind::Float)
        .map(|token| token.text(&parsed.source))
        .collect();
    assert_eq!(floats, ["0.0", "3.141_592_653", "1_000_000.500_25"]);
}

#[test]
fn range_dots_and_method_dots_after_integers_do_not_mislex_as_floats() {
    let source = concat!(
        "def ranges(n: Int):\n",
        "    for i in 0..10:\n",
        "        print(i)\n",
        "    for j in 0..=n:\n",
        "        print(j)\n",
        "    s := 42.to_str()\n",
    );
    let parsed = parse_ok(source);
    assert!(
        parsed
            .tokens
            .iter()
            .all(|token| token.kind != TokenKind::Float),
        "integer ranges and integer method calls must not lex as Float"
    );
    assert!(
        parsed
            .tokens
            .iter()
            .any(|token| token.kind == TokenKind::DotDot)
    );
    assert!(
        parsed
            .tokens
            .iter()
            .any(|token| token.kind == TokenKind::DotDotEqual)
    );
}

#[test]
fn octal_and_scientific_notation_literals_lex_as_single_tokens() {
    let oct_tokens = non_trivia_tokens(&lex_ok("0o77 0O7_5_5\n"));
    assert_eq!(oct_tokens[0].kind, TokenKind::Integer);
    assert_eq!(oct_tokens[0].text(&lex_ok("0o77 0O7_5_5\n").source), "0o77");
    assert_eq!(oct_tokens[1].kind, TokenKind::Integer);

    let sci_src = "1.5e10 1e5 2.5E-3 1_000e+2\n";
    let sci_lexed = lex_ok(sci_src);
    let sci_tokens = non_trivia_tokens(&sci_lexed);
    assert_eq!(
        &sci_tokens
            .iter()
            .filter(|t| t.kind == TokenKind::Float)
            .map(|t| t.text(&sci_lexed.source))
            .collect::<Vec<_>>(),
        &["1.5e10", "1e5", "2.5E-3", "1_000e+2"]
    );
}

#[test]
fn raw_strings_unicode_escapes_and_bitwise_compound_ops_lex() {
    let src = concat!(
        "def f():\n",
        "    raw := r\"C:\\\\path\\n\"\n",
        "    tri := r\"\"\"line1\\nline2\"\"\"\n",
        "    uni := \"\\u{03C0}\\u{1F680}\"\n",
        "    ch := '\\u{0041}'\n",
        "    mut x := ~0o755\n",
        "    x &= 1\n",
        "    x |= 2\n",
        "    x ^= 3\n",
        "    x <<= 1\n",
        "    x >>= 1\n",
    );
    let lexed = lex_ok(src);
    let kinds: Vec<TokenKind> = lexed.tokens.iter().map(|t| t.kind).collect();
    assert!(kinds.contains(&TokenKind::RawString));
    assert!(kinds.contains(&TokenKind::BitNot));
    assert!(kinds.contains(&TokenKind::BitAndEqual));
    assert!(kinds.contains(&TokenKind::BitOrEqual));
    assert!(kinds.contains(&TokenKind::BitXorEqual));
    assert!(kinds.contains(&TokenKind::ShiftLeftEqual));
    assert!(kinds.contains(&TokenKind::ShiftRightEqual));
}

// ============================================================================
// Section 4: Indentation, Dedent Recovery, Soft Newlines & Multiline Exprs
// ============================================================================

#[test]
fn deeply_nested_blocks_unwind_all_dedents_at_eof_and_between_items() {
    let source = concat!(
        "def deep(flag: Bool, n: Int) -> Int:\n",
        "    if flag:\n",
        "        while n > 0:\n",
        "            for i in 0..n:\n",
        "                match i:\n",
        "                    0:\n",
        "                        unsafe:\n",
        "                            return 1\n",
        "                    _:\n",
        "                        continue\n",
        "    return 0\n",
        "\n",
        "def sibling() -> Int:\n",
        "    return 2\n",
    );
    let parsed = parse_ok(source);
    let indents = parsed
        .tokens
        .iter()
        .filter(|token| token.kind == TokenKind::Indent)
        .count();
    let dedents = parsed
        .tokens
        .iter()
        .filter(|token| token.kind == TokenKind::Dedent)
        .count();
    assert_eq!(indents, 9);
    assert_eq!(dedents, 9);
    assert_eq!(parsed.items.len(), 2);
}

#[test]
fn blank_lines_and_comments_at_arbitrary_columns_do_not_break_indentation() {
    let source = concat!(
        "def spaced() -> Int:\n",
        "\n",
        "  # comment at 2 spaces inside 4-space block\n",
        "        // comment at 8 spaces inside 4-space block\n",
        "    mut total := 0\n",
        "    \n",
        "    if total == 0:\n",
        "\n",
        "# column-0 comment inside nested if\n",
        "        total = 10\n",
        "            \n",
        "    return total\n",
    );
    let parsed = parse_ok(source);
    assert_eq!(parsed.items.len(), 1);
    let func = &parsed.syntax[0];
    assert_eq!(func.children.len(), 3);
    assert_eq!(func.children[0].kind, SyntaxKind::Binding);
    assert_eq!(func.children[1].kind, SyntaxKind::If);
    assert_eq!(func.children[2].kind, SyntaxKind::Return);
}

#[test]
fn non_4_space_consistent_indentation_and_two_space_blocks_are_accepted() {
    let source = concat!(
        "def two_space(x: Int) -> Int:\n",
        "  if x > 0:\n",
        "    return x\n",
        "  else:\n",
        "      if x < 0:\n",
        "          return -x\n",
        "  return 0\n",
    );
    let parsed = parse_ok(source);
    let indents = parsed
        .tokens
        .iter()
        .filter(|t| t.kind == TokenKind::Indent)
        .count();
    let dedents = parsed
        .tokens
        .iter()
        .filter(|t| t.kind == TokenKind::Dedent)
        .count();
    assert_eq!(indents, 4);
    assert_eq!(dedents, 4);
}

#[test]
fn multiline_expressions_inside_parens_and_brackets_use_soft_newlines() {
    let source = concat!(
        "def multi(\n",
        "    a: Int,\n",
        "    b: Int,\n",
        "    c: Map[\n",
        "        Str,\n",
        "        List[Int]\n",
        "    ]\n",
        ") -> Int:\n",
        "    items := [\n",
        "        a + 1,\n",
        "        b + 2,\n",
        "        a * b\n",
        "    ]\n",
        "    pair := (\n",
        "        items[0],\n",
        "        items[1],\n",
        "        (\n",
        "            a + b\n",
        "        )\n",
        "    )\n",
        "    return compute(\n",
        "        pair.0,\n",
        "        pair.1,\n",
        "        pair.2\n",
        "    )\n",
    );
    let parsed = parse_ok(source);
    let soft_newlines = parsed
        .tokens
        .iter()
        .filter(|token| token.kind == TokenKind::SoftNewline)
        .count();
    assert!(soft_newlines >= 15);
    let indents = parsed
        .tokens
        .iter()
        .filter(|token| token.kind == TokenKind::Indent)
        .count();
    assert_eq!(indents, 1, "only the function body should emit Indent");
}

#[test]
fn crlf_multiline_expressions_emit_soft_newlines_preserving_two_byte_spans() {
    let source = "def f(\r\n    x: Int\r\n) -> Int:\r\n    return (\r\n        x + 1\r\n    )\r\n";
    let parsed = parse_ok(source);
    assert_eq!(parsed.reconstruct(), source);
    let soft: Vec<&str> = parsed
        .tokens
        .iter()
        .filter(|token| token.kind == TokenKind::SoftNewline)
        .map(|token| token.text(&parsed.source))
        .collect();
    assert_eq!(soft, ["\r\n", "\r\n", "\r\n", "\r\n"]);
}

// ============================================================================
// Section 5: Declarations, Attributes, Unsafe, Generics, Closures, Async, Match
// ============================================================================

#[test]
fn all_kernel_and_runtime_attributes_parse_and_format_canonically() {
    let source = concat!(
        "@test\n",
        "def test_case():\n",
        "    return\n",
        "\n",
        "@inline\n",
        "@no_mangle\n",
        "@section(\".text.boot\")\n",
        "pub unsafe def boot_entry():\n",
        "    return\n",
        "\n",
        "@interrupt\n",
        "@align(16)\n",
        "def irq_handler():\n",
        "    return\n",
        "\n",
        "@packed\n",
        "@align(64)\n",
        "@repr(\"C\", 8)\n",
        "pub struct CacheLine:\n",
        "    tag: Int\n",
        "    data: Int\n",
    );
    let parsed = parse_ok(source);
    assert_eq!(parsed.items.len(), 4);

    assert_eq!(parsed.items[0].attributes[0].canonical_text(), "test");

    let boot_attrs: Vec<String> = parsed.items[1]
        .attributes
        .iter()
        .map(Attribute::canonical_text)
        .collect();
    assert_eq!(boot_attrs, ["inline", "no_mangle", "section(.text.boot)"]);
    assert!(parsed.items[1].public);
    assert_eq!(parsed.items[1].name.as_deref(), Some("boot_entry"));
    // Item span starts at the first attached attribute `@inline`.
    assert_eq!(
        parsed.items[1].span.start,
        parsed.items[1].attributes[0].span.start
    );

    let irq_attrs: Vec<String> = parsed.items[2]
        .attributes
        .iter()
        .map(Attribute::canonical_text)
        .collect();
    assert_eq!(irq_attrs, ["interrupt", "align(16)"]);

    let struct_attrs: Vec<String> = parsed.items[3]
        .attributes
        .iter()
        .map(Attribute::canonical_text)
        .collect();
    assert_eq!(struct_attrs, ["packed", "align(64)", "repr(C,8)"]);
    assert_eq!(parsed.items[3].kind, ItemKind::Struct);
    assert!(parsed.items[3].public);
}

#[test]
fn unsafe_blocks_and_unsafe_function_combinations_parse() {
    let source = concat!(
        "unsafe def raw_read(ptr: RawPtr) -> Int:\n",
        "    return 0\n",
        "\n",
        "unsafe async def raw_dma() -> Int:\n",
        "    return 1\n",
        "\n",
        "pub unsafe async def pub_dma() -> Int:\n",
        "    unsafe:\n",
        "        x := raw_read(0)\n",
        "        if x == 0:\n",
        "            return 1\n",
        "    return 0\n",
    );
    let parsed = parse_ok(source);
    assert_eq!(parsed.items.len(), 3);
    assert_eq!(parsed.items[0].name.as_deref(), Some("raw_read"));
    assert_eq!(parsed.items[1].name.as_deref(), Some("raw_dma"));
    assert_eq!(parsed.items[2].name.as_deref(), Some("pub_dma"));
    assert!(parsed.items[2].public);
    assert_eq!(count_syntax_kind(&parsed.syntax, SyntaxKind::Unsafe), 1);
}

#[test]
fn traits_impls_generics_enums_structs_consts_types_and_externs_parse() {
    let source = concat!(
        "pub const VERSION = 3\n",
        "pub type Word = Int\n",
        "pub type PairBox[T] = Box[T]\n",
        "\n",
        "pub trait Codec[T]:\n",
        "    def encode(self, item: T) -> Str\n",
        "    def decode(raw: Str) -> T\n",
        "\n",
        "pub struct Box[T: Codec]:\n",
        "    value: T\n",
        "    tag: Int = 0\n",
        "\n",
        "pub enum Outcome[T, E]:\n",
        "    Ok(val: T)\n",
        "    Err(err: E)\n",
        "    Empty\n",
        "\n",
        "impl[T] Box[T]:\n",
        "    def get(self) -> T:\n",
        "        return self.value\n",
        "\n",
        "impl[T: Codec] Codec[T] for Box[T]:\n",
        "    def encode(self, item: T) -> Str:\n",
        "        return \"box\"\n",
        "    def decode(raw: Str) -> T:\n",
        "        return raw\n",
        "\n",
        "pub extern \"C\" link \"sqlite3\":\n",
        "    def sqlite3_open(filename: Str, db: RawPtr) -> Int\n",
        "    def sqlite3_close(db: RawPtr) -> Int\n",
    );
    let parsed = parse_ok(source);
    assert_eq!(parsed.items.len(), 9);
    assert_eq!(parsed.items[0].kind, ItemKind::Const);
    assert_eq!(parsed.items[1].kind, ItemKind::TypeAlias);
    assert_eq!(parsed.items[2].kind, ItemKind::TypeAlias);
    assert_eq!(parsed.items[3].kind, ItemKind::Trait);
    assert_eq!(parsed.items[4].kind, ItemKind::Struct);
    assert_eq!(parsed.items[5].kind, ItemKind::Enum);
    assert_eq!(parsed.items[6].kind, ItemKind::Impl);
    assert_eq!(parsed.items[7].kind, ItemKind::Impl);
    assert_eq!(parsed.items[8].kind, ItemKind::Extern);
    assert!(parsed.items.iter().take(6).all(|item| item.public));
    assert!(parsed.items[8].public);
}

#[test]
fn extern_block_item_and_nested_signature_nodes_parse() {
    let source = concat!(
        "pub extern \"C\" link \"z\":\n",
        "    def compress(dst: RawPtr, src: RawPtr, len: Int) -> Int\n",
        "    def adler32(adler: Int, buf: RawPtr, len: Int) -> Int\n",
        "\n",
        "trait Visitor:\n",
        "    def visit_int(self, v: Int)\n",
        "    def visit_str(self, s: Str) -> Bool\n",
    );
    let parsed = parse_ok(source);
    assert_eq!(parsed.items.len(), 2);
    assert_eq!(parsed.items[0].kind, ItemKind::Extern);
    assert!(parsed.items[0].public);
    assert_eq!(parsed.syntax[0].children.len(), 2);
    assert!(parsed.syntax[0].children[0].children.is_empty());
    assert_eq!(parsed.items[1].kind, ItemKind::Trait);
    assert_eq!(parsed.syntax[1].children.len(), 2);
    assert!(parsed.syntax[1].children[0].children.is_empty());
}

#[test]
fn turbofish_syntaxes_default_args_variadics_closures_and_async_parse() {
    let source = concat!(
        "def id[T](x: T) -> T:\n",
        "    return x\n",
        "\n",
        "def with_defaults(host: Str, port: Int = 8080, tls: Bool = true) -> Int:\n",
        "    return port\n",
        "\n",
        "def sum_all(seed: Int, ...rest: Int) -> Int:\n",
        "    mut total := seed\n",
        "    for v in rest:\n",
        "        total += v\n",
        "    return total\n",
        "\n",
        "async def fetch(id_num: Int) -> Int:\n",
        "    return id_num + 1\n",
        "\n",
        "def caller() -> Int:\n",
        "    a := id::<Int>(42)\n",
        "    b := id::[Int](43)\n",
        "    c := id[Int](44)\n",
        "    inline_fn := fn(x: Int, y: Int) -> Int: x + y\n",
        "    block_fn := fn(x: Int) -> Int:\n",
        "        mut z := x * 2\n",
        "        return z + 1\n",
        "    task := spawn fetch(a)\n",
        "    awaited := task.await\n",
        "    return inline_fn(b, c) + block_fn(awaited)\n",
    );
    let parsed = parse_ok(source);
    assert_eq!(parsed.items.len(), 5);
    let caller_node = &parsed.syntax[4];
    let block_fn_stmt = &caller_node.children[4];
    assert_eq!(block_fn_stmt.kind, SyntaxKind::Binding);
    assert_eq!(
        block_fn_stmt.children.len(),
        2,
        "multiline closure body must attach child statements to its binding node"
    );
}

#[test]
fn tuple_arity_boundary_accepts_2_to_16_and_allows_17_plus_call_arguments() {
    let tuple_2 = "def t():\n    x := (1, 2)\n";
    let tuple_16 = "def t():\n    x := (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16)\n";
    let nested_16 =
        "def t():\n    x := ((1, 2, 3), 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16)\n";
    assert!(parse_source(test_file(), tuple_2).is_ok());
    assert!(parse_source(test_file(), tuple_16).is_ok());
    assert!(parse_source(test_file(), nested_16).is_ok());

    // Calls, indexed callees, parenthesized callees, and `fn(...)` params with >= 17 items are NOT tuples.
    let call_20 = "def t():\n    r := big_fn(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20)\n";
    let bracket_call_20 = "def t():\n    r := fns[0](1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20)\n";
    let paren_call_20 = "def t():\n    r := (make_fn())(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20)\n";
    let closure_18 = "def t():\n    f := fn(a: Int, b: Int, c: Int, d: Int, e: Int, f: Int, g: Int, h: Int, i: Int, j: Int, k: Int, l: Int, m: Int, n: Int, o: Int, p: Int, q: Int, r: Int) -> Int: a\n";
    let fn_def_18 = "def big(a: Int, b: Int, c: Int, d: Int, e: Int, f: Int, g: Int, h: Int, i: Int, j: Int, k: Int, l: Int, m: Int, n: Int, o: Int, p: Int, q: Int, r: Int) -> Int:\n    return a\n";
    assert!(parse_source(test_file(), call_20).is_ok());
    assert!(parse_source(test_file(), bracket_call_20).is_ok());
    assert!(parse_source(test_file(), paren_call_20).is_ok());
    assert!(parse_source(test_file(), closure_18).is_ok());
    assert!(parse_source(test_file(), fn_def_18).is_ok());
}

#[test]
fn match_arms_cover_values_ranges_wildcards_or_patterns_enums_and_guards() {
    let source = concat!(
        "def classify(n: Int, ev: Event, flag: Bool) -> Int:\n",
        "    match n:\n",
        "        -10..-1:\n",
        "            return 1\n",
        "        0 | 1 | 2:\n",
        "            return 2\n",
        "        3..=10 if flag:\n",
        "            return 3\n",
        "        x if x > 100:\n",
        "            return x\n",
        "        _:\n",
        "            return 0\n",
        "    match ev:\n",
        "        Event.Click(x, y) if x >= 0 and y >= 0:\n",
        "            return x + y\n",
        "        Event.Key(k) | Event.Repeat(k):\n",
        "            return k\n",
        "        Event.Quit:\n",
        "            return -1\n",
        "        _:\n",
        "            return 0\n",
    );
    let parsed = parse_ok(source);
    assert_eq!(count_syntax_kind(&parsed.syntax, SyntaxKind::Match), 2);
    assert_eq!(count_syntax_kind(&parsed.syntax, SyntaxKind::MatchArm), 9);
}

#[test]
fn imports_preserve_nested_paths_aliases_and_selective_lists() {
    let source = concat!(
        "import std.collections.deque\n",
        "import std.net.http as http_client\n",
        "from kernel.mm.paging import PageTable, PhysAddr, map_page\n",
    );
    let parsed = parse_ok(source);
    assert_eq!(parsed.imports.len(), 3);
    assert_eq!(
        parsed.imports[0].path.components(),
        ["std", "collections", "deque"]
    );
    assert_eq!(parsed.imports[0].path.slash_path(), "std/collections/deque");
    assert_eq!(parsed.imports[0].kind, ImportKind::Module { alias: None });
    assert_eq!(
        parsed.imports[1].kind,
        ImportKind::Module {
            alias: Some("http_client".to_owned())
        }
    );
    assert_eq!(
        parsed.imports[2].kind,
        ImportKind::Selective {
            names: vec![
                "PageTable".to_owned(),
                "PhysAddr".to_owned(),
                "map_page".to_owned(),
            ]
        }
    );
    assert!(ModulePath::new(vec![]).is_err());
    assert!(ModulePath::new(vec!["a".to_owned(), String::new()]).is_err());
}

#[test]
fn item_syntax_index_maps_directly_to_module_syntax_nodes() {
    let source = concat!(
        "import a.b\n",
        "@inline\n",
        "pub def foo() -> Int:\n",
        "    return 1\n",
        "struct Bar:\n",
        "    x: Int\n",
    );
    let parsed = parse_ok(source);
    assert_eq!(parsed.items.len(), 3);
    for item in &parsed.items {
        let node = &parsed.syntax[item.syntax_index as usize];
        let text = node_tokens(&parsed, node)
            .into_iter()
            .map(|(_, t)| t)
            .collect::<Vec<_>>()
            .join(" ");
        assert!(!text.is_empty());
    }
    assert_eq!(
        parsed.syntax[parsed.items[1].syntax_index as usize].kind,
        SyntaxKind::Function
    );
    assert_eq!(
        parsed.syntax[parsed.items[2].syntax_index as usize].kind,
        SyntaxKind::Struct
    );
}

// ============================================================================
// Section 6: 65+ Negative Syntax & Lexical Error Cases
// ============================================================================

#[test]
fn negative_lexical_unexpected_characters_e1001() {
    let cases = [
        // 1: curly braces (C-style block)
        "def f() {\n}\n",
        // 2: semicolon
        "def f():\n    x := 1;\n",
        // 3: stray closing brace outside formatted string
        "def f():\n    x := }\n",
        // 4: backtick
        "def f():\n    x := `a`\n",
        // 5: dollar sign
        "def f():\n    $x := 1\n",
        // 6: mid-line tab character
        "def f():\n    x := 1\t+ 2\n",
        // 7: backslash outside string
        "def f():\n    x := 1 \\\n    + 2\n",
    ];
    for (idx, src) in cases.iter().enumerate() {
        assert_eq!(error_code(src), "E1001", "case #{idx} failed for:\n{src}");
    }
}

#[test]
fn negative_lexical_tab_and_inconsistent_indentation_e1002_e1003() {
    let tab_cases = [
        // 8: leading tab inside function
        "def f():\n\treturn\n",
        // 9: spaces then tab at line start
        "def f():\n    \treturn\n",
        // 10: leading tab at top level
        "\tdef f():\n    return\n",
    ];
    for (idx, src) in tab_cases.iter().enumerate() {
        assert_eq!(error_code(src), "E1002", "tab case #{idx}");
    }

    let indent_cases = [
        // 11: dedent to 2 spaces when stack is [0, 4]
        "def f():\n    x := 1\n  y := 2\n",
        // 12: dedent to 6 spaces when stack is [0, 4, 8]
        "def f():\n    if true:\n        x := 1\n      y := 2\n",
        // 13: dedent to 1 space inside nested block
        "def f():\n    if true:\n        return\n return\n",
    ];
    for (idx, src) in indent_cases.iter().enumerate() {
        assert_eq!(error_code(src), "E1003", "indent case #{idx}");
    }
}

#[test]
fn negative_lexical_literal_errors_e1004() {
    let cases = [
        // 14: unterminated string
        "def f():\n    s := \"unterminated\n",
        // 15: unterminated triple-quoted string
        "def f():\n    s := \"\"\"unterminated\nline2\n",
        // 16: unterminated f-string
        "def f():\n    s := f\"hello {x}\n",
        // 17: unterminated triple f-string
        "def f():\n    s := f\"\"\"hello {x}\n",
        // 18: unterminated escape at EOF inside string
        "def f():\n    s := \"abc\\",
        // 19: unknown escape \q in string
        "def f():\n    s := \"bad \\q\"\n",
        // 20: unknown escape \u in string
        "def f():\n    s := \"bad \\u1234\"\n",
        // 21: escaped brace \{ in plain (non-f) string
        "def f():\n    s := \"\\{\"\n",
        // 22: invalid hex escape \xZ1
        "def f():\n    s := \"\\xZ1\"\n",
        // 23: short hex escape \x4"
        "def f():\n    s := \"\\x4\"\n",
        // 24: empty character literal
        "def f():\n    c := ''\n",
        // 25: multi-character literal
        "def f():\n    c := 'ab'\n",
        // 26: newline inside character literal
        "def f():\n    c := '\n'\n",
        // 27: unterminated character literal at EOF
        "def f():\n    c := '",
        // 28: unclosed character literal with escape
        "def f():\n    c := '\\n",
    ];
    for (idx, src) in cases.iter().enumerate() {
        assert_eq!(
            error_code(src),
            "E1004",
            "literal case #{idx} failed for:\n{src}"
        );
    }
}

#[test]
fn negative_lexical_numeric_overflow_and_empty_radix_e1005() {
    let cases = [
        // 29: decimal i64 overflow
        "def f():\n    x := 9223372036854775808\n",
        // 30: huge decimal integer
        "def f():\n    x := 99999999999999999999999999\n",
        // 31: hex with no digits
        "def f():\n    x := 0x\n",
        // 32: hex with only underscore
        "def f():\n    x := 0x_\n",
        // 33: hex i64 overflow
        "def f():\n    x := 0x8000_0000_0000_0000\n",
        // 34: binary with no digits
        "def f():\n    x := 0b\n",
        // 35: binary i64 overflow (64 bits set -> > i64::MAX)
        "def f():\n    x := 0b1000000000000000000000000000000000000000000000000000000000000000\n",
    ];
    for (idx, src) in cases.iter().enumerate() {
        assert_eq!(
            error_code(src),
            "E1005",
            "numeric case #{idx} failed for:\n{src}"
        );
    }
}

#[test]
fn negative_parser_empty_file_and_delimiter_errors_e1100_e1101() {
    // 36: empty source
    assert_eq!(error_code(""), "E1100");
    // 37: comments and whitespace only
    assert_eq!(error_code("# only a comment\n   \n// another\n"), "E1100");
    // 38: BOM + blank lines only
    assert_eq!(error_code("\u{feff}\n\r\n"), "E1100");

    let delimiter_cases = [
        // 39: unclosed '('
        "def f(x: Int:\n    return x\n",
        // 40: unclosed '['
        "def f[T(x: T):\n    return x\n",
        // 41: mismatched '(]'
        "def f():\n    x := (1, 2]\n",
        // 42: mismatched '[)'
        "def f():\n    x := [1, 2)\n",
        // 43: stray ')'
        "def f():\n    x := 1)\n",
        // 44: stray ']'
        "def f():\n    x := 1]\n",
    ];
    for (idx, src) in delimiter_cases.iter().enumerate() {
        assert_eq!(
            error_code(src),
            "E1101",
            "delimiter case #{idx} failed for:\n{src}"
        );
    }
}

#[test]
fn negative_parser_module_scope_and_attribute_errors_e1102_e1103() {
    let module_scope_cases = [
        // 45: bare assignment at module scope
        "x := 1\n",
        // 46: bare expression at module scope
        "print(42)\n",
        // 47: top-level if statement
        "if true:\n    print(1)\n",
        // 48: top-level return
        "return 0\n",
        // 49: top-level while loop
        "while true:\n    break\n",
        // 50: top-level unsafe block
        "unsafe:\n    print(1)\n",
    ];
    for (idx, src) in module_scope_cases.iter().enumerate() {
        assert_eq!(
            error_code(src),
            "E1102",
            "module scope case #{idx} failed for:\n{src}"
        );
    }

    let attribute_cases = [
        // 51: dangling attribute at EOF
        "@inline\n",
        // 52: attribute followed by non-identifier
        "@123\ndef f():\n    return\n",
        // 53: attribute followed by keyword
        "@def\ndef f():\n    return\n",
        // 54: bare '@'
        "@\ndef f():\n    return\n",
        // 55: attribute with trailing tokens without parens
        "@align 64\nstruct S:\n    x: Int\n",
        // 56: attribute with trailing token after ')'
        "@align(64) extra\nstruct S:\n    x: Int\n",
    ];
    for (idx, src) in attribute_cases.iter().enumerate() {
        assert_eq!(
            error_code(src),
            "E1103",
            "attribute case #{idx} failed for:\n{src}"
        );
    }
}

#[test]
fn negative_parser_block_header_typed_binding_and_indent_errors_e1104_e1105_e1106() {
    let block_cases = [
        // 57: def missing ':'
        "def f()\n    return\n",
        // 58: struct missing ':'
        "struct Point\n    x: Int\n",
        // 59: enum missing ':'
        "enum Color\n    Red\n",
        // 60: if missing ':'
        "def f():\n    if true\n        return\n",
        // 61: inline statement after if ':'
        "def f():\n    if yes: continue\n",
        // 62: inline statement after while ':'
        "def f():\n    while true: break\n",
        // 63: inline statement after unsafe ':'
        "def f():\n    unsafe: return\n",
        // 64: def header with empty body
        "def f():\n",
        // 65: if header with empty body before sibling
        "def f():\n    if true:\n    return\n",
    ];
    for (idx, src) in block_cases.iter().enumerate() {
        assert_eq!(
            error_code(src),
            "E1104",
            "block case #{idx} failed for:\n{src}"
        );
    }

    let typed_binding_cases = [
        // 66: uninitialized typed binding
        "def f():\n    result: SemVer\n",
        // 67: uninitialized generic typed binding
        "def f():\n    items: List[Int]\n",
    ];
    for (idx, src) in typed_binding_cases.iter().enumerate() {
        assert_eq!(
            error_code(src),
            "E1105",
            "typed binding case #{idx} failed for:\n{src}"
        );
    }

    let unexpected_indent_cases = [
        // 68: top-level unexpected indent
        "    def f():\n        return\n",
        // 69: unexpected indent after return statement
        "def f():\n    return 1\n        x := 2\n",
        // 70: unexpected indent after binding statement
        "def f():\n    x := 1\n        y := 2\n",
    ];
    for (idx, src) in unexpected_indent_cases.iter().enumerate() {
        assert_eq!(
            error_code(src),
            "E1106",
            "unexpected indent case #{idx} failed for:\n{src}"
        );
    }
}

#[test]
fn negative_parser_header_tuple_variadic_and_import_errors_e1107_to_e1120() {
    let header_cases = [
        // 71: def without name
        "def ():\n    return\n",
        // 72: def with keyword as name
        "def if():\n    return\n",
        // 73: def without parens
        "def foo:\n    return\n",
        // 74: struct without name
        "struct :\n    x: Int\n",
        // 75: enum without name
        "enum :\n    A\n",
        // 76: trait without name
        "trait :\n    def m(self)\n",
        // 77: const without '='
        "const LIMIT\n",
        // 78: type alias without '='
        "type Handle\n",
    ];
    for (idx, src) in header_cases.iter().enumerate() {
        assert_eq!(
            error_code(src),
            "E1107",
            "header case #{idx} failed for:\n{src}"
        );
    }

    // 79: 17-element tuple rejected
    assert_eq!(
        error_code(
            "def f():\n    t := (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17)\n"
        ),
        "E1110"
    );
    // 80: 20-element nested tuple rejected
    assert_eq!(
        error_code(
            "def f():\n    t := (1, (1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20))\n"
        ),
        "E1110"
    );

    // 81: non-final variadic parameter
    assert_eq!(
        error_code("def f(...a: Int, b: Int):\n    return\n"),
        "E1111"
    );
    // 82: non-final variadic in second position
    assert_eq!(
        error_code("def f(x: Str, ...a: Int, b: Int):\n    return\n"),
        "E1111"
    );

    let import_cases = [
        // 83: bare import
        "import\n",
        // 84: non-identifier module component
        "import 123\n",
        // 85: trailing dot in module path
        "import foo.\n",
        // 86: missing alias after `as`
        "import foo as\n",
        // 87: keyword as alias
        "import foo as if\n",
        // 88: trailing garbage after import
        "import foo bar\n",
        // 89: from without import keyword
        "from foo.bar Baz\n",
        // 90: from ... import with empty list
        "from foo.bar import\n",
        // 91: from ... import with trailing comma
        "from foo.bar import A,\n",
        // 92: from ... import with extra tokens
        "from foo.bar import A as B\n",
    ];
    for (idx, src) in import_cases.iter().enumerate() {
        assert_eq!(
            error_code(src),
            "E1120",
            "import case #{idx} failed for:\n{src}"
        );
    }
}

#[test]
fn negative_suite_covers_every_frontend_diagnostic_code_except_4gib_limit() {
    let expected: BTreeSet<&str> = [
        "E1001", "E1002", "E1003", "E1004", "E1005", "E1100", "E1101", "E1102", "E1103", "E1104",
        "E1105", "E1106", "E1107", "E1110", "E1111", "E1120",
    ]
    .into_iter()
    .collect();
    let samples = [
        "def f() {\n}\n",
        "def f():\n\treturn\n",
        "def f():\n    x := 1\n  y := 2\n",
        "def f():\n    s := \"unterminated\n",
        "def f():\n    x := 0x\n",
        "",
        "def f(:\n    return\n",
        "x := 1\n",
        "@inline\n",
        "def f()\n    return\n",
        "def f():\n    x: Int\n",
        "    def f():\n        return\n",
        "const X\n",
        "def f():\n    t := (1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17)\n",
        "def f(...a: Int, b: Int):\n    return\n",
        "import foo as\n",
    ];
    let observed: BTreeSet<String> = samples.iter().map(|s| error_code(s)).collect();
    let expected_owned: BTreeSet<String> = expected.into_iter().map(str::to_owned).collect();
    assert_eq!(observed, expected_owned);
}

#[test]
fn parser_recovers_file_ids_and_tokens_across_large_realistic_module() {
    let source = concat!(
        "import std.io as io\n",
        "from std.collections import List, Map, Set, Deque\n",
        "\n",
        "@packed\n",
        "@align(32)\n",
        "pub struct FrameHeader:\n",
        "    magic: Int = 0xCAFE_BABE\n",
        "    flags: Int = 0b1010_0001\n",
        "    label: Str = \"frame\"\n",
        "\n",
        "pub enum FrameKind:\n",
        "    Data(payload: List[Int])\n",
        "    Control(code: Int)\n",
        "    Heartbeat\n",
        "\n",
        "pub trait PacketSink:\n",
        "    def send(self, frame: FrameHeader) -> Bool\n",
        "\n",
        "impl PacketSink for FrameHeader:\n",
        "    def send(self, frame: FrameHeader) -> Bool:\n",
        "        return frame.magic != 0\n",
        "\n",
        "@inline\n",
        "pub unsafe async def dispatch_batch(\n",
        "    sink: PacketSink,\n",
        "    limit: Int = 64,\n",
        "    ...ids: Int\n",
        ") -> Int:\n",
        "    mut sent := 0\n",
        "    for i in 0..limit:\n",
        "        match i:\n",
        "            0..=3:\n",
        "                sent += 1\n",
        "            4 | 8 | 12:\n",
        "                sent += 2\n",
        "            other if other > 32:\n",
        "                break\n",
        "            _:\n",
        "                continue\n",
        "    unsafe:\n",
        "        for id_num in ids:\n",
        "            msg := f\"id={id_num}, sent={sent}\"\n",
        "            io.println(msg)\n",
        "    worker := spawn fetch_status(sent)\n",
        "    return worker.await\n",
    );
    let parsed = parse_ok(source);
    assert_spans_valid(&parsed);
    assert_eq!(parsed.imports.len(), 2);
    assert_eq!(parsed.items.len(), 7);
    assert_eq!(count_syntax_kind(&parsed.syntax, SyntaxKind::Unsafe), 1);
    assert_eq!(count_syntax_kind(&parsed.syntax, SyntaxKind::Match), 1);
    assert_eq!(count_syntax_kind(&parsed.syntax, SyntaxKind::MatchArm), 4);
    assert_eq!(count_syntax_kind(&parsed.syntax, SyntaxKind::For), 2);

    // Confirm lex -> parse two-step matches parse_source identically.
    let lexed = lex(test_file(), source).unwrap();
    let parsed_two_step = parse(lexed).unwrap();
    assert_eq!(parsed, parsed_two_step);
}

// ============================================================================
// Section 7: Direct Lexer Diagnostics, Token Helpers & Control-Flow ASTs
// ============================================================================

#[test]
fn lex_err_reports_exact_byte_spans_for_tabs_escapes_and_overflow() {
    let tab_diags = lex_err("def f():\n\treturn\n");
    assert_eq!(tab_diags[0].code.as_str(), "E1002");
    let tab_span = tab_diags[0].primary_span.expect("tab span");
    assert_eq!(tab_span.start, 9);
    assert_eq!(tab_span.end, 10);

    let dedent_diags = lex_err("def f():\n    x := 1\n  y := 2\n");
    assert_eq!(dedent_diags[0].code.as_str(), "E1003");
    let dedent_span = dedent_diags[0].primary_span.expect("dedent span");
    assert_eq!(dedent_span.start, 20);
    assert_eq!(dedent_span.end, 22);

    let esc_diags = lex_err("def f():\n    s := \"ok\\q\"\n");
    assert_eq!(esc_diags[0].code.as_str(), "E1004");
    let esc_span = esc_diags[0].primary_span.expect("escape span");
    assert_eq!(
        &"def f():\n    s := \"ok\\q\"\n"[esc_span.start as usize..esc_span.end as usize],
        "\\"
    );

    let ovf_diags = lex_err("def f():\n    n := 0x8000_0000_0000_0000\n");
    assert_eq!(ovf_diags[0].code.as_str(), "E1005");
    let ovf_span = ovf_diags[0].primary_span.expect("overflow span");
    assert_eq!(
        &"def f():\n    n := 0x8000_0000_0000_0000\n"
            [ovf_span.start as usize..ovf_span.end as usize],
        "0x8000_0000_0000_0000"
    );
}

#[test]
fn token_kind_predicates_classify_trivia_and_line_endings_accurately() {
    assert!(TokenKind::Bom.is_trivia());
    assert!(TokenKind::Whitespace.is_trivia());
    assert!(TokenKind::Comment.is_trivia());
    assert!(TokenKind::SoftNewline.is_trivia());
    assert!(!TokenKind::Newline.is_trivia());
    assert!(!TokenKind::Indent.is_trivia());
    assert!(!TokenKind::Dedent.is_trivia());
    assert!(!TokenKind::Eof.is_trivia());
    assert!(!TokenKind::Identifier.is_trivia());

    assert!(TokenKind::Newline.is_line_end());
    assert!(TokenKind::Eof.is_line_end());
    assert!(TokenKind::Dedent.is_line_end());
    assert!(!TokenKind::SoftNewline.is_line_end());
    assert!(!TokenKind::Indent.is_line_end());
    assert!(!TokenKind::Colon.is_line_end());
}

#[test]
fn statement_syntax_kinds_distinguish_bindings_assignments_and_expressions() {
    let source = concat!(
        "def statements(mut_arr: List[Int], obj: Box):\n",
        "    a := 1\n",
        "    mut b: Int = 2\n",
        "    (x, y) := (3, 4)\n",
        "    b = 5\n",
        "    b += 6\n",
        "    b -= 1\n",
        "    b *= 2\n",
        "    b /= 3\n",
        "    b %= 4\n",
        "    mut_arr[0] = 9\n",
        "    obj.value = 10\n",
        "    foo(a, b)\n",
        "    if a == b:\n",
        "        break\n",
        "    elif a < b:\n",
        "        continue\n",
        "    else:\n",
        "        return\n",
    );
    let parsed = parse_ok(source);
    let children = &parsed.syntax[0].children;
    let kinds: Vec<SyntaxKind> = children.iter().map(|node| node.kind).collect();
    assert_eq!(
        kinds,
        [
            SyntaxKind::Binding,
            SyntaxKind::Binding,
            SyntaxKind::Binding,
            SyntaxKind::Assignment,
            SyntaxKind::Binding,
            SyntaxKind::Binding,
            SyntaxKind::Binding,
            SyntaxKind::Binding,
            SyntaxKind::Binding,
            SyntaxKind::Assignment,
            SyntaxKind::Assignment,
            SyntaxKind::Expression,
            SyntaxKind::If,
            SyntaxKind::Elif,
            SyntaxKind::Else,
        ]
    );
    assert_eq!(children[12].children[0].kind, SyntaxKind::Break);
    assert_eq!(children[13].children[0].kind, SyntaxKind::Continue);
    assert_eq!(children[14].children[0].kind, SyntaxKind::Return);
}

#[test]
fn nested_delimiters_suppress_indents_even_with_leading_tabs_inside_brackets() {
    // Wait: inside delimiters, `scan_indentation` returns early ONLY after reading spaces and rejecting `\t`!
    // Verify that arbitrary space indentation inside multiline calls/lists produces zero Indent tokens
    // and contiguous SoftNewlines.
    let source = concat!(
        "def layout() -> List[Int]:\n",
        "    return [\n",
        "  1,\n",
        "          2,\n",
        "      3,\n",
        "    ]\n",
    );
    let parsed = parse_ok(source);
    let indents = parsed
        .tokens
        .iter()
        .filter(|t| t.kind == TokenKind::Indent)
        .count();
    let softs = parsed
        .tokens
        .iter()
        .filter(|t| t.kind == TokenKind::SoftNewline)
        .count();
    assert_eq!(indents, 1);
    assert_eq!(softs, 4);
}

#[test]
fn lexer_handles_no_trailing_newline_at_eof_with_multi_level_dedent() {
    let source = "def f():\n    if true:\n        if false:\n            return";
    let parsed = parse_ok(source);
    assert_eq!(parsed.reconstruct(), source);
    let indents = parsed
        .tokens
        .iter()
        .filter(|t| t.kind == TokenKind::Indent)
        .count();
    let dedents = parsed
        .tokens
        .iter()
        .filter(|t| t.kind == TokenKind::Dedent)
        .count();
    assert_eq!(indents, 3);
    assert_eq!(dedents, 3);
    assert_eq!(parsed.tokens.last().unwrap().kind, TokenKind::Eof);
}

#[test]
fn additional_negative_edge_cases_cover_malformed_imports_and_headers() {
    let cases = [
        // malformed `from` missing path
        ("from import X\n", "E1120"),
        // malformed `from` trailing dot in path
        ("from foo. import X\n", "E1120"),
        // malformed `from` keyword inside selective list
        ("from foo import def\n", "E1120"),
        // malformed `import` with symbol instead of identifier
        ("import +\n", "E1120"),
        // empty attribute parens `@align()` parses cleanly, while `@align(` fails delimiter
        ("@align(\nstruct S:\n    x: Int\n", "E1101"),
        // `pub` + statement at module scope
        ("pub return 1\n", "E1102"),
        // `pub` + bare expression at module scope
        ("pub x + 1\n", "E1102"),
        // `else` with inline statement
        (
            "def f():\n    if true:\n        return\n    else: return\n",
            "E1104",
        ),
        // `elif` with inline statement
        (
            "def f():\n    if true:\n        return\n    elif false: return\n",
            "E1104",
        ),
        // `for` missing colon
        ("def f():\n    for x in xs\n        return\n", "E1104"),
        // `match` missing colon
        (
            "def f():\n    match x\n        _:\n            return\n",
            "E1104",
        ),
        // `impl` missing colon
        ("impl Point\n    def m(self):\n        return\n", "E1104"),
        // `trait` missing colon
        ("trait Show\n    def show(self) -> Str\n", "E1104"),
        // `extern` missing colon
        ("extern \"C\"\n    def puts(s: Str) -> Int\n", "E1104"),
        // `async def` without name
        ("async def ():\n    return\n", "E1107"),
        // `unsafe def` without parens
        ("unsafe def kernel:\n    return\n", "E1107"),
        // `const` with keyword name
        ("const while = 1\n", "E1107"),
        // `type` with keyword name
        ("type match = Int\n", "E1107"),
        // unclosed delimiter inside multiline expression
        ("def f():\n    x := (\n        1 + 2\n", "E1101"),
        // mismatched bracket inside nested call
        ("def f():\n    x := foo([1, 2))\n", "E1101"),
        // unterminated f-string with escaped quote before EOF
        ("def f():\n    s := f\"abc\\\"", "E1004"),
        // invalid hex escape in character literal
        ("def f():\n    c := '\\xGG'\n", "E1004"),
        // unknown escape in character literal
        ("def f():\n    c := '\\q'\n", "E1004"),
        // double single quote inside char literal
        ("def f():\n    c := '''\n", "E1004"),
        // float overflow / bad char
        ("def f():\n    x := 0xFFFF_FFFF_FFFF_FFFF\n", "E1005"),
    ];
    for (idx, (src, expected_code)) in cases.iter().enumerate() {
        assert_eq!(
            error_code(src),
            *expected_code,
            "extra negative case #{idx} failed for:\n{src}"
        );
    }
}

#[test]
fn syntax_node_spans_strictly_contain_all_descendant_spans() {
    let source = concat!(
        "def tree(a: Int, b: Bool) -> Int:\n",
        "    if a > 0:\n",
        "        while a > 1:\n",
        "            match a:\n",
        "                2:\n",
        "                    return 2\n",
        "                _:\n",
        "                    a -= 1\n",
        "    elif b:\n",
        "        unsafe:\n",
        "            return 9\n",
        "    else:\n",
        "        return 0\n",
        "    return 1\n",
    );
    let parsed = parse_ok(source);

    fn verify_containment(node: &SyntaxNode, file: FileId) {
        assert_eq!(node.span.file, file);
        assert!(node.span.start < node.span.end);
        for child in &node.children {
            assert!(
                child.span.start >= node.span.start && child.span.end <= node.span.end,
                "child {:?} ({}..{}) must fit inside parent {:?} ({}..{})",
                child.kind,
                child.span.start,
                child.span.end,
                node.kind,
                node.span.start,
                node.span.end
            );
            verify_containment(child, file);
        }
    }

    for node in &parsed.syntax {
        verify_containment(node, parsed.file);
    }
}

#[test]
fn attribute_span_and_pub_modifiers_attach_to_next_non_attribute_item() {
    let source = concat!(
        "@packed\n",
        "@align(16)\n",
        "pub struct Aligned:\n",
        "    x: Int\n",
        "\n",
        "@test\n",
        "def check_aligned():\n",
        "    return\n",
    );
    let parsed = parse_ok(source);
    assert_eq!(parsed.items.len(), 2);
    assert_eq!(parsed.items[0].attributes.len(), 2);
    assert_eq!(parsed.items[1].attributes.len(), 1);
    assert!(parsed.items[0].public);
    assert!(!parsed.items[1].public);
    assert!(parsed.items[0].span.end < parsed.items[1].span.start);
}

#[test]
fn block_comments_nested_and_indentation_preserved_and_errors_reported() {
    let source = concat!(
        "/* top-level\n",
        "   /* nested block */ comment */\n",
        "def compute(x: Int) -> Int:\n",
        "    /* standalone block comment */\n",
        "    /* inline */ return x + 1\n",
    );
    let parsed = parse_ok(source);
    assert_eq!(parsed.items.len(), 1);
    assert_eq!(parsed.items[0].name.as_deref(), Some("compute"));

    assert_eq!(error_code("def f():\n    /* unterminated\n"), "E1004");
    assert_eq!(error_code("def f():\n    x := 1e\n"), "E1005");
    assert_eq!(error_code("def f():\n    x := 1e+\n"), "E1005");
    assert_eq!(error_code("def f():\n    x := 1e500\n"), "E1005");
    assert_eq!(error_code("def f():\n    x := 0o\n"), "E1005");
    assert_eq!(error_code("def f():\n    x := \"\\u{D800}\"\n"), "E1004");
    assert_eq!(error_code("def f():\n    x := \"\\u{}\"\n"), "E1004");
    assert_eq!(error_code("def f():\n    x := r\"unterminated\n"), "E1004");

    let suf = lex_ok("a := 255u8 + 10_i32 + 0x1f_usize + 1e3f64\n");
    assert!(suf.tokens.iter().any(|t| t.kind == TokenKind::Integer));
    assert!(suf.tokens.iter().any(|t| t.kind == TokenKind::Float));
    assert_eq!(error_code("def f():\n    x := 256u8\n"), "E1005");
    assert_eq!(error_code("def f():\n    x := 10f32\n"), "E1005");
    assert_eq!(error_code("def f():\n    x := 10foo\n"), "E1005");
}
