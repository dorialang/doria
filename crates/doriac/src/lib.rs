pub mod arg_binding;
pub mod ast;
pub mod backend;
pub mod builtins;
mod checked_effects;
pub mod class_layout;
pub mod codegen_cranelift;
#[cfg(feature = "llvm-backend")]
pub mod codegen_llvm;
pub mod codegen_native;
pub mod codegen_php;
pub mod collection_diagnostics;
pub mod compiler_known_io;
pub mod const_eval;
mod constructor_init;
pub mod control_flow;
pub mod dataflow;
pub mod diagnostics;
pub mod enums;
pub mod format_string;
pub mod hir;
pub mod lexer;
pub mod lowering;
pub mod mir;
pub mod mir_interpreter;
pub mod mir_lowering;
pub mod mir_validation;
pub mod names;
mod narrowing;
pub mod native_abi;
pub mod native_closure_abi;
pub mod numeric;
pub mod ownership;
pub mod parser;
pub mod performance;
mod php_closure;
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
use names::{CompilationContext, GlobalSymbolFacts};
use source::{SourceFile, Span};

struct PreparedSource {
    authored: Program,
    resolved: Program,
    context: CompilationContext,
    global_symbols: GlobalSymbolFacts,
}

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
    let context = CompilationContext::standalone(source.path.clone());
    check_source_file_with_context(&source, context)
}

pub fn check_source_with_context(
    path: impl Into<String>,
    text: impl Into<String>,
    context: CompilationContext,
) -> DiagnosticResult<Program> {
    let source = SourceFile::new(path, text);
    check_source_file_with_context(&source, context)
}

/// Parses and semantically analyzes one source file for editor tooling.
///
/// The returned program is the user-authored syntax tree, while the semantic
/// analysis includes compiler-known declarations required by the source. This
/// keeps editor spans anchored to source without bypassing compiler setup.
pub fn analyze_source_for_ide(
    path: impl Into<String>,
    text: impl Into<String>,
) -> DiagnosticResult<(Program, semantics::SemanticAnalysis)> {
    let source = SourceFile::new(path, text);
    let context = CompilationContext::standalone(source.path.clone());
    analyze_source_for_ide_with_context(source.path.clone(), source.text.clone(), context)
}

pub fn analyze_source_for_ide_with_context(
    path: impl Into<String>,
    text: impl Into<String>,
    context: CompilationContext,
) -> DiagnosticResult<(Program, semantics::SemanticAnalysis)> {
    let source = SourceFile::new(path, text);
    let authored = parse_source_file(&source)?;
    compiler_known_io::validate_reserved_identities(&authored)?;
    let mut resolution = names::resolve_program_for_ide(&authored, &context);
    let uses_compiler_known_io = compiler_known_io::source_uses_io_intrinsics(&source)?
        || compiler_known_io::resolved_facts_use_canonical_io(&resolution.resolved.facts);
    if uses_compiler_known_io {
        resolution.resolved.program =
            compiler_known_io::augment_program(&resolution.resolved.program);
    }
    let mut analysis = if resolution.diagnostics.is_empty() {
        semantics::analyze_program_for_ide_with_source(
            &resolution.resolved.program,
            Some(&source.text),
        )
    } else {
        semantics::SemanticAnalysis {
            info: semantics::SemanticInfo::default(),
            diagnostics: resolution.diagnostics,
        }
    };
    analysis.info.compilation_context = context;
    analysis.info.global_symbols = resolution.resolved.facts;
    Ok((authored, analysis))
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
    let context = CompilationContext::standalone(source.path.clone());
    lower_source_file_with_context(source, context)
}

pub fn lower_source_with_context(
    path: impl Into<String>,
    text: impl Into<String>,
    context: CompilationContext,
) -> DiagnosticResult<hir::Program> {
    let source = SourceFile::new(path, text);
    lower_source_file_with_context(source, context)
}

fn lower_source_file_with_context(
    source: SourceFile,
    context: CompilationContext,
) -> DiagnosticResult<hir::Program> {
    let prepared = prepare_source(&source, context)?;
    let mut semantic_info =
        semantics::analyze_program_with_source(&prepared.resolved, &source.text)?;
    semantic_info.compilation_context = prepared.context;
    semantic_info.global_symbols = prepared.global_symbols;
    let mut hir = lowering::lower_program_with_semantics(&prepared.resolved, semantic_info)?;
    hir.source_path = source.path;
    hir.source_text = source.text;
    Ok(hir)
}

pub fn lower_source_to_mir(
    path: impl Into<String>,
    text: impl Into<String>,
) -> DiagnosticResult<mir::Program> {
    let hir = lower_source(path, text)?;
    let mir = mir_lowering::lower_program(&hir)?;
    mir_validation::validate_program(&mir).map_err(|error| {
        error.diagnostics.unwrap_or_else(|| {
            vec![Diagnostic::new("B0001", error.message, Span::default())
                .with_title("Malformed MIR")]
        })
    })?;
    Ok(mir)
}

pub fn lower_source_to_mir_with_context(
    path: impl Into<String>,
    text: impl Into<String>,
    context: CompilationContext,
) -> DiagnosticResult<mir::Program> {
    let hir = lower_source_with_context(path, text, context)?;
    let mir = mir_lowering::lower_program(&hir)?;
    mir_validation::validate_program(&mir).map_err(|error| {
        error.diagnostics.unwrap_or_else(|| {
            vec![Diagnostic::new("B0001", error.message, Span::default())
                .with_title("Malformed MIR")]
        })
    })?;
    Ok(mir)
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

pub fn compile_source_with_context(
    path: impl Into<String>,
    text: impl Into<String>,
    context: CompilationContext,
    options: CompileOptions,
) -> Result<backend::BackendOutput, Vec<Diagnostic>> {
    let hir = lower_source_with_context(path, text, context)?;
    backend::emit_with_options(&hir, options).map_err(|error| {
        error
            .diagnostics
            .unwrap_or_else(|| vec![Diagnostic::new("B0001", error.message, Span::default())])
    })
}

fn check_source_file_with_context(
    source: &SourceFile,
    context: CompilationContext,
) -> DiagnosticResult<Program> {
    let prepared = prepare_source(source, context)?;
    semantics::analyze_program_with_source(&prepared.resolved, &source.text)?;
    Ok(prepared.authored)
}

fn prepare_source(
    source: &SourceFile,
    context: CompilationContext,
) -> DiagnosticResult<PreparedSource> {
    let authored = parse_source_file(source)?;
    compiler_known_io::validate_reserved_identities(&authored)?;
    let mut resolved = names::resolve_program(&authored, &context)?;
    let uses_compiler_known_io = compiler_known_io::source_uses_io_intrinsics(source)?
        || compiler_known_io::resolved_facts_use_canonical_io(&resolved.facts);
    if uses_compiler_known_io {
        resolved.program = compiler_known_io::augment_program(&resolved.program);
    }
    Ok(PreparedSource {
        authored,
        resolved: resolved.program,
        context,
        global_symbols: resolved.facts,
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
