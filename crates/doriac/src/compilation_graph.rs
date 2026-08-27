use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};

use crate::ast::{Item, MemberAccess, Program};
use crate::build_plan::{
    source_is_active, BuildPlanDocument, DependencyKind, GeneratedFor, Package, SourceOrigin,
    SourceScope, TargetKind,
};
use crate::diagnostics::{
    Diagnostic, DiagnosticLabel, DiagnosticResult, DiagnosticSource, LabelRole,
};
use crate::names::{
    CompilationContext, Edition, GlobalSymbolDeclaration, GlobalSymbolFacts,
    NameResolutionEnvironment, PackageIdentity, SourceIdentity,
};
use crate::runtime_digest::sha256_hex;
use crate::source::{SourceFile, SourceId, Span};
use crate::source_map::{SourceMap, SourceRecord};
use crate::source_provider::{
    path_uses_exact_case, IncludeRequest, ProvidedSource, SourceProvider, SourceProviderError,
    SourceProviderErrorKind, SourceRequest,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphCompleteness {
    Complete,
    Partial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectStructureAuthority {
    Authoritative,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphLoadOptions {
    pub completeness: GraphCompleteness,
    pub project_structure: ProjectStructureAuthority,
}

impl Default for GraphLoadOptions {
    fn default() -> Self {
        Self {
            completeness: GraphCompleteness::Complete,
            project_structure: ProjectStructureAuthority::Authoritative,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageNode {
    pub identity: PackageIdentity,
    pub canonical_root: PathBuf,
    pub normal_dependencies: BTreeSet<String>,
    pub development_dependencies: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphSource {
    pub id: SourceId,
    pub identity: SourceIdentity,
    pub package: PackageIdentity,
    pub package_relative_path: String,
    pub display_path: String,
    pub canonical_path: Option<PathBuf>,
    pub scope: SourceScope,
    pub origin: SourceOrigin,
    pub generated_for: Option<GeneratedFor>,
    pub included: bool,
    pub content_fingerprint: String,
    pub source: SourceFile,
    pub authored: Program,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncludeEdge {
    pub including: SourceIdentity,
    pub included: SourceIdentity,
    pub literal_span: Span,
    pub literal: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompilationGraph {
    pub build_plan: crate::build_plan::BuildPlan,
    pub completeness: GraphCompleteness,
    pub packages: BTreeMap<String, PackageNode>,
    pub sources: BTreeMap<String, GraphSource>,
    pub source_map: SourceMap,
    pub include_edges: Vec<IncludeEdge>,
    pub selected_entry: Option<SourceIdentity>,
    pub fingerprint: String,
}

#[derive(Debug, Clone)]
pub struct GraphLoadFailure {
    pub diagnostics: Vec<Diagnostic>,
    pub source_map: Box<SourceMap>,
}

#[derive(Debug, Clone)]
pub struct GraphSemanticAnalysis {
    pub authored_sources: BTreeMap<String, Program>,
    pub resolved_program: Program,
    pub semantic_info: crate::semantics::SemanticInfo,
    pub semantic_dependency_edges: Vec<SemanticDependencyEdge>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticDependencyEdge {
    pub source: SourceIdentity,
    pub target: SourceIdentity,
    pub symbol: crate::names::GlobalSymbolId,
    pub role: crate::names::GlobalReferenceRole,
}

#[derive(Debug, Clone)]
struct PendingSource {
    identity: SourceIdentity,
    package: PackageIdentity,
    package_relative_path: String,
    scope: SourceScope,
    origin: SourceOrigin,
    generated_for: Option<GeneratedFor>,
    included: bool,
    provided: ProvidedSource,
}

pub fn load_compilation_graph(
    document: &BuildPlanDocument,
    provider: &impl SourceProvider,
) -> DiagnosticResult<CompilationGraph> {
    load_compilation_graph_detailed(document, provider).map_err(|failure| failure.diagnostics)
}

pub fn load_compilation_graph_detailed(
    document: &BuildPlanDocument,
    provider: &impl SourceProvider,
) -> Result<CompilationGraph, GraphLoadFailure> {
    load_compilation_graph_inner(document, provider, GraphLoadOptions::default(), None)
}

pub fn load_compilation_graph_with_completeness(
    document: &BuildPlanDocument,
    provider: &impl SourceProvider,
    completeness: GraphCompleteness,
) -> DiagnosticResult<CompilationGraph> {
    load_compilation_graph_inner(
        document,
        provider,
        GraphLoadOptions {
            completeness,
            ..GraphLoadOptions::default()
        },
        None,
    )
    .map_err(|failure| failure.diagnostics)
}

pub fn load_compilation_graph_with_options(
    document: &BuildPlanDocument,
    provider: &impl SourceProvider,
    options: GraphLoadOptions,
) -> DiagnosticResult<CompilationGraph> {
    load_compilation_graph_inner(document, provider, options, None)
        .map_err(|failure| failure.diagnostics)
}

pub(crate) fn load_compilation_graph_with_session(
    document: &BuildPlanDocument,
    provider: &impl SourceProvider,
    options: GraphLoadOptions,
    session: &mut crate::incremental::CompilationSession,
) -> DiagnosticResult<CompilationGraph> {
    load_compilation_graph_inner(document, provider, options, Some(session))
        .map_err(|failure| failure.diagnostics)
}

fn load_compilation_graph_inner(
    document: &BuildPlanDocument,
    provider: &impl SourceProvider,
    options: GraphLoadOptions,
    mut session: Option<&mut crate::incremental::CompilationSession>,
) -> Result<CompilationGraph, GraphLoadFailure> {
    crate::build_plan::validate_build_plan(&document.plan).map_err(|diagnostics| {
        GraphLoadFailure {
            diagnostics,
            source_map: Box::default(),
        }
    })?;
    let active_scopes = document
        .plan
        .selected_target
        .active_scopes
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut diagnostics = Vec::new();
    let mut packages = BTreeMap::new();
    for package in &document.plan.packages {
        let package_root = if Path::new(&package.root).is_absolute() {
            PathBuf::from(&package.root)
        } else {
            document.directory.join(&package.root)
        };
        let canonical_root = match package_root.canonicalize() {
            Ok(root) if root.is_dir() => root,
            Ok(_) => {
                diagnostics.push(plan_input(format!(
                    "package root for `{}` is not a directory",
                    package.identity
                )));
                continue;
            }
            Err(error) => {
                diagnostics.push(plan_input(format!(
                    "package root for `{}` could not be canonicalized: {error}",
                    package.identity
                )));
                continue;
            }
        };
        validate_mapping_paths(package, &canonical_root, &mut diagnostics);
        let identity = PackageIdentity::Named(package.identity.clone());
        packages.insert(
            package.identity.clone(),
            PackageNode {
                identity,
                canonical_root,
                normal_dependencies: package
                    .dependencies
                    .iter()
                    .filter(|dependency| dependency.kind == DependencyKind::Normal)
                    .map(|dependency| dependency.package.clone())
                    .collect(),
                development_dependencies: package
                    .dependencies
                    .iter()
                    .filter(|dependency| dependency.kind == DependencyKind::Development)
                    .map(|dependency| dependency.package.clone())
                    .collect(),
            },
        );
    }
    validate_package_cycles(&document.plan.packages, &active_scopes, &mut diagnostics);
    if !diagnostics.is_empty() {
        return Err(GraphLoadFailure {
            diagnostics,
            source_map: Box::default(),
        });
    }

    let package_specs = document
        .plan
        .packages
        .iter()
        .map(|package| (package.identity.as_str(), package))
        .collect::<HashMap<_, _>>();
    let mut pending = BTreeMap::<String, PendingSource>::new();
    let mut canonical_owners = BTreeMap::<String, String>::new();
    let mut folded_paths = BTreeMap::<(String, String), String>::new();
    for package in &document.plan.packages {
        let package_node = packages
            .get(&package.identity)
            .expect("validated package has a package node");
        let mut sources = package
            .sources
            .iter()
            .filter(|source| source_is_active(source, &active_scopes))
            .collect::<Vec<_>>();
        sources.sort_by(|left, right| left.identity.cmp(&right.identity));
        for source in sources {
            match provider.read_source(SourceRequest {
                package,
                canonical_package_root: &package_node.canonical_root,
                source,
            }) {
                Ok(provided) => insert_pending_source(
                    &mut pending,
                    &mut canonical_owners,
                    &mut folded_paths,
                    PendingSource {
                        identity: SourceIdentity(source.identity.clone()),
                        package: package_node.identity.clone(),
                        package_relative_path: provided.package_relative_path.clone(),
                        scope: source.scope,
                        origin: source.origin,
                        generated_for: source.generated_for,
                        included: false,
                        provided,
                    },
                    &mut diagnostics,
                ),
                Err(error) => diagnostics.push(provider_diagnostic(error, None)),
            }
        }
    }
    if !diagnostics.is_empty() {
        return Err(GraphLoadFailure {
            diagnostics,
            source_map: Box::new(pending_source_map(&pending)),
        });
    }

    let mut include_edges = Vec::new();
    let mut inspected = BTreeSet::new();
    let mut queue = pending.keys().cloned().collect::<VecDeque<_>>();
    while let Some(identity) = queue.pop_front() {
        if !inspected.insert(identity.clone()) {
            continue;
        }
        let Some(source) = pending.get(&identity).cloned() else {
            continue;
        };
        let cached = session.as_deref().and_then(|session| {
            session.cached_for_include(&identity, &source.provided.content_fingerprint)
        });
        let temporary = SourceFile::new(
            source.provided.display_path.clone(),
            source.provided.text.clone(),
        );
        let authored = match cached.map_or_else(|| crate::parse_source_file(&temporary), Ok) {
            Ok(program) => program,
            Err(source_diagnostics) => {
                diagnostics.extend(retarget_diagnostics(
                    source_diagnostics,
                    &source.provided.display_path,
                ));
                continue;
            }
        };
        let package_name = source.package.display_name().to_string();
        let package = package_specs
            .get(package_name.as_str())
            .expect("pending source belongs to a validated package");
        let package_node = packages
            .get(&package_name)
            .expect("pending source belongs to a validated package node");
        for include in authored.includes {
            match provider.read_included_source(IncludeRequest {
                package,
                canonical_package_root: &package_node.canonical_root,
                including_relative_path: &source.package_relative_path,
                include_path: &include.value,
            }) {
                Ok(provided) => {
                    let existing_identity = pending
                        .values()
                        .find(|candidate| same_canonical_source(candidate, &provided))
                        .map(|candidate| candidate.identity.clone());
                    let included_identity = existing_identity.clone().unwrap_or_else(|| {
                        SourceIdentity(format!(
                            "{}:{}",
                            source.package.display_name(),
                            provided.package_relative_path
                        ))
                    });
                    if existing_identity.is_none() && pending.contains_key(&included_identity.0) {
                        diagnostics.push(plan_input(format!(
                            "included source `{}` resolves to source identity `{}`, which is already assigned to a different canonical file",
                            provided.display_path, included_identity.0
                        )));
                        continue;
                    }
                    include_edges.push(IncludeEdge {
                        including: source.identity.clone(),
                        included: included_identity.clone(),
                        literal_span: include.literal_span,
                        literal: include.value.clone(),
                    });
                    if !pending.contains_key(&included_identity.0) {
                        let inserted = PendingSource {
                            identity: included_identity.clone(),
                            package: source.package.clone(),
                            package_relative_path: provided.package_relative_path.clone(),
                            scope: source.scope,
                            origin: SourceOrigin::Explicit,
                            generated_for: source.generated_for,
                            included: true,
                            provided,
                        };
                        insert_pending_source(
                            &mut pending,
                            &mut canonical_owners,
                            &mut folded_paths,
                            inserted,
                            &mut diagnostics,
                        );
                        queue.push_back(included_identity.0);
                    }
                }
                Err(error) => diagnostics.push(provider_diagnostic(
                    error,
                    Some((&source.provided.display_path, include.literal_span)),
                )),
            }
        }
    }
    if !diagnostics.is_empty() {
        return Err(GraphLoadFailure {
            diagnostics,
            source_map: Box::new(pending_source_map(&pending)),
        });
    }

    include_edges.sort_by(|left, right| {
        (&left.including.0, &left.included.0, left.literal_span.start).cmp(&(
            &right.including.0,
            &right.included.0,
            right.literal_span.start,
        ))
    });
    include_edges.dedup_by(|left, right| {
        left.including == right.including && left.included == right.included
    });

    let source_map = pending_source_map(&pending);
    let mut sources = BTreeMap::new();
    for (identity, pending_source) in pending {
        let source_file = source_map
            .get(&pending_source.identity)
            .expect("pending source has a source-map record")
            .source
            .clone();
        let id = source_file.id;
        let authored = if let Some(cached) = session.as_deref_mut().and_then(|session| {
            session.cached_for_source(&identity, &pending_source.provided.content_fingerprint, id)
        }) {
            if let Some(session) = session.as_deref_mut() {
                session.record_reused_source(
                    identity.clone(),
                    pending_source.provided.content_fingerprint.clone(),
                    id,
                    cached.clone(),
                );
            }
            cached
        } else {
            let parsed = crate::parse_source_file(&source_file).map_err(|source_diagnostics| {
                GraphLoadFailure {
                    diagnostics: retarget_diagnostics(
                        source_diagnostics,
                        &pending_source.provided.display_path,
                    ),
                    source_map: Box::new(source_map.clone()),
                }
            })?;
            if let Some(session) = session.as_deref_mut() {
                session.record_parsed_source(
                    identity.clone(),
                    pending_source.provided.content_fingerprint.clone(),
                    id,
                    parsed.clone(),
                );
            }
            parsed
        };
        validate_source_shape(
            &document.plan,
            package_specs
                .get(pending_source.package.display_name())
                .expect("source package exists"),
            &pending_source,
            &authored,
            options.project_structure,
            &mut diagnostics,
        );
        sources.insert(
            identity,
            GraphSource {
                id,
                identity: pending_source.identity,
                package: pending_source.package,
                package_relative_path: pending_source.package_relative_path,
                display_path: pending_source.provided.display_path,
                canonical_path: pending_source.provided.canonical_path,
                scope: pending_source.scope,
                origin: pending_source.origin,
                generated_for: pending_source.generated_for,
                included: pending_source.included,
                content_fingerprint: pending_source.provided.content_fingerprint,
                source: source_file,
                authored,
            },
        );
    }
    if !diagnostics.is_empty() {
        return Err(GraphLoadFailure {
            diagnostics,
            source_map: Box::new(source_map),
        });
    }
    for edge in &mut include_edges {
        if let Some(including) = sources.get(&edge.including.0) {
            edge.literal_span.source = including.id;
        }
    }
    let selected_entry = document
        .plan
        .selected_target
        .entry_source
        .as_ref()
        .map(|identity| SourceIdentity(identity.clone()));
    let fingerprint = graph_fingerprint(&document.plan, &sources, &include_edges);
    Ok(CompilationGraph {
        build_plan: document.plan.clone(),
        completeness: options.completeness,
        packages,
        source_map,
        sources,
        include_edges,
        selected_entry,
        fingerprint,
    })
}

fn pending_source_map(pending: &BTreeMap<String, PendingSource>) -> SourceMap {
    let source_ids = stable_source_ids(pending.keys());
    SourceMap::from_ordered_records(
        pending
            .iter()
            .map(|(identity, pending_source)| {
                let source = SourceFile::with_id(
                    source_ids[identity],
                    pending_source.provided.display_path.clone(),
                    pending_source.provided.text.clone(),
                );
                SourceRecord {
                    identity: pending_source.identity.clone(),
                    package: pending_source.package.clone(),
                    display_path: pending_source.provided.display_path.clone(),
                    canonical_path: pending_source
                        .provided
                        .canonical_path
                        .as_ref()
                        .map(|path| path.display().to_string()),
                    content_fingerprint: pending_source.provided.content_fingerprint.clone(),
                    source,
                }
            })
            .collect(),
    )
}

fn stable_source_ids<'a>(
    identities: impl Iterator<Item = &'a String>,
) -> BTreeMap<String, SourceId> {
    let mut allocated = BTreeSet::new();
    let mut result = BTreeMap::new();
    for identity in identities {
        let digest = sha256_hex(identity.as_bytes());
        let mut candidate = u32::from_str_radix(&digest[..8], 16)
            .expect("SHA-256 hexadecimal prefix is a u32")
            & 0x7fff_ffff;
        while candidate == crate::compiler_known_io::SYNTHETIC_SOURCE_ID.0
            || !allocated.insert(candidate)
        {
            candidate = candidate.wrapping_add(1) & 0x7fff_ffff;
        }
        result.insert(identity.clone(), SourceId(candidate));
    }
    result
}

pub fn compilation_context(source: &GraphSource) -> CompilationContext {
    CompilationContext {
        edition: Edition::Doria2026,
        package: source.package.clone(),
        source: source.identity.clone(),
    }
}

pub fn analyze_compilation_graph_for_ide(graph: &CompilationGraph) -> GraphSemanticAnalysis {
    let mut diagnostics = Vec::new();
    let mut declaration_groups = BTreeMap::<String, Vec<GlobalSymbolDeclaration>>::new();
    for source in graph.sources.values() {
        let context = compilation_context(source);
        if let Err(source_diagnostics) =
            crate::compiler_known_io::validate_reserved_identities(&source.authored)
        {
            diagnostics.extend(source_diagnostics);
        }
        for declaration in crate::names::graph_declaration_headers(&source.authored, &context) {
            declaration_groups
                .entry(declaration.qualified_name.clone())
                .or_default()
                .push(declaration);
        }
    }

    let mut declarations = BTreeMap::new();
    for (qualified_name, mut group) in declaration_groups {
        group.sort_by(|left, right| {
            (&left.source_identity.0, left.name_span)
                .cmp(&(&right.source_identity.0, right.name_span))
        });
        if group.len() > 1 {
            let first = &group[0];
            let mut diagnostic = Diagnostic::new(
                "E0684",
                format!("global declaration `{qualified_name}` is declared more than once"),
                first.name_span,
            )
            .with_title("Duplicate Fully Qualified Declaration")
            .with_primary_label("The First Deterministic Declaration Is Here");
            for declaration in group.iter().skip(1) {
                diagnostic.labels.push(DiagnosticLabel {
                    source: diagnostic_source_for_span(graph, declaration.name_span),
                    span: declaration.name_span,
                    role: LabelRole::Secondary,
                    message: format!("Another {:?} Declaration Is Here", declaration.kind),
                });
            }
            diagnostics.push(diagnostic);
        }
        declarations.insert(qualified_name, group.remove(0));
    }

    let mut combined = Program {
        namespace: None,
        imports: Vec::new(),
        includes: Vec::new(),
        qualified_names: Vec::new(),
        attributes: Vec::new(),
        items: Vec::new(),
    };
    let mut facts = GlobalSymbolFacts::default();
    let mut authored_sources = BTreeMap::new();
    let mut contexts = HashMap::new();
    let mut selected_namespace = None;
    let mut selected_namespace_declaration = None;
    let mut compiler_known_seen = BTreeSet::new();
    let mut uses_compiler_known_io = false;

    for (identity, source) in &graph.sources {
        match crate::compiler_known_io::source_uses_io_intrinsics(&source.source) {
            Ok(uses_io) => uses_compiler_known_io |= uses_io,
            Err(source_diagnostics) => diagnostics.extend(source_diagnostics),
        }
        let context = compilation_context(source);
        contexts.insert(source.id, context.clone());
        let environment = NameResolutionEnvironment {
            declarations: declarations.clone(),
            visible_packages: visible_packages(graph, source),
            direct_normal_dependencies: direct_normal_dependencies(graph, source),
            direct_development_dependencies: direct_development_dependencies(graph, source),
            dependency_paths: dependency_paths(graph, source.package.display_name()),
            complete: graph.completeness == GraphCompleteness::Complete,
            includes_resolved: true,
        };
        let resolution = crate::names::resolve_program_in_graph_for_ide(
            &source.authored,
            &context,
            &environment,
        );
        diagnostics.extend(resolution.diagnostics);
        let resolved = resolution.resolved;
        if graph.selected_entry.as_ref() == Some(&source.identity) {
            selected_namespace = resolved.facts.namespace.clone();
            selected_namespace_declaration = resolved.facts.namespace_declaration.clone();
        }
        combined
            .qualified_names
            .extend(resolved.program.qualified_names.iter().cloned());
        combined
            .attributes
            .extend(resolved.program.attributes.iter().cloned());
        combined
            .items
            .extend(resolved.program.items.iter().cloned());
        facts.namespaces.extend(resolved.facts.namespaces);
        facts.declarations.extend(resolved.facts.declarations);
        facts.references.extend(resolved.facts.references);
        facts.imports.extend(resolved.facts.imports);
        facts.unresolved.extend(resolved.facts.unresolved);
        for known in resolved.facts.compiler_known {
            let key = format!("{:?}:{}", known.id.owner, known.id.qualified_name);
            if compiler_known_seen.insert(key) {
                facts.compiler_known.push(known);
            }
        }
        authored_sources.insert(identity.clone(), source.authored.clone());
    }
    facts.namespace = selected_namespace;
    facts.namespace_declaration = selected_namespace_declaration;

    if uses_compiler_known_io || crate::compiler_known_io::resolved_facts_use_canonical_io(&facts) {
        combined = crate::compiler_known_io::augment_program(&combined);
    }

    let source_texts = graph
        .sources
        .values()
        .map(|source| (source.id, source.source.text.as_str()))
        .chain(std::iter::once((
            crate::compiler_known_io::SYNTHETIC_SOURCE_ID,
            "",
        )))
        .collect::<HashMap<_, _>>();
    let compiler_known_context = selected_context(graph);
    contexts.insert(
        crate::compiler_known_io::SYNTHETIC_SOURCE_ID,
        CompilationContext {
            edition: compiler_known_context.edition,
            package: PackageIdentity::CompilerKnown,
            source: SourceIdentity(crate::compiler_known_io::SYNTHETIC_SOURCE_IDENTITY.to_string()),
        },
    );
    let mut semantic = crate::semantics::analyze_program_for_ide_with_graph_context(
        &combined,
        &source_texts,
        selected_context(graph),
        contexts,
        facts,
    );
    diagnostics.append(&mut semantic.diagnostics);
    let declaration_sources = semantic
        .info
        .global_symbols
        .declarations
        .iter()
        .map(|declaration| (declaration.id.clone(), declaration.source_identity.clone()))
        .collect::<HashMap<_, _>>();
    let mut semantic_dependency_edges = semantic
        .info
        .global_symbols
        .references
        .iter()
        .filter_map(|reference| {
            declaration_sources
                .get(&reference.symbol_id)
                .filter(|target| **target != reference.source_identity)
                .map(|target| SemanticDependencyEdge {
                    source: reference.source_identity.clone(),
                    target: target.clone(),
                    symbol: reference.symbol_id.clone(),
                    role: reference.role,
                })
        })
        .collect::<Vec<_>>();
    semantic_dependency_edges.sort_by(|left, right| {
        (
            &left.source.0,
            &left.target.0,
            &left.symbol.qualified_name,
            left.role,
        )
            .cmp(&(
                &right.source.0,
                &right.target.0,
                &right.symbol.qualified_name,
                right.role,
            ))
    });
    semantic_dependency_edges.dedup();
    retarget_graph_diagnostics(graph, &mut diagnostics);
    sort_graph_diagnostics(graph, &mut diagnostics);

    GraphSemanticAnalysis {
        authored_sources,
        resolved_program: combined,
        semantic_info: semantic.info,
        semantic_dependency_edges,
        diagnostics,
    }
}

pub fn check_compilation_graph(graph: &CompilationGraph) -> DiagnosticResult<Program> {
    let analysis = analyze_compilation_graph_for_ide(graph);
    if analysis.diagnostics.is_empty() {
        Ok(analysis.resolved_program)
    } else {
        Err(analysis.diagnostics)
    }
}

fn selected_context(graph: &CompilationGraph) -> CompilationContext {
    graph
        .selected_entry
        .as_ref()
        .and_then(|entry| graph.sources.get(&entry.0))
        .or_else(|| graph.sources.values().next())
        .map(compilation_context)
        .unwrap_or_default()
}

fn visible_packages(graph: &CompilationGraph, source: &GraphSource) -> BTreeSet<String> {
    let package_name = source.package.display_name();
    let mut visible = BTreeSet::from([package_name.to_string()]);
    let Some(package) = graph.packages.get(package_name) else {
        return visible;
    };
    visible.extend(package.normal_dependencies.iter().cloned());
    let development = source.scope == SourceScope::Development
        || source.generated_for == Some(GeneratedFor::Development);
    if development {
        visible.extend(package.development_dependencies.iter().cloned());
    }
    visible
}

fn direct_normal_dependencies(graph: &CompilationGraph, source: &GraphSource) -> BTreeSet<String> {
    graph
        .packages
        .get(source.package.display_name())
        .map(|package| package.normal_dependencies.clone())
        .unwrap_or_default()
}

fn direct_development_dependencies(
    graph: &CompilationGraph,
    source: &GraphSource,
) -> BTreeSet<String> {
    graph
        .packages
        .get(source.package.display_name())
        .map(|package| package.development_dependencies.clone())
        .unwrap_or_default()
}

fn dependency_paths(graph: &CompilationGraph, start: &str) -> BTreeMap<String, Vec<String>> {
    let mut paths = BTreeMap::from([(start.to_string(), vec![start.to_string()])]);
    let mut pending = VecDeque::from([start.to_string()]);
    while let Some(package_name) = pending.pop_front() {
        let Some(package) = graph.packages.get(&package_name) else {
            continue;
        };
        let mut dependencies = package
            .normal_dependencies
            .iter()
            .chain(&package.development_dependencies)
            .cloned()
            .collect::<Vec<_>>();
        dependencies.sort();
        for dependency in dependencies {
            if paths.contains_key(&dependency) {
                continue;
            }
            let mut path = paths
                .get(&package_name)
                .cloned()
                .expect("queued package has a dependency path");
            path.push(dependency.clone());
            paths.insert(dependency.clone(), path);
            pending.push_back(dependency);
        }
    }
    paths.remove(start);
    paths
}

fn diagnostic_source_for_span(graph: &CompilationGraph, span: Span) -> DiagnosticSource {
    graph
        .source_map
        .by_id(span.source)
        .map(|record| DiagnosticSource::Path(record.display_path.clone()))
        .unwrap_or(DiagnosticSource::Unavailable)
}

pub fn retarget_graph_diagnostics(graph: &CompilationGraph, diagnostics: &mut [Diagnostic]) {
    for diagnostic in diagnostics {
        for label in &mut diagnostic.labels {
            if label.source == DiagnosticSource::Current {
                label.source = diagnostic_source_for_span(graph, label.span);
            }
        }
        for fix in &mut diagnostic.fixes {
            for edit in &mut fix.edits {
                if edit.source == DiagnosticSource::Current {
                    edit.source = diagnostic_source_for_span(graph, edit.span);
                }
            }
        }
    }
}

fn sort_graph_diagnostics(graph: &CompilationGraph, diagnostics: &mut [Diagnostic]) {
    diagnostics.sort_by(|left, right| {
        let key = |diagnostic: &Diagnostic| {
            let label = diagnostic
                .labels
                .iter()
                .find(|label| label.role == LabelRole::Primary)
                .or_else(|| diagnostic.labels.first());
            let span = label.map_or(diagnostic.span, |label| label.span);
            let record = graph.source_map.by_id(span.source);
            (
                record
                    .map(|record| record.package.display_name())
                    .unwrap_or(""),
                record
                    .map(|record| record.display_path.as_str())
                    .unwrap_or(""),
                span,
                diagnostic.code,
            )
        };
        key(left).cmp(&key(right))
    });
}

fn insert_pending_source(
    pending: &mut BTreeMap<String, PendingSource>,
    canonical_owners: &mut BTreeMap<String, String>,
    folded_paths: &mut BTreeMap<(String, String), String>,
    source: PendingSource,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if pending.contains_key(&source.identity.0) {
        diagnostics.push(plan_input(format!(
            "source identity `{}` is present more than once",
            source.identity.0
        )));
        return;
    }
    let canonical_key = source
        .provided
        .canonical_path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| {
            format!(
                "{}:{}",
                source.package.display_name(),
                source.package_relative_path
            )
        });
    if let Some(previous) = canonical_owners.insert(canonical_key, source.identity.0.clone()) {
        diagnostics.push(plan_input(format!(
            "sources `{previous}` and `{}` resolve to the same canonical file",
            source.identity.0
        )));
        return;
    }
    let folded_key = (
        source.package.display_name().to_string(),
        source.package_relative_path.to_ascii_lowercase(),
    );
    if let Some(previous) = folded_paths.insert(folded_key, source.package_relative_path.clone()) {
        if previous != source.package_relative_path {
            diagnostics.push(plan_input(format!(
                "source paths `{previous}` and `{}` collide on case-insensitive filesystems",
                source.package_relative_path
            )));
            return;
        }
    }
    pending.insert(source.identity.0.clone(), source);
}

fn same_canonical_source(candidate: &PendingSource, provided: &ProvidedSource) -> bool {
    match (&candidate.provided.canonical_path, &provided.canonical_path) {
        (Some(left), Some(right)) => left == right,
        (None, None) => {
            candidate.package_relative_path == provided.package_relative_path
                && candidate.provided.display_path == provided.display_path
        }
        _ => false,
    }
}

fn validate_mapping_paths(package: &Package, root: &Path, diagnostics: &mut Vec<Diagnostic>) {
    let mut folded = BTreeSet::new();
    for mapping in &package.namespace_mappings {
        let relative = mapping.path.trim_matches(['/', '\\']);
        if !relative.is_empty() {
            match path_uses_exact_case(root, Path::new(relative)) {
                Ok(true) => {}
                Ok(false) => diagnostics.push(
                    plan_input(format!(
                        "namespace mapping path `{}` for `{}` does not match filesystem casing",
                        mapping.path, package.identity
                    ))
                    .with_title("Namespace Mapping Path Casing Does Not Match"),
                ),
                Err(error) => {
                    diagnostics.push(provider_diagnostic(error, None));
                    continue;
                }
            }
        }
        let candidate = root.join(relative);
        let canonical = match candidate.canonicalize() {
            Ok(path) => path,
            Err(error) => {
                diagnostics.push(plan_input(format!(
                    "namespace mapping path `{}` for `{}` is invalid: {error}",
                    mapping.path, package.identity
                )));
                continue;
            }
        };
        if !canonical.starts_with(root) {
            diagnostics.push(plan_input(format!(
                "namespace mapping path `{}` escapes package `{}`",
                mapping.path, package.identity
            )));
        }
        let key = (
            mapping.prefix.to_ascii_lowercase(),
            mapping.path.replace('\\', "/").to_ascii_lowercase(),
            mapping.scope,
            mapping.generated_for,
        );
        if !folded.insert(key) {
            diagnostics.push(plan_input(format!(
                "package `{}` contains duplicate or case-colliding namespace mappings",
                package.identity
            )));
        }
    }
}

fn validate_package_cycles(
    packages: &[Package],
    active_scopes: &BTreeSet<SourceScope>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let graph = packages
        .iter()
        .map(|package| {
            let edges = package
                .dependencies
                .iter()
                .filter(|dependency| {
                    dependency.kind == DependencyKind::Normal
                        || active_scopes.contains(&SourceScope::Development)
                })
                .map(|dependency| dependency.package.clone())
                .collect::<BTreeSet<_>>();
            (package.identity.clone(), edges)
        })
        .collect::<BTreeMap<_, _>>();
    let mut visited = BTreeSet::new();
    let mut active = Vec::new();
    for package in graph.keys() {
        detect_cycle(package, &graph, &mut visited, &mut active, diagnostics);
    }
}

fn detect_cycle(
    package: &str,
    graph: &BTreeMap<String, BTreeSet<String>>,
    visited: &mut BTreeSet<String>,
    active: &mut Vec<String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Some(index) = active.iter().position(|entry| entry == package) {
        let mut cycle = active[index..].to_vec();
        cycle.push(package.to_string());
        diagnostics.push(
            plan_input(format!("package dependency cycle: {}", cycle.join(" -> ")))
                .with_title("Package Dependency Cycle Is Not Allowed"),
        );
        return;
    }
    if !visited.insert(package.to_string()) {
        return;
    }
    active.push(package.to_string());
    if let Some(edges) = graph.get(package) {
        for dependency in edges {
            detect_cycle(dependency, graph, visited, active, diagnostics);
        }
    }
    active.pop();
}

fn validate_source_shape(
    plan: &crate::build_plan::BuildPlan,
    package: &Package,
    source: &PendingSource,
    authored: &Program,
    project_structure: ProjectStructureAuthority,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let is_entry = plan.selected_target.entry_source.as_deref() == Some(&source.identity.0);
    let has_statements = authored
        .items
        .iter()
        .any(|item| matches!(item, Item::Statement(_)));
    if project_structure == ProjectStructureAuthority::Authoritative
        && has_statements
        && (!is_entry || plan.selected_target.kind == TargetKind::Library)
    {
        diagnostics.push(
            source_language(
                "E0683",
                "top-level executable statements are allowed only in the selected binary entry source",
                authored
                    .items
                    .iter()
                    .find_map(|item| match item {
                        Item::Statement(statement) => Some(statement_span(statement)),
                        _ => None,
                    })
                    .unwrap_or_default(),
                &source.provided.display_path,
            )
            .with_title("Source Is Not A Binary Entry"),
        );
    }
    if source.included && has_statements {
        diagnostics.push(
            source_language(
                "E0679",
                "an included source must contain declarations only",
                authored
                    .items
                    .iter()
                    .find_map(|item| match item {
                        Item::Statement(statement) => Some(statement_span(statement)),
                        _ => None,
                    })
                    .unwrap_or_default(),
                &source.provided.display_path,
            )
            .with_title("Included Source Contains Executable Statements"),
        );
    }
    if project_structure == ProjectStructureAuthority::Authoritative {
        validate_layout(package, source, authored, is_entry, diagnostics);
    }
}

fn validate_layout(
    package: &Package,
    source: &PendingSource,
    authored: &Program,
    is_entry: bool,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let path = source.package_relative_path.replace('\\', "/");
    let external_types = external_type_declarations(authored);
    let actual_namespace = authored
        .namespace
        .as_ref()
        .map(|namespace| namespace.name.canonical())
        .unwrap_or_default();
    let mut mappings = package
        .namespace_mappings
        .iter()
        .filter(|mapping| mapping_applies(mapping, source))
        .filter_map(|mapping| {
            let prefix = mapping.prefix.trim_end_matches('\\');
            let matches = if prefix.is_empty() {
                true
            } else {
                actual_namespace == prefix || actual_namespace.starts_with(&format!("{prefix}\\"))
            };
            matches.then_some((prefix.len(), mapping))
        })
        .collect::<Vec<_>>();
    mappings.sort_by_key(|mapping| std::cmp::Reverse(mapping.0));
    if mappings.len() > 1 && mappings[0].0 == mappings[1].0 {
        diagnostics.push(
            source_language(
                "E0680",
                "the source matches more than one equally specific namespace mapping",
                authored
                    .namespace
                    .as_ref()
                    .map_or_else(Span::default, |namespace| namespace.span),
                &source.provided.display_path,
            )
            .with_title("Namespace Mapping Is Ambiguous"),
        );
        validate_external_type_filename(&path, source, is_entry, &external_types, diagnostics);
        return;
    }
    let Some((_, mapping)) = mappings.first().copied() else {
        diagnostics.push(
            source_language(
                "E0680",
                if authored.namespace.is_some() {
                    "the source namespace has no active namespace mapping"
                } else {
                    "the root-namespace source has no active root namespace mapping"
                },
                authored
                    .namespace
                    .as_ref()
                    .map_or_else(Span::default, |namespace| namespace.span),
                &source.provided.display_path,
            )
            .with_title("Namespace Mapping Is Missing"),
        );
        validate_external_type_filename(&path, source, is_entry, &external_types, diagnostics);
        return;
    };
    let prefix = mapping.prefix.trim_end_matches('\\');
    let namespace_suffix = if prefix.is_empty() {
        actual_namespace.as_str()
    } else {
        actual_namespace
            .strip_prefix(prefix)
            .unwrap_or_default()
            .trim_start_matches('\\')
    };
    let mapping_path = mapping.path.trim_matches('/').replace('\\', "/");
    let expected_directory = if namespace_suffix.is_empty() {
        mapping_path
    } else if mapping_path.is_empty() {
        namespace_suffix.replace('\\', "/")
    } else {
        format!("{mapping_path}/{}", namespace_suffix.replace('\\', "/"))
    };
    let actual_directory = Path::new(&path)
        .parent()
        .and_then(Path::to_str)
        .unwrap_or_default()
        .replace('\\', "/");
    if actual_directory.trim_matches('/') != expected_directory.trim_matches('/') {
        diagnostics.push(
            source_language(
                "E0680",
                format!(
                    "source path `{path}` does not place namespace `{actual_namespace}` beneath mapped directory `{}`",
                    mapping.path
                ),
                authored
                    .namespace
                    .as_ref()
                    .map_or_else(Span::default, |namespace| namespace.span),
                &source.provided.display_path,
            )
            .with_title("Source Namespace Does Not Match Its Path"),
        );
    }
    validate_external_type_filename(&path, source, is_entry, &external_types, diagnostics);
}

fn external_type_declarations(authored: &Program) -> Vec<(&str, Span)> {
    authored
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Class(value) if value.access == MemberAccess::External => {
                Some((value.name.as_str(), value.name_span))
            }
            Item::Enum(value) if value.access == MemberAccess::External => {
                Some((value.name.as_str(), value.name_span))
            }
            Item::Interface(value) if value.access == MemberAccess::External => {
                Some((value.name.as_str(), value.name_span))
            }
            Item::Trait(value) if value.access == MemberAccess::External => {
                Some((value.name.as_str(), value.name_span))
            }
            _ => None,
        })
        .collect()
}

fn validate_external_type_filename(
    path: &str,
    source: &PendingSource,
    is_entry: bool,
    external_types: &[(&str, Span)],
    diagnostics: &mut Vec<Diagnostic>,
) {
    if external_types.len() > 1 {
        let mut diagnostic = source_language(
            "E0680",
            "a source file may expose at most one external type declaration",
            external_types[1].1,
            &source.provided.display_path,
        )
        .with_title("Source Contains Several External Types");
        for (_, span) in external_types {
            diagnostic = diagnostic.with_related(*span, "external type declared here");
        }
        diagnostics.push(diagnostic);
    } else if source.scope != SourceScope::Generated && !is_entry {
        if let Some((name, span)) = external_types.first() {
            let expected = name.rsplit('\\').next().unwrap_or(name);
            let stem = Path::new(&path)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default();
            if stem != expected {
                diagnostics.push(
                    source_language(
                        "E0680",
                        format!(
                            "external type `{expected}` must be declared in `{expected}.doria`, not `{stem}.doria`"
                        ),
                        *span,
                        &source.provided.display_path,
                    )
                    .with_title("External Type Filename Does Not Match"),
                );
            }
        }
    }
}

fn mapping_applies(mapping: &crate::build_plan::NamespaceMapping, source: &PendingSource) -> bool {
    mapping.scope == source.scope && mapping.generated_for == source.generated_for
}

fn graph_fingerprint(
    plan: &crate::build_plan::BuildPlan,
    sources: &BTreeMap<String, GraphSource>,
    include_edges: &[IncludeEdge],
) -> String {
    let mut normalized = plan.clone();
    normalized
        .packages
        .sort_by(|left, right| left.identity.cmp(&right.identity));
    for package in &mut normalized.packages {
        package
            .sources
            .sort_by(|left, right| left.identity.cmp(&right.identity));
        package
            .dependencies
            .sort_by(|left, right| (&left.package, left.kind).cmp(&(&right.package, right.kind)));
        package.namespace_mappings.sort_by(|left, right| {
            (&left.prefix, &left.path, left.scope, left.generated_for).cmp(&(
                &right.prefix,
                &right.path,
                right.scope,
                right.generated_for,
            ))
        });
    }
    normalized.selected_target.active_scopes.sort();
    let mut bytes = serde_json::to_vec(&normalized).expect("typed build plan serializes");
    for (identity, source) in sources {
        bytes.extend_from_slice(identity.as_bytes());
        bytes.extend_from_slice(source.content_fingerprint.as_bytes());
    }
    for edge in include_edges {
        bytes.extend_from_slice(edge.including.0.as_bytes());
        bytes.extend_from_slice(edge.included.0.as_bytes());
    }
    sha256_hex(&bytes)
}

fn provider_diagnostic(error: SourceProviderError, include: Option<(&str, Span)>) -> Diagnostic {
    let (code, title) = match error.kind {
        SourceProviderErrorKind::Missing => ("E0677", "Source File Is Missing"),
        SourceProviderErrorKind::Directory => ("E0677", "Source Path Is A Directory"),
        SourceProviderErrorKind::OutsidePackage => ("E0677", "Source Escapes Its Package"),
        SourceProviderErrorKind::CaseMismatch => ("E0677", "Source Path Casing Does Not Match"),
        SourceProviderErrorKind::InvalidUtf8 => ("E0677", "Source Is Not Valid UTF-8"),
        SourceProviderErrorKind::Unreadable => ("E0677", "Source Could Not Be Read"),
        SourceProviderErrorKind::InvalidPath => ("E0677", "Source Path Is Invalid"),
    };
    let span = include.map_or_else(Span::default, |(_, span)| span);
    let mut diagnostic = crate::build_plan::compiler_input_diagnostic(
        code,
        format!("{}: {}", error.display_path, error.details),
        span,
    )
    .with_title(title)
    .with_developer_details(error.details);
    if let Some((source, _)) = include {
        diagnostic = diagnostic.with_primary_source(DiagnosticSource::Path(source.to_string()));
    }
    diagnostic
}

fn plan_input(message: impl Into<String>) -> Diagnostic {
    crate::build_plan::compiler_input_diagnostic("E0678", message, Span::default())
        .with_title("Compilation Graph Is Invalid")
}

fn source_language(
    code: &'static str,
    message: impl Into<String>,
    span: Span,
    path: &str,
) -> Diagnostic {
    Diagnostic::new(code, message, span)
        .with_primary_source(DiagnosticSource::Path(path.to_string()))
}

fn retarget_diagnostics(mut diagnostics: Vec<Diagnostic>, path: &str) -> Vec<Diagnostic> {
    for diagnostic in &mut diagnostics {
        for label in &mut diagnostic.labels {
            if label.source == DiagnosticSource::Current {
                label.source = DiagnosticSource::Path(path.to_string());
            }
        }
        for fix in &mut diagnostic.fixes {
            for edit in &mut fix.edits {
                if edit.source == DiagnosticSource::Current {
                    edit.source = DiagnosticSource::Path(path.to_string());
                }
            }
        }
    }
    diagnostics
}

fn statement_span(statement: &crate::ast::Stmt) -> Span {
    match statement {
        crate::ast::Stmt::Block(value) => value.span,
        crate::ast::Stmt::VarDecl(value) => value.span,
        crate::ast::Stmt::Assignment(value) => value.span,
        crate::ast::Stmt::Echo { span, .. }
        | crate::ast::Stmt::Return { span, .. }
        | crate::ast::Stmt::Break { span }
        | crate::ast::Stmt::Continue { span }
        | crate::ast::Stmt::Expr { span, .. } => *span,
        crate::ast::Stmt::Throw(value) => value.span,
        crate::ast::Stmt::Try(value) => value.span,
        crate::ast::Stmt::If(value) => value.span,
        crate::ast::Stmt::While(value) => value.span,
        crate::ast::Stmt::DoWhile(value) => value.span,
        crate::ast::Stmt::For(value) => value.span,
        crate::ast::Stmt::Foreach(value) => value.span,
        crate::ast::Stmt::Increment(value) => value.span,
    }
}
