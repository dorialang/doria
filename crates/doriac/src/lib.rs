pub mod arg_binding;
pub mod ast;
pub mod backend;
pub mod builtins;
pub mod class_layout;
pub mod codegen_cranelift;
#[cfg(feature = "llvm-backend")]
pub mod codegen_llvm;
pub mod codegen_native;
pub mod codegen_php;
pub mod const_eval;
mod constructor_init;
pub mod control_flow;
pub mod dataflow;
pub mod diagnostics;
pub mod format_string;
pub mod hir;
pub mod lexer;
pub mod lowering;
pub mod mir;
pub mod mir_interpreter;
pub mod mir_lowering;
pub mod mir_validation;
mod narrowing;
pub mod native_abi;
pub mod numeric;
pub mod ownership;
pub mod parser;
pub mod performance;
pub mod return_analysis;
pub mod runtime_artifact;
#[path = "runtime_digest.rs"]
mod runtime_digest;
pub mod semantics;
pub mod source;
pub mod string_literal;
pub mod symbols;
pub mod types;

pub const TOOLCHAIN_VERSION: &str = env!("DORIA_TOOLCHAIN_VERSION");
pub const BUILD_COMMIT: &str = env!("DORIA_BUILD_COMMIT");

use ast::Program;
use backend::{BackendTarget, CompileOptions};
use diagnostics::{Diagnostic, DiagnosticFormat, DiagnosticResult, RenderOptions};
use source::{SourceFile, Span};

pub fn lex_source(
    path: impl Into<String>,
    text: impl Into<String>,
) -> DiagnosticResult<Vec<lexer::Token>> {
    let source = SourceFile::new(path, text);
    lexer::Lexer::new(&source).lex()
}

pub fn parse_source(path: impl Into<String>, text: impl Into<String>) -> DiagnosticResult<Program> {
    let source = SourceFile::new(path, text);
    parse_source_file(&source)
}

pub fn check_source(path: impl Into<String>, text: impl Into<String>) -> DiagnosticResult<Program> {
    let source = SourceFile::new(path, text);
    let program = parse_source_file(&source)?;
    semantics::check_program(&program)?;
    Ok(program)
}

pub fn compile_source_to_php(
    path: impl Into<String>,
    text: impl Into<String>,
) -> DiagnosticResult<String> {
    match compile_source(path, text, BackendTarget::Php)? {
        backend::BackendOutput::Text { contents, .. } => Ok(contents),
        _ => Err(vec![Diagnostic::new(
            "B0002",
            "PHP backend did not return text output",
            Span::default(),
        )]),
    }
}

pub fn compile_source_to_debug(
    path: impl Into<String>,
    text: impl Into<String>,
) -> DiagnosticResult<String> {
    match compile_source(path, text, BackendTarget::Debug)? {
        backend::BackendOutput::Text { contents, .. } => Ok(contents),
        _ => Err(vec![Diagnostic::new(
            "B0002",
            "debug backend did not return text output",
            Span::default(),
        )]),
    }
}

pub fn lower_source(
    path: impl Into<String>,
    text: impl Into<String>,
) -> DiagnosticResult<hir::Program> {
    let source = SourceFile::new(path, text);
    let program = parse_source_file(&source)?;
    let semantic_info = semantics::analyze_program(&program)?;
    let mut hir = lowering::lower_program_with_semantics(&program, semantic_info)?;
    hir.source_path = source.path;
    hir.source_text = source.text;
    Ok(hir)
}

pub fn lower_source_to_mir(
    path: impl Into<String>,
    text: impl Into<String>,
) -> DiagnosticResult<mir::Program> {
    let hir = lower_source(path, text)?;
    mir_lowering::lower_program(&hir)
}

pub fn compile_source(
    path: impl Into<String>,
    text: impl Into<String>,
    target: BackendTarget,
) -> Result<backend::BackendOutput, Vec<Diagnostic>> {
    compile_source_with_options(path, text, CompileOptions::new(target))
}

pub fn compile_source_with_options(
    path: impl Into<String>,
    text: impl Into<String>,
    options: CompileOptions,
) -> Result<backend::BackendOutput, Vec<Diagnostic>> {
    let hir = lower_source(path, text)?;
    backend::emit_with_options(&hir, options).map_err(|error| {
        error.diagnostics.unwrap_or_else(|| {
            let summary = error
                .message
                .lines()
                .next()
                .unwrap_or("the backend did not provide an error summary")
                .to_string();
            vec![
                Diagnostic::new("B0001", error.message, Span::default())
                    .with_note(summary)
                    .with_help(
                        "use `--diagnostic-format json` or set `DORIA_DIAGNOSTIC_DEBUG=1` for complete developer details",
                    ),
            ]
        })
    })
}

pub fn parse_source_file(source: &SourceFile) -> DiagnosticResult<Program> {
    let tokens = lexer::Lexer::new(source).lex()?;
    parser::Parser::new(tokens).parse_program()
}

pub fn render_diagnostics(
    path: impl Into<String>,
    text: impl Into<String>,
    diagnostics: &[Diagnostic],
) -> String {
    let source = SourceFile::new(path, text);
    diagnostics::render_diagnostics(&source, diagnostics, RenderOptions::default())
}

pub fn diagnostics_json(diagnostics: &[Diagnostic]) -> String {
    render_diagnostics_with_options(
        "<unknown>",
        "",
        diagnostics,
        RenderOptions {
            format: DiagnosticFormat::Json,
            ..RenderOptions::default()
        },
    )
}

pub fn render_diagnostics_with_options(
    path: impl Into<String>,
    text: impl Into<String>,
    diagnostics: &[Diagnostic],
    options: RenderOptions,
) -> String {
    let source = SourceFile::new(path, text);
    diagnostics::render_diagnostics(&source, diagnostics, options)
}
