use doriac::ast::{ClassMember, Item};
use doriac::semantics::contracts::NominalConditionFacts;
use doriac::source::{ExpansionId, Span};
use doriac::types::{ClassType, InterfaceType, ResolvedType};

fn analyze(source: &str) -> (doriac::ast::Program, doriac::semantics::SemanticAnalysis) {
    let (program, analysis) =
        doriac::analyze_source_for_ide("condition-composition.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{source}: {:?}",
        analysis.diagnostics
    );
    (program, analysis)
}

fn effective_conditions(
    analysis: &doriac::semantics::SemanticAnalysis,
) -> Vec<(Span, &NominalConditionFacts)> {
    let mut conditions = analysis
        .info
        .contracts
        .conditions
        .iter()
        .filter(|(span, _)| span.expansion != ExpansionId::default())
        .map(|(span, condition)| (*span, condition))
        .collect::<Vec<_>>();
    conditions.sort_by_key(|(span, _)| *span);
    conditions
}

#[test]
fn trait_conditions_preserve_authored_identity_and_substitute_owner_parameters() {
    let source = r#"
interface Ready<T> {}
trait Inspection<T> {
    function inspect(T $value): int where (T implements Ready<T>) { return 1; }
}
class Record<U> { uses Inspection<U>; }
"#;
    let (program, analysis) = analyze(source);
    let Item::Trait(declaration) = &program.items[1] else {
        panic!("trait")
    };
    let ClassMember::Method(method) = &declaration.members[0] else {
        panic!("method")
    };
    let Item::Class(class) = &program.items[2] else {
        panic!("class")
    };
    let conditions = effective_conditions(&analysis);
    assert_eq!(conditions.len(), 1);
    let (span, condition) = conditions[0];
    let authored = &analysis.info.contracts.conditions[&method.span];
    assert_eq!(span.start, method.span.start);
    assert_eq!(condition.clause.expansion, span.expansion);
    assert_eq!(condition.clause.start, authored.clause.start);
    assert_eq!(condition.parameters.len(), 1);
    assert_eq!(condition.parameters[0].name, "U");
    assert_eq!(
        condition.parameters[0].declaration,
        class.type_params[0].span
    );
    assert_eq!(
        condition.obligations[0].subject,
        authored.obligations[0].subject
    );
    assert_eq!(
        condition.obligations[0].subject_span,
        authored.obligations[0].subject_span
    );
    assert_eq!(
        condition.obligations[0].subject_type,
        ResolvedType::TypeParameter("U".into())
    );
    assert_eq!(
        condition.obligations[0].interface,
        InterfaceType::new("Ready", vec![ResolvedType::TypeParameter("U".into())])
    );
    assert_eq!(analysis.info.contracts.callable_conditions[&span], span);
}

#[test]
fn concrete_substitution_retains_false_predicates_and_exact_nominal_categories() {
    let source = r#"
interface Ready {}
enum Mode { case Active; }
class No {}
trait Inspection<T> {
    function inspect(T $value): int where (T implements Ready) { return 1; }
}
class Record { uses Inspection<No>; }
class Erased { uses Inspection<Ready>; }
class EnumValue { uses Inspection<Mode>; }
class Nullable { uses Inspection<?No>; }
"#;
    let (_, analysis) = analyze(source);
    let conditions = effective_conditions(&analysis);
    let subjects = conditions
        .iter()
        .map(|(_, condition)| {
            assert!(condition.parameters.is_empty());
            assert_eq!(condition.obligations[0].subject.name, "T");
            condition.obligations[0].subject_type.clone()
        })
        .collect::<Vec<_>>();
    let no = ResolvedType::Class(ClassType::new("No", vec![]));
    assert_eq!(subjects.len(), 4);
    assert_eq!(subjects[0], no);
    assert_eq!(
        subjects[1],
        ResolvedType::Interface(InterfaceType::new("Ready", vec![]))
    );
    assert!(matches!(&subjects[2], ResolvedType::Enum(ty) if ty.name == "Mode"));
    assert_eq!(subjects[3], ResolvedType::Nullable(Box::new(no)));
}

#[test]
fn nested_traits_compose_checked_types_simultaneously_in_both_source_orders() {
    let root = "interface Ready<T> {} trait Root<T, U> { function inspect(T $value): int where (T implements Ready<U>, U implements Ready<T>) { return 1; } }";
    let middle = "trait Middle<X, Y> { uses Root<List<Y>, X>; }";
    let record = "class Record<A, B> { uses Middle<B, A>; }";
    for source in [
        format!("{root} {middle} {record}"),
        format!("{record} {middle} {root}"),
    ] {
        let (_, analysis) = analyze(&source);
        let conditions = effective_conditions(&analysis);
        assert_eq!(conditions.len(), 1);
        let condition = conditions[0].1;
        let a = ResolvedType::List(Box::new(ResolvedType::TypeParameter("A".into())));
        let b = ResolvedType::TypeParameter("B".into());
        assert_eq!(condition.obligations[0].subject_type, a);
        assert_eq!(
            condition.obligations[0].interface.arguments,
            vec![b.clone()]
        );
        assert_eq!(condition.obligations[1].subject_type, b);
        assert_eq!(condition.obligations[1].interface.arguments, vec![a]);
        assert_eq!(
            condition
                .parameters
                .iter()
                .map(|parameter| parameter.name.as_str())
                .collect::<Vec<_>>(),
            vec!["A", "B"]
        );
    }
}

#[test]
fn method_parameter_renaming_cannot_capture_a_composing_class_parameter() {
    let source = r#"
interface Ready<T> {}
trait Inspection<T> {
    static function inspect<U>(U $value): int where (T implements Ready<List<U>>, U implements Ready<T>) { return 1; }
}
class Record<U> { uses Inspection<U>; }
"#;
    let (_, analysis) = analyze(source);
    let conditions = effective_conditions(&analysis);
    assert_eq!(conditions.len(), 1);
    let condition = conditions[0].1;
    assert_eq!(condition.parameters.len(), 2);
    let owner = ResolvedType::TypeParameter("U".into());
    let method = ResolvedType::TypeParameter(condition.parameters[1].name.clone());
    assert!(condition.parameters[1].name.starts_with("U#trait"));
    assert_eq!(
        condition.parameters[1].declaration.expansion,
        conditions[0].0.expansion
    );
    assert_eq!(condition.obligations[0].subject_type, owner);
    assert_eq!(
        condition.obligations[0].interface.arguments,
        vec![ResolvedType::List(Box::new(method.clone()))]
    );
    assert_eq!(condition.obligations[1].subject_type, method);
    assert_eq!(condition.obligations[1].interface.arguments, vec![owner]);
}

#[test]
fn nested_trait_self_keeps_the_composing_class_and_its_owner_arguments() {
    let source = r#"
interface Ready<T> {}
trait Inspection<T> {
    static function inspect<U>(U $value): int where (T implements Ready<List<self>>, U implements Ready<self>) { return 1; }
}
trait Nested<X> { uses Inspection<X>; }
class Record<U> { uses Nested<U>; }
"#;
    let (_, analysis) = analyze(source);
    let conditions = effective_conditions(&analysis);
    assert_eq!(conditions.len(), 1);
    let condition = conditions[0].1;
    let receiver = ResolvedType::Class(ClassType::new(
        "Record",
        vec![ResolvedType::TypeParameter("U".into())],
    ));
    assert_eq!(
        condition.obligations[0].interface.arguments,
        vec![ResolvedType::List(Box::new(receiver.clone()))]
    );
    assert_eq!(condition.obligations[1].interface.arguments, vec![receiver]);
    assert!(
        matches!(&condition.obligations[1].subject_type, ResolvedType::TypeParameter(name) if name.starts_with("U#trait"))
    );
}

#[test]
fn trait_aliases_keep_distinct_effective_identities_and_the_same_authored_predicate() {
    let source = r#"
interface Ready {}
trait Inspection<T> {
    function inspect(T $value): int where (T implements Ready) { return 1; }
}
class Record<U> { uses Inspection<U> { Inspection<U>::inspect as other; } }
"#;
    let (_, analysis) = analyze(source);
    let conditions = effective_conditions(&analysis);
    assert_eq!(conditions.len(), 2);
    assert_ne!(conditions[0].0.expansion, conditions[1].0.expansion);
    assert_eq!(conditions[0].1.obligations, conditions[1].1.obligations);
    for (span, condition) in conditions {
        assert_eq!(condition.clause.expansion, span.expansion);
        assert_eq!(analysis.info.contracts.callable_conditions[&span], span);
    }
}

#[test]
fn nongeneric_traits_keep_method_parameters_distinct_in_generic_and_plain_classes() {
    let source = r#"
interface Ready {}
trait Inspection {
    static function inspect<U>(U $value): int where (U implements Ready) { return 1; }
}
class Record<U> { uses Inspection; }
class Plain { uses Inspection; }
"#;
    let (_, analysis) = analyze(source);
    let conditions = effective_conditions(&analysis);
    assert_eq!(conditions.len(), 2);
    assert_eq!(conditions[0].1.parameters.len(), 2);
    assert_eq!(conditions[1].1.parameters.len(), 1);
    for (declaration, condition) in conditions {
        let method = condition.parameters.last().unwrap();
        assert!(method.name.starts_with("U#trait"));
        assert_eq!(method.declaration.expansion, declaration.expansion);
        assert_eq!(
            condition.obligations[0].subject_type,
            ResolvedType::TypeParameter(method.name.clone())
        );
        assert_eq!(condition.obligations[0].subject.name, "U");
        assert!(condition
            .parameters
            .iter()
            .take(condition.parameters.len() - 1)
            .all(|owner| owner.name != method.name));
    }
}

#[test]
fn method_parameters_still_cannot_shadow_their_authored_trait_owner() {
    let source = r#"
interface Ready {}
trait Inspection<T> {
    function inspect<T>(T $value): int where (T implements Ready) { return 1; }
}
class Record<U> { uses Inspection<U>; }
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("authored-shadow.doria", source).unwrap();
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|finding| finding.code == "E0530"),
        "{:?}",
        analysis.diagnostics
    );
    assert!(effective_conditions(&analysis).is_empty());
    assert!(doriac::check_source("authored-shadow.doria", source).is_err());
}

#[test]
fn diamond_composition_has_one_condition_for_the_canonical_member() {
    let source = r#"
interface Ready {}
trait Root<T> { function inspect(T $value): int where (T implements Ready) { return 1; } }
trait Left<U> { uses Root<U>; }
trait Right<V> { uses Root<V>; }
class Record<W> { uses Left<W>, Right<W>; }
"#;
    let (_, analysis) = analyze(source);
    let conditions = effective_conditions(&analysis);
    assert_eq!(conditions.len(), 1);
    assert_eq!(
        conditions[0].1.obligations[0].subject_type,
        ResolvedType::TypeParameter("W".into())
    );
}

#[test]
fn composed_property_accessors_share_one_substituted_condition() {
    let source = r#"
interface Ready {}
trait Inspection<T> {
    writable int $state where (T implements Ready) { get => 1; set (int $next) {} }
}
class Record<U> { uses Inspection<U>; }
"#;
    let (_, analysis) = analyze(source);
    let conditions = effective_conditions(&analysis);
    assert_eq!(conditions.len(), 1);
    let (property, condition) = conditions[0];
    let hooks = analysis
        .info
        .contracts
        .callable_conditions
        .iter()
        .filter(|(callable, owner)| {
            callable.expansion != ExpansionId::default() && **owner == property
        })
        .collect::<Vec<_>>();
    assert_eq!(hooks.len(), 2);
    assert!(hooks
        .iter()
        .all(|(callable, _)| callable.expansion == property.expansion));
    assert_eq!(
        condition.obligations[0].subject_type,
        ResolvedType::TypeParameter("U".into())
    );
}

#[test]
fn a_false_guard_cannot_select_between_duplicate_trait_member_bodies() {
    let source = r#"
interface Ready {}
class No {}
trait First<T> { function inspect(): int where (T implements Ready) { return 1; } }
trait Second<T> { function inspect(): int where (T implements Ready) { return 2; } }
class Record { uses First<No>, Second<No>; }
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("duplicates.doria", source).unwrap();
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|finding| finding.code == "E0757"),
        "{:?}",
        analysis.diagnostics
    );
    assert!(doriac::check_source("duplicates.doria", source).is_err());
    assert!(analysis
        .info
        .contracts
        .conformances
        .iter()
        .all(|fact| fact.origin.source != doriac::source::SourceId::default()));
}

#[test]
fn an_unresolved_authored_guard_cannot_become_an_unconditional_expanded_callable() {
    let source = r#"
interface Ready {}
trait Inspection<T> { function inspect(): int where (U implements Ready) { return 1; } }
class Record<V> { uses Inspection<V>; }
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("unresolved.doria", source).unwrap();
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|finding| finding.code == "E0778"),
        "{:?}",
        analysis.diagnostics
    );
    assert!(
        analysis
            .diagnostics
            .iter()
            .all(|finding| matches!(finding.code, "E0778")),
        "{:?}",
        analysis.diagnostics
    );
    let unresolved = analysis
        .info
        .contracts
        .callable_conditions
        .iter()
        .filter(|(callable, _)| callable.expansion != ExpansionId::default())
        .collect::<Vec<_>>();
    assert_eq!(unresolved.len(), 1);
    assert_eq!(unresolved[0].0, unresolved[0].1);
    assert!(!analysis
        .info
        .contracts
        .conditions
        .contains_key(unresolved[0].1));
    assert!(doriac::check_source("unresolved.doria", source).is_err());
}
