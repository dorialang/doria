use std::collections::{HashMap, HashSet};
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

use crate::ast::{AttributeTargetKind, AttributeTargetRole};
use crate::build_plan::{GeneratedFor, TargetKind};
use crate::const_eval::ConstValue;
use crate::diagnostics::{Diagnostic, DiagnosticResult};
use crate::names::{GlobalSymbolId, PackageIdentity, SourceIdentity};
use crate::source::Span;
use crate::types::ResolvedType;

pub const ATTRIBUTE_METADATA_SCHEMA_VERSION: u32 = 1;
pub const ATTRIBUTE_PROCESSOR_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AttributeClassIdentity {
    CompilerKnown(String),
    User(GlobalSymbolId),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AttributeTarget {
    GlobalDeclaration {
        declaration: GlobalSymbolId,
        kind: AttributeTargetKind,
    },
    ClassMember {
        class: GlobalSymbolId,
        kind: AttributeTargetKind,
        name: String,
        span: Span,
    },
    CallableParameter {
        callable: String,
        parameter_index: usize,
        parameter_name: String,
        roles: Vec<AttributeTargetRole>,
        span: Span,
    },
    EnumCase {
        enumeration: GlobalSymbolId,
        case_index: usize,
        case_name: String,
        span: Span,
    },
    EnumPayloadField {
        enumeration: GlobalSymbolId,
        case_index: usize,
        field_index: usize,
        field_name: String,
        span: Span,
    },
}

impl AttributeTarget {
    pub fn canonical_key(&self) -> String {
        match self {
            Self::GlobalDeclaration { declaration, kind } => {
                format!(
                    "global:{}:{}",
                    declaration.qualified_name,
                    kind.protocol_name()
                )
            }
            Self::ClassMember {
                class, kind, name, ..
            } => format!(
                "member:{}:{}:{name}",
                class.qualified_name,
                kind.protocol_name()
            ),
            Self::CallableParameter {
                callable,
                parameter_index,
                roles,
                ..
            } => format!(
                "parameter:{callable}:{parameter_index}:{}",
                roles
                    .iter()
                    .map(|role| role.protocol_name())
                    .collect::<Vec<_>>()
                    .join("+")
            ),
            Self::EnumCase {
                enumeration,
                case_index,
                ..
            } => format!("enum-case:{}:{case_index}", enumeration.qualified_name),
            Self::EnumPayloadField {
                enumeration,
                case_index,
                field_index,
                ..
            } => format!(
                "enum-payload:{}:{case_index}:{field_index}",
                enumeration.qualified_name
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeSchemaParameter {
    pub index: usize,
    pub name: String,
    pub ty: ResolvedType,
    pub has_default: bool,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeClassSchema {
    pub identity: AttributeClassIdentity,
    pub canonical_name: String,
    pub source: Option<SourceIdentity>,
    pub package: PackageIdentity,
    pub declaration_span: Option<Span>,
    pub parameters: Vec<AttributeSchemaParameter>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeAuthoredArgument {
    pub index: usize,
    pub name: Option<String>,
    pub span: Span,
    pub bound_parameter_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeBoundArgument {
    pub parameter_index: usize,
    pub parameter_name: String,
    pub ty: ResolvedType,
    pub value: AttributeValue,
    pub defaulted: bool,
    pub authored_argument_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeApplication {
    pub identity: String,
    pub class_identity: AttributeClassIdentity,
    pub canonical_class_name: String,
    pub target: AttributeTarget,
    pub source: SourceIdentity,
    pub package: PackageIdentity,
    pub group_ordinal: usize,
    pub application_ordinal: usize,
    pub authored_arguments: Vec<AttributeAuthoredArgument>,
    pub bound_arguments: Vec<AttributeBoundArgument>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeValue {
    pub semantic_type: ResolvedType,
    pub value: AttributeValueKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttributeValueKind {
    Integer {
        value: String,
    },
    Float {
        value: String,
    },
    String(String),
    Bool(bool),
    Null,
    Enum {
        case: String,
    },
    PayloadEnum {
        case: String,
        fields: Vec<AttributeValue>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttributeSemanticInfo {
    pub schemas: Vec<AttributeClassSchema>,
    pub applications: Vec<AttributeApplication>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttributeMetadataDocumentV1 {
    pub schema_version: u32,
    pub edition: String,
    pub compiler_revision: String,
    pub graph_fingerprint: String,
    pub selected_target: MetadataTargetV1,
    pub packages: Vec<MetadataPackageV1>,
    pub sources: Vec<MetadataSourceV1>,
    pub attribute_classes: Vec<MetadataAttributeClassV1>,
    pub applications: Vec<MetadataApplicationV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataTargetV1 {
    pub package: String,
    pub kind: String,
    pub entry_source: Option<String>,
}

impl MetadataTargetV1 {
    pub fn from_parts(
        package: &PackageIdentity,
        kind: TargetKind,
        entry_source: Option<&SourceIdentity>,
    ) -> Self {
        Self {
            package: package.display_name().to_string(),
            kind: match kind {
                TargetKind::Binary => "binary",
                TargetKind::Library => "library",
            }
            .to_string(),
            entry_source: entry_source.map(|source| public_source_identity(&source.0)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataPackageV1 {
    pub identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataSourceV1 {
    pub identity: String,
    pub package: String,
    pub display_path: String,
    pub byte_length: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataLocationV1 {
    pub source: String,
    pub display_path: String,
    pub byte_start: usize,
    pub byte_end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataAttributeClassV1 {
    pub identity: String,
    pub canonical_name: String,
    pub package: String,
    pub source: Option<String>,
    pub parameters: Vec<MetadataSchemaParameterV1>,
    pub location: Option<MetadataLocationV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataSchemaParameterV1 {
    pub index: usize,
    pub name: String,
    pub r#type: String,
    pub has_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataApplicationV1 {
    pub identity: String,
    pub attribute_class: String,
    pub target: String,
    pub source: String,
    pub package: String,
    pub group_ordinal: usize,
    pub application_ordinal: usize,
    pub authored_arguments: Vec<MetadataAuthoredArgumentV1>,
    pub bound_arguments: Vec<MetadataBoundArgumentV1>,
    pub location: MetadataLocationV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataAuthoredArgumentV1 {
    pub index: usize,
    pub name: Option<String>,
    pub bound_parameter_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataBoundArgumentV1 {
    pub parameter_index: usize,
    pub parameter_name: String,
    pub r#type: String,
    pub value: MetadataValueV1,
    pub defaulted: bool,
    pub authored_argument_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum MetadataValueV1 {
    Integer {
        r#type: String,
        value: String,
    },
    Float {
        r#type: String,
        value: String,
    },
    String {
        r#type: String,
        value: String,
    },
    Bool {
        r#type: String,
        value: bool,
    },
    Null {
        r#type: String,
    },
    Enum {
        r#type: String,
        case: String,
    },
    PayloadEnum {
        r#type: String,
        case: String,
        fields: Vec<MetadataValueV1>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttributeProcessorRequestV1 {
    pub schema_version: u32,
    pub edition: String,
    pub compiler_revision: String,
    pub graph_fingerprint: String,
    pub processor_package: String,
    pub selected_target: MetadataTargetV1,
    pub sources: Vec<MetadataSourceV1>,
    pub attribute_classes: Vec<MetadataAttributeClassV1>,
    pub applications: Vec<MetadataApplicationV1>,
}

impl AttributeProcessorRequestV1 {
    pub fn from_metadata(
        document: &AttributeMetadataDocumentV1,
        processor_package: impl Into<String>,
    ) -> DiagnosticResult<Self> {
        let processor_package = processor_package.into();
        validate_processor_package(&processor_package)?;
        Ok(Self {
            schema_version: ATTRIBUTE_PROCESSOR_SCHEMA_VERSION,
            edition: document.edition.clone(),
            compiler_revision: document.compiler_revision.clone(),
            graph_fingerprint: document.graph_fingerprint.clone(),
            processor_package,
            selected_target: document.selected_target.clone(),
            sources: document.sources.clone(),
            attribute_classes: document.attribute_classes.clone(),
            applications: document.applications.clone(),
        })
    }
}

pub fn validate_processor_request(request: &AttributeProcessorRequestV1) -> DiagnosticResult<()> {
    let mut diagnostics = Vec::new();
    if request.schema_version != ATTRIBUTE_PROCESSOR_SCHEMA_VERSION {
        diagnostics.push(protocol_diagnostic(format!(
            "unsupported attribute processor schema version `{}`",
            request.schema_version
        )));
    }
    if request.edition.is_empty()
        || request.compiler_revision.is_empty()
        || request.graph_fingerprint.is_empty()
    {
        diagnostics.push(protocol_diagnostic(
            "attribute processor request provenance fields must not be empty",
        ));
    }
    if let Err(mut package_diagnostics) = validate_processor_package(&request.processor_package) {
        diagnostics.append(&mut package_diagnostics);
    }
    let mut source_identities = HashSet::new();
    for source in &request.sources {
        if source.identity.is_empty()
            || source.package.is_empty()
            || source.display_path.is_empty()
            || !source_identities.insert(source.identity.as_str())
        {
            diagnostics.push(protocol_diagnostic(
                "attribute processor request contains an invalid or duplicate source inventory entry",
            ));
        }
    }
    for application in &request.applications {
        for argument in &application.bound_arguments {
            if argument.r#type != metadata_value_type(&argument.value) {
                diagnostics.push(protocol_diagnostic(format!(
                    "attribute argument `{}` has inconsistent metadata type identities",
                    argument.parameter_name
                )));
                continue;
            }
            if let Err(message) = validate_metadata_value(&argument.value) {
                diagnostics.push(protocol_diagnostic(message));
            }
        }
    }

    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

pub fn parse_processor_request_json(bytes: &[u8]) -> DiagnosticResult<AttributeProcessorRequestV1> {
    let request =
        serde_json::from_slice::<AttributeProcessorRequestV1>(bytes).map_err(|error| {
            vec![protocol_diagnostic(format!(
                "invalid attribute processor request JSON: {error}"
            ))]
        })?;
    validate_processor_request(&request)?;
    Ok(request)
}

pub fn parse_processor_response_json(
    bytes: &[u8],
    request: &AttributeProcessorRequestV1,
    handwritten_sources: &[String],
) -> DiagnosticResult<AttributeProcessorResponseV1> {
    let response =
        serde_json::from_slice::<AttributeProcessorResponseV1>(bytes).map_err(|error| {
            vec![protocol_diagnostic(format!(
                "invalid attribute processor response JSON: {error}"
            ))]
        })?;
    validate_processor_response(&response, request, handwritten_sources)?;
    Ok(response)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttributeProcessorResponseV1 {
    pub schema_version: u32,
    pub graph_fingerprint: String,
    pub diagnostics: Vec<ProcessorDiagnosticV1>,
    pub generated_sources: Vec<GeneratedSourceV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessorDiagnosticV1 {
    pub code: String,
    pub title: String,
    pub severity: ProcessorDiagnosticSeverityV1,
    pub message: String,
    pub labels: Vec<ProcessorDiagnosticLabelV1>,
    pub explanation: Option<String>,
    pub help: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProcessorDiagnosticSeverityV1 {
    Error,
    Warning,
    Information,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessorDiagnosticLabelV1 {
    pub source: String,
    pub byte_start: usize,
    pub byte_end: usize,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GeneratedSourceV1 {
    pub relative_path: String,
    pub generated_for: GeneratedForV1,
    pub contents: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GeneratedForV1 {
    Main,
    Development,
}

impl From<GeneratedFor> for GeneratedForV1 {
    fn from(value: GeneratedFor) -> Self {
        match value {
            GeneratedFor::Main => Self::Main,
            GeneratedFor::Development => Self::Development,
        }
    }
}

pub fn validate_processor_response(
    response: &AttributeProcessorResponseV1,
    request: &AttributeProcessorRequestV1,
    handwritten_sources: &[String],
) -> DiagnosticResult<()> {
    let mut diagnostics = Vec::new();
    if response.schema_version != ATTRIBUTE_PROCESSOR_SCHEMA_VERSION {
        diagnostics.push(protocol_diagnostic(format!(
            "unsupported attribute processor schema version `{}`",
            response.schema_version
        )));
    }
    if response.graph_fingerprint != request.graph_fingerprint {
        diagnostics.push(protocol_diagnostic(
            "attribute processor response graph fingerprint does not match the request",
        ));
    }

    let handwritten = handwritten_sources
        .iter()
        .map(|path| path.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    let source_lengths = request
        .sources
        .iter()
        .map(|source| (source.identity.as_str(), source.byte_length))
        .collect::<HashMap<_, _>>();
    let mut exact = HashSet::new();
    let mut folded = HashSet::new();
    for source in &response.generated_sources {
        if let Err(message) = validate_generated_path(&source.relative_path) {
            diagnostics.push(protocol_diagnostic(message));
            continue;
        }
        let folded_path = source.relative_path.to_ascii_lowercase();
        if !exact.insert(source.relative_path.clone()) {
            diagnostics.push(protocol_diagnostic(format!(
                "duplicate generated source path `{}`",
                source.relative_path
            )));
        } else if !folded.insert(folded_path.clone()) {
            diagnostics.push(protocol_diagnostic(format!(
                "generated source path `{}` collides by case with another output",
                source.relative_path
            )));
        }
        if handwritten.contains(&folded_path) {
            diagnostics.push(protocol_diagnostic(format!(
                "generated source path `{}` would overwrite handwritten source",
                source.relative_path
            )));
        }
        let actual_hash = crate::runtime_digest::sha256_hex(source.contents.as_bytes());
        if source.content_hash != actual_hash {
            diagnostics.push(protocol_diagnostic(format!(
                "generated source `{}` has an invalid content hash",
                source.relative_path
            )));
        }
    }

    for diagnostic in &response.diagnostics {
        if doria_diagnostic_catalogue::DIAGNOSTIC_CODES.contains(&diagnostic.code.as_str()) {
            diagnostics.push(protocol_diagnostic(format!(
                "processor diagnostic code `{}` is reserved by the compiler",
                diagnostic.code
            )));
        }
        if !is_title_case(&diagnostic.title) {
            diagnostics.push(protocol_diagnostic(format!(
                "processor diagnostic title `{}` is not Title Case",
                diagnostic.title
            )));
        }
        if diagnostic.code.is_empty()
            || diagnostic.message.is_empty()
            || diagnostic
                .labels
                .iter()
                .any(|label| label.source.is_empty() || label.byte_start > label.byte_end)
        {
            diagnostics.push(protocol_diagnostic(format!(
                "processor diagnostic `{}` is missing required structured information",
                diagnostic.code
            )));
        }
        for label in &diagnostic.labels {
            match source_lengths.get(label.source.as_str()) {
                Some(byte_length) if label.byte_end <= *byte_length => {}
                Some(_) => diagnostics.push(protocol_diagnostic(format!(
                    "processor diagnostic `{}` has a label outside source `{}`",
                    diagnostic.code, label.source
                ))),
                None if !label.source.is_empty() => diagnostics.push(protocol_diagnostic(format!(
                    "processor diagnostic `{}` refers to unknown source `{}`",
                    diagnostic.code, label.source
                ))),
                None => {}
            }
        }
        if contains_unsafe_terminal_text(&diagnostic.code)
            || contains_unsafe_terminal_text(&diagnostic.title)
            || contains_unsafe_terminal_text(&diagnostic.message)
            || diagnostic
                .explanation
                .as_deref()
                .is_some_and(contains_unsafe_terminal_text)
            || diagnostic
                .help
                .as_deref()
                .is_some_and(contains_unsafe_terminal_text)
            || diagnostic.labels.iter().any(|label| {
                contains_unsafe_terminal_text(&label.source)
                    || contains_unsafe_terminal_text(&label.message)
            })
        {
            diagnostics.push(protocol_diagnostic(format!(
                "processor diagnostic `{}` contains unsafe terminal control bytes",
                diagnostic.code
            )));
        }
    }

    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

fn validate_processor_package(value: &str) -> DiagnosticResult<()> {
    let mut parts = value.split('/');
    let valid_part = |part: &str| {
        !part.is_empty()
            && part.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
            })
    };
    let valid = parts.next().is_some_and(valid_part)
        && parts.next().is_some_and(valid_part)
        && parts.next().is_none();
    if valid {
        Ok(())
    } else {
        Err(vec![protocol_diagnostic(format!(
            "invalid processor package identity `{value}`"
        ))])
    }
}

fn validate_generated_path(value: &str) -> Result<(), String> {
    if value.is_empty() || value.contains('\0') || value.contains("://") || value.contains('\\') {
        return Err(format!("invalid generated source path `{value}`"));
    }
    let path = Path::new(value);
    if path.is_absolute() || value.as_bytes().get(1) == Some(&b':') {
        return Err(format!(
            "generated source path `{value}` must be package-relative"
        ));
    }
    let mut has_component = false;
    for component in path.components() {
        match component {
            Component::Normal(_) => has_component = true,
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(format!("generated source path `{value}` is not normalized"));
            }
        }
    }
    if !has_component {
        return Err(format!("invalid generated source path `{value}`"));
    }
    let normalized = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => part.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    if normalized != value {
        return Err(format!("generated source path `{value}` is not normalized"));
    }
    if path.extension().and_then(|extension| extension.to_str()) != Some("doria") {
        return Err(format!(
            "generated source path `{value}` must name a `.doria` source file"
        ));
    }
    Ok(())
}

fn metadata_value_type(value: &MetadataValueV1) -> &str {
    match value {
        MetadataValueV1::Integer { r#type, .. }
        | MetadataValueV1::Float { r#type, .. }
        | MetadataValueV1::String { r#type, .. }
        | MetadataValueV1::Bool { r#type, .. }
        | MetadataValueV1::Null { r#type }
        | MetadataValueV1::Enum { r#type, .. }
        | MetadataValueV1::PayloadEnum { r#type, .. } => r#type,
    }
}

fn validate_metadata_value(value: &MetadataValueV1) -> Result<(), String> {
    let ty = metadata_value_type(value);
    let base = ty.strip_prefix('?').unwrap_or(ty);
    match value {
        MetadataValueV1::Integer { value, .. } => {
            let Some(integer_type) = crate::numeric::IntegerType::from_source_name(base) else {
                return Err(format!("metadata integer uses non-integer type `{ty}`"));
            };
            let (negative, magnitude) = value
                .strip_prefix('-')
                .map_or((false, value.as_str()), |magnitude| (true, magnitude));
            let magnitude = crate::numeric::parse_decimal_magnitude(magnitude)
                .ok_or_else(|| format!("metadata integer `{value}` is malformed or oversized"))?;
            let parsed =
                crate::numeric::IntegerValue::from_literal(integer_type, magnitude, negative)
                    .ok_or_else(|| format!("metadata integer `{value}` is outside `{ty}` range"))?;
            if parsed.display() != *value {
                return Err(format!("metadata integer `{value}` is not canonical"));
            }
        }
        MetadataValueV1::Float { value, .. } => {
            let Some(float_type) = crate::numeric::FloatType::from_source_name(base) else {
                return Err(format!("metadata float uses non-float type `{ty}`"));
            };
            let parsed = match value.as_str() {
                "NaN" => crate::numeric::FloatValue::from_f64(f64::NAN),
                "Infinity" => crate::numeric::FloatValue::from_f64(f64::INFINITY),
                "-Infinity" => crate::numeric::FloatValue::from_f64(f64::NEG_INFINITY),
                _ => crate::numeric::FloatValue::parse_decimal(float_type, value)
                    .ok_or_else(|| format!("metadata float `{value}` is malformed"))?,
            };
            let parsed = match float_type {
                crate::numeric::FloatType::Float32 => {
                    crate::numeric::FloatValue::from_f32(parsed.as_f64() as f32)
                }
                crate::numeric::FloatType::Float64 => parsed,
            };
            if parsed.display() != *value {
                return Err(format!("metadata float `{value}` is not canonical"));
            }
        }
        MetadataValueV1::String { .. } if base != "string" => {
            return Err(format!("metadata string uses non-string type `{ty}`"));
        }
        MetadataValueV1::Bool { .. } if base != "bool" => {
            return Err(format!("metadata boolean uses non-bool type `{ty}`"));
        }
        MetadataValueV1::Null { .. } if !ty.starts_with('?') => {
            return Err(format!(
                "metadata null requires a nullable type, found `{ty}`"
            ));
        }
        MetadataValueV1::Enum { case, .. } | MetadataValueV1::PayloadEnum { case, .. }
            if base.is_empty() || case.is_empty() =>
        {
            return Err("metadata enum identity and case must not be empty".to_string());
        }
        MetadataValueV1::PayloadEnum { fields, .. } => {
            for field in fields {
                validate_metadata_value(field)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn protocol_diagnostic(message: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E0699", message, Span::default())
        .with_title("Invalid Attribute Processor Protocol")
}

fn is_title_case(value: &str) -> bool {
    value.split_whitespace().all(|word| {
        !word.chars().any(char::is_alphabetic)
            || word.chars().next().is_some_and(char::is_uppercase)
    })
}

fn contains_unsafe_terminal_text(value: &str) -> bool {
    value
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
}

pub fn metadata_type_name(ty: &ResolvedType) -> String {
    match ty {
        ResolvedType::Void => "void".to_string(),
        ResolvedType::Integer(ty) => ty.source_name().to_string(),
        ResolvedType::Float(ty) => ty.source_name().to_string(),
        ResolvedType::String => "string".to_string(),
        ResolvedType::Bool => "bool".to_string(),
        ResolvedType::Null => "null".to_string(),
        ResolvedType::Enum(ty) => ty.name.clone(),
        ResolvedType::Nullable(inner) => format!("?{}", metadata_type_name(inner)),
        ResolvedType::Bytes => "Bytes".to_string(),
        ResolvedType::Mixed => "mixed".to_string(),
        ResolvedType::Error => "Error".to_string(),
        ResolvedType::TypeParameter(name) => name.clone(),
        ResolvedType::Function(_) => "function".to_string(),
        ResolvedType::Class(class) => class.name.clone(),
        ResolvedType::TypedArray(inner) => format!("{}[]", metadata_type_name(inner)),
        ResolvedType::List(inner) => format!("List<{}>", metadata_type_name(inner)),
        ResolvedType::Dictionary(key, value) => format!(
            "Dictionary<{}, {}>",
            metadata_type_name(key),
            metadata_type_name(value)
        ),
        ResolvedType::SortedDictionary(key, value) => format!(
            "SortedDictionary<{}, {}>",
            metadata_type_name(key),
            metadata_type_name(value)
        ),
        ResolvedType::Set(inner) => format!("Set<{}>", metadata_type_name(inner)),
        ResolvedType::SortedSet(inner) => format!("SortedSet<{}>", metadata_type_name(inner)),
        ResolvedType::PriorityQueue(inner) => {
            format!("PriorityQueue<{}>", metadata_type_name(inner))
        }
        ResolvedType::Deque(inner) => format!("Deque<{}>", metadata_type_name(inner)),
        ResolvedType::SharedHandle(kind, inner) => {
            format!("{}<{}>", kind.source_name(), metadata_type_name(inner))
        }
        ResolvedType::Unsupported => "Unknown".to_string(),
    }
}

pub fn attribute_value_from_const(
    semantic_type: ResolvedType,
    value: ConstValue,
    evaluation: &crate::const_eval::Evaluation,
) -> Option<AttributeValue> {
    let value = match value {
        ConstValue::Integer(value) => AttributeValueKind::Integer {
            value: value.display(),
        },
        ConstValue::Float(value) => AttributeValueKind::Float {
            value: value.display(),
        },
        ConstValue::String(value) => AttributeValueKind::String(value),
        ConstValue::Bool(value) => AttributeValueKind::Bool(value),
        ConstValue::Null => AttributeValueKind::Null,
        ConstValue::Enum(value) => {
            let (_, case) = evaluation.enum_case_name(value)?;
            AttributeValueKind::Enum {
                case: case.to_string(),
            }
        }
        ConstValue::PayloadEnum(value) => {
            let (_, case) = evaluation.payload_case_name(value.enum_id, value.case_id)?;
            let fields = value
                .fields
                .into_iter()
                .zip(value.field_types)
                .map(|(field, ty)| {
                    let resolved = resolved_const_type(&ty, evaluation)?;
                    attribute_value_from_const(resolved, field, evaluation)
                })
                .collect::<Option<Vec<_>>>()?;
            AttributeValueKind::PayloadEnum {
                case: case.to_string(),
                fields,
            }
        }
    };
    Some(AttributeValue {
        semantic_type,
        value,
    })
}

fn resolved_const_type(
    ty: &crate::types::TypeRef,
    evaluation: &crate::const_eval::Evaluation,
) -> Option<ResolvedType> {
    let base = match ty.name.as_str() {
        "string" => ResolvedType::String,
        "bool" => ResolvedType::Bool,
        name if crate::numeric::IntegerType::from_source_name(name).is_some() => {
            ResolvedType::Integer(crate::numeric::IntegerType::from_source_name(name)?)
        }
        name if crate::numeric::FloatType::from_source_name(name).is_some() => {
            ResolvedType::Float(crate::numeric::FloatType::from_source_name(name)?)
        }
        name => ResolvedType::Enum(crate::enums::EnumType {
            id: *evaluation.enum_names.get(name)?,
            name: name.to_string(),
        }),
    };
    Some(if ty.nullable {
        ResolvedType::Nullable(Box::new(base))
    } else {
        base
    })
}

impl AttributeValue {
    pub fn to_metadata_value(&self) -> MetadataValueV1 {
        let ty = metadata_type_name(&self.semantic_type);
        match &self.value {
            AttributeValueKind::Integer { value } => MetadataValueV1::Integer {
                r#type: ty,
                value: value.clone(),
            },
            AttributeValueKind::Float { value } => MetadataValueV1::Float {
                r#type: ty,
                value: value.clone(),
            },
            AttributeValueKind::String(value) => MetadataValueV1::String {
                r#type: ty,
                value: value.clone(),
            },
            AttributeValueKind::Bool(value) => MetadataValueV1::Bool {
                r#type: ty,
                value: *value,
            },
            AttributeValueKind::Null => MetadataValueV1::Null { r#type: ty },
            AttributeValueKind::Enum { case } => MetadataValueV1::Enum {
                r#type: ty,
                case: case.clone(),
            },
            AttributeValueKind::PayloadEnum { case, fields } => MetadataValueV1::PayloadEnum {
                r#type: ty,
                case: case.clone(),
                fields: fields.iter().map(Self::to_metadata_value).collect(),
            },
        }
    }
}

pub fn metadata_document(
    hir: &crate::hir::Program,
    graph_fingerprint: impl Into<String>,
) -> AttributeMetadataDocumentV1 {
    let mut sources = hir
        .sources
        .iter()
        .map(|source| MetadataSourceV1 {
            identity: public_source_identity(&source.identity.0),
            package: source.package.display_name().to_string(),
            display_path: public_display_path(&source.display_path),
            byte_length: source.source.text.len(),
        })
        .collect::<Vec<_>>();
    sources.sort_by(|left, right| {
        (left.package.as_str(), left.identity.as_str())
            .cmp(&(right.package.as_str(), right.identity.as_str()))
    });
    let mut packages = hir
        .packages
        .iter()
        .map(|package| MetadataPackageV1 {
            identity: package.identity.display_name().to_string(),
        })
        .collect::<Vec<_>>();
    packages.sort_by(|left, right| left.identity.cmp(&right.identity));

    let location_for = |span: Span, source_identity: Option<&SourceIdentity>| {
        let source = hir.sources.iter().find(|source| source.id == span.source);
        let identity = source
            .map(|source| public_source_identity(&source.identity.0))
            .or_else(|| source_identity.map(|source| public_source_identity(&source.0)))
            .unwrap_or_else(|| "<unknown>".to_string());
        let display_path = source
            .map(|source| public_display_path(&source.display_path))
            .unwrap_or_else(|| identity.clone());
        MetadataLocationV1 {
            source: identity,
            display_path,
            byte_start: span.start,
            byte_end: span.end,
        }
    };

    let mut attribute_classes = hir
        .attribute_metadata
        .schemas
        .iter()
        .map(|schema| MetadataAttributeClassV1 {
            identity: attribute_class_identity(&schema.identity),
            canonical_name: schema.canonical_name.clone(),
            package: schema.package.display_name().to_string(),
            source: schema
                .source
                .as_ref()
                .map(|source| public_source_identity(&source.0)),
            parameters: schema
                .parameters
                .iter()
                .map(|parameter| MetadataSchemaParameterV1 {
                    index: parameter.index,
                    name: parameter.name.clone(),
                    r#type: metadata_type_name(&parameter.ty),
                    has_default: parameter.has_default,
                })
                .collect(),
            location: schema
                .declaration_span
                .map(|span| location_for(span, schema.source.as_ref())),
        })
        .collect::<Vec<_>>();
    attribute_classes.sort_by(|left, right| left.identity.cmp(&right.identity));

    let mut applications = hir
        .attribute_metadata
        .applications
        .iter()
        .map(|application| MetadataApplicationV1 {
            identity: format!(
                "{}#{}:{}:{}",
                public_source_identity(&application.source.0),
                application.target.canonical_key(),
                application.group_ordinal,
                application.application_ordinal
            ),
            attribute_class: attribute_class_identity(&application.class_identity),
            target: application.target.canonical_key(),
            source: public_source_identity(&application.source.0),
            package: application.package.display_name().to_string(),
            group_ordinal: application.group_ordinal,
            application_ordinal: application.application_ordinal,
            authored_arguments: application
                .authored_arguments
                .iter()
                .map(|argument| MetadataAuthoredArgumentV1 {
                    index: argument.index,
                    name: argument.name.clone(),
                    bound_parameter_index: argument.bound_parameter_index,
                })
                .collect(),
            bound_arguments: application
                .bound_arguments
                .iter()
                .map(|argument| MetadataBoundArgumentV1 {
                    parameter_index: argument.parameter_index,
                    parameter_name: argument.parameter_name.clone(),
                    r#type: metadata_type_name(&argument.ty),
                    value: argument.value.to_metadata_value(),
                    defaulted: argument.defaulted,
                    authored_argument_index: argument.authored_argument_index,
                })
                .collect(),
            location: location_for(application.span, Some(&application.source)),
        })
        .collect::<Vec<_>>();
    applications.sort_by(|left, right| {
        (
            left.package.as_str(),
            left.source.as_str(),
            left.location.byte_start,
            left.group_ordinal,
            left.application_ordinal,
        )
            .cmp(&(
                right.package.as_str(),
                right.source.as_str(),
                right.location.byte_start,
                right.group_ordinal,
                right.application_ordinal,
            ))
    });

    AttributeMetadataDocumentV1 {
        schema_version: ATTRIBUTE_METADATA_SCHEMA_VERSION,
        edition: hir
            .semantic_info
            .compilation_context
            .edition
            .source_name()
            .to_string(),
        compiler_revision: crate::BUILD_COMMIT.to_string(),
        graph_fingerprint: graph_fingerprint.into(),
        selected_target: MetadataTargetV1::from_parts(
            &hir.selected_target.package,
            hir.selected_target.kind,
            hir.selected_target.entry_source.as_ref(),
        ),
        packages,
        sources,
        attribute_classes,
        applications,
    }
}

fn attribute_class_identity(identity: &AttributeClassIdentity) -> String {
    match identity {
        AttributeClassIdentity::CompilerKnown(name) => format!("compiler-known:{name}"),
        AttributeClassIdentity::User(id) => id.qualified_name.clone(),
    }
}

fn public_source_identity(identity: &str) -> String {
    if Path::new(identity).is_absolute() {
        Path::new(identity)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("source.doria")
            .to_string()
    } else {
        identity.to_string()
    }
}

fn public_display_path(path: &str) -> String {
    if Path::new(path).is_absolute() {
        Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("source.doria")
            .to_string()
    } else {
        path.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn processor_request(sources: Vec<MetadataSourceV1>) -> AttributeProcessorRequestV1 {
        AttributeProcessorRequestV1 {
            schema_version: ATTRIBUTE_PROCESSOR_SCHEMA_VERSION,
            edition: "2026".to_string(),
            compiler_revision: "revision".to_string(),
            graph_fingerprint: "graph".to_string(),
            processor_package: "acme/routes".to_string(),
            selected_target: MetadataTargetV1 {
                package: "acme/application".to_string(),
                kind: "binary".to_string(),
                entry_source: None,
            },
            sources,
            attribute_classes: Vec::new(),
            applications: Vec::new(),
        }
    }

    fn processor_source(identity: &str, byte_length: usize) -> MetadataSourceV1 {
        MetadataSourceV1 {
            identity: identity.to_string(),
            package: "acme/application".to_string(),
            display_path: "main.doria".to_string(),
            byte_length,
        }
    }

    #[test]
    fn processor_response_rejects_unsafe_and_colliding_outputs() {
        let contents = "function generated(): void {}".to_string();
        let hash = crate::runtime_digest::sha256_hex(contents.as_bytes());
        let response = AttributeProcessorResponseV1 {
            schema_version: 1,
            graph_fingerprint: "graph".to_string(),
            diagnostics: Vec::new(),
            generated_sources: vec![
                GeneratedSourceV1 {
                    relative_path: "generated/Main.doria".to_string(),
                    generated_for: GeneratedForV1::Main,
                    contents: contents.clone(),
                    content_hash: hash.clone(),
                },
                GeneratedSourceV1 {
                    relative_path: "generated/main.doria".to_string(),
                    generated_for: GeneratedForV1::Main,
                    contents,
                    content_hash: hash,
                },
                GeneratedSourceV1 {
                    relative_path: "../escape.doria".to_string(),
                    generated_for: GeneratedForV1::Main,
                    contents: String::new(),
                    content_hash: crate::runtime_digest::sha256_hex(b""),
                },
            ],
        };
        let diagnostics =
            validate_processor_response(&response, &processor_request(Vec::new()), &[])
                .unwrap_err();
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("collides by case")));
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("not normalized")));
    }

    #[test]
    fn protocol_deserialization_rejects_unknown_fields() {
        let json = r#"{
            "schemaVersion": 1,
            "graphFingerprint": "graph",
            "diagnostics": [],
            "generatedSources": [],
            "unexpected": true
        }"#;
        assert!(serde_json::from_str::<AttributeProcessorResponseV1>(json).is_err());
    }

    #[test]
    fn processor_request_parser_enforces_version_package_and_typed_values() {
        let valid = r#"{
            "schemaVersion": 1,
            "edition": "2026",
            "compilerRevision": "revision",
            "graphFingerprint": "graph",
            "processorPackage": "acme/routes",
            "selectedTarget": {
                "package": "acme/application",
                "kind": "binary",
                "entrySource": "acme/application:main.doria"
            },
            "sources": [],
            "attributeClasses": [],
            "applications": []
        }"#;
        let request = parse_processor_request_json(valid.as_bytes()).expect("valid request");
        assert_eq!(request.processor_package, "acme/routes");

        let future = valid.replace("\"schemaVersion\": 1", "\"schemaVersion\": 2");
        assert!(parse_processor_request_json(future.as_bytes()).is_err());
        let invalid_package = valid.replace("acme/routes", "Acme Routes");
        assert!(parse_processor_request_json(invalid_package.as_bytes()).is_err());
        assert!(parse_processor_request_json(&[0xff, 0xfe]).is_err());
    }

    #[test]
    fn processor_response_parser_rejects_cross_platform_paths_and_unsafe_diagnostics() {
        let response = r#"{
            "schemaVersion": 1,
            "graphFingerprint": "graph",
            "diagnostics": [{
                "code": "ROUTE001",
                "title": "Invalid Route",
                "severity": "error",
                "message": "unsafe\u001b[31m",
                "labels": [{
                    "source": "acme/application:main.doria",
                    "byteStart": 10,
                    "byteEnd": 5,
                    "message": "route"
                }],
                "explanation": null,
                "help": null
            }],
            "generatedSources": [{
                "relativePath": "..\\escape.doria",
                "generatedFor": "main",
                "contents": "",
                "contentHash": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
            }]
        }"#;
        let request = processor_request(vec![processor_source("acme/application:main.doria", 20)]);
        let diagnostics = parse_processor_response_json(response.as_bytes(), &request, &[])
            .expect_err("unsafe response must be rejected");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("generated source path")));
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("unsafe terminal control bytes")));
        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("missing required structured information")));
    }

    #[test]
    fn processor_response_accepts_only_doria_source_outputs() {
        let response = AttributeProcessorResponseV1 {
            schema_version: ATTRIBUTE_PROCESSOR_SCHEMA_VERSION,
            graph_fingerprint: "graph".to_string(),
            diagnostics: Vec::new(),
            generated_sources: ["Baton.toml", "Cargo.toml", "generated/program.bin"]
                .into_iter()
                .map(|relative_path| GeneratedSourceV1 {
                    relative_path: relative_path.to_string(),
                    generated_for: GeneratedForV1::Main,
                    contents: String::new(),
                    content_hash: crate::runtime_digest::sha256_hex(b""),
                })
                .collect(),
        };

        let diagnostics =
            validate_processor_response(&response, &processor_request(Vec::new()), &[])
                .expect_err("non-source processor outputs must be rejected");
        assert_eq!(
            diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.message.contains("`.doria` source file"))
                .count(),
            3
        );
    }

    #[test]
    fn processor_diagnostic_labels_are_bounded_by_the_request_source_inventory() {
        let response = AttributeProcessorResponseV1 {
            schema_version: ATTRIBUTE_PROCESSOR_SCHEMA_VERSION,
            graph_fingerprint: "graph".to_string(),
            diagnostics: vec![ProcessorDiagnosticV1 {
                code: "ROUTE001".to_string(),
                title: "Invalid Route".to_string(),
                severity: ProcessorDiagnosticSeverityV1::Error,
                message: "invalid route".to_string(),
                labels: vec![
                    ProcessorDiagnosticLabelV1 {
                        source: "made-up.doria".to_string(),
                        byte_start: 0,
                        byte_end: 1,
                        message: "unknown source".to_string(),
                    },
                    ProcessorDiagnosticLabelV1 {
                        source: "acme/application:main.doria".to_string(),
                        byte_start: 5,
                        byte_end: 11,
                        message: "outside source".to_string(),
                    },
                ],
                explanation: None,
                help: None,
            }],
            generated_sources: Vec::new(),
        };
        let request = processor_request(vec![processor_source("acme/application:main.doria", 10)]);

        let diagnostics = validate_processor_response(&response, &request, &[])
            .expect_err("invented and out-of-bounds labels must be rejected");
        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("unknown source `made-up.doria`")));
        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("label outside source `acme/application:main.doria`")));
    }
}
