fn run(source: &str) -> String {
    let mir = doriac::lower_source_to_mir("traits.doria", source).expect("composed program lowers");
    String::from_utf8(
        doriac::mir_interpreter::interpret(&mir)
            .expect("composed program executes")
            .stdout,
    )
    .unwrap()
}

#[test]
fn durable_trait_fixtures_preserve_interpreter_results() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for path in include_str!("fixtures/native_parity_examples.txt")
        .lines()
        .filter(|path| path.starts_with("examples/native/main_stage35_trait_"))
    {
        let source = std::fs::read_to_string(root.join(path)).unwrap();
        let mir = doriac::lower_source_to_mir(path, source)
            .unwrap_or_else(|diagnostics| panic!("{path}: {diagnostics:#?}"));
        let actual = doriac::mir_interpreter::interpret(&mir).unwrap();
        let stem = std::path::Path::new(path).file_stem().unwrap();
        let fixtures = root
            .join("crates/doriac/tests/fixtures/native_io")
            .join(stem);
        assert_eq!(
            actual.stdout,
            std::fs::read(fixtures.join("expected_stdout")).unwrap(),
            "{path}"
        );
        assert_eq!(
            actual.stderr,
            std::fs::read(fixtures.join("expected_stderr")).unwrap(),
            "{path}"
        );
        assert_eq!(
            actual.exit_status,
            std::fs::read_to_string(fixtures.join("expected_status"))
                .unwrap()
                .trim()
                .parse::<i32>()
                .unwrap(),
            "{path}"
        );
    }
}

#[test]
fn checked_method_receivers_have_statement_owners() {
    use doriac::mir::{ClassExpression, Rvalue, Statement, Terminator};
    let program = doriac::lower_source_to_mir(
        "receivers.doria",
        include_str!("../../../examples/native/main_stage35_trait_generic.doria"),
    )
    .unwrap();
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    let mut receivers = 0;
    for block in &main.blocks {
        if let Terminator::CheckedCall { args, .. } = &block.terminator {
            if let Some(Rvalue::Class(receiver)) = args.first() {
                let ClassExpression::Local {
                    local,
                    transfer: false,
                    ..
                } = receiver
                else {
                    panic!("checked receiver must borrow a tracked owner: {receiver:?}");
                };
                assert!(main.locals[local.0].owned);
                assert!(main.blocks.iter().flat_map(|block| &block.statements).any(|statement| {
                    matches!(statement, Statement::DropClass { local: dropped, .. } if dropped == local)
                }));
                receivers += 1;
            }
        }
    }
    assert_eq!(receivers, 2);
}

#[test]
fn selected_and_aliased_methods_satisfy_requirements_and_interfaces() {
    assert_eq!(
        run(r#"
interface Renderable { function render(): string; }
trait HasTitle {
    string $title = "Doria";
    function formatPrefix(): string;
    function render(): string { return $this->formatPrefix() . $this->title; }
}
trait CompactFormatting { function format(): string { return "compact"; } }
trait VerboseFormatting { function format(): string { return "verbose"; } }
class Report implements Renderable {
    uses HasTitle, CompactFormatting, VerboseFormatting {
        CompactFormatting::format insteadof VerboseFormatting;
        VerboseFormatting::format as formatVerbose;
    }
    function formatPrefix(): string { return "Title: "; }
}
function renderReport(Renderable $report): string { return $report->render(); }
function main(): void {
    let $report = new Report();
    echo renderReport($report) . "\n";
    echo $report->format() . "\n";
    echo $report->formatVerbose() . "\n";
}
"#),
        "Title: Doria\ncompact\nverbose\n"
    );
}

#[test]
fn unsatisfied_or_incompatible_requirements_are_not_runtime_members() {
    for implementation in ["", "function value(): string { return \"wrong\"; }"] {
        let source = format!("trait Required {{ function value(): int; }} class Box {{ uses Required; {implementation} }}");
        let (_, analysis) = doriac::analyze_source_for_ide("traits.doria", &source).unwrap();
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0757"),
            "{:?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn two_composers_and_aliases_keep_independent_expression_and_closure_identities() {
    assert_eq!(
        run(r#"
trait Value<T> {
    function identity(T $value): T { let $f = fn(T $item) => $item; return $f($value); }
}
class Numbers { uses Value<int>; }
class Flags { uses Value<bool>; }
function main(): void {
    echo (new Numbers())->identity(42);
    echo (new Flags())->identity(true);
}
"#),
        "42true"
    );
}

#[test]
fn composed_parameters_defaults_and_lexical_owners_have_effective_identities() {
    use doriac::const_eval::{ConstValue, ParameterDefaultKey};
    use doriac::symbols::{BindingKind, LexicalOwner};
    let source = r#"
trait Greeting {
    function greet(string $message = "hi"): string { return $message; }
}
class First { uses Greeting { Greeting::greet as again; } }
class Second { uses Greeting; }
function main(): void { echo (new First())->again(); echo (new Second())->greet(); }
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("identity.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let origins = &analysis.info.composition.origins;
    assert_eq!(origins.len(), 3);
    let mut bindings = std::collections::HashSet::new();
    for origin in origins {
        let parameters = analysis
            .info
            .binding_resolution
            .declarations_by_id
            .values()
            .filter(|binding| {
                binding.owner == LexicalOwner::Callable(origin.declaration)
                    && binding.kind == BindingKind::MethodParameter
            })
            .collect::<Vec<_>>();
        assert_eq!(parameters.len(), 1);
        assert!(bindings.insert(parameters[0].id));
        assert!(origin.declaration.contains(parameters[0].span.unwrap()));
        assert_eq!(
            analysis.info.parameter_defaults.get(&ParameterDefaultKey {
                function: origin.declaration,
                parameter_index: 0,
            }),
            Some(&ConstValue::String("hi".into()))
        );
    }
    assert_eq!(run(source), "hihi");
}

#[test]
fn nested_generic_traits_can_use_their_composed_members() {
    assert_eq!(
        run(r#"
trait Identity<T> { function identity(take T $value): T { return $value; } }
trait Wrapper<U> {
    uses Identity<List<U>>;
    function count(take List<U> $items): int { return $this->identity($items)->count; }
}
class Numbers { uses Wrapper<int>; }
function main(): void { echo (new Numbers())->count([1, 2]); }
"#),
        "2"
    );
}

#[test]
fn composer_self_constants_and_static_cells_are_independent() {
    assert_eq!(
        run(r#"
trait Counter {
    static writable int $count = 0;
    const int STEP = 2;
    static function advance(): int { self::count += self::STEP; return self::count; }
    function make(): self { return new self(); }
}
class Left { uses Counter; }
class Right { uses Counter; }
function main(): void { echo Left::advance(); echo Left::advance(); echo Right::advance(); }
"#),
        "242"
    );
}

#[test]
fn class_override_wrapper_can_keep_an_internal_trait_alias() {
    assert_eq!(
        run(r#"
open class Base { open function render(): string { return "base"; } }
trait Rendering { function render(): string { return "trait"; } }
class Report extends Base {
    uses Rendering { Rendering::render as internal renderFromTrait; }
    override function render(): string { return $this->renderFromTrait(); }
}
function main(): void { echo (new Report())->render(); }
"#),
        "trait"
    );
}

#[test]
fn unresolved_conflicts_do_not_publish_checked_conformance() {
    for source in [
        "interface I { function value(): int; } trait A { function value(): int { return 1; } } trait B { function value(): int { return 2; } } class C implements I { uses A, B; function value(): int { return 3; } }",
        "trait A { int $value = 1; } trait B { int $value = 1; } class C { uses A, B; }",
        "trait A { int $value = 1; } class C { uses A; function value(): int { return 1; } }",
    ] {
        let (_, analysis) = doriac::analyze_source_for_ide("conflicts.doria", source).unwrap();
        assert!(analysis.diagnostics.iter().any(|diagnostic| diagnostic.code == "E0757"), "{:?}", analysis.diagnostics);
        assert!(analysis.info.contracts.conformances.iter().filter(|fact| matches!(&fact.implementing_type, doriac::types::ResolvedType::Class(class) if class.name == "C")).all(|fact| fact.status != doriac::semantics::contracts::ConformanceStatus::Checked), "{:?}", analysis.info.contracts.conformances);
    }
}

#[test]
fn invalid_composed_bodies_hierarchy_and_initialization_revoke_conformance() {
    for source in [
        "interface I { function value(): int; } trait T { function value(): int { return \"wrong\"; } } class C implements I { uses T; }",
        "interface I { function value(): int; } open class B { open function value(): int { return 1; } } trait T { function value(): int { return 2; } } class C extends B implements I { uses T; }",
        "interface I { function value(): int; } trait T { int $uninitialized; function value(): int { return 2; } } class C implements I { uses T; }",
    ] {
        let (_, analysis) = doriac::analyze_source_for_ide("invalid-class.doria", source).unwrap();
        assert!(!analysis.diagnostics.is_empty());
        assert!(analysis.info.contracts.conformances.iter().filter(|fact| matches!(&fact.implementing_type, doriac::types::ResolvedType::Class(class) if class.name == "C")).all(|fact| fact.status == doriac::semantics::contracts::ConformanceStatus::Invalid), "{:?}", analysis.info.contracts.conformances);
        assert!(analysis.info.class_member_surfaces.iter().filter(|surface| surface.receiver.name == "C").all(|surface| !surface.valid));
        if let Some(diagnostic) = analysis.diagnostics.iter().find(|diagnostic| diagnostic.code == "E0726") {
            assert!(diagnostic.help.as_deref().unwrap().contains("class-authored override wrapper"));
            assert!(diagnostic.fixes.is_empty());
        }
    }
}

#[test]
fn method_binders_do_not_capture_a_composer_type_parameter() {
    assert_eq!(
        run(r#"
trait Chooses<T> {
    function choose<U>(T $left, U $right): U { return $right; }
}
class Choice<U> { uses Chooses<U>; }
function main(): void { echo (new Choice<int>())->choose(7, true); }
"#),
        "true"
    );
}

#[test]
fn nested_diamond_deduplicates_before_adaptation_and_initializes_once() {
    assert_eq!(
        run(r#"
function initialize(): int { echo "init "; return 3; }
trait Root { int $value = initialize(); function read(): int { return $this->value; } }
trait Left { uses Root; }
trait Right { uses Root; }
trait Diamond { uses Left, Right; }
class Box { uses Diamond { Diamond::read as again; } }
function main(): void { let $box = new Box(); echo $box->read(); echo $box->again(); }
"#),
        "init 33"
    );
}

#[test]
fn composed_members_preserve_ownership_access_effect_and_hierarchy_rejections() {
    for (source, code) in [
        ("trait T { writable int $value = 0; writable function set(): void { $this->value = 1; } } class C { uses T { T::set as mutate; } } function main(): void { let $c = new C(); $c->mutate(); }", "E0203"),
        ("trait T { function value(): int { return 1; } } class C { uses T { T::value as internal hidden; } } function main(): void { echo (new C())->hidden(); }", "E0307"),
        ("trait T { int $value; } class C { uses T; }", "E0500"),
        ("open class B { function value(): int { return 1; } } trait T { function value(): int { return 2; } } class C extends B { uses T; }", "E0727"),
        ("trait T { function value(int $input): int { return $input; } } class C { uses T; function value(string $input): int { return 1; } }", "E0757"),
        ("trait T { function value(): int; } class C { uses T; function value(): string { return \"wrong\"; } }", "E0757"),
        ("trait T<A> { A $value; } class C { uses T<int>, T<string>; }", "E0757"),
        ("trait T { function value(): int { return 1; } } class C { uses T { T::missing as alias; } }", "E0757"),
    ] {
        let (_, analysis) = doriac::analyze_source_for_ide("negative.doria", source).unwrap();
        assert!(analysis.diagnostics.iter().any(|diagnostic| diagnostic.code == code), "expected {code}: {source}\n{:?}", analysis.diagnostics);
        assert!(!analysis.diagnostics.iter().any(|diagnostic| diagnostic.code == "E0493"));
    }
}

#[test]
fn trait_aliases_preserve_checked_effects_and_retained_cursor_loans() {
    let source = r#"
class Failure implements Error { string $message = "failed"; }
trait Checked { function run(): void throws Failure { throw new Failure(); } }
class Worker { uses Checked { Checked::run as alias; } }
function bad(): void { (new Worker())->alias(); }
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("effects.doria", source).unwrap();
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0631"),
        "{:?}",
        analysis.diagnostics
    );
    let source = include_str!("../../../examples/native/main_stage35_trait_iteration.doria");
    let invalid = source
        .replace("let $shelf = new Shelf", "let writable $shelf = new Shelf")
        .replace(
            "while ($cursor->hasCurrent()) {",
            "$shelf = new Shelf([]); while ($cursor->hasCurrent()) {",
        );
    let (_, analysis) = doriac::analyze_source_for_ide("loans.doria", invalid).unwrap();
    assert!(
        !analysis.diagnostics.is_empty(),
        "a cursor must retain its source"
    );
}
