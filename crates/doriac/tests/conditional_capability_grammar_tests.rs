use doriac::ast::transform::{Transform, Transformable};
use doriac::ast::{ClassMember, Item};
use doriac::diagnostics::DiagnosticKind;
use doriac::source::Span;

const FIXTURE: &str = include_str!("fixtures/conditional_capabilities.doria");

fn text(source: &str, span: Span) -> &str {
    &source[span.start..span.end]
}

#[test]
fn local_clauses_preserve_conformance_and_member_boundaries() {
    let program = doriac::parse_source("conditions.doria", FIXTURE).unwrap();
    let Item::Class(class) = &program.items[2] else {
        panic!("class");
    };
    assert_eq!(class.implements.len(), 2);
    for (entry, name) in class.implements.iter().zip(["Ready", "Pending"]) {
        assert_eq!(entry.ty.name, name);
        let condition = entry.condition.as_ref().unwrap();
        assert_eq!(text(FIXTURE, condition.keyword_span), "where");
        assert_eq!(text(FIXTURE, condition.delimiters.open_span), "(");
        assert_eq!(text(FIXTURE, condition.delimiters.close_span), ")");
        assert_eq!(condition.bounds[0].subject.text, "T");
        assert_eq!(text(FIXTURE, condition.bounds[0].subject.span), "T");
        assert_eq!(
            text(FIXTURE, condition.bounds[0].implements_span),
            "implements"
        );
        assert_eq!(text(FIXTURE, condition.bounds[0].type_spans[0]), name);
    }
    assert_eq!(text(FIXTURE, class.syntax.inheritance_comma_spans[0]), ",");
    let guarded = class
        .members
        .iter()
        .filter_map(|member| match member {
            ClassMember::Method(method) => method.availability.as_ref(),
            ClassMember::Property(property) => property.availability.as_ref(),
            _ => None,
        })
        .count();
    assert_eq!(guarded, 4);
}

#[test]
fn conjunctions_delimit_multiple_subjects_nested_types_and_effects() {
    let source = r#"class Adapter<T, U> implements First where (T implements App\Ready<List<U>>, Other, U implements Third), Last {
        function inspect<V>(V $value): int throws Failure where (T implements First, V implements App\Ready<List<U>>, Other) { return 1; }
    }"#;
    let program = doriac::parse_source("conjunction.doria", source).unwrap();
    let Item::Class(class) = &program.items[0] else {
        panic!("class");
    };
    assert!(class.implements[1].condition.is_none());
    let condition = class.implements[0].condition.as_ref().unwrap();
    assert_eq!(condition.bounds.len(), 2);
    assert_eq!(condition.delimiters.comma_spans.len(), 2);
    assert_eq!(condition.bounds[0].interfaces.len(), 2);
    assert_eq!(condition.bounds[1].subject.text, "U");
    assert_eq!(
        text(source, condition.bounds[0].type_spans[0]),
        "App\\Ready<List<U>>"
    );
    let ClassMember::Method(method) = &class.members[0] else {
        panic!("method");
    };
    assert_eq!(
        method.throws.as_ref().unwrap().entries[0].ty.name,
        "Failure"
    );
    assert_eq!(
        method.availability.as_ref().unwrap().bounds[1].subject.text,
        "V"
    );
}

#[test]
fn accepted_clauses_publish_checked_facts_without_unconditional_witnesses() {
    let (_, analysis) = doriac::analyze_source_for_ide("conditions.doria", FIXTURE).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert!(!analysis.info.callable_signatures.is_empty());
    assert_eq!(analysis.info.contracts.conditions.len(), 8);
    assert!(analysis
        .info
        .contracts
        .conformances
        .iter()
        .all(|fact| fact.origin.source != doriac::source::SourceId::default()));
    doriac::compile_source_to_php("conditions.doria", FIXTURE).unwrap();
    let ast = doriac::parse_source("conditions.doria", FIXTURE).unwrap();
    assert!(
        doriac::lowering::lower_program_with_semantics(&ast, Default::default())
            .unwrap_err()
            .iter()
            .all(|error| error.kind == DiagnosticKind::InternalCompiler)
    );
}

#[test]
fn guarded_getters_still_reject_potentially_blocking_interface_calls() {
    let source = r#"
interface Ready { function ready(): int; }
class Adapter<T> {
    function __construct(take T $source) {}
    int $state where (T implements Ready) { get { return $this->source->ready(); } }
}
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("blocking-guard.doria", source).unwrap();
    assert_eq!(analysis.diagnostics.len(), 1, "{:?}", analysis.diagnostics);
    assert!(analysis
        .diagnostics
        .iter()
        .any(|finding| finding.code == "E0770"));
}

#[test]
fn conditional_storage_lifecycle_and_fixed_requirements_are_language_errors() {
    for source in [
        "class C<T> { int $stored where (T implements Ready); }",
        "class C<T> { int $stored where (T implements Ready) = 1; }",
        "class C<T> { int $stored where (T implements Ready) { get { return $this->stored; } } }",
        "class C<T> { int $stored where (T implements Ready) = 1 { get { return 1; } } }",
        "class C<T> { function __construct() where (T implements Ready) {} }",
        "class C<T> { function __destruct(): void where (T implements Ready) {} }",
        "interface I<T> { function inspect(): int where (T implements Ready); }",
        "interface I<T> { int $state where (T implements Ready) { get; } }",
        "function inspect<T>(): int where (T implements Ready) { return 1; }",
    ] {
        doriac::parse_source("placement.doria", source).expect(source);
        let errors = doriac::check_source("placement.doria", source).unwrap_err();
        assert!(
            errors
                .iter()
                .all(|error| error.code == "E0777" && error.kind == DiagnosticKind::Language),
            "{source}: {errors:?}"
        );
    }
}

#[test]
fn where_remains_contextual_and_match_guards_keep_if() {
    let source = "function where(int $where): int { return $where; } class C { int $where = 1; } class D { function where(): int { return 2; } }";
    doriac::check_source("contextual.doria", source).unwrap();
    assert!(doriac::parse_source(
        "match.doria",
        "function main(): int { return match (1) { 1 where true => 1, default => 0, }; }"
    )
    .is_err());
}

#[test]
fn condition_interface_names_use_shared_namespace_and_alias_resolution() {
    let source = "namespace App; use App\\Ready as R; interface Ready {} class C<T> implements R where (T implements R) {}";
    let (_, analysis) = doriac::analyze_source_for_ide("aliases.doria", source).unwrap();
    assert!(analysis.diagnostics.is_empty());
    assert!(analysis
        .info
        .global_symbols
        .references
        .iter()
        .any(
            |reference| reference.role == doriac::names::GlobalReferenceRole::Type
                && reference.import_alias.as_deref() == Some("R")
                && text(source, reference.source_span) == "R"
        ));
}

#[test]
fn nested_generic_closes_preserve_commas_in_the_enclosing_list() {
    let source = "class C<T> implements Ready<List<T>>, Other { function inspect(Dictionary<int, List<T>> $value, int $count): void {} }";
    let program = doriac::parse_source("nested-types.doria", source).unwrap();
    let Item::Class(class) = &program.items[0] else {
        panic!("class");
    };
    assert_eq!(class.implements.len(), 2);
    assert_eq!(class.implements[0].ty.arguments.len(), 1);
    assert_eq!(class.implements[1].ty.name, "Other");
    let ClassMember::Method(method) = &class.members[0] else {
        panic!("method");
    };
    assert_eq!(method.params.len(), 2);
    assert_eq!(method.params[0].ty.arguments.len(), 2);
    assert_eq!(method.params[1].name, "count");
}

#[test]
fn malformed_clauses_recover_before_later_declarations() {
    for clause in [
        "where ()",
        "where (T)",
        "where (T implements)",
        "where (T implements Ready,)",
        "where T implements Ready)",
    ] {
        let broken = format!("class C<T> implements Ready {clause} {{}}");
        let source = format!("{broken}\nclass Later {{ function okay(): void {{}} }}");
        let errors = doriac::parse_source("recovery.doria", &source).unwrap_err();
        assert!(errors.len() <= 4, "{clause}: {errors:?}");
        assert!(
            errors.iter().all(|error| error.span.start <= broken.len()),
            "{clause}: {errors:?}"
        );
    }
}

#[test]
fn syntax_transforms_preserve_guard_types_and_all_guard_spans() {
    struct Shift;
    impl Transform for Shift {
        fn span(&mut self, span: &mut Span) {
            *span = span.at(span.start + 100, span.end + 100);
        }
        fn type_ref(&mut self, ty: &mut doriac::types::TypeRef) {
            if ty.name == "Ready" {
                ty.name = "Renamed".to_string();
            }
            ty.walk(self);
        }
    }
    let mut program = doriac::parse_source("transform.doria", FIXTURE).unwrap();
    let Item::Class(class) = &mut program.items[2] else {
        panic!("class");
    };
    let original = class.implements[0].condition.clone().unwrap();
    class.implements[0].transform(&mut Shift);
    let shifted = class.implements[0].condition.as_ref().unwrap();
    assert_eq!(shifted.span.start, original.span.start + 100);
    assert_eq!(
        shifted.bounds[0].subject.span.start,
        original.bounds[0].subject.span.start + 100
    );
    assert_eq!(
        shifted.bounds[0].type_spans[0].start,
        original.bounds[0].type_spans[0].start + 100
    );
    assert_eq!(
        shifted.delimiters.close_span.end,
        original.delimiters.close_span.end + 100
    );
    assert_eq!(shifted.bounds[0].interfaces[0].name, "Renamed");
    let ClassMember::Method(method) = &mut class.members[2] else {
        panic!("method");
    };
    let before = method.availability.clone().unwrap();
    method.transform(&mut Shift);
    assert_eq!(
        method.availability.as_ref().unwrap().span.start,
        before.span.start + 100
    );
}

#[test]
fn checked_conditions_preserve_lexical_parameter_and_nominal_type_identities() {
    use doriac::types::{InterfaceType, ResolvedType};
    let source = r#"namespace App;
        use App\Ready as R;
        interface Ready<T> {}
        class Adapter<T> implements R<T> where (T implements R<T>) {
            static function inspect<U>(): int where (T implements R<List<U>>, U implements R<T>) { return 1; }
        }
        trait Inspection<T> {
            int $state where (T implements R<T>) { get { return 1; } }
        }"#;
    let (program, analysis) = doriac::analyze_source_for_ide("checked.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let Item::Class(class) = &program.items[1] else {
        panic!("class")
    };
    let ClassMember::Method(method) = &class.members[0] else {
        panic!("method")
    };
    let Item::Trait(trait_decl) = &program.items[2] else {
        panic!("trait")
    };
    let ClassMember::Property(property) = &trait_decl.members[0] else {
        panic!("property")
    };
    let conditions = &analysis.info.contracts.conditions;
    assert_eq!(conditions.len(), 3);
    let conformance = &conditions[&class.syntax.inheritance_type_spans[0]];
    assert_eq!(
        conformance.parameters[0].declaration,
        class.type_params[0].span
    );
    assert_eq!(
        conformance.obligations[0].subject,
        conformance.parameters[0]
    );
    assert_eq!(
        conformance.obligations[0].interface,
        InterfaceType::new(
            "App\\Ready",
            vec![ResolvedType::TypeParameter("T".to_string())]
        )
    );
    assert_eq!(
        text(source, conformance.obligations[0].interface_span),
        "R<T>"
    );
    let inspect = &conditions[&method.span];
    assert_eq!(inspect.clause, method.availability.as_ref().unwrap().span);
    assert_eq!(inspect.parameters.len(), 2);
    assert_eq!(
        inspect.parameters[1].declaration,
        method.type_params[0].span
    );
    assert_eq!(
        inspect.obligations[0].interface.arguments,
        vec![ResolvedType::List(Box::new(ResolvedType::TypeParameter(
            "U".to_string()
        )))]
    );
    assert_eq!(inspect.obligations[1].subject, inspect.parameters[1]);
    let state = &conditions[&property.span];
    assert_eq!(state.parameters[0].name, conformance.parameters[0].name);
    assert_ne!(
        state.parameters[0].declaration,
        conformance.parameters[0].declaration
    );
    assert_eq!(
        state.parameters[0].declaration,
        trait_decl.type_params[0].span
    );
    assert!(analysis
        .info
        .contracts
        .conformances
        .iter()
        .all(|fact| fact.origin.source != doriac::source::SourceId::default()));
    assert!(analysis.info.method_call_targets.is_empty());
}

#[test]
fn invalid_condition_subjects_are_source_identified_and_publish_no_predicate() {
    for subject in ["Missing", "Ready", "Concrete"] {
        let source = format!("interface Ready {{}} class Concrete {{}} class C<T> {{ function inspect(): int where ({subject} implements Ready) {{ return 1; }} }}");
        let (_, analysis) = doriac::analyze_source_for_ide("subject.doria", &source).unwrap();
        let error = analysis
            .diagnostics
            .iter()
            .find(|error| error.code == "E0778")
            .expect(&source);
        assert_eq!(error.kind, DiagnosticKind::Language);
        assert_eq!(text(&source, error.span), subject);
        assert!(analysis.info.contracts.conditions.is_empty());
    }
}

#[test]
fn condition_interface_arguments_keep_the_enclosing_self_context() {
    use doriac::types::{ClassType, ResolvedType};
    let source = "interface Ready<T> {} class C<T> implements Ready<self> where (T implements Ready<self>) { function inspect(): void where (T implements Ready<self>) {} } trait Inspection<T> { int $state where (T implements Ready<self>) { get { return 1; } } }";
    let (program, analysis) = doriac::analyze_source_for_ide("self.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert_eq!(analysis.info.contracts.conditions.len(), 3);
    let Item::Class(class) = &program.items[1] else {
        panic!("class")
    };
    let ClassMember::Method(method) = &class.members[0] else {
        panic!("method")
    };
    for declaration in [class.syntax.inheritance_type_spans[0], method.span] {
        assert_eq!(
            analysis.info.contracts.conditions[&declaration].obligations[0]
                .interface
                .arguments,
            vec![ResolvedType::Class(ClassType::new(
                "C",
                vec![ResolvedType::TypeParameter("T".to_string())]
            ))]
        );
    }
    let Item::Trait(trait_decl) = &program.items[2] else {
        panic!("trait")
    };
    let ClassMember::Property(property) = &trait_decl.members[0] else {
        panic!("property")
    };
    assert_eq!(
        analysis.info.contracts.conditions[&property.span].obligations[0]
            .interface
            .arguments,
        vec![ResolvedType::TraitSelf("Inspection".to_string())]
    );
    let ordinary = "interface Ready<T> {} class C<T> implements Ready<self> {} function accept<T implements Ready<C<int>>>(T $value): void {} function main(): void { let $source = new C<int>(); accept($source); }";
    doriac::check_source("ordinary-self.doria", ordinary).unwrap();
    let (_, analysis) = doriac::analyze_source_for_ide("ordinary-self.doria", ordinary).unwrap();
    assert!(analysis
        .info
        .contracts
        .conformances
        .iter()
        .any(
            |fact| fact.status == doriac::semantics::contracts::ConformanceStatus::Checked
                && fact.interface.name == "Ready"
                && fact.interface.arguments
                    == vec![ResolvedType::Class(ClassType::new(
                        "C",
                        vec![ResolvedType::Integer(doriac::types::IntegerType::Int64)]
                    ))]
        ));
}

#[test]
fn inactive_conditions_cannot_hide_invalid_interface_types() {
    for (interface, code) in [
        ("Concrete", "E0750"),
        ("?Ready<T>", "E0750"),
        ("Ready", "E0402"),
        ("Ready<T, int>", "E0402"),
        ("Ready<1>", "E0536"),
    ] {
        let source = format!("interface Ready<T> {{}} class Concrete {{}} class C<T> {{ function inspect(): int where (T implements {interface}) {{ return 1; }} }}");
        let (_, analysis) = doriac::analyze_source_for_ide("interface.doria", &source).unwrap();
        assert!(
            analysis.diagnostics.iter().any(|error| error.code == code),
            "{source}: {:?}",
            analysis.diagnostics
        );
        assert!(analysis.info.contracts.conditions.is_empty(), "{source}");
    }
}

#[test]
fn conditional_conformance_and_member_shapes_use_existing_declaration_checks() {
    for (declaration, code) in [
        ("class C<T> implements Ready<int, int> where (T implements Ready<T>) {}", "E0402"),
        ("class C<T> { function inspect<U>(): void where (T implements Ready<U>) {} function inspect(): void {} }", "E0481"),
        ("trait C<T> { function inspect<U>(): void where (T implements Ready<U>) {} function inspect(): void {} }", "E0309"),
        ("class C<T> { function inspect<T>(): void where (T implements Ready<T>) {} }", "E0530"),
        ("trait C<T> { function inspect<T>(): void where (T implements Ready<T>) {} }", "E0530"),
    ] {
        let source = format!("interface Ready<T> {{}} {declaration}");
        let (_, analysis) = doriac::analyze_source_for_ide("shape.doria", &source).unwrap();
        assert!(analysis.diagnostics.iter().any(|error| error.code == code), "{source}: {:?}", analysis.diagnostics);
        assert!(analysis.info.contracts.conformances.iter().all(|fact|
            fact.origin.source != doriac::source::SourceId::default()));
        if code == "E0530" {
            assert!(analysis.info.contracts.conditions.is_empty());
        }
    }
}

#[test]
fn concrete_generic_constraints_use_conditional_proofs_independently_of_source_order() {
    let declarations = "interface Ready {} class Full implements Ready {} class Plain {} class Adapter<T> implements Ready where (T implements Ready) {} class Required<T implements Ready> {}";
    for source_type in ["Full", "Plain"] {
        let consumer = format!("class Consumer {{ function inspect(Required<Adapter<{source_type}>> $value): void {{}} }}");
        for source in [
            format!("{declarations} {consumer}"),
            format!("{consumer} {declarations}"),
        ] {
            let (_, analysis) =
                doriac::analyze_source_for_ide("constraint-proof.doria", &source).unwrap();
            if source_type == "Full" {
                assert!(
                    analysis.diagnostics.is_empty(),
                    "{:?}",
                    analysis.diagnostics
                );
            } else {
                assert!(
                    analysis
                        .diagnostics
                        .iter()
                        .any(|error| error.code == "E0535"
                            && error.message.contains("Adapter<Plain>")),
                    "{:?}",
                    analysis.diagnostics
                );
            }
            assert_eq!(analysis.info.contracts.conditions.len(), 1);
        }
    }
}

#[test]
fn primitive_constraint_evidence_cannot_satisfy_an_interface_valued_return_contract() {
    let source = "interface Boxed { function value(): Equatable<int>; } class Invalid implements Boxed { function value(): int { return 1; } } function main(): void {}";
    let errors = doriac::check_source("primitive-interface-result.doria", source).unwrap_err();
    assert!(
        errors.iter().any(|error| error.code == "E0755"),
        "{errors:?}"
    );
}

fn assert_availability_result(source: &str, expected: Option<&str>) {
    let (_, analysis) = doriac::analyze_source_for_ide("availability.doria", source).unwrap();
    match expected {
        Some(code) => assert!(
            analysis.diagnostics.iter().any(|error| error.code == code),
            "{source}: {:?}",
            analysis.diagnostics
        ),
        None => assert!(
            analysis.diagnostics.is_empty(),
            "{source}: {:?}",
            analysis.diagnostics
        ),
    }
    for error in analysis
        .diagnostics
        .iter()
        .filter(|error| matches!(error.code, "E0755" | "E0729"))
    {
        assert!(text(source, error.span).starts_with("where ("), "{error:?}");
    }
}

#[test]
fn conformance_assumptions_must_imply_implementation_availability() {
    let interfaces = "interface Ready { function read(): int; } interface Pending extends Ready {}";
    for (parameters, conformance, implementation, expected) in [
        (
            "T",
            "where (T implements Ready)",
            "where (T implements Ready)",
            None,
        ),
        (
            "T",
            "where (T implements Pending)",
            "where (T implements Ready)",
            None,
        ),
        ("T", "where (T implements Ready)", "", None),
        ("T implements Ready", "", "where (T implements Ready)", None),
        (
            "T",
            "where (T implements Ready)",
            "where (T implements Pending)",
            Some("E0755"),
        ),
        ("T", "", "where (T implements Ready)", Some("E0755")),
    ] {
        assert_availability_result(&format!("{interfaces} class C<{parameters}> implements Ready {conformance} {{ function read(): int {implementation} {{ return 1; }} }}"), expected);
    }
}

#[test]
fn overrides_may_widen_availability_but_cannot_strengthen_it() {
    let interfaces = "interface Ready {} interface Pending extends Ready {}";
    for (parameters, parent, child, expected) in [
        ("U", "where (T implements Ready)", "", None),
        (
            "U",
            "where (T implements Pending)",
            "where (U implements Ready)",
            None,
        ),
        (
            "U",
            "where (T implements Ready)",
            "where (U implements Ready)",
            None,
        ),
        (
            "U implements Pending",
            "where (T implements Ready)",
            "where (U implements Pending)",
            None,
        ),
        (
            "U",
            "where (T implements Ready)",
            "where (U implements Pending)",
            Some("E0729"),
        ),
        ("U", "", "where (U implements Ready)", Some("E0729")),
    ] {
        assert_availability_result(&format!("{interfaces} open class Parent<T> {{ open function read(): int {parent} {{ return 1; }} }} class Child<{parameters}> extends Parent<U> {{ override function read(): int {child} {{ return 2; }} }}"), expected);
    }
}

#[test]
fn computed_property_guards_cover_both_accessor_contracts() {
    let interfaces = "interface Ready {} interface Pending extends Ready {} interface Value { writable int $value { get; set (int $next); } }";
    for (premise, conclusion, expected) in [
        ("Ready", "Ready", None),
        ("Pending", "Ready", None),
        ("Ready", "Pending", Some("E0755")),
    ] {
        let source = format!("{interfaces} class C<T> implements Value where (T implements {premise}) {{ writable int $value where (T implements {conclusion}) {{ get => 1; set (int $next) {{}} }} }}");
        assert_availability_result(&source, expected);
        let (program, analysis) =
            doriac::analyze_source_for_ide("availability.doria", &source).unwrap();
        let property = program
            .items
            .iter()
            .find_map(|item| match item {
                Item::Class(class) => class.members.iter().find_map(|member| match member {
                    ClassMember::Property(property) => Some(property),
                    _ => None,
                }),
                _ => None,
            })
            .unwrap();
        assert_eq!(analysis.info.contracts.callable_conditions.len(), 2);
        for hook in &property.hooks {
            assert_eq!(
                analysis.info.contracts.callable_conditions[&hook.span],
                property.span
            );
        }
    }
    for (parent, child, expected) in [
        ("Pending", "Ready", None),
        ("Ready", "Pending", Some("E0729")),
    ] {
        assert_availability_result(&format!("{interfaces} open class Parent<T> {{ open writable int $value where (T implements {parent}) {{ get => 1; set (int $next) {{}} }} }} class Child<U> extends Parent<U> {{ override writable int $value where (U implements {child}) {{ get => 2; set (int $next) {{}} }} }}"), expected);
    }
}

#[test]
fn generic_method_guard_proofs_keep_method_and_owner_parameters_distinct() {
    for (requirement, owner, implementation, expected) in [
        ("T implements Ready", "T", "V implements Ready", None),
        ("T", "T implements Ready", "V", Some("E0755")),
    ] {
        assert_availability_result(&format!("interface Ready {{}} interface Work {{ function inspect<{requirement}>(T $value): int; }} class C<{owner}> implements Work {{ function inspect<{implementation}>(V $value): int where (V implements Ready) {{ return 1; }} }}"), expected);
    }
    for (parent, child, expected) in [("T", "U", None), ("U", "U", Some("E0729"))] {
        assert_availability_result(&format!("interface Ready {{}} open class Parent<T> {{ open function inspect<U>(U $value): int where ({parent} implements Ready) {{ return 1; }} }} class Child<U> extends Parent<U> {{ override function inspect<W>(W $value): int where ({child} implements Ready) {{ return 2; }} }}"), expected);
    }
}

#[test]
fn inherited_conformance_conditions_use_their_original_owner_bindings() {
    let source = "interface Ready { function read(): int; } interface Pending extends Ready {} open class Parent<T> implements Ready where (T implements Ready) { open function read(): int where (T implements Ready) { return 1; } } class Child<U> extends Parent<U> { override function read(): int where (U implements Pending) { return 2; } }";
    assert_availability_result(source, Some("E0755"));
    assert_availability_result(
        &source.replace("where (U implements Pending)", "where (U implements Ready)"),
        None,
    );
}

#[test]
fn borrowed_computed_getter_keeps_its_property_condition_identity() {
    let source = "interface Ready {} class Item {} interface View { Item $item { borrowed get; } } class C<T> implements View where (T implements Ready) { Item $stored; function __construct(parameter take Item $stored) { $this->stored = $stored; } Item $item where (T implements Ready) { borrowed get { return $this->stored; } } }";
    assert_availability_result(source, None);
    let (program, analysis) =
        doriac::analyze_source_for_ide("borrowed-condition.doria", source).unwrap();
    let property = program
        .items
        .iter()
        .find_map(|item| match item {
            Item::Class(class) if class.name == "C" => {
                class.members.iter().find_map(|member| match member {
                    ClassMember::Property(property) if property.name == "item" => Some(property),
                    _ => None,
                })
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(
        analysis.info.contracts.callable_conditions[&property.hooks[0].span],
        property.span
    );
}
