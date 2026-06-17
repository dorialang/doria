use std::path::PathBuf;
use std::str::FromStr;

use crate::{codegen_php, hir};

pub trait Backend {
    fn target(&self) -> BackendTarget;
    fn emit(&self, program: &hir::Program) -> Result<BackendOutput, BackendError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendOutput {
    Text { extension: String, contents: String },
    Binary { extension: String, bytes: Vec<u8> },
    Artifact { path: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendError {
    pub message: String,
}

impl BackendError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
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
        matches!(self, BackendTarget::Php)
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

pub fn emit(program: &hir::Program, target: BackendTarget) -> Result<BackendOutput, BackendError> {
    match target {
        BackendTarget::Php => PhpBackend.emit(program),
        BackendTarget::Native | BackendTarget::Debug | BackendTarget::Wasm => Err(format!(
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
