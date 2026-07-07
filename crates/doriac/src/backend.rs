use std::path::PathBuf;
use std::str::FromStr;

use crate::diagnostics::Diagnostic;
use crate::{codegen_native, codegen_php, hir, mir_interpreter, mir_lowering};

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
        Ok(BackendOutput::Text {
            extension: "php".to_string(),
            contents: codegen_php::generate(program),
        })
    }
}

pub struct NativeBackend;

impl Backend for NativeBackend {
    fn target(&self) -> BackendTarget {
        BackendTarget::Native
    }

    fn emit(&self, program: &hir::Program) -> Result<BackendOutput, BackendError> {
        Ok(BackendOutput::Executable {
            extension: native_executable_extension().to_string(),
            bytes: codegen_native::generate_executable(program)?,
        })
    }
}

pub struct DebugBackend;

impl Backend for DebugBackend {
    fn target(&self) -> BackendTarget {
        BackendTarget::Debug
    }

    fn emit(&self, program: &hir::Program) -> Result<BackendOutput, BackendError> {
        let mir = mir_lowering::lower_program(program).map_err(BackendError::from_diagnostics)?;
        let output = mir_interpreter::interpret(&mir)
            .map_err(|error| BackendError::new(format!("MIR interpreter failure: {error}")))?;

        Ok(BackendOutput::Text {
            extension: "debug".to_string(),
            contents: mir_interpreter::render_debug_output(&output),
        })
    }
}

pub fn emit(program: &hir::Program, target: BackendTarget) -> Result<BackendOutput, BackendError> {
    match target {
        BackendTarget::Native => NativeBackend.emit(program),
        BackendTarget::Php => PhpBackend.emit(program),
        BackendTarget::Debug => DebugBackend.emit(program),
        BackendTarget::Wasm => Err(format!(
            "backend `{}` ({}) is planned but not implemented yet",
            target.name(),
            target.description()
        )
        .into()),
    }
}

impl From<String> for BackendError {
    fn from(message: String) -> Self {
        BackendError::new(message)
    }
}

fn native_executable_extension() -> &'static str {
    if cfg!(windows) {
        "exe"
    } else {
        ""
    }
}
