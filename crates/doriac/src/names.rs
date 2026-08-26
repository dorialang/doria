use std::collections::{BTreeMap, HashMap, HashSet};

use crate::ast::*;
use crate::builtins::{is_reserved_intrinsic_name, Builtin};
use crate::diagnostics::{Diagnostic, DiagnosticResult};
use crate::source::{QualifiedNameRef, Span};
use crate::types::{SharedHandleKind, TypeArgumentRef, TypeRef};

pub const EXTERNAL_SYMBOL_BOUNDARY_CODE: &str = "E0671";
pub const INCLUDE_BOUNDARY_CODE: &str = "E0672";
pub const NAMESPACE_NAMING_CODE: &str = "E0675";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Edition {
    Doria2026,
}

impl Edition {
    pub const fn source_name(self) -> &'static str {
        match self {
            Self::Doria2026 => "2026",
        }
    }

    pub fn parse(value: &str, span: Span) -> DiagnosticResult<Self> {
        match value {
            "2026" => Ok(Self::Doria2026),
            _ => Err(vec![Diagnostic::new(
                "E0673",
                format!("unknown Doria edition `{value}`"),
                span,
            )
            .with_title("Unknown Doria Edition")]),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PackageIdentity {
    Standalone,
    Named(String),
    SyntheticTooling(String),
}

impl PackageIdentity {
    pub fn named(value: impl Into<String>, span: Span) -> DiagnosticResult<Self> {
        let value = value.into();
        let mut parts = value.split('/');
        let vendor = parts.next().unwrap_or_default();
        let package = parts.next().unwrap_or_default();
        let valid_part = |part: &str| {
            !part.is_empty()
                && part.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'-' | b'_')
                })
        };
        if parts.next().is_some() || !valid_part(vendor) || !valid_part(package) {
            return Err(vec![Diagnostic::new(
                "E0674",
                format!("invalid compiler package identity `{value}`"),
                span,
            )
            .with_title("Invalid Package Identity")
            .with_help("supply a lowercase `vendor/package` identity")]);
        }
        Ok(Self::Named(value))
    }

    pub fn display_name(&self) -> &str {
        match self {
            Self::Standalone => "standalone",
            Self::Named(name) | Self::SyntheticTooling(name) => name,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceIdentity(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CompilationContext {
    pub edition: Edition,
    pub package: PackageIdentity,
    pub source: SourceIdentity,
}

impl CompilationContext {
    pub fn standalone(source: impl Into<String>) -> Self {
        Self {
            edition: Edition::Doria2026,
            package: PackageIdentity::Standalone,
            source: SourceIdentity(source.into()),
        }
    }
}

impl Default for CompilationContext {
    fn default() -> Self {
        Self::standalone("<unknown>")
    }
}

pub fn source_name_is(name: &str, expected: &str) -> bool {
    name.rsplit('\\').next() == Some(expected)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CompilerSymbolIdentity {
    Prelude(String),
    Intrinsic(String),
    StandardIo(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GlobalSymbolOwner {
    Package(PackageIdentity),
    CompilerKnown(CompilerSymbolIdentity),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GlobalSymbolId {
    pub owner: GlobalSymbolOwner,
    pub qualified_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlobalSymbolKind {
    Class,
    Enum,
    Interface,
    Trait,
    Function,
    Constant,
    CompilerKnownType,
    CompilerKnownIntrinsic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlobalReferenceRole {
    Type,
    Value,
    FunctionCall,
    Constructor,
    StaticQualifier,
    Extends,
    Implements,
    Throws,
    Catch,
    TypeTest,
    MatchPattern,
    ImportTarget,
    ImportAliasUse,
    Include,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalSymbolDeclaration {
    pub id: GlobalSymbolId,
    pub kind: GlobalSymbolKind,
    pub source_name: String,
    pub qualified_name: String,
    pub name_span: Span,
    pub declaration_span: Span,
    pub source_identity: SourceIdentity,
    pub access: MemberAccess,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalSymbolReference {
    pub symbol_id: GlobalSymbolId,
    pub source_span: Span,
    pub role: GlobalReferenceRole,
    pub source_spelling: String,
    pub import_alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportFact {
    pub alias: String,
    pub target: String,
    pub alias_span: Span,
    pub target_span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceFact {
    pub name: QualifiedNameRef,
    pub keyword_span: Span,
    pub semicolon_span: Span,
    pub declaration_span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompilerKnownProvenance {
    EditionPrelude(Edition),
    Intrinsic,
    StandardIo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompilerKnownSymbolFact {
    pub id: GlobalSymbolId,
    pub kind: GlobalSymbolKind,
    pub source_name: String,
    pub provenance: CompilerKnownProvenance,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GlobalSymbolFacts {
    pub namespace: Option<String>,
    pub namespace_declaration: Option<NamespaceFact>,
    pub declarations: Vec<GlobalSymbolDeclaration>,
    pub references: Vec<GlobalSymbolReference>,
    pub imports: Vec<ImportFact>,
    pub compiler_known: Vec<CompilerKnownSymbolFact>,
    pub unresolved: Vec<UnresolvedGlobalReference>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedGlobalReference {
    pub source_span: Span,
    pub role: GlobalReferenceRole,
    pub source_spelling: String,
    pub import_alias: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedProgram {
    pub program: Program,
    pub facts: GlobalSymbolFacts,
}

#[derive(Debug, Clone)]
pub struct ResolutionAnalysis {
    pub resolved: ResolvedProgram,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreludeEntry {
    pub name: &'static str,
    pub reserved: bool,
}

pub const EDITION_2026_PRELUDE: &[PreludeEntry] = &[
    PreludeEntry {
        name: "Displayable",
        reserved: true,
    },
    PreludeEntry {
        name: "Error",
        reserved: true,
    },
    PreludeEntry {
        name: "Comparable",
        reserved: false,
    },
    PreludeEntry {
        name: "Hashable",
        reserved: false,
    },
    PreludeEntry {
        name: "Equatable",
        reserved: false,
    },
    PreludeEntry {
        name: "Int",
        reserved: true,
    },
    PreludeEntry {
        name: "Int8",
        reserved: true,
    },
    PreludeEntry {
        name: "Int16",
        reserved: true,
    },
    PreludeEntry {
        name: "Int32",
        reserved: true,
    },
    PreludeEntry {
        name: "Int64",
        reserved: true,
    },
    PreludeEntry {
        name: "UInt8",
        reserved: true,
    },
    PreludeEntry {
        name: "UInt16",
        reserved: true,
    },
    PreludeEntry {
        name: "UInt32",
        reserved: true,
    },
    PreludeEntry {
        name: "UInt64",
        reserved: true,
    },
    PreludeEntry {
        name: "Float",
        reserved: true,
    },
    PreludeEntry {
        name: "Float32",
        reserved: true,
    },
    PreludeEntry {
        name: "Float64",
        reserved: true,
    },
    PreludeEntry {
        name: "Bool",
        reserved: true,
    },
    PreludeEntry {
        name: "String",
        reserved: true,
    },
    PreludeEntry {
        name: "Bytes",
        reserved: true,
    },
    PreludeEntry {
        name: "List",
        reserved: true,
    },
    PreludeEntry {
        name: "Dictionary",
        reserved: true,
    },
    PreludeEntry {
        name: "SortedDictionary",
        reserved: true,
    },
    PreludeEntry {
        name: "Set",
        reserved: true,
    },
    PreludeEntry {
        name: "SortedSet",
        reserved: true,
    },
    PreludeEntry {
        name: "PriorityQueue",
        reserved: true,
    },
    PreludeEntry {
        name: "Deque",
        reserved: true,
    },
    PreludeEntry {
        name: "SharedReference",
        reserved: true,
    },
    PreludeEntry {
        name: "WeakReference",
        reserved: true,
    },
    PreludeEntry {
        name: "WritableSharedReference",
        reserved: true,
    },
    PreludeEntry {
        name: "WritableWeakReference",
        reserved: true,
    },
    PreludeEntry {
        name: "ReadonlySharedReferenceAccess",
        reserved: true,
    },
    PreludeEntry {
        name: "WritableSharedReferenceAccess",
        reserved: true,
    },
];

pub const fn edition_prelude(edition: Edition) -> &'static [PreludeEntry] {
    match edition {
        Edition::Doria2026 => EDITION_2026_PRELUDE,
    }
}

pub fn resolve_program(
    program: &Program,
    context: &CompilationContext,
) -> DiagnosticResult<ResolvedProgram> {
    let analysis = resolve_program_for_ide(program, context);
    if analysis.diagnostics.is_empty() {
        Ok(analysis.resolved)
    } else {
        Err(analysis.diagnostics)
    }
}

pub fn resolve_program_for_ide(
    program: &Program,
    context: &CompilationContext,
) -> ResolutionAnalysis {
    Resolver::new(program, context).resolve()
}

struct DeclarationRecord {
    id: GlobalSymbolId,
    kind: GlobalSymbolKind,
    source_name: String,
    name_span: Span,
    declaration_span: Span,
    access: MemberAccess,
}

struct Resolver<'a> {
    authored: &'a Program,
    context: &'a CompilationContext,
    namespace: Option<String>,
    declarations: BTreeMap<String, DeclarationRecord>,
    imports: HashMap<String, ImportFact>,
    diagnostics: Vec<Diagnostic>,
    references: Vec<GlobalSymbolReference>,
    unresolved: Vec<UnresolvedGlobalReference>,
    external_causes: HashSet<String>,
}

impl<'a> Resolver<'a> {
    fn new(authored: &'a Program, context: &'a CompilationContext) -> Self {
        Self {
            authored,
            context,
            namespace: authored
                .namespace
                .as_ref()
                .map(|value| value.name.canonical()),
            declarations: BTreeMap::new(),
            imports: HashMap::new(),
            diagnostics: Vec::new(),
            references: Vec::new(),
            unresolved: Vec::new(),
            external_causes: HashSet::new(),
        }
    }

    fn resolve(mut self) -> ResolutionAnalysis {
        self.validate_namespace_name();
        self.collect_declarations();
        self.collect_imports();
        for include in &self.authored.includes {
            self.unresolved.push(UnresolvedGlobalReference {
                source_span: include.literal_span,
                role: GlobalReferenceRole::Include,
                source_spelling: include.value.clone(),
                import_alias: None,
            });
            self.diagnostics.push(
                Diagnostic::unsupported_stage(
                    INCLUDE_BOUNDARY_CODE,
                    "include syntax is accepted; source resolution lands in Stage 31 Slice 2",
                    include.span,
                )
                .with_title("Include Resolution Awaits Stage 31 Slice 2")
                .with_explanation(
                    "The compiler does not emit a runtime include. Slice 2 resolves this path relative to the including source, enforces package containment, and includes each canonical source once.",
                )
                .with_help("keep this include unchanged while the package compilation graph is implemented"),
            );
        }
        let mut program = self.authored.clone();
        self.normalize_program(&mut program);

        let declarations = self
            .declarations
            .values()
            .map(|declaration| GlobalSymbolDeclaration {
                id: declaration.id.clone(),
                kind: declaration.kind,
                source_name: declaration.source_name.clone(),
                qualified_name: declaration.id.qualified_name.clone(),
                name_span: declaration.name_span,
                declaration_span: declaration.declaration_span,
                source_identity: self.context.source.clone(),
                access: declaration.access.clone(),
            })
            .collect();
        let mut imports = self.imports.into_values().collect::<Vec<_>>();
        imports.sort_by_key(|import| import.alias_span.start);
        ResolutionAnalysis {
            resolved: ResolvedProgram {
                program,
                facts: GlobalSymbolFacts {
                    namespace: self.namespace,
                    namespace_declaration: self.authored.namespace.as_ref().map(|namespace| {
                        NamespaceFact {
                            name: namespace.name.clone(),
                            keyword_span: namespace.keyword_span,
                            semicolon_span: namespace.semicolon_span,
                            declaration_span: namespace.span,
                        }
                    }),
                    declarations,
                    references: self.references,
                    imports,
                    compiler_known: compiler_known_symbol_facts(self.context.edition),
                    unresolved: self.unresolved,
                },
            },
            diagnostics: self.diagnostics,
        }
    }

    fn validate_namespace_name(&mut self) {
        let Some(namespace) = &self.authored.namespace else {
            return;
        };
        for segment in &namespace.name.segments {
            if namespace_segment_uses_pascal_case(&segment.text) {
                continue;
            }
            self.diagnostics.push(
                Diagnostic::new(
                    NAMESPACE_NAMING_CODE,
                    format!(
                        "namespace segment `{}` must use PascalCase with folded acronyms",
                        segment.text
                    ),
                    segment.span,
                )
                .with_title("Namespace Segment Must Use PascalCase")
                .with_help("use spellings such as `Acme`, `Io`, or `Http`"),
            );
        }
    }

    fn canonical_declaration_name(&self, source_name: &str) -> String {
        if source_name.contains('\\') || self.namespace.is_none() {
            source_name.to_string()
        } else {
            format!("{}\\{source_name}", self.namespace.as_deref().unwrap())
        }
    }

    fn collect_declarations(&mut self) {
        for item in &self.authored.items {
            let declaration = match item {
                Item::Class(value) => Some((
                    value.name.as_str(),
                    value.name_span,
                    value.span,
                    GlobalSymbolKind::Class,
                    MemberAccess::External,
                )),
                Item::Enum(value) => Some((
                    value.name.as_str(),
                    value.name_span,
                    value.span,
                    GlobalSymbolKind::Enum,
                    MemberAccess::External,
                )),
                Item::Interface(value) => Some((
                    value.name.as_str(),
                    value.name_span,
                    value.span,
                    GlobalSymbolKind::Interface,
                    MemberAccess::External,
                )),
                Item::Trait(value) => Some((
                    value.name.as_str(),
                    value.name_span,
                    value.span,
                    GlobalSymbolKind::Trait,
                    MemberAccess::External,
                )),
                Item::Function(value) => Some((
                    value.name.as_str(),
                    value.name_span,
                    value.span,
                    GlobalSymbolKind::Function,
                    value.access.clone(),
                )),
                Item::Constant(value) => Some((
                    value.name.as_str(),
                    value.name_span,
                    value.span,
                    GlobalSymbolKind::Constant,
                    value.access.clone(),
                )),
                Item::Statement(_) => None,
            };
            let Some((source_name, name_span, declaration_span, kind, access)) = declaration else {
                continue;
            };
            let canonical = self.canonical_declaration_name(source_name);
            let compiler_known = crate::compiler_known_io::is_canonical_type(&canonical);
            let owner = if compiler_known {
                GlobalSymbolOwner::CompilerKnown(CompilerSymbolIdentity::StandardIo(
                    canonical.clone(),
                ))
            } else {
                GlobalSymbolOwner::Package(self.context.package.clone())
            };
            let record = DeclarationRecord {
                id: GlobalSymbolId {
                    owner,
                    qualified_name: canonical.clone(),
                },
                kind: if compiler_known {
                    GlobalSymbolKind::CompilerKnownType
                } else {
                    kind
                },
                source_name: source_name.to_string(),
                name_span,
                declaration_span,
                access,
            };
            if let Some(previous) = self.declarations.get(&canonical) {
                self.diagnostics.push(duplicate_declaration_diagnostic(
                    &canonical, kind, name_span, previous,
                ));
            } else {
                self.declarations.insert(canonical, record);
            }
        }
    }

    fn collect_imports(&mut self) {
        let local_names = self
            .declarations
            .values()
            .map(|declaration| {
                (
                    declaration.source_name.clone(),
                    (declaration.id.qualified_name.clone(), declaration.name_span),
                )
            })
            .collect::<HashMap<_, _>>();
        let mut imported_targets = HashMap::<String, Span>::new();
        for declaration in &self.authored.imports {
            for entry in &declaration.entries {
                let target = if let Some(prefix) = &declaration.prefix {
                    format!("{}\\{}", prefix.canonical(), entry.target.canonical())
                } else {
                    entry.target.canonical()
                };
                let alias = entry
                    .alias
                    .as_ref()
                    .map(|alias| alias.text.clone())
                    .unwrap_or_else(|| entry.target.final_segment().text.clone());
                let alias_span = entry
                    .alias
                    .as_ref()
                    .map_or(entry.target.final_segment().span, |alias| alias.span);
                if is_reserved_intrinsic_name(&alias) {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0670",
                            format!("import alias `{alias}` collides with a language intrinsic"),
                            alias_span,
                        )
                        .with_title("Import Alias Is Reserved"),
                    );
                    continue;
                }
                if edition_prelude(self.context.edition)
                    .iter()
                    .any(|entry| entry.name == alias && entry.reserved)
                    || alias.to_ascii_lowercase().starts_with("__doria")
                {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0670",
                            format!(
                                "import alias `{alias}` collides with a reserved compiler-known name"
                            ),
                            alias_span,
                        )
                        .with_title("Import Alias Is Reserved"),
                    );
                    continue;
                }
                if let Some((local_target, span)) = local_names.get(&alias) {
                    if local_target != &target {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0670",
                                format!(
                                    "import alias `{alias}` collides with a declaration in this file"
                                ),
                                alias_span,
                            )
                            .with_title("Import Alias Conflicts With Declaration")
                            .with_related(*span, "the declaration using this name is here"),
                        );
                        continue;
                    }
                }
                if let Some(previous) = self.imports.get(&alias) {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0670",
                            format!("import alias `{alias}` is declared more than once"),
                            alias_span,
                        )
                        .with_title("Duplicate Import Alias")
                        .with_related(previous.alias_span, "the first alias is here"),
                    );
                    continue;
                }
                if let Some(previous) = imported_targets.get(&target) {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0670",
                            format!("`{target}` is imported more than once"),
                            entry.target.span,
                        )
                        .with_title("Duplicate Import Entry")
                        .with_related(*previous, "the first import is here"),
                    );
                    continue;
                }
                imported_targets.insert(target.clone(), entry.target.span);
                if !self.declarations.contains_key(&target)
                    && !crate::compiler_known_io::is_canonical_type(&target)
                {
                    self.imports.insert(
                        alias.clone(),
                        ImportFact {
                            alias,
                            target: target.clone(),
                            alias_span,
                            target_span: entry.target.span,
                        },
                    );
                    self.report_external(
                        &target,
                        entry.target.span,
                        GlobalReferenceRole::ImportTarget,
                        None,
                    );
                    continue;
                }
                let symbol = self
                    .symbol_id(&target)
                    .expect("known import target has an identity");
                self.references.push(GlobalSymbolReference {
                    symbol_id: symbol,
                    source_span: entry.target.span,
                    role: GlobalReferenceRole::ImportTarget,
                    source_spelling: entry.target.canonical(),
                    import_alias: entry.alias.as_ref().map(|alias| alias.text.clone()),
                });
                self.imports.insert(
                    alias.clone(),
                    ImportFact {
                        alias,
                        target,
                        alias_span,
                        target_span: entry.target.span,
                    },
                );
            }
        }
    }

    fn report_external(
        &mut self,
        name: &str,
        span: Span,
        role: GlobalReferenceRole,
        import_alias: Option<String>,
    ) {
        self.unresolved.push(UnresolvedGlobalReference {
            source_span: span,
            role,
            source_spelling: name.to_string(),
            import_alias,
        });
        if !self.external_causes.insert(name.to_string()) {
            return;
        }
        self.diagnostics.push(
            Diagnostic::unsupported_stage(
                EXTERNAL_SYMBOL_BOUNDARY_CODE,
                format!(
                    "qualified name `{name}` is valid Doria, but the current single-source compiler has no package compilation graph"
                ),
                span,
            )
            .with_title("External Symbol Resolution Awaits Stage 31 Slice 2")
            .with_explanation(
                "Stage 31 Slice 2 indexes build-plan sources and direct dependencies. This source name does not require rewriting.",
            )
            .with_help("keep the qualified name unchanged until the package graph is supplied"),
        );
    }

    fn symbol_id(&self, canonical: &str) -> Option<GlobalSymbolId> {
        if let Some(declaration) = self.declarations.get(canonical) {
            return Some(declaration.id.clone());
        }
        if crate::compiler_known_io::is_canonical_type(canonical) {
            return Some(GlobalSymbolId {
                owner: GlobalSymbolOwner::CompilerKnown(CompilerSymbolIdentity::StandardIo(
                    canonical.to_string(),
                )),
                qualified_name: canonical.to_string(),
            });
        }
        if edition_prelude(self.context.edition)
            .iter()
            .any(|entry| entry.name == canonical)
        {
            return Some(GlobalSymbolId {
                owner: GlobalSymbolOwner::CompilerKnown(CompilerSymbolIdentity::Prelude(
                    canonical.to_string(),
                )),
                qualified_name: canonical.to_string(),
            });
        }
        if Builtin::from_name(canonical).is_some() {
            return Some(GlobalSymbolId {
                owner: GlobalSymbolOwner::CompilerKnown(CompilerSymbolIdentity::Intrinsic(
                    canonical.to_string(),
                )),
                qualified_name: canonical.to_string(),
            });
        }
        None
    }

    fn resolve_name(
        &mut self,
        source_name: &str,
        span: Span,
        role: GlobalReferenceRole,
        intrinsic_position: bool,
    ) -> Option<String> {
        if matches!(source_name, "self" | "parent") {
            return Some(source_name.to_string());
        }
        let (canonical, alias) = if source_name.contains('\\')
            || (intrinsic_position && Builtin::from_name(source_name).is_some())
        {
            (source_name.to_string(), None)
        } else if let Some(import) = self.imports.get(source_name) {
            (import.target.clone(), Some(source_name.to_string()))
        } else {
            let current_namespace = self.namespace.as_ref().map_or_else(
                || source_name.to_string(),
                |namespace| format!("{namespace}\\{source_name}"),
            );
            if self.declarations.contains_key(&current_namespace) {
                (current_namespace, None)
            } else if edition_prelude(self.context.edition)
                .iter()
                .any(|entry| entry.name == source_name)
            {
                (source_name.to_string(), None)
            } else {
                return None;
            }
        };

        let Some(symbol_id) = self.symbol_id(&canonical) else {
            if source_name.contains('\\') || alias.is_some() {
                self.report_external(&canonical, span, role, alias);
            }
            return None;
        };
        self.references.push(GlobalSymbolReference {
            symbol_id,
            source_span: span,
            role,
            source_spelling: source_name.to_string(),
            import_alias: alias,
        });
        Some(canonical)
    }

    fn occurrence_span(&self, source_name: &str, within: Span) -> Span {
        self.authored
            .qualified_names
            .iter()
            .filter(|name| {
                name.canonical() == source_name
                    && name.span.start >= within.start
                    && name.span.end <= within.end
            })
            .min_by_key(|name| name.span.start)
            .map_or(within, |name| name.span)
    }

    fn normalize_program(&mut self, program: &mut Program) {
        for item in &mut program.items {
            self.normalize_item(item);
        }
    }

    fn normalize_item(&mut self, item: &mut Item) {
        match item {
            Item::Class(class) => self.normalize_class(class),
            Item::Enum(definition) => self.normalize_enum(definition),
            Item::Interface(declaration) => {
                declaration.name = self.canonical_declaration_name(&declaration.name);
            }
            Item::Trait(declaration) => {
                declaration.name = self.canonical_declaration_name(&declaration.name);
                for member in &mut declaration.members {
                    self.normalize_class_member(member);
                }
            }
            Item::Function(function) => {
                function.name = self.canonical_declaration_name(&function.name);
                self.normalize_function(function);
            }
            Item::Constant(constant) => {
                constant.name = self.canonical_declaration_name(&constant.name);
                self.normalize_const(constant);
            }
            Item::Statement(statement) => self.normalize_statement(statement),
        }
    }

    fn normalize_class(&mut self, class: &mut ClassDecl) {
        class.name = self.canonical_declaration_name(&class.name);
        if let Some(parent) = &mut class.parent {
            let source_span = self.occurrence_span(parent, class.parent_span.unwrap_or(class.span));
            if let Some(resolved) =
                self.resolve_name(parent, source_span, GlobalReferenceRole::Extends, false)
            {
                *parent = resolved;
            }
        }
        for interface in &mut class.implements {
            let source_span = self.occurrence_span(interface, class.span);
            if let Some(resolved) = self.resolve_name(
                interface,
                source_span,
                GlobalReferenceRole::Implements,
                false,
            ) {
                *interface = resolved;
            }
        }
        self.normalize_type_params(&mut class.type_params);
        for member in &mut class.members {
            self.normalize_class_member(member);
        }
    }

    fn normalize_enum(&mut self, definition: &mut EnumDecl) {
        definition.name = self.canonical_declaration_name(&definition.name);
        self.normalize_type_params(&mut definition.type_params);
        if let Some(ty) = &mut definition.backing_type {
            self.normalize_type(ty, definition.span, GlobalReferenceRole::Type);
        }
        for case in &mut definition.cases {
            for field in &mut case.payload {
                self.normalize_type(&mut field.ty, field.span, GlobalReferenceRole::Type);
            }
            if let Some(value) = &mut case.backing_value {
                self.normalize_expression(value);
            }
        }
    }

    fn normalize_type_params(&mut self, params: &mut [TypeParamDecl]) {
        for param in params {
            for constraint in &mut param.constraints {
                self.normalize_type(constraint, param.span, GlobalReferenceRole::Type);
            }
            if let Some(default) = &mut param.default_type {
                self.normalize_type(default, param.span, GlobalReferenceRole::Type);
            }
        }
    }

    fn normalize_class_member(&mut self, member: &mut ClassMember) {
        match member {
            ClassMember::Property(property) => {
                self.normalize_type(&mut property.ty, property.span, GlobalReferenceRole::Type);
                if let Some(initializer) = &mut property.initializer {
                    self.normalize_expression(initializer);
                }
            }
            ClassMember::Method(method) => self.normalize_function(method),
            ClassMember::Constant(constant) => self.normalize_const(constant),
        }
    }

    fn normalize_const(&mut self, constant: &mut ConstDecl) {
        if let Some(ty) = &mut constant.ty {
            self.normalize_type(ty, constant.span, GlobalReferenceRole::Type);
        }
        self.normalize_expression(&mut constant.initializer);
    }

    fn normalize_function(&mut self, function: &mut FunctionDecl) {
        self.normalize_type_params(&mut function.type_params);
        for param in &mut function.params {
            self.normalize_type(&mut param.ty, param.span, GlobalReferenceRole::Type);
            if let Some(default) = &mut param.default {
                self.normalize_expression(default);
            }
        }
        if let Some(return_type) = &mut function.return_type {
            self.normalize_type(return_type, function.span, GlobalReferenceRole::Type);
        }
        if let Some(throws) = &mut function.throws {
            for entry in &mut throws.entries {
                self.normalize_type(&mut entry.ty, entry.span, GlobalReferenceRole::Throws);
            }
        }
        self.normalize_block(&mut function.body);
    }

    fn normalize_type(&mut self, ty: &mut TypeRef, fallback: Span, role: GlobalReferenceRole) {
        if let Some(grouped) = &mut ty.grouped {
            self.normalize_type(&mut grouped.inner, grouped.span, role);
        }
        if let Some(function) = &mut ty.function {
            for parameter in &mut function.parameters {
                self.normalize_type(
                    &mut parameter.ty,
                    parameter.type_span,
                    GlobalReferenceRole::Type,
                );
            }
            self.normalize_type(
                &mut function.return_type,
                function.return_type_span,
                GlobalReferenceRole::Type,
            );
            if let Some(throws) = &mut function.throws_clause {
                for effect in &mut throws.entries {
                    self.normalize_type(
                        &mut effect.ty,
                        effect.type_span,
                        GlobalReferenceRole::Throws,
                    );
                }
            }
        }
        for argument in &mut ty.arguments {
            if let TypeArgumentRef::Type(argument) = argument {
                self.normalize_type(argument, fallback, GlobalReferenceRole::Type);
            }
        }
        if ty.function.is_some() || ty.grouped.is_some() || is_language_type_name(&ty.name) {
            return;
        }
        let span = ty.source_name.as_ref().map_or(fallback, |name| name.span);
        if let Some(resolved) = self.resolve_name(&ty.name, span, role, false) {
            ty.name = resolved;
        }
    }

    fn normalize_arguments(&mut self, arguments: &mut [Argument]) {
        for argument in arguments {
            self.normalize_expression(&mut argument.value);
        }
    }

    fn normalize_block(&mut self, block: &mut Block) {
        for statement in &mut block.statements {
            self.normalize_statement(statement);
        }
    }

    fn normalize_statement(&mut self, statement: &mut Stmt) {
        match statement {
            Stmt::Block(block) => self.normalize_block(block),
            Stmt::VarDecl(declaration) => {
                if let Some(ty) = &mut declaration.ty {
                    self.normalize_type(ty, declaration.span, GlobalReferenceRole::Type);
                }
                self.normalize_expression(&mut declaration.initializer);
            }
            Stmt::Assignment(assignment) => {
                self.normalize_expression(&mut assignment.target);
                self.normalize_expression(&mut assignment.value);
            }
            Stmt::Echo { expr, .. } => self.normalize_expression(expr),
            Stmt::Return { expr, .. } => {
                if let Some(expr) = expr {
                    self.normalize_expression(expr);
                }
            }
            Stmt::Throw(statement) => self.normalize_expression(&mut statement.expr),
            Stmt::Try(statement) => {
                self.normalize_block(&mut statement.body);
                for clause in &mut statement.catches {
                    self.normalize_type(&mut clause.ty, clause.ty_span, GlobalReferenceRole::Catch);
                    self.normalize_block(&mut clause.body);
                }
                if let Some(finally) = &mut statement.finally {
                    self.normalize_block(&mut finally.body);
                }
            }
            Stmt::If(statement) => {
                self.normalize_given(&mut statement.given);
                self.normalize_expression(&mut statement.condition);
                self.normalize_block(&mut statement.then_block);
                if let Some(branch) = &mut statement.else_branch {
                    match branch {
                        ElseBranch::If(statement) => self.normalize_if(statement),
                        ElseBranch::Block(block) => self.normalize_block(block),
                    }
                }
                self.normalize_finally(&mut statement.finally);
            }
            Stmt::While(statement) => {
                self.normalize_given(&mut statement.given);
                self.normalize_expression(&mut statement.condition);
                self.normalize_block(&mut statement.body);
                self.normalize_finally(&mut statement.finally);
            }
            Stmt::DoWhile(statement) => {
                self.normalize_block(&mut statement.body);
                self.normalize_expression(&mut statement.condition);
                self.normalize_finally(&mut statement.finally);
            }
            Stmt::For(statement) => {
                if let Some(initializer) = &mut statement.initializer {
                    match initializer {
                        ForInitializer::VarDecl(declaration) => {
                            if let Some(ty) = &mut declaration.ty {
                                self.normalize_type(
                                    ty,
                                    declaration.span,
                                    GlobalReferenceRole::Type,
                                );
                            }
                            self.normalize_expression(&mut declaration.initializer);
                        }
                        ForInitializer::Assignment(assignment) => {
                            self.normalize_expression(&mut assignment.target);
                            self.normalize_expression(&mut assignment.value);
                        }
                    }
                }
                if let Some(condition) = &mut statement.condition {
                    self.normalize_expression(condition);
                }
                if let Some(increment) = &mut statement.increment {
                    match increment {
                        ForIncrement::Increment(increment) => {
                            self.normalize_expression(&mut increment.target);
                        }
                        ForIncrement::Assignment(assignment) => {
                            self.normalize_expression(&mut assignment.target);
                            self.normalize_expression(&mut assignment.value);
                        }
                    }
                }
                self.normalize_block(&mut statement.body);
            }
            Stmt::Break { .. } | Stmt::Continue { .. } => {}
            Stmt::Foreach(statement) => {
                self.normalize_expression(&mut statement.iterable);
                if let Some(key) = &mut statement.key {
                    if let Some(ty) = &mut key.ty {
                        self.normalize_type(ty, key.span, GlobalReferenceRole::Type);
                    }
                }
                if let Some(ty) = &mut statement.value.ty {
                    self.normalize_type(ty, statement.value.span, GlobalReferenceRole::Type);
                }
                self.normalize_block(&mut statement.body);
            }
            Stmt::Increment(statement) => {
                self.normalize_expression(&mut statement.target);
            }
            Stmt::Expr { expr, .. } => self.normalize_expression(expr),
        }
    }

    fn normalize_if(&mut self, statement: &mut IfStmt) {
        self.normalize_given(&mut statement.given);
        self.normalize_expression(&mut statement.condition);
        self.normalize_block(&mut statement.then_block);
        if let Some(branch) = &mut statement.else_branch {
            match branch {
                ElseBranch::If(statement) => self.normalize_if(statement),
                ElseBranch::Block(block) => self.normalize_block(block),
            }
        }
        self.normalize_finally(&mut statement.finally);
    }

    fn normalize_given(&mut self, given: &mut Option<GivenPrelude>) {
        if let Some(given) = given {
            self.normalize_block(&mut given.block);
        }
    }

    fn normalize_finally(&mut self, finally: &mut Option<ControlFlowFinally>) {
        if let Some(finally) = finally {
            self.normalize_block(&mut finally.block);
        }
    }

    fn normalize_expression(&mut self, expression: &mut Expr) {
        match expression {
            Expr::Variable { .. }
            | Expr::This { .. }
            | Expr::String { .. }
            | Expr::Int { .. }
            | Expr::Float { .. }
            | Expr::Bool { .. }
            | Expr::Null { .. } => {}
            Expr::Identifier { name, span } => {
                if let Some(resolved) =
                    self.resolve_name(name, *span, GlobalReferenceRole::Value, true)
                {
                    *name = resolved;
                }
            }
            Expr::InterpolatedString { parts, .. } => {
                for part in parts {
                    if let InterpolatedStringPart::Expr(expression) = part {
                        self.normalize_expression(expression);
                    }
                }
            }
            Expr::Array { elements, .. } => {
                for element in elements {
                    if let Some(key) = &mut element.key {
                        self.normalize_expression(key);
                    }
                    self.normalize_expression(&mut element.value);
                }
            }
            Expr::ArrayRepeat { value, count, .. } => {
                self.normalize_expression(value);
                self.normalize_expression(count);
            }
            Expr::Index {
                collection, index, ..
            } => {
                self.normalize_expression(collection);
                self.normalize_expression(index);
            }
            Expr::PropertyAccess { object, .. } => self.normalize_expression(object),
            Expr::MethodCall { object, args, .. } => {
                self.normalize_expression(object);
                self.normalize_arguments(args);
            }
            Expr::IsType { expr, ty, span } => {
                self.normalize_expression(expr);
                self.normalize_type(ty, *span, GlobalReferenceRole::TypeTest);
            }
            Expr::FunctionCall { name, args, span } => {
                let source_name = name.clone();
                let source_span = self.occurrence_span(&source_name, *span);
                if let Some(resolved) = self.resolve_name(
                    &source_name,
                    source_span,
                    GlobalReferenceRole::FunctionCall,
                    true,
                ) {
                    *name = resolved;
                }
                self.normalize_arguments(args);
            }
            Expr::CallableCall { callee, args, .. } => {
                self.normalize_expression(callee);
                self.normalize_arguments(args);
            }
            Expr::StaticCall {
                qualifier,
                qualifier_span,
                args,
                ..
            } => {
                self.normalize_static_qualifier(
                    qualifier,
                    *qualifier_span,
                    GlobalReferenceRole::StaticQualifier,
                );
                self.normalize_arguments(args);
            }
            Expr::StaticMember {
                qualifier,
                qualifier_span,
                ..
            } => self.normalize_static_qualifier(
                qualifier,
                *qualifier_span,
                GlobalReferenceRole::StaticQualifier,
            ),
            Expr::New {
                class_type,
                args,
                span,
                ..
            } => {
                self.normalize_type(class_type, *span, GlobalReferenceRole::Constructor);
                self.normalize_arguments(args);
            }
            Expr::Grouped { expr, .. } | Expr::Unary { expr, .. } => {
                self.normalize_expression(expr);
            }
            Expr::Binary { left, right, .. }
            | Expr::Range {
                start: left,
                end: right,
                ..
            } => {
                self.normalize_expression(left);
                self.normalize_expression(right);
            }
            Expr::Match {
                scrutinee, arms, ..
            } => {
                self.normalize_expression(scrutinee);
                for arm in arms {
                    match &mut arm.pattern {
                        MatchPattern::Default { .. } => {}
                        MatchPattern::EnumCase {
                            qualifier,
                            qualifier_span,
                            ..
                        } => {
                            if let Some(resolved) = self.resolve_name(
                                qualifier,
                                *qualifier_span,
                                GlobalReferenceRole::MatchPattern,
                                false,
                            ) {
                                *qualifier = resolved;
                            }
                        }
                        MatchPattern::TypeBinding { ty, span, .. } => {
                            self.normalize_type(ty, *span, GlobalReferenceRole::MatchPattern);
                        }
                        MatchPattern::Expression(expression) => {
                            self.normalize_expression(expression);
                        }
                    }
                    if let Some(guard) = &mut arm.guard {
                        self.normalize_expression(&mut guard.condition);
                    }
                    self.normalize_expression(&mut arm.value);
                }
            }
            Expr::When(when) => {
                self.normalize_given(&mut when.given);
                if let Some(result_type) = &mut when.result_type {
                    self.normalize_type(result_type, when.span, GlobalReferenceRole::Type);
                }
                for branch in &mut when.branches {
                    if let Some(condition) = &mut branch.condition {
                        self.normalize_expression(condition);
                    }
                    self.normalize_block(&mut branch.block);
                }
                self.normalize_finally(&mut when.finally);
            }
            Expr::Closure(closure) => {
                for parameter in &mut closure.parameters {
                    self.normalize_type(
                        &mut parameter.ty,
                        parameter.type_span,
                        GlobalReferenceRole::Type,
                    );
                }
                if let Some(return_type) = &mut closure.return_type {
                    self.normalize_type(
                        &mut return_type.ty,
                        return_type.type_span,
                        GlobalReferenceRole::Type,
                    );
                }
                match &mut closure.body {
                    ClosureBody::Expression { expression, .. } => {
                        self.normalize_expression(expression);
                    }
                    ClosureBody::Block(block) => self.normalize_block(block),
                }
            }
        }
    }

    fn normalize_static_qualifier(
        &mut self,
        qualifier: &mut StaticQualifier,
        span: Span,
        role: GlobalReferenceRole,
    ) {
        if let StaticQualifier::Class(name) = qualifier {
            if let Some(resolved) = self.resolve_name(name, span, role, false) {
                *name = resolved;
            }
        }
    }
}

fn namespace_segment_uses_pascal_case(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    bytes.first().is_some_and(u8::is_ascii_uppercase)
        && bytes.iter().all(u8::is_ascii_alphanumeric)
        && !bytes
            .windows(2)
            .any(|pair| pair[0].is_ascii_uppercase() && pair[1].is_ascii_uppercase())
}

fn duplicate_declaration_diagnostic(
    canonical: &str,
    kind: GlobalSymbolKind,
    span: Span,
    previous: &DeclarationRecord,
) -> Diagnostic {
    let (code, title, message) = match (previous.kind, kind) {
        (GlobalSymbolKind::Class, GlobalSymbolKind::Class) => (
            "E0300",
            "Duplicate Class",
            format!("class `{canonical}` is already declared"),
        ),
        (GlobalSymbolKind::Function, GlobalSymbolKind::Function) => (
            "E0308",
            "Duplicate Function",
            format!("function `{canonical}` is already declared"),
        ),
        (GlobalSymbolKind::Constant, GlobalSymbolKind::Constant) => (
            "E0481",
            "Duplicate Constant",
            format!("constant `{canonical}` is already declared"),
        ),
        (GlobalSymbolKind::Enum, GlobalSymbolKind::Enum) => (
            "E0560",
            "Duplicate Enum",
            format!("enum `{canonical}` is already declared"),
        ),
        (previous, current)
            if is_type_declaration_kind(previous) && is_type_declaration_kind(current) =>
        {
            (
                "E0561",
                "Type Name Collision",
                format!("type name `{canonical}` is already used by another declaration"),
            )
        }
        _ => (
            "E0669",
            "Duplicate Global Declaration",
            format!("duplicate global declaration `{canonical}`"),
        ),
    };
    Diagnostic::new(code, message, span)
        .with_title(title)
        .with_related(previous.name_span, "the first declaration is here")
}

fn is_type_declaration_kind(kind: GlobalSymbolKind) -> bool {
    matches!(
        kind,
        GlobalSymbolKind::Class
            | GlobalSymbolKind::Enum
            | GlobalSymbolKind::Interface
            | GlobalSymbolKind::Trait
    )
}

pub fn compiler_known_symbol_facts(edition: Edition) -> Vec<CompilerKnownSymbolFact> {
    let mut facts = edition_prelude(edition)
        .iter()
        .map(|entry| CompilerKnownSymbolFact {
            id: GlobalSymbolId {
                owner: GlobalSymbolOwner::CompilerKnown(CompilerSymbolIdentity::Prelude(
                    entry.name.to_string(),
                )),
                qualified_name: entry.name.to_string(),
            },
            kind: GlobalSymbolKind::CompilerKnownType,
            source_name: entry.name.to_string(),
            provenance: CompilerKnownProvenance::EditionPrelude(edition),
        })
        .collect::<Vec<_>>();
    facts.extend(Builtin::ALL.into_iter().map(|builtin| {
        let name = builtin.name().to_string();
        CompilerKnownSymbolFact {
            id: GlobalSymbolId {
                owner: GlobalSymbolOwner::CompilerKnown(CompilerSymbolIdentity::Intrinsic(
                    name.clone(),
                )),
                qualified_name: name.clone(),
            },
            kind: GlobalSymbolKind::CompilerKnownIntrinsic,
            source_name: name,
            provenance: CompilerKnownProvenance::Intrinsic,
        }
    }));
    facts.extend(crate::compiler_known_io::CANONICAL_TYPES.map(|canonical| {
        CompilerKnownSymbolFact {
            id: GlobalSymbolId {
                owner: GlobalSymbolOwner::CompilerKnown(CompilerSymbolIdentity::StandardIo(
                    canonical.to_string(),
                )),
                qualified_name: canonical.to_string(),
            },
            kind: GlobalSymbolKind::CompilerKnownType,
            source_name: canonical.to_string(),
            provenance: CompilerKnownProvenance::StandardIo,
        }
    }));
    facts
}

fn is_language_type_name(name: &str) -> bool {
    matches!(
        name,
        "void"
            | "int"
            | "int8"
            | "int16"
            | "int32"
            | "int64"
            | "uint8"
            | "uint16"
            | "uint32"
            | "uint64"
            | "float"
            | "float32"
            | "float64"
            | "string"
            | "bool"
            | "null"
            | "mixed"
            | "object"
            | "resource"
            | "array"
            | "Unknown"
            | "[]"
            | "self"
    ) || SharedHandleKind::from_source_name(name).is_some()
}
