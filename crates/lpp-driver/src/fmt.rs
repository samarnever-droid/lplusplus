//! Canonical L++ source formatter (`lpp fmt`) shared with `lpp-lsp`.

use std::fs;
use std::path::{Path, PathBuf};

/// Format an L++ source buffer into canonical 4-space indented form.
///
/// Preserves semantic block nesting from the source's indentation hierarchy while
/// normalizing tabs to 4 spaces, stripping trailing whitespace, collapsing excess
/// blank lines (max 1 consecutive blank line), normalizing trailing newlines,
/// and preserving comments. Guaranteed idempotent.
#[must_use]
pub fn format_source(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    // Stack of raw column indentations seen so far; level 0 is always 0.
    let mut raw_stack: Vec<usize> = vec![0];
    let mut prev_blank = true; // suppresses leading blank lines
    let mut expect_block = false;
    let mut bracket_depth: i32 = 0;

    for raw_line in source.lines() {
        let normalized_ws = expand_leading_tabs(raw_line);
        let trimmed = normalized_ws.trim();

        if trimmed.is_empty() {
            if !prev_blank {
                out.push('\n');
                prev_blank = true;
            }
            continue;
        }
        prev_blank = false;

        let raw_indent = normalized_ws.len() - normalized_ws.trim_start().len();
        let is_comment = trimmed.starts_with('#');
        let inside_brackets = bracket_depth > 0;

        let level = if inside_brackets {
            raw_stack.len().saturating_sub(1) + 1
        } else {
            if expect_block && raw_indent > *raw_stack.last().unwrap_or(&0) {
                raw_stack.push(raw_indent);
            } else {
                while raw_stack.len() > 1 && raw_indent < *raw_stack.last().unwrap_or(&0) {
                    raw_stack.pop();
                }
                if !is_comment && raw_indent > *raw_stack.last().unwrap_or(&0) && expect_block {
                    raw_stack.push(raw_indent);
                }
            }
            let mut lvl = raw_stack.len().saturating_sub(1);
            if !is_comment && (starts_with_kw(trimmed, "elif") || starts_with_kw(trimmed, "else")) {
                // `elif`/`else` stays aligned with its opening `if`/`for`/`while`.
                if lvl > 0 && raw_indent <= *raw_stack.get(lvl).unwrap_or(&raw_indent) {
                    // raw_stack already popped if raw_indent matched `if`
                }
                let _ = &mut lvl;
            }
            lvl
        };

        for _ in 0..(level * 4) {
            out.push(' ');
        }
        out.push_str(trimmed);
        out.push('\n');

        if !is_comment {
            bracket_depth = update_bracket_depth(bracket_depth, trimmed);
            let code_part = strip_line_comment(trimmed).trim_end();
            expect_block = code_part.ends_with(':') || code_part.ends_with("=>");
        }
    }

    while out.ends_with("\n\n") {
        out.pop();
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn expand_leading_tabs(line: &str) -> String {
    let trimmed_start = line.trim_start_matches([' ', '\t']);
    let prefix = &line[..line.len() - trimmed_start.len()];
    let mut expanded = String::with_capacity(line.len() + 8);
    for ch in prefix.chars() {
        if ch == '\t' {
            expanded.push_str("    ");
        } else {
            expanded.push(ch);
        }
    }
    expanded.push_str(trimmed_start.trim_end());
    expanded
}

fn starts_with_kw(trimmed: &str, kw: &str) -> bool {
    trimmed
        .strip_prefix(kw)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', ':', '(']))
}

fn strip_line_comment(line: &str) -> &str {
    let mut in_str: Option<char> = None;
    let mut escaped = false;
    for (idx, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if let Some(q) = in_str {
            if ch == '\\' {
                escaped = true;
            } else if ch == q {
                in_str = None;
            }
        } else if ch == '"' || ch == '\'' {
            in_str = Some(ch);
        } else if ch == '#' {
            return &line[..idx];
        }
    }
    line
}

fn update_bracket_depth(mut depth: i32, line: &str) -> i32 {
    let mut in_str: Option<char> = None;
    let mut escaped = false;
    for ch in line.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if let Some(q) = in_str {
            if ch == '\\' {
                escaped = true;
            } else if ch == q {
                in_str = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => in_str = Some(ch),
            '#' => break,
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = (depth - 1).max(0),
            _ => {}
        }
    }
    depth
}

fn collect_lpp_files(path: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if path.is_file() {
        if path.extension().and_then(|e| e.to_str()) == Some("lpp") {
            out.push(path.to_path_buf());
        }
        return Ok(());
    }
    if path.is_dir() {
        let mut entries: Vec<PathBuf> = fs::read_dir(path)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .collect();
        entries.sort();
        for entry in entries {
            let name = entry.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(name, ".git" | "target" | ".lpp_packages" | "node_modules") {
                continue;
            }
            collect_lpp_files(&entry, out)?;
        }
    }
    Ok(())
}

/// CLI entry point for `lpp fmt`.
pub fn run_fmt(args: &[String], cwd: &Path) -> i32 {
    let mut check_only = false;
    let mut use_stdin = false;
    let mut targets: Vec<PathBuf> = Vec::new();

    for arg in args {
        match arg.as_str() {
            "--check" => check_only = true,
            "--write" | "-w" => check_only = false,
            "--stdin" => use_stdin = true,
            "--help" | "-h" => {
                println!("Usage: lpp fmt [--check] [--write] [--stdin] [path...]");
                return 0;
            }
            other if other.starts_with('-') => {
                eprintln!("fmt: unknown flag `{other}`");
                return 2;
            }
            path => {
                let p = Path::new(path);
                targets.push(if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    cwd.join(p)
                });
            }
        }
    }

    if use_stdin {
        use std::io::Read;
        let mut input = String::new();
        if let Err(err) = std::io::stdin().read_to_string(&mut input) {
            eprintln!("fmt: failed to read stdin: {err}");
            return 1;
        }
        let formatted = format_source(&input);
        if check_only {
            return if formatted == input { 0 } else { 1 };
        }
        print!("{formatted}");
        return 0;
    }

    if targets.is_empty() {
        targets.push(cwd.to_path_buf());
    }

    let mut files = Vec::new();
    for target in &targets {
        if !target.exists() {
            eprintln!("fmt: path not found: {}", target.display());
            return 2;
        }
        if let Err(err) = collect_lpp_files(target, &mut files) {
            eprintln!("fmt: failed walking {}: {err}", target.display());
            return 1;
        }
    }

    if files.is_empty() {
        eprintln!("fmt: no `.lpp` files found");
        return 1;
    }

    let mut unformatted = 0usize;
    let mut changed = 0usize;
    for file in &files {
        let Ok(src) = fs::read_to_string(file) else {
            eprintln!("fmt: failed to read {}", file.display());
            return 1;
        };
        let formatted = format_source(&src);
        if formatted != src {
            if check_only {
                println!("Diff in {}", file.display());
                unformatted += 1;
            } else if let Err(err) = fs::write(file, formatted) {
                eprintln!("fmt: failed to write {}: {err}", file.display());
                return 1;
            } else {
                changed += 1;
            }
        }
    }

    if check_only {
        if unformatted > 0 {
            eprintln!("fmt: {unformatted} file(s) need formatting");
            return 1;
        }
        return 0;
    }

    if changed > 0 {
        println!("Formatted {changed} file(s).");
    }
    0
}
