//! Positive nominal proofs under a fixed caller environment. Declaration
//! resolution never contributes its local bounds to that environment.

use super::contracts::{NominalConditionFacts, NominalTypeParameter};
use super::*;
use crate::types::InterfaceType;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct Query {
    pub(super) subject: TypeId,
    pub(super) interface: InterfaceType<TypeId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Step {
    pub(super) query: Query,
    pub(super) origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Failure {
    Unsatisfied(Query),
    Cycle(Vec<Step>),
    UnresolvedCondition(Span),
    MissingSubstitution(NominalTypeParameter),
}

#[derive(Debug, Clone, Default)]
pub(super) struct Context {
    assumptions: HashMap<TypeId, Vec<Step>>,
    proven: HashSet<Query>,
    active: Vec<Step>,
}

impl Context {
    pub(super) fn frozen_premises(&self) -> Self {
        Self {
            assumptions: self.assumptions.clone(),
            ..Self::default()
        }
    }

    pub(super) fn assume(&mut self, step: Step) {
        let assumptions = self.assumptions.entry(step.query.subject).or_default();
        if let Some(existing) = assumptions
            .iter_mut()
            .find(|existing| existing.query == step.query)
        {
            if existing.origin.is_none() {
                existing.origin = step.origin;
            }
        } else {
            assumptions.push(step);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EdgeKind {
    Conformance,
    ClassParent,
    InterfaceParent,
}

pub(super) struct Edge {
    pub(super) ty: TypeId,
    pub(super) origin: Span,
    pub(super) kind: EdgeKind,
    pub(super) conditional: bool,
    pub(super) obligations: Result<Vec<Step>, Failure>,
}

#[derive(Default)]
pub(super) struct Edges {
    pub(super) entries: Vec<Edge>,
    pub(super) valid: bool,
}

impl Checker<'_> {
    /// Authored nominal conformance for interface-valued contracts. Unboxed
    /// primitive constraint facts do not permit interface-valued transport.
    pub(super) fn type_has_declared_interface(
        &mut self,
        subject: TypeId,
        interface: &InterfaceType<TypeId>,
    ) -> bool {
        if self.primitive_interface_erasure_is_forbidden(subject) {
            return false;
        }
        let mut context = self.nominal_proof_context();
        self.prove_nominal(
            Query {
                subject,
                interface: interface.clone(),
            },
            &mut context,
            None,
            &[],
        )
        .is_ok()
    }

    pub(super) fn nominal_proof_context(&mut self) -> Context {
        self.with_nominal_identity(Self::nominal_proof_context_identity)
    }

    fn nominal_proof_context_identity(&mut self) -> Context {
        let mut names = self
            .type_parameter_scopes
            .iter()
            .flat_map(|scope| scope.keys().cloned())
            .collect::<Vec<_>>();
        names.sort();
        names.dedup();
        let mut context = Context::default();
        for name in names {
            let subject = self.types.intern(TypeKind::TypeParameter(name.clone()));
            let interfaces = self.constrained_interfaces(&name, Span::default());
            for interface in interfaces {
                context.assume(Step {
                    query: Query { subject, interface },
                    origin: None,
                });
            }
        }
        for step in self.nominal_condition_scopes.iter().flatten() {
            context.assume(step.clone());
        }
        context
    }

    pub(super) fn condition_queries(
        &mut self,
        condition: &NominalConditionFacts,
        bindings: &HashMap<String, TypeId>,
    ) -> Result<Vec<Step>, Failure> {
        for parameter in &condition.parameters {
            if !bindings.contains_key(&parameter.name) {
                return Err(Failure::MissingSubstitution(parameter.clone()));
            }
        }
        Ok(condition
            .obligations
            .iter()
            .map(|obligation| {
                let interface = InterfaceType::new(
                    &obligation.interface.name,
                    obligation
                        .interface
                        .arguments
                        .iter()
                        .map(|argument| {
                            let argument = self.types.intern_resolved(argument);
                            self.substitute_type_id(argument, bindings)
                        })
                        .collect(),
                );
                let subject = self.types.intern_resolved(&obligation.subject_type);
                let subject = self.substitute_type_id(subject, bindings);
                Step {
                    query: Query { subject, interface },
                    origin: Some(obligation.interface_span),
                }
            })
            .collect())
    }

    pub(super) fn prove_nominal(
        &mut self,
        query: Query,
        context: &mut Context,
        origin: Option<Span>,
        parents: &[(String, usize)],
    ) -> Result<(), Failure> {
        if context.proven.contains(&query) {
            return Ok(());
        }
        let step = Step {
            query: query.clone(),
            origin,
        };
        let nominal_name = match self.types.kind(query.subject) {
            TypeKind::Class(class) => Some(class.name.clone()),
            TypeKind::Interface(interface) => Some(interface.name.clone()),
            _ => None,
        };
        let repeated = context
            .active
            .iter()
            .position(|active| active.query == query)
            .or_else(|| {
                nominal_name.as_ref().and_then(|name| {
                    parents
                        .iter()
                        .find(|(parent, _)| parent == name)
                        .map(|(_, index)| *index)
                })
            });
        if let Some(index) = repeated {
            let mut chain = context.active[index..].to_vec();
            chain.push(step);
            return Err(Failure::Cycle(chain));
        }
        let mut parents = parents.to_vec();
        if let Some(name) = nominal_name {
            parents.push((name, context.active.len()));
        }
        context.active.push(step);
        let result = self.prove_nominal_step(&query, context, &parents);
        context.active.pop();
        if result.is_ok() {
            context.proven.insert(query);
        }
        result
    }

    fn prove_nominal_step(
        &mut self,
        query: &Query,
        context: &mut Context,
        parents: &[(String, usize)],
    ) -> Result<(), Failure> {
        if self.nominal_builtin_fact(query) {
            self.types
                .intern(TypeKind::Interface(query.interface.clone()));
            return Ok(());
        }
        if matches!(self.types.kind(query.subject), TypeKind::Interface(interface) if interface == &query.interface)
        {
            return Ok(());
        }
        let mut failure = Failure::Unsatisfied(query.clone());
        // Explicit assumptions are premises, including when checking a body
        // under its own authored condition. They never come from rule scopes.
        for assumption in context
            .assumptions
            .get(&query.subject)
            .cloned()
            .unwrap_or_default()
        {
            let subject = self
                .types
                .intern(TypeKind::Interface(assumption.query.interface));
            match self.prove_nominal(
                Query {
                    subject,
                    interface: query.interface.clone(),
                },
                context,
                assumption.origin,
                &[],
            ) {
                Ok(()) => return Ok(()),
                Err(cycle @ Failure::Cycle(_)) => failure = cycle,
                Err(_) => {}
            }
        }
        for edge in self.nominal_edges(query.subject).entries {
            let result = edge.obligations.and_then(|obligations| {
                for obligation in obligations {
                    self.prove_nominal(obligation.query, context, obligation.origin, &[])?;
                }
                self.prove_nominal(
                    Query {
                        subject: edge.ty,
                        interface: query.interface.clone(),
                    },
                    context,
                    Some(edge.origin),
                    parents,
                )
            });
            match result {
                Ok(()) => return Ok(()),
                Err(cycle @ Failure::Cycle(_)) => failure = cycle,
                Err(other) if !matches!(failure, Failure::Cycle(_)) => failure = other,
                Err(_) => {}
            }
        }
        // Failed paths may depend on an ancestor still being proved. Caching
        // them would poison an alternative path with an independent seed.
        Err(failure)
    }

    fn nominal_builtin_fact(&self, query: &Query) -> bool {
        let target = &query.interface;
        if matches!(self.types.kind(query.subject), TypeKind::Error) {
            return target.name == "Error" && target.arguments.is_empty();
        }
        if !self.is_core_interface(&target.name) {
            return false;
        }
        if matches!(
            self.types.kind(query.subject),
            TypeKind::Integer(_) | TypeKind::Float(_) | TypeKind::Bool | TypeKind::String
        ) && self.primitive_satisfies_compiler_constraint(query.subject, &target.name)
        {
            return match target.name.as_str() {
                "Comparable" | "Equatable" => target.arguments.as_slice() == [query.subject],
                "Hashable" | "Displayable" => target.arguments.is_empty(),
                _ => false,
            };
        }
        if target.name != "Iterable" {
            return false;
        }
        let element = match self.types.kind(query.subject) {
            TypeKind::TypedArray(element)
            | TypeKind::List(element)
            | TypeKind::Set(element)
            | TypeKind::SortedSet(element)
            | TypeKind::Deque(element) => Some(*element),
            TypeKind::Dictionary(_, value) | TypeKind::SortedDictionary(_, value) => Some(*value),
            _ => None,
        };
        element.is_some_and(|element| target.arguments.as_slice() == [element])
    }

    pub(super) fn nominal_edges(&mut self, subject: TypeId) -> Edges {
        let (parameters, arguments, owner, authored) = match self.types.kind(subject).clone() {
            TypeKind::Interface(interface) => {
                let Some(declaration) = self.interface_declaration(&interface.name).cloned() else {
                    return Edges::default();
                };
                let edges = declaration
                    .parents
                    .into_iter()
                    .zip(declaration.syntax.inheritance_type_spans)
                    .map(|(ty, span)| (ty, span, None, EdgeKind::InterfaceParent))
                    .collect::<Vec<_>>();
                (declaration.type_params, interface.arguments, None, edges)
            }
            TypeKind::Class(class) => {
                let Some(declaration) = self.program.items.iter().find_map(|item| match item {
                    Item::Class(declaration) if declaration.name == class.name => {
                        Some(declaration.clone())
                    }
                    _ => None,
                }) else {
                    return Edges::default();
                };
                let mut edges = declaration
                    .implements
                    .into_iter()
                    .zip(declaration.syntax.inheritance_type_spans)
                    .map(|(entry, span)| {
                        (
                            entry.ty,
                            span,
                            entry.condition.map(|condition| condition.span),
                            EdgeKind::Conformance,
                        )
                    })
                    .collect::<Vec<_>>();
                if let Some(parent) = declaration.parent {
                    edges.push((
                        parent,
                        declaration.parent_span.unwrap_or(declaration.span),
                        None,
                        EdgeKind::ClassParent,
                    ));
                }
                (
                    declaration.type_params,
                    class.arguments,
                    Some(class.name),
                    edges,
                )
            }
            _ => return Edges::default(),
        };
        if parameters.len() != arguments.len() {
            return Edges::default();
        }
        let bindings = parameters
            .iter()
            .zip(arguments)
            .map(|(parameter, argument)| (parameter.name.clone(), argument))
            .collect::<HashMap<_, _>>();
        self.type_parameter_scopes
            .push(type_parameter_scope(&parameters));
        let mut edges = Edges {
            entries: Vec::new(),
            valid: true,
        };
        for (ty, origin, clause, kind) in authored {
            // Declaration-bound checking may need identities before all of
            // its checked records exist. Completed records remain authoritative.
            let resolved = match (kind, self.declared_conformance_types.get(&origin).cloned()) {
                (EdgeKind::Conformance, Some(interface)) => {
                    interface.map(|interface| self.types.intern(TypeKind::Interface(interface)))
                }
                _ => self.with_nominal_identity(|checker| match kind {
                    EdgeKind::Conformance | EdgeKind::InterfaceParent => checker
                        .resolve_interface_edge_in_owner(&ty, origin, owner.as_deref())
                        .map(|interface| checker.types.intern(TypeKind::Interface(interface))),
                    EdgeKind::ClassParent => {
                        let parent =
                            checker.resolve_type_ref_with_class(&ty, origin, owner.as_deref());
                        matches!(checker.types.kind(parent), TypeKind::Class(_)).then_some(parent)
                    }
                }),
            };
            let Some(resolved) = resolved else {
                edges.valid = false;
                continue;
            };
            let ty = self.substitute_type_id(resolved, &bindings);
            let obligations = match clause {
                None => Ok(Vec::new()),
                Some(clause) => match self.contracts.conditions.get(&origin).cloned() {
                    Some(condition) => self.condition_queries(&condition, &bindings),
                    None => Err(Failure::UnresolvedCondition(clause)),
                },
            };
            edges.valid &= obligations.is_ok();
            edges.entries.push(Edge {
                ty,
                origin,
                kind,
                conditional: clause.is_some(),
                obligations,
            });
        }
        self.type_parameter_scopes.pop();
        edges
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_checker(source: &str, inspect: impl FnOnce(&mut Checker<'_>)) {
        let context = crate::names::CompilationContext::standalone("proof.doria");
        let program = crate::parse_source("proof.doria", source).unwrap();
        let resolution = crate::names::resolve_program_for_ide(&program, &context);
        assert!(
            resolution.diagnostics.is_empty(),
            "{:?}",
            resolution.diagnostics
        );
        let program = resolution.resolved.program;
        let source_texts = HashMap::from([(crate::source::SourceId(0), source)]);
        let checker = Checker::new(
            &program,
            crate::const_eval::evaluate_program_with_diagnostics(&program).0,
            &source_texts,
            context,
            HashMap::new(),
            resolution.resolved.facts,
            HashMap::new(),
            crate::testing::TestSemanticFacts::default(),
        );
        super::super::conditional_capabilities::with_checked_program(checker, inspect);
    }

    fn valid(checker: &Checker<'_>) {
        assert!(checker.diagnostics.is_empty(), "{:?}", checker.diagnostics);
        assert!(checker.nominal_condition_scopes.is_empty());
        assert_eq!(checker.nominal_identity_depth, 0);
    }

    fn class(checker: &mut Checker<'_>, name: &str, arguments: Vec<TypeId>) -> TypeId {
        checker
            .types
            .intern(TypeKind::Class(ClassType::new(name, arguments)))
    }

    fn query(subject: TypeId, name: &str, arguments: Vec<TypeId>) -> Query {
        Query {
            subject,
            interface: InterfaceType::new(name, arguments),
        }
    }

    #[test]
    fn frozen_premises_discard_transient_proof_state_and_remain_independent() {
        with_checker(
            "interface Ready {} interface Writable {} class Yes implements Ready {}",
            |checker| {
                valid(checker);
                let parameter = checker.types.intern(TypeKind::TypeParameter("T".into()));
                let yes = class(checker, "Yes", vec![]);
                let mut caller = Context::default();
                caller.assume(Step {
                    query: query(parameter, "Ready", vec![]),
                    origin: None,
                });
                assert!(proves(checker, &mut caller, query(yes, "Ready", vec![])));
                assert!(!caller.proven.is_empty());
                caller.active.push(Step {
                    query: query(parameter, "Writable", vec![]),
                    origin: None,
                });
                let mut frozen = caller.frozen_premises();
                assert!(frozen.proven.is_empty());
                assert!(frozen.active.is_empty());
                assert_eq!(frozen.assumptions, caller.assumptions);
                caller.active.clear();
                caller.assume(Step {
                    query: query(parameter, "Writable", vec![]),
                    origin: None,
                });
                assert!(proves(
                    checker,
                    &mut frozen,
                    query(parameter, "Ready", vec![])
                ));
                assert!(!proves(
                    checker,
                    &mut frozen,
                    query(parameter, "Writable", vec![])
                ));
                assert!(proves(
                    checker,
                    &mut caller,
                    query(parameter, "Writable", vec![])
                ));
            },
        );
    }

    #[test]
    fn a_class_in_an_implements_clause_does_not_grant_its_conformances() {
        with_checker(
            "interface Ready {} class Named implements Ready {} class Holder implements Named {}",
            |checker| {
                let holder = class(checker, "Holder", vec![]);
                let mut context = checker.nominal_proof_context();
                assert!(!proves(
                    checker,
                    &mut context,
                    query(holder, "Ready", vec![])
                ));
                assert!(checker
                    .diagnostics
                    .iter()
                    .any(|finding| finding.code == "E0750"));
            },
        );
    }

    #[test]
    fn composed_conditions_prove_exact_concrete_subjects_without_inventing_conformance() {
        with_checker(
            r#"
interface Ready {}
class Yes implements Ready {}
class No {}
trait Inspection<T> {
    function inspect(): int where (T implements Ready) { return 1; }
}
class Active { uses Inspection<Yes>; }
class Inactive { uses Inspection<No>; }
class Erased { uses Inspection<Ready>; }
class Nullable { uses Inspection<?Yes>; }
"#,
            |checker| {
                valid(checker);
                let mut conditions = checker
                    .contracts
                    .conditions
                    .iter()
                    .filter(|(span, _)| span.expansion != crate::source::ExpansionId::default())
                    .map(|(span, condition)| (*span, condition.clone()))
                    .collect::<Vec<_>>();
                conditions.sort_by_key(|(span, _)| *span);
                assert_eq!(conditions.len(), 4);
                for ((_, condition), expected) in conditions.iter().zip([true, false, true, false])
                {
                    assert!(condition.parameters.is_empty());
                    let steps = checker
                        .condition_queries(condition, &HashMap::new())
                        .unwrap();
                    assert_eq!(steps.len(), 1);
                    let mut context = checker.nominal_proof_context();
                    assert_eq!(
                        proves(checker, &mut context, steps[0].query.clone()),
                        expected
                    );
                }
                let active = class(checker, "Active", vec![]);
                let mut context = checker.nominal_proof_context();
                assert!(!proves(
                    checker,
                    &mut context,
                    query(active, "Ready", vec![])
                ));
            },
        );
    }

    #[test]
    fn composed_method_queries_substitute_owner_and_renamed_method_arguments_once() {
        with_checker(
            r#"
interface Ready<T> {}
class Item {}
class Owner implements Ready<Item> {}
trait Inspection<T> {
    static function inspect<U>(U $value): int where (T implements Ready<U>) { return 1; }
}
class Record<U> { uses Inspection<U>; }
"#,
            |checker| {
                valid(checker);
                let declaration = checker.classes["Record"].methods["inspect"].declaration;
                let condition = checker.contracts.conditions[&declaration].clone();
                assert_eq!(condition.parameters.len(), 2);
                let method_parameter = condition.parameters[1].name.clone();
                assert_ne!(method_parameter, "U");
                let item = class(checker, "Item", vec![]);
                let owner = class(checker, "Owner", vec![]);
                let bindings = HashMap::from([("U".into(), owner), (method_parameter, item)]);
                let steps = checker.condition_queries(&condition, &bindings).unwrap();
                assert_eq!(steps[0].query, query(owner, "Ready", vec![item]));
                let mut context = checker.nominal_proof_context();
                assert!(proves(checker, &mut context, steps[0].query.clone()));
                assert!(matches!(
                    checker.condition_queries(&condition, &HashMap::new()),
                    Err(Failure::MissingSubstitution(_))
                ));
            },
        );
    }

    #[test]
    fn composed_self_queries_bind_the_receiver_without_discovering_executable_instances() {
        with_checker(
            r#"
interface Ready<T> {}
class Item implements Ready<Record<Item>> {}
class Other {}
trait Inspection<T> {
    static function inspect<U>(U $value): int where (T implements Ready<self>) { return 1; }
}
trait Nested<X> { uses Inspection<X>; }
class Record<U> { uses Nested<U>; }
"#,
            |checker| {
                valid(checker);
                let declaration = checker.classes["Record"].methods["inspect"].declaration;
                let condition = checker.contracts.conditions[&declaration].clone();
                let item = class(checker, "Item", vec![]);
                let other = class(checker, "Other", vec![]);
                let receiver = class(checker, "Record", vec![item]);
                let bindings = HashMap::from([
                    ("U".into(), item),
                    (condition.parameters[1].name.clone(), other),
                ]);
                let instances = checker.class_instantiations.clone();
                let steps = checker.condition_queries(&condition, &bindings).unwrap();
                assert_eq!(steps[0].query, query(item, "Ready", vec![receiver]));
                assert_eq!(checker.class_instantiations, instances);
                let mut context = checker.nominal_proof_context();
                assert!(proves(checker, &mut context, steps[0].query.clone()));
            },
        );
    }

    fn proves(checker: &mut Checker<'_>, context: &mut Context, query: Query) -> bool {
        let result = checker.prove_nominal(query, context, None, &[]).is_ok();
        assert!(context.active.is_empty());
        assert!(checker.type_parameter_scopes.is_empty());
        assert_eq!(checker.nominal_identity_depth, 0);
        result
    }

    #[test]
    fn proof_identity_resolution_does_not_discover_class_instances() {
        with_checker("class Box<T> {} class Item {}", |checker| {
            let instances = checker.class_instantiations.clone();
            let templates = checker.nominal_instantiation_templates.clone();
            let callables = checker.callable_nominal_instantiation_templates.clone();
            checker
                .type_parameter_scopes
                .push(HashMap::from([("T".into(), vec![])]));
            let previous = checker.current_callable.replace(Span::new(1, 2));
            let symbolic = TypeRef::generic("Box", vec![TypeRef::named("T")]);
            let concrete = TypeRef::generic("Box", vec![TypeRef::named("Item")]);
            checker.with_nominal_identity(|checker| {
                let symbolic =
                    checker.resolve_type_ref_with_class(&symbolic, Span::default(), Some("Box"));
                let concrete =
                    checker.resolve_type_ref_with_class(&concrete, Span::default(), Some("Box"));
                assert!(matches!(checker.types.kind(concrete), TypeKind::Class(_)));
                let item = class(checker, "Item", vec![]);
                let substituted =
                    checker.substitute_type(symbolic, &HashMap::from([("T".into(), item)]));
                assert_eq!(concrete, substituted);
            });
            assert_eq!(checker.class_instantiations, instances);
            assert_eq!(checker.nominal_instantiation_templates, templates);
            assert_eq!(checker.callable_nominal_instantiation_templates, callables);
            checker.resolve_type_ref_with_class(&concrete, Span::default(), Some("Box"));
            checker.resolve_type_ref_with_class(&symbolic, Span::default(), Some("Box"));
            assert_eq!(checker.class_instantiations, instances);
            assert_eq!(checker.nominal_instantiation_templates, templates);
            let item = checker
                .types
                .intern(TypeKind::Class(ClassType::new("Item", vec![])));
            let parameter = checker.types.intern(TypeKind::TypeParameter("T".into()));
            assert_eq!(
                checker.callable_nominal_instantiation_templates[&Span::new(1, 2)],
                HashSet::from([
                    instantiation::NominalInstance::Class(ClassType::new("Box", vec![item])),
                    instantiation::NominalInstance::Class(ClassType::new("Box", vec![parameter])),
                    instantiation::NominalInstance::Class(ClassType::new("Item", vec![])),
                ]),
            );
            checker.current_callable = previous;
            checker.resolve_type_ref_with_class(&concrete, Span::default(), Some("Box"));
            checker.resolve_type_ref_with_class(&symbolic, Span::default(), Some("Box"));
            assert!(checker
                .class_instantiations
                .iter()
                .any(|instance| instance.name == "Box"));
            assert!(checker.nominal_instantiation_templates.contains_key("Box"));
            assert!(checker
                .callable_nominal_instantiation_templates
                .contains_key(&Span::new(1, 2)));
            checker.type_parameter_scopes.pop();
            valid(checker);
        });
    }

    #[test]
    fn conditional_tiers_and_repeated_adapter_names_keep_exact_specializations() {
        let source = "interface Sync {} interface Ready {} interface Pending extends Sync, Ready {} class Plain implements Sync {} class Full implements Pending {} class Adapter<T> implements Sync, Ready where (T implements Ready), Pending where (T implements Pending) {}";
        with_checker(source, |checker| {
            valid(checker);
            let plain = class(checker, "Plain", vec![]);
            let full = class(checker, "Full", vec![]);
            let plain = class(checker, "Adapter", vec![plain]);
            let full = class(checker, "Adapter", vec![full]);
            let nested = class(checker, "Adapter", vec![full]);
            let mut context = checker.nominal_proof_context();
            for (subject, ready) in [(full, true), (plain, false), (nested, true), (plain, false)] {
                assert!(proves(
                    checker,
                    &mut context,
                    query(subject, "Sync", vec![])
                ));
                assert_eq!(
                    proves(checker, &mut context, query(subject, "Ready", vec![])),
                    ready
                );
                assert_eq!(
                    proves(checker, &mut context, query(subject, "Pending", vec![])),
                    ready
                );
            }
            let erased = checker
                .types
                .intern(TypeKind::Interface(InterfaceType::new("Sync", vec![])));
            assert!(!proves(
                checker,
                &mut context,
                query(erased, "Ready", vec![])
            ));
        });
    }

    #[test]
    fn rule_scope_bounds_do_not_become_caller_assumptions() {
        let source = "interface Ready {} interface Pending extends Ready {} interface Out {} class C<T implements Ready> implements Out where (T implements Ready) {}";
        with_checker(source, |checker| {
            valid(checker);
            let parameter = checker.types.intern(TypeKind::TypeParameter("T".into()));
            let subject = class(checker, "C", vec![parameter]);
            let target = InterfaceType::new("Out", vec![]);
            let mut frozen = checker.nominal_proof_context();
            checker.type_parameter_scopes.push(HashMap::from([(
                "T".into(),
                vec![TypeRef::named("Pending")],
            )]));
            assert!(checker
                .prove_nominal(query(subject, "Out", vec![]), &mut frozen, None, &[])
                .is_err());
            assert!(checker.type_has_declared_interface(subject, &target));
            checker.type_parameter_scopes.pop();
            assert!(!checker.type_has_declared_interface(subject, &target));
        });
    }

    #[test]
    fn multiple_subjects_and_nested_interface_arguments_substitute_together() {
        let source = "interface Ready {} interface Base<T> {} interface Out<T> {} class Item implements Ready {} class Plain {} class Source implements Base<List<Item>> {} class Pair<T, U> implements Out<List<U>> where (T implements Base<List<U>>, U implements Ready) {}";
        with_checker(source, |checker| {
            valid(checker);
            let item = class(checker, "Item", vec![]);
            let plain = class(checker, "Plain", vec![]);
            let source = class(checker, "Source", vec![]);
            let good = class(checker, "Pair", vec![source, item]);
            let bad = class(checker, "Pair", vec![source, plain]);
            let items = checker.types.intern(TypeKind::List(item));
            let plains = checker.types.intern(TypeKind::List(plain));
            let mut context = checker.nominal_proof_context();
            assert!(proves(
                checker,
                &mut context,
                query(good, "Out", vec![items])
            ));
            assert!(!proves(
                checker,
                &mut context,
                query(good, "Out", vec![plains])
            ));
            assert!(!proves(
                checker,
                &mut context,
                query(bad, "Out", vec![plains])
            ));
        });
    }

    #[test]
    fn aliases_resolve_canonically_and_user_contracts_do_not_gain_primitive_facts() {
        let source = "namespace App; use App\\Ready as R; interface Ready {} interface Comparable<T> {} class Source implements R {} class C<T> implements R where (T implements R) {}";
        with_checker(source, |checker| {
            valid(checker);
            let source = class(checker, "App\\Source", vec![]);
            let subject = class(checker, "App\\C", vec![source]);
            let int = checker.types.intern(TypeKind::Integer(IntegerType::Int64));
            let mut context = checker.nominal_proof_context();
            assert!(proves(
                checker,
                &mut context,
                query(subject, "App\\Ready", vec![])
            ));
            assert!(!proves(
                checker,
                &mut context,
                query(int, "App\\Comparable", vec![int])
            ));
        });
    }

    #[test]
    fn conditional_inherited_edges_share_generic_substitution_and_self_context() {
        let source = "interface Ready {} interface Out<T> {} class Item implements Ready {} class Plain {} open class Parent<T> implements Out<self> where (T implements Ready) {} class Child<U> extends Parent<U> {}";
        with_checker(source, |checker| {
            valid(checker);
            let item = class(checker, "Item", vec![]);
            let plain = class(checker, "Plain", vec![]);
            let parent = class(checker, "Parent", vec![item]);
            let child = class(checker, "Child", vec![item]);
            let absent = class(checker, "Child", vec![plain]);
            let mut context = checker.nominal_proof_context();
            assert!(proves(
                checker,
                &mut context,
                query(child, "Out", vec![parent])
            ));
            assert!(!proves(
                checker,
                &mut context,
                query(child, "Out", vec![child])
            ));
            assert!(!proves(
                checker,
                &mut context,
                query(absent, "Out", vec![parent])
            ));
        });
    }

    #[test]
    fn cycles_keep_typed_chains_and_do_not_poison_independently_seeded_paths() {
        with_checker("interface Goal {} interface A extends B, Goal {} interface B extends A {} interface Missing {}", |checker| {
            // Declaration-cycle validation still rejects this source. The
            // proof kernel must also terminate without accepting a self-proof.
            assert!(!checker.diagnostics.is_empty());
            let a = checker.types.intern(TypeKind::Interface(InterfaceType::new("A", vec![])));
            let b = checker.types.intern(TypeKind::Interface(InterfaceType::new("B", vec![])));
            let mut context = checker.nominal_proof_context();
            let failure = checker.prove_nominal(query(a, "Missing", vec![]), &mut context, None, &[]).unwrap_err();
            let Failure::Cycle(chain) = failure else { panic!("cycle") };
            assert_eq!(chain.first().unwrap().query, chain.last().unwrap().query);
            assert!(chain.iter().skip(1).all(|step| step.origin.is_some()));
            assert!(proves(checker, &mut context, query(a, "Goal", vec![])));
            assert!(proves(checker, &mut context, query(b, "Goal", vec![])));
            assert!(!proves(checker, &mut context, query(b, "Missing", vec![])));
        });
    }

    #[test]
    fn expanding_declaration_cycles_terminate_without_an_arbitrary_depth_limit() {
        with_checker(
            "interface Goal {} interface Loop<T> extends Loop<List<T>> {}",
            |checker| {
                assert!(!checker.diagnostics.is_empty());
                let int = checker.types.intern(TypeKind::Integer(IntegerType::Int64));
                let subject = checker
                    .types
                    .intern(TypeKind::Interface(InterfaceType::new("Loop", vec![int])));
                let mut context = checker.nominal_proof_context();
                let failure = checker
                    .prove_nominal(query(subject, "Goal", vec![]), &mut context, None, &[])
                    .unwrap_err();
                let Failure::Cycle(chain) = failure else {
                    panic!("cycle")
                };
                assert_eq!(chain.len(), 2);
                assert_ne!(chain[0].query.subject, chain[1].query.subject);
                assert!(chain[1].origin.is_some());
            },
        );
    }

    #[test]
    fn primitive_and_collection_facts_preserve_exact_types_and_nullable_boundaries() {
        with_checker(
            "class Lookalike { function ready(): int { return 1; } } interface Ready {}",
            |checker| {
                valid(checker);
                let int = checker.types.intern(TypeKind::Integer(IntegerType::Int64));
                let narrow = checker.types.intern(TypeKind::Integer(IntegerType::Int32));
                let float = checker.types.intern(TypeKind::Float(FloatType::Float64));
                let nullable = checker.types.intern(TypeKind::Nullable(int));
                let list = checker.types.intern(TypeKind::List(int));
                let lookalike = class(checker, "Lookalike", vec![]);
                let mut context = checker.nominal_proof_context();
                for (subject, target, args, expected) in [
                    (int, "Equatable", vec![int], true),
                    (int, "Equatable", vec![narrow], false),
                    (float, "Equatable", vec![float], true),
                    (float, "Hashable", vec![], false),
                    (nullable, "Hashable", vec![], false),
                    (list, "Iterable", vec![int], true),
                    (list, "Iterable", vec![narrow], false),
                    (lookalike, "Ready", vec![], false),
                ] {
                    assert_eq!(
                        proves(checker, &mut context, query(subject, target, args)),
                        expected
                    );
                }
                assert!(!checker
                    .type_has_declared_interface(int, &InterfaceType::new("Equatable", vec![int])));
            },
        );
    }

    #[test]
    fn builtin_admission_uses_exact_conditional_and_inherited_specializations() {
        let source = r#"
interface Enabled {}
interface Printable extends Displayable {}
interface Problem extends Error {}
class Yes implements Enabled {}
class No {}
open class Adapter<T> implements Printable where (T implements Enabled), Problem where (T implements Enabled) {
    function __construct(string $message) {}
    function toString(): string { return "adapter"; }
}
class Child<T> extends Adapter<T> {
    function __construct(parameter string $message) { parent::__construct($message); }
}
"#;
        with_checker(source, |checker| {
            valid(checker);
            let yes = class(checker, "Yes", vec![]);
            let no = class(checker, "No", vec![]);
            let active = class(checker, "Adapter", vec![yes]);
            let inactive = class(checker, "Adapter", vec![no]);
            let child_active = class(checker, "Child", vec![yes]);
            let child_inactive = class(checker, "Child", vec![no]);
            let nullable = checker.types.intern(TypeKind::Nullable(active));
            let error = checker.types.intern(TypeKind::Error);
            for (subject, expected) in [
                (inactive, false),
                (active, true),
                (child_inactive, false),
                (child_active, true),
                (active, true),
                (inactive, false),
                (nullable, false),
            ] {
                assert_eq!(checker.type_implements_error(subject), expected);
                assert_eq!(checker.is_assignable(error, subject), expected);
                assert_eq!(
                    matches!(
                        checker.display_conversion_kind(subject),
                        DisplayConversionKind::Displayable
                    ),
                    expected
                );
                for name in ["Displayable", "Error"] {
                    assert_eq!(
                        checker.constraint_accepts_type_argument(
                            &TypeRef::named(name),
                            subject,
                            &HashMap::new(),
                            Span::default()
                        ),
                        expected,
                        "{name}: {}",
                        checker.types.display(subject)
                    );
                }
            }
        });
    }

    #[test]
    fn builtin_admission_uses_only_current_guard_premises() {
        with_checker(
            "interface Enabled {} class Adapter<T> implements Displayable where (T implements Enabled) { function toString(): string { return \"adapter\"; } }",
            |checker| {
                valid(checker);
                let parameter = checker.types.intern(TypeKind::TypeParameter("T".into()));
                let subject = class(checker, "Adapter", vec![parameter]);
                assert!(!checker.is_display_convertible_type(subject));
                assert!(!checker.is_error_interface_receiver(parameter));
                checker.nominal_condition_scopes.push(vec![
                    Step { query: query(parameter, "Enabled", vec![]), origin: None },
                    Step { query: query(parameter, "Error", vec![]), origin: None },
                ]);
                assert!(checker.is_display_convertible_type(subject));
                assert!(checker.type_implements_error(parameter));
                assert!(checker.is_error_interface_receiver(parameter));
                checker.nominal_condition_scopes.pop();
                assert!(!checker.is_display_convertible_type(subject));
                assert!(!checker.type_implements_error(parameter));
                assert!(!checker.is_error_interface_receiver(parameter));
            },
        );
    }

    #[test]
    fn primitive_display_constraints_do_not_authorize_interface_erasure() {
        with_checker("", |checker| {
            valid(checker);
            let int = checker.types.intern(TypeKind::Integer(IntegerType::Int64));
            let float = checker.types.intern(TypeKind::Float(FloatType::Float64));
            let bool_ty = checker.types.intern(TypeKind::Bool);
            let string = checker.types.intern(TypeKind::String);
            let erased = checker.types.intern(TypeKind::Interface(InterfaceType::new(
                "Displayable",
                vec![],
            )));
            for subject in [int, float, bool_ty, string] {
                assert!(matches!(
                    checker.display_conversion_kind(subject),
                    DisplayConversionKind::Primitive
                ));
                assert!(checker.constraint_accepts_type_argument(
                    &TypeRef::named("Displayable"),
                    subject,
                    &HashMap::new(),
                    Span::default()
                ));
                assert!(!checker.type_implements_builtin(subject, BuiltinInterface::Displayable));
                assert!(!checker.is_assignable(erased, subject));
            }
        });
    }

    #[test]
    fn unresolved_conditions_and_missing_parameter_bindings_never_become_unconditional() {
        with_checker(
            "interface Ready {} class C<T> implements Ready where (T implements Ready) {}",
            |checker| {
                valid(checker);
                let condition = checker
                    .contracts
                    .conditions
                    .values()
                    .next()
                    .unwrap()
                    .clone();
                assert!(matches!(
                    checker.condition_queries(&condition, &HashMap::new()),
                    Err(Failure::MissingSubstitution(_))
                ));
                checker.contracts.conditions.clear();
                let ready = checker
                    .types
                    .intern(TypeKind::Interface(InterfaceType::new("Ready", vec![])));
                let subject = class(checker, "C", vec![ready]);
                let mut context = checker.nominal_proof_context();
                assert!(matches!(
                    checker.prove_nominal(query(subject, "Ready", vec![]), &mut context, None, &[]),
                    Err(Failure::UnresolvedCondition(_))
                ));
            },
        );
    }
}
