use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use lpp::diagnostics;
use lpp::lexer::Lexer;
use lpp::parser::Parser;
use lpp::semantic::Resolver;
use lpp::typecheck::TypeChecker;
use lpp::ast::{Program, TopLevel};

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
}

fn read_lsp_message<R: BufRead>(reader: &mut R) -> Result<Option<String>, String> {
    let mut content_length: Option<usize> = None;
    let mut line = String::new();

    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line).map_err(|e| e.to_string())?;
        if bytes_read == 0 {
            return Ok(None);
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some(val) = trimmed.strip_prefix("Content-Length:") {
            content_length = val.trim().parse::<usize>().ok();
        }
    }

    let length = content_length.ok_or_else(|| "Missing Content-Length header".to_string())?;
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).map_err(|e| e.to_string())?;

    String::from_utf8(body).map(Some).map_err(|e| e.to_string())
}

fn send_lsp_response<W: Write>(writer: &mut W, response: &JsonRpcResponse) -> io::Result<()> {
    let body = serde_json::to_string(response)?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    writer.write_all(header.as_bytes())?;
    writer.write_all(body.as_bytes())?;
    writer.flush()
}

fn send_lsp_notification<W: Write>(writer: &mut W, method: &str, params: Value) -> io::Result<()> {
    let notification = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params
    });
    let body = serde_json::to_string(&notification)?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    writer.write_all(header.as_bytes())?;
    writer.write_all(body.as_bytes())?;
    writer.flush()
}

#[allow(dead_code)]
fn uri_to_path(uri: &str) -> Option<PathBuf> {
    if let Some(stripped) = uri.strip_prefix("file:///") {
        #[cfg(windows)]
        return Some(PathBuf::from(stripped.replace('/', "\\")));
        #[cfg(not(windows))]
        return Some(PathBuf::from(format!("/{}", stripped)));
    } else if let Some(stripped) = uri.strip_prefix("file://") {
        return Some(PathBuf::from(stripped));
    }
    None
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let stdout = io::stdout();
    let mut writer = stdout.lock();

    let mut documents: HashMap<String, String> = HashMap::new();
    let mut ast_cache: HashMap<String, Program> = HashMap::new();

    while let Ok(Some(msg_str)) = read_lsp_message(&mut reader) {
        let req: JsonRpcRequest = match serde_json::from_str(&msg_str) {
            Ok(r) => r,
            Err(_) => continue,
        };

        match req.method.as_str() {
            "initialize" => {
                let response = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: req.id.unwrap_or(Value::Null),
                    result: Some(json!({
                        "capabilities": {
                            "textDocumentSync": 1,
                            "completionProvider": {
                                "triggerCharacters": [".", ":", "("]
                            },
                            "hoverProvider": true,
                            "definitionProvider": true,
                            "documentSymbolProvider": true,
                            "documentFormattingProvider": true,
                            "semanticTokensProvider": {
                                "legend": {
                                    "tokenTypes": ["keyword", "type", "function", "variable", "string", "number", "operator", "comment", "parameter", "property"],
                                    "tokenModifiers": ["declaration", "definition", "readonly"]
                                },
                                "full": true
                            }
                        },
                        "serverInfo": {
                            "name": "lpp-lsp",
                            "version": "4.6.0"
                        }
                    })),
                    error: None,
                };
                send_lsp_response(&mut writer, &response)?;
            }
            "initialized" => {}
            "shutdown" => {
                let response = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: req.id.unwrap_or(Value::Null),
                    result: Some(Value::Null),
                    error: None,
                };
                send_lsp_response(&mut writer, &response)?;
            }
            "exit" => break,
            "textDocument/didOpen" => {
                if let Some(uri) = req.params.get("textDocument").and_then(|t| t.get("uri")).and_then(|u| u.as_str()) {
                    if let Some(text) = req.params.get("textDocument").and_then(|t| t.get("text")).and_then(|s| s.as_str()) {
                        documents.insert(uri.to_string(), text.to_string());
                        process_and_publish_diagnostics(&mut writer, uri, text, &mut ast_cache)?;
                    }
                }
            }
            "textDocument/didChange" => {
                if let Some(uri) = req.params.get("textDocument").and_then(|t| t.get("uri")).and_then(|u| u.as_str()) {
                    if let Some(changes) = req.params.get("contentChanges").and_then(|c| c.as_array()) {
                        if let Some(last_change) = changes.last().and_then(|c| c.get("text")).and_then(|s| s.as_str()) {
                            documents.insert(uri.to_string(), last_change.to_string());
                            process_and_publish_diagnostics(&mut writer, uri, last_change, &mut ast_cache)?;
                        }
                    }
                }
            }
            "textDocument/didSave" => {
                if let Some(uri) = req.params.get("textDocument").and_then(|t| t.get("uri")).and_then(|u| u.as_str()) {
                    if let Some(text) = documents.get(uri) {
                        process_and_publish_diagnostics(&mut writer, uri, text, &mut ast_cache)?;
                    }
                }
            }
            "textDocument/completion" => {
                let mut completions = vec![
                    json!({ "label": "def", "kind": 14, "detail": "Function Definition" }),
                    json!({ "label": "struct", "kind": 22, "detail": "Struct Definition" }),
                    json!({ "label": "enum", "kind": 13, "detail": "Enum Definition" }),
                    json!({ "label": "mut", "kind": 14, "detail": "Mutable Binding" }),
                    json!({ "label": "if", "kind": 14, "detail": "If Branch" }),
                    json!({ "label": "while", "kind": 14, "detail": "While Loop" }),
                    json!({ "label": "return", "kind": 14, "detail": "Return Statement" }),
                    json!({ "label": "Int", "kind": 21, "detail": "64-bit Integer Type" }),
                    json!({ "label": "Str", "kind": 21, "detail": "String Type" }),
                    json!({ "label": "Bool", "kind": 21, "detail": "Boolean Type" }),
                    json!({ "label": "Void", "kind": 21, "detail": "Void Return Type" }),
                    json!({ "label": "CPtr", "kind": 21, "detail": "Safe Checked C-Pointer" }),
                    json!({ "label": "CMemory", "kind": 21, "detail": "Memory Heap Context" }),
                    json!({ "label": "Buffer", "kind": 21, "detail": "Binary Byte Buffer" }),
                ];

                if let Some(uri) = req.params.get("textDocument").and_then(|t| t.get("uri")).and_then(|u| u.as_str()) {
                    if let Some(ast) = ast_cache.get(uri) {
                        for decl in &ast.declarations {
                            match decl {
                                TopLevel::Function(f) => {
                                    completions.push(json!({
                                        "label": f.name,
                                        "kind": 3,
                                        "detail": format!("def {}(...) -> {:?}", f.name, f.return_type)
                                    }));
                                }
                                TopLevel::Struct(s) => {
                                    completions.push(json!({
                                        "label": s.name,
                                        "kind": 22,
                                        "detail": format!("struct {}", s.name)
                                    }));
                                }
                                TopLevel::Enum(e) => {
                                    completions.push(json!({
                                        "label": e.name,
                                        "kind": 13,
                                        "detail": format!("enum {}", e.name)
                                    }));
                                }
                                _ => {}
                            }
                        }
                    }
                }

                let response = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: req.id.unwrap_or(Value::Null),
                    result: Some(json!(completions)),
                    error: None,
                };
                send_lsp_response(&mut writer, &response)?;
            }
            "textDocument/documentSymbol" => {
                let mut symbols = Vec::new();
                if let Some(uri) = req.params.get("textDocument").and_then(|t| t.get("uri")).and_then(|u| u.as_str()) {
                    if let Some(ast) = ast_cache.get(uri) {
                        for (idx, decl) in ast.declarations.iter().enumerate() {
                            match decl {
                                TopLevel::Function(f) => {
                                    symbols.push(json!({
                                        "name": f.name,
                                        "kind": 12, // Function
                                        "range": {
                                            "start": { "line": idx * 2, "character": 0 },
                                            "end": { "line": idx * 2 + 1, "character": 0 }
                                        },
                                        "selectionRange": {
                                            "start": { "line": idx * 2, "character": 0 },
                                            "end": { "line": idx * 2, "character": f.name.len() }
                                        }
                                    }));
                                }
                                TopLevel::Struct(s) => {
                                    symbols.push(json!({
                                        "name": s.name,
                                        "kind": 23, // Struct
                                        "range": {
                                            "start": { "line": idx * 2, "character": 0 },
                                            "end": { "line": idx * 2 + 1, "character": 0 }
                                        },
                                        "selectionRange": {
                                            "start": { "line": idx * 2, "character": 0 },
                                            "end": { "line": idx * 2, "character": s.name.len() }
                                        }
                                    }));
                                }
                                TopLevel::Enum(e) => {
                                    symbols.push(json!({
                                        "name": e.name,
                                        "kind": 10, // Enum
                                        "range": {
                                            "start": { "line": idx * 2, "character": 0 },
                                            "end": { "line": idx * 2 + 1, "character": 0 }
                                        },
                                        "selectionRange": {
                                            "start": { "line": idx * 2, "character": 0 },
                                            "end": { "line": idx * 2, "character": e.name.len() }
                                        }
                                    }));
                                }
                                _ => {}
                            }
                        }
                    }
                }

                let response = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: req.id.unwrap_or(Value::Null),
                    result: Some(json!(symbols)),
                    error: None,
                };
                send_lsp_response(&mut writer, &response)?;
            }
            "textDocument/hover" => {
                let mut hover_val = "### L++ Language Symbol\nSelect or inspect symbols in L++.";
                if let Some(uri) = req.params.get("textDocument").and_then(|t| t.get("uri")).and_then(|u| u.as_str()) {
                    if let Some(doc_text) = documents.get(uri) {
                        if let Some(pos) = req.params.get("position") {
                            let line_idx = pos.get("line").and_then(|l| l.as_u64()).unwrap_or(0) as usize;
                            if let Some(line) = doc_text.lines().nth(line_idx) {
                                if line.contains("def ") {
                                    hover_val = "### L++ Function Declaration\nDefines a type-checked function frame.";
                                } else if line.contains("struct ") {
                                    hover_val = "### L++ Struct Declaration\nDefines a structured value type with zero-overhead MIR escape analysis.";
                                } else if line.contains("CPtr") {
                                    hover_val = "### `CPtr` (Safe Checked Pointer)\nFat C-pointer with generation ID, bounds checking, and subobject security.";
                                } else if line.contains("CMemory") {
                                    hover_val = "### `CMemory` (Isolated Heap Context)\nChecked memory allocator tracking active allocations and UAF guards.";
                                } else if line.contains("Buffer") {
                                    hover_val = "### `Buffer` (Binary Byte Buffer)\nStructured binary reader/writer supporting endianness and string conversions.";
                                }
                            }
                        }
                    }
                }

                let response = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: req.id.unwrap_or(Value::Null),
                    result: Some(json!({
                        "contents": {
                            "kind": "markdown",
                            "value": hover_val
                        }
                    })),
                    error: None,
                };
                send_lsp_response(&mut writer, &response)?;
            }
            "textDocument/definition" => {
                let mut locations = Vec::new();
                if let Some(uri) = req.params.get("textDocument").and_then(|t| t.get("uri")).and_then(|u| u.as_str()) {
                    if let Some(doc_text) = documents.get(uri) {
                        if let Some(pos) = req.params.get("position") {
                            let line_idx = pos.get("line").and_then(|l| l.as_u64()).unwrap_or(0) as usize;
                            if let Some(line) = doc_text.lines().nth(line_idx) {
                                // Extract word under position
                                let char_idx = pos.get("character").and_then(|c| c.as_u64()).unwrap_or(0) as usize;
                                let word = extract_word_at_col(line, char_idx);
                                if !word.is_empty() {
                                    // Search current document first
                                    for (i, l) in doc_text.lines().enumerate() {
                                        if l.contains(&format!("def {}", word)) || l.contains(&format!("struct {}", word)) || l.contains(&format!("enum {}", word)) {
                                            locations.push(json!({
                                                "uri": uri,
                                                "range": {
                                                    "start": { "line": i, "character": 0 },
                                                    "end": { "line": i, "character": l.len() }
                                                }
                                            }));
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                let response = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: req.id.unwrap_or(Value::Null),
                    result: Some(json!(locations)),
                    error: None,
                };
                send_lsp_response(&mut writer, &response)?;
            }
            "textDocument/formatting" => {
                let mut edits = Vec::new();
                if let Some(uri) = req.params.get("textDocument").and_then(|t| t.get("uri")).and_then(|u| u.as_str()) {
                    if let Some(text) = documents.get(uri) {
                        let formatted = format_lpp_code(text);
                        let line_count = text.lines().count();
                        edits.push(json!({
                            "range": {
                                "start": { "line": 0, "character": 0 },
                                "end": { "line": line_count + 1, "character": 0 }
                            },
                            "newText": formatted
                        }));
                    }
                }

                let response = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: req.id.unwrap_or(Value::Null),
                    result: Some(json!(edits)),
                    error: None,
                };
                send_lsp_response(&mut writer, &response)?;
            }
            _ => {}
        }
    }

    Ok(())
}

fn extract_word_at_col(line: &str, col: usize) -> &str {
    let bytes = line.as_bytes();
    if col >= bytes.len() {
        return "";
    }
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut start = col;
    while start > 0 && is_ident(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = col;
    while end < bytes.len() && is_ident(bytes[end]) {
        end += 1;
    }
    if start < end {
        &line[start..end]
    } else {
        ""
    }
}

fn format_lpp_code(text: &str) -> String {
    let mut result = String::new();
    let mut indent_level: usize = 0;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            result.push('\n');
            continue;
        }

        if trimmed.starts_with("else") || trimmed.starts_with("elif") {
            let current_indent = if indent_level > 0 { indent_level - 1 } else { 0 };
            result.push_str(&" ".repeat(current_indent * 4));
        } else {
            result.push_str(&" ".repeat(indent_level * 4));
        }

        result.push_str(trimmed);
        result.push('\n');

        if trimmed.ends_with(':') {
            indent_level += 1;
        } else if trimmed.starts_with("return") || trimmed.starts_with("break") || trimmed.starts_with("continue") {
            if indent_level > 0 {
                indent_level -= 1;
            }
        }
    }

    result
}

fn process_and_publish_diagnostics<W: Write>(
    writer: &mut W,
    uri: &str,
    text: &str,
    ast_cache: &mut HashMap<String, Program>,
) -> io::Result<()> {
    let mut lsp_diagnostics = Vec::new();

    let mut lexer = Lexer::new(text);
    match lexer.tokenize() {
        Ok(tokens) => {
            let mut parser = Parser::new(tokens);
            match parser.parse() {
                Ok(mut ast) => {
                    let mut resolver = Resolver::new();
                    if let Err(e) = resolver.resolve_program(&mut ast) {
                        let (line, col, msg) = diagnostics::parse_line_col_message_with_source(&e, text);
                        lsp_diagnostics.push(json!({
                            "range": {
                                "start": { "line": if line > 0 { line - 1 } else { 0 }, "character": col },
                                "end": { "line": if line > 0 { line - 1 } else { 0 }, "character": col + 5 }
                            },
                            "severity": 1,
                            "code": "SemanticError",
                            "source": "lpp-lsp",
                            "message": msg
                        }));
                    } else {
                        let mut type_checker = TypeChecker::new(&mut resolver.table);
                        if let Err(e) = type_checker.check_program(&ast) {
                            let (line, col, msg) = diagnostics::parse_line_col_message_with_source(&e, text);
                            lsp_diagnostics.push(json!({
                                "range": {
                                    "start": { "line": if line > 0 { line - 1 } else { 0 }, "character": col },
                                    "end": { "line": if line > 0 { line - 1 } else { 0 }, "character": col + 5 }
                                },
                                "severity": 1,
                                "code": "TypeError",
                                "source": "lpp-lsp",
                                "message": msg
                            }));
                        }
                    }
                    ast_cache.insert(uri.to_string(), ast);
                }
                Err(e) => {
                    let (line, col, msg) = diagnostics::parse_line_col_message_with_source(&e, text);
                    lsp_diagnostics.push(json!({
                        "range": {
                            "start": { "line": if line > 0 { line - 1 } else { 0 }, "character": col },
                            "end": { "line": if line > 0 { line - 1 } else { 0 }, "character": col + 5 }
                        },
                        "severity": 1,
                        "code": "SyntaxError",
                        "source": "lpp-lsp",
                        "message": msg
                    }));
                }
            }
        }
        Err(e) => {
            let (line, col, msg) = diagnostics::parse_line_col_message_with_source(&e, text);
            lsp_diagnostics.push(json!({
                "range": {
                    "start": { "line": if line > 0 { line - 1 } else { 0 }, "character": col },
                    "end": { "line": if line > 0 { line - 1 } else { 0 }, "character": col + 5 }
                },
                "severity": 1,
                "code": "LexerError",
                "source": "lpp-lsp",
                "message": msg
            }));
        }
    }

    send_lsp_notification(
        writer,
        "textDocument/publishDiagnostics",
        json!({
            "uri": uri,
            "diagnostics": lsp_diagnostics
        }),
    )
}
