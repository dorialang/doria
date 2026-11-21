//! What a program emits (Decision 0148 section 17, amendment of 2026-11-21).
//!
//! Checking computes once which functions and methods are reachable from the
//! program's roots, and every backend emits exactly those: MIR lowering, so
//! the interpreter and both native profiles, and PHP output. Every function is
//! still fully checked.
//!
//! The roots are the entry point (`main`, with the entry source's top-level
//! statements), tests, static initializers and every declaration of a
//! compiler-owned source. A program without an entry point is a library, and
//! each declaration with a body is a root. A class constructed in reachable
//! code makes every member of it and of its ancestors reachable: its
//! constructor and property initializers, destructor, property hooks and
//! methods, which covers each virtual and interface dispatch target an
//! instance can reach. A call, construction or C-callable function value
//! written in a reachable body reaches its target, except inside a branch
//! whose condition is constant false for the target; a closure belongs to the
//! body it is written in. An Error class named as a type in reachable code
//! (in `throws`, a `catch` or any other type position) reaches its Error
//! implementation, which identifies the class in reports and catches even
//! when nothing constructs it. Sites in a class declaration outside its
//! members belong to the class.

use super::*;

/// What one recorded site reaches.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum ReachTarget {
    /// A function, method, hook or constructor, by declaration.
    Callable(Span),
    /// The construction of a class, by canonical name.
    Class(String),
    /// A class named as a type, by canonical name: its Error implementation,
    /// when it implements Error.
    NamedType(String),
}

/// The functions and methods a program emits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reachability {
    callables: HashSet<Span>,
    classes: HashSet<String>,
    error_implementations: HashSet<String>,
    dead_branches: Vec<Span>,
}

impl Reachability {
    /// Whether the declaration's body or binding is emitted.
    pub fn reaches(&self, declaration: Span) -> bool {
        self.callables.contains(&declaration)
    }

    /// Whether reachable code constructs the class or a class derived from it.
    pub fn constructs(&self, class: &str) -> bool {
        self.classes.contains(class)
    }

    /// Whether the Error implementation of an Error class is emitted:
    /// reachable code constructs the class or names it as a type. It is the
    /// one interface implementation an unconstructed class has.
    pub fn reaches_error_implementation(&self, class: &str) -> bool {
        self.error_implementations.contains(class)
    }

    /// Whether code at `site` runs: it lies outside every branch whose
    /// condition is constant false for the target.
    pub fn is_live(&self, site: Span) -> bool {
        !self
            .dead_branches
            .iter()
            .any(|branch| branch.contains(site))
    }

    pub(super) fn compute(checker: &Checker<'_>) -> Self {
        let program = checker.callable_body_program();
        let mut regions = Vec::new();
        let mut class_members = HashMap::<&str, Vec<Span>>::new();
        let mut roots = Vec::new();
        let mut root_classes = Vec::new();
        let mut entry = false;
        let mut bodies = Vec::new();
        for item in &program.items {
            match item {
                Item::Function(function) => {
                    regions.push((function.span, Owner::Callable(function.span)));
                    if !matches!(function.body, FunctionBody::Foreign { .. }) {
                        bodies.push(function.span);
                    }
                    if function.span.source.is_compiler_owned() {
                        roots.push(function.span);
                    } else if crate::names::source_name_is(&function.name, "main") {
                        entry = true;
                        roots.push(function.span);
                    }
                }
                Item::Class(class) => {
                    regions.push((class.span, Owner::Class(class.name.as_str())));
                    let members = class_members.entry(class.name.as_str()).or_default();
                    for member in &class.members {
                        for callable in crate::property_hooks::member_callables(
                            member,
                            crate::property_hooks::PropertyHookContext::Class,
                        ) {
                            regions.push((callable.span, Owner::Callable(callable.span)));
                            members.push(callable.span);
                        }
                        let ClassMember::Property(property) = member else {
                            continue;
                        };
                        if let Some(initializer) = &property.initializer {
                            // A static initializer runs at program start; an
                            // instance initializer runs at each construction.
                            regions.push((
                                initializer.span(),
                                if property.is_static {
                                    Owner::Root
                                } else {
                                    Owner::Class(class.name.as_str())
                                },
                            ));
                        }
                    }
                    if class.span.source.is_compiler_owned() {
                        root_classes.push(class.name.as_str());
                    }
                }
                Item::Trait(_)
                | Item::Enum(_)
                | Item::Interface(_)
                | Item::Constant(_)
                | Item::ExternBlock(_)
                | Item::ForeignAggregate(_)
                | Item::Statement(_) => {}
            }
        }
        for test in &checker.test_semantics.tests {
            roots.push(test.declaration_span);
        }
        roots.extend(
            checker
                .test_semantics
                .generated_function_spans
                .iter()
                .copied(),
        );
        if !entry {
            // A library has no entry point: each declaration with a body is
            // part of its output.
            roots.extend(bodies);
            root_classes.extend(class_members.keys().copied());
        }

        let mut sites = checker.reach_sites.clone();
        for (site, name) in &checker.foreign.c_abi_function_values {
            if let Some(function) = checker.functions.get(name) {
                sites.push((*site, ReachTarget::Callable(function.declaration)));
            }
        }
        for (site, target) in &checker.call_targets {
            if let CallableTarget::ConstrainedMethod {
                implementations, ..
            } = target
            {
                sites.extend(implementations.iter().map(|implementation| {
                    (*site, ReachTarget::Callable(implementation.declaration))
                }));
            }
        }
        sites.extend(
            checker
                .generic_call_graph
                .edges()
                .map(|(site, declaration)| (site, ReachTarget::Callable(declaration))),
        );

        let error_classes = checker
            .contracts
            .conformances
            .iter()
            .filter(|conformance| {
                conformance.status == super::contracts::ConformanceStatus::Checked
                    && conformance.interface
                        == crate::types::InterfaceType::new("Error", Vec::new())
            })
            .filter_map(|conformance| match &conformance.implementing_type {
                ResolvedType::Class(class) => Some(class.name.as_str()),
                _ => None,
            })
            .collect::<HashSet<_>>();

        let dead_branches = checker.dead_branches.clone();
        let mut by_owner = HashMap::<Owner<'_>, Vec<ReachTarget>>::new();
        for (site, target) in sites {
            if dead_branches.iter().any(|branch| branch.contains(site)) {
                continue;
            }
            by_owner
                .entry(owner_of(&regions, site))
                .or_default()
                .push(target);
        }

        let mut reachability = Self {
            dead_branches,
            ..Self::default()
        };
        let mut pending = vec![Owner::Root];
        pending.extend(roots.into_iter().map(Owner::Callable));
        pending.extend(root_classes.into_iter().map(Owner::Class));
        let mut visited = HashSet::new();
        while let Some(owner) = pending.pop() {
            if !visited.insert(owner) {
                continue;
            }
            match owner {
                Owner::Root => {}
                Owner::Callable(declaration) => {
                    reachability.callables.insert(declaration);
                }
                Owner::Class(name) => {
                    reachability.classes.insert(name.to_string());
                    if error_classes.contains(name) {
                        reachability.error_implementations.insert(name.to_string());
                    }
                    pending.extend(
                        class_members
                            .get(name)
                            .into_iter()
                            .flatten()
                            .map(|member| Owner::Callable(*member)),
                    );
                    if let Some(parent) = checker
                        .classes
                        .get(name)
                        .and_then(|class| class.parent.as_ref())
                    {
                        if let Some((parent, _)) = class_members.get_key_value(parent.name.as_str())
                        {
                            pending.push(Owner::Class(*parent));
                        }
                    }
                }
            }
            for target in by_owner.get(&owner).into_iter().flatten() {
                pending.push(match target {
                    ReachTarget::Callable(declaration) => Owner::Callable(*declaration),
                    ReachTarget::Class(name) => match class_members.get_key_value(name.as_str()) {
                        Some((name, _)) => Owner::Class(*name),
                        None => continue,
                    },
                    ReachTarget::NamedType(name) => {
                        if error_classes.contains(name.as_str()) {
                            reachability.error_implementations.insert(name.clone());
                        }
                        continue;
                    }
                });
            }
        }
        reachability
    }
}

/// The code a recorded site belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Owner<'a> {
    /// The entry source's top-level statements, static initializers and every
    /// other site outside a callable body.
    Root,
    /// A function, method, hook or constructor body, closures included.
    Callable(Span),
    /// A class's instance property initializers and every other site in its
    /// declaration outside its members and static initializers.
    Class(&'a str),
}

/// The innermost region holding `site`.
fn owner_of<'a>(regions: &[(Span, Owner<'a>)], site: Span) -> Owner<'a> {
    regions
        .iter()
        .filter(|(region, _)| region.contains(site))
        .min_by_key(|(region, _)| region.end - region.start)
        .map_or(Owner::Root, |(_, owner)| *owner)
}
