use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use lpp_common::{FileId, SourceMap};
use lpp_driver::fmt::format_source;
use lpp_driver::frontend_eval::check_in_memory;
use lpp_frontend::{ItemKind, ParsedModule, parse_source};

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
    let mut ast_cache: HashMap<String, ParsedModule> = HashMap::new();

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
                            "version": env!("CARGO_PKG_VERSION")
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
                if let Some(uri) = req
                    .params
                    .get("textDocument")
                    .and_then(|t| t.get("uri"))
                    .and_then(|u| u.as_str())
                {
                    if let Some(text) = req
                        .params
                        .get("textDocument")
                        .and_then(|t| t.get("text"))
                        .and_then(|s| s.as_str())
                    {
                        documents.insert(uri.to_string(), text.to_string());
                        process_and_publish_diagnostics(&mut writer, uri, text, &mut ast_cache)?;
                    }
                }
            }
            "textDocument/didChange" => {
                if let Some(uri) = req
                    .params
                    .get("textDocument")
                    .and_then(|t| t.get("uri"))
                    .and_then(|u| u.as_str())
                {
                    if let Some(changes) =
                        req.params.get("contentChanges").and_then(|c| c.as_array())
                    {
                        if let Some(last_change) = changes
                            .last()
                            .and_then(|c| c.get("text"))
                            .and_then(|s| s.as_str())
                        {
                            documents.insert(uri.to_string(), last_change.to_string());
                            process_and_publish_diagnostics(
                                &mut writer,
                                uri,
                                last_change,
                                &mut ast_cache,
                            )?;
                        }
                    }
                }
            }
            "textDocument/didSave" => {
                if let Some(uri) = req
                    .params
                    .get("textDocument")
                    .and_then(|t| t.get("uri"))
                    .and_then(|u| u.as_str())
                {
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

                if let Some(uri) = req
                    .params
                    .get("textDocument")
                    .and_then(|t| t.get("uri"))
                    .and_then(|u| u.as_str())
                {
                    if let Some(ast) = ast_cache.get(uri) {
                        for item in &ast.items {
                            if let Some(ref name) = item.name {
                                match item.kind {
                                    ItemKind::Function => {
                                        completions.push(json!({
                                            "label": name,
                                            "kind": 3,
                                            "detail": format!("def {}(...)", name)
                                        }));
                                    }
                                    ItemKind::Struct => {
                                        completions.push(json!({
                                            "label": name,
                                            "kind": 22,
                                            "detail": format!("struct {}", name)
                                        }));
                                    }
                                    ItemKind::Enum => {
                                        completions.push(json!({
                                            "label": name,
                                            "kind": 13,
                                            "detail": format!("enum {}", name)
                                        }));
                                    }
                                    _ => {}
                                }
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
                if let Some(uri) = req
                    .params
                    .get("textDocument")
                    .and_then(|t| t.get("uri"))
                    .and_then(|u| u.as_str())
                {
                    if let (Some(ast), Some(doc_text)) = (ast_cache.get(uri), documents.get(uri)) {
                        let mut sources = SourceMap::new();
                        let fid = sources
                            .add_file(uri, doc_text.clone())
                            .unwrap_or(FileId::from_raw(1));
                        let sf = sources.get(fid);
                        for item in &ast.items {
                            if let Some(ref name) = item.name {
                                let kind = match item.kind {
                                    ItemKind::Function => 12,
                                    ItemKind::Struct => 23,
                                    ItemKind::Enum => 10,
                                    ItemKind::Trait => 11,
                                    ItemKind::Impl => 6,
                                    _ => continue,
                                };
                                let (sl, sc) = sf
                                    .and_then(|s| s.line_column(item.span.start).ok())
                                    .unwrap_or((1usize, 1usize));
                                let (el, ec) = sf
                                    .and_then(|s| s.line_column(item.span.end).ok())
                                    .unwrap_or((sl, sc + name.len()));
                                let sl0 = sl.saturating_sub(1);
                                let sc0 = sc.saturating_sub(1);
                                let el0 = el.saturating_sub(1).max(sl0);
                                let ec0 = ec.saturating_sub(1).max(sc0 + name.len());
                                symbols.push(json!({
                                    "name": name,
                                    "kind": kind,
                                    "range": {
                                        "start": { "line": sl0, "character": sc0 },
                                        "end": { "line": el0, "character": ec0 }
                                    },
                                    "selectionRange": {
                                        "start": { "line": sl0, "character": sc0 },
                                        "end": { "line": sl0, "character": sc0 + name.len() }
                                    }
                                }));
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
                let mut hover_val =
                    String::from("### L++ Language Symbol\nSelect or inspect symbols in L++.");
                if let Some(uri) = req
                    .params
                    .get("textDocument")
                    .and_then(|t| t.get("uri"))
                    .and_then(|u| u.as_str())
                {
                    if let Some(doc_text) = documents.get(uri) {
                        if let Some(pos) = req.params.get("position") {
                            let line_idx =
                                pos.get("line").and_then(|l| l.as_u64()).unwrap_or(0) as usize;
                            let char_idx =
                                pos.get("character").and_then(|c| c.as_u64()).unwrap_or(0) as usize;
                            if let Some(line) = doc_text.lines().nth(line_idx) {
                                let word = extract_word_at_col(line, char_idx);
                                if !word.is_empty() {
                                    if let Some(decl_line) = doc_text.lines().find(|l| {
                                        let t = l.trim();
                                        t.starts_with(&format!("def {word}("))
                                            || t.starts_with(&format!("async def {word}("))
                                            || t.starts_with(&format!("unsafe def {word}("))
                                            || t.starts_with(&format!("struct {word}"))
                                            || t.starts_with(&format!("enum {word}"))
                                            || t.starts_with(&format!("trait {word}"))
                                            || t.starts_with(&format!("type {word}"))
                                    }) {
                                        hover_val = format!("```lpp\n{}\n```", decl_line.trim());
                                    }
                                }
                                if hover_val.starts_with("### L++ Language Symbol") {
                                    if line.contains("def ") {
                                        hover_val = "### L++ Function Declaration\nDefines a statically type-checked function.".to_string();
                                    } else if line.contains("struct ") {
                                        hover_val = "### L++ Struct Declaration\nDefines an ARC/stack composite struct type.".to_string();
                                    } else if line.contains("enum ") {
                                        hover_val = "### L++ Enum Declaration\nDefines an algebraic data type with exhaustive pattern matching.".to_string();
                                    } else if line.contains("unsafe") {
                                        hover_val = "### `unsafe` Block / Function\nPermits checked raw pointers, MMIO, and low-level intrinsics.".to_string();
                                    } else if line.contains("VectorI64x") {
                                        hover_val = "### `VectorI64x2` / `VectorI64x4`\nHardware 128-bit / 256-bit integer SIMD vector.".to_string();
                                    }
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
                if let Some(uri) = req
                    .params
                    .get("textDocument")
                    .and_then(|t| t.get("uri"))
                    .and_then(|u| u.as_str())
                {
                    if let Some(doc_text) = documents.get(uri) {
                        if let Some(pos) = req.params.get("position") {
                            let line_idx =
                                pos.get("line").and_then(|l| l.as_u64()).unwrap_or(0) as usize;
                            if let Some(line) = doc_text.lines().nth(line_idx) {
                                // Extract word under position
                                let char_idx =
                                    pos.get("character").and_then(|c| c.as_u64()).unwrap_or(0)
                                        as usize;
                                let word = extract_word_at_col(line, char_idx);
                                if !word.is_empty() {
                                    // Search current document first
                                    for (i, l) in doc_text.lines().enumerate() {
                                        if l.contains(&format!("def {}", word))
                                            || l.contains(&format!("struct {}", word))
                                            || l.contains(&format!("enum {}", word))
                                        {
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
                if let Some(uri) = req
                    .params
                    .get("textDocument")
                    .and_then(|t| t.get("uri"))
                    .and_then(|u| u.as_str())
                {
                    if let Some(text) = documents.get(uri) {
                        let formatted = format_source(text);
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
    if start < end { &line[start..end] } else { "" }
}

fn process_and_publish_diagnostics<W: Write>(
    writer: &mut W,
    uri: &str,
    text: &str,
    ast_cache: &mut HashMap<String, ParsedModule>,
) -> io::Result<()> {
    let mut lsp_diagnostics = Vec::new();
    let mut sources = SourceMap::new();
    let file_id = sources
        .add_file(uri, text.to_string())
        .unwrap_or(FileId::from_raw(1));

    if let Ok(ast) = parse_source(file_id, text) {
        ast_cache.insert(uri.to_string(), ast);
    }

    let check_path = uri_to_path(uri).unwrap_or_else(|| PathBuf::from("buffer.lpp"));
    if let Err(diags) = check_in_memory(text, &check_path) {
        for diag in diags {
            let start_line = diag.line.saturating_sub(1);
            let start_char = diag.col.saturating_sub(1);
            let end_line = diag.end_line.saturating_sub(1).max(start_line);
            let end_char = diag.end_col.saturating_sub(1).max(start_char + 1);
            lsp_diagnostics.push(json!({
                "range": {
                    "start": { "line": start_line, "character": start_char },
                    "end": { "line": end_line, "character": end_char }
                },
                "severity": 1,
                "code": diag.code,
                "source": "lpp-lsp",
                "message": diag.message
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
