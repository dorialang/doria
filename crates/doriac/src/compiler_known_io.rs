use crate::ast::{
    Block, ClassDecl, ClassMember, EnumCaseDecl, EnumDecl, EnumPayloadField, Expr, FunctionDecl,
    Item, MemberAccess, Param, Program,
};
use crate::diagnostics::{Diagnostic, DiagnosticResult};
use crate::lexer::{Lexer, TokenKind};
use crate::names::{CompilerSymbolIdentity, GlobalSymbolFacts, GlobalSymbolOwner};
use crate::source::{SourceFile, Span};
use crate::types::TypeRef;

pub const IO_OPERATION: &str = "Doria\\Std\\Io\\IoOperation";
pub const IO_TARGET: &str = "Doria\\Std\\Io\\IoTarget";
pub const IO_ERROR_REASON: &str = "Doria\\Std\\Io\\IoErrorReason";
pub const UTF8_INPUT_SOURCE: &str = "Doria\\Std\\Io\\Utf8InputSource";
pub const IO_ERROR: &str = "Doria\\Std\\Io\\IoError";
pub const INVALID_UTF8_ERROR: &str = "Doria\\Std\\Io\\InvalidUtf8Error";
pub const SYNTHETIC_SOURCE_ID: crate::source::SourceId = crate::source::SourceId(0x7fff_ffff);
pub const SYNTHETIC_SOURCE_IDENTITY: &str = "<compiler-known:stdio>";

pub const CANONICAL_TYPES: [&str; 6] = [
    IO_OPERATION,
    IO_TARGET,
    IO_ERROR_REASON,
    UTF8_INPUT_SOURCE,
    IO_ERROR,
    INVALID_UTF8_ERROR,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IoOperation {
    Open,
    Read,
    Write,
    Append,
    Flush,
}

impl IoOperation {
    pub(crate) const fn case_name(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::Read => "Read",
            Self::Write => "Write",
            Self::Append => "Append",
            Self::Flush => "Flush",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum IoTarget {
    File(String),
    StandardInput,
    StandardOutput,
    StandardError,
}

impl IoTarget {
    pub(crate) const fn case_name(&self) -> &'static str {
        match self {
            Self::File(_) => "File",
            Self::StandardInput => "StandardInput",
            Self::StandardOutput => "StandardOutput",
            Self::StandardError => "StandardError",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IoErrorReason {
    NotFound,
    PermissionDenied,
    InvalidInput,
    Interrupted,
    ResourceExhausted,
    Unsupported,
    Closed,
    Other,
}

impl IoErrorReason {
    pub(crate) const fn case_name(self) -> &'static str {
        match self {
            Self::NotFound => "NotFound",
            Self::PermissionDenied => "PermissionDenied",
            Self::InvalidInput => "InvalidInput",
            Self::Interrupted => "Interrupted",
            Self::ResourceExhausted => "ResourceExhausted",
            Self::Unsupported => "Unsupported",
            Self::Closed => "Closed",
            Self::Other => "Other",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Utf8InputSource {
    File(String),
    StandardInput,
}

impl Utf8InputSource {
    pub(crate) const fn case_name(&self) -> &'static str {
        match self {
            Self::File(_) => "File",
            Self::StandardInput => "StandardInput",
        }
    }
}

pub(crate) fn io_error_message(
    operation: IoOperation,
    target: &IoTarget,
    reason: IoErrorReason,
) -> String {
    use doria_diagnostic_catalogue::{
        visit_io_error_message_parts, IoMessageOperation, IoMessageReason, IoMessageTarget,
    };

    let operation = match operation {
        IoOperation::Open => IoMessageOperation::Open,
        IoOperation::Read => IoMessageOperation::Read,
        IoOperation::Write => IoMessageOperation::Write,
        IoOperation::Append => IoMessageOperation::Append,
        IoOperation::Flush => IoMessageOperation::Flush,
    };
    let target = match target {
        IoTarget::File(path) => IoMessageTarget::File(path.as_bytes()),
        IoTarget::StandardInput => IoMessageTarget::StandardInput,
        IoTarget::StandardOutput => IoMessageTarget::StandardOutput,
        IoTarget::StandardError => IoMessageTarget::StandardError,
    };
    let reason = match reason {
        IoErrorReason::NotFound => IoMessageReason::NotFound,
        IoErrorReason::PermissionDenied => IoMessageReason::PermissionDenied,
        IoErrorReason::InvalidInput => IoMessageReason::InvalidInput,
        IoErrorReason::Interrupted => IoMessageReason::Interrupted,
        IoErrorReason::ResourceExhausted => IoMessageReason::ResourceExhausted,
        IoErrorReason::Unsupported => IoMessageReason::Unsupported,
        IoErrorReason::Closed => IoMessageReason::Closed,
        IoErrorReason::Other => IoMessageReason::Other,
    };
    let mut bytes = Vec::new();
    visit_io_error_message_parts(operation, target, reason, |part| {
        bytes.extend_from_slice(part)
    });
    String::from_utf8(bytes).expect("compiler-known I/O messages are valid UTF-8")
}

pub(crate) fn invalid_utf8_message(
    source: &Utf8InputSource,
    _valid_byte_count: usize,
    _invalid_byte_count: Option<usize>,
) -> String {
    use doria_diagnostic_catalogue::{visit_invalid_utf8_message_parts, Utf8MessageSource};

    let source = match source {
        Utf8InputSource::File(path) => Utf8MessageSource::File(path.as_bytes()),
        Utf8InputSource::StandardInput => Utf8MessageSource::StandardInput,
    };
    let mut bytes = Vec::new();
    visit_invalid_utf8_message_parts(source, |part| bytes.extend_from_slice(part));
    String::from_utf8(bytes).expect("compiler-known UTF-8 messages are valid UTF-8")
}

pub fn is_canonical_type(name: &str) -> bool {
    CANONICAL_TYPES.contains(&name)
}

pub fn validate_reserved_identities(program: &Program) -> DiagnosticResult<()> {
    let mut diagnostics = Vec::new();
    let namespace = program
        .namespace
        .as_ref()
        .map(|namespace| namespace.name.canonical());
    for item in &program.items {
        let (name, span) = match item {
            Item::Class(class) => (&class.name, class.name_span),
            Item::Enum(definition) => (&definition.name, definition.name_span),
            Item::Interface(interface) => (&interface.name, interface.name_span),
            Item::Trait(declaration) => (&declaration.name, declaration.name_span),
            _ => continue,
        };
        let canonical = namespace
            .as_ref()
            .map_or_else(|| name.clone(), |namespace| format!("{namespace}\\{name}"));
        if is_canonical_type(&canonical) {
            diagnostics.push(
                Diagnostic::new(
                    "E0640",
                    format!("`{canonical}` is a compiler-known Doria standard-library type"),
                    span,
                )
                .with_title("Reserved Standard-Library Type")
                .with_help("choose a different declaration name"),
            );
        }
    }
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

/// Returns whether executable source syntax needs the compiler-known I/O family.
///
/// The central name resolver owns canonical type identity. This narrow token
/// pass covers I/O operations and structural callable syntax. Structural
/// callables use one ambient-capable indirect ABI even when a particular value
/// is ambient-free, so their MIR profiles also need the canonical descriptors.
pub fn source_uses_io_intrinsics(source: &SourceFile) -> DiagnosticResult<bool> {
    let tokens = Lexer::new(source).lex()?;
    Ok(tokens.iter().enumerate().any(|(index, token)| {
        matches!(token.kind, TokenKind::Echo | TokenKind::Fn)
            || (matches!(token.kind, TokenKind::Function)
                && tokens
                    .get(index + 1)
                    .is_some_and(|next| matches!(next.kind, TokenKind::LeftParen)))
            || match &token.kind {
                TokenKind::Identifier(name) => crate::builtins::Builtin::from_name(name)
                    .is_some_and(|builtin| !builtin.checked_error_types().is_empty()),
                _ => false,
            }
    }))
}

pub fn resolved_facts_use_canonical_io(facts: &GlobalSymbolFacts) -> bool {
    facts.references.iter().any(|reference| {
        matches!(
            reference.symbol_id.owner,
            GlobalSymbolOwner::CompilerKnown(CompilerSymbolIdentity::StandardIo(_))
        )
    })
}

/// Adds the six Stage 29 compiler-known I/O identities to the ordinary AST
/// model. General namespace lookup has already resolved every authored use;
/// after this representation boundary, semantic analysis and every backend see
/// normal classes and enums.
pub fn augment_program(program: &Program) -> Program {
    let mut augmented = program.clone();
    let synthetic = |index| Span::in_source(SYNTHETIC_SOURCE_ID, index, index + 1);
    augmented.items.extend([
        unit_enum(
            IO_OPERATION,
            &["Open", "Read", "Write", "Append", "Flush"],
            synthetic(0),
        ),
        payload_enum(
            IO_TARGET,
            &[
                ("File", Some(("string", "path"))),
                ("StandardInput", None),
                ("StandardOutput", None),
                ("StandardError", None),
            ],
            synthetic(1),
        ),
        unit_enum(
            IO_ERROR_REASON,
            &[
                "NotFound",
                "PermissionDenied",
                "InvalidInput",
                "Interrupted",
                "ResourceExhausted",
                "Unsupported",
                "Closed",
                "Other",
            ],
            synthetic(2),
        ),
        payload_enum(
            UTF8_INPUT_SOURCE,
            &[("File", Some(("string", "path"))), ("StandardInput", None)],
            synthetic(3),
        ),
        error_class(
            IO_ERROR,
            &[
                ("string", "message", false),
                (IO_OPERATION, "operation", false),
                (IO_TARGET, "target", false),
                (IO_ERROR_REASON, "reason", false),
                ("int", "systemCode", true),
            ],
            synthetic(4),
        ),
        error_class(
            INVALID_UTF8_ERROR,
            &[
                ("string", "message", false),
                (UTF8_INPUT_SOURCE, "source", false),
                ("int", "validByteCount", false),
                ("int", "invalidByteCount", true),
            ],
            synthetic(5),
        ),
    ]);
    augmented
}

fn unit_enum(name: &str, cases: &[&str], span: Span) -> Item {
    payload_enum(
        name,
        &cases.iter().map(|case| (*case, None)).collect::<Vec<_>>(),
        span,
    )
}

fn payload_enum(name: &str, cases: &[(&str, Option<(&str, &str)>)], span: Span) -> Item {
    Item::Enum(EnumDecl {
        access: MemberAccess::External,
        access_span: None,
        name: name.to_string(),
        name_span: span,
        type_params: Vec::new(),
        backing_type: None,
        cases: cases
            .iter()
            .map(|(case, payload)| EnumCaseDecl {
                name: (*case).to_string(),
                name_span: span,
                payload: payload
                    .iter()
                    .map(|(ty, field)| EnumPayloadField {
                        ty: TypeRef::named(*ty),
                        name: (*field).to_string(),
                        span,
                    })
                    .collect(),
                backing_value: None,
                span,
            })
            .collect(),
        span,
    })
}

fn error_class(name: &str, properties: &[(&str, &str, bool)], span: Span) -> Item {
    let params = properties
        .iter()
        .map(|(ty, property, nullable)| Param {
            promoted_access: Some(MemberAccess::External),
            take: false,
            take_span: None,
            writable: false,
            writable_span: None,
            ownership_modifier_insert: span,
            ty: if *nullable {
                TypeRef::named(*ty).nullable()
            } else {
                TypeRef::named(*ty)
            },
            name: (*property).to_string(),
            default: nullable.then_some(Expr::Null { span }),
            span,
        })
        .collect();
    Item::Class(ClassDecl {
        access: MemberAccess::External,
        access_span: None,
        name: name.to_string(),
        name_span: span,
        type_params: Vec::new(),
        parent: None,
        parent_span: None,
        implements: vec!["Error".to_string()],
        members: vec![ClassMember::Method(FunctionDecl {
            access: MemberAccess::External,
            access_span: None,
            writable_this: false,
            writable_span: None,
            is_static: false,
            static_span: None,
            name: "__construct".to_string(),
            name_span: span,
            type_params: Vec::new(),
            params,
            return_type: None,
            throws: None,
            body: Block {
                statements: Vec::new(),
                span,
            },
            span,
        })],
        span,
    })
}

#[cfg(test)]
mod tests {
    use super::source_uses_io_intrinsics;
    use crate::source::SourceFile;

    #[test]
    fn structural_callable_syntax_requests_canonical_io_transport_symbols() {
        for source in [
            "function invoke(function(): void $callback): void {}",
            "function main(): void { let $callback = function (): void {}; }",
            "function main(): void { let $callback = fn() => 1; }",
        ] {
            assert!(
                source_uses_io_intrinsics(&SourceFile::new("callable.doria", source))
                    .expect("callable source should lex"),
                "{source}"
            );
        }

        assert!(!source_uses_io_intrinsics(&SourceFile::new(
            "plain.doria",
            "function helper(): void {}"
        ))
        .expect("plain function source should lex"));
    }
}
