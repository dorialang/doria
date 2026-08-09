//! Opt-in compiler performance evidence.
//!
//! The ordinary compile path does not construct, serialize, or write a report.
//! This module deliberately owns a separate path so measurement cannot become a
//! hidden cost paid by every compilation.

use std::fs;
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

    let started = Instant::now();
    let semantic_info = semantics::analyze_program(&ast)?;
    let semantic = started.elapsed();

    let started = Instant::now();
    let mut hir = lowering::lower_program_with_semantics(&ast, semantic_info)?;
    hir.source_path = source.path.clone();
    hir.source_text = source.text.clone();
    let hir_lowering = started.elapsed();

    let started = Instant::now();
    let (mir, structure) = mir_lowering::lower_program_with_metrics(&hir)?;
    let mir_lowering = started.elapsed();

    let (bytes, native) =
        codegen_native::generate_executable_with_performance(&mir, options.native_profile)
            .map_err(backend_diagnostics)?;
    let total = source_load + total_started.elapsed();
    let source_line_count = if text.is_empty() {
        0
    } else {
        source
            .line_count()
            .saturating_sub(usize::from(text.ends_with('\n')))
    };
    let ast_item_count = ast.items.len();
    let output_size = bytes.len();
    let runtime_artifact_bytes = fs::metadata(&native.runtime_artifact)
        .ok()
        .map(|metadata| metadata.len());
    // Hashing happens here rather than during selection so ordinary
    // compilation pays nothing for it; a report was explicitly asked for.
    let runtime = native.runtime.provenance();
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
            "lines": source_line_count
        },
        "target": options.target.name(),
        "profile": options.native_profile.name(),
        "backend": backend,
        "linker": {
            "executable": native.linker,
            "command": native.link_command
        },
        "success": true,
        "totalDurationNs": duration_ns(total),
        "artifacts": {
            "output": {"bytes": output_size},
            // Identity of the archive actually linked. A timing result that
            // cannot name its runtime describes an unknown program, which is
            // how a published finding once had to be withdrawn.
            "runtime": {
                "path": runtime.path,
                "origin": runtime.origin,
                "metadataPath": runtime.metadata_path,
                "bytes": runtime.bytes,
                "sha256": runtime.sha256,
                "abiVersion": runtime.abi_version,
                "runtimeRevision": runtime.runtime_revision,
                "targetTriple": runtime.target_triple,
                "profile": runtime.profile,
                "digestMatchesMetadata": runtime.digest_matches_metadata,
                "identified": runtime.abi_version.is_some()
            }
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
            "sourceLineCount": source_line_count,
            "astItemCount": ast_item_count,
            "outputBytes": output_size,
            "functionCount": mir.functions.len(),
            "classCount": mir.classes.len(),
            "collectionTypeCount": mir.collection_types.len(),
            "mirFunctionCount": mir.functions.len(),
            "mirBasicBlockCount": structure.basic_block_count,
            "mirStatementCount": structure.statement_count,
            "mirTerminatorCount": structure.terminator_count,
            "callableSpecializationCount": structure.callable_specialization_count,
            "classSpecializationCount": structure.class_specialization_count,
            "totalGenericSpecializationCount": structure.callable_specialization_count + structure.class_specialization_count,
            "runtimeArtifactBytes": runtime_artifact_bytes,
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
