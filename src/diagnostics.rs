use std::fmt::Write as _;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum DiagnosticKind {
    Lexer,
    Syntax,
    Semantic,
    Type,
    Import,
}

impl DiagnosticKind {
    #[allow(dead_code)]
    pub fn label(&self) -> &'static str {
        match self {
            DiagnosticKind::Lexer => "Lexer Error",
            DiagnosticKind::Syntax => "Syntax Error",
            DiagnosticKind::Semantic => "Semantic Error",
            DiagnosticKind::Type => "Type Error",
            DiagnosticKind::Import => "Import Error",
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            DiagnosticKind::Lexer => "E0001",
            DiagnosticKind::Syntax => "E0002",
            DiagnosticKind::Semantic => "E0003",
            DiagnosticKind::Type => "E0004",
            DiagnosticKind::Import => "E0005",
        }
    }
}

pub struct Diagnostic<'a> {
    pub kind: DiagnosticKind,
    pub filename: &'a str,
    pub source: &'a str,
    pub line: usize,
    pub column: usize,
    pub message: &'a str,
    pub help: Option<&'a str>,
}

impl<'a> Diagnostic<'a> {
    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "error[{}]: {}",
            self.kind.code(),
            self.message
        );
        let _ = writeln!(
            out,
            "  --> {}:{}:{}",
            self.filename, self.line, self.column
        );

        let lines: Vec<&str> = self.source.lines().collect();
        let line_num_str = self.line.to_string();
        let pad = " ".repeat(line_num_str.len());

        let _ = writeln!(out, "   {} |", pad);

        if self.line > 0 && self.line <= lines.len() {
            let line_content = lines[self.line - 1];
            let _ = writeln!(out, "{} | {}", line_num_str, line_content);

            let col_offset = if self.column > 0 { self.column - 1 } else { 0 };
            let mut caret_line = String::new();
            for (idx, ch) in line_content.chars().enumerate() {
                if idx >= col_offset {
                    break;
                }
                if ch == '\t' {
                    caret_line.push('\t');
                } else {
                    caret_line.push(' ');
                }
            }

            let _ = writeln!(
                out,
                "   {} | {}^ {}",
                pad, caret_line, self.message
            );
        } else {
            let _ = writeln!(out, "   {} | ^ {}", pad, self.message);
        }

        if let Some(help_msg) = self.help {
            let _ = writeln!(out, "   {} |", pad);
            let _ = writeln!(out, "   {} = help: {}", pad, help_msg);
        }

        out
    }
}

/// Helper function to parse error strings formatted like `[line 14:col 5] Message`
/// and render them using `Diagnostic`.
pub fn render_error_string(filename: &str, source: &str, kind: DiagnosticKind, raw_err: &str) -> String {
    let (line, col, msg) = parse_line_col_message_with_source(raw_err, source);

    let help = derive_help_message(raw_err, &kind);

    let diag = Diagnostic {
        kind,
        filename,
        source,
        line,
        column: col,
        message: &msg,
        help: help.as_deref(),
    };
    diag.render()
}

pub fn parse_line_col_message_with_source(raw_err: &str, source: &str) -> (usize, usize, String) {
    let (mut line, mut col, msg) = parse_line_col_message(raw_err);
    if line == 1 && col == 1 && !raw_err.contains("[line ") {
        let (loc_line, loc_col) = locate_symbol_in_source(source, raw_err);
        line = loc_line;
        col = loc_col;
    }
    (line, col, msg)
}

pub fn locate_symbol_in_source(source: &str, raw_err: &str) -> (usize, usize) {
    let var_target = if let Some(s) = raw_err.find("variable '") {
        let rest = &raw_err[s + 10..];
        rest.find('\'').map(|e| &rest[..e])
    } else if let Some(s) = raw_err.find("struct '") {
        let rest = &raw_err[s + 8..];
        rest.find('\'').map(|e| &rest[..e])
    } else if let Some(s) = raw_err.find("identifier '") {
        let rest = &raw_err[s + 13..];
        rest.find('\'').map(|e| &rest[..e])
    } else if let Some(s) = raw_err.find("function '") {
        let rest = &raw_err[s + 10..];
        rest.find('\'').map(|e| &rest[..e])
    } else {
        None
    };

    if let Some(target) = var_target {
        for (line_idx, line) in source.lines().enumerate() {
            if let Some(col_idx) = line.find(target) {
                return (line_idx + 1, col_idx + 1);
            }
        }
    }
    (1, 1)
}

/// Attempt to generate an automatic fix for a given error and source code.
/// Returns `Some((fixed_source, fix_description))` if an automatic fix is available.
pub fn try_auto_fix(source: &str, raw_err: &str) -> Option<(String, String)> {
    if raw_err.contains("immutable variable '") {
        if let Some(s) = raw_err.find("variable '") {
            let rest = &raw_err[s + 10..];
            if let Some(e) = rest.find('\'') {
                let var_name = &rest[..e];
                let target = format!("{} :=", var_name);
                let target_no_space = format!("{}:=", var_name);
                if source.contains(&target) {
                    let replacement = format!("mut {} :=", var_name);
                    let new_src = source.replacen(&target, &replacement, 1);
                    let desc = format!("convert '{}:=' to 'mut {} :='", var_name, var_name);
                    return Some((new_src, desc));
                } else if source.contains(&target_no_space) {
                    let replacement = format!("mut {}:=", var_name);
                    let new_src = source.replacen(&target_no_space, &replacement, 1);
                    let desc = format!("convert '{}:=' to 'mut {}:='", var_name, var_name);
                    return Some((new_src, desc));
                }
            }
        }
    }
    if raw_err.contains("Void function '") && raw_err.contains("' cannot return a value") {
        if let Some(s) = raw_err.find("function '") {
            let rest = &raw_err[s + 10..];
            if let Some(e) = rest.find('\'') {
                let func_name = &rest[..e];
                let target = format!("def {}():", func_name);
                if source.contains(&target) {
                    let replacement = format!("def {}() -> Int:", func_name);
                    let new_src = source.replacen(&target, &replacement, 1);
                    let desc = format!("add return type '-> Int' to function '{}'", func_name);
                    return Some((new_src, desc));
                }
            }
        }
    }
    None
}

fn parse_line_col_message(raw_err: &str) -> (usize, usize, String) {
    if let Some(start) = raw_err.find("[line ") {
        if let Some(end) = raw_err[start..].find(']') {
            let tag = &raw_err[start + 6..start + end]; // e.g. "14:col 5"
            let mut line = 1;
            let mut col = 1;

            if let Some(col_idx) = tag.find(":col ") {
                if let Ok(l) = tag[..col_idx].parse::<usize>() {
                    line = l;
                }
                if let Ok(c) = tag[col_idx + 5..].parse::<usize>() {
                    col = c;
                }
            }

            let rest = raw_err[start + end + 1..].trim();
            let clean_msg = rest
                .strip_prefix("Syntax Error:")
                .or_else(|| rest.strip_prefix("Lexer error:"))
                .or_else(|| rest.strip_prefix("Semantic Error:"))
                .or_else(|| rest.strip_prefix("Type Error:"))
                .unwrap_or(rest)
                .trim();

            return (line, col, clean_msg.to_string());
        }
    }

    (1, 1, raw_err.to_string())
}

fn derive_help_message(raw_err: &str, kind: &DiagnosticKind) -> Option<String> {
    if raw_err.contains("immutable variable") {
        return Some("declare the variable with 'mut x := ...' to allow mutation".to_string());
    }
    if raw_err.contains("Expected ':' after if condition") {
        return Some("if statements in L++ must end with a colon (e.g. 'if x == 1:')".to_string());
    }
    if raw_err.contains("Expected 'def'") || raw_err.contains("Expected ':'") {
        return Some("check block syntax, function signatures, and indentation levels".to_string());
    }
    if raw_err.contains("not found") || raw_err.contains("Undeclared") {
        return Some("verify module imports or symbol declarations".to_string());
    }
    match kind {
        DiagnosticKind::Lexer => Some("ensure indentation uses spaces, not tabs, and string quotes match".to_string()),
        DiagnosticKind::Type => Some("ensure parameter and return types match the expected signatures".to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_rust_style_diagnostic() {
        let source = "def main():\n    x = 42\n    print(x)";
        let diag = Diagnostic {
            kind: DiagnosticKind::Semantic,
            filename: "src/main.lpp",
            source,
            line: 2,
            column: 5,
            message: "Cannot reassign immutable variable 'x'",
            help: Some("declare variable with 'mut x := ...'"),
        };

        let rendered = diag.render();
        assert!(rendered.contains("error[E0003]: Cannot reassign immutable variable 'x'"));
        assert!(rendered.contains("  --> src/main.lpp:2:5"));
        assert!(rendered.contains("2 |     x = 42"));
        assert!(rendered.contains("^ Cannot reassign immutable variable 'x'"));
        assert!(rendered.contains("= help: declare variable with 'mut x := ...'"));
    }
}
