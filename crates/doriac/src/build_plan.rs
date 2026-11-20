use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::diagnostics::{Diagnostic, DiagnosticKind, DiagnosticResult, DiagnosticSource};
pub use crate::native_link::{NativeLibrary, WindowsSubsystem};
use crate::source::Span;

/// The newest build-plan schema; every version in
/// [`SUPPORTED_BUILD_PLAN_SCHEMA_VERSIONS`] is accepted.
pub const BUILD_PLAN_SCHEMA_VERSION: u32 = 2;

/// Schema 2 adds each package's `nativeLibraries` and `unsafePolicy` and the
/// selected target's `windowsSubsystem` (Decision 0148 section 18, amended
/// 2026-11-20). Schema 1 plans keep their meaning.
pub const SUPPORTED_BUILD_PLAN_SCHEMA_VERSIONS: [u32; 2] = [1, 2];
const SUPPORTED: &str = "`1` or `2`";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BuildPlan {
    pub schema_version: u32,
    pub edition: String,
    pub root_package: String,
    pub selected_target: SelectedTarget,
    pub packages: Vec<Package>,
    pub compiler: CompilerOptions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectedTarget {
    pub package: String,
    pub name: String,
    pub kind: TargetKind,
    pub entry_source: Option<String>,
    pub active_scopes: Vec<SourceScope>,
    /// Schema 2: `"console"` or `"gui"` for a binary target, absent for a
    /// library (Decision 0148 section 21).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windows_subsystem: Option<WindowsSubsystem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetKind {
    Binary,
    Library,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Package {
    pub identity: String,
    pub root: String,
    #[serde(default)]
    pub namespace_mappings: Vec<NamespaceMapping>,
    #[serde(default)]
    pub sources: Vec<Source>,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
    /// Schema 2: the resolved link inputs of each logical library the
    /// package's sources name with `#[Library]` (Decision 0148 section 18).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub native_libraries: Vec<NativeLibrary>,
    /// Schema 2: the package's unsafe policy (Decision 0148 section 25).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsafe_policy: Option<UnsafePolicy>,
}

/// Whether a package's sources may contain unsafe code (Decision 0148
/// section 25).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UnsafePolicy {
    Allow,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NamespaceMapping {
    pub prefix: String,
    pub path: String,
    pub scope: SourceScope,
    #[serde(default)]
    pub generated_for: Option<GeneratedFor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Source {
    pub identity: String,
    pub path: String,
    pub scope: SourceScope,
    pub origin: SourceOrigin,
    #[serde(default)]
    pub generated_for: Option<GeneratedFor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceScope {
    Main,
    Development,
    Generated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceOrigin {
    Entry,
    Autoload,
    Explicit,
    Generated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GeneratedFor {
    Main,
    Development,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Dependency {
    pub package: String,
    pub kind: DependencyKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DependencyKind {
    Normal,
    Development,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompilerOptions {
    pub target: CompilerTarget,
    pub native_profile: Option<BuildNativeProfile>,
    pub target_triple: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompilerTarget {
    Debug,
    Native,
    Php,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BuildNativeProfile {
    Fast,
    Release,
}

impl BuildNativeProfile {
    /// The backend profile a build plan's `nativeProfile` selects. Whether this
    /// doriac build compiles it is `NativeProfile::is_available`.
    pub const fn native(self) -> crate::backend::NativeProfile {
        match self {
            Self::Fast => crate::backend::NativeProfile::Fast,
            Self::Release => crate::backend::NativeProfile::Release,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildPlanDocument {
    pub path: String,
    pub directory: PathBuf,
    pub text: String,
    pub plan: BuildPlan,
}

pub fn parse_build_plan(
    path: impl Into<String>,
    text: impl Into<String>,
) -> DiagnosticResult<BuildPlan> {
    let path = path.into();
    let text = text.into();
    let value = serde_json::from_str::<serde_json::Value>(&text)
        .map_err(|error| build_plan_json_diagnostic(&path, &text, &error))?;
    if let Some(version) = value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
    {
        if !SUPPORTED_BUILD_PLAN_SCHEMA_VERSIONS
            .iter()
            .any(|supported| u64::from(*supported) == version)
        {
            return Err(vec![compiler_input_diagnostic(
                "E0676",
                format!("unknown build-plan schema version `{version}`; expected {SUPPORTED}"),
                Span::default(),
            )
            .with_title("Build Plan Schema Version Is Unsupported")
            .with_primary_source(DiagnosticSource::Path(path))]);
        }
    }
    let plan = serde_json::from_str::<BuildPlan>(&text)
        .map_err(|error| build_plan_json_diagnostic(&path, &text, &error))?;
    validate_build_plan(&plan)?;
    Ok(plan)
}

pub fn encode_build_plan(plan: &BuildPlan) -> DiagnosticResult<String> {
    validate_build_plan(plan)?;
    serde_json::to_string_pretty(plan).map_err(|error| {
        vec![compiler_input_diagnostic(
            "E0676",
            format!(
                "could not encode build-plan schema {}: {error}",
                plan.schema_version
            ),
            Span::default(),
        )
        .with_title("Build Plan Could Not Be Encoded")]
    })
}

pub fn parse_build_plan_document(
    path: impl Into<String>,
    text: impl Into<String>,
) -> DiagnosticResult<BuildPlanDocument> {
    let path = path.into();
    let text = text.into();
    let plan = parse_build_plan(path.clone(), text.clone())?;
    let directory = Path::new(&path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    Ok(BuildPlanDocument {
        path,
        directory,
        text,
        plan,
    })
}

pub fn validate_build_plan(plan: &BuildPlan) -> DiagnosticResult<()> {
    let mut diagnostics = Vec::new();
    if !SUPPORTED_BUILD_PLAN_SCHEMA_VERSIONS.contains(&plan.schema_version) {
        diagnostics.push(
            compiler_input_diagnostic(
                "E0676",
                format!(
                    "unknown build-plan schema version `{}`; expected {SUPPORTED}",
                    plan.schema_version
                ),
                Span::default(),
            )
            .with_title("Build Plan Schema Version Is Unsupported"),
        );
    }
    if plan.edition != "2026" {
        diagnostics.push(
            compiler_input_diagnostic(
                "E0676",
                format!("unknown Doria edition `{}` in build plan", plan.edition),
                Span::default(),
            )
            .with_title("Build Plan Edition Is Unsupported"),
        );
    }
    validate_package_identity(&plan.root_package, "root package", &mut diagnostics);
    validate_package_identity(
        &plan.selected_target.package,
        "selected target package",
        &mut diagnostics,
    );
    if plan.selected_target.name.trim().is_empty() {
        diagnostics.push(input("selected target name cannot be empty"));
    }
    match (
        plan.selected_target.kind,
        plan.selected_target.entry_source.as_ref(),
    ) {
        (TargetKind::Binary, None) => {
            diagnostics.push(input("a binary selected target requires an entrySource"))
        }
        (TargetKind::Library, Some(_)) => diagnostics.push(input(
            "a library selected target must not declare an entrySource",
        )),
        _ => {}
    }
    let active_scopes = plan
        .selected_target
        .active_scopes
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if !active_scopes.contains(&SourceScope::Main) {
        diagnostics.push(input("selectedTarget.activeScopes must include `main`"));
    }
    if active_scopes.len() != plan.selected_target.active_scopes.len() {
        diagnostics.push(input(
            "selectedTarget.activeScopes contains a duplicate scope",
        ));
    }
    match (plan.compiler.target, plan.compiler.native_profile) {
        (CompilerTarget::Native, None) => diagnostics.push(input(
            "compiler.nativeProfile is required when compiler.target is `native`",
        )),
        (CompilerTarget::Debug | CompilerTarget::Php, Some(_)) => diagnostics.push(input(
            "compiler.nativeProfile must be null unless compiler.target is `native`",
        )),
        _ => {}
    }
    let schema_2 = plan.schema_version >= 2;
    match (
        schema_2,
        plan.selected_target.kind,
        plan.selected_target.windows_subsystem,
    ) {
        (false, _, Some(_)) => diagnostics.push(input(
            "selectedTarget.windowsSubsystem requires build-plan schema 2",
        )),
        (true, TargetKind::Binary, None) => diagnostics.push(input(
            "a binary selected target requires a windowsSubsystem in build-plan schema 2",
        )),
        (true, TargetKind::Library, Some(_)) => diagnostics.push(input(
            "a library selected target must not declare a windowsSubsystem",
        )),
        _ => {}
    }

    let mut packages = BTreeMap::new();
    let mut source_identities = BTreeSet::new();
    for package in &plan.packages {
        validate_package_identity(&package.identity, "package", &mut diagnostics);
        if packages
            .insert(package.identity.as_str(), package)
            .is_some()
        {
            diagnostics.push(input(format!(
                "package identity `{}` is declared more than once",
                package.identity
            )));
        }
        let mut package_paths = BTreeSet::new();
        let mut mappings = BTreeSet::new();
        let mut mapping_authorities = BTreeMap::new();
        for mapping in &package.namespace_mappings {
            if !mapping.path.is_empty() {
                validate_relative_plan_path(
                    mapping.path.trim_end_matches(['/', '\\']),
                    "namespace mapping",
                    &mut diagnostics,
                );
            }
            if !mapping.prefix.is_empty() && !mapping.prefix.ends_with('\\') {
                diagnostics.push(input(format!(
                    "namespace mapping prefix `{}` must be empty or end in `\\`",
                    mapping.prefix
                )));
            }
            if !mappings.insert((
                mapping.prefix.as_str(),
                mapping.path.as_str(),
                mapping.scope,
            )) {
                diagnostics.push(input(format!(
                    "namespace mapping `{}` to `{}` is duplicated",
                    mapping.prefix, mapping.path
                )));
            }
            let authority = (
                mapping.prefix.as_str(),
                mapping.scope,
                mapping.generated_for,
            );
            if let Some(previous_path) =
                mapping_authorities.insert(authority, mapping.path.as_str())
            {
                if previous_path != mapping.path {
                    diagnostics.push(input(format!(
                        "namespace mapping prefix `{}` has equally specific paths `{previous_path}` and `{}` for the same source surface",
                        mapping.prefix, mapping.path
                    )));
                }
            }
            validate_generated_scope(
                mapping.scope,
                mapping.generated_for,
                "namespace mapping",
                &mut diagnostics,
            );
        }
        for source in &package.sources {
            validate_relative_plan_path(&source.path, "source", &mut diagnostics);
            if source.identity.trim().is_empty()
                || !source
                    .identity
                    .starts_with(&format!("{}:", package.identity))
            {
                diagnostics.push(input(format!(
                    "source identity `{}` must be nonempty and owned by package `{}`",
                    source.identity, package.identity
                )));
            }
            if !source_identities.insert(source.identity.as_str()) {
                diagnostics.push(input(format!(
                    "source identity `{}` is declared more than once",
                    source.identity
                )));
            }
            let folded = source.path.replace('\\', "/").to_ascii_lowercase();
            if !package_paths.insert(folded) {
                diagnostics.push(input(format!(
                    "package `{}` contains duplicate or case-colliding source path `{}`",
                    package.identity, source.path
                )));
            }
            validate_generated_scope(
                source.scope,
                source.generated_for,
                "source",
                &mut diagnostics,
            );
            let selected_generated_entry = source.scope == SourceScope::Generated
                && source.origin == SourceOrigin::Entry
                && plan.selected_target.entry_source.as_deref() == Some(&source.identity);
            if source.scope == SourceScope::Generated
                && source.origin != SourceOrigin::Generated
                && !selected_generated_entry
            {
                diagnostics.push(input(format!(
                    "generated source `{}` must use origin `generated` unless it is the selected entry source",
                    source.identity
                )));
            }
            if source.scope != SourceScope::Generated && source.origin == SourceOrigin::Generated {
                diagnostics.push(input(format!(
                    "non-generated source `{}` must not use origin `generated`",
                    source.identity
                )));
            }
            if source.origin == SourceOrigin::Entry
                && plan.selected_target.entry_source.as_deref() != Some(&source.identity)
            {
                diagnostics.push(input(format!(
                    "source `{}` uses origin `entry` but is not the selected entry source",
                    source.identity
                )));
            }
        }
        validate_native_facts(package, schema_2, &mut diagnostics);
        let mut dependencies = BTreeSet::new();
        let mut dependency_packages = BTreeSet::new();
        for dependency in &package.dependencies {
            validate_package_identity(&dependency.package, "dependency", &mut diagnostics);
            if !dependencies.insert((dependency.package.as_str(), dependency.kind)) {
                diagnostics.push(input(format!(
                    "dependency `{}` with kind `{:?}` is duplicated",
                    dependency.package, dependency.kind
                )));
            }
            if !dependency_packages.insert(dependency.package.as_str()) {
                diagnostics.push(input(format!(
                    "dependency `{}` is declared with more than one dependency kind",
                    dependency.package
                )));
            }
        }
    }
    if !packages.contains_key(plan.root_package.as_str()) {
        diagnostics.push(input(format!(
            "root package `{}` is not present in packages",
            plan.root_package
        )));
    }
    if plan.selected_target.package != plan.root_package {
        diagnostics.push(input("the selected target package must equal rootPackage"));
    }
    let Some(target_package) = packages.get(plan.selected_target.package.as_str()) else {
        diagnostics.push(input(format!(
            "selected target package `{}` is not present in packages",
            plan.selected_target.package
        )));
        return diagnostics.is_empty().then_some(()).ok_or(diagnostics);
    };
    if let Some(entry) = &plan.selected_target.entry_source {
        match target_package
            .sources
            .iter()
            .find(|source| &source.identity == entry)
        {
            None => diagnostics.push(input(format!(
                "entry source `{entry}` is not declared by selected target package"
            ))),
            Some(source) if !source_is_active(source, &active_scopes) => diagnostics.push(input(
                format!("entry source `{entry}` is not active for the selected target"),
            )),
            Some(source) => {
                if source.origin != SourceOrigin::Entry {
                    diagnostics.push(input(format!(
                        "entry source `{entry}` must use origin `entry`"
                    )));
                }
            }
        }
    }
    for package in &plan.packages {
        for dependency in &package.dependencies {
            if !packages.contains_key(dependency.package.as_str()) {
                diagnostics.push(input(format!(
                    "package `{}` depends on missing package `{}`",
                    package.identity, dependency.package
                )));
            }
        }
    }

    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

/// The schema-2 package fields: `unsafePolicy` and `nativeLibraries`.
fn validate_native_facts(package: &Package, schema_2: bool, diagnostics: &mut Vec<Diagnostic>) {
    let identity = &package.identity;
    match (schema_2, package.unsafe_policy) {
        (false, Some(_)) => diagnostics.push(input(format!(
            "package `{identity}` declares unsafePolicy, which requires build-plan schema 2"
        ))),
        (true, None) => diagnostics.push(input(format!(
            "package `{identity}` requires an unsafePolicy in build-plan schema 2"
        ))),
        _ => {}
    }
    if !schema_2 && !package.native_libraries.is_empty() {
        diagnostics.push(input(format!(
            "package `{identity}` declares nativeLibraries, which requires build-plan schema 2"
        )));
    }
    let mut names = BTreeSet::new();
    for library in &package.native_libraries {
        if !names.insert(library.name.as_str()) {
            diagnostics.push(input(format!(
                "package `{identity}` declares native library `{}` more than once",
                library.name
            )));
        }
        for problem in library.problems() {
            diagnostics.push(input(format!("package `{identity}`: {problem}")));
        }
        let paths = library
            .inputs
            .iter()
            .filter_map(|input| match input {
                crate::native_link::LinkInput::SearchPath { path }
                | crate::native_link::LinkInput::File { path } => Some(path),
                _ => None,
            })
            .chain(&library.runtime_files);
        for path in paths {
            // A path pkg-config resolved is absolute machine-local input,
            // as an absolute package root is; a project path stays inside
            // its package.
            if !path.is_absolute() {
                validate_relative_plan_path(
                    &path.to_string_lossy(),
                    &format!("native library `{}`", library.name),
                    diagnostics,
                );
            }
        }
    }
}

fn build_plan_json_diagnostic(
    path: &str,
    text: &str,
    error: &serde_json::Error,
) -> Vec<Diagnostic> {
    let span = json_error_span(text, error.line(), error.column());
    vec![compiler_input_diagnostic(
        "E0676",
        format!(
            "invalid build plan at line {}, column {}: {error}",
            error.line(),
            error.column()
        ),
        span,
    )
    .with_title("Build Plan Is Invalid")
    .with_primary_label("Build Plan JSON Is Invalid")
    .with_primary_source(DiagnosticSource::Path(path.to_string()))
    .with_explanation("Build plans use strict camelCase JSON fields.")]
}

fn validate_relative_plan_path(value: &str, subject: &str, diagnostics: &mut Vec<Diagnostic>) {
    let path = Path::new(value);
    let invalid = value.is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        });
    if invalid {
        diagnostics.push(input(format!(
            "{subject} path `{value}` must be a nonempty package-relative path without parent traversal"
        )));
    }
}

pub fn source_is_active(source: &Source, active_scopes: &BTreeSet<SourceScope>) -> bool {
    match source.scope {
        SourceScope::Main => active_scopes.contains(&SourceScope::Main),
        SourceScope::Development => active_scopes.contains(&SourceScope::Development),
        SourceScope::Generated => {
            active_scopes.contains(&SourceScope::Generated)
                && source
                    .generated_for
                    .is_some_and(|generated_for| match generated_for {
                        GeneratedFor::Main => active_scopes.contains(&SourceScope::Main),
                        GeneratedFor::Development => {
                            active_scopes.contains(&SourceScope::Development)
                        }
                    })
        }
    }
}

fn validate_generated_scope(
    scope: SourceScope,
    generated_for: Option<GeneratedFor>,
    subject: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match (scope, generated_for) {
        (SourceScope::Generated, None) => {
            diagnostics.push(input(format!("generated {subject} requires generatedFor")))
        }
        (SourceScope::Main | SourceScope::Development, Some(_)) => diagnostics.push(input(
            format!("non-generated {subject} must not declare generatedFor"),
        )),
        _ => {}
    }
}

fn validate_package_identity(value: &str, subject: &str, diagnostics: &mut Vec<Diagnostic>) {
    let parts = value.split('/').collect::<Vec<_>>();
    let valid = parts.len() == 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'-' | b'_')
                })
        });
    if !valid {
        diagnostics.push(input(format!(
            "{subject} identity `{value}` must use lowercase `vendor/package` form"
        )));
    }
}

fn json_error_span(text: &str, line: usize, column: usize) -> Span {
    let line_start = text
        .split_inclusive('\n')
        .take(line.saturating_sub(1))
        .map(str::len)
        .sum::<usize>();
    let start = line_start
        .saturating_add(column.saturating_sub(1))
        .min(text.len());
    Span::new(start, start.saturating_add(1).min(text.len()))
}

pub(crate) fn compiler_input_diagnostic(
    code: &'static str,
    message: impl Into<String>,
    span: Span,
) -> Diagnostic {
    Diagnostic::new(code, message, span).with_kind(DiagnosticKind::CompilerInput)
}

fn input(message: impl Into<String>) -> Diagnostic {
    compiler_input_diagnostic("E0676", message, Span::default()).with_title("Build Plan Is Invalid")
}
