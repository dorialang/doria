use std::collections::HashMap;
use std::io::{self, BufRead, Write};

use serde_json::{json, Value};

use crate::diagnostics::Diagnostic;
use crate::lexer::{Token, TokenKind};
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

pub fn code_actions_for_document(uri: &str, text: &str) -> Vec<Value> {
    let Err(diagnostics) = crate::check_source(uri.to_string(), text.to_string()) else {
        return Vec::new();
    };

    diagnostics
        .iter()
        .filter_map(|diagnostic| {
            let fix = diagnostic.fix.as_ref()?;
            let edit = json!({
                "range": span_to_range(text, fix.span),
                "newText": fix.replacement,
            });
            let mut changes = serde_json::Map::new();
            changes.insert(uri.to_string(), Value::Array(vec![edit]));

            Some(json!({
                "title": diagnostic
                    .help
                    .as_deref()
                    .unwrap_or("Apply compiler-suggested fix"),
                "kind": "quickfix",
                "diagnostics": [diagnostic_to_lsp(text, diagnostic)],
                "isPreferred": true,
                "edit": {
                    "changes": changes,
                },
            }))
        })
        .collect()
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
            "textDocument/codeAction" => {
                if let Some(id) = id {
                    let actions = self.code_actions(message.get("params"));
                    send_response(writer, id, actions)?;
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

    fn code_actions(&self, params: Option<&Value>) -> Value {
        let Some(uri) = params
            .and_then(|params| params.get("textDocument"))
            .and_then(|text_document| text_document.get("uri"))
            .and_then(Value::as_str)
        else {
            return json!([]);
        };
        let Some(document) = self.documents.get(uri) else {
            return json!([]);
        };

        Value::Array(code_actions_for_document(uri, &document.text))
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
            "hoverProvider": true,
            "codeActionProvider": true
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
    let planned_keywords = [
        "interface",
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
        "throw",
        "throws",
        "try",
        "catch",
        "finally",
        "when",
        "given",
    ];
    let types = [
        "void",
        "int",
        "int8",
        "int16",
        "int32",
        "int64",
        "uint8",
        "uint16",
        "uint32",
        "uint64",
        "float",
        "float32",
        "float64",
        "string",
        "bool",
        "mixed",
        "List",
        "Dictionary",
        "Set",
    ];
    let reserved_types = ["resource"];
    let integer_conversions = [
        ("Int::from", "int (the int64 alias)"),
        ("Int8::from", "int8"),
        ("Int16::from", "int16"),
        ("Int32::from", "int32"),
        ("Int64::from", "int64"),
        ("UInt8::from", "uint8"),
        ("UInt16::from", "uint16"),
        ("UInt32::from", "uint32"),
        ("UInt64::from", "uint64"),
    ];

    let mut items = Vec::new();
    items.extend(keywords.into_iter().map(|keyword| {
        let planned = planned_keywords.contains(&keyword);
        let mut item = json!({
            "label": keyword,
            "kind": 14,
            "detail": if planned { "planned Doria keyword" } else { "Doria keyword" },
        });
        if planned {
            item["documentation"] =
                json!("Accepted planned Doria syntax; compiler support lands in a later stage.");
        }
        item
    }));
    items.extend(types.into_iter().map(|ty| {
        let mut item = json!({
            "label": ty,
            "kind": 25,
            "detail": "Doria type",
        });
        if let Some(documentation) = integer_type_description(ty) {
            item["detail"] = json!("implemented Doria integer type");
            item["documentation"] = json!(documentation);
        }
        if let Some(documentation) = scalar_runtime_type_description(ty) {
            item["detail"] = json!("implemented Doria scalar type");
            item["documentation"] = json!(documentation);
        }
        item
    }));
    items.extend(reserved_types.into_iter().map(|ty| {
        json!({
            "label": ty,
            "kind": 25,
            "detail": "Reserved Doria type name",
        })
    }));
    items.push(json!({
        "label": "Displayable",
        "kind": 8,
        "detail": "compiler-known Doria interface",
        "documentation": "`interface Displayable` requires an explicit `implements Displayable` declaration and exactly `function toString(): string`. It controls interpolation, echo, concatenation, and `%s`; general interfaces remain planned for Stage 35.",
    }));
    items.push(json!({
        "label": "toString",
        "kind": 2,
        "detail": "function toString(): string",
        "documentation": "The exact readonly instance method required by the compiler-known `Displayable` contract.",
    }));
    items.push(json!({
        "label": "panic",
        "kind": 3,
        "detail": "Doria built-in function",
        "documentation": "Terminates execution with a fatal panic, Doria stack trace, and status 101.",
    }));
    items.extend(
        [
            (
                "read_line",
                "read_line(): ?string",
                "Reads one UTF-8 line, strips LF or CRLF, and returns null only at EOF.",
            ),
            (
                "sprintf",
                "sprintf(string $format, ...): string",
                "Formats values with a compile-time-checked literal format string in Stage 17.",
            ),
            (
                "printf",
                "printf(string $format, ...): void",
                "Writes a compile-time-checked format with no added newline and returns void.",
            ),
            (
                "read_file",
                "read_file(string $path): string",
                "Reads a complete UTF-8 text file or panics on failure.",
            ),
            (
                "write_file",
                "write_file(string $path, string $contents): void",
                "Creates or truncates a UTF-8 text file and writes exact bytes.",
            ),
            (
                "write_stderr",
                "write_stderr(string $value): void",
                "Writes exact UTF-8 bytes to stderr without adding a newline.",
            ),
        ]
        .into_iter()
        .map(|(label, detail, documentation)| {
            json!({
                "label": label,
                "kind": 3,
                "detail": detail,
                "documentation": documentation,
            })
        }),
    );
    items.extend(integer_conversions.into_iter().map(|(label, target)| {
        json!({
            "label": label,
            "kind": 3,
            "detail": "Doria integer conversion intrinsic",
            "documentation": format!(
                "Compiler-known explicit conversion to `{target}`. Accepts exactly one integer expression and panics when the value is out of range."
            ),
        })
    }));
    items.extend([
        json!({
            "label": "Int::toFloat",
            "kind": 3,
            "detail": "Doria scalar conversion intrinsic",
            "documentation": "Converts canonical `int`/`int64` to canonical `float`/`float64` with IEEE 754 round-to-nearest, ties-to-even. This conversion does not panic.",
        }),
        json!({
            "label": "Float::toInt",
            "kind": 3,
            "detail": "Doria scalar conversion intrinsic",
            "documentation": "Truncates canonical `float`/`float64` toward zero to canonical `int`/`int64`; NaN, infinity, and out-of-range values panic.",
        }),
    ]);

    json!({
        "isIncomplete": false,
        "items": items,
    })
}

fn scalar_runtime_type_description(name: &str) -> Option<&'static str> {
    match name {
        "float" => Some("Implemented canonical IEEE 754 binary64 scalar type; exact alias of `float64`."),
        "float64" => Some("Implemented IEEE 754 binary64 scalar type; exact alias of `float`."),
        "float32" => Some("Implemented distinct IEEE 754 binary32 scalar type."),
        "bool" => Some("Implemented Copy scalar type with runtime locals, parameters, returns, calls, and short-circuit operators."),
        _ => None,
    }
}

fn integer_type_description(name: &str) -> Option<&'static str> {
    match name {
        "int" => {
            Some("Implemented signed 64-bit integer type. `int` is an exact alias for `int64`.")
        }
        "int8" => Some("Implemented signed 8-bit integer type."),
        "int16" => Some("Implemented signed 16-bit integer type."),
        "int32" => Some("Implemented signed 32-bit integer type."),
        "int64" => {
            Some("Implemented signed 64-bit integer type; the same canonical type as `int`.")
        }
        "uint8" => Some("Implemented unsigned 8-bit integer type."),
        "uint16" => Some("Implemented unsigned 16-bit integer type."),
        "uint32" => Some("Implemented unsigned 32-bit integer type."),
        "uint64" => Some("Implemented unsigned 64-bit integer type."),
        _ => None,
    }
}

fn hover_at_offset(text: &str, offset: usize) -> Option<Value> {
    let tokens = crate::lex_source("<lsp>", text.to_string()).ok()?;
    let token_index = tokens.iter().position(|token| {
        !matches!(token.kind, TokenKind::Eof)
            && token.span.start <= offset
            && offset <= token.span.end
    })?;
    let token = &tokens[token_index];
    let description = integer_conversion_hover_at(&tokens, token_index)
        .or_else(|| hover_description(&token.kind))?;

    Some(json!({
        "contents": {
            "kind": "markdown",
            "value": description,
        },
        "range": span_to_range(text, token.span),
    }))
}

fn integer_conversion_hover_at(tokens: &[Token], token_index: usize) -> Option<&'static str> {
    let TokenKind::Identifier(name) = &tokens[token_index].kind else {
        return None;
    };

    if let Some(description) = integer_conversion_description(name) {
        return Some(description);
    }

    if token_index < 2 {
        return None;
    }
    if !matches!(tokens[token_index - 1].kind, TokenKind::DoubleColon) {
        return None;
    }

    let TokenKind::Identifier(companion) = &tokens[token_index - 2].kind else {
        return None;
    };
    match (companion.as_str(), name.as_str()) {
        ("Int", "toFloat") => cross_kind_conversion_description("Int::toFloat"),
        ("Float", "toInt") => cross_kind_conversion_description("Float::toInt"),
        (_, "from") => integer_conversion_description(companion),
        _ => None,
    }
}

fn cross_kind_conversion_description(name: &str) -> Option<&'static str> {
    match name {
        "Int::toFloat" => Some("`Int::toFloat(value)` converts canonical `int`/`int64` to canonical `float`/`float64` using IEEE 754 round-to-nearest, ties-to-even, without panicking."),
        "Float::toInt" => Some("`Float::toInt(value)` truncates canonical `float`/`float64` toward zero to canonical `int`/`int64`; NaN, infinity, and out-of-range values panic."),
        _ => None,
    }
}

fn integer_conversion_description(companion: &str) -> Option<&'static str> {
    match companion {
        "Int" => Some("`Int::from(value)` explicitly converts one integer expression to `int`, the exact `int64` alias. Out-of-range conversion panics."),
        "Int8" => Some("`Int8::from(value)` explicitly converts one integer expression to `int8`. Out-of-range conversion panics."),
        "Int16" => Some("`Int16::from(value)` explicitly converts one integer expression to `int16`. Out-of-range conversion panics."),
        "Int32" => Some("`Int32::from(value)` explicitly converts one integer expression to `int32`. Out-of-range conversion panics."),
        "Int64" => Some("`Int64::from(value)` explicitly converts one integer expression to `int64`, the same canonical type as `int`. Out-of-range conversion panics."),
        "UInt8" => Some("`UInt8::from(value)` explicitly converts one integer expression to `uint8`. Out-of-range conversion panics."),
        "UInt16" => Some("`UInt16::from(value)` explicitly converts one integer expression to `uint16`. Out-of-range conversion panics."),
        "UInt32" => Some("`UInt32::from(value)` explicitly converts one integer expression to `uint32`. Out-of-range conversion panics."),
        "UInt64" => Some("`UInt64::from(value)` explicitly converts one integer expression to `uint64`. Out-of-range conversion panics."),
        _ => None,
    }
}

fn hover_description(kind: &TokenKind) -> Option<&'static str> {
    match kind {
        TokenKind::Class => Some("Declares a Doria class."),
        TokenKind::Interface => Some(
            "General interface declarations are planned for Stage 35. Stage 18 provides only the compiler-known `Displayable` contract.",
        ),
        TokenKind::Implements => Some(
            "Declares nominal conformance. Stage 18 supports only the compiler-known `Displayable` contract; general conformance is planned for Stage 35.",
        ),
        TokenKind::Function => Some("Declares a function or method."),
        TokenKind::Let => Some("Declares a local binding with an inferred type."),
        TokenKind::Writable => {
            Some("Marks a binding, property, parameter, or method receiver as mutable.")
        }
        TokenKind::Internal => {
            Some("Marks a class member as hidden from the external object surface.")
        }
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
        TokenKind::IntType => integer_type_description("int"),
        TokenKind::Int8Type => integer_type_description("int8"),
        TokenKind::Int16Type => integer_type_description("int16"),
        TokenKind::Int32Type => integer_type_description("int32"),
        TokenKind::Int64Type => integer_type_description("int64"),
        TokenKind::UInt8Type => integer_type_description("uint8"),
        TokenKind::UInt16Type => integer_type_description("uint16"),
        TokenKind::UInt32Type => integer_type_description("uint32"),
        TokenKind::UInt64Type => integer_type_description("uint64"),
        TokenKind::FloatType => scalar_runtime_type_description("float"),
        TokenKind::Float32Type => scalar_runtime_type_description("float32"),
        TokenKind::Float64Type => scalar_runtime_type_description("float64"),
        TokenKind::StringType => Some("The immutable UTF-8 `string` primitive type. Stage 17 also supports the narrow `?string` EOF seed used by `read_line`.") ,
        TokenKind::BoolType => scalar_runtime_type_description("bool"),
        TokenKind::True | TokenKind::False => Some("Boolean literal."),
        TokenKind::Null => Some("Null literal. Stage 17 supports `?string` narrowly for `read_line` EOF; general nullable types remain planned for Stage 22."),
        TokenKind::Reserved(_) => Some("Reserved for future Doria syntax."),
        TokenKind::Identifier(name) => match name.as_str() {
            "Displayable" => Some("`interface Displayable` is the compiler-known display contract. A class must explicitly declare `implements Displayable` and provide `function toString(): string`. It controls interpolation, echo, concatenation, and `%s`; general interfaces remain planned for Stage 35."),
            "toString" => Some("`function toString(): string` is the exact externally accessible readonly instance method required by `Displayable`."),
            "List" => Some("Ordered collection alias: `List<T>`."),
            "Dictionary" => Some("Key-value collection alias: `Dictionary<K, V>`."),
            "Set" => Some("Unique-value collection alias: `Set<T>`."),
            "mixed" => Some("Dynamic boundary type. Operations on `mixed` require future narrowing syntax before use."),
            "resource" => Some("Reserved for future PHP interop; not a usable core type."),
            companion @ ("Int" | "Int8" | "Int16" | "Int32" | "Int64" | "UInt8"
            | "UInt16" | "UInt32" | "UInt64") => integer_conversion_description(companion),
            "panic" => Some(
                "Built-in fatal runtime function: `panic(\"message\");`. Panics are not catchable and exit with status 101.",
            ),
            "read_line" => Some("`read_line(): ?string` reads one UTF-8 line, strips LF or CRLF, preserves empty and unterminated final lines, and returns `null` only at EOF."),
            "sprintf" => Some("`sprintf(string $format, ...): string` uses a compile-time-checked literal format string in Stage 17."),
            "printf" => Some("`printf(string $format, ...): void` uses the same checked formatter as `sprintf`, adds no newline, and returns void."),
            "read_file" => Some("`read_file(string $path): string` reads complete UTF-8 text and panics on failure."),
            "write_file" => Some("`write_file(string $path, string $contents): void` creates or truncates a UTF-8 text file and writes exact bytes."),
            "write_stderr" => Some("`write_stderr(string $value): void` writes exact bytes to stderr without adding a newline."),
            _ => None,
        },
        TokenKind::Variable(_) => Some("Doria variable. Variables must be declared before use."),
        _ => None,
    }
}

fn diagnostic_to_lsp(text: &str, diagnostic: &Diagnostic) -> Value {
    let message = if let Some(help) = &diagnostic.help {
        format!("{}\nHelp: {help}", diagnostic.message)
    } else {
        diagnostic.message.clone()
    };

    let mut value = json!({
        "range": span_to_range(text, diagnostic.span),
        "severity": 1,
        "code": diagnostic.code,
        "source": "doriac",
        "message": message,
    });
    if let Some(fix) = &diagnostic.fix {
        value["data"] = json!({
            "fix": {
                "range": span_to_range(text, fix.span),
                "newText": fix.replacement,
            }
        });
    }
    value
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

    fn completion_item(label: &str) -> Value {
        completion_items()["items"]
            .as_array()
            .expect("completion items should be an array")
            .iter()
            .find(|item| item["label"] == label)
            .unwrap_or_else(|| panic!("completion item `{label}` should exist"))
            .clone()
    }
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
    fn completions_mark_accepted_planned_keywords() {
        for keyword in [
            "enum", "case", "match", "async", "await", "unsafe", "extern", "open", "override",
            "with", "take", "throw", "throws", "try", "catch", "finally", "when", "given",
        ] {
            let item = completion_item(keyword);
            assert_eq!(item["detail"], "planned Doria keyword");
            assert_eq!(
                item["documentation"],
                "Accepted planned Doria syntax; compiler support lands in a later stage."
            );
        }
    }

    #[test]
    fn completions_keep_rejected_syntax_out() {
        let labels = completion_labels();
        let rejected = [
            ["pub", "lic"].concat(),
            ["pri", "vate"].concat(),
            ["pro", "tected"].concat(),
            ["requ", "ire"].concat(),
            ["requ", "ire_once"].concat(),
            ["include", "_once"].concat(),
            ["=", "=="].concat(),
            ["!", "=="].concat(),
            ["#de", "fine"].concat(),
            ["#inc", "lude"].concat(),
            ["ar", "ray"].concat(),
        ];
        for rejected in rejected {
            assert!(
                !labels.iter().any(|label| label == &rejected),
                "rejected syntax `{rejected}` must not be an active LSP completion"
            );
        }
    }

    #[test]
    fn planned_keyword_usage_still_reports_compiler_diagnostics() {
        let diagnostics = diagnostics_for_document(
            "test.doria",
            r#"enum Option
{
}
"#,
        );

        assert!(
            !diagnostics.is_empty(),
            "planned syntax should remain rejected by compiler diagnostics until implemented"
        );
    }

    #[test]
    fn completion_and_hover_document_the_narrow_displayable_contract() {
        let completion = completion_item("Displayable");
        let documentation = completion["documentation"]
            .as_str()
            .expect("Displayable completion should have documentation");
        assert!(documentation.contains("interface Displayable"));
        assert!(documentation.contains("function toString(): string"));
        assert!(documentation.contains("interpolation, echo, concatenation, and `%s`"));
        assert!(documentation.contains("general interfaces remain planned for Stage 35"));

        let source = "class Label implements Displayable {}";
        let hover = hover_at_offset(
            source,
            source.find("Displayable").expect("Displayable token"),
        )
        .expect("Displayable should provide hover information");
        let text = hover["contents"]["value"]
            .as_str()
            .expect("hover contents should be markdown");
        assert!(text.contains("interface Displayable"));
        assert!(text.contains("function toString(): string"));
        assert!(text.contains("general interfaces remain planned for Stage 35"));
    }
    #[test]
    fn hover_help_does_not_present_planned_syntax_as_immediate_fixes() {
        let null_hover = hover_description(&TokenKind::Null).expect("null should have hover text");
        assert!(null_hover.contains("planned"));
        assert!(null_hover.contains("Stage 22"));
        assert!(null_hover.contains("`?string`"));

        let mixed_hover = hover_description(&TokenKind::Identifier("mixed".to_string()))
            .expect("mixed should have hover text");
        assert!(mixed_hover.contains("future narrowing syntax"));
        assert!(!mixed_hover.contains("`is`"));
        assert!(!mixed_hover.contains("`match`"));
    }
    #[test]
    fn completions_do_not_offer_unrelated_future_types() {
        let labels = completion_labels();
        for unsupported in [
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
            "int8",
            "int16",
            "int32",
            "int64",
            "uint8",
            "uint16",
            "uint32",
            "uint64",
            "float",
            "float32",
            "float64",
            "string",
            "bool",
            "mixed",
            "resource",
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
    fn integer_type_completions_and_hover_mark_stage_13_coverage() {
        let integer_types = [
            ("int", TokenKind::IntType),
            ("int8", TokenKind::Int8Type),
            ("int16", TokenKind::Int16Type),
            ("int32", TokenKind::Int32Type),
            ("int64", TokenKind::Int64Type),
            ("uint8", TokenKind::UInt8Type),
            ("uint16", TokenKind::UInt16Type),
            ("uint32", TokenKind::UInt32Type),
            ("uint64", TokenKind::UInt64Type),
        ];

        for (name, kind) in integer_types {
            let item = completion_item(name);
            assert_eq!(item["detail"], "implemented Doria integer type");
            assert!(item["documentation"]
                .as_str()
                .expect("integer type completion should have documentation")
                .contains("Implemented"));

            let hover = hover_description(&kind).expect("integer type should have hover text");
            assert!(hover.contains("Implemented"));
        }

        let int_documentation = completion_item("int")["documentation"]
            .as_str()
            .expect("int completion should have documentation")
            .to_string();
        assert!(int_documentation.contains("exact alias for `int64`"));
        let int_hover = hover_description(&TokenKind::IntType).expect("int should have hover text");
        assert!(int_hover.contains("exact alias for `int64`"));
    }

    #[test]
    fn float_and_bool_completions_and_hover_mark_stage_14_runtime_coverage() {
        for (name, kind) in [
            ("float", TokenKind::FloatType),
            ("float32", TokenKind::Float32Type),
            ("float64", TokenKind::Float64Type),
            ("bool", TokenKind::BoolType),
        ] {
            let item = completion_item(name);
            assert_eq!(item["detail"], "implemented Doria scalar type");
            assert!(item["documentation"]
                .as_str()
                .expect("scalar completion should have documentation")
                .contains("Implemented"));

            let hover = hover_description(&kind).expect("scalar should have hover text");
            assert!(hover.contains("Implemented"));
        }
        assert!(completion_item("float")["documentation"]
            .as_str()
            .unwrap()
            .contains("alias of `float64`"));
    }

    #[test]
    fn integer_conversion_completions_and_hover_are_exposed() {
        for companion in [
            "Int", "Int8", "Int16", "Int32", "Int64", "UInt8", "UInt16", "UInt32", "UInt64",
        ] {
            let label = format!("{companion}::from");
            let item = completion_item(&label);
            assert_eq!(item["detail"], "Doria integer conversion intrinsic");
            assert!(item["documentation"]
                .as_str()
                .expect("conversion completion should have documentation")
                .contains("panics"));

            let hover = hover_description(&TokenKind::Identifier(companion.to_string()))
                .expect("conversion companion should have hover text");
            assert!(hover.contains(&label));
        }

        let source = "let $converted = UInt8::from($value);";
        let from_offset = source.find("from").expect("source should contain from") + 1;
        let hover = hover_at_offset(source, from_offset)
            .expect("from in a conversion intrinsic should have contextual hover text");
        let hover_text = hover["contents"]["value"]
            .as_str()
            .expect("hover contents should be text");
        assert!(hover_text.contains("`UInt8::from(value)`"));
        assert!(hover_text.contains("Out-of-range conversion panics"));
    }

    #[test]
    fn cross_kind_conversion_completions_and_hover_are_exposed() {
        for (label, source, method) in [
            ("Int::toFloat", "Int::toFloat($value)", "toFloat"),
            ("Float::toInt", "Float::toInt($value)", "toInt"),
        ] {
            let item = completion_item(label);
            assert_eq!(item["detail"], "Doria scalar conversion intrinsic");
            let offset = source.find(method).unwrap() + 1;
            let hover = hover_at_offset(source, offset).expect("intrinsic should have hover");
            assert!(hover["contents"]["value"].as_str().unwrap().contains(label));
        }

        for name in ["toFloat", "toInt"] {
            let source = format!("function {name}(): int {{ return 42; }}");
            let offset = source.find(name).unwrap() + 1;
            assert!(
                hover_at_offset(&source, offset).is_none(),
                "unqualified user function {name} must not receive intrinsic hover text"
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

    #[test]
    fn completion_and_hover_expose_panic_as_a_builtin_function() {
        assert_eq!(
            completion_detail("panic").as_deref(),
            Some("Doria built-in function")
        );
        let hover = hover_description(&TokenKind::Identifier("panic".to_string()))
            .expect("panic should have hover text");
        assert!(hover.contains("status 101"));
        assert!(hover.contains("not catchable"));
    }

    #[test]
    fn completions_and_hover_expose_stage17_builtins() {
        for (name, signature, required_hover) in [
            ("read_line", "read_line(): ?string", "only at EOF"),
            (
                "sprintf",
                "sprintf(string $format, ...): string",
                "literal format",
            ),
            (
                "printf",
                "printf(string $format, ...): void",
                "adds no newline",
            ),
            ("read_file", "read_file(string $path): string", "UTF-8"),
            (
                "write_file",
                "write_file(string $path, string $contents): void",
                "UTF-8",
            ),
            (
                "write_stderr",
                "write_stderr(string $value): void",
                "without adding a newline",
            ),
        ] {
            assert_eq!(completion_detail(name).as_deref(), Some(signature));
            let hover = hover_description(&TokenKind::Identifier(name.to_string()))
                .expect("Stage 17 builtin should have hover text");
            assert!(hover.contains(required_hover), "{name}: {hover}");
        }
        assert!(!completion_labels().contains(&"print".to_string()));
    }
}
