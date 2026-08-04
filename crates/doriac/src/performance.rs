//! Opt-in compiler performance evidence.
//!
//! The ordinary compile path does not construct, serialize, or write a report.
//! This module deliberately owns a separate path so measurement cannot become a
//! hidden cost paid by every compilation.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::backend::{BackendError, BackendOutput, CompileOptions};
use crate::diagnostics::Diagnostic;
use crate::source::SourceFile;
use crate::{codegen_native, lowering, mir_lowering, semantics};

pub const REPORT_SCHEMA_VERSION: u64 = 1;

#[derive(Debug)]
pub struct PerformanceCompilation {
    pub output: BackendOutput,
    pub report: Value,
}

pub fn compile_native(
    path: String,
    text: String,
    options: CompileOptions,
    source_load: Duration,
    command: Vec<String>,
) -> Result<PerformanceCompilation, Vec<Diagnostic>> {
    debug_assert_eq!(options.target, crate::backend::BackendTarget::Native);
    let total_started = Instant::now();
    let source = SourceFile::new(path.clone(), text.clone());

    let started = Instant::now();
    let ast = crate::parse_source_file(&source)?;
    let parse = started.elapsed();
    let ast_item_count = ast.items.len();

    let started = Instant::now();
    let semantic_info = semantics::analyze_program(&ast)?;
    let semantic = started.elapsed();
    let callable_specializations = semantic_info
        .generic_call_specializations
        .iter()
        .filter_map(|(span, specialization)| {
            semantic_info
                .call_targets
                .get(span)
                .map(|target| (target.clone(), specialization.clone()))
        })
        .collect::<HashSet<_>>()
        .len();
    let class_specializations = semantic_info
        .classes
        .iter()
        .filter(|class| !class.arguments.is_empty())
        .count();

    let started = Instant::now();
    let mut hir = lowering::lower_program_with_semantics(&ast, semantic_info)?;
    hir.source_path = source.path.clone();
    hir.source_text = source.text.clone();
    let hir_lowering = started.elapsed();

    let started = Instant::now();
    let mir = mir_lowering::lower_program(&hir)?;
    let mir_lowering = started.elapsed();
    let mir_basic_block_count = mir
        .functions
        .iter()
        .map(|function| function.blocks.len())
        .sum::<usize>();
    let mir_statement_count = mir
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .map(|block| block.statements.len())
        .sum::<usize>();
    let mir_terminator_count = mir_basic_block_count;

    let (bytes, native) =
        codegen_native::generate_executable_with_performance(&mir, options.native_profile)
            .map_err(backend_diagnostics)?;
    let output_size = bytes.len();
    let total = source_load + total_started.elapsed();
    let backend = match options.native_profile {
        crate::backend::NativeProfile::Fast => "cranelift",
        crate::backend::NativeProfile::Release => "llvm",
    };
    let report = json!({
        "schemaVersion": REPORT_SCHEMA_VERSION,
        "compiler": {
            "component": "doriac",
            "toolchainVersion": crate::TOOLCHAIN_VERSION,
            "commit": crate::BUILD_COMMIT
        },
        "command": command,
        "source": {
            "path": path,
            "bytes": text.len(),
            "lines": if text.is_empty() { 0 } else { text.lines().count() }
        },
        "target": options.target.name(),
        "profile": options.native_profile.name(),
        "backend": backend,
        "success": true,
        "totalDurationNs": duration_ns(total),
        "artifacts": {
            "output": {"bytes": output_size},
            "runtime": {"path": native.runtime_artifact_path, "bytes": native.runtime_artifact_bytes}
        },
        "phases": {
            "sourceLoad": available(source_load),
            "lexing": unavailable("integrated into parse"),
            "parse": available(parse),
            "constantEvaluation": unavailable("integrated into semanticAnalysis"),
            "semanticAnalysis": available(semantic),
            "ownershipChecking": unavailable("integrated into semanticAnalysis"),
            "borrowChecking": unavailable("integrated into semanticAnalysis"),
            "hirLowering": available(hir_lowering),
            "mirLowering": available(mir_lowering),
            "mirValidation": available(native.mir_validation),
            "craneliftCodeGeneration": if backend == "cranelift" { available(native.code_generation) } else { unavailable("LLVM profile selected") },
            "llvmCodeGeneration": if backend == "llvm" { available(native.code_generation) } else { unavailable("Cranelift profile selected") },
            "runtimeArtifactSelection": available(native.runtime_selection),
            "objectEmission": unavailable("integrated into the selected code-generation phase"),
            "link": available(native.linking)
        },
        "metrics": {
            "sourceLineCount": if text.is_empty() { 0 } else { text.lines().count() },
            "astItemCount": ast_item_count,
            "outputBytes": output_size,
            "functionCount": mir.functions.len(),
            "classCount": mir.classes.len(),
            "collectionTypeCount": mir.collection_types.len(),
            "mirFunctionCount": mir.functions.len(),
            "mirBasicBlockCount": mir_basic_block_count,
            "mirStatementCount": mir_statement_count,
            "mirTerminatorCount": mir_terminator_count,
            "callableSpecializationCount": callable_specializations,
            "classSpecializationCount": class_specializations,
            "totalGenericSpecializationCount": callable_specializations + class_specializations,
            "runtimeArtifactBytes": native.runtime_artifact_bytes,
            "peakRssBytes": {"available": false, "reason": "portable in-process peak RSS collection is unavailable"}
        }
    });
    Ok(PerformanceCompilation {
        output: BackendOutput::Executable {
            extension: crate::backend::native_executable_extension().to_string(),
            bytes,
        },
        report,
    })
}

fn backend_diagnostics(error: BackendError) -> Vec<Diagnostic> {
    error.diagnostics.unwrap_or_else(|| {
        vec![Diagnostic::new(
            "B0001",
            error.message,
            crate::source::Span::default(),
        )]
    })
}

fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

fn available(duration: Duration) -> Value {
    json!({"available": true, "durationNs": duration_ns(duration)})
}

fn unavailable(reason: &str) -> Value {
    json!({"available": false, "reason": reason})
}
