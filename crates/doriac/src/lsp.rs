use std::collections::HashMap;
use std::io::{self, BufRead, Write};

use serde_json::{json, Value};

use crate::diagnostics::Diagnostic;
use crate::lexer::TokenKind;
use crate::source::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LspPosition {
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone)]
struct Document {
    text: String,
    version: Option<i64>,
}

#[derive(Default)]
struct Server {
    documents: HashMap<String, Document>,
}

pub fn run_stdio() -> Result<(), String> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = io::BufReader::new(stdin.lock());
    let mut writer = io::BufWriter::new(stdout.lock());
    let mut server = Server::default();

    while let Some(body) = read_message(&mut reader)? {
        let message = serde_json::from_slice::<Value>(&body)
            .map_err(|error| format!("failed to parse LSP message: {error}"))?;
        if !server.handle_message(message, &mut writer)? {
            break;
        }
        writer
            .flush()
            .map_err(|error| format!("failed to flush LSP response: {error}"))?;
    }

    Ok(())
}

pub fn byte_offset_to_position(text: &str, offset: usize) -> LspPosition {
    let clamped = offset.min(text.len());
    let mut line = 0_u32;
    let mut character = 0_u32;

    for (byte_index, ch) in text.char_indices() {
        if byte_index >= clamped {
            break;
        }

        if ch == '\n' {
            line += 1;
            character = 0;
        } else {
            character += ch.len_utf16() as u32;
        }
    }

    LspPosition { line, character }
}

pub fn position_to_byte_offset(text: &str, line: u32, character: u32) -> usize {
    let mut current_line = 0_u32;
    let mut current_character = 0_u32;

    for (byte_index, ch) in text.char_indices() {
        if current_line == line && current_character >= character {
            return byte_index;
        }

        if ch == '\n' {
            if current_line == line {
                return byte_index;
            }
            current_line += 1;
            current_character = 0;
            continue;
        }

        if current_line == line {
            let next_character = current_character + ch.len_utf16() as u32;
            if next_character > character {
                return byte_index;
            }
            current_character = next_character;
        }
    }

    text.len()
}

pub fn diagnostics_for_document(uri: &str, text: &str) -> Vec<Value> {
    match crate::check_source(uri.to_string(), text.to_string()) {
        Ok(_) => Vec::new(),
        Err(diagnostics) => diagnostics
            .iter()
            .map(|diagnostic| diagnostic_to_lsp(text, diagnostic))
            .collect(),
    }
}

impl Server {
    fn handle_message<W: Write>(&mut self, message: Value, writer: &mut W) -> Result<bool, String> {
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return Ok(true);
        };

        let id = message.get("id").cloned();
        match method {
            "initialize" => {
                if let Some(id) = id {
                    send_response(writer, id, initialize_result())?;
                }
            }
            "initialized" => {}
            "shutdown" => {
                if let Some(id) = id {
                    send_response(writer, id, Value::Null)?;
                }
            }
            "exit" => return Ok(false),
            "textDocument/didOpen" => self.did_open(message.get("params"), writer)?,
            "textDocument/didChange" => self.did_change(message.get("params"), writer)?,
            "textDocument/didSave" => self.did_save(message.get("params"), writer)?,
            "textDocument/didClose" => self.did_close(message.get("params"), writer)?,
            "textDocument/completion" => {
                if let Some(id) = id {
                    send_response(writer, id, completion_items())?;
                }
            }
            "textDocument/hover" => {
                if let Some(id) = id {
                    let hover = self.hover(message.get("params"));
                    send_response(writer, id, hover.unwrap_or(Value::Null))?;
                }
            }
            _ => {
                if let Some(id) = id {
                    send_error(
                        writer,
                        id,
                        -32601,
                        format!("method `{method}` is not supported"),
                    )?;
                }
            }
        }

        Ok(true)
    }

    fn did_open<W: Write>(&mut self, params: Option<&Value>, writer: &mut W) -> Result<(), String> {
        let Some(text_document) = params.and_then(|params| params.get("textDocument")) else {
            return Ok(());
        };
        let Some(uri) = text_document.get("uri").and_then(Value::as_str) else {
            return Ok(());
        };
        let Some(text) = text_document.get("text").and_then(Value::as_str) else {
            return Ok(());
        };
        let version = text_document.get("version").and_then(Value::as_i64);

        self.documents.insert(
            uri.to_string(),
            Document {
                text: text.to_string(),
                version,
            },
        );
        self.publish_diagnostics(uri, writer)
    }

    fn did_change<W: Write>(
        &mut self,
        params: Option<&Value>,
        writer: &mut W,
    ) -> Result<(), String> {
        let Some(params) = params else {
            return Ok(());
        };
        let Some(text_document) = params.get("textDocument") else {
            return Ok(());
        };
        let Some(uri) = text_document.get("uri").and_then(Value::as_str) else {
            return Ok(());
        };
        let version = text_document.get("version").and_then(Value::as_i64);
        let Some(changes) = params.get("contentChanges").and_then(Value::as_array) else {
            return Ok(());
        };
        let Some(text) = changes
            .last()
            .and_then(|change| change.get("text"))
            .and_then(Value::as_str)
        else {
            return Ok(());
        };

        self.documents.insert(
            uri.to_string(),
            Document {
                text: text.to_string(),
                version,
            },
        );
        self.publish_diagnostics(uri, writer)
    }

    fn did_save<W: Write>(&mut self, params: Option<&Value>, writer: &mut W) -> Result<(), String> {
        let Some(text_document) = params.and_then(|params| params.get("textDocument")) else {
            return Ok(());
        };
        let Some(uri) = text_document.get("uri").and_then(Value::as_str) else {
            return Ok(());
        };

        if let Some(text) = params
            .and_then(|params| params.get("text"))
            .and_then(Value::as_str)
        {
            let version = self
                .documents
                .get(uri)
                .and_then(|document| document.version);
            self.documents.insert(
                uri.to_string(),
                Document {
                    text: text.to_string(),
                    version,
                },
            );
        }

        self.publish_diagnostics(uri, writer)
    }

    fn did_close<W: Write>(
        &mut self,
        params: Option<&Value>,
        writer: &mut W,
    ) -> Result<(), String> {
        let Some(text_document) = params.and_then(|params| params.get("textDocument")) else {
            return Ok(());
        };
        let Some(uri) = text_document.get("uri").and_then(Value::as_str) else {
            return Ok(());
        };

        self.documents.remove(uri);
        send_notification(
            writer,
            "textDocument/publishDiagnostics",
            json!({
                "uri": uri,
                "diagnostics": [],
            }),
        )
    }

    fn publish_diagnostics<W: Write>(&self, uri: &str, writer: &mut W) -> Result<(), String> {
        let Some(document) = self.documents.get(uri) else {
            return Ok(());
        };

        let mut params = json!({
            "uri": uri,
            "diagnostics": diagnostics_for_document(uri, &document.text),
        });

        if let Some(version) = document.version {
            params["version"] = json!(version);
        }

        send_notification(writer, "textDocument/publishDiagnostics", params)
    }

    fn hover(&self, params: Option<&Value>) -> Option<Value> {
        let params = params?;
        let uri = params
            .get("textDocument")
            .and_then(|text_document| text_document.get("uri"))
            .and_then(Value::as_str)?;
        let line = params
            .get("position")
            .and_then(|position| position.get("line"))
            .and_then(Value::as_u64)? as u32;
        let character = params
            .get("position")
            .and_then(|position| position.get("character"))
            .and_then(Value::as_u64)? as u32;
        let document = self.documents.get(uri)?;
        let offset = position_to_byte_offset(&document.text, line, character);
        hover_at_offset(&document.text, offset)
    }
}

fn initialize_result() -> Value {
    json!({
        "capabilities": {
            "textDocumentSync": {
                "openClose": true,
                "change": 1,
                "save": {
                    "includeText": false
                }
            },
            "completionProvider": {
                "triggerCharacters": ["$", ">", ":"]
            },
            "hoverProvider": true
        },
        "serverInfo": {
            "name": "doria-lsp",
            "version": env!("CARGO_PKG_VERSION")
        }
    })
}

fn completion_items() -> Value {
    let keywords = [
        "class",
        "interface",
        "trait",
        "extends",
        "implements",
        "function",
        "let",
        "writable",
        "readonly",
        "internal",
        "return",
        "echo",
        "new",
        "namespace",
        "use",
        "uses",
        "as",
        "include",
        "declare",
        "foreach",
        "if",
        "else",
        "while",
        "for",
        "break",
        "continue",
        "static",
        "not",
        "and",
        "or",
        "xor",
        "true",
        "false",
        "null",
        "throw",
        "throws",
        "try",
        "catch",
        "finally",
        "when",
        "given",
        "enum",
        "case",
        "match",
        "async",
        "await",
        "unsafe",
        "extern",
        "open",
        "override",
        "with",
        "take",
    ];
    let types = [
        "void",
        "int",
        "float",
        "string",
        "bool",
        "mixed",
        "array",
        "List",
        "Dictionary",
        "Set",
    ];
    let reserved_types = ["resource"];

    let mut items = Vec::new();
    items.extend(keywords.into_iter().map(|keyword| {
        json!({
            "label": keyword,
            "kind": 14,
            "detail": "Doria keyword",
        })
    }));
    items.extend(types.into_iter().map(|ty| {
        json!({
            "label": ty,
            "kind": 25,
            "detail": "Doria type",
        })
    }));
    items.extend(reserved_types.into_iter().map(|ty| {
        json!({
            "label": ty,
            "kind": 25,
            "detail": "Reserved Doria type name",
        })
    }));

    json!({
        "isIncomplete": false,
        "items": items,
    })
}

fn hover_at_offset(text: &str, offset: usize) -> Option<Value> {
    let tokens = crate::lex_source("<lsp>", text.to_string()).ok()?;
    let token = tokens.into_iter().find(|token| {
        !matches!(token.kind, TokenKind::Eof)
            && token.span.start <= offset
            && offset <= token.span.end
    })?;
    let description = hover_description(&token.kind)?;

    Some(json!({
        "contents": {
            "kind": "markdown",
            "value": description,
        },
        "range": span_to_range(text, token.span),
    }))
}

fn hover_description(kind: &TokenKind) -> Option<&'static str> {
    match kind {
        TokenKind::Class => Some("Declares a Doria class."),
        TokenKind::Function => Some("Declares a function or method."),
        TokenKind::Let => Some("Declares a local binding with an inferred type."),
        TokenKind::Writable => Some("Marks a binding, property, parameter, or method receiver as mutable."),
        TokenKind::Internal => Some("Marks a class member as hidden from the external object surface."),
        TokenKind::Readonly => Some("Reserved for explicit readonly syntax."),
        TokenKind::Return => Some("Returns a value from the current function."),
        TokenKind::Echo => Some("Emits a value through the current backend."),
        TokenKind::New => Some("Constructs an instance of a class."),
        TokenKind::Foreach => Some("Iterates over a list or dictionary value."),
        TokenKind::As => Some("Separates a `foreach` iterable from its binding."),
        TokenKind::Static => Some("Reserved for static members and calls."),
        TokenKind::Not => Some("Boolean NOT operator; exact synonym for `!`."),
        TokenKind::And => Some("Boolean AND operator; exact synonym for `&&`."),
        TokenKind::Or => Some("Boolean OR operator; exact synonym for `||`."),
        TokenKind::Xor => Some("Bool-only exclusive OR operator."),
        TokenKind::Void => Some("The `void` return type."),
        TokenKind::IntType => Some("The `int` primitive type."),
        TokenKind::FloatType => Some("The `float` primitive type."),
        TokenKind::StringType => Some("The `string` primitive type."),
        TokenKind::BoolType => Some("The `bool` primitive type."),
        TokenKind::ArrayType => Some("PHP-compatible array type; prefer `List<T>`, `Dictionary<K, V>`, or `Set<T>` in Doria APIs."),
        TokenKind::True | TokenKind::False => Some("Boolean literal."),
        TokenKind::Null => Some("Null literal. Nullable values are spelled `?T`; `null` is not a type name."),
        TokenKind::Reserved(_) => Some("Reserved for future Doria syntax."),
        TokenKind::Identifier(name) => match name.as_str() {
            "List" => Some("Ordered collection alias: `List<T>`."),
            "Dictionary" => Some("Key-value collection alias: `Dictionary<K, V>`."),
            "Set" => Some("Unique-value collection alias: `Set<T>`."),
            "mixed" => Some("Dynamic boundary type. Narrow with `is` or `match` before using it."),
            "resource" => Some("Reserved for future PHP interop; not a usable core type."),
            _ => None,
        },
        TokenKind::Variable(_) => Some("Doria variable. Variables must be declared before use."),
        _ => None,
    }
}

fn diagnostic_to_lsp(text: &str, diagnostic: &Diagnostic) -> Value {
    let message = if let Some(help) = &diagnostic.help {
        format!("{}\nhelp: {help}", diagnostic.message)
    } else {
        diagnostic.message.clone()
    };

    json!({
        "range": span_to_range(text, diagnostic.span),
        "severity": 1,
        "code": diagnostic.code,
        "source": "doriac",
        "message": message,
    })
}

fn span_to_range(text: &str, span: Span) -> Value {
    let start = byte_offset_to_position(text, span.start);
    let end = byte_offset_to_position(text, span.end);
    json!({
        "start": {
            "line": start.line,
            "character": start.character,
        },
        "end": {
            "line": end.line,
            "character": end.character,
        },
    })
}

fn read_message<R: BufRead>(reader: &mut R) -> Result<Option<Vec<u8>>, String> {
    let mut content_length = None::<usize>;

    loop {
        let mut line = String::new();
        let bytes_read = reader
            .read_line(&mut line)
            .map_err(|error| format!("failed to read LSP header: {error}"))?;

        if bytes_read == 0 {
            return if content_length.is_some() {
                Err("unexpected EOF while reading LSP headers".to_string())
            } else {
                Ok(None)
            };
        }

        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }

        if trimmed.to_ascii_lowercase().starts_with("content-length:") {
            let (_, value) = trimmed
                .split_once(':')
                .ok_or_else(|| "malformed Content-Length header".to_string())?;
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|error| format!("invalid Content-Length header: {error}"))?,
            );
        }
    }

    let length = content_length.ok_or_else(|| "missing Content-Length header".to_string())?;
    let mut body = vec![0_u8; length];
    reader
        .read_exact(&mut body)
        .map_err(|error| format!("failed to read LSP body: {error}"))?;
    Ok(Some(body))
}

fn send_response<W: Write>(writer: &mut W, id: Value, result: Value) -> Result<(), String> {
    send_message(
        writer,
        &json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        }),
    )
}

fn send_error<W: Write>(
    writer: &mut W,
    id: Value,
    code: i64,
    message: String,
) -> Result<(), String> {
    send_message(
        writer,
        &json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": code,
                "message": message,
            },
        }),
    )
}

fn send_notification<W: Write>(writer: &mut W, method: &str, params: Value) -> Result<(), String> {
    send_message(
        writer,
        &json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }),
    )
}

fn send_message<W: Write>(writer: &mut W, message: &Value) -> Result<(), String> {
    let body = serde_json::to_vec(message)
        .map_err(|error| format!("failed to encode LSP message: {error}"))?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())
        .map_err(|error| format!("failed to write LSP header: {error}"))?;
    writer
        .write_all(&body)
        .map_err(|error| format!("failed to write LSP body: {error}"))
}
#[cfg(test)]
mod tests {
    use super::*;

    fn completion_labels() -> Vec<String> {
        completion_items()["items"]
            .as_array()
            .expect("completion items should be an array")
            .iter()
            .map(|item| {
                item["label"]
                    .as_str()
                    .expect("completion item labels should be strings")
                    .to_string()
            })
            .collect()
    }

    fn completion_detail(label: &str) -> Option<String> {
        completion_items()["items"]
            .as_array()
            .expect("completion items should be an array")
            .iter()
            .find(|item| item["label"].as_str() == Some(label))
            .and_then(|item| item["detail"].as_str())
            .map(ToOwned::to_owned)
    }

    #[test]
    fn completions_do_not_offer_unsupported_future_types() {
        let labels = completion_labels();
        for unsupported in [
            "int8",
            "int16",
            "int32",
            "int64",
            "uint8",
            "uint16",
            "uint32",
            "uint64",
            "float32",
            "float64",
            "never",
            "Shared",
            "Weak",
            "SharedMut",
            "Sendable",
            "Shareable",
            "Ptr",
            "MutPtr",
            "Bytes",
        ] {
            assert!(
                !labels.iter().any(|label| label == unsupported),
                "unsupported future type `{unsupported}` must not be an active LSP completion"
            );
        }
    }

    #[test]
    fn completions_keep_supported_types() {
        let labels = completion_labels();
        for supported in [
            "void",
            "int",
            "float",
            "string",
            "bool",
            "mixed",
            "resource",
            "array",
            "List",
            "Dictionary",
            "Set",
        ] {
            assert!(
                labels.iter().any(|label| label == supported),
                "supported or reserved type `{supported}` should remain an LSP completion"
            );
        }
    }

    #[test]
    fn completion_marks_resource_as_reserved() {
        assert_eq!(
            completion_detail("resource").as_deref(),
            Some("Reserved Doria type name")
        );
    }
}
