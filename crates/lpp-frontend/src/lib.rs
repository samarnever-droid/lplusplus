#![forbid(unsafe_code)]

mod lexer;
mod literal;
mod parser;
mod snapshot;
mod syntax;
mod token;

pub use lexer::{LexedFile, lex, lex_shared};
pub use parser::parse;
pub use snapshot::syntax_snapshot;
pub use syntax::{
    Attribute, Import, ImportKind, InvalidModulePath, Item, ItemKind, ModulePath, ParsedModule,
    SyntaxKind, SyntaxNode, TokenRange,
};
pub use token::{Keyword, Token, TokenKind};

use std::sync::Arc;

use lpp_common::{Diagnostic, FileId};

pub fn parse_source(file: FileId, source: &str) -> Result<ParsedModule, Vec<Diagnostic>> {
    parse_shared(file, Arc::from(source))
}

pub fn parse_shared(file: FileId, source: Arc<str>) -> Result<ParsedModule, Vec<Diagnostic>> {
    parse(lex_shared(file, source)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file() -> FileId {
        FileId::from_raw(3)
    }

    fn parse_ok(source: &str) -> ParsedModule {
        parse_source(file(), source)
            .unwrap_or_else(|diagnostics| panic!("expected parse success, got {diagnostics:#?}"))
    }

    fn error_code(source: &str) -> String {
        parse_source(file(), source).expect_err("expected parse failure")[0]
            .code
            .as_str()
            .to_owned()
    }

    #[test]
    fn token_storage_is_compact_and_shares_one_source_buffer() {
        assert_eq!(std::mem::size_of::<Token>(), 16);
        let parsed = parse_ok("def main():\n    return\n");
        assert_eq!(parsed.reconstruct(), "def main():\n    return\n");
        assert!(
            parsed
                .tokens
                .iter()
                .all(|token| token.text(&parsed.source).len()
                    == token.span.end as usize - token.span.start as usize)
        );
    }

    #[test]
    fn parses_items_blocks_attributes_and_imports() {
        let source = concat!(
            "import std.io as io\n",
            "from app.model import User, Role\n",
            "@inline\n",
            "async def load(path: Str) -> Int:\n",
            "    if path == \"\":\n",
            "        return 0\n",
            "    return 1\n",
        );
        let parsed = parse_ok(source);
        assert_eq!(parsed.reconstruct(), source);
        assert_eq!(parsed.items.len(), 3);
        assert_eq!(parsed.imports.len(), 2);
        assert_eq!(parsed.items[2].name.as_deref(), Some("load"));
        assert_eq!(parsed.items[2].attributes[0].name, "inline");
        assert_eq!(parsed.syntax[3].children.len(), 2);
    }

    #[test]
    fn import_ast_preserves_paths_aliases_and_names() {
        let parsed = parse_ok("import foo.bar as baz\nfrom foo.bar import One, Two\n");
        assert_eq!(parsed.imports[0].path.components(), &["foo", "bar"]);
        assert_eq!(
            parsed.imports[0].kind,
            ImportKind::Module {
                alias: Some("baz".to_owned())
            }
        );
        assert_eq!(
            parsed.imports[1].kind,
            ImportKind::Selective {
                names: vec!["One".to_owned(), "Two".to_owned()]
            }
        );
    }

    #[test]
    fn snapshot_is_deterministic_and_span_complete() {
        let parsed = parse_ok("def id(x: Int) -> Int:\n    return x\n");
        let first = syntax_snapshot(&parsed);
        let second = syntax_snapshot(&parsed);
        let expected = concat!(
            "file 3\n",
            "tokens\n",
            "  0000 Keyword(Def) 0..3 \"def\"\n",
            "  0001 Whitespace 3..4 \" \"\n",
            "  0002 Identifier 4..6 \"id\"\n",
            "  0003 LeftParen 6..7 \"(\"\n",
            "  0004 Identifier 7..8 \"x\"\n",
            "  0005 Colon 8..9 \":\"\n",
            "  0006 Whitespace 9..10 \" \"\n",
            "  0007 Identifier 10..13 \"Int\"\n",
            "  0008 RightParen 13..14 \")\"\n",
            "  0009 Whitespace 14..15 \" \"\n",
            "  0010 Arrow 15..17 \"->\"\n",
            "  0011 Whitespace 17..18 \" \"\n",
            "  0012 Identifier 18..21 \"Int\"\n",
            "  0013 Colon 21..22 \":\"\n",
            "  0014 Newline 22..23 \"\\n\"\n",
            "  0015 Whitespace 23..27 \"    \"\n",
            "  0016 Indent 27..27 \"\"\n",
            "  0017 Keyword(Return) 27..33 \"return\"\n",
            "  0018 Whitespace 33..34 \" \"\n",
            "  0019 Identifier 34..35 \"x\"\n",
            "  0020 Newline 35..36 \"\\n\"\n",
            "  0021 Dedent 36..36 \"\"\n",
            "  0022 Eof 36..36 \"\"\n",
            "syntax\n",
            "  Function 0..35 [0..14]\n",
            "    Return 27..35 [17..20]\n",
            "imports\n",
        );
        assert_eq!(first, expected);
        assert_eq!(first, second);
    }

    #[test]
    fn rejects_known_frontend_debt_forms() {
        assert_eq!(error_code("def f():\n    result: SemVer\n"), "E1105");
        assert_eq!(error_code("def f():\n    if yes: continue\n"), "E1104");
        assert_eq!(error_code("def f():\n    x := (1, 2, 3, 4, 5)\n"), "E1110");
        assert_eq!(
            error_code("def f(a: Int..., b: Int):\n    return\n"),
            "E1111"
        );
        assert_eq!(error_code("def f() {\n}\n"), "E1001");
    }

    #[test]
    fn rejects_malformed_headers_and_imports() {
        assert_eq!(error_code("def (): \n    return\n"), "E1107");
        assert_eq!(error_code("import foo as\n"), "E1120");
        assert_eq!(error_code("from foo Bar\n"), "E1120");
        assert_eq!(error_code("const VALUE\n"), "E1107");
    }
}
