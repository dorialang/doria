use std::collections::{BTreeMap, BTreeSet};

use crate::ast::{ClassMember, FunctionDecl, Item, MemberAccess, Program};
use crate::build_plan::{BuildPlanDocument, CompilerOptions};
use crate::compilation_graph::CompilationGraph;
use crate::diagnostics::DiagnosticResult;
use crate::runtime_digest::sha256_hex;
use crate::source::SourceId;
use crate::source_provider::SourceProvider;

#[derive(Debug, Clone)]
pub(crate) struct CachedSource {
    pub content_fingerprint: String,
    pub source_id: SourceId,
    pub authored: Program,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceSnapshot {
    context_fingerprint: String,
    declaration_fingerprint: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IncrementalFacts {
    pub parsed_sources: BTreeSet<String>,
    pub reused_sources: BTreeSet<String>,
    pub added_sources: BTreeSet<String>,
    pub removed_sources: BTreeSet<String>,
    pub changed_sources: BTreeSet<String>,
    pub context_changed_sources: BTreeSet<String>,
    pub declaration_changed_sources: BTreeSet<String>,
    pub body_only_changed_sources: BTreeSet<String>,
    pub reused_declaration_indexes: BTreeSet<String>,
    pub invalidated_sources: BTreeSet<String>,
    pub compiler_input_changed: bool,
    pub selected_target_changed: bool,
    pub backend_input_changed: bool,
    pub build_plan_fingerprint: String,
    pub selected_target_fingerprint: String,
    pub backend_input_fingerprint: String,
    pub semantic_dependency_fingerprint: String,
    pub graph_fingerprint: String,
}

#[derive(Debug, Clone)]
pub struct CompilationUpdate {
    pub graph: CompilationGraph,
    pub facts: IncrementalFacts,
}

#[derive(Debug, Clone, Default)]
pub struct CompilationSession {
    cache: BTreeMap<String, CachedSource>,
    current_cache: BTreeMap<String, CachedSource>,
    source_snapshots: BTreeMap<String, SourceSnapshot>,
    facts: IncrementalFacts,
    reverse_dependencies: BTreeMap<String, BTreeSet<String>>,
    reverse_include_dependencies: BTreeMap<String, BTreeSet<String>>,
    build_plan_fingerprint: String,
    selected_target_fingerprint: String,
    backend_input_fingerprint: String,
}

impl CompilationSession {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load_graph(
        &mut self,
        document: &BuildPlanDocument,
        provider: &impl SourceProvider,
    ) -> DiagnosticResult<CompilationUpdate> {
        self.load_graph_with_options(
            document,
            provider,
            crate::compilation_graph::GraphLoadOptions::default(),
        )
    }

    pub fn load_graph_with_options(
        &mut self,
        document: &BuildPlanDocument,
        provider: &impl SourceProvider,
        options: crate::compilation_graph::GraphLoadOptions,
    ) -> DiagnosticResult<CompilationUpdate> {
        self.begin_update();
        let graph = crate::compilation_graph::load_compilation_graph_with_session(
            document, provider, options, self,
        )?;
        self.finish_update(&graph);
        Ok(CompilationUpdate {
            graph,
            facts: self.facts.clone(),
        })
    }

    pub fn last_facts(&self) -> &IncrementalFacts {
        &self.facts
    }

    pub fn analyze_graph(
        &mut self,
        graph: &CompilationGraph,
    ) -> crate::compilation_graph::GraphSemanticAnalysis {
        let analysis = crate::compilation_graph::analyze_compilation_graph_for_ide(graph);
        let mut reverse = BTreeMap::<String, BTreeSet<String>>::new();
        for edge in &analysis.semantic_dependency_edges {
            reverse
                .entry(edge.target.0.clone())
                .or_default()
                .insert(edge.source.0.clone());
        }
        let mut reverse_includes = BTreeMap::<String, BTreeSet<String>>::new();
        for edge in &graph.include_edges {
            reverse_includes
                .entry(edge.included.0.clone())
                .or_default()
                .insert(edge.including.0.clone());
        }
        let mut dependency_surface = String::new();
        for edge in &analysis.semantic_dependency_edges {
            dependency_surface.push_str(&edge.source.0);
            dependency_surface.push('>');
            dependency_surface.push_str(&edge.target.0);
            dependency_surface.push(':');
            dependency_surface.push_str(&edge.symbol.qualified_name);
            dependency_surface.push(':');
            dependency_surface.push_str(&format!("{:?}", edge.role));
            dependency_surface.push(';');
        }
        for edge in &graph.include_edges {
            dependency_surface.push_str("include:");
            dependency_surface.push_str(&edge.including.0);
            dependency_surface.push('>');
            dependency_surface.push_str(&edge.included.0);
            dependency_surface.push(';');
        }
        self.facts.semantic_dependency_fingerprint = sha256_hex(dependency_surface.as_bytes());
        let mut invalidated = self.facts.changed_sources.clone();
        invalidated.extend(self.facts.added_sources.iter().cloned());
        invalidated.extend(self.facts.removed_sources.iter().cloned());
        let mut semantic_pending = self
            .facts
            .context_changed_sources
            .union(&self.facts.declaration_changed_sources)
            .cloned()
            .collect::<BTreeSet<_>>();
        semantic_pending.extend(self.facts.removed_sources.iter().cloned());
        let mut pending = semantic_pending.into_iter().collect::<Vec<_>>();
        while let Some(source) = pending.pop() {
            if let Some(dependents) = self.reverse_dependencies.get(&source) {
                for dependent in dependents {
                    if invalidated.insert(dependent.clone()) {
                        pending.push(dependent.clone());
                    }
                }
            }
        }
        let mut include_pending = self
            .facts
            .changed_sources
            .union(&self.facts.removed_sources)
            .cloned()
            .collect::<Vec<_>>();
        while let Some(source) = include_pending.pop() {
            if let Some(dependents) = self.reverse_include_dependencies.get(&source) {
                for dependent in dependents {
                    if invalidated.insert(dependent.clone()) {
                        include_pending.push(dependent.clone());
                    }
                }
            }
        }
        if self.facts.compiler_input_changed
            || self.facts.selected_target_changed
            || !self.facts.added_sources.is_empty()
        {
            invalidated.extend(graph.sources.keys().cloned());
        }
        self.facts.invalidated_sources = invalidated;
        self.reverse_dependencies = reverse;
        self.reverse_include_dependencies = reverse_includes;
        analysis
    }

    pub(crate) fn cached_for_include(
        &self,
        identity: &str,
        content_fingerprint: &str,
    ) -> Option<Program> {
        self.cache
            .get(identity)
            .filter(|cached| cached.content_fingerprint == content_fingerprint)
            .map(|cached| cached.authored.clone())
    }

    pub(crate) fn cached_for_source(
        &mut self,
        identity: &str,
        content_fingerprint: &str,
        source_id: SourceId,
    ) -> Option<Program> {
        let cached = self.cache.get(identity).filter(|cached| {
            cached.content_fingerprint == content_fingerprint && cached.source_id == source_id
        })?;
        self.facts.reused_sources.insert(identity.to_string());
        Some(cached.authored.clone())
    }

    pub(crate) fn record_parsed_source(
        &mut self,
        identity: String,
        content_fingerprint: String,
        source_id: SourceId,
        authored: Program,
    ) {
        self.facts.parsed_sources.insert(identity.clone());
        if self
            .cache
            .get(&identity)
            .is_some_and(|cached| cached.content_fingerprint != content_fingerprint)
        {
            self.facts.changed_sources.insert(identity.clone());
        }
        self.current_cache.insert(
            identity,
            CachedSource {
                content_fingerprint,
                source_id,
                authored,
            },
        );
    }

    pub(crate) fn record_reused_source(
        &mut self,
        identity: String,
        content_fingerprint: String,
        source_id: SourceId,
        authored: Program,
    ) {
        self.current_cache.insert(
            identity,
            CachedSource {
                content_fingerprint,
                source_id,
                authored,
            },
        );
    }

    fn begin_update(&mut self) {
        self.current_cache.clear();
        self.facts = IncrementalFacts::default();
    }

    fn finish_update(&mut self, graph: &CompilationGraph) {
        let previous = self.cache.keys().cloned().collect::<BTreeSet<_>>();
        let current = self.current_cache.keys().cloned().collect::<BTreeSet<_>>();
        self.facts.added_sources = current.difference(&previous).cloned().collect();
        self.facts.removed_sources = previous.difference(&current).cloned().collect();
        let current_snapshots = graph
            .sources
            .iter()
            .map(|(identity, source)| {
                (
                    identity.clone(),
                    SourceSnapshot {
                        context_fingerprint: source_context_fingerprint(graph, source),
                        declaration_fingerprint: declaration_fingerprint(source),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        for identity in current.intersection(&previous) {
            let Some(current_snapshot) = current_snapshots.get(identity) else {
                continue;
            };
            let Some(previous_snapshot) = self.source_snapshots.get(identity) else {
                continue;
            };
            if current_snapshot.context_fingerprint != previous_snapshot.context_fingerprint {
                self.facts.context_changed_sources.insert(identity.clone());
            }
            if current_snapshot.declaration_fingerprint != previous_snapshot.declaration_fingerprint
            {
                self.facts
                    .declaration_changed_sources
                    .insert(identity.clone());
            } else if current_snapshot.context_fingerprint == previous_snapshot.context_fingerprint
            {
                self.facts
                    .reused_declaration_indexes
                    .insert(identity.clone());
                if self.facts.changed_sources.contains(identity) {
                    self.facts
                        .body_only_changed_sources
                        .insert(identity.clone());
                }
            }
        }

        let build_plan_fingerprint = semantic_build_plan_fingerprint(graph);
        let selected_target_fingerprint =
            fingerprint_serializable(&graph.build_plan.selected_target);
        let backend_input_fingerprint = fingerprint_serializable(&graph.build_plan.compiler);
        self.facts.compiler_input_changed = !self.build_plan_fingerprint.is_empty()
            && self.build_plan_fingerprint != build_plan_fingerprint;
        self.facts.selected_target_changed = !self.selected_target_fingerprint.is_empty()
            && self.selected_target_fingerprint != selected_target_fingerprint;
        self.facts.backend_input_changed = !self.backend_input_fingerprint.is_empty()
            && self.backend_input_fingerprint != backend_input_fingerprint;
        self.facts.build_plan_fingerprint = build_plan_fingerprint.clone();
        self.facts.selected_target_fingerprint = selected_target_fingerprint.clone();
        self.facts.backend_input_fingerprint = backend_input_fingerprint.clone();
        self.facts.graph_fingerprint = graph.fingerprint.clone();
        self.source_snapshots = current_snapshots;
        self.build_plan_fingerprint = build_plan_fingerprint;
        self.selected_target_fingerprint = selected_target_fingerprint;
        self.backend_input_fingerprint = backend_input_fingerprint;
        self.cache = std::mem::take(&mut self.current_cache);
    }
}

fn semantic_build_plan_fingerprint(graph: &CompilationGraph) -> String {
    let mut plan = graph.build_plan.clone();
    plan.compiler = CompilerOptions {
        target: crate::build_plan::CompilerTarget::Debug,
        native_profile: None,
        target_triple: None,
    };
    normalize_plan(&mut plan);
    fingerprint_serializable(&plan)
}

fn normalize_plan(plan: &mut crate::build_plan::BuildPlan) {
    plan.packages
        .sort_by(|left, right| left.identity.cmp(&right.identity));
    for package in &mut plan.packages {
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
    plan.selected_target.active_scopes.sort();
}

fn fingerprint_serializable(value: &impl serde::Serialize) -> String {
    sha256_hex(&serde_json::to_vec(value).expect("typed incremental input serializes"))
}

fn source_context_fingerprint(
    graph: &CompilationGraph,
    source: &crate::compilation_graph::GraphSource,
) -> String {
    let mut surface = format!(
        "package={};source={};scope={:?};origin={:?};generated={:?};included={};",
        source.package.display_name(),
        source.identity.0,
        source.scope,
        source.origin,
        source.generated_for,
        source.included
    );
    if let Some(namespace) = &source.authored.namespace {
        surface.push_str("namespace=");
        surface.push_str(&namespace.name.canonical());
        surface.push(';');
    }
    let mut imports = source
        .authored
        .imports
        .iter()
        .flat_map(|import| {
            import.entries.iter().map(|entry| {
                let prefix = import
                    .prefix
                    .as_ref()
                    .map_or_else(String::new, |prefix| format!("{}\\", prefix.canonical()));
                format!(
                    "{prefix}{} as {}",
                    entry.target.canonical(),
                    entry.alias.as_ref().map_or("", |alias| alias.text.as_str())
                )
            })
        })
        .collect::<Vec<_>>();
    imports.sort();
    for import in imports {
        surface.push_str("use=");
        surface.push_str(&import);
        surface.push(';');
    }
    let mut includes = graph
        .include_edges
        .iter()
        .filter(|edge| edge.including == source.identity)
        .map(|edge| edge.included.0.as_str())
        .collect::<Vec<_>>();
    includes.sort_unstable();
    for included in includes {
        surface.push_str("include=");
        surface.push_str(included);
        surface.push(';');
    }
    sha256_hex(surface.as_bytes())
}

fn declaration_fingerprint(source: &crate::compilation_graph::GraphSource) -> String {
    let mut surface = String::new();
    for attachment in &source.authored.attributes {
        surface.push_str("attribute-target:");
        surface.push_str(&format!("{:?}", attachment.target.kind));
        surface.push(':');
        surface.push_str(&attachment.target.target_span.start.to_string());
        surface.push(':');
        for role in &attachment.target.roles {
            surface.push_str(&format!("{role:?},"));
        }
        for group in &attachment.groups {
            surface.push_str(
                source
                    .source
                    .text
                    .get(group.span.start..group.span.end)
                    .unwrap_or(""),
            );
        }
        surface.push('|');
    }
    for item in &source.authored.items {
        append_item_signature(&mut surface, item, &source.source.text);
    }
    sha256_hex(surface.as_bytes())
}

fn append_item_signature(surface: &mut String, item: &Item, source_text: &str) {
    match item {
        Item::Class(class) => {
            surface.push_str("class:");
            append_access(surface, class.access);
            surface.push_str(&class.name);
            append_type_params(surface, &class.type_params);
            surface.push_str("extends:");
            surface.push_str(class.parent.as_deref().unwrap_or(""));
            surface.push_str("implements:");
            for implemented in &class.implements {
                surface.push_str(implemented);
                surface.push(',');
            }
            for member in &class.members {
                append_member_signature(surface, member, source_text);
            }
        }
        Item::Enum(value) => {
            surface.push_str("enum:");
            append_access(surface, value.access);
            surface.push_str(&value.name);
            append_type_params(surface, &value.type_params);
            append_optional_type(surface, value.backing_type.as_ref());
            for case in &value.cases {
                surface.push_str("case:");
                surface.push_str(&case.name);
                for field in &case.payload {
                    surface.push_str(&field.ty.to_string());
                    surface.push(':');
                    surface.push_str(&field.name);
                    surface.push(',');
                }
                append_optional_expression(surface, case.backing_value.as_ref(), source_text);
            }
        }
        Item::Interface(value) => {
            surface.push_str("interface:");
            append_access(surface, value.access);
            surface.push_str(&value.name);
        }
        Item::Trait(value) => {
            surface.push_str("trait:");
            append_access(surface, value.access);
            surface.push_str(&value.name);
            for member in &value.members {
                append_member_signature(surface, member, source_text);
            }
        }
        Item::Function(function) => {
            append_function_signature(surface, "function", function, source_text)
        }
        Item::Constant(value) => {
            surface.push_str("const:");
            append_access(surface, value.access);
            surface.push_str(&value.name);
            append_optional_type(surface, value.ty.as_ref());
            append_expression(surface, &value.initializer, source_text);
        }
        Item::Statement(_) => {}
    }
    surface.push('|');
}

fn append_member_signature(surface: &mut String, member: &ClassMember, source_text: &str) {
    match member {
        ClassMember::Property(property) => {
            surface.push_str("property:");
            append_access(surface, property.access);
            surface.push(if property.is_static { 's' } else { '-' });
            surface.push(if property.writable { 'w' } else { 'r' });
            surface.push_str(&property.ty.to_string());
            surface.push(':');
            surface.push_str(&property.name);
        }
        ClassMember::Method(function) => {
            append_function_signature(surface, "method", function, source_text)
        }
        ClassMember::Constant(value) => {
            surface.push_str("member-const:");
            append_access(surface, value.access);
            surface.push_str(&value.name);
            append_optional_type(surface, value.ty.as_ref());
            append_expression(surface, &value.initializer, source_text);
        }
    }
}

fn append_function_signature(
    surface: &mut String,
    kind: &str,
    function: &FunctionDecl,
    source_text: &str,
) {
    surface.push_str(kind);
    surface.push(':');
    append_access(surface, function.access);
    surface.push(if function.is_static { 's' } else { '-' });
    surface.push(if function.writable_this { 'w' } else { 'r' });
    surface.push_str(&function.name);
    append_type_params(surface, &function.type_params);
    for parameter in &function.params {
        surface.push_str("param:");
        if let Some(access) = parameter.promoted_access {
            append_access(surface, access);
        }
        surface.push(if parameter.take {
            't'
        } else if parameter.writable {
            'w'
        } else {
            'r'
        });
        surface.push_str(&parameter.ty.to_string());
        surface.push(':');
        surface.push_str(&parameter.name);
        append_optional_expression(surface, parameter.default.as_ref(), source_text);
    }
    append_optional_type(surface, function.return_type.as_ref());
    if let Some(throws) = &function.throws {
        surface.push_str("throws:");
        for entry in &throws.entries {
            surface.push_str(&entry.ty.to_string());
            surface.push(',');
        }
    }
}

fn append_type_params(surface: &mut String, parameters: &[crate::ast::TypeParamDecl]) {
    for parameter in parameters {
        surface.push_str("type-param:");
        surface.push_str(&parameter.name);
        for constraint in &parameter.constraints {
            surface.push_str(&constraint.to_string());
            surface.push(',');
        }
        append_optional_type(surface, parameter.default_type.as_ref());
    }
}

fn append_optional_type(surface: &mut String, ty: Option<&crate::types::TypeRef>) {
    if let Some(ty) = ty {
        surface.push_str(&ty.to_string());
    } else {
        surface.push('-');
    }
    surface.push(';');
}

fn append_optional_expression(
    surface: &mut String,
    expression: Option<&crate::ast::Expr>,
    source_text: &str,
) {
    match expression {
        Some(expression) => append_expression(surface, expression, source_text),
        None => surface.push_str("expr:-;"),
    }
}

fn append_expression(surface: &mut String, expression: &crate::ast::Expr, source_text: &str) {
    let span = expression.span();
    surface.push_str("expr:");
    surface.push_str(
        source_text
            .get(span.start..span.end)
            .unwrap_or("<unavailable>"),
    );
    surface.push(';');
}

fn append_access(surface: &mut String, access: MemberAccess) {
    surface.push_str(match access {
        MemberAccess::External => "external:",
        MemberAccess::Internal => "internal:",
    });
}
