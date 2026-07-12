pub mod ast;
pub mod backend;
pub mod builtins;
pub mod codegen_cranelift;
#[cfg(feature = "llvm-backend")]
pub mod codegen_llvm;
pub mod codegen_native;
pub mod codegen_php;
pub mod control_flow;
pub mod dataflow;
pub mod diagnostics;
pub mod format_string;
pub mod hir;
pub mod lexer;
pub mod lowering;
pub mod lsp;
pub mod mir;
pub mod mir_interpreter;
pub mod mir_lowering;
pub mod mir_validation;
pub mod native_abi;
pub mod numeric;
pub mod parser;
pub mod return_analysis;
pub mod runtime_artifact;
pub mod semantics;
pub mod source;
pub mod symbols;
pub mod types;

use ast::Program;
use backend::{BackendTarget, CompileOptions};
use diagnostics::{Diagnostic, DiagnosticResult};
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

pub fn lower_source(
    path: impl Into<String>,
    text: impl Into<String>,
) -> DiagnosticResult<hir::Program> {
    let source = SourceFile::new(path, text);
    let program = parse_source_file(&source)?;
    let semantic_info = semantics::analyze_program(&program)?;
    Ok(lowering::lower_program_with_semantics(
        &program,
        semantic_info,
    ))
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
        error
            .diagnostics
            .unwrap_or_else(|| vec![Diagnostic::new("B0001", error.message, Span::default())])
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
    diagnostics
        .iter()
        .map(|diagnostic| diagnostic.render(&source))
        .collect::<Vec<_>>()
        .join("\n")
}
