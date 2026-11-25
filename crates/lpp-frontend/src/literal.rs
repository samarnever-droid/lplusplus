#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LiteralError {
    pub offset: usize,
    pub message: &'static str,
}

pub(crate) fn scan_string(
    input: &str,
    start: usize,
    formatted: bool,
) -> Result<usize, LiteralError> {
    let quote = start + usize::from(formatted);
    let triple = input[quote..].starts_with("\"\"\"");
    let mut offset = quote + if triple { 3 } else { 1 };

    while offset < input.len() {
        if triple && input[offset..].starts_with("\"\"\"") {
            return Ok(offset + 3);
        }
        let (character, width) = next(input, offset);
        if !triple && character == '"' {
            return Ok(offset + width);
        }
        if character == '\\' {
            offset = scan_escape(input, offset, formatted)?;
            continue;
        }
        offset += width;
    }

    Err(LiteralError {
        offset: start,
        message: if formatted {
            "unterminated formatted string"
        } else if triple {
            "unterminated triple-quoted string"
        } else {
            "unterminated string literal"
        },
    })
}

pub(crate) fn scan_raw_string(input: &str, start: usize) -> Result<usize, LiteralError> {
    let quote = start + 1;
    let triple = input[quote..].starts_with("\"\"\"");
    let mut offset = quote + if triple { 3 } else { 1 };

    while offset < input.len() {
        if triple && input[offset..].starts_with("\"\"\"") {
            return Ok(offset + 3);
        }
        let (character, width) = next(input, offset);
        if !triple && character == '"' {
            return Ok(offset + width);
        }
        offset += width;
    }

    Err(LiteralError {
        offset: start,
        message: if triple {
            "unterminated triple-quoted raw string"
        } else {
            "unterminated raw string literal"
        },
    })
}

pub(crate) fn scan_character(input: &str, start: usize) -> Result<usize, LiteralError> {
    let value_start = start + 1;
    if value_start >= input.len() {
        return Err(LiteralError {
            offset: start,
            message: "unterminated character literal",
        });
    }
    let end = if input[value_start..].starts_with('\\') {
        scan_escape(input, value_start, false)?
    } else {
        let (character, width) = next(input, value_start);
        if character == '\n' || character == '\r' || character == '\'' {
            return Err(LiteralError {
                offset: value_start,
                message: "character literal must contain exactly one character",
            });
        }
        value_start + width
    };
    if !input[end..].starts_with('\'') {
        return Err(LiteralError {
            offset: end.min(input.len()),
            message: "unclosed character literal",
        });
    }
    Ok(end + 1)
}

fn scan_escape(input: &str, slash: usize, formatted: bool) -> Result<usize, LiteralError> {
    let escaped_at = slash + 1;
    if escaped_at >= input.len() {
        return Err(LiteralError {
            offset: slash,
            message: "unterminated escape sequence",
        });
    }
    let (escaped, width) = next(input, escaped_at);
    let simple = matches!(escaped, 'n' | 'r' | 't' | '0' | '"' | '\'' | '\\')
        || (formatted && matches!(escaped, '{' | '}'));
    if simple {
        return Ok(escaped_at + width);
    }
    if escaped == 'x' {
        let first = escaped_at + width;
        let second = first.checked_add(1).ok_or(LiteralError {
            offset: slash,
            message: "unterminated hexadecimal escape",
        })?;
        if second >= input.len()
            || !input.as_bytes()[first].is_ascii_hexdigit()
            || !input.as_bytes()[second].is_ascii_hexdigit()
        {
            return Err(LiteralError {
                offset: slash,
                message: "invalid hexadecimal escape",
            });
        }
        return Ok(second + 1);
    }
    if escaped == 'u' {
        let brace = escaped_at + width;
        if brace >= input.len() || input.as_bytes()[brace] != b'{' {
            return Err(LiteralError {
                offset: slash,
                message: "invalid unicode escape; expected '\\u{...}'",
            });
        }
        let digits_start = brace + 1;
        let mut cursor = digits_start;
        while cursor < input.len() && input.as_bytes()[cursor].is_ascii_hexdigit() {
            cursor += 1;
        }
        if cursor >= input.len() || input.as_bytes()[cursor] != b'}' {
            return Err(LiteralError {
                offset: slash,
                message: "unterminated unicode escape",
            });
        }
        let hex = &input[digits_start..cursor];
        if hex.is_empty() || hex.len() > 6 {
            return Err(LiteralError {
                offset: slash,
                message: "unicode escape must contain 1 to 6 hex digits",
            });
        }
        let Ok(codepoint) = u32::from_str_radix(hex, 16) else {
            return Err(LiteralError {
                offset: slash,
                message: "invalid unicode escape",
            });
        };
        if char::from_u32(codepoint).is_none() {
            return Err(LiteralError {
                offset: slash,
                message: "invalid unicode scalar value",
            });
        }
        return Ok(cursor + 1);
    }
    Err(LiteralError {
        offset: slash,
        message: "unknown escape sequence",
    })
}

fn next(input: &str, offset: usize) -> (char, usize) {
    let character = input[offset..]
        .chars()
        .next()
        .expect("offset is inside source text");
    (character, character.len_utf8())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_supported_literal_forms() {
        assert_eq!(scan_string("\"hello\" rest", 0, false), Ok(7));
        assert_eq!(scan_string("\"\"\"a\nb\"\"\"", 0, false), Ok(9));
        assert_eq!(scan_string("f\"value {x}\"", 0, true), Ok(12));
        assert_eq!(scan_character("'\\x41'", 0), Ok(6));
    }

    #[test]
    fn rejects_unknown_escapes() {
        assert_eq!(
            scan_string("\"bad \\q\"", 0, false).unwrap_err().message,
            "unknown escape sequence"
        );
    }
}
