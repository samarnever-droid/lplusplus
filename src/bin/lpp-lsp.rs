use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

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
            return Ok(None); // EOF
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break; // Header section ended
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let stdout = io::stdout();
    let mut writer = stdout.lock();

    let mut documents: HashMap<String, String> = HashMap::new();

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
                            "textDocumentSync": 1, // Full document sync
                            "completionProvider": {
                                "triggerCharacters": [".", ":"]
                            },
                            "hoverProvider": true,
                            "definitionProvider": true,
                            "documentSymbolProvider": true
                        },
                        "serverInfo": {
                            "name": "lpp-lsp",
                            "version": "3.4.0"
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
                        publish_diagnostics(&mut writer, uri, text)?;
                    }
                }
            }
            "textDocument/didChange" => {
                if let Some(uri) = req.params.get("textDocument").and_then(|t| t.get("uri")).and_then(|u| u.as_str()) {
                    if let Some(changes) = req.params.get("contentChanges").and_then(|c| c.as_array()) {
                        if let Some(last_change) = changes.last().and_then(|c| c.get("text")).and_then(|s| s.as_str()) {
                            documents.insert(uri.to_string(), last_change.to_string());
                            publish_diagnostics(&mut writer, uri, last_change)?;
                        }
                    }
                }
            }
            "textDocument/didSave" => {
                if let Some(uri) = req.params.get("textDocument").and_then(|t| t.get("uri")).and_then(|u| u.as_str()) {
                    if let Some(text) = documents.get(uri) {
                        publish_diagnostics(&mut writer, uri, text)?;
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
                    json!({ "label": "Float", "kind": 21, "detail": "Double Float Type" }),
                    json!({ "label": "Bool", "kind": 21, "detail": "Boolean Type" }),
                    json!({ "label": "print_str", "kind": 3, "detail": "print_str(s: Str) -> Void" }),
                    json!({ "label": "print", "kind": 3, "detail": "print(v: Any) -> Void" }),
                    json!({ "label": "input", "kind": 3, "detail": "input() -> Str" }),
                    json!({ "label": "read_file", "kind": 3, "detail": "read_file(path: Str) -> Str" }),
                    json!({ "label": "write_file", "kind": 3, "detail": "write_file(path: Str, data: Str) -> Int" }),
                    json!({ "label": "str_concat", "kind": 3, "detail": "str_concat(a: Str, b: Str) -> Str" }),
                    json!({ "label": "int_to_str", "kind": 3, "detail": "int_to_str(val: Int) -> Str" }),
                ];

                if let Some(uri) = req.params.get("textDocument").and_then(|t| t.get("uri")).and_then(|u| u.as_str()) {
                    if let Some(doc_text) = documents.get(uri) {
                        for line in doc_text.lines() {
                            let trimmed = line.trim();
                            if let Some(rest) = trimmed.strip_prefix("def ") {
                                if let Some(name) = rest.split('(').next() {
                                    completions.push(json!({ "label": name.trim(), "kind": 3, "detail": "User Function" }));
                                }
                            } else if let Some(rest) = trimmed.strip_prefix("struct ") {
                                if let Some(name) = rest.split(':').next() {
                                    completions.push(json!({ "label": name.trim(), "kind": 22, "detail": "User Struct" }));
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
            "textDocument/hover" => {
                let mut hover_contents = "L++ Language Reference";
                if let Some(uri) = req.params.get("textDocument").and_then(|t| t.get("uri")).and_then(|u| u.as_str()) {
                    if let Some(pos) = req.params.get("position") {
                        let line_idx = pos.get("line").and_then(|l| l.as_u64()).unwrap_or(0) as usize;
                        if let Some(doc_text) = documents.get(uri) {
                            if let Some(line) = doc_text.lines().nth(line_idx) {
                                if line.contains("def ") {
                                    hover_contents = "### L++ Function\nDefines a top-level or method function frame.";
                                } else if line.contains("struct ") {
                                    hover_contents = "### L++ Struct\nDefines a stack/heap data structure with MIR escape-analysis optimizations.";
                                } else if line.contains("print_str") {
                                    hover_contents = "### `print_str(s: Str)`\nOutputs a raw string to standard output.";
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
                            "value": hover_contents
                        }
                    })),
                    error: None,
                };
                send_lsp_response(&mut writer, &response)?;
            }
            _ => {}
        }
    }

    Ok(())
}

fn publish_diagnostics<W: Write>(writer: &mut W, uri: &str, text: &str) -> io::Result<()> {
    let mut diagnostics = Vec::new();

    // Check for syntax errors by scanning document lines
    for (line_idx, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("if ") && !trimmed.ends_with(':') {
            diagnostics.push(json!({
                "range": {
                    "start": { "line": line_idx, "character": 0 },
                    "end": { "line": line_idx, "character": line.len() }
                },
                "severity": 1,
                "code": "E0002",
                "source": "lpp-lsp",
                "message": "if statement condition must end with a colon ':'"
            }));
        }
    }

    send_lsp_notification(
        writer,
        "textDocument/publishDiagnostics",
        json!({
            "uri": uri,
            "diagnostics": diagnostics
        }),
    )
}
