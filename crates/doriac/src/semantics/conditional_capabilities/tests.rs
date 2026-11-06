use super::*;
use crate::semantics::{BorrowSource, ReturnBorrowKind, SemanticInfo};
use crate::types::{ClassType, IntegerType, ResolvedType, TypeKind};

fn with_checker(source: &str, inspect: impl FnOnce(&mut Checker<'_>)) {
    with_checker_diagnostics(source, |checker| {
        assert!(
            checker.diagnostics.is_empty(),
            "{source}: {:?}",
            checker.diagnostics
        );
        inspect(checker);
    });
}

fn with_checker_diagnostics(source: &str, inspect: impl FnOnce(&mut Checker<'_>)) {
    with_declarations(source, |declarations| {
        with_checked_program(declarations, |checker| {
            assert!(checker.nominal_condition_scopes.is_empty());
            inspect(checker);
        });
    });
}

fn with_declarations<R>(source: &str, inspect: impl FnOnce(Checker<'_>) -> R) -> R {
    let context = crate::names::CompilationContext::standalone("composed-bodies.doria");
    let program = crate::parse_source("composed-bodies.doria", source).unwrap();
    let source_file = crate::source::SourceFile::new("composed-bodies.doria", source);
    let prepared = crate::prepare_parsed_source(&source_file, context.clone(), program).unwrap();
    let program = prepared.resolved;
    let mut source_texts = HashMap::from([(crate::source::SourceId(0), source)]);
    let mut contexts = HashMap::new();
    crate::append_compiler_known_semantic_context(
        &program,
        &context,
        &mut source_texts,
        &mut contexts,
    );
    let checker = Checker::new(
        &program,
        crate::const_eval::evaluate_program_with_diagnostics(&program).0,
        &source_texts,
        context,
        contexts,
        prepared.global_symbols,
        prepared.source_semantic_contexts,
        prepared.test_semantics,
    );
    inspect(checker)
}

#[test]
fn checked_execution_preserves_widened_override_dispatch() {
    assert_checked_execution(
        "widened override dispatch",
        r#"
interface Ready {}
class Yes implements Ready {}
interface Value { function value(): int; }
open class Base<T> implements Value where (T implements Ready) {
    open function value(): int where (T implements Ready) { return 1; }
}
class Child<T> extends Base<T> {
    override function value(): int { return 21; }
}
function viaBase(Base<Yes> $source): int { return $source->value(); }
function viaInterface(Value $source): int { return $source->value(); }
function main(): int {
    let $active = new Child<Yes>();
    let $independent = new Child<int>();
    if ($independent->value() != 21) { panic("widened override availability"); }
    return viaBase($active) + viaInterface($active);
}
"#,
        42,
    );
}

#[test]
fn checked_execution_preserves_composed_error_cleanup() {
    assert_checked_execution(
        "composed checked-error cleanup",
        r#"
interface Ready {}
class Yes implements Ready {}
class Missing implements Error { string $message = "missing"; }
class State { static writable int $drops = 0; }
class Token { function __destruct() { State::drops++; } }
interface Work { function work(bool $fail): int throws Missing; }
trait Worker<T> {
    function work(bool $fail): int throws Missing where (T implements Ready) {
        let $token = new Token();
        if ($fail) { throw new Missing(); }
        return 7;
    }
}
class Adapter<T> implements Work where (T implements Ready) { uses Worker<T>; }
function invoke(Work $source, bool $fail): int throws Missing {
    return $source->work($fail);
}
function main(): int throws Missing {
    let $adapter = new Adapter<Yes>();
    if (invoke($adapter, false) != 7) { panic("successful conditional return"); }
    if (State::drops != 1) { panic("successful conditional cleanup"); }
    writable bool $caught = false;
    try { invoke($adapter, true); } catch (Missing) { $caught = true; }
    if (!$caught) { panic("conditional checked error"); }
    if (State::drops != 2) { panic("failed conditional cleanup"); }
    return State::drops;
}
"#,
        2,
    );
}

#[test]
fn checked_execution_preserves_composed_writable_borrowed_getters() {
    assert_checked_execution(
        "composed writable borrowed getter",
        r#"
interface Ready {}
class Yes implements Ready {}
class Box { int $value = 42; }
trait Slot<S, T> {
    writable T $stored;
    writable int $reads = 0;
    T $current where (S implements Ready) { writable borrowed get {
        $this->reads++;
        return $this->stored;
    } }
}
class Storage<S, T> {
    uses Slot<S, T>;
    function __construct(parameter take T $source) { $this->stored = $source; }
}
function main(): int {
    writable Storage<Yes, Box> $storage = new Storage<Yes, Box>(new Box());
    let $view = $storage->current;
    if ($storage->reads != 1) { panic("conditional getter receiver mutation"); }
    return $view->value;
}
"#,
        42,
    );
}

fn assert_checked_execution(name: &str, source: &str, status: i32) {
    let hir = crate::lower_source(name, source)
        .unwrap_or_else(|diagnostics| panic!("{name}: {diagnostics:?}"));
    let mir = crate::mir_lowering::lower_program(&hir)
        .unwrap_or_else(|diagnostics| panic!("{name}: {diagnostics:?}"));
    crate::mir_validation::validate_program(&mir).unwrap();
    let output = crate::mir_interpreter::interpret(&mir).unwrap();
    assert_eq!(output.exit_status, status, "{name}: {output:?}");
    assert!(output.stdout.is_empty(), "{name}: {output:?}");
    assert!(output.stderr.is_empty(), "{name}: {output:?}");
    assert!(output.runtime_diagnostic.is_none(), "{name}: {output:?}");
    for profile in [
        crate::backend::NativeProfile::Fast,
        #[cfg(feature = "llvm-backend")]
        crate::backend::NativeProfile::Release,
    ] {
        let output = crate::native_execution::execute_native(&mir, profile);
        assert_eq!(
            output.status.code(),
            Some(status),
            "{name}, {profile:?}: {output:?}"
        );
        assert!(output.stdout.is_empty(), "{name}, {profile:?}: {output:?}");
        assert!(output.stderr.is_empty(), "{name}, {profile:?}: {output:?}");
    }
    assert!(hir.semantic_info.closures.is_empty());
    assert!(hir.semantic_info.callable_value_calls.is_empty());
    for native in [Some(&mir), None] {
        let php = crate::codegen_php::generate(&hir, native).unwrap();
        let script = crate::php_execution::Script::new(&format!(
            "{}\nexit(__DoriaFunction_6d61696e());",
            php.strip_prefix("<?php").unwrap()
        ));
        let output = script.command(&["-d", "ffi.enable=0"]).output().unwrap();
        assert_eq!(output.status.code(), Some(status), "{name}: {output:?}");
        assert!(output.stdout.is_empty(), "{name}: {output:?}");
        assert!(output.stderr.is_empty(), "{name}: {output:?}");
    }
}

#[test]
fn conditional_source_facts_share_public_finalization_and_execution() {
    for (composed, captured) in [(false, false), (false, true), (true, false), (true, true)] {
        let answer = if captured {
            "let $value = self::make(42); let $read = fn() with ($value) => $value; return $read();"
        } else {
            "return self::make(42);"
        };
        let members = format!(
            r#"
    writable int $storedValue = 0;
    static function make<U>(U $value): U where (T implements Ready) {{ return identity($value); }}
    function answer(): int where (T implements Ready) {{ {answer} }}
    function choose<U>(take U $value): U where (T implements Ready, U implements Ready) {{ return $value; }}
    function read(T $value): int where (T implements Ready) {{ return $value->read(); }}
    writable int $value where (T implements Ready) {{ get => $this->storedValue; set (int $next) {{ $this->storedValue = $next; }} }}
"#
        );
        let (owner, parameter) = if composed {
            (
                format!("trait Access<T> {{ {members} }} trait Middle<U> {{ uses Access<U>; }} class Adapter<S> implements Answer where (S implements Ready) {{ uses Middle<S>; }} class Other<Q> implements Answer where (Q implements Ready) {{ uses Middle<Q>; }} class Inactive {{ uses Middle<int>; }}"),
                "S",
            )
        } else {
            (
                format!(
                    "class Adapter<T> implements Answer where (T implements Ready) {{ {members} }} class Other<T> implements Answer where (T implements Ready) {{ {members} }} class Inactive {{}}"
                ),
                "T",
            )
        };
        let source = format!(
            r#"
interface Ready {{ function read(): int; }}
class Yes implements Ready {{ function read(): int {{ return 7; }} }}
#[Attribute]
class Mark {{}}
interface Answer {{ function answer(): int; writable int $value {{ get; set (int $next); }} }}
function identity<U>(U $value): U {{ return $value; }}
trait Slot<S, T> {{
    writable T $stored;
    T $current where (S implements Ready) {{ writable borrowed get => $this->stored; }}
}}
class Storage<S, T> {{
    uses Slot<S, T>;
    function __construct(parameter take T $source) {{ $this->stored = $source; }}
}}
{owner}
#[Mark]
function inspect(writable Adapter<Yes> $source): int {{
    let $chosen = $source->choose(new Yes());
    if ($source->read($chosen) != 7) {{ panic("conditional constrained call"); }}
    $source->value = 42;
    return $source->answer() + $source->value;
}}
function keep(Adapter<int> $source): void {{}}
function viaInterface(writable Answer $source): int {{
    $source->value = 43;
    return $source->answer() + $source->value;
}}
function main(): int {{
    let $inactive = new Inactive();
    writable Adapter<Yes> $adapter = new Adapter<Yes>();
    if (inspect($adapter) != 84) {{ panic("conditional direct dispatch"); }}
    writable Other<Yes> $other = new Other<Yes>();
    if (viaInterface($other) != 85) {{ panic("conditional second receiver"); }}
    return viaInterface($adapter);
}}
"#
        );
        let (program, analysis) = with_declarations(&source, |declarations| {
            let program = declarations.program.clone();
            (program, checked_analysis(declarations))
        });
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
        let info = analysis.info;
        for method in info
            .method_hierarchy
            .values()
            .filter(|method| method.declaring_class == "Inactive")
        {
            if info
                .contracts
                .callable_conditions
                .contains_key(&method.declaration)
            {
                assert!(info.callable_signatures.contains_key(&method.declaration));
                assert!(info
                    .binding_resolution
                    .receiver_for_callable(method.declaration)
                    .is_none());
                assert!(!info
                    .expression_types
                    .keys()
                    .any(|span| method.declaration.contains(*span)));
            }
        }
        let mut receivers = Vec::new();
        let mut closures = Vec::new();
        for class in ["Adapter", "Other"] {
            let answer = info
                .method_hierarchy
                .values()
                .find(|method| method.declaring_class == class && method.name == "answer")
                .unwrap();
            receivers.push(
                info.binding_resolution
                    .receiver_for_callable(answer.declaration)
                    .unwrap(),
            );
            let closure =
                info.binding_resolution
                    .closure_owners
                    .iter()
                    .find_map(|(closure, owner)| {
                        (*owner == crate::symbols::LexicalOwner::Callable(answer.declaration))
                            .then_some(*closure)
                    });
            assert_eq!(closure.is_some(), captured);
            if let Some(closure) = closure {
                assert!(info.closures.contains_key(&closure));
                assert!(info.closure_ownership.contains_key(&closure));
                closures.push(closure);
            }
        }
        assert_ne!(receivers[0], receivers[1]);
        if captured {
            assert_ne!(closures[0], closures[1]);
        }
        assert!(info
            .attributes
            .applications
            .iter()
            .any(|application| { application.canonical_class_name == "Mark" }));
        let current =
            crate::property_hooks::accessor_name("current", crate::ast::PropertyHookKind::Get);
        let storage_getter = info
            .method_hierarchy
            .iter()
            .find_map(|(declaration, method)| {
                (method.declaring_class == "Storage" && method.name == current)
                    .then_some(*declaration)
            })
            .unwrap();
        assert_eq!(
            info.retained_callables[&storage_getter].writeback.returns,
            vec![crate::ownership::writeback::Home::Typed(
                ResolvedType::TypeParameter("T".into())
            )]
        );
        let getter =
            crate::property_hooks::accessor_name("value", crate::ast::PropertyHookKind::Get);
        let setter =
            crate::property_hooks::accessor_name("value", crate::ast::PropertyHookKind::Set);
        let effective = info.composition.apply(&program);
        let make_parameter = effective
            .items
            .iter()
            .find_map(|item| match item {
                Item::Class(class) if class.name == "Adapter" => {
                    class.members.iter().find_map(|member| match member {
                        ClassMember::Method(method) if method.name == "make" => {
                            Some(method.type_params[0].name.clone())
                        }
                        _ => None,
                    })
                }
                _ => None,
            })
            .unwrap();
        for name in ["answer", "make", getter.as_str(), setter.as_str()] {
            let declaration = info
                .method_hierarchy
                .iter()
                .find_map(|(declaration, method)| {
                    (method.declaring_class == "Adapter" && method.name == name)
                        .then_some(*declaration)
                })
                .unwrap();
            for (ty, expected) in [
                (ResolvedType::Class(ClassType::new("Yes", vec![])), true),
                (ResolvedType::Integer(IntegerType::Int64), false),
            ] {
                let mut bindings = HashMap::from([(parameter.into(), ty)]);
                if name == "make" {
                    bindings.insert(
                        make_parameter.clone(),
                        ResolvedType::Integer(IntegerType::Int64),
                    );
                }
                assert_eq!(
                    info.callable_available(declaration, &bindings),
                    Some(expected),
                    "composed={composed}, member={name}"
                );
            }
        }
        let conformance = info
            .contracts
            .conformances
            .iter()
            .find(|fact| {
                fact.interface.name == "Answer"
                    && fact.implementing_type
                        == ResolvedType::Class(ClassType::new(
                            "Adapter",
                            vec![ResolvedType::Class(ClassType::new("Yes", vec![]))],
                        ))
            })
            .unwrap();
        assert_eq!(
            info.interface_witnesses
                .conformance(conformance)
                .unwrap()
                .len(),
            3
        );
        assert!(!info.contracts.conformances.iter().any(|fact| {
            fact.interface.name == "Answer"
                && fact.implementing_type
                    == ResolvedType::Class(ClassType::new(
                        "Adapter",
                        vec![ResolvedType::Integer(IntegerType::Int64)],
                    ))
        }));
        assert!(matches!(
            info.virtual_dispatch,
            super::super::virtual_dispatch::Plan::Checked { .. }
        ));
        assert!(!info.call_targets.is_empty());
        assert!(!info.binding_resolution.declarations_by_id.is_empty());
        let context = info.compilation_context.clone();
        let hir = crate::lowering::lower_program_with_semantics(&program, info).unwrap();
        let hir = crate::complete_standalone_hir(
            hir,
            crate::source::SourceFile::new(context.source.0.clone(), source.clone()),
            &context,
        );
        let constructors = crate::monomorphization::synthetic_constructors(&hir);
        let declarations = crate::monomorphization::callable_declarations(&hir, &constructors);
        let classes = hir
            .semantic_info
            .classes
            .iter()
            .map(|class| {
                (
                    ClassType::new(class.declaration_name.clone(), class.arguments.clone()),
                    class.id,
                )
            })
            .collect();
        let (instances, _) = crate::monomorphization::collect_callable_instances(
            &hir,
            &declarations,
            &classes,
            &hir.semantic_info,
        )
        .unwrap();
        assert!(!instances.iter().any(|instance| {
            let declaration = declarations[instance.declaration];
            hir.semantic_info
                .method_hierarchy
                .get(&declaration.function.span)
                .is_some_and(|method| {
                    method.declaring_class == "Inactive"
                        && hir
                            .semantic_info
                            .contracts
                            .callable_conditions
                            .contains_key(&method.declaration)
                })
        }));
        for name in [
            "answer",
            getter.as_str(),
            setter.as_str(),
            "make",
            "choose",
            "read",
        ] {
            for (argument, expected) in [
                (ResolvedType::Class(ClassType::new("Yes", vec![])), true),
                (ResolvedType::Integer(IntegerType::Int64), false),
            ] {
                assert_eq!(
                    instances.iter().any(|instance| {
                        let declaration = declarations[instance.declaration];
                        declaration.function.name == name
                            && hir
                                .semantic_info
                                .method_hierarchy
                                .get(&declaration.function.span)
                                .is_some_and(|method| method.declaring_class == "Adapter")
                            && declaration.class_arguments == [argument.clone()]
                    }),
                    expected,
                    "composed={composed}, materialized={name}<{argument:?}>"
                );
            }
        }
        assert!(instances.iter().any(|instance| {
            let declaration = declarations[instance.declaration];
            declaration.function.name == "identity"
                && instance.arguments
                    == [super::super::GenericArgument::Type(ResolvedType::Integer(
                        IntegerType::Int64,
                    ))]
        }));
        let mir = crate::mir_lowering::lower_program(&hir).unwrap();
        crate::mir_validation::validate_program(&mir).unwrap();
        let output = crate::mir_interpreter::interpret(&mir).unwrap();
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
        assert_eq!(output.exit_status, 85);
        assert!(output.runtime_diagnostic.is_none());
        for profile in [
            crate::backend::NativeProfile::Fast,
            #[cfg(feature = "llvm-backend")]
            crate::backend::NativeProfile::Release,
        ] {
            let output = crate::native_execution::execute_native(&mir, profile);
            assert_eq!(output.status.code(), Some(85), "{profile:?}: {output:?}");
            assert!(output.stdout.is_empty(), "{profile:?}: {output:?}");
            assert!(output.stderr.is_empty(), "{profile:?}: {output:?}");
        }
        // The production PHP backend requires MIR for closure descriptors.
        // Closure-free variants also exercise its MIR-independent path.
        for native in std::iter::once(Some(&mir)).chain((!captured).then_some(None)) {
            let php = crate::codegen_php::generate(&hir, native).unwrap();
            let body = format!(
                "{}\nexit(__DoriaFunction_6d61696e());",
                php.strip_prefix("<?php").unwrap()
            );
            let script = crate::php_execution::Script::new(&body);
            let output = script.command(&["-d", "ffi.enable=0"]).output().unwrap();
            assert_eq!(output.status.code(), Some(85), "{output:?}\n{php}");
            assert!(output.stdout.is_empty(), "{output:?}\n{php}");
            assert!(output.stderr.is_empty(), "{output:?}\n{php}");
        }
        let (_, public) =
            crate::analyze_source_for_ide("public-conditional.doria", &source).unwrap();
        assert!(public.diagnostics.is_empty(), "{:?}", public.diagnostics);
        assert!(!public.info.classes.is_empty());
        assert!(!public.info.call_targets.is_empty());
        assert!(!public.info.binding_resolution.declarations_by_id.is_empty());
        assert!(matches!(
            public.info.callable_admissions,
            super::super::CallableAdmissions::Checked(_)
        ));
        assert!(matches!(
            public.info.interface_witnesses,
            super::super::interface_witnesses::Plan::Checked(_)
        ));
        assert!(matches!(
            public.info.virtual_dispatch,
            super::super::virtual_dispatch::Plan::Checked { .. }
        ));
        let invalid = with_declarations(
            &format!("{source}\nconst int BROKEN = 1 / 0;"),
            checked_analysis,
        );
        assert!(invalid
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.starts_with("E04")));
        assert!(matches!(
            invalid.info.interface_witnesses,
            super::super::interface_witnesses::Plan::Unprepared
        ));
    }
}

#[test]
fn checked_preparation_preserves_errors_in_inactive_trait_bodies() {
    let source = r#"
interface Ready { function read(): int; }
trait Reader<T> {
    function read(T $source): int where (T implements Ready) {
        return $source->missing();
    }
}
class Dormant { uses Reader<int>; }
function main(): int { let $reader = new Dormant(); return 0; }
"#;
    let analysis = with_declarations(source, checked_analysis);
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0537"),
        "{:?}",
        analysis.diagnostics
    );
    let declaration = analysis
        .info
        .method_hierarchy
        .values()
        .find(|method| method.declaring_class == "Dormant" && method.name == "read")
        .unwrap()
        .declaration;
    assert!(analysis.info.callable_signatures.contains_key(&declaration));
    assert!(analysis
        .info
        .binding_resolution
        .receiver_for_callable(declaration)
        .is_none());
    assert!(matches!(
        analysis.info.interface_witnesses,
        super::super::interface_witnesses::Plan::Unprepared
    ));
    assert!(matches!(
        analysis.info.virtual_dispatch,
        super::super::virtual_dispatch::Plan::Unprepared
    ));
}

fn ownership_facts(
    checker: &mut Checker<'_>,
) -> super::super::specialized_ownership::FactsByInstance {
    let (ownership, analysis) = analyze_ownership(checker);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert!(checker.diagnostics.is_empty(), "{:?}", checker.diagnostics);
    ownership
}

fn analyze_ownership(
    checker: &mut Checker<'_>,
) -> (
    super::super::specialized_ownership::FactsByInstance,
    crate::ownership::OwnershipAnalysis,
) {
    let fields = checker.property_backing_fields();
    let program = checker.callable_body_program();
    let classes = super::super::collect_ordered_class_semantics_once(
        checker.program,
        checker,
        &fields,
        super::super::ClassFactsPurpose::SourceOwnership,
    );
    let enums = super::super::collect_ordered_enum_semantics(checker);
    let conversions = checker.interface_conversion_types();
    let provenance = checker.prepare_call_provenance(&fields);
    checker.analyze_ownership(program, &classes, &enums, &conversions, &provenance)
}

#[test]
fn guarded_borrowed_getters_share_the_receiver_result_contract() {
    for composed in [false, true] {
        for (result, valid) in [("$this->stored", true), ("new Item()", false)] {
            let members = format!(
                "Item $stored = new Item(); Item $current where (S implements Ready) {{ borrowed get => {result}; }}"
            );
            let owner = if composed {
                format!("trait View<S> {{ {members} }} class Adapter<T> {{ uses View<T>; }}")
            } else {
                format!("class Adapter<S> {{ {members} }}")
            };
            let source = format!("interface Ready {{}} class Item {{}} {owner}");
            with_checker_diagnostics(&source, |checker| {
                assert_eq!(
                    checker.diagnostics.iter().any(|d| d.code == "E0766"),
                    !valid,
                    "{source}: {:?}",
                    checker.diagnostics
                );
                assert!(
                    checker.diagnostics.iter().all(|d| d.code == "E0766"),
                    "{source}: {:?}",
                    checker.diagnostics
                );
            });
            let ordinary = source.replace(" where (S implements Ready)", "");
            let (_, analysis) = crate::analyze_source_for_ide("ordinary.doria", &ordinary).unwrap();
            assert_eq!(
                analysis.diagnostics.iter().any(|d| d.code == "E0766"),
                !valid,
                "{ordinary}: {:?}",
                analysis.diagnostics
            );
        }
    }
}

#[test]
fn guarded_hooks_check_helpers_callbacks_and_cleanup_without_concrete_callers() {
    for composed in [false, true] {
        for (body, blocks) in [
            ("return output();", true),
            ("let $action = fn() => output(); return $action();", true),
            ("let $owned = new Blocker(); return 1;", true),
            ("let $action = fn() => 7; return $action();", false),
        ] {
            let members = format!("int $value where (S implements Ready) {{ get {{ {body} }} }}");
            let owner = if composed {
                format!("trait View<S> {{ {members} }} class Adapter<T> {{ uses View<T>; }}")
            } else {
                format!("class Adapter<S> {{ {members} }}")
            };
            let source = format!(
                r#"
interface Ready {{}}
function output(): int {{ echo "blocked"; return 1; }}
class Blocker {{ function __destruct() {{ try {{ echo "cleanup"; }} catch (Error) {{}} }} }}
{owner}
"#
            );
            with_checker_diagnostics(&source, |checker| {
                assert_eq!(
                    checker.diagnostics.iter().any(|d| d.code == "E0770"),
                    blocks,
                    "{source}: {:?}",
                    checker.diagnostics
                );
                assert!(
                    checker.diagnostics.iter().all(|d| d.code == "E0770"),
                    "{source}: {:?}",
                    checker.diagnostics
                );
            });
            let ordinary = source.replace(" where (S implements Ready)", "");
            let (_, analysis) = crate::analyze_source_for_ide("ordinary.doria", &ordinary).unwrap();
            assert_eq!(
                analysis.diagnostics.iter().any(|d| d.code == "E0770"),
                blocks,
                "{ordinary}: {:?}",
                analysis.diagnostics
            );
        }
    }
}

#[test]
fn guarded_composed_hooks_follow_pure_aliases_and_deferred_callbacks() {
    for body in [
        "return $this->chosen();",
        "let $action = fn() => 7; return $action();",
    ] {
        let source = format!(
            r#"
interface Ready {{}}
trait Operations<S> {{ function pure(): int where (S implements Ready) {{ return 7; }} }}
trait View<S> {{
    uses Operations<S> {{ Operations<S>::pure as chosen; }}
    int $value where (S implements Ready) {{ get {{ {body} }} }}
    function(): int $deferred where (S implements Ready) {{ get => fn() => output(); }}
}}
function output(): int {{ echo "later"; return 1; }}
class Adapter<T> {{ uses View<T>; }}
"#
        );
        with_checker(&source, |_| {});
    }
}

#[test]
fn guarded_programs_preserve_call_scoped_request_storage_rules() {
    for request in ["ReadRequest", "WriteRequest"] {
        for declaration in [
            format!("function own(take {request} $value): void {{}}"),
            format!("function pack(List<{request}> $values): void {{}}"),
            format!("class Stored {{ ?{request} $value = null; }}"),
            format!("trait Stored {{ ?{request} $value = null; }} class Container {{ uses Stored; }}"),
            format!("function escape({request} $value): function(): void {{ return fn() with ($value) => touch($value); }} function touch({request} $value): void {{}}"),
        ] {
            let source = format!(r#"
use Doria\Std\Io\{request};
interface Ready {{}}
class Guard<S> {{ function marker(): void where (S implements Ready) {{}} }}
{declaration}
"#);
            with_checker_diagnostics(&source, |checker| {
                assert!(checker.diagnostics.iter().any(|d| d.code == "E0772"),
                    "{source}: {:?}", checker.diagnostics);
            });
        }
        let source = format!(
            r#"
use Doria\Std\Io\{request};
interface Ready {{}}
class Guard<S> {{ function marker(): void where (S implements Ready) {{}} }}
function lend({request} $value): {request} {{ return $value; }}
"#
        );
        with_checker(&source, |_| {});
    }
}

#[test]
fn guarded_constructors_preserve_initialization_and_partial_cleanup_facts() {
    for composed in [false, true] {
        let fields = "Item $first = new Item(); Item $second = fail();";
        let owner = if composed {
            format!("trait Slots {{ {fields} }} class Adapter<S> {{ uses Slots; function __construct() throws Missing {{}} function marker(): void where (S implements Ready) {{}} }}")
        } else {
            format!("class Adapter<S> {{ {fields} function __construct() throws Missing {{}} function marker(): void where (S implements Ready) {{}} }}")
        };
        let source = format!(
            r#"
interface Ready {{}}
class Item {{}}
class Missing implements Error {{ string $message = "missing"; }}
function fail(): Item throws Missing {{ throw new Missing(); }}
{owner}
"#
        );
        with_checker(&source, |checker| {
            let fields = checker.property_backing_fields();
            let partial = checker.check_constructor_initialization(&fields);
            let class = &checker.classes["Adapter"];
            let first = class.properties["first"].declaration_span;
            let first = fields.get(&first).map_or(first, |field| field.declaration);
            let release = partial
                .iter()
                .find(|release| release.class == class.declaration)
                .expect("throwing initializer retains its composing-class cleanup");
            assert_eq!(release.initialized_properties, vec![first], "{source}");
        });
        let source = source.replace("Item $second = fail();", "Item $second;");
        with_checker_diagnostics(&source, |checker| {
            assert!(
                checker.diagnostics.iter().any(|d| d.code == "E0500"),
                "{source}: {:?}",
                checker.diagnostics
            );
        });
    }
}

#[test]
fn conditional_pending_phases_keep_their_nominal_nonblocking_contract() {
    for reading in [false, true] {
        for composed in [false, true] {
            for preparation in [false, true] {
                for (guarded, blocks) in [(false, true), (true, true), (true, false)] {
                    let condition = if guarded {
                        " where (S implements Ready)"
                    } else {
                        ""
                    };
                    let operation = if blocks { "echo \"blocked\";" } else { "" };
                    let argument = if reading { "destination" } else { "source" };
                    let prepare = if preparation { "$this->chosen();" } else { "" };
                    let complete = if preparation { "" } else { "$this->chosen();" };
                    let (capability, file, request, outcome, region, method, phase) = if reading {
                        (
                            "PendingReadableStream",
                            "ReadableFile",
                            "ReadRequest",
                            "ReadOutcome",
                            "writable WritableByteRegion",
                            "read",
                            "Read",
                        )
                    } else {
                        (
                            "PendingWritableStream",
                            "WritableFile",
                            "WriteRequest",
                            "WriteOutcome",
                            "ByteRegion",
                            "write",
                            "Write",
                        )
                    };
                    let helpers = if composed {
                        format!("trait Operations<S> {{ function step(): void{condition} {{ {operation} }} }}")
                    } else {
                        String::new()
                    };
                    let step = if composed {
                        "uses Operations<S> { Operations<S>::step as chosen; }".to_string()
                    } else {
                        format!("function chosen(): void{condition} {{ {operation} }}")
                    };
                    let members = format!(
                        r#"
    {step}
    internal writable {file} $source;
    ReadinessSource $readinessSource {{ borrowed get => $this->source->readinessSource; }}
    writable function {method}({region} ${argument}): {outcome} throws IoError {{ return $this->source->{method}(${argument}); }}
    writable function prepare{phase}(writable {request} $request, {region} ${argument}): void throws IoError{condition} {{ {prepare} }}
    writable function complete{phase}({request} $request, {region} ${argument}): ?{outcome} throws IoError{condition} {{ {complete} return {outcome}::WouldBlock; }}
"#
                    );
                    let owner = if composed {
                        format!("trait Phases<S> {{ {members} }} class Adapter<S> implements {capability}{condition} {{ uses Phases<S>; function __construct(parameter take {file} $source) {{ $this->source = $source; }} }}")
                    } else {
                        format!("class Adapter<S> implements {capability}{condition} {{ {members} function __construct(parameter take {file} $source) {{ $this->source = $source; }} }}")
                    };
                    let source = format!(
                        r#"
use Doria\Std\Io\{{PendingWritableStream, WritableFile, WriteRequest, WriteOutcome,
    PendingReadableStream, ReadableFile, ReadRequest, ReadOutcome,
    ReadinessSource, WritableByteRegion, ByteRegion, IoError}};
interface Ready {{}}
{helpers}
{owner}
"#
                    );
                    with_checker_diagnostics(&source, |checker| {
                        assert_eq!(
                            checker.diagnostics.iter().any(|d| d.code == "E0771"),
                            blocks,
                            "{source}: {:?}",
                            checker.diagnostics
                        );
                        assert!(
                            checker.diagnostics.iter().all(|d| d.code == "E0771"),
                            "{source}: {:?}",
                            checker.diagnostics
                        );
                        if guarded {
                            assert!(!checker.contracts.conformances.iter().any(|fact| {
                    matches!(&fact.implementing_type, ResolvedType::Class(class) if class.name == "Adapter")
                }), "source requirements must not publish an unconditional witness");
                        }
                    });
                }
            }
        }
    }
}

#[test]
fn guarded_hooks_include_partial_constructor_cleanup() {
    for composed in [false, true] {
        for (operation, blocks) in [
            ("", false),
            ("try { echo \"cleanup\"; } catch (Error) {}", true),
        ] {
            let fields = "Token $first = new Token(); Token $second = fail();";
            let record = if composed {
                format!("trait Fields {{ {fields} }} class Record<S> {{ uses Fields; function __construct() throws Missing {{}} }}")
            } else {
                format!("class Record<S> {{ {fields} function __construct() throws Missing {{}} }}")
            };
            let source = format!(
                r#"
interface Ready {{}}
class Token {{ function __destruct() {{ {operation} }} }}
class Missing implements Error {{ string $message = "missing"; }}
function fail(): Token throws Missing {{ throw new Missing(); }}
{record}
class View<S> {{
    int $number where (S implements Ready) {{ get {{
        try {{ let $record = new Record<S>(); }} catch (Missing) {{}}
        return 7;
    }} }}
}}
"#
            );
            with_checker_diagnostics(&source, |checker| {
                assert_eq!(
                    checker.diagnostics.iter().any(|d| d.code == "E0770"),
                    blocks,
                    "{source}: {:?}",
                    checker.diagnostics
                );
                assert!(
                    checker.diagnostics.iter().all(|d| d.code == "E0770"),
                    "{source}: {:?}",
                    checker.diagnostics
                );
            });
        }
    }
}

#[test]
fn pending_phase_names_without_nominal_conformance_remain_ordinary_methods() {
    with_checker(
        r#"
interface Ready {}
class Adapter<S> {
    function prepareWrite(): void where (S implements Ready) { echo "ordinary"; }
    function completeRead(): void where (S implements Ready) { echo "ordinary"; }
}
"#,
        |_| {},
    );
}

#[test]
fn nonblocking_cleanup_preserves_generic_obligations_and_composing_receivers() {
    for composed in [false, true] {
        for (value, blocks) in [("Quiet", false), ("Blocker", true)] {
            let owner = if composed {
                format!("trait Consume<S> {{ function accept<U>(take U $value): int where (S implements Ready) {{ return 7; }} }} trait View<S> {{ uses Consume<S> {{ Consume<S>::accept as chosen; }} int $number where (S implements Ready) {{ get => $this->chosen(new {value}()); }} }} class Adapter<T> {{ uses View<T>; }}")
            } else {
                format!("class Adapter<S> {{ int $number where (S implements Ready) {{ get => consume(new {value}()); }} }}")
            };
            let source = format!(
                r#"
interface Ready {{}}
class Quiet {{}}
class Blocker {{ function __destruct() {{ try {{ echo "cleanup"; }} catch (Error) {{}} }} }}
function consume<T>(take T $value): int {{ return 7; }}
function unrelated(): int {{ return consume(new Blocker()); }}
{owner}
"#
            );
            with_checker_diagnostics(&source, |checker| {
                assert_eq!(
                    checker.diagnostics.iter().any(|d| d.code == "E0770"),
                    blocks,
                    "{source}: {:?}",
                    checker.diagnostics
                );
                assert!(
                    checker.diagnostics.iter().all(|d| d.code == "E0770"),
                    "{source}: {:?}",
                    checker.diagnostics
                );
            });
        }
    }
}

#[test]
fn guarded_returned_callbacks_keep_specialized_cleanup_and_escape_separate() {
    for composed in [false, true] {
        for value in ["Quiet", "Blocker"] {
            for escape in [false, true] {
                let method = "function retain<T>(take T $value): function(): int where (S implements Ready) { return fn() with (take $value) => observe($value); }";
                let (result, body) = if escape {
                    (
                        "function(): int",
                        format!("return $this->chosen(new {value}());"),
                    )
                } else {
                    (
                        "int",
                        format!(
                            "let $callback = $this->chosen(new {value}()); return $callback();"
                        ),
                    )
                };
                let owner = if composed {
                    format!("trait Retain<S> {{ {method} }} trait View<S> {{ uses Retain<S> {{ Retain<S>::retain as chosen; }} {result} $result where (S implements Ready) {{ get {{ {body} }} }} }} class Adapter<U> {{ uses View<U>; }}")
                } else {
                    format!("class Adapter<S> {{ {} {result} $result where (S implements Ready) {{ get {{ {body} }} }} }}", method.replace("function retain<T>", "function chosen<T>"))
                };
                let source = format!(
                    r#"
interface Ready {{}}
class Quiet {{}}
class Blocker {{ function __destruct() {{ try {{ echo "cleanup"; }} catch (Error) {{}} }} }}
function observe<T>(T $value): int {{ return 7; }}
{owner}
"#
                );
                with_checker_diagnostics(&source, |checker| {
                    assert_eq!(
                        checker.diagnostics.iter().any(|d| d.code == "E0770"),
                        value == "Blocker" && !escape,
                        "{source}: {:?}",
                        checker.diagnostics
                    );
                    assert!(
                        checker.diagnostics.iter().all(|d| d.code == "E0770"),
                        "{source}: {:?}",
                        checker.diagnostics
                    );
                });
            }
        }
    }
}

#[test]
fn guarded_composed_self_cleanup_uses_each_receivers_destructor() {
    for (name, operation, blocks) in [
        ("Quiet", "", false),
        (
            "Blocker",
            "try { echo \"cleanup\"; } catch (Error) {}",
            true,
        ),
    ] {
        let source = format!(
            r#"
interface Ready {{}}
class Yes implements Ready {{}}
trait Factory<S> {{
    function make(): self where (S implements Ready) {{ return new self(); }}
}}
trait View<S> {{
    uses Factory<S> {{ Factory<S>::make as chosen; }}
    int $number where (S implements Ready) {{ get {{
        let $value = $this->chosen();
        return 7;
    }} }}
}}
class {name} {{ uses View<Yes>; function __destruct() {{ {operation} }} }}
"#
        );
        with_checker_diagnostics(&source, |checker| {
            assert_eq!(
                checker.diagnostics.iter().any(|d| d.code == "E0770"),
                blocks,
                "{source}: {:?}",
                checker.diagnostics
            );
            assert!(
                checker.diagnostics.iter().all(|d| d.code == "E0770"),
                "{source}: {:?}",
                checker.diagnostics
            );
        });
    }
}

#[test]
fn guarded_region_copies_keep_storage_loans_after_descriptor_rebinding() {
    for composed in [false, true] {
        for mutation in [false, true] {
            let method = "function relay<T>(take T $value): T where (S implements Ready) { let $copy = fn(take T $item) => $item; return $copy($value); }";
            let owner = if composed {
                format!("trait Relay<S> {{ {method} }} trait Middle<M> {{ uses Relay<M> {{ Relay<M>::relay as chosen; }} }} class Adapter<U> {{ uses Middle<U>; }}")
            } else {
                format!(
                    "class Adapter<S> {{ {} }}",
                    method.replace("function relay<T>", "function chosen<T>")
                )
            };
            let mutation = if mutation { "$left[0] = 9;" } else { "" };
            let source = format!(
                r#"
use Doria\Std\Io\ByteRegion;
interface Ready {{}}
class Yes implements Ready {{}}
{owner}
function inspect(Adapter<Yes> $adapter): void {{
    writable Bytes $left = Bytes::allocate(2);
    writable Bytes $right = Bytes::allocate(2);
    let writable $descriptor = $left->getRegion(0, 1);
    let $copy = $adapter->chosen($descriptor);
    $descriptor = $right->getRegion(0, 1);
    {mutation}
    echo $copy[0];
    $left[0] = 7;
}}
"#
            );
            with_checker_diagnostics(&source, |checker| {
                assert_eq!(
                    checker.diagnostics.iter().any(|d| d.code == "E0763"),
                    !mutation.is_empty(),
                    "{source}: {:?}",
                    checker.diagnostics
                );
                assert!(
                    checker.diagnostics.iter().all(|d| d.code == "E0763"),
                    "{source}: {:?}",
                    checker.diagnostics
                );
            });
            let ordinary = source.replace(" where (S implements Ready)", "");
            let (_, analysis) =
                crate::analyze_source_for_ide("ordinary-region.doria", &ordinary).unwrap();
            assert_eq!(
                analysis.diagnostics.iter().any(|d| d.code == "E0763"),
                !mutation.is_empty(),
                "{ordinary}: {:?}",
                analysis.diagnostics
            );
            assert!(
                analysis.diagnostics.iter().all(|d| d.code == "E0763"),
                "{ordinary}: {:?}",
                analysis.diagnostics
            );
        }
    }
}

#[test]
fn guarded_request_captures_are_checked_after_generic_substitution() {
    for composed in [false, true] {
        for ty in ["int", "ReadRequest", "WriteRequest"] {
            let method = "function retain<T>(T $value): function(): int where (S implements Ready) { return fn() with ($value) => observe($value); }";
            let owner = if composed {
                format!("trait Retain<S> {{ {method} }} class Adapter<U> {{ uses Retain<U> {{ Retain<U>::retain as chosen; }} }}")
            } else {
                format!(
                    "class Adapter<S> {{ {} }}",
                    method.replace("function retain<T>", "function chosen<T>")
                )
            };
            let source = format!(
                r#"
use Doria\Std\Io\{{ReadRequest, WriteRequest}};
interface Ready {{}}
class Yes implements Ready {{}}
function observe<T>(T $value): int {{ return 7; }}
{owner}
function forward(Adapter<Yes> $adapter, {ty} $value): function(): int {{ return $adapter->chosen($value); }}
"#
            );
            with_checker_diagnostics(&source, |checker| {
                assert_eq!(
                    checker.diagnostics.iter().any(|d| d.code == "E0772"),
                    ty != "int",
                    "{source}: {:?}",
                    checker.diagnostics
                );
                assert!(
                    checker.diagnostics.iter().all(|d| d.code == "E0772"),
                    "{source}: {:?}",
                    checker.diagnostics
                );
            });
        }
    }
}

#[test]
fn guarded_inherited_partial_cleanup_preserves_parent_type_bindings() {
    for transitive in [false, true] {
        for container in [false, true] {
            for (value, blocks) in [("Quiet", false), ("Blocker", true)] {
                let parent = if transitive { "Middle<U>" } else { "Parent<U>" };
                let parent_argument = if container { "List<U>" } else { "U" };
                let child_parameter = parent_argument;
                let argument = if container {
                    format!("[new {value}()]")
                } else {
                    format!("new {value}()")
                };
                let middle = if transitive {
                    "open class Middle<M> extends Parent<M> { function __construct(parameter take M $value) throws Missing { parent::__construct($value); } }".to_string()
                } else {
                    String::new()
                };
                let parent = if container {
                    parent.replace("<U>", "<List<U>>")
                } else {
                    parent.to_string()
                };
                let source = format!(
                    r#"
interface Ready {{}}
class Quiet {{}}
class Blocker {{ function __destruct() {{ try {{ echo "cleanup"; }} catch (Error) {{}} }} }}
class Missing implements Error {{ string $message = "missing"; }}
open class Parent<P> {{
    function __construct(take P $value) throws Missing {{ throw new Missing(); }}
}}
{middle}
class Child<U> extends {parent} {{
    function __construct(parameter take {child_parameter} $value) throws Missing {{ parent::__construct($value); }}
}}
class View<S> {{
    int $number where (S implements Ready) {{ get {{
        try {{ let $child = new Child<{value}>({argument}); }} catch (Missing) {{}}
        return 7;
    }} }}
}}
"#
                );
                with_checker_diagnostics(&source, |checker| {
                    assert_eq!(
                        checker.diagnostics.iter().any(|d| d.code == "E0770"),
                        blocks,
                        "{source}: {:?}",
                        checker.diagnostics
                    );
                    assert!(
                        checker.diagnostics.iter().all(|d| d.code == "E0770"),
                        "{source}: {:?}",
                        checker.diagnostics
                    );
                    let fields = checker.property_backing_fields();
                    let cleanups = checker.check_constructor_initialization(&fields);
                    let parent = &checker.classes["Parent"];
                    let property = parent.properties["value"].declaration_span;
                    let property = fields
                        .get(&property)
                        .map_or(property, |field| field.declaration);
                    assert!(
                cleanups.iter().any(|cleanup| cleanup.class
                    == checker.classes["Child"].declaration
                    && cleanup.initialized_properties.contains(&property)),
                "the child failure retains its inherited initialized property: {cleanups:?}"
            );
                });
            }
        }
    }
}

#[test]
fn guarded_pending_forwarders_use_abstract_nominal_nonblocking_contracts() {
    for reading in [false, true] {
        for blocks in [false, true] {
            let (capability, request, outcome, region, method, phase, argument) = if reading {
                (
                    "PendingReadableStream",
                    "ReadRequest",
                    "ReadOutcome",
                    "writable WritableByteRegion",
                    "read",
                    "Read",
                    "destination",
                )
            } else {
                (
                    "PendingWritableStream",
                    "WriteRequest",
                    "WriteOutcome",
                    "ByteRegion",
                    "write",
                    "Write",
                    "source",
                )
            };
            let preparation = if blocks {
                format!("$this->source->{method}(${argument});")
            } else {
                format!("$this->source->prepare{phase}($request, ${argument});")
            };
            let source = format!(
                r#"
use Doria\Std\Io\{{PendingReadableStream, PendingWritableStream, ReadRequest, WriteRequest,
    ReadOutcome, WriteOutcome, ReadinessSource, WritableByteRegion, ByteRegion, IoError}};
trait Forward<T> {{
    internal writable T $source;
    ReadinessSource $readinessSource where (T implements {capability}) {{ borrowed get => $this->source->readinessSource; }}
    writable function {method}({region} ${argument}): {outcome} throws IoError where (T implements {capability}) {{
        return $this->source->{method}(${argument});
    }}
    writable function prepare{phase}(writable {request} $request, {region} ${argument}): void throws IoError where (T implements {capability}) {{
        {preparation}
    }}
    writable function complete{phase}({request} $request, {region} ${argument}): ?{outcome} throws IoError where (T implements {capability}) {{
        return $this->source->complete{phase}($request, ${argument});
    }}
}}
class Adapter<S> implements {capability} where (S implements {capability}) {{
    uses Forward<S>;
    function __construct(parameter take S $source) {{ $this->source = $source; }}
}}
class Unready {{}}
function keep(Adapter<Unready> $adapter): void {{}}
"#
            );
            with_checker_diagnostics(&source, |checker| {
                assert_eq!(
                    checker.diagnostics.iter().any(|d| d.code == "E0771"),
                    blocks,
                    "{source}: {:?}",
                    checker.diagnostics
                );
                assert!(
                    checker.diagnostics.iter().all(|d| d.code == "E0771"),
                    "{source}: {:?}",
                    checker.diagnostics
                );
            });
        }
    }
}

#[test]
fn lifecycle_omitted_and_explicit_void_publish_the_same_cleanup_contract() {
    for annotation in ["", ": void"] {
        let source = format!(
            r#"
interface Ready {{}}
open class Parent<T> {{ function __construct(){annotation} {{}} function __destruct(){annotation} {{}} }}
class Child<U> extends Parent<U> {{ function __construct(){annotation} {{ parent::__construct(); }} }}
class Guard<S> {{ function marker(): void where (S implements Ready) {{}} }}
"#
        );
        with_checker(&source, |checker| {
            for (owner, method) in [
                ("Parent", "__construct"),
                ("Parent", "__destruct"),
                ("Child", "__construct"),
            ] {
                let declaration = checker.classes[owner].methods[method].declaration;
                let result = checker.function_signatures[&declaration].return_ty;
                assert_eq!(
                    checker.types.kind(result),
                    &TypeKind::Void,
                    "{owner}::{method}"
                );
            }
            let (_, ownership) = analyze_ownership(checker);
            for (site, target) in &checker.call_targets {
                if matches!(target, super::super::CallableTarget::Method { method_name, direct_parent: true, .. } if method_name == "__construct")
                {
                    assert_eq!(checker.expression_types[site], ResolvedType::Void);
                    assert!(
                        !ownership.cleanup.values.values().any(|value| value.source
                            == crate::ownership::cleanup::Source::Expression(*site)),
                        "a void lifecycle handoff creates no owned result"
                    );
                }
            }
        });
        let ordinary = source.replace(" where (S implements Ready)", "");
        crate::check_source("ordinary-lifecycle.doria", ordinary).unwrap();
    }
}

#[test]
fn checked_admissions_preserve_effective_owner_method_and_accessor_identities() {
    for composed in [false, true] {
        let members = r#"
    function optional(): int where (S implements Ready) { return 1; }
    static function fixed(): int where (S implements Ready) { return 2; }
    function choose<U>(take U $value): U where (S implements Ready, U implements Ready) { return $value; }
    writable int $value where (S implements Ready) { get => 3; set (int $next) {} }
"#;
        let owner = if composed {
            format!("trait Wrapper<S> {{ {members} }} trait Middle<V> {{ uses Wrapper<V> {{ Wrapper<V>::choose as select; }} }} class Adapter<T> {{ uses Middle<T>; }} class Fixed {{ uses Middle<Yes>; }}")
        } else {
            format!("class Adapter<S> {{ {members} }}")
        };
        let chosen = if composed { "select" } else { "choose" };
        let parameter = if composed { "T" } else { "S" };
        let source = format!(
            r#"
interface Ready {{}}
class Yes implements Ready {{}}
class No {{}}
{owner}
function inspect(Adapter<Yes> $adapter): Yes {{ return $adapter->{chosen}(new Yes()); }}
function keep(Adapter<No> $adapter): void {{}}
"#
        );
        with_checker(&source, |checker| {
            let info = SemanticInfo {
                contracts: checker.contracts.clone(),
                callable_admissions: checker.checked_callable_admissions(),
                ..SemanticInfo::default()
            };
            let class = &checker.classes["Adapter"];
            let hooks = class.properties["value"].hooks.as_ref().unwrap();
            for declaration in [
                class.methods["optional"].declaration,
                class.methods["fixed"].declaration,
                hooks.getter.unwrap().declaration,
                hooks.setter.unwrap().declaration,
            ] {
                for name in ["Yes", "No"] {
                    let bindings = HashMap::from([(
                        parameter.to_string(),
                        ResolvedType::Class(ClassType::new(name, vec![])),
                    )]);
                    assert_eq!(
                        info.callable_available(declaration, &bindings),
                        Some(name == "Yes"),
                        "composed={composed}: {name}: {declaration:?}"
                    );
                }
                assert_eq!(
                    info.callable_available(declaration, &HashMap::new()),
                    Some(false)
                );
            }
            let declaration = class.methods[chosen].declaration;
            let method_parameter = &class.methods[chosen].type_params[0].name;
            for owner in ["Yes", "No"] {
                for method in ["Yes", "No"] {
                    let bindings = HashMap::from([
                        (
                            method_parameter.clone(),
                            ResolvedType::Class(ClassType::new(method, vec![])),
                        ),
                        (
                            parameter.to_string(),
                            ResolvedType::Class(ClassType::new(owner, vec![])),
                        ),
                    ]);
                    assert_eq!(
                        info.callable_available(declaration, &bindings),
                        Some(owner == "Yes" && method == "Yes")
                    );
                }
            }
            if composed {
                assert_eq!(
                    info.callable_available(
                        checker.classes["Fixed"].methods["optional"].declaration,
                        &HashMap::new()
                    ),
                    Some(true)
                );
            }
            assert_eq!(
                SemanticInfo::default().callable_available(declaration, &HashMap::new()),
                Some(true)
            );
            let unprepared = SemanticInfo {
                contracts: info.contracts.clone(),
                ..SemanticInfo::default()
            };
            assert_eq!(
                unprepared.callable_available(declaration, &HashMap::new()),
                None
            );
        });
    }
}

#[test]
fn virtual_generic_calls_collect_override_body_dependencies() {
    let source = r#"
class Box<T> {}
open class Base { open function inspect<T>(T $value): int { return 1; } }
class Child extends Base {
    override function inspect<U>(U $value): int { let $box = new Box<U>(); return 2; }
}
function inspectBase(Base $source): int { return $source->inspect(7); }
function main(): void { echo inspectBase(new Child()); }
"#;
    with_checker(source, |checker| {
        let declaration = checker.classes["Child"].methods["inspect"].declaration;
        let int = checker.types.intern(TypeKind::Integer(IntegerType::Int64));
        assert!(
            checker.callable_instances.iter().any(|instance| {
                instance.declaration == declaration
                    && instance
                        .substitutions()
                        .values()
                        .any(|argument| *argument == int)
            }),
            "virtual override was not admitted"
        );
        assert!(checker
            .class_instantiations
            .contains(&ClassType::new("Box", vec![int])));
    });
}

#[test]
fn callable_dependency_roots_keep_self_and_mutual_references_acyclic() {
    for reverse in [false, true] {
        let first = "class First { function same(): ?First { return null; } static function other(): ?Second { return null; } }";
        let second = "class Second { function same(): ?Second { return null; } function other(): ?First { return null; } }";
        let source = if reverse {
            format!("{second}{first}")
        } else {
            format!("{first}{second}")
        };
        with_checker(&source, |checker| {
            let instances = checker.class_instantiations.clone();
            super::super::collect_concrete_instances(checker.program, checker);
            assert_eq!(checker.class_instantiations, instances);
            for name in ["First", "Second"] {
                assert!(checker
                    .class_instantiations
                    .contains(&ClassType::new(name, vec![])));
            }
            for root in checker.nominal_instantiation_parents.keys() {
                let mut visited = HashSet::new();
                let mut current = Some(root);
                while let Some(instance) = current {
                    assert!(
                        visited.insert(instance),
                        "dependency ancestry contains a cycle: {root:?}"
                    );
                    current = checker.nominal_instantiation_parents.get(instance);
                }
            }
            assert!(checker.diagnostics.is_empty(), "{:?}", checker.diagnostics);
        });
    }
}

#[test]
fn guarded_dependencies_follow_active_declarations_without_leaking_between_instances() {
    let source = r#"
interface Ready {}
interface Uses<T> {}
class Yes implements Ready {}
class No {}
class Storage<T> {}
class MethodValue<T> {}
class PropertyValue<T> {}
class ConformanceValue<T> {}
class FixedValue<T> {}
class UncalledValue<T> {}
class Owner<T> implements Uses<ConformanceValue<T>> where (T implements Ready) {
    ?Storage<T> $storage = null;
    function enabled(): ?MethodValue<T> where (T implements Ready) { return null; }
    static function fixed(): ?FixedValue<int> where (T implements Ready) { return null; }
    ?PropertyValue<T> $value where (T implements Ready) { get { return null; } }
    function unused<U>(U $value): ?UncalledValue<int> { return null; }
}
"#;
    for order in [["No", "Yes"], ["Yes", "No"]] {
        with_checker(source, |checker| {
            let mut saw_active = false;
            for name in order {
                let argument = checker
                    .types
                    .intern(TypeKind::Class(ClassType::new(name, vec![])));
                checker
                    .class_instantiations
                    .insert(ClassType::new("Owner", vec![argument]));
                super::super::collect_concrete_instances(checker.program, checker);
                saw_active |= name == "Yes";
                assert!(!checker
                    .class_instantiations
                    .iter()
                    .any(|instance| instance.name == "UncalledValue"));
                let has = |owner: &str| {
                    checker
                        .class_instantiations
                        .contains(&ClassType::new(owner, vec![argument]))
                };
                assert!(
                    has("Storage"),
                    "{name}: unconditional storage must remain available"
                );
                for owner in ["MethodValue", "PropertyValue", "ConformanceValue"] {
                    assert_eq!(has(owner), name == "Yes", "{name}: {owner}");
                }
                assert_eq!(
                    checker
                        .class_instantiations
                        .iter()
                        .any(|instance| instance.name == "FixedValue"),
                    saw_active
                );
                assert!(
                    checker.diagnostics.is_empty(),
                    "{name}: {:?}",
                    checker.diagnostics
                );
            }
        });
    }
}

#[test]
fn guarded_dependencies_do_not_seed_calls_from_inactive_bodies() {
    let source = r#"
interface Ready {}
class Yes implements Ready {}
class No {}
class Value<T> {}
function make<U>(U $value): ?Value<U> { return null; }
class Owner<T> {
    function enabled(): ?Value<Yes> where (T implements Ready) { return make(new Yes()); }
}

"#;
    with_checker(source, |checker| {
        let make = checker.functions["make"].declaration;
        for name in ["No", "Yes"] {
            let argument = checker
                .types
                .intern(TypeKind::Class(ClassType::new(name, vec![])));
            checker
                .class_instantiations
                .insert(ClassType::new("Owner", vec![argument]));
            super::super::collect_concrete_instances(checker.program, checker);
            assert_eq!(
                checker
                    .class_instantiations
                    .iter()
                    .any(|instance| instance.name == "Value"),
                name == "Yes"
            );
            assert_eq!(
                checker
                    .callable_instances
                    .iter()
                    .any(|instance| instance.declaration == make),
                name == "Yes"
            );
            assert!(
                checker.diagnostics.is_empty(),
                "{name}: {:?}",
                checker.diagnostics
            );
        }
    });
}

#[test]
fn enum_dependencies_follow_callable_and_property_availability() {
    let source = r#"
interface Ready {}
class Yes implements Ready {}
class No {}
enum MethodValue<T> { case Value(T $value); }
enum PropertyValue<T> { case Value(T $value); }
enum FixedValue<T> { case Value(T $value); }
enum UncalledValue<T> { case Value(T $value); }
class Owner<T> {
    function enabled(): ?MethodValue<T> where (T implements Ready) { return null; }
    static function fixed(): ?FixedValue<int> where (T implements Ready) { return null; }
    ?PropertyValue<T> $value where (T implements Ready) { get { return null; } }
    function unused<U>(U $value): ?UncalledValue<int> { return null; }
}
"#;
    for order in [["No", "Yes"], ["Yes", "No"]] {
        with_checker_diagnostics(source, |checker| {
            let mut saw_active = false;
            for name in order {
                let argument = checker
                    .types
                    .intern(TypeKind::Class(ClassType::new(name, vec![])));
                checker
                    .class_instantiations
                    .insert(ClassType::new("Owner", vec![argument]));
                super::super::collect_concrete_instances(checker.program, checker);
                saw_active |= name == "Yes";
                for owner in ["MethodValue", "PropertyValue"] {
                    assert_eq!(
                        checker
                            .enum_instances
                            .keys()
                            .any(|instance| instance.name == owner
                                && instance.arguments == [argument]),
                        name == "Yes",
                        "{name}: {owner}"
                    );
                }
                assert_eq!(
                    checker
                        .enum_instances
                        .keys()
                        .any(|instance| instance.name == "FixedValue"),
                    saw_active
                );
                assert!(!checker
                    .enum_instances
                    .keys()
                    .any(|instance| instance.name == "UncalledValue"));
                assert!(
                    checker.diagnostics.is_empty(),
                    "{name}: {:?}",
                    checker.diagnostics
                );
            }
        });
    }
}

#[test]
fn guarded_composed_bodies_collect_authored_call_and_class_dependencies() {
    for composed in [false, true] {
        for reverse in [false, true] {
            let declarations = "interface Ready {} class Yes implements Ready {} class No {} \
                class Box<T> {} class Local<T> {} \
                function produce<U>(take U $value): int { let $box = new Box<U>(); return 1; }";
            let method = "function run(): int where (S implements Ready) { \
                let $local = new Local<S>(); return produce(7); }";
            let owner = if composed {
                format!("trait Wrapper<S> {{ {method} }} class Adapter<T> {{ uses Wrapper<T>; }}")
            } else {
                format!("class Adapter<S> {{ {method} }}")
            };
            let source = if reverse {
                format!("{owner}{declarations}")
            } else {
                format!("{declarations}{owner}")
            };
            with_checker(&source, |checker| {
                let produce = checker.functions["produce"].declaration;
                for name in ["No", "Yes"] {
                    let argument = checker
                        .types
                        .intern(TypeKind::Class(ClassType::new(name, vec![])));
                    checker
                        .class_instantiations
                        .insert(ClassType::new("Adapter", vec![argument]));
                    super::super::collect_concrete_instances(checker.program, checker);
                    let active = name == "Yes";
                    let int = checker.types.intern(TypeKind::Integer(IntegerType::Int64));
                    assert_eq!(
                        checker.callable_instances.iter().any(|instance| {
                            instance.declaration == produce
                                && instance.substitutions().get("U") == Some(&int)
                        }),
                        active,
                        "composed={composed}, reverse={reverse}, {name}: generic call"
                    );
                    assert_eq!(
                        checker
                            .class_instantiations
                            .contains(&ClassType::new("Box", vec![int])),
                        active,
                        "composed={composed}, reverse={reverse}, {name}: callee body"
                    );
                    assert_eq!(
                        checker
                            .class_instantiations
                            .contains(&ClassType::new("Local", vec![argument])),
                        active,
                        "composed={composed}, reverse={reverse}, {name}: caller body"
                    );
                    assert!(checker.diagnostics.is_empty(), "{:?}", checker.diagnostics);
                }
            });
        }
    }
}

#[test]
fn guarded_composed_dependencies_preserve_self_method_bindings_and_accessors() {
    let declarations = r#"
interface Ready {}
class Yes implements Ready {}
class No {}
class Box<T> {}
class SelfBox<T> {}
class GetterBox<T> {}
function produce<U>(take U $value): int { let $box = new Box<U>(); return 1; }
trait Wrapper<S> {
    function run<M>(take M $value): int where (S implements Ready) {
        let $self = new SelfBox<self>();
        return produce($value);
    }
    int $value where (S implements Ready) {
        get { let $box = new GetterBox<S>(); return produce(7); }
    }
}
trait Middle<V> { uses Wrapper<V> { Wrapper<V>::run as alias; } }
class Adapter<T> { uses Middle<T>; }
class Fixed { uses Middle<Yes>; }
trait Unused<S> {
    function run(): int where (S implements Ready) { return produce("unused"); }
}
"#;
    let callers = r#"
function inspect(Adapter<Yes> $adapter): int { return $adapter->alias(7); }
function inspectFixed(Fixed $adapter): int { return $adapter->alias(7); }
"#;
    for source in [
        format!("{declarations}{callers}"),
        format!("{callers}{declarations}"),
    ] {
        with_checker(&source, |checker| {
            let no = checker
                .types
                .intern(TypeKind::Class(ClassType::new("No", vec![])));
            checker
                .class_instantiations
                .insert(ClassType::new("Adapter", vec![no]));
            super::super::collect_concrete_instances(checker.program, checker);
            let yes = checker
                .types
                .intern(TypeKind::Class(ClassType::new("Yes", vec![])));
            let adapter = checker
                .types
                .intern(TypeKind::Class(ClassType::new("Adapter", vec![yes])));
            let fixed = checker
                .types
                .intern(TypeKind::Class(ClassType::new("Fixed", vec![])));
            let int = checker.types.intern(TypeKind::Integer(IntegerType::Int64));
            for (name, argument) in [
                ("Box", int),
                ("SelfBox", adapter),
                ("SelfBox", fixed),
                ("GetterBox", yes),
            ] {
                assert!(
                    checker
                        .class_instantiations
                        .contains(&ClassType::new(name, vec![argument])),
                    "missing {name}<{}>",
                    checker.types.display(argument)
                );
            }
            assert!(!checker
                .class_instantiations
                .contains(&ClassType::new("GetterBox", vec![no])));
            let string = checker.types.intern(TypeKind::String);
            assert!(!checker
                .class_instantiations
                .contains(&ClassType::new("Box", vec![string])));
            assert!(checker.diagnostics.is_empty(), "{:?}", checker.diagnostics);
        });
    }
}

#[test]
fn guarded_trait_calls_admit_concrete_member_dependencies() {
    for composed in [false, true] {
        for is_static in [false, true] {
            let modifier = if is_static { "static " } else { "" };
            let call = if is_static {
                "self::relay($value)"
            } else {
                "$this->relay($value)"
            };
            let condition = if composed {
                " where (S implements Ready)"
            } else {
                ""
            };
            let members = format!(
                "{modifier}function relay<N>(take N $value): int{condition} {{ \
                let $box = new Box<N>(); return 1; }} \
                {modifier}function run<M>(take M $value): int{condition} {{ return {call}; }}"
            );
            let owner = if composed {
                format!(
                    "trait Wrapper<S> {{ {members} }} class Active {{ uses Wrapper<Yes>; }} \
                    class Inactive {{ uses Wrapper<No>; }}"
                )
            } else {
                format!("class Active {{ {members} }} class Inactive {{}}")
            };
            let entry = if is_static {
                "function inspect(): int { return Active::run(7); }"
            } else {
                "function inspect(Active $adapter): int { return $adapter->run(7); }"
            };
            let declarations = format!("interface Ready {{}} class Yes implements Ready {{}} class No {{}} class Box<T> {{}} {owner}");
            for source in [
                format!("{declarations}{entry}"),
                format!("{entry}{declarations}"),
            ] {
                with_checker(&source, |checker| {
                    let relay = &checker.classes["Active"].methods["relay"];
                    let declaration = relay.declaration;
                    let parameter = relay.type_params[0].name.clone();
                    let int = checker.types.intern(TypeKind::Integer(IntegerType::Int64));
                    assert!(
                        checker.callable_instances.iter().any(|instance| {
                            instance.declaration == declaration
                                && instance.substitutions().get(&parameter) == Some(&int)
                        }),
                        "composed={composed}, static={is_static}: relay specialization"
                    );
                    assert!(
                        checker
                            .class_instantiations
                            .contains(&ClassType::new("Box", vec![int])),
                        "composed={composed}, static={is_static}: relay body dependency"
                    );
                    if composed {
                        let inactive = checker.classes["Inactive"].methods["relay"].declaration;
                        assert!(!checker
                            .callable_instances
                            .iter()
                            .any(|instance| instance.declaration == inactive));
                    }
                });
            }
        }
    }
}

#[test]
fn guarded_trait_recursion_keeps_finite_callable_ancestry() {
    for composed in [false, true] {
        for is_static in [false, true] {
            for expanding in [false, true] {
                let modifier = if is_static { "static " } else { "" };
                let argument = if expanding {
                    "new Box<U>($value)"
                } else {
                    "$value"
                };
                let call = if is_static {
                    format!("self::recur({argument})")
                } else {
                    format!("$this->recur({argument})")
                };
                let condition = if composed {
                    " where (S implements Ready)"
                } else {
                    ""
                };
                let method = format!("{modifier}function recur<U>(take U $value): int{condition} {{ return {call}; }}");
                let (owner, entry_name) = if composed {
                    (format!("trait Repeat<S> {{ {method} }} trait Middle<T> {{ uses Repeat<T> {{ Repeat<T>::recur as again; }} }} \
                        class Active {{ uses Middle<Yes>; }} class Inactive {{ uses Middle<No>; }}"), "again")
                } else {
                    (
                        format!("class Active {{ {method} }} class Inactive {{}}"),
                        "recur",
                    )
                };
                let entry = if is_static {
                    format!("function start(): int {{ return Active::{entry_name}(7); }}")
                } else {
                    format!("function start(Active $adapter): int {{ return $adapter->{entry_name}(7); }}")
                };
                let source = format!(
                    "interface Ready {{}} class Yes implements Ready {{}} class No {{}} \
                    class Box<T> {{ function __construct(take T $value) {{}} }} {owner} {entry}"
                );
                with_checker_diagnostics(&source, |checker| {
                    assert_eq!(
                        checker
                            .diagnostics
                            .iter()
                            .filter(|finding| finding.code == "E0539")
                            .count(),
                        usize::from(expanding),
                        "composed={composed}, static={is_static}, expanding={expanding}: {:?}",
                        checker.diagnostics
                    );
                    assert!(
                        checker
                            .diagnostics
                            .iter()
                            .all(|finding| finding.code == "E0539"),
                        "{:?}",
                        checker.diagnostics
                    );
                    if composed {
                        for method in checker.classes["Inactive"].methods.values() {
                            assert!(!checker
                                .callable_instances
                                .iter()
                                .any(|instance| instance.declaration == method.declaration));
                        }
                    }
                });
            }
        }
    }
}

#[test]
fn guarded_trait_return_queries_select_effective_methods_and_getters() {
    for generic_owner in [false, true] {
        for getter in [false, true] {
            let members = if getter {
                "Bytes $buffer = Bytes::allocate(1); \
                ByteRegion $view where (S implements Ready) { borrowed get { return $this->buffer->getRegion(0, 1); } } \
                function forward(): ByteRegion where (S implements Ready) { return $this->view; }"
            } else {
                "function relay<N>(N $value): N where (S implements Ready) { return $value; } \
                function forward<M>(M $value): M where (S implements Ready) { return $this->relay($value); }"
            };
            let (owner, receiver, name) = if generic_owner {
                ("trait Middle<V> { uses Wrapper<V> { Wrapper<V>::forward as alias; } } class Active<T> { uses Middle<T>; }", "Active<Yes>", "alias")
            } else {
                ("class Active { uses Wrapper<Yes>; }", "Active", "forward")
            };
            let entry = if getter {
                format!("function inspect({receiver} $adapter): ByteRegion {{ return $adapter->{name}(); }}")
            } else {
                format!(
                    "function inspect({receiver} $adapter): int {{ return $adapter->{name}(7); }}"
                )
            };
            let source = format!(
            "use Doria\\Std\\Io\\ByteRegion; interface Ready {{}} class Yes implements Ready {{}} \
            trait Wrapper<S> {{ {members} }} {owner} {entry}"
        );
            with_checker(&source, |checker| {
                let forward = checker.classes["Active"].methods[name].declaration;
                let instance = checker
                    .callable_instances
                    .iter()
                    .find(|instance| instance.declaration == forward)
                    .unwrap()
                    .clone();
                let bindings = instance
                    .bindings
                    .iter()
                    .map(|(name, ty)| (name.clone(), checker.types.resolved(*ty)))
                    .collect();
                let body = super::super::callable_contracts::BodyInstance::new(
                    checker.callable_body_program,
                    forward,
                    bindings,
                    &checker.composition,
                );
                let mut call = None;
                for item in &checker.callable_body_program().items {
                    if let Item::Trait(declaration) = item {
                        for member in &declaration.members {
                            if let ClassMember::Method(function) = member {
                                if function.span == body.declaration {
                                    crate::ast::visit::block(
                                        function.body.as_block().unwrap(),
                                        &mut |expression| {
                                            if matches!(
                                                expression,
                                                crate::ast::Expr::MethodCall { .. }
                                                    | crate::ast::Expr::PropertyAccess { .. }
                                            ) {
                                                call = Some(expression.clone());
                                            }
                                        },
                                    );
                                }
                            }
                        }
                    }
                }
                let call = call.expect("the forwarding body has a checked member call");
                let before = checker.call_targets.clone();
                let accessors_before = checker.property_accessor_calls.clone();
                let (selected_instance, selected) = checker
                    .return_call_instance(&call, |ty| body.substitute(ty))
                    .expect("an authored return query selects its concrete member");
                assert_eq!(checker.call_targets, before);
                assert_eq!(checker.property_accessor_calls, accessors_before);
                let expected = if getter {
                    checker.classes["Active"].properties["view"]
                        .hooks
                        .as_ref()
                        .unwrap()
                        .getter
                        .unwrap()
                        .declaration
                } else {
                    checker.classes["Active"].methods["relay"].declaration
                };
                assert_eq!(selected, expected, "getter={getter}");
                let mut expected_bindings = HashMap::new();
                if generic_owner {
                    expected_bindings.insert(
                        "T".into(),
                        ResolvedType::Class(ClassType::new("Yes", vec![])),
                    );
                }
                if !getter {
                    expected_bindings.insert(
                        checker.classes["Active"].methods["relay"].type_params[0]
                            .name
                            .clone(),
                        ResolvedType::Integer(IntegerType::Int64),
                    );
                }
                assert_eq!(
                    selected_instance,
                    super::super::specialized_ownership::Instance::new(
                        expected,
                        &expected_bindings
                    )
                );
            });
        }
    }
}

#[test]
fn guarded_composed_dependencies_select_constrained_methods_and_getters() {
    let source = r#"
interface Ready { function read(): int; int $value { get; } }
class Yes implements Ready {
    function read(): int { return 1; }
    int $value { get { return 2; } }
}
class No {}
trait Wrapper<S> {
    function run(S $source): int where (S implements Ready) {
        return $source->read() + $source->value;
    }
}
class Adapter<T> { uses Wrapper<T>; }
"#;
    with_checker(source, |checker| {
        for name in ["No", "Yes"] {
            let argument = checker
                .types
                .intern(TypeKind::Class(ClassType::new(name, vec![])));
            checker
                .class_instantiations
                .insert(ClassType::new("Adapter", vec![argument]));
            super::super::collect_concrete_instances(checker.program, checker);
            let selected = |target: &super::super::CallableTarget| {
                let super::super::CallableTarget::ConstrainedMethod {
                    implementations, ..
                } = target
                else {
                    return false;
                };
                implementations.iter().any(|implementation| {
                    implementation.receiver == ResolvedType::Class(ClassType::new("Yes", vec![]))
                        && implementation.declaring_class.name == "Yes"
                })
            };
            assert_eq!(
                checker
                    .call_targets
                    .values()
                    .filter(|target| selected(target))
                    .count(),
                usize::from(name == "Yes"),
                "{name}: method"
            );
            assert_eq!(
                checker
                    .property_accessor_calls
                    .values()
                    .filter_map(|calls| calls.getter.as_ref())
                    .filter(|getter| selected(&getter.target))
                    .count(),
                usize::from(name == "Yes"),
                "{name}: getter"
            );
            assert!(checker.diagnostics.is_empty(), "{:?}", checker.diagnostics);
        }
    });
}

#[test]
fn guarded_dependencies_keep_recursive_specialization_checks() {
    let declarations = r#"
interface Ready {}
class Yes implements Ready {}
class No {}
class Wrap<T> implements Ready {}
"#;
    for owner in [
        "class Owner<T> { function enabled(): ?Owner<Wrap<T>> where (T implements Ready) { return null; } }",
        "class Owner<T> { function enabled(): int where (T implements Ready) { let $next = new Owner<Wrap<T>>(); return 1; } }",
        "trait Wrapper<S> { function enabled(): int where (S implements Ready) { let $next = new Owner<Wrap<S>>(); return 1; } } class Owner<T> { uses Wrapper<T>; }",
    ] {
      for (name, expected_cycle) in [("No", false), ("Yes", true)] {
        with_checker(&format!("{declarations}{owner}"), |checker| {
            let argument = checker
                .types
                .intern(TypeKind::Class(ClassType::new(name, vec![])));
            checker
                .class_instantiations
                .insert(ClassType::new("Owner", vec![argument]));
            super::super::collect_concrete_instances(checker.program, checker);
            assert_eq!(
                checker
                    .diagnostics
                    .iter()
                    .any(|finding| finding.code == "E0539"),
                expected_cycle,
                "{name}: {:?}",
                checker.diagnostics
            );
            assert!(
                checker
                    .diagnostics
                    .iter()
                    .all(|finding| finding.code == "E0539"),
                "{name}: {:?}",
                checker.diagnostics
            );
        });
      }
    }
}

#[test]
fn conformance_cache_does_not_retain_a_callers_guard_premises() {
    with_checker(
        "interface Ready {} class Adapter<T> implements Iterable<int> where (T implements Ready) {}",
        |checker| {
            let int = checker.types.intern(TypeKind::Integer(IntegerType::Int64));
            let parameter = checker.types.intern(TypeKind::TypeParameter("T".into()));
            let class = ClassType::new("Adapter", vec![parameter]);
            let subject = checker.types.intern(TypeKind::Class(class.clone()));
            assert_eq!(checker.public_iterable_element(subject), None);
            let declared = checker.declared_class_conformance_cache.len();
            assert!(declared > 0);
            checker.nominal_condition_scopes.push(vec![
                super::super::nominal_proof::Step {
                    query: super::super::nominal_proof::Query {
                        subject: parameter,
                        interface: crate::types::InterfaceType::new("Ready", vec![]),
                    },
                    origin: None,
                },
            ]);
            assert_eq!(
                checker.public_iterable_element(subject),
                Some(int)
            );
            checker.nominal_condition_scopes.pop();
            assert_eq!(checker.public_iterable_element(subject), None);
            assert_eq!(checker.declared_class_conformance_cache.len(), declared);
            assert!(checker.diagnostics.is_empty(), "{:?}", checker.diagnostics);
            assert!(checker.take_conditional_declaration_contracts().conformances.is_empty());
        },
    );
}

#[test]
fn stored_type_checks_do_not_register_executable_specializations() {
    let source = "interface Ready {} interface Writable {} interface Root<E> {} \
        class Both implements Ready, Writable {} \
        class Adapter<T> implements Root<int> where (T implements Ready), Root<string> where (T implements Writable) {} \
        class Owner<T> { ?Adapter<T> $value = null; }";
    with_checker(source, |checker| {
        let both = checker
            .types
            .intern(TypeKind::Class(ClassType::new("Both", vec![])));
        let owner = checker
            .types
            .intern(TypeKind::Class(ClassType::new("Owner", vec![both])));
        let instances = checker.class_instantiations.clone();
        let templates = checker.nominal_instantiation_templates.clone();
        let callable_templates = checker.callable_nominal_instantiation_templates.clone();
        checker.defer_formed_type_conformances(owner);
        checker.check_pending_class_conformances();
        assert_eq!(checker.class_instantiations, instances);
        assert_eq!(checker.nominal_instantiation_templates, templates);
        assert_eq!(
            checker.callable_nominal_instantiation_templates,
            callable_templates
        );
        assert_eq!(checker.diagnostics.len(), 1, "{:?}", checker.diagnostics);
        assert_eq!(checker.diagnostics[0].code, "E0752");
        assert!(checker
            .take_conditional_declaration_contracts()
            .conformances
            .is_empty());
    });
}

#[test]
fn active_target_checks_do_not_register_executable_specializations() {
    let source = "interface Ready {} interface Writable {} interface Root<E> {} interface Uses<E> {} \
        class Both implements Ready, Writable {} \
        class Adapter<T> implements Root<int> where (T implements Ready), Root<string> where (T implements Writable) {} \
        class Owner<T> implements Uses<Adapter<T>> where (T implements Ready) {}";
    with_checker(source, |checker| {
        let both = checker
            .types
            .intern(TypeKind::Class(ClassType::new("Both", vec![])));
        let owner = checker
            .types
            .intern(TypeKind::Class(ClassType::new("Owner", vec![both])));
        let instances = checker.class_instantiations.clone();
        let templates = checker.nominal_instantiation_templates.clone();
        let callable_templates = checker.callable_nominal_instantiation_templates.clone();
        checker.defer_formed_type_conformances(owner);
        checker.check_pending_class_conformances();
        assert_eq!(checker.class_instantiations, instances);
        assert_eq!(checker.nominal_instantiation_templates, templates);
        assert_eq!(
            checker.callable_nominal_instantiation_templates,
            callable_templates
        );
        assert_eq!(checker.diagnostics.len(), 1, "{:?}", checker.diagnostics);
        assert_eq!(checker.diagnostics[0].code, "E0752");
        assert!(checker
            .take_conditional_declaration_contracts()
            .conformances
            .is_empty());
    });
}

#[test]
fn conditional_inherited_conformances_coalesce_all_active_overlap() {
    let declarations = r#"
interface Ready {}
interface Writable {}
interface Root<E> {}
class Both implements Ready, Writable {}
class ReadyOnly implements Ready {}
class WritableOnly implements Writable {}
class Neither {}
open class Base<T> implements Root<int> where (T implements Ready) {}
class Child<T> extends Base<T> implements Root<int> where (T implements Writable) {}
"#;
    for names in [
        ["Both", "ReadyOnly", "WritableOnly", "Neither"],
        ["Neither", "WritableOnly", "ReadyOnly", "Both"],
    ] {
        with_checker(declarations, |checker| {
            let int = checker.types.intern(TypeKind::Integer(IntegerType::Int64));
            for name in names {
                let argument = checker
                    .types
                    .intern(TypeKind::Class(ClassType::new(name, vec![])));
                let child = ClassType::new("Child", vec![argument]);
                let view = checker.class_interface_closure(&child, &mut HashSet::new());
                assert!(view.valid, "{name}: {:?}", checker.diagnostics);
                assert_eq!(view.entries.len(), usize::from(name != "Neither"), "{name}");
                if let Some((interface, _)) = view.entries.first() {
                    assert_eq!(interface.name, "Root");
                    assert_eq!(interface.arguments, [int]);
                }
            }
            assert!(checker.diagnostics.is_empty(), "{:?}", checker.diagnostics);
        });
    }
}

#[test]
fn unconditional_conformance_and_guarded_inheritance_share_one_active_entry() {
    for (base, child) in [
        ("where (T implements Ready)", ""),
        ("", "where (T implements Ready)"),
    ] {
        let source = format!(
            "interface Ready {{}} interface Root<E> {{}} class Yes implements Ready {{}} class No {{}} \
             open class Base<T> implements Root<int> {base} {{}} \
             class Child<T> extends Base<T> implements Root<int> {child} {{}}"
        );
        with_checker(&source, |checker| {
            for name in ["Yes", "No"] {
                let argument = checker
                    .types
                    .intern(TypeKind::Class(ClassType::new(name, vec![])));
                let child = ClassType::new("Child", vec![argument]);
                let view = checker.class_interface_closure(&child, &mut HashSet::new());
                assert!(view.valid, "{name}: {:?}", checker.diagnostics);
                assert_eq!(view.entries.len(), 1);
            }
            assert!(checker.diagnostics.is_empty(), "{:?}", checker.diagnostics);
        });
    }
}

#[test]
fn guarded_interface_diamonds_keep_one_specialized_ancestor() {
    for edges in [
        "Left<int> where (T implements Ready), Right<int> where (T implements Writable)",
        "Right<int> where (T implements Writable), Left<int> where (T implements Ready)",
    ] {
        let source = format!(
            "interface Ready {{}} interface Writable {{}} interface Root<E> {{}} \
             interface Left<E> extends Root<E> {{}} interface Right<E> extends Root<E> {{}} \
             class Both implements Ready, Writable {{}} class Adapter<T> implements {edges} {{}}"
        );
        with_checker(&source, |checker| {
            let both = checker
                .types
                .intern(TypeKind::Class(ClassType::new("Both", vec![])));
            let class = ClassType::new("Adapter", vec![both]);
            let view = checker.class_interface_closure(&class, &mut HashSet::new());
            assert!(view.valid, "{:?}", checker.diagnostics);
            assert_eq!(view.entries.len(), 3);
            assert_eq!(
                view.entries
                    .iter()
                    .filter(|(ty, _)| ty.name == "Root")
                    .count(),
                1
            );
            assert!(checker.diagnostics.is_empty(), "{:?}", checker.diagnostics);
        });
    }
}

#[test]
fn active_specialization_conflicts_have_no_iteration_element() {
    let source = r#"
interface Ready {}
interface Writable {}
interface Left<E> extends Iterable<E> {}
interface Right<E> extends Iterable<E> {}
class Both implements Ready, Writable {}
class ReadyOnly implements Ready {}
class WritableOnly implements Writable {}
class Neither {}
class Adapter<T> implements Left<int> where (T implements Ready), Right<string> where (T implements Writable) {}
"#;
    with_checker(source, |checker| {
        let int = checker.types.intern(TypeKind::Integer(IntegerType::Int64));
        let string = checker.types.intern(TypeKind::String);
        for (name, element) in [
            ("Neither", None),
            ("ReadyOnly", Some(int)),
            ("WritableOnly", Some(string)),
            ("Both", None),
        ] {
            let argument = checker
                .types
                .intern(TypeKind::Class(ClassType::new(name, vec![])));
            let class = ClassType::new("Adapter", vec![argument]);
            let subject = checker.types.intern(TypeKind::Class(class.clone()));
            let view = checker.class_interface_closure(&class, &mut HashSet::new());
            assert_eq!(
                view.valid,
                name != "Both",
                "{name}: {:?}",
                checker.diagnostics
            );
            assert_eq!(checker.public_iterable_element(subject), element, "{name}");
        }
        let conflicts = checker
            .diagnostics
            .iter()
            .filter(|finding| finding.code == "E0752")
            .collect::<Vec<_>>();
        assert_eq!(conflicts.len(), 1, "{:?}", checker.diagnostics);
        assert_eq!(
            &source[conflicts[0].span.start..conflicts[0].span.end],
            "Right<string>"
        );
        for guard in ["Ready", "Writable"] {
            assert!(conflicts[0]
                .related
                .iter()
                .any(|related| &source[related.span.start..related.span.end] == guard));
        }
    });
}

#[test]
fn inactive_source_duplicate_conformances_are_still_invalid() {
    let source = "interface Ready {} interface Writable {} interface Root<E> {} \
                  class Neither {} class Adapter<T> implements Root<int> where (T implements Ready), Root<int> where (T implements Writable) {}";
    with_checker_diagnostics(source, |checker| {
        assert!(checker
            .diagnostics
            .iter()
            .any(|finding| finding.code == "E0752"));
        let neither = checker
            .types
            .intern(TypeKind::Class(ClassType::new("Neither", vec![])));
        let class = ClassType::new("Adapter", vec![neither]);
        for _ in 0..2 {
            let view = checker.class_interface_closure(&class, &mut HashSet::new());
            assert!(!view.valid);
            assert!(view.entries.is_empty());
        }
        assert_eq!(
            checker
                .diagnostics
                .iter()
                .filter(|finding| finding.code == "E0752")
                .count(),
            1
        );
    });
}

#[test]
fn authored_conformance_identity_survives_equal_concrete_substitution() {
    let source = "interface Ready {} interface Root<E> {} class Yes implements Ready {} \
                  class Other implements Ready {} class Adapter<T, U> implements Root<T> where (T implements Ready), Root<U> where (U implements Ready) {}";
    with_checker(source, |checker| {
        let yes = checker
            .types
            .intern(TypeKind::Class(ClassType::new("Yes", vec![])));
        let other = checker
            .types
            .intern(TypeKind::Class(ClassType::new("Other", vec![])));
        for (right, valid) in [(yes, true), (other, false), (yes, true)] {
            let class = ClassType::new("Adapter", vec![yes, right]);
            let view = checker.class_interface_closure(&class, &mut HashSet::new());
            assert_eq!(view.valid, valid, "{:?}", checker.diagnostics);
            assert_eq!(view.entries.len(), 1);
        }
        assert_eq!(
            checker
                .diagnostics
                .iter()
                .filter(|finding| finding.code == "E0752")
                .count(),
            1
        );
        assert!(checker
            .take_conditional_declaration_contracts()
            .conformances
            .is_empty());
    });
}

#[test]
fn composed_method_loans_preserve_parameter_roots_and_copy_specialization() {
    let declarations = r#"
interface Ready {}
trait Borrow<T> {
    function keep(T $value): T where (T implements Ready) { return $value; }
}
trait Middle<U> { uses Borrow<U> { Borrow<U>::keep as alias; } }
class Adapter<V> { uses Middle<V>; }
class CopyAdapter { uses Borrow<int>; }
"#;
    let callers = r#"
function keep<T implements Ready>(Adapter<T> $adapter, T $value): T {
    return $adapter->alias($value);
}
"#;
    for source in [
        format!("{declarations}{callers}"),
        format!("{callers}{declarations}"),
    ] {
        with_checker(&source, |checker| {
            for name in ["keep", "alias"] {
                let method = &checker.classes["Adapter"].methods[name];
                let borrow = method.return_borrow.expect("composed parameter loan");
                assert_eq!(borrow.source, BorrowSource::Parameter(0));
                assert_eq!(borrow.kind, ReturnBorrowKind::Value);
                assert_eq!(
                    checker.function_signatures[&method.declaration].return_borrow,
                    Some(borrow)
                );
            }
            assert_eq!(
                checker.classes["CopyAdapter"].methods["keep"].return_borrow,
                None
            );
            assert_eq!(
                checker.functions["keep"].return_borrow.unwrap().source,
                BorrowSource::Parameter(1)
            );
        });
    }
}

#[test]
fn guarded_generic_calls_refine_copy_and_region_return_contracts() {
    for composed in [false, true] {
        for take in ["", "take "] {
            for (body, closure) in [
                ("return $value;".to_string(), false),
                ("return $this->relay($value);".to_string(), false),
                ("return self::relayStatic($value);".to_string(), false),
                ("return $this->relay(value: $value);".to_string(), false),
                (
                    "return self::relayStatic(value: $value);".to_string(),
                    false,
                ),
                (
                    format!(
                        "let $identity = fn({take}U $item) => $item; return $identity($value);"
                    ),
                    true,
                ),
            ] {
                let member =
                format!("function forward<U>({take}U $value): U where (T implements Ready) {{ {body} }}");
                let helpers = format!("function relay<U>({take}U $value): U where (T implements Ready) {{ return $value; }} \
                    static function relayStatic<U>({take}U $value): U where (T implements Ready) {{ return $value; }}");
                let owner = if composed {
                    format!("trait Forward<T> {{ {helpers} {member} }} trait Middle<M> {{ uses Forward<M> {{ Forward<M>::forward as alias; }} }} class Adapter<V> {{ uses Middle<V>; }}")
                } else {
                    format!("class Adapter<T> {{ {helpers} {member} function alias<U>({take}U $value): U where (T implements Ready) {{ {body} }} }}")
                };
                let declarations = format!("use Doria\\Std\\Io\\ByteRegion; interface Ready {{}} class Yes implements Ready {{}} class Item {{}} {owner}");
                let callers = ["forward", "alias"].into_iter().flat_map(|method| [
                format!("function {method}Integer(Adapter<Yes> $adapter, int $value): int {{ return $adapter->{method}($value); }}"),
                format!("function {method}Region(Adapter<Yes> $adapter, ByteRegion $value): ByteRegion {{ return $adapter->{method}($value); }}"),
                format!("function {method}Item(take Adapter<Yes> $adapter, {take}Item $value): Item {{ return $adapter->{method}($value); }}"),
            ]).collect::<String>();
                for source in [
                    format!("{declarations}{callers}"),
                    format!("{callers}{declarations}"),
                ] {
                    with_checker(&source, |checker| {
                        let ownership = ownership_facts(checker);
                        for name in ["forward", "alias"] {
                            let declaration = checker.classes["Adapter"].methods[name].declaration;
                            let parameter =
                                &checker.function_signatures[&declaration].type_params[0].name;
                            let instances = checker
                                .callable_instances
                                .iter()
                                .filter(|instance| instance.declaration == declaration)
                                .collect::<Vec<_>>();
                            assert_eq!(instances.len(), 3);
                            for instance in instances {
                                let bindings = instance
                                    .bindings
                                    .iter()
                                    .map(|(name, ty)| (name.clone(), checker.types.resolved(*ty)))
                                    .collect::<HashMap<_, _>>();
                                let key = super::super::specialized_ownership::Instance::new(
                                    declaration,
                                    &bindings,
                                );
                                let facts = ownership
                                    .get(&key)
                                    .expect("every concrete body is ownership checked");
                                if closure {
                                    assert_ne!(
                                        facts.as_ref(),
                                        &super::super::specialized_ownership::Facts::default()
                                    );
                                }
                                if composed {
                                    let body = super::super::callable_contracts::BodyInstance::new(
                                        checker.callable_body_program,
                                        declaration,
                                        bindings.clone(),
                                        &checker.composition,
                                    );
                                    assert_eq!(
                                        body.declaration,
                                        checker.composition.callable_templates[&declaration]
                                            .declaration
                                    );
                                    assert_eq!(
                                        body.substitute(&ResolvedType::TypeParameter("U".into())),
                                        bindings[parameter]
                                    );
                                    assert_eq!(
                                        body.substitute(&ResolvedType::TypeParameter("T".into())),
                                        ResolvedType::Class(ClassType::new("Yes", vec![]))
                                    );
                                    assert_eq!(
                                        body.substitute(&ResolvedType::TraitSelf("Forward".into())),
                                        ResolvedType::Class(ClassType::new(
                                            "Adapter",
                                            vec![ResolvedType::Class(ClassType::new(
                                                "Yes",
                                                vec![]
                                            ))]
                                        ))
                                    );
                                }
                                let returned = super::super::return_specialization::lookup(
                                    &checker.specialized_return_borrow_seed,
                                    declaration,
                                    &bindings,
                                );
                                match &bindings[parameter] {
                                    ResolvedType::Integer(_) => {
                                        assert_eq!(returned, take.is_empty().then_some(None));
                                    }
                                    ResolvedType::ByteRegion(_) => {
                                        let borrow = returned.flatten().expect(
                                            "a concrete region retains its byte-storage source",
                                        );
                                        assert_eq!(borrow.source, BorrowSource::Parameter(0));
                                        assert_eq!(borrow.kind, ReturnBorrowKind::Retained);
                                        assert!(!borrow.writable);
                                    }
                                    ResolvedType::Class(class) if class.name == "Item" => {}
                                    ty => panic!("unexpected specialization: {ty:?}"),
                                }
                            }
                        }
                        for name in ["forward", "alias"] {
                            assert_eq!(
                                checker.functions[&format!("{name}Integer")].return_borrow,
                                None
                            );
                            let returned = checker.functions[&format!("{name}Region")]
                                .return_borrow
                                .unwrap();
                            assert_eq!(returned.source, BorrowSource::Parameter(1));
                            assert_eq!(returned.kind, ReturnBorrowKind::Retained);
                            let item = checker.functions[&format!("{name}Item")].return_borrow;
                            if take.is_empty() {
                                let item = item
                                    .expect("the only borrowed parameter supplies the Move result");
                                assert_eq!(item.source, BorrowSource::Parameter(1));
                                assert_eq!(item.kind, ReturnBorrowKind::Value);
                            } else {
                                assert_eq!(item, None);
                            }
                        }
                    });
                }
            }
        }
    }
}

#[test]
fn guarded_copy_erasure_preserves_snapshots_and_rejects_temporary_loans() {
    for composed in [false, true] {
        for (body, copy_error) in [
            ("return $stream;", None),
            ("return erase($stream);", None),
            ("let $owner = erase($stream); return $owner;", None),
            (
                "let $callback = function(T $value): ReadinessStream { return $value; }; return $callback($stream);",
                None,
            ),
            ("return borrowed($stream);", Some("E0478")),
            ("return borrowed(erase($stream));", Some("E0478")),
            (
                "let $callback = function(T $value): ReadinessStream { return borrowed($value); }; return $callback($stream);",
                Some("E0478"),
            ),
            (
                "let $owner = erase($stream); let $alias = borrowed($owner); return $alias;",
                Some("E0474"),
            ),
        ] {
            let method = format!(
                "function relay<T implements ReadinessStream>(T $stream): ReadinessStream where (S implements Ready) {{ {body} }}"
            );
            let (owner, method_name) = if composed {
                (
                    format!("trait Projector<S> {{ {method} }} trait Middle<U> {{ uses Projector<U> {{ Projector<U>::relay as forward; }} }} class Adapter<V> {{ uses Middle<V>; }}"),
                    "forward",
                )
            } else {
                (format!("class Adapter<S> {{ {method} }}"), "relay")
            };
            let declarations = format!(
                r#"
use Doria\Std\Io\{{StandardStreams, ReadinessStream}};
interface Ready {{}}
class Yes implements Ready {{}}
function borrowed(ReadinessStream $stream): ReadinessStream {{ return $stream; }}
function erase<T implements ReadinessStream>(T $stream): ReadinessStream {{ return $stream; }}
function consume(take ReadinessStream $stream): void {{}}
{owner}
"#
            );
            let move_caller = format!(
                r#"
function inspectMove(Adapter<Yes> $adapter, ReadinessStream $stream): void {{
    let $moveAlias = $adapter->{method_name}($stream);
    let $source = $moveAlias->readinessSource;
}}
"#
            );
            with_checker(&format!("{declarations}{move_caller}"), |checker| {
                assert!(checker.contracts.interface_specializations.iter().any(|graph| {
                    graph.valid
                        && graph.specialization.name == "Doria\\Std\\Io\\ReadinessStream"
                }));
                let alias = checker
                    .binding_resolution
                    .declarations_by_id
                    .values()
                    .find(|binding| binding.name == "moveAlias")
                    .unwrap();
                assert_eq!(
                    alias.ownership,
                    crate::symbols::BindingOwnership::ReadonlyBorrow,
                    "composed={composed}, body={body}"
                );
            });
            let copy_caller = format!(
                r#"
function inspectCopy(Adapter<Yes> $adapter): void {{
    let writable $input = StandardStreams::input;
    let $copyOwner = $adapter->{method_name}($input);
    $input = StandardStreams::input;
    consume($copyOwner);
}}
"#
            );
            let source = format!("{declarations}{move_caller}{copy_caller}");
            with_checker_diagnostics(&source, |checker| {
                if let Some(code) = copy_error {
                    assert!(
                        checker.diagnostics.iter().any(|diagnostic| diagnostic.code == code),
                        "composed={composed}, body={body}: {:?}",
                        checker.diagnostics
                    );
                    assert!(
                        checker.diagnostics.iter().all(|diagnostic| {
                            !diagnostic.code.starts_with('I') && !diagnostic.code.starts_with('B')
                        }),
                        "{:?}",
                        checker.diagnostics
                    );
                } else {
                    assert!(checker.diagnostics.is_empty(), "{:?}", checker.diagnostics);
                    let owner = checker
                        .binding_resolution
                        .declarations_by_id
                        .values()
                        .find(|binding| binding.name == "copyOwner")
                        .unwrap();
                    assert_eq!(owner.ownership, crate::symbols::BindingOwnership::Owned);
                }
            });
        }
    }
}

#[test]
fn guarded_erased_iterator_returns_preserve_cleanup_source_loans() {
    for composed in [false, true] {
        for erased in [false, true] {
            for body in [
                "return $source;",
                "return erase($source);",
                "let $identity = function(take T $input): Iterator<int> { return $input; }; return $identity($source);",
            ] {
                let method = format!(
                    "function relay<T implements Iterator<int>>(take T $source): Iterator<int> where (S implements Ready) {{ {body} }}"
                );
                let (owner, method_name) = if composed {
                    (
                        format!("trait Relay<S> {{ {method} }} trait Middle<U> {{ uses Relay<U> {{ Relay<U>::relay as forward; }} }} class Adapter<V> {{ uses Middle<V>; }}"),
                        "forward",
                    )
                } else {
                    (format!("class Adapter<S> {{ {method} }}"), "relay")
                };
                let declarations = format!(
                    r#"
interface Ready {{}}
class Yes implements Ready {{}}
class Cursor implements Iterator<int> {{
    function __construct(borrow List<int> $source) {{}}
    function hasCurrent(): bool {{ return true; }}
    function getCurrent(): int {{ return $this->source[0]; }}
    writable function advance(): void {{}}
}}
function erase<U implements Iterator<int>>(take U $source): Iterator<int> {{ return $source; }}
{owner}
"#
                );
                let input = if erased {
                    "Iterator<int> $input = new Cursor($values);"
                } else {
                    "let $input = new Cursor($values);"
                };
                let caller = format!(
                    r#"
function inspect(Adapter<Yes> $adapter): void {{
    writable List<int> $values = [7];
    {{
        {input}
        let $view = $adapter->{method_name}($input);
        let $current = $view->getCurrent();
    }}
    $values->add(8);
}}
"#
                );
                with_checker(&format!("{declarations}{caller}"), |checker| {
                    let sources = checker.retained_source_seed.as_ref().unwrap();
                    assert_eq!(
                        sources.lifetime(&ResolvedType::Interface(crate::types::InterfaceType::new(
                            "Iterator",
                            vec![ResolvedType::Integer(IntegerType::Int64)],
                        ))),
                        Some(crate::types::RetainedLifetime::Cleanup)
                    );
                    let view = checker
                        .binding_resolution
                        .declarations_by_id
                        .values()
                        .find(|binding| binding.name == "view")
                        .unwrap();
                    assert_eq!(view.ownership, crate::symbols::BindingOwnership::Owned);
                });
                for replacement in [
                    "$values->add(9); let $current = $view->getCurrent();",
                    "let $current = $view->getCurrent(); $values->add(9);",
                ] {
                    let invalid = caller.replace("let $current = $view->getCurrent();", replacement);
                    with_checker_diagnostics(&format!("{declarations}{invalid}"), |checker| {
                        assert!(
                            checker.diagnostics.iter().any(|diagnostic| diagnostic.code == "E0763"),
                            "composed={composed}, erased={erased}, body={body}, use={replacement}: {:?}",
                            checker.diagnostics
                        );
                        assert!(checker.diagnostics.iter().all(|diagnostic| {
                            !diagnostic.code.starts_with('I') && !diagnostic.code.starts_with('B')
                        }));
                    });
                }
                let escaping = format!(
                    r#"
function escape(Adapter<Yes> $adapter): Iterator<int> {{
    writable List<int> $values = [7];
    {input}
    return $adapter->{method_name}($input);
}}
"#
                );
                with_checker_diagnostics(&format!("{declarations}{escaping}"), |checker| {
                    assert!(
                        checker.diagnostics.iter().any(|diagnostic| diagnostic.code == "E0762"),
                        "composed={composed}, erased={erased}, body={body}: {:?}",
                        checker.diagnostics
                    );
                });
            }
        }
    }
}

#[test]
fn concrete_composed_bodies_keep_authored_specialization_without_instance_arguments() {
    for take in ["", "take "] {
        let source = format!(
            r#"
use Doria\Std\Io\ByteRegion;
interface Ready {{}}
class Yes implements Ready {{}}
class No {{}}
class Item {{}}
trait Forward<S, T> {{
    function forward({take}T $value): T where (S implements Ready) {{
        let $identity = fn({take}T $item) => $item;
        return $identity($value);
    }}
}}
trait Middle<A, B> {{ uses Forward<A, B> {{ Forward<A, B>::forward as alias; }} }}
class IntegerAdapter {{ uses Middle<Yes, int>; }}
class ItemAdapter {{ uses Middle<Yes, Item>; }}
class RegionAdapter {{ uses Middle<Yes, ByteRegion>; }}
class InactiveAdapter {{ uses Middle<No, int>; }}
function integer(IntegerAdapter $adapter, int $value): int {{ return $adapter->alias($value); }}
function item(take ItemAdapter $adapter, {take}Item $value): Item {{ return $adapter->alias($value); }}
function region(RegionAdapter $adapter, ByteRegion $value): ByteRegion {{ return $adapter->alias($value); }}
"#
        );
        with_checker(&source, |checker| {
            let ownership = ownership_facts(checker);
            let bindings = HashMap::new();
            for owner in [
                "IntegerAdapter",
                "ItemAdapter",
                "RegionAdapter",
                "InactiveAdapter",
            ] {
                for name in ["forward", "alias"] {
                    let declaration = checker.classes[owner].methods[name].declaration;
                    let instances = checker
                        .callable_instances
                        .iter()
                        .filter(|instance| instance.declaration == declaration)
                        .collect::<Vec<_>>();
                    let key =
                        super::super::specialized_ownership::Instance::new(declaration, &bindings);
                    if owner == "InactiveAdapter" {
                        assert!(instances.is_empty());
                        assert!(!ownership.contains_key(&key));
                        continue;
                    }
                    assert_eq!(instances.len(), 1);
                    assert!(instances[0].bindings.is_empty());
                    let facts = ownership
                        .get(&key)
                        .expect("a fixed trait substitution is still ownership checked");
                    assert_ne!(
                        facts.as_ref(),
                        &super::super::specialized_ownership::Facts::default()
                    );
                    let returned = super::super::return_specialization::lookup(
                        &checker.specialized_return_borrow_seed,
                        declaration,
                        &bindings,
                    );
                    if owner == "IntegerAdapter" {
                        assert_eq!(returned, take.is_empty().then_some(None));
                    } else if owner == "RegionAdapter" {
                        let returned = returned.flatten().expect("fixed region storage loan");
                        assert_eq!(returned.source, BorrowSource::Parameter(0));
                        assert_eq!(returned.kind, ReturnBorrowKind::Retained);
                    }
                }
            }
            let region = checker.functions["region"]
                .return_borrow
                .expect("forwarded region storage loan");
            assert_eq!(region.source, BorrowSource::Parameter(1));
            assert_eq!(region.kind, ReturnBorrowKind::Retained);
        });
    }
}

#[test]
fn guarded_callback_contracts_keep_known_pure_and_io_effects_distinct() {
    for (callback, pure) in [
        ("function(): void {}", true),
        ("function(): void { echo \"value\"; }", false),
    ] {
        for composed in [false, true] {
            let body = format!(
                "function invoke(): void where (T implements Ready) {{ let $call = {callback}; $call(); }}"
            );
            let declaration = if composed {
                format!("trait Invoke<T> {{ {body} }} class Adapter<U> {{ uses Invoke<U>; }}")
            } else {
                format!("class Adapter<T> {{ {body} }}")
            };
            let source = format!("interface Ready {{}} {declaration}");
            with_checker(&source, |checker| {
                assert!(
                    !checker.callback_ambient_effect_seed.is_empty(),
                    "{source}: calls {:?}, closures {:?}",
                    checker.callable_value_calls,
                    checker.closures,
                );
                for (site, effects) in &checker.callback_ambient_effect_seed {
                    assert!(checker.callable_value_calls.contains_key(site));
                    assert_eq!(effects.is_empty(), pure, "{source}: {effects:?}");
                }
                assert_eq!(
                    checker.classes["Adapter"].methods["invoke"]
                        .checked_effects
                        .is_empty(),
                    pure,
                );
                assert!(checker.retained_source_seed.is_some());
            });
        }
    }
}

#[test]
fn guarded_programs_refine_concrete_retained_storage_without_tainting_copy_storage() {
    with_checker(
        r#"
use Doria\Std\Io\ByteRegion;
interface Ready {}
class Holder<T> {
    function __construct(take T $value) {}
    function ready(): void where (T implements Ready) {}
}
function region(Holder<ByteRegion> $value): void {}
function integer(Holder<int> $value): void {}
"#,
        |checker| {
            let sources = checker.retained_source_seed.as_ref().unwrap();
            assert_eq!(
                sources.lifetime(&ResolvedType::Class(ClassType::new(
                    "Holder",
                    vec![ResolvedType::ByteRegion(
                        crate::types::ByteRegionKind::Readonly
                    )],
                ))),
                Some(crate::types::RetainedLifetime::Cleanup),
            );
            assert_eq!(
                sources.lifetime(&ResolvedType::Class(ClassType::new(
                    "Holder",
                    vec![ResolvedType::Integer(IntegerType::Int64)],
                ))),
                None,
            );
        },
    );
}

#[test]
fn guarded_owned_callbacks_preserve_concrete_cleanup_loans() {
    for composed in [false, true] {
        let method = r#"
    function relay<T>(take T $value): T where (S implements Ready) {
        let $identity = fn(take T $item) => $item;
        return $identity($value);
    }
"#;
        let owner = if composed {
            format!("trait Relay<S> {{ {method} }} trait Middle<U> {{ uses Relay<U> {{ Relay<U>::relay as forward; }} }} class Adapter<V> {{ uses Middle<V>; }}")
        } else {
            format!("class Adapter<S> {{ {method} }}")
        };
        let relay = if composed { "forward" } else { "relay" };
        let declarations = format!(
            r#"
interface Ready {{}}
class Yes implements Ready {{}}
class Item {{ function __construct(int $value) {{}} }}
class Envelope<T> {{ function __construct(take T $value) {{}} }}
class Cursor implements Iterator<int> {{
    function __construct(borrow List<int> $source) {{}}
    function hasCurrent(): bool {{ return true; }}
    function getCurrent(): int {{ return $this->source[0]; }}
    writable function advance(): void {{}}
}}
{owner}
"#
        );
        let caller = format!(
            r#"
function inspect(Adapter<Yes> $adapter): void {{
    echo $adapter->{relay}(42);
    echo $adapter->{relay}(new Item(3))->value;
    writable List<int> $source = [7];
    {{
        let $cursor = $adapter->{relay}(new Cursor($source));
        echo $cursor->getCurrent();
        let $owner = $adapter->{relay}(new Envelope<Cursor>(new Cursor($source)));
        echo $owner->value->getCurrent();
        let $cursors = $adapter->{relay}([new Cursor($source)]);
        echo $cursors[0]->getCurrent();
    }}
    $source->add(8);
}}
"#
        );
        for source in [
            format!("{declarations}{caller}"),
            format!("{caller}{declarations}"),
        ] {
            crate::check_source(
                "ordinary-owned-callback.doria",
                source.replace(" where (S implements Ready)", ""),
            )
            .unwrap();
            with_checker(&source, |checker| {
                let facts = ownership_facts(checker);
                let declaration = checker.classes["Adapter"].methods[relay].declaration;
                let instances = checker
                    .callable_instances
                    .iter()
                    .filter(|instance| instance.declaration == declaration)
                    .collect::<Vec<_>>();
                assert_eq!(
                    instances.len(),
                    5,
                    "all concrete callback payloads are checked"
                );
                for instance in instances {
                    let bindings = instance
                        .bindings
                        .iter()
                        .map(|(name, ty)| (name.clone(), checker.types.resolved(*ty)))
                        .collect();
                    assert!(facts.contains_key(
                        &super::super::specialized_ownership::Instance::new(declaration, &bindings)
                    ));
                }
            });
        }
        for use_site in [
            "echo $cursor->getCurrent();",
            "echo $owner->value->getCurrent();",
            "echo $cursors[0]->getCurrent();",
        ] {
            let invalid = format!(
                "{declarations}{}",
                caller.replace(use_site, &format!("{use_site} $source->add(9);"))
            );
            let ordinary = crate::check_source(
                "ordinary-callback-mutation.doria",
                invalid.replace(" where (S implements Ready)", ""),
            )
            .unwrap_err();
            assert!(ordinary.iter().any(|diagnostic| diagnostic.code == "E0763"));
            with_checker_diagnostics(&invalid, |checker| {
                let (_, analysis) = analyze_ownership(checker);
                assert!(
                    checker
                        .diagnostics
                        .iter()
                        .any(|diagnostic| diagnostic.code == "E0763"),
                    "composed={composed}, use={use_site}: a live concrete retained owner must protect its source: {:?}; summaries={:?}",
                    checker.diagnostics,
                    analysis.retained_callables
                );
            });
        }
        let escaping = format!(
            r#"{declarations}
function escape(Adapter<Yes> $adapter): Cursor {{
    let $source = [7];
    return $adapter->{relay}(new Cursor($source));
}}
"#
        );
        with_checker_diagnostics(&escaping, |checker| {
            assert!(
                !checker.diagnostics.is_empty(),
                "a concrete cleanup loan cannot escape its local source"
            );
        });
    }
}

#[test]
fn composed_retained_summaries_keep_each_receivers_physical_property_type() {
    let source = r#"
interface Ready {}
class Yes implements Ready {}
open class Counter {}
class Child extends Counter {}
trait Slot<S, T> {
    writable T $stored;
    T $current where (S implements Ready) { writable borrowed get => $this->stored; }
}
class Narrow {
    uses Slot<Yes, Child>;
    function __construct(parameter take Child $value) { $this->stored = $value; }
}
class Wide {
    uses Slot<Yes, Counter>;
    function __construct(parameter take Counter $value) { $this->stored = $value; }
}
"#;
    with_checker(source, |checker| {
        let (_, analysis) = analyze_ownership(checker);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
        for (owner, stored) in [("Narrow", "Child"), ("Wide", "Counter")] {
            let getter = checker.classes[owner].properties["current"]
                .hooks
                .as_ref()
                .unwrap()
                .getter
                .unwrap()
                .declaration;
            let summary = &analysis.retained_callables[&getter];
            assert_eq!(
                summary.writeback.returns,
                vec![crate::ownership::writeback::Home::Typed(
                    ResolvedType::Class(ClassType::new(stored, vec![]))
                )],
                "{owner}"
            );
        }
    });
}

#[test]
fn composed_recursive_callback_writeback_preserves_the_callers_storage_type() {
    let declarations = r#"
interface Ready {}
class Yes implements Ready {}
class Counter {
    writable int $value = 1;
    writable function bump(): void { $this->value++; }
}
function lend(writable Counter $value): ?Counter { return $value; }
trait Apply<S> {
    function apply<T>(function(writable ?T): void $callback, writable ?T $value): void where (S implements Ready) {
        $callback($value);
    }
}
trait Forward<U> {
    uses Apply<U> { Apply<U>::apply as routed; }
    function forward<T>(function(writable ?T): void $callback, writable ?T $value, int $depth): void where (U implements Ready) {
        if ($depth > 0) { $this->forward($callback, $value, $depth - 1); }
        else { $this->routed($callback, $value); }
    }
}
class Adapter<V> { uses Forward<V>; }
"#;
    for (replacement, rejected) in [
        ("$value?->bump();", false),
        ("$value = new Counter();", false),
        ("$value = null;", true),
    ] {
        let caller = format!(
            r#"
function inspect(Adapter<Yes> $adapter): void {{
    let writable $owner = new Counter();
    let writable $view = lend($owner);
    let $callback = function(writable ?Counter $value): void {{ {replacement} }};
    $adapter->forward($callback, $view, 2);
}}
"#
        );
        for source in [
            format!("{declarations}{caller}"),
            format!("{caller}{declarations}"),
        ] {
            let ordinary = crate::check_source(
                "ordinary-recursive-writeback.doria",
                source
                    .replace(" where (S implements Ready)", "")
                    .replace(" where (U implements Ready)", ""),
            );
            if rejected {
                assert!(ordinary
                    .unwrap_err()
                    .iter()
                    .any(|diagnostic| diagnostic.code == "E0403"));
            } else {
                ordinary.unwrap();
            }
            with_checker_diagnostics(&source, |checker| {
                if rejected {
                    assert!(
                        checker
                            .diagnostics
                            .iter()
                            .any(|diagnostic| diagnostic.code == "E0403"),
                        "{replacement}: {:?}",
                        checker.diagnostics
                    );
                } else {
                    assert!(
                        checker.diagnostics.is_empty(),
                        "{replacement}: {:?}",
                        checker.diagnostics
                    );
                }
            });
        }
    }
}

#[test]
fn guarded_generic_callable_getters_preserve_retained_receiver_sources() {
    for composed in [false, true] {
        let method = r#"
    function relay<T implements Factory<function(): int>>(T $source): function(): int where (S implements Ready) {
        return $source->callback;
    }
"#;
        let owner = if composed {
            format!("trait Relay<S> {{ {method} }} trait Middle<U> {{ uses Relay<U> {{ Relay<U>::relay as forward; }} }} class Adapter<V> {{ uses Middle<V>; }}")
        } else {
            format!("class Adapter<S> {{ {method} }}")
        };
        let relay = if composed { "forward" } else { "relay" };
        let declarations = format!(
            r#"
interface Ready {{}}
class Yes implements Ready {{}}
interface Factory<T> {{ T $callback {{ get; }} }}
class Source implements Factory<function(): int> {{
    int $value = 42;
    function(): int $callback {{ get {{ return fn() with ($this) => $this->value; }} }}
}}
{owner}
"#
        );
        let callers = format!(
            r#"
function forward(Adapter<Yes> $adapter, Source $source): function(): int {{
    return $adapter->{relay}($source);
}}
"#
        );
        for source in [
            format!("{declarations}{callers}"),
            format!("{callers}{declarations}"),
        ] {
            with_checker(&source, |checker| {
                ownership_facts(checker);
                let borrow = checker.functions["forward"]
                    .return_borrow
                    .expect("fresh callback retains its receiver source");
                assert_eq!(borrow.source, BorrowSource::Parameter(1));
                assert_eq!(borrow.kind, ReturnBorrowKind::Retained);
                assert_eq!(borrow.lifetime, crate::types::RetainedLifetime::Cleanup);
            });
        }
        let escaping = format!(
            r#"{declarations}
function escape(Adapter<Yes> $adapter): function(): int {{
    let $source = new Source();
    return $adapter->{relay}($source);
}}
"#
        );
        with_checker_diagnostics(&escaping, |checker| {
            assert!(
                checker
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == "E0658"),
                "a returned closure cannot outlive its concrete receiver: {:?}",
                checker.diagnostics
            );
        });
    }
}

#[test]
fn guarded_programs_share_ordinary_hierarchy_contract_checks() {
    let guard = "interface Ready {} class Adapter<T> { function guarded(): void where (T implements Ready) {} }";
    for (source, code) in [
        ("open class Root { function __construct(int $value) {} } class Child extends Root { function __construct() {} }", "E0732"),
        ("open class Root { function __construct(int $value) {} } class Child extends Root { function __construct() { let $value = 1; parent::__construct($value); } }", "E0733"),
        ("open class Root { int $value = 42; open function callback(): function(): int { return fn() => 42; } } class Child extends Root { override function callback(): function(): int { return fn() with ($this) => $this->value; } }", "E0729"),
    ] {
        let ordinary = crate::check_source("hierarchy-control.doria", source).expect_err("ordinary hierarchy control must reject this contract");
        assert!(ordinary.iter().any(|diagnostic| diagnostic.code == code), "{ordinary:?}");
        for guarded in [format!("{guard}{source}"), format!("{source}{guard}")] {
            with_checker_diagnostics(&guarded, |checker| {
                assert!(checker.diagnostics.iter().any(|diagnostic| diagnostic.code == code),
                    "guarded checking must retain ordinary {code} validation: {:?}", checker.diagnostics);
            });
        }
    }
}

#[test]
fn composed_getters_preserve_the_authored_receiver_loan() {
    with_checker(
        r#"
interface Ready {}
trait Borrow<T> {
    T $source;
    T $view where (T implements Ready) { get => $this->source; }
}
class Adapter<U> {
    uses Borrow<U>;
    function __construct(parameter take U $value) { $this->source = $value; }
}
"#,
        |checker| {
            let declaration = checker.classes["Adapter"].properties["view"]
                .hooks
                .as_ref()
                .unwrap()
                .getter
                .unwrap()
                .declaration;
            let getter = &checker.function_signatures[&declaration];
            assert_eq!(getter.return_borrow.unwrap().source, BorrowSource::Receiver);
            let template = &checker.composition.callable_templates[&getter.declaration];
            assert_ne!(template.declaration, getter.declaration);
            assert_eq!(
                checker.function_signatures[&template.declaration].return_borrow,
                getter.return_borrow
            );
        },
    );
}

#[test]
fn composed_ambient_effects_reach_forward_callers_and_aliases() {
    let declarations = r#"
interface Ready {}
trait Output<T> {
    function emit(): void where (T implements Ready) { echo "value"; }
}
class Adapter<U> { uses Output<U> { Output<U>::emit as alias; } }
"#;
    let callers = r#"
function first<T implements Ready>(Adapter<T> $adapter): void { second($adapter); }
function second<T implements Ready>(Adapter<T> $adapter): void { $adapter->alias(); }
"#;
    for source in [
        format!("{declarations}{callers}"),
        format!("{callers}{declarations}"),
    ] {
        with_checker(&source, |checker| {
            let method = &checker.classes["Adapter"].methods["emit"];
            let effects = &method.checked_effects;
            assert!(!effects.is_empty());
            for name in ["emit", "alias"] {
                assert_eq!(
                    &checker.classes["Adapter"].methods[name].checked_effects,
                    effects
                );
            }
            for name in ["first", "second"] {
                assert_eq!(&checker.functions[name].checked_effects, effects);
            }
        });
    }
}

#[test]
fn composed_returned_closures_preserve_borrowed_capture_sources() {
    let declarations = r#"
interface Ready { function read(): int; }
trait Bind<T> {
    function bind(T $value): function(): int where (T implements Ready) {
        return fn() with ($value) => $value->read();
    }
}
class Adapter<U> { uses Bind<U>; }
"#;
    let callers = r#"
function bind<T implements Ready>(Adapter<T> $adapter, T $source): function(): int {
    return $adapter->bind($source);
}
"#;
    for source in [
        format!("{declarations}{callers}"),
        format!("{callers}{declarations}"),
    ] {
        with_checker(&source, |checker| {
            let method = &checker.classes["Adapter"].methods["bind"];
            let borrow = method
                .return_borrow
                .expect("returned callback retains its source");
            assert_eq!(borrow.source, BorrowSource::Parameter(0));
            assert_eq!(borrow.kind, ReturnBorrowKind::Retained);
            let caller = checker.functions["bind"].return_borrow.unwrap();
            assert_eq!(caller.source, BorrowSource::Parameter(1));
            assert_eq!(caller.kind, ReturnBorrowKind::Retained);
        });
    }
}

#[test]
fn imported_trait_callable_summaries_reach_the_lexical_trait_and_class() {
    let root = r#"
interface Ready {}
trait Borrow<T> {
    function keep(T $value): T where (T implements Ready) { return $value; }
    function emit(): void where (T implements Ready) { echo "value"; }
}
"#;
    let caller = r#"
trait Forward<U> {
    uses Borrow<U> { Borrow<U>::keep as alias; }
    function forward(U $source): U where (U implements Ready) { return $this->alias($source); }
    function handled(): void where (U implements Ready) {
        try { $this->emit(); } catch (Doria\Std\Io\IoError) {}
    }
}
class Adapter<V> { uses Forward<V>; }
"#;
    for source in [format!("{root}{caller}"), format!("{caller}{root}")] {
        with_checker(&source, |checker| {
            let authored = checker.trait_method("Forward", "forward").unwrap();
            assert_eq!(
                authored.return_borrow.unwrap().source,
                BorrowSource::Parameter(0)
            );
            let effective = &checker.classes["Adapter"].methods["forward"];
            assert_eq!(effective.return_borrow, authored.return_borrow);
            let emit = checker.trait_method("Forward", "emit").unwrap();
            assert!(!emit.checked_effects.is_empty());
            assert!(checker
                .trait_method("Forward", "handled")
                .unwrap()
                .checked_effects
                .is_empty());
            assert!(checker.classes["Adapter"].methods["handled"]
                .checked_effects
                .is_empty());
        });
    }
}

#[test]
fn source_trait_views_preserve_conflicts_and_their_lexical_owner() {
    with_checker(
        r#"
interface Ready {}
trait Root<T> { function read(): int where (T implements Ready) { return 1; } }
trait Forward<U> {
    uses Root<U>;
    function read(): int where (U implements Ready) { return 2; }
}
"#,
        |checker| {
            let imported = &checker.composition.source_trait_members["Forward"];
            assert_eq!(imported.len(), 1);
            let span = crate::trait_composition::member_span(&imported[0]);
            let template = &checker.composition.callable_templates[&span];
            assert_eq!(
                template.receiver,
                crate::types::ResolvedType::TraitSelf("Forward".into())
            );
            assert!(checker.trait_method("Forward", "read").is_none());
            assert!(checker.composition.classes.is_empty());
            assert!(checker.composition.invalid_classes.is_empty());
            let published = checker.take_conditional_declaration_contracts();
            assert_eq!(published.conditions.len(), 2);
            assert!(published
                .conditions
                .keys()
                .all(|span| span.expansion.0 == 0));
        },
    );
}

#[test]
fn caught_trait_effects_are_removed_from_composed_and_forward_contracts() {
    let declarations = r#"
interface Ready {}
trait Output<T> {
    function emit(): void where (T implements Ready) { echo "value"; }
    function handled(): void where (T implements Ready) {
        try { $this->emit(); } catch (Doria\Std\Io\IoError) {}
    }
}
class Adapter<U> { uses Output<U>; }
"#;
    let callers = r#"
function first<T implements Ready>(Adapter<T> $adapter): void { second($adapter); }
function second<T implements Ready>(Adapter<T> $adapter): void { $adapter->handled(); }
"#;
    for source in [
        format!("{declarations}{callers}"),
        format!("{callers}{declarations}"),
    ] {
        with_checker(&source, |checker| {
            assert!(!checker.classes["Adapter"].methods["emit"]
                .checked_effects
                .is_empty());
            assert!(checker.classes["Adapter"].methods["handled"]
                .checked_effects
                .is_empty());
            for name in ["first", "second"] {
                assert!(checker.functions[name].checked_effects.is_empty());
            }
        });
    }
}
