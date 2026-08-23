use std::path::PathBuf;
use std::str::FromStr;

use crate::diagnostics::Diagnostic;
use crate::source::Span;
use crate::{codegen_native, codegen_php, hir, mir, mir_interpreter, mir_lowering};

pub trait Backend {
    fn target(&self) -> BackendTarget;
    fn emit(&self, program: &hir::Program) -> Result<BackendOutput, BackendError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendOutput {
    Text { extension: String, contents: String },
    Binary { extension: String, bytes: Vec<u8> },
    Executable { extension: String, bytes: Vec<u8> },
    Artifact { path: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendError {
    pub message: String,
    pub diagnostics: Option<Vec<Diagnostic>>,
}

impl BackendError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            diagnostics: None,
        }
    }

    pub fn from_diagnostics(diagnostics: Vec<Diagnostic>) -> Self {
        let message = diagnostics
            .iter()
            .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
            .collect::<Vec<_>>()
            .join("\n");
        Self {
            message,
            diagnostics: Some(diagnostics),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendTarget {
    Native,
    Php,
    Debug,
    Wasm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeProfile {
    Fast,
    Release,
}

impl NativeProfile {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Release => "release",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompileOptions {
    pub target: BackendTarget,
    pub native_profile: NativeProfile,
}

impl CompileOptions {
    pub const fn new(target: BackendTarget) -> Self {
        Self {
            target,
            native_profile: NativeProfile::Fast,
        }
    }

    pub const fn native(native_profile: NativeProfile) -> Self {
        Self {
            target: BackendTarget::Native,
            native_profile,
        }
    }
}

impl BackendTarget {
    pub fn name(self) -> &'static str {
        match self {
            BackendTarget::Native => "native",
            BackendTarget::Php => "php",
            BackendTarget::Debug => "debug",
            BackendTarget::Wasm => "wasm",
        }
    }

    pub fn is_available(self) -> bool {
        matches!(
            self,
            BackendTarget::Native | BackendTarget::Php | BackendTarget::Debug
        )
    }

    pub fn description(self) -> &'static str {
        match self {
            BackendTarget::Native => "native machine code",
            BackendTarget::Php => "PHP compatibility/inspection",
            BackendTarget::Debug => "debug interpreter",
            BackendTarget::Wasm => "WebAssembly",
        }
    }
}

impl FromStr for BackendTarget {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "native" => Ok(BackendTarget::Native),
            "php" => Ok(BackendTarget::Php),
            "debug" => Ok(BackendTarget::Debug),
            "wasm" => Ok(BackendTarget::Wasm),
            _ => Err(format!("unknown backend target `{value}`")),
        }
    }
}

pub struct PhpBackend;

impl Backend for PhpBackend {
    fn target(&self) -> BackendTarget {
        BackendTarget::Php
    }

    fn emit(&self, program: &hir::Program) -> Result<BackendOutput, BackendError> {
        reject_executable_closure_hir_route(program, BackendTarget::Php)?;
        Ok(BackendOutput::Text {
            extension: "php".to_string(),
            contents: codegen_php::generate(program)?,
        })
    }
}

pub struct NativeBackend;

impl Backend for NativeBackend {
    fn target(&self) -> BackendTarget {
        BackendTarget::Native
    }

    fn emit(&self, program: &hir::Program) -> Result<BackendOutput, BackendError> {
        emit_native(program, NativeProfile::Fast)
    }
}

fn emit_native(
    program: &hir::Program,
    native_profile: NativeProfile,
) -> Result<BackendOutput, BackendError> {
    let mir = lower_validated_mir(program)?;
    reject_executable_closure_mir_route(&mir, BackendTarget::Native)?;
    Ok(BackendOutput::Executable {
        extension: native_executable_extension().to_string(),
        bytes: codegen_native::generate_executable(&mir, native_profile)?,
    })
}

pub struct DebugBackend;

impl Backend for DebugBackend {
    fn target(&self) -> BackendTarget {
        BackendTarget::Debug
    }

    fn emit(&self, program: &hir::Program) -> Result<BackendOutput, BackendError> {
        let mir = lower_validated_mir(program)?;
        let output = mir_interpreter::interpret(&mir).map_err(|error| {
            BackendError::from_diagnostics(vec![Diagnostic::new(
                "M1102",
                format!("MIR interpreter failure: {error}"),
                Span::default(),
            )])
        })?;

        Ok(BackendOutput::Text {
            extension: "debug".to_string(),
            contents: mir_interpreter::render_debug_output(&output),
        })
    }
}

fn lower_validated_mir(program: &hir::Program) -> Result<mir::Program, BackendError> {
    let mir = mir_lowering::lower_program(program).map_err(BackendError::from_diagnostics)?;
    crate::mir_validation::validate_program(&mir)?;
    Ok(mir)
}

fn reject_executable_closure_hir_route(
    program: &hir::Program,
    target: BackendTarget,
) -> Result<(), BackendError> {
    let span = program
        .semantic_info
        .closures
        .values()
        .map(|closure| closure.execution_boundary_span)
        .chain(
            program
                .semantic_info
                .callable_value_calls
                .keys()
                .map(|(start, end)| Span::new(*start, *end)),
        )
        .min_by_key(|span| (span.start, span.end));
    reject_executable_closure_span(span, target)
}

fn reject_executable_closure_mir_route(
    program: &mir::Program,
    target: BackendTarget,
) -> Result<(), BackendError> {
    let span = program
        .closure_descriptors
        .iter()
        .map(|descriptor| descriptor.source_span)
        .chain(program.functions.iter().flat_map(|function| {
            function
                .blocks
                .iter()
                .filter_map(|block| match block.terminator {
                    mir::Terminator::IndirectCall { span, .. }
                    | mir::Terminator::CheckedIndirectCall { span, .. } => Some(span),
                    _ => None,
                })
        }))
        .min_by_key(|span| (span.start, span.end));
    reject_executable_closure_span(span, target)
}

fn reject_executable_closure_span(
    span: Option<Span>,
    target: BackendTarget,
) -> Result<(), BackendError> {
    let Some(span) = span else {
        return Ok(());
    };
    let (title, message, explanation) = match target {
        BackendTarget::Native => (
            "Closure Native Execution Is Not Yet Available",
            "native closure execution lands in Stage 30e",
            "Closure semantics, ownership, HIR, MIR, and debug-interpreter execution are implemented. Native runtime and code generation land in Stage 30e.",
        ),
        BackendTarget::Php => (
            "Closure PHP Output Is Not Yet Available",
            "PHP closure lowering lands in Stage 30f",
            "Closure semantics, ownership, HIR, MIR, and debug-interpreter execution are implemented. Explicit PHP compatibility lowering lands in Stage 30f; PHP automatic capture does not define Doria behavior.",
        ),
        BackendTarget::Debug | BackendTarget::Wasm => return Ok(()),
    };
    Err(BackendError::from_diagnostics(vec![
        Diagnostic::unsupported_stage("E0641", message, span)
            .with_title(title)
            .with_explanation(explanation)
            .with_help("execute this source with `--target debug`; no source rewrite is required"),
    ]))
}

pub fn emit(program: &hir::Program, target: BackendTarget) -> Result<BackendOutput, BackendError> {
    emit_with_options(program, CompileOptions::new(target))
}

pub fn emit_with_options(
    program: &hir::Program,
    options: CompileOptions,
) -> Result<BackendOutput, BackendError> {
    if options.target != BackendTarget::Native && options.native_profile == NativeProfile::Release {
        return Err(BackendError::new(
            "--release is only valid for the native target",
        ));
    }

    match options.target {
        BackendTarget::Native => emit_native(program, options.native_profile),
        BackendTarget::Php => PhpBackend.emit(program),
        BackendTarget::Debug => DebugBackend.emit(program),
        BackendTarget::Wasm => Err(format!(
            "backend `{}` ({}) is planned but not implemented yet",
            options.target.name(),
            options.target.description()
        )
        .into()),
    }
}

impl From<String> for BackendError {
    fn from(message: String) -> Self {
        BackendError::new(message)
    }
}

pub(crate) fn native_executable_extension() -> &'static str {
    if cfg!(windows) {
        "exe"
    } else {
        ""
    }
}
