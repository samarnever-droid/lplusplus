use std::fmt::Write as _;

use crate::{ImportKind, ParsedModule, SyntaxNode};

#[must_use]
pub fn syntax_snapshot(module: &ParsedModule) -> String {
    let mut output = String::new();
    writeln!(output, "file {}", module.file.raw()).expect("writing to a String cannot fail");
    writeln!(output, "tokens").expect("writing to a String cannot fail");
    for (index, token) in module.tokens.iter().enumerate() {
        writeln!(
            output,
            "  {index:04} {:?} {}..{} {:?}",
            token.kind,
            token.span.start,
            token.span.end,
            token.text(&module.source)
        )
        .expect("writing to a String cannot fail");
    }
    writeln!(output, "syntax").expect("writing to a String cannot fail");
    for node in &module.syntax {
        write_node(&mut output, node, 1);
    }
    writeln!(output, "imports").expect("writing to a String cannot fail");
    for import in &module.imports {
        write!(output, "  {}", import.path.components().join("."))
            .expect("writing to a String cannot fail");
        match &import.kind {
            ImportKind::Module { alias: Some(alias) } => {
                write!(output, " as {alias}").expect("writing to a String cannot fail");
            }
            ImportKind::Module { alias: None } => {}
            ImportKind::Selective { names } => {
                write!(output, " import {}", names.join(","))
                    .expect("writing to a String cannot fail");
            }
        }
        writeln!(output, " @{}..{}", import.span.start, import.span.end)
            .expect("writing to a String cannot fail");
    }
    output
}

fn write_node(output: &mut String, node: &SyntaxNode, depth: usize) {
    writeln!(
        output,
        "{}{:?} {}..{} [{}..{}]",
        "  ".repeat(depth),
        node.kind,
        node.span.start,
        node.span.end,
        node.tokens.start,
        node.tokens.end,
    )
    .expect("writing to a String cannot fail");
    for child in &node.children {
        write_node(output, child, depth + 1);
    }
}
