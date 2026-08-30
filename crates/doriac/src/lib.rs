pub mod arg_binding;
pub mod ast;
pub mod attributes;
pub mod backend;
pub mod build_plan;
pub mod builtins;
mod checked_effects;
pub use checked_effects::{CheckedEffectClass, CheckedEffectProfile};
pub mod class_layout;
pub mod codegen_cranelift;
#[cfg(feature = "llvm-backend")]
pub mod codegen_llvm;
pub mod codegen_native;
pub mod codegen_php;
pub mod collection_diagnostics;
pub mod compilation_graph;
pub mod compiler_known_io;
pub mod compiler_known_test;
pub mod const_eval;
mod constructor_init;
pub mod control_flow;
pub mod dataflow;
pub mod diagnostics;
pub mod enums;
pub mod format_string;
pub mod hir;
pub mod incremental;
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
pub mod source_map;
pub mod source_provider;
pub mod string_literal;
pub mod symbols;
pub mod testing;
pub mod types;

pub const TOOLCHAIN_VERSION: &str = env!("DORIA_TOOLCHAIN_VERSION");
pub const BUILD_COMMIT: &str = env!("DORIA_BUILD_COMMIT");

use ast::Program;
use backend::{BackendTarget, CompileOptions};
use diagnostics::{Diagnostic, DiagnosticFormat, DiagnosticResult, RenderOptions};
use names::{CompilationContext, GlobalSymbolFacts};
use source::{SourceFile, Span};
use std::path::Path;

struct PreparedSource {
    authored: Program,
    resolved: Program,
    context: CompilationContext,
    global_symbols: GlobalSymbolFacts,
    source_semantic_contexts:
        std::collections::HashMap<source::SourceId, testing::SourceSemanticContext>,
    test_semantics: testing::TestSemanticFacts,
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
    compiler_known_test::validate_reserved_identities(&authored)?;
    let mut resolution = names::resolve_program_for_ide(&authored, &context);
    let uses_compiler_known_io = compiler_known_io::source_uses_io_intrinsics(&source)?
        || compiler_known_io::resolved_facts_use_canonical_io(&resolution.resolved.facts);
    if uses_compiler_known_io {
        resolution.resolved.program =
            compiler_known_io::augment_program(&resolution.resolved.program);
    }
    let source_context = testing::SourceSemanticContext::standalone(context.clone());
    let (evaluation, _) =
        const_eval::evaluate_program_with_diagnostics(&resolution.resolved.program);
    let elaboration = testing::elaborate_source(
        &resolution.resolved.program,
        &resolution.resolved.facts,
        &source_context,
        &evaluation,
    );
    resolution.resolved.program = elaboration.program;
    resolution.diagnostics.extend(elaboration.diagnostics);
    let resolution_succeeded = resolution.diagnostics.is_empty();
    let mut analysis = if resolution_succeeded {
        let mut source_texts = std::collections::HashMap::from([(source.id, source.text.as_str())]);
        let mut contexts = std::collections::HashMap::from([(source.id, context.clone())]);
        append_compiler_known_semantic_context(
            &resolution.resolved.program,
            &context,
            &mut source_texts,
            &mut contexts,
        );
        semantics::analyze_program_for_ide_with_graph_and_test_context(
            &resolution.resolved.program,
            &source_texts,
            context.clone(),
            contexts,
            std::collections::HashMap::from([(source.id, source_context)]),
            resolution.resolved.facts.clone(),
            elaboration.facts,
        )
    } else {
        semantics::SemanticAnalysis {
            info: semantics::SemanticInfo::default(),
            diagnostics: resolution.diagnostics,
        }
    };
    if !resolution_succeeded {
        analysis.info.compilation_context = context.clone();
        analysis
            .info
            .compilation_contexts
            .insert(source.id, context);
        analysis.info.global_symbols = resolution.resolved.facts;
    }
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

pub fn metadata_source(
    path: impl Into<String>,
    text: impl Into<String>,
) -> DiagnosticResult<attributes::AttributeMetadataDocumentV1> {
    let path = path.into();
    let text = text.into();
    let fingerprint =
        runtime_digest::sha256_hex(format!("source={path};contents={text}").as_bytes());
    let hir = lower_source(path, text)?;
    Ok(attributes::metadata_document(&hir, fingerprint))
}

pub fn metadata_source_v2(
    path: impl Into<String>,
    text: impl Into<String>,
) -> DiagnosticResult<attributes::AttributeMetadataDocumentV2> {
    let path = path.into();
    let text = text.into();
    let fingerprint =
        runtime_digest::sha256_hex(format!("source={path};contents={text}").as_bytes());
    let hir = lower_source(path, text)?;
    Ok(attributes::metadata_document_v2(&hir, fingerprint))
}

pub fn metadata_source_v3(
    path: impl Into<String>,
    text: impl Into<String>,
) -> DiagnosticResult<attributes::AttributeMetadataDocumentV3> {
    let path = path.into();
    let text = text.into();
    let fingerprint =
        runtime_digest::sha256_hex(format!("source={path};contents={text}").as_bytes());
    let hir = lower_source(path, text)?;
    Ok(attributes::metadata_document_v3(&hir, fingerprint))
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
    let semantic_info = analyze_prepared_source(&prepared, &source)?;
    let hir = lowering::lower_program_with_semantics(&prepared.resolved, semantic_info)?;
    Ok(complete_standalone_hir(hir, source, &prepared.context))
}

pub(crate) fn complete_standalone_hir(
    mut hir: hir::Program,
    source: SourceFile,
    context: &CompilationContext,
) -> hir::Program {
    hir.sources = vec![hir::SourceUnit {
        id: source.id,
        identity: context.source.clone(),
        package: context.package.clone(),
        display_path: source.path.clone(),
        scope: build_plan::SourceScope::Main,
        origin: build_plan::SourceOrigin::Entry,
        generated_for: None,
        active: true,
        source: source.clone(),
    }];
    hir.packages = vec![hir::PackageUnit {
        identity: context.package.clone(),
        normal_dependencies: Vec::new(),
        development_dependencies: Vec::new(),
    }];
    hir.source_path = source.path;
    hir.source_text = source.text;
    hir
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

pub fn load_build_plan_file(
    path: impl AsRef<Path>,
) -> DiagnosticResult<(
    build_plan::BuildPlanDocument,
    compilation_graph::CompilationGraph,
)> {
    let path = path.as_ref();
    let display = path.display().to_string();
    let text = std::fs::read_to_string(path).map_err(|error| {
        vec![build_plan::compiler_input_diagnostic(
            "E0676",
            format!("could not read build plan `{display}`: {error}"),
            Span::default(),
        )
        .with_title("Build Plan Could Not Be Read")
        .with_primary_source(diagnostics::DiagnosticSource::Path(display.clone()))]
    })?;
    let document = build_plan::parse_build_plan_document(display, text)?;
    let graph = compilation_graph::load_compilation_graph(
        &document,
        &source_provider::FileSystemSourceProvider,
    )?;
    Ok((document, graph))
}

pub fn check_build_plan_file(path: impl AsRef<Path>) -> DiagnosticResult<ast::Program> {
    let (_, graph) = load_build_plan_file(path)?;
    compilation_graph::check_compilation_graph(&graph)
}

pub fn analyze_compilation_graph_for_ide(
    graph: &compilation_graph::CompilationGraph,
) -> compilation_graph::GraphSemanticAnalysis {
    compilation_graph::analyze_compilation_graph_for_ide(graph)
}

pub fn lower_compilation_graph(
    graph: &compilation_graph::CompilationGraph,
) -> DiagnosticResult<hir::Program> {
    let analysis = compilation_graph::analyze_compilation_graph_for_ide(graph);
    if !analysis.diagnostics.is_empty() {
        return Err(analysis.diagnostics);
    }
    let mut hir =
        lowering::lower_program_with_semantics(&analysis.resolved_program, analysis.semantic_info)?;
    hir.sources = graph
        .sources
        .values()
        .map(|source| hir::SourceUnit {
            id: source.id,
            identity: source.identity.clone(),
            package: source.package.clone(),
            display_path: source.display_path.clone(),
            scope: source.scope,
            origin: source.origin,
            generated_for: source.generated_for,
            active: true,
            source: source.source.clone(),
        })
        .collect();
    hir.packages = graph
        .packages
        .values()
        .map(|package| hir::PackageUnit {
            identity: package.identity.clone(),
            normal_dependencies: package
                .normal_dependencies
                .iter()
                .cloned()
                .map(names::PackageIdentity::Named)
                .collect(),
            development_dependencies: package
                .development_dependencies
                .iter()
                .cloned()
                .map(names::PackageIdentity::Named)
                .collect(),
        })
        .collect();
    hir.selected_target = hir::SelectedTarget {
        package: names::PackageIdentity::Named(graph.build_plan.selected_target.package.clone()),
        kind: graph.build_plan.selected_target.kind,
        entry_source: graph.selected_entry.clone(),
    };
    let compatibility_source = graph
        .selected_entry
        .as_ref()
        .and_then(|entry| graph.sources.get(&entry.0))
        .or_else(|| graph.sources.values().next());
    if let Some(source) = compatibility_source {
        hir.source_path = source.display_path.clone();
        hir.source_text = source.source.text.clone();
        hir.namespace = source
            .authored
            .namespace
            .as_ref()
            .map(|namespace| hir::NamespaceDecl {
                name: namespace.name.canonical(),
                span: namespace.span,
            });
    }
    Ok(hir)
}

pub fn metadata_compilation_graph(
    graph: &compilation_graph::CompilationGraph,
) -> DiagnosticResult<attributes::AttributeMetadataDocumentV1> {
    let hir = lower_compilation_graph(graph)?;
    Ok(attributes::metadata_document(
        &hir,
        graph.fingerprint.clone(),
    ))
}

pub fn metadata_compilation_graph_v2(
    graph: &compilation_graph::CompilationGraph,
) -> DiagnosticResult<attributes::AttributeMetadataDocumentV2> {
    let hir = lower_compilation_graph(graph)?;
    Ok(attributes::metadata_document_v2(
        &hir,
        graph.fingerprint.clone(),
    ))
}

pub fn metadata_compilation_graph_v3(
    graph: &compilation_graph::CompilationGraph,
) -> DiagnosticResult<attributes::AttributeMetadataDocumentV3> {
    let hir = lower_compilation_graph(graph)?;
    Ok(attributes::metadata_document_v3(
        &hir,
        graph.fingerprint.clone(),
    ))
}

pub fn metadata_build_plan_file(
    path: impl AsRef<Path>,
) -> DiagnosticResult<attributes::AttributeMetadataDocumentV1> {
    let (_, graph) = load_build_plan_file(path)?;
    metadata_compilation_graph(&graph)
}

pub fn lower_build_plan_file(path: impl AsRef<Path>) -> DiagnosticResult<hir::Program> {
    let (_, graph) = load_build_plan_file(path)?;
    lower_compilation_graph(&graph)
}

pub fn lower_compilation_graph_to_mir(
    graph: &compilation_graph::CompilationGraph,
) -> DiagnosticResult<mir::Program> {
    let hir = lower_compilation_graph(graph)?;
    let mir = mir_lowering::lower_program(&hir)?;
    mir_validation::validate_program(&mir).map_err(|error| {
        error.diagnostics.unwrap_or_else(|| {
            vec![Diagnostic::new("B0001", error.message, Span::default())
                .with_title("Malformed MIR")]
        })
    })?;
    Ok(mir)
}

pub fn lower_build_plan_file_to_mir(path: impl AsRef<Path>) -> DiagnosticResult<mir::Program> {
    let (_, graph) = load_build_plan_file(path)?;
    lower_compilation_graph_to_mir(&graph)
}

pub fn compile_compilation_graph(
    graph: &compilation_graph::CompilationGraph,
) -> Result<backend::BackendOutput, Vec<Diagnostic>> {
    if graph.build_plan.selected_target.kind == build_plan::TargetKind::Library {
        return Err(vec![build_plan::compiler_input_diagnostic(
            "E0685",
            "compiling an artifact requires a selected binary target",
            Span::default(),
        )
        .with_title("Executable Target Is Required")]);
    }
    let options = match (
        graph.build_plan.compiler.target,
        graph.build_plan.compiler.native_profile,
    ) {
        (build_plan::CompilerTarget::Debug, _) => CompileOptions::new(BackendTarget::Debug),
        (build_plan::CompilerTarget::Php, _) => CompileOptions::new(BackendTarget::Php),
        (build_plan::CompilerTarget::Native, Some(build_plan::BuildNativeProfile::Fast)) => {
            CompileOptions::native(backend::NativeProfile::Fast)
        }
        (build_plan::CompilerTarget::Native, Some(build_plan::BuildNativeProfile::Release)) => {
            CompileOptions::native(backend::NativeProfile::Release)
        }
        (build_plan::CompilerTarget::Native, None) => {
            unreachable!("validated native plan has a profile")
        }
    };
    let hir = lower_compilation_graph(graph)?;
    backend::emit_with_options(&hir, options).map_err(|error| {
        error
            .diagnostics
            .unwrap_or_else(|| vec![Diagnostic::new("B0001", error.message, Span::default())])
    })
}

pub fn compile_build_plan_file(
    path: impl AsRef<Path>,
) -> Result<backend::BackendOutput, Vec<Diagnostic>> {
    let (_, graph) = load_build_plan_file(path)?;
    compile_compilation_graph(&graph)
}

fn check_source_file_with_context(
    source: &SourceFile,
    context: CompilationContext,
) -> DiagnosticResult<Program> {
    let prepared = prepare_source(source, context)?;
    analyze_prepared_source(&prepared, source)?;
    Ok(prepared.authored)
}

pub(crate) fn analyze_prepared_source(
    prepared: &PreparedSource,
    source: &SourceFile,
) -> DiagnosticResult<semantics::SemanticInfo> {
    let mut source_texts = std::collections::HashMap::from([(source.id, source.text.as_str())]);
    let mut contexts = std::collections::HashMap::from([(source.id, prepared.context.clone())]);
    append_compiler_known_semantic_context(
        &prepared.resolved,
        &prepared.context,
        &mut source_texts,
        &mut contexts,
    );
    let analysis = semantics::analyze_program_for_ide_with_graph_and_test_context(
        &prepared.resolved,
        &source_texts,
        prepared.context.clone(),
        contexts,
        prepared.source_semantic_contexts.clone(),
        prepared.global_symbols.clone(),
        prepared.test_semantics.clone(),
    );
    if analysis.diagnostics.is_empty() {
        Ok(analysis.info)
    } else {
        Err(analysis.diagnostics)
    }
}

fn append_compiler_known_semantic_context(
    program: &ast::Program,
    context: &CompilationContext,
    source_texts: &mut std::collections::HashMap<source::SourceId, &str>,
    contexts: &mut std::collections::HashMap<source::SourceId, CompilationContext>,
) {
    if !ast_uses_compiler_known_source(program) {
        return;
    }
    source_texts.insert(compiler_known_io::SYNTHETIC_SOURCE_ID, "");
    contexts.insert(
        compiler_known_io::SYNTHETIC_SOURCE_ID,
        CompilationContext {
            edition: context.edition,
            package: names::PackageIdentity::CompilerKnown,
            source: names::SourceIdentity(compiler_known_io::SYNTHETIC_SOURCE_IDENTITY.to_string()),
        },
    );
}

fn ast_uses_compiler_known_source(program: &ast::Program) -> bool {
    let synthetic = compiler_known_io::SYNTHETIC_SOURCE_ID;
    program.items.iter().any(|item| match item {
        ast::Item::Class(value) => value.span.source == synthetic,
        ast::Item::Enum(value) => value.span.source == synthetic,
        ast::Item::Interface(value) => value.span.source == synthetic,
        ast::Item::Trait(value) => value.span.source == synthetic,
        ast::Item::Function(value) => value.span.source == synthetic,
        ast::Item::Constant(value) => value.span.source == synthetic,
        ast::Item::Statement(_) => false,
    })
}

fn prepare_source(
    source: &SourceFile,
    context: CompilationContext,
) -> DiagnosticResult<PreparedSource> {
    let authored = parse_source_file(source)?;
    prepare_parsed_source(source, context, authored)
}

pub(crate) fn prepare_parsed_source(
    source: &SourceFile,
    context: CompilationContext,
    authored: Program,
) -> DiagnosticResult<PreparedSource> {
    compiler_known_io::validate_reserved_identities(&authored)?;
    compiler_known_test::validate_reserved_identities(&authored)?;
    let mut resolved = names::resolve_program(&authored, &context)?;
    let uses_compiler_known_io = compiler_known_io::source_uses_io_intrinsics(source)?
        || compiler_known_io::resolved_facts_use_canonical_io(&resolved.facts);
    if uses_compiler_known_io {
        resolved.program = compiler_known_io::augment_program(&resolved.program);
    }
    let source_context = testing::SourceSemanticContext::standalone(context.clone());
    let (evaluation, _) = const_eval::evaluate_program_with_diagnostics(&resolved.program);
    let elaboration = testing::elaborate_source(
        &resolved.program,
        &resolved.facts,
        &source_context,
        &evaluation,
    );
    if !elaboration.diagnostics.is_empty() {
        return Err(elaboration.diagnostics);
    }
    Ok(PreparedSource {
        authored,
        resolved: elaboration.program,
        context,
        global_symbols: resolved.facts,
        source_semantic_contexts: std::collections::HashMap::from([(source.id, source_context)]),
        test_semantics: elaboration.facts,
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
