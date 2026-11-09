use super::*;
use crate::php_execution;
use crate::semantics::CallableAdmissions;
use crate::types::IntegerType;

type MaterializedCallable = (Span, Vec<ResolvedType>, Vec<GenericArgument>);

fn assert_dispatch_output(program: &hir::Program, expected: &[u8]) {
    let native = crate::mir_lowering::lower_program(program).unwrap();
    crate::mir_validation::validate_program(&native).unwrap();
    let output = crate::mir_interpreter::interpret(&native).unwrap();
    assert_eq!(output.stdout, expected);
    assert!(output.runtime_diagnostic.is_none(), "{output:?}");
    assert_php_dispatch_output(program, Some(&native), expected);
}

fn assert_php_dispatch_output(
    program: &hir::Program,
    native: Option<&crate::mir::Program>,
    expected: &[u8],
) {
    let php = crate::codegen_php::generate(program, native).unwrap();
    let body = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").unwrap()
    );
    let script = php_execution::Script::new(&body);
    let output = script.command(&["-d", "ffi.enable=0"]).output().unwrap();
    assert!(output.status.success(), "{output:?}\n{php}");
    assert!(output.stderr.is_empty(), "{output:?}\n{php}");
    assert_eq!(output.stdout, expected, "{php}");
}

fn assert_native_dispatch_output(program: &hir::Program, expected: &str) {
    let native = crate::mir_lowering::lower_program(program).unwrap();
    for &profile in crate::backend::NativeProfile::AVAILABLE {
        crate::native_execution::assert_native_execution(&native, profile, expected);
    }
}

fn checked_hir(text: &str) -> hir::Program {
    let source = crate::source::SourceFile::new("callable-admissions.doria", text);
    let context = crate::names::CompilationContext::standalone(source.path.clone());
    let prepared = crate::prepare_source(&source, context.clone()).unwrap();
    let (_, analysis) = crate::analyze_source_for_ide_with_source_context(
        source.path.clone(),
        text,
        crate::testing::SourceSemanticContext::standalone(context.clone()),
    )
    .unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let program =
        crate::lowering::lower_program_with_semantics(&prepared.resolved, analysis.info).unwrap();
    crate::complete_standalone_hir(program, source, &context)
}

fn int() -> ResolvedType {
    ResolvedType::Integer(IntegerType::Int64)
}

fn collect(program: &hir::Program) -> DiagnosticResult<Vec<MaterializedCallable>> {
    let synthetic = synthetic_constructors(program);
    let declarations = callable_declarations(program, &synthetic);
    let classes = program
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
    collect_callable_instances(program, &declarations, &classes, &program.semantic_info).map(
        |(instances, _)| {
            instances
                .into_iter()
                .map(|instance| {
                    let declaration = declarations[instance.declaration];
                    (
                        declaration.function.span,
                        declaration.class_arguments.to_vec(),
                        instance.arguments,
                    )
                })
                .collect()
        },
    )
}

// These fixtures alter checked facts to exercise the shared consumer boundary.
// These fixtures additionally exercise malformed checked specialization facts.
fn guard(program: &mut hir::Program, name: &str) -> Span {
    let synthetic = synthetic_constructors(program);
    let declaration = callable_declarations(program, &synthetic)
        .into_iter()
        .find(|declaration| declaration.function.name == name)
        .unwrap()
        .function
        .span;
    program
        .semantic_info
        .contracts
        .callable_conditions
        .insert(declaration, declaration);
    declaration
}

#[test]
fn guarded_roots_keep_declaration_slots_and_unconditional_constructors() {
    let mut program = checked_hir(
        r#"
class Adapter<T> {
    function always(): int { return 1; }
    function optional(): int { return 2; }
    int $value { get { return 3; } }
}
function first(Adapter<int> $adapter): int { return $adapter->always(); }
function second(Adapter<string> $adapter): int { return $adapter->always(); }
"#,
    );
    let synthetic = synthetic_constructors(&program);
    let slots = callable_declarations(&program, &synthetic)
        .into_iter()
        .map(|declaration| {
            (
                declaration.function.span,
                declaration.class,
                declaration.function.name.clone(),
            )
        })
        .collect::<Vec<_>>();
    let method = guard(&mut program, "optional");
    let getter = {
        let declarations = callable_declarations(&program, &synthetic);
        declarations
            .iter()
            .find(|declaration| {
                declaration.function.name != "optional"
                    && declaration.function.name != "always"
                    && declaration.class.is_some()
                    && declaration.function.name != "__construct"
            })
            .unwrap()
            .function
            .span
    };
    program
        .semantic_info
        .contracts
        .callable_conditions
        .insert(getter, getter);
    let missing = collect(&program).unwrap_err();
    assert_eq!(missing[0].code, "I2401");
    assert!(missing[0]
        .message
        .contains("no checked admission inventory"));

    program.semantic_info.callable_admissions = CallableAdmissions::checked([]);
    let unavailable = collect(&program).unwrap();
    assert!(unavailable
        .iter()
        .all(|(span, _, _)| *span != method && *span != getter));

    let bindings = HashMap::from([("T".to_string(), int())]);
    program.semantic_info.callable_admissions =
        CallableAdmissions::checked([(method, bindings.clone()), (getter, bindings)]);
    let available = collect(&program).unwrap();
    for span in [method, getter] {
        let instances = available
            .iter()
            .filter(|(declaration, _, _)| *declaration == span)
            .collect::<Vec<_>>();
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].1, vec![int()]);
    }
    let after = callable_declarations(&program, &synthetic);
    assert_eq!(
        slots,
        after
            .iter()
            .map(|declaration| (
                declaration.function.span,
                declaration.class,
                declaration.function.name.clone()
            ))
            .collect::<Vec<_>>()
    );
    for declaration in after
        .iter()
        .filter(|declaration| declaration.function.name == "__construct")
    {
        assert!(available
            .iter()
            .any(|(span, arguments, _)| *span == declaration.function.span
                && arguments == declaration.class_arguments));
    }
}

#[test]
fn guarded_generic_calls_require_the_exact_checked_arguments() {
    let mut program = checked_hir(
        r#"
function choose<T>(T $value): T { return $value; }
function main(): int { return choose(7); }
"#,
    );
    let declaration = guard(&mut program, "choose");
    assert!(collect(&program).unwrap_err()[0]
        .message
        .contains("no checked admission inventory"));
    for argument in [ResolvedType::String, int()] {
        program.semantic_info.callable_admissions = CallableAdmissions::checked([(
            declaration,
            HashMap::from([("T".to_string(), argument.clone())]),
        )]);
        let result = collect(&program);
        if argument == int() {
            assert!(result
                .unwrap()
                .iter()
                .any(|(span, _, arguments)| *span == declaration
                    && arguments == &[GenericArgument::Type(int())]));
        } else {
            assert!(result.unwrap_err()[0]
                .message
                .contains("unavailable callable"));
        }
    }
}

#[test]
fn guarded_generic_methods_include_owner_and_method_bindings() {
    for is_static in [false, true] {
        let modifier = if is_static { "static " } else { "" };
        let call = if is_static {
            "$adapter->selected()"
        } else {
            "$adapter->choose(7)"
        };
        let bridge = if is_static {
            "function selected(): int { return self::choose(7); }"
        } else {
            ""
        };
        let mut program = checked_hir(&format!(
            r#"
class Adapter<T> {{ {modifier}function choose<U>(take U $value): U {{ return $value; }} {bridge} }}
function inspect(Adapter<string> $adapter): int {{ return {call}; }}
"#
        ));
        let declaration = guard(&mut program, "choose");
        for owner in [int(), ResolvedType::String] {
            program.semantic_info.callable_admissions = CallableAdmissions::checked([(
                declaration,
                HashMap::from([("U".to_string(), int()), ("T".to_string(), owner.clone())]),
            )]);
            let result = collect(&program);
            if owner == ResolvedType::String {
                assert!(result
                    .unwrap()
                    .iter()
                    .any(|(span, owner, arguments)| *span == declaration
                        && owner == &[ResolvedType::String]
                        && arguments == &[GenericArgument::Type(int())]));
            } else {
                assert!(result.unwrap_err()[0]
                    .message
                    .contains("unavailable callable"));
            }
        }
    }
}

#[test]
fn guarded_erased_generic_calls_use_implementation_parameter_names() {
    let mut program = checked_hir(
        r#"
interface Select { function choose<T>(take T $value): T; }
class Picker implements Select { function choose<U>(take U $value): U { return $value; } }
function inspect(Select $picker): int { return $picker->choose(7); }
function keep(Picker $picker): void {}
"#,
    );
    let declaration = guard(&mut program, "choose");
    for parameter in ["T", "U"] {
        program.semantic_info.callable_admissions = CallableAdmissions::checked([(
            declaration,
            HashMap::from([(parameter.to_string(), int())]),
        )]);
        let result = collect(&program);
        if parameter == "U" {
            assert!(result
                .unwrap()
                .iter()
                .any(|(span, _, arguments)| *span == declaration
                    && arguments == &[GenericArgument::Type(int())]));
        } else {
            assert!(result.unwrap_err()[0]
                .message
                .contains("unavailable implementation"));
        }
    }
}

#[test]
fn guarded_nongeneric_witnesses_require_each_concrete_owner() {
    let mut program = checked_hir(
        r#"
interface Read { function read(): int; }
class Adapter<T> implements Read { function read(): int { return 1; } }
function first(Adapter<int> $source): void {}
function second(Adapter<string> $source): void {}
"#,
    );
    let declaration = guard(&mut program, "read");
    let int_bindings = HashMap::from([("T".to_string(), int())]);
    let string_bindings = HashMap::from([("T".to_string(), ResolvedType::String)]);
    program.semantic_info.callable_admissions =
        CallableAdmissions::checked([(declaration, int_bindings.clone())]);
    let error = collect(&program).unwrap_err();
    assert_eq!(error[0].code, "I2401");
    assert!(error[0]
        .message
        .contains("interface witness selects an unavailable callable"));
    program.semantic_info.callable_admissions =
        CallableAdmissions::checked([(declaration, int_bindings), (declaration, string_bindings)]);
    let instances = collect(&program).unwrap();
    assert_eq!(
        instances
            .iter()
            .filter(|(span, _, _)| *span == declaration)
            .count(),
        2
    );
}

#[test]
fn guarded_inherited_witnesses_use_the_declaring_ancestor_bindings() {
    let mut program = checked_hir(
        r#"
interface Read { function read(): int; }
open class Base<T> { function read(): int { return 1; } }
class Child extends Base<string> implements Read {}
function first(Base<int> $source): void {}
function second(Child $source): void {}
"#,
    );
    let declaration = guard(&mut program, "read");
    for argument in [int(), ResolvedType::String] {
        program.semantic_info.callable_admissions = CallableAdmissions::checked([(
            declaration,
            HashMap::from([("T".to_string(), argument.clone())]),
        )]);
        let result = collect(&program);
        if argument == ResolvedType::String {
            let instances = result.unwrap();
            let selected = instances
                .iter()
                .filter(|(span, _, _)| *span == declaration)
                .collect::<Vec<_>>();
            assert_eq!(selected.len(), 1);
            assert_eq!(selected[0].1, vec![ResolvedType::String]);
        } else {
            assert!(result.unwrap_err()[0]
                .message
                .contains("interface witness selects an unavailable callable"));
        }
    }
}

#[test]
fn guarded_witnesses_check_getter_and_setter_admissions_independently() {
    for kind in [
        crate::ast::PropertyHookKind::Get,
        crate::ast::PropertyHookKind::Set,
    ] {
        let mut program = checked_hir(
            r#"
interface Cell { writable int $value { get; set (int $next); } }
class Adapter<T> implements Cell {
    writable int $value { get => 1; set (int $next) {} }
}
function keep(Adapter<int> $source): void {}
"#,
        );
        let name = crate::property_hooks::accessor_name("value", kind);
        let declaration = guard(&mut program, &name);
        program.semantic_info.callable_admissions = CallableAdmissions::checked([]);
        assert!(collect(&program).unwrap_err()[0]
            .message
            .contains("interface witness selects an unavailable callable"));
        program.semantic_info.callable_admissions =
            CallableAdmissions::checked([(declaration, HashMap::from([("T".to_string(), int())]))]);
        assert!(collect(&program)
            .unwrap()
            .iter()
            .any(|(span, _, _)| *span == declaration));
    }
}

#[test]
fn guarded_core_witnesses_require_admission_before_any_call() {
    let mut program = checked_hir(
        r#"
class Notice implements Displayable { function toString(): string { return "ready"; } }
"#,
    );
    let declaration = guard(&mut program, "toString");
    program.semantic_info.callable_admissions = CallableAdmissions::checked([]);
    assert!(collect(&program).unwrap_err()[0]
        .message
        .contains("interface witness selects an unavailable callable"));
    program.semantic_info.callable_admissions =
        CallableAdmissions::checked([(declaration, HashMap::new())]);
    assert!(collect(&program)
        .unwrap()
        .iter()
        .any(|(span, _, _)| *span == declaration));
}

#[test]
fn guarded_witnesses_reject_missing_implementation_declarations() {
    let mut program = checked_hir(
        r#"
class Notice implements Displayable { function toString(): string { return "ready"; } }
"#,
    );
    let declaration = guard(&mut program, "toString");
    program.semantic_info.callable_admissions =
        CallableAdmissions::checked([(declaration, HashMap::new())]);
    for item in &mut program.items {
        if let hir::Item::Class(class) = item {
            class.members.retain(|member| !matches!(member, hir::ClassMember::Method(method) if method.span == declaration));
        }
    }
    let errors = collect(&program).unwrap_err();
    assert_eq!(errors[0].code, "I2401");
    assert!(errors[0]
        .message
        .contains("no declaration with its exact bindings"));
}

#[test]
fn guarded_witnesses_ignore_nonmaterialized_conformances() {
    let program = checked_hir(
        r#"
class Notice implements Displayable { function toString(): string { return "ready"; } }
"#,
    );
    for variant in 0..4 {
        let mut program = program.clone();
        guard(&mut program, "toString");
        program.semantic_info.callable_admissions = CallableAdmissions::checked([]);
        let conformance = program
            .semantic_info
            .contracts
            .conformances
            .iter_mut()
            .find(|conformance| conformance.interface.name == "Displayable")
            .unwrap();
        match variant {
            0 => conformance.status = crate::semantics::contracts::ConformanceStatus::Invalid,
            1 => conformance.implementing_type = ResolvedType::String,
            2 => {
                conformance.implementing_type =
                    ResolvedType::Class(ClassType::new("Unregistered", vec![]))
            }
            3 => conformance
                .implementations
                .iter_mut()
                .for_each(|implementation| implementation.implementation = None),
            _ => unreachable!(),
        }
        if variant == 3 {
            assert_eq!(collect(&program).unwrap_err()[0].code, "I2401");
        } else {
            assert!(collect(&program).is_ok(), "variant {variant}");
        }
    }
}

#[test]
fn checked_graph_resolves_generic_initializer_factories_per_owner() {
    let program = checked_hir(
        r#"
class Token<T> {}
class Box<T> {}
function produce<T>(T $value): int { let $box = new Box<T>(); return 1; }
class Holder<T> { int $value = produce(new Token<T>()); }
function main(): void { let $holder = new Holder<int>(); echo $holder->value; }
"#,
    );
    assert_dispatch_output(&program, b"1");
}

#[test]
fn checked_graph_keeps_parent_and_static_receiver_dispatch_scopes() {
    for direct_parent in [false, true] {
        let child = if direct_parent {
            "class Child extends Base { override function inspect<U>(U $value): int { return 2; } function run(): int { return parent::inspect(7); } }"
        } else {
            "class Child extends Base {}"
        };
        let call = if direct_parent {
            "$source->run()"
        } else {
            "$source->inspect(7)"
        };
        let source = format!(
            r#"
class Box<T> {{}}
open class Base {{ open function inspect<T>(T $value): int {{ return 1; }} }}
{child}
class Sibling extends Base {{ override function inspect<V>(V $value): int {{ return $this->inspect(new Box<V>()); }} }}
function inspectChild(Child $source): int {{ return {call}; }}
function main(): void {{ echo inspectChild(new Child()); }}
"#
        );
        let program = checked_hir(&source);
        assert_dispatch_output(&program, b"1");
        assert_native_dispatch_output(&program, "1");
    }
}

#[test]
fn checked_graph_resolves_constrained_interface_receivers() {
    let program = checked_hir(
        r#"
interface Read { function inspect<U>(U $value): int; }
class Reader implements Read { function inspect<V>(V $value): int { return 3; } }
function forward<T implements Read>(T $source): int { return $source->inspect(7); }
function erased(Read $source): int { return forward($source); }
function main(): void { echo erased(new Reader()); }
"#,
    );
    assert_dispatch_output(&program, b"3");
}

#[test]
fn checked_graph_includes_global_generic_call_roots() {
    let program = checked_hir(
        r#"
function identity<T>(T $value): T { return $value; }
identity(7);
function main(): void { echo 7; }
"#,
    );
    assert!(collect(&program)
        .unwrap()
        .iter()
        .any(|(_, _, arguments)| arguments == &[GenericArgument::Type(int())]));
    assert_dispatch_output(&program, b"7");
}

#[test]
fn checked_graph_includes_global_closure_call_roots() {
    let program = checked_hir(
        r#"
function preceding(): int { return 0; }
function identity<T>(T $value): T { return $value; }
let $run = fn () => identity(7);
$run();
function main(): void { echo 7; }
"#,
    );
    assert!(collect(&program)
        .unwrap()
        .iter()
        .any(|(_, _, arguments)| arguments == &[GenericArgument::Type(int())]));
    assert_dispatch_output(&program, b"7");
}

#[test]
fn checked_graph_reaches_initializers_of_factory_discovered_classes() {
    let program = checked_hir(
        r#"
class Token<T> {}
function produce<T>(T $value): int { return 4; }
class Holder<T> { int $value = produce(new Token<T>()); }
function create<T>(): Holder<T> { return new Holder<T>(); }
function main(): void { Holder<int> $holder = create(); echo $holder->value; }
"#,
    );
    assert_dispatch_output(&program, b"4");
}

#[test]
fn checked_graph_missing_preparation_never_reconstructs_targets() {
    let mut program = checked_hir("function main(): void {}");
    program.semantic_info.generic_call_graph = crate::semantics::GenericCallGraph::Unprepared;
    let errors = collect(&program).unwrap_err();
    assert_eq!(errors[0].code, "I2401");
    assert!(errors[0]
        .message
        .contains("generic-call graph is unavailable"));
}

#[test]
fn inactive_parent_methods_reserve_their_canonical_slots() {
    let mut program = checked_hir(
        r#"
open class Base {
    open function first(): int { return 1; }
    open function second(): int { return 3; }
}
class Child extends Base { override function first(): int { return 2; } }
function main(): void { let $child = new Child(); echo $child->first(); echo $child->second(); }
"#,
    );
    let base = program
        .items
        .iter()
        .find_map(|item| match item {
            hir::Item::Class(class) if class.name == "Base" => {
                class.members.iter().find_map(|member| match member {
                    hir::ClassMember::Method(method) if method.name == "first" => Some(method.span),
                    _ => None,
                })
            }
            _ => None,
        })
        .unwrap();
    program
        .semantic_info
        .contracts
        .callable_conditions
        .insert(base, base);
    program.semantic_info.callable_admissions = CallableAdmissions::checked([]);
    let native = crate::mir_lowering::lower_program(&program).unwrap();
    let base = native
        .classes
        .iter()
        .find(|class| class.declaration_name == "Base")
        .unwrap();
    assert_eq!(
        base.virtual_methods.len(),
        2,
        "inactive declaration lost its slot"
    );
    assert!(base.virtual_methods[0].implementation.is_none());
    assert!(base.virtual_methods[1].implementation.is_some());
    let child = native
        .classes
        .iter()
        .find(|class| class.declaration_name == "Child")
        .unwrap();
    assert_eq!(child.virtual_methods.len(), 2);
    for (base, child) in base.virtual_methods.iter().zip(&child.virtual_methods) {
        assert_eq!(base.family, child.family);
        assert!(child.implementation.is_some());
    }
    assert_dispatch_output(&program, b"23");
    assert_native_dispatch_output(&program, "23");
}

#[test]
fn generic_virtual_families_reserve_roots_without_instantiating_unused_bodies() {
    let program = checked_hir(
        r#"
class Box<T> {}
open class Base<T> { open function inspect<U>(U $value): int { return 1; } }
class Child<V> extends Base<V> { override function inspect<W>(W $value): int { return 2; } }
class Sibling extends Base<int> {
    override function inspect<T>(T $value): int { return $this->inspect(new Box<T>()); }
}
function main(): void {
    let $first = new Child<int>();
    let $second = new Child<string>();
    echo $first->inspect(7);
    echo $second->inspect(true);
}
"#,
    );
    let native = crate::mir_lowering::lower_program(&program).unwrap();
    let bases = native
        .classes
        .iter()
        .filter(|class| class.declaration_name == "Base")
        .collect::<Vec<_>>();
    assert_eq!(bases.len(), 2);
    for base in bases {
        assert_eq!(
            base.virtual_methods.len(),
            1,
            "concrete root family was not reserved"
        );
        assert!(base.virtual_methods[0].implementation.is_none());
    }
    let sibling = native
        .classes
        .iter()
        .find(|class| class.declaration_name == "Sibling")
        .unwrap();
    assert!(sibling.virtual_methods[0].implementation.is_none());
    assert!(!native
        .functions
        .iter()
        .any(|function| function.name.contains("Sibling::inspect")));
    assert_dispatch_output(&program, b"22");
    assert_native_dispatch_output(&program, "22");
}

#[test]
fn lifecycle_calls_on_other_objects_keep_normal_virtual_dispatch() {
    let program = checked_hir(
        r#"
open class Base { open function inspect(): int { return 1; } }
class Child extends Base { override function inspect(): int { return 2; } }
class Trace { static writable int $value = 0; }
class ConstructorProbe {
    function __construct(parameter Base $other) { Trace::value = Trace::value * 10 + $other->inspect(); }
}
class DestructorProbe {
    function __construct(take Base $other) {}
    function __destruct() { Trace::value = Trace::value * 10 + $this->other->inspect(); }
}
function main(): void {
    let $other = new Child();
    { let $probe = new ConstructorProbe($other); }
    { let $probe = new DestructorProbe(new Child()); }
    echo Trace::value;
}
"#,
    );
    assert_dispatch_output(&program, b"22");
    assert_native_dispatch_output(&program, "22");
}

#[test]
fn virtual_slot_validation_rejects_inactive_targets_and_forged_families() {
    let program = checked_hir(
        r#"
open class Base { open function inspect(): int { return 1; } }
class Child extends Base { override function inspect(): int { return 2; } }
function inspectBase(Base $value): int { return $value->inspect(); }
function main(): void { echo inspectBase(new Child()); }
"#,
    );
    let native = crate::mir_lowering::lower_program(&program).unwrap();
    crate::mir_validation::validate_program(&native).unwrap();
    let child = native
        .classes
        .iter()
        .position(|class| class.declaration_name == "Child")
        .unwrap();
    let mut missing = native.clone();
    missing.classes[child].virtual_methods[0].implementation = None;
    assert!(crate::mir_validation::validate_program(&missing).is_err());
    let mut changed = native.clone();
    changed.classes[child].virtual_methods[0].family.owner = changed.classes[child].id;
    assert!(crate::mir_validation::validate_program(&changed).is_err());
    let mut wrong_implementation = native.clone();
    let entry = wrong_implementation.classes[child].virtual_methods[0]
        .implementation
        .unwrap();
    wrong_implementation.functions[entry.0]
        .method
        .as_mut()
        .unwrap()
        .virtual_family = None;
    assert!(crate::mir_validation::validate_program(&wrong_implementation).is_err());
}

#[test]
fn virtual_slots_require_the_shared_checked_plan_for_native_and_php() {
    let mut program = checked_hir(
        r#"
open class Base { open function inspect(): int { return 1; } }
function main(): void { let $value = new Base(); echo $value->inspect(); }
"#,
    );
    program.semantic_info.virtual_dispatch = Default::default();
    let native = crate::mir_lowering::lower_program(&program).unwrap_err();
    let php = crate::codegen_php::generate(&program, None)
        .unwrap_err()
        .diagnostics
        .unwrap();
    assert_eq!(native, php);
    assert_eq!(native[0].code, "I2401");
    assert!(native[0]
        .message
        .contains("virtual-slot plan is unavailable"));
}

#[test]
fn virtual_generic_calls_materialize_the_override_closure() {
    let program = checked_hir(
        r#"
class Box<T> {}
open class Base { open function inspect<T>(T $value): int { return 1; } }
class Child extends Base {
    override function inspect<U>(U $value): int { let $box = new Box<U>(); return 2; }
}
function inspectBase(Base $source): int { return $source->inspect(7); }
function main(): void { echo inspectBase(new Child()); }
"#,
    );
    let child = program
        .items
        .iter()
        .find_map(|item| match item {
            hir::Item::Class(class) if class.name == "Child" => {
                class.members.iter().find_map(|member| match member {
                    hir::ClassMember::Method(method) if method.name == "inspect" => {
                        Some(method.span)
                    }
                    _ => None,
                })
            }
            _ => None,
        })
        .unwrap();
    assert!(
        collect(&program)
            .unwrap()
            .iter()
            .any(|(span, _, arguments)| *span == child
                && arguments == &[GenericArgument::Type(int())]),
        "virtual override was not materialized"
    );
    assert_dispatch_output(&program, b"2");
    assert_native_dispatch_output(&program, "2");
}

#[test]
fn guarded_witness_rejections_cross_native_and_php_boundaries() {
    let method = r#"
interface Read { function read(): int; }
class Reader<T> implements Read { function read(): int { return 1; } }
function inspect(Read $source): int { return $source->read(); }
function main(): void { echo inspect(new Reader<int>()); }
"#;
    let property = r#"
interface Cell { writable int $value { get; set (int $next); } }
class Reader<T> implements Cell {
    writable int $value { get => 1; set (int $next) {} }
}
function inspect(Cell $source): int { return $source->value; }
function main(): void { echo inspect(new Reader<int>()); }
"#;
    let getter = crate::property_hooks::accessor_name("value", crate::ast::PropertyHookKind::Get);
    let setter = crate::property_hooks::accessor_name("value", crate::ast::PropertyHookKind::Set);
    for (source, member) in [
        (method, "read"),
        (property, getter.as_str()),
        (property, setter.as_str()),
    ] {
        let mut program = checked_hir(source);
        let declaration = guard(&mut program, member);
        for admissions in [
            CallableAdmissions::Unprepared,
            CallableAdmissions::checked([]),
        ] {
            program.semantic_info.callable_admissions = admissions;
            let expected = collect(&program).unwrap_err();
            let native = crate::mir_lowering::lower_program(&program).unwrap_err();
            let php = crate::codegen_php::generate(&program, None)
                .unwrap_err()
                .diagnostics
                .unwrap();
            assert_eq!(native, expected);
            assert_eq!(php, expected);
            assert_eq!(native[0].code, "I2401");
        }
        program.semantic_info.callable_admissions =
            CallableAdmissions::checked([(declaration, HashMap::from([("T".to_string(), int())]))]);
        let native = crate::mir_lowering::lower_program(&program).unwrap();
        crate::mir_validation::validate_program(&native).unwrap();
        let output = crate::mir_interpreter::interpret(&native).unwrap();
        assert_eq!(output.stdout, b"1");
        assert!(output.runtime_diagnostic.is_none(), "{output:?}");
        assert!(crate::codegen_php::generate(&program, Some(&native))
            .unwrap()
            .starts_with("<?php"));
    }
}

#[test]
fn checked_witness_target_identity_is_shared_by_native_and_php() {
    let mut program = checked_hir(
        r#"
interface Read { function read(): int; }
class Reader<T> implements Read {
    function read(): int { return 1; }
    function other(): int { return 2; }
}
function inspect(Read $source): int { return $source->read(); }
function main(): void { echo inspect(new Reader<int>()); }
"#,
    );
    let other = program
        .items
        .iter()
        .find_map(|item| match item {
            hir::Item::Class(class) if class.name == "Reader" => {
                class.members.iter().find_map(|member| match member {
                    hir::ClassMember::Method(method) if method.name == "other" => Some(method.span),
                    _ => None,
                })
            }
            _ => None,
        })
        .unwrap();
    let conformance = program.semantic_info.contracts.conformances.iter_mut().find(|fact| {
        matches!(&fact.implementing_type, ResolvedType::Class(class) if class.name == "Reader" && class.arguments == vec![int()])
    }).unwrap();
    conformance.implementations[0].implementation = Some(other);
    let native = crate::mir_lowering::lower_program(&program).unwrap_err();
    let Err(php) = crate::codegen_php::generate(&program, None) else {
        panic!("PHP must reject a changed canonical witness");
    };
    let php = php.diagnostics.unwrap();
    assert_eq!(native, php);
    assert_eq!(native[0].code, "I2401");
}

#[test]
fn inherited_generic_witnesses_preserve_concrete_owner_bindings() {
    let program = checked_hir(
        r#"
interface Read<T> { function read<U>(U $value): int; }
open class Base<T> implements Read<T> { function read<U>(U $value): int { return 7; } }
class Child<V> extends Base<List<V>> {}
function inspect(Read<List<int>> $source): int { return $source->read(true); }
function main(): void { echo inspect(new Child<int>()); }
"#,
    );
    assert_dispatch_output(&program, b"7");
    assert_native_dispatch_output(&program, "7");
}

#[test]
fn canonical_witnesses_reject_missing_preparation_and_inconsistent_facts() {
    let original = checked_hir(
        r#"
interface Read { function read(): int; }
class Reader<T> implements Read { function read(): int { return 1; } }
function inspect(Read $source): int { return $source->read(); }
function main(): void { echo inspect(new Reader<int>()); }
"#,
    );
    for variant in 0..5 {
        let mut program = original.clone();
        if variant == 0 {
            program.semantic_info.interface_witnesses = Default::default();
        } else {
            let conformance = program.semantic_info.contracts.conformances.iter_mut().find(|fact| {
                matches!(&fact.implementing_type, ResolvedType::Class(class) if class.name == "Reader" && class.arguments == vec![int()])
            }).unwrap();
            match variant {
                1 => conformance.implementations[0].implementation = None,
                2 => {
                    conformance.implementations[0].requirement_origins[0].declaration =
                        Span::default()
                }
                3 => {
                    conformance.implementations[0].exact_dynamic_return = Some(ResolvedType::String)
                }
                4 => conformance
                    .implementations
                    .push(conformance.implementations[0].clone()),
                _ => unreachable!(),
            }
        }
        let Err(native) = crate::mir_lowering::lower_program(&program) else {
            panic!("native must reject inconsistent witness facts");
        };
        let Err(php) = crate::codegen_php::generate(&program, None) else {
            panic!("PHP must reject inconsistent witness facts");
        };
        let php = php.diagnostics.unwrap();
        assert_eq!(native, php, "variant {variant}");
        assert_eq!(native[0].code, "I2401", "variant {variant}");
    }
}

#[test]
fn covariant_open_child_calls_preserve_the_physical_slot_return_contract() {
    let program = checked_hir(
        r#"
open class Animal { int $value = 7; }
class Dog extends Animal {}
open class Factory {
    open function create(): ?Animal { return new Animal(); }
}
open class DogFactory extends Factory {
    override function create(): Dog { return new Dog(); }
}
class LeafFactory extends DogFactory {
    override function create(): Dog { return new Dog(); }
}
function produce(DogFactory $factory): Dog { return $factory->create(); }
function main(): void { let $dog = produce(new LeafFactory()); echo $dog->value; }
"#,
    );
    let native = crate::mir_lowering::lower_program(&program).unwrap();
    crate::mir_validation::validate_program(&native).unwrap();
    #[cfg(feature = "llvm-backend")]
    {
        let ir = crate::codegen_llvm::lower_mir_to_llvm_ir(&native).unwrap();
        let calls = ir
            .lines()
            .filter(|line| line.contains("call ") && line.contains("%class.virtual.entry("))
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), 1, "{calls:?}");
        assert!(calls[0].contains("call { ptr, ptr }"), "{}", calls[0]);
    }
    let child = native
        .functions
        .iter()
        .find(|function| {
            function.method.as_ref().is_some_and(|method| {
                native.classes[method.class.0].declaration_name == "DogFactory"
                    && method.name == "create"
            }) && function.virtual_slot.is_some()
                && !function.name.ends_with("::<virtual-return-adapter>")
        })
        .unwrap();
    let mut malformed = native.clone();
    let owner = child.method.as_ref().unwrap().class;
    malformed.classes[owner.0].virtual_methods[0].implementation = Some(child.id);
    let error = crate::mir_validation::validate_program(&malformed).unwrap_err();
    assert!(error.message.contains("canonical return ABI"), "{error:?}");
    assert_dispatch_output(&program, b"7");
    assert_native_dispatch_output(&program, "7");
}

#[test]
fn covariant_checked_child_calls_adapt_only_successful_results() {
    for case in ["Dog", "?Dog", "null"] {
        let returned = if case == "null" { "?Dog" } else { case };
        let value = if case == "null" { "null" } else { "new Dog()" };
        let expected = if case == "null" { "Nx" } else { "7Dx" };
        let inspect = if returned == "Dog" {
            "echo $dog->value;"
        } else if case == "null" {
            "if ($dog == null) { echo \"N\"; }"
        } else {
            "if ($dog != null) { echo $dog->value; }"
        };
        let program = checked_hir(&format!(
            r#"
class Failure implements Error {{ function __construct(string $message) {{}} }}
open class Animal {{ int $value = 7; }}
class Dog extends Animal {{ function __destruct() {{ try {{ echo "D"; }} catch (Error) {{}} }} }}
open class Factory {{
    open function create(bool $fail): ?Animal throws Failure {{
        if ($fail) {{ throw new Failure("x"); }}
        return new Animal();
    }}
}}
open class DogFactory extends Factory {{
    override function create(bool $fail): {returned} throws Failure {{
        if ($fail) {{ throw new Failure("x"); }}
        return {value};
    }}
}}
class LeafFactory extends DogFactory {{}}
function produce(DogFactory $factory, bool $fail): {returned} throws Failure {{ return $factory->create($fail); }}
function main(): void {{
    try {{
        let $dog = produce(new LeafFactory(), false);
        {inspect}
        let $failed = produce(new LeafFactory(), true);
    }} catch (Failure $error) {{ echo $error->message; }}
}}
"#,
        ));
        #[cfg(feature = "llvm-backend")]
        {
            let native = crate::mir_lowering::lower_program(&program).unwrap();
            let ir = crate::codegen_llvm::lower_mir_to_llvm_ir(&native).unwrap();
            assert!(ir.contains("%checked.virtual.result = alloca { ptr, ptr }"));
            assert!(ir.contains("checked.virtual.result.success:"));
        }
        assert_dispatch_output(&program, expected.as_bytes());
        assert_native_dispatch_output(&program, expected);
    }
}

#[test]
fn covariant_slots_keep_the_root_contract_when_its_body_is_inactive() {
    let mut program = checked_hir(
        r#"
open class Animal { int $value = 7; }
class Dog extends Animal {}
open class Factory { open function create(): Animal { return new Animal(); } }
open class DogFactory extends Factory { override function create(): Dog { return new Dog(); } }
class LeafFactory extends DogFactory {}
function produce(DogFactory $factory): Dog { return $factory->create(); }
function main(): void { let $dog = produce(new LeafFactory()); echo $dog->value; }
"#,
    );
    let root = program
        .semantic_info
        .method_hierarchy
        .values()
        .find(|method| method.declaring_class == "Factory" && method.name == "create")
        .unwrap()
        .declaration;
    program
        .semantic_info
        .contracts
        .callable_conditions
        .insert(root, root);
    program.semantic_info.callable_admissions = CallableAdmissions::checked([]);
    let native = crate::mir_lowering::lower_program(&program).unwrap();
    let root = native
        .classes
        .iter()
        .find(|class| class.declaration_name == "Factory")
        .unwrap();
    assert!(root.virtual_methods[0].implementation.is_none());
    for class in native
        .classes
        .iter()
        .filter(|class| class.declaration_name.ends_with("Factory") && class.id != root.id)
    {
        let slot = &class.virtual_methods[0];
        assert_eq!(slot.family, root.virtual_methods[0].family);
        assert_eq!(
            native.functions[slot.implementation.unwrap().0].return_type,
            slot.family.return_type
        );
    }
    assert_dispatch_output(&program, b"7");
    assert_native_dispatch_output(&program, "7");
}

#[test]
fn covariant_generic_slots_preserve_concrete_root_bindings() {
    let program = checked_hir(
        r#"
open class Animal { int $value = 7; }
class Dog extends Animal {}
open class Factory<T> { open function create<U>(U $value): Animal { return new Animal(); } }
open class DogFactory<V> extends Factory<List<V>> {
    override function create<W>(W $value): Dog { return new Dog(); }
}
class LeafFactory extends DogFactory<int> {}
function produce(DogFactory<int> $factory): Dog { return $factory->create(true); }
function main(): void { let $dog = produce(new LeafFactory()); echo $dog->value; }
"#,
    );
    assert_dispatch_output(&program, b"7");
    assert_native_dispatch_output(&program, "7");
}

#[test]
fn covariant_borrowed_slots_preserve_the_returned_dynamic_descriptor() {
    let program = checked_hir(
        r#"
open class Animal { open function number(): int { return 1; } }
open class Dog extends Animal { override function number(): int { return 2; } }
class Poodle extends Dog { override function number(): int { return 9; } }
open class Factory {
    Animal $animal = new Animal();
    open function create(): Animal { return $this->animal; }
    open function optional(): ?Animal { return $this->animal; }
}
open class DogFactory extends Factory {
    Dog $dog = new Poodle();
    override function create(): Dog { return $this->dog; }
    override function optional(): ?Dog { return $this->dog; }
}
class LeafFactory extends DogFactory {}
function produce(DogFactory $factory): Dog { return $factory->create(); }
function produceOptional(DogFactory $factory): ?Dog { return $factory->optional(); }
function main(): void {
    let $factory = new LeafFactory();
    let $dog = produce($factory);
    echo $dog->number();
    let $optional = produceOptional($factory);
    if ($optional != null) { echo $optional->number(); }
}
"#,
    );
    assert_dispatch_output(&program, b"99");
    assert_native_dispatch_output(&program, "99");
}

#[test]
fn virtual_overrides_preserve_uniform_checked_transport_when_required_errors_narrow() {
    let program = checked_hir(include_str!(
        "../../tests/fixtures/virtual_required_error_narrowing.doria"
    ));
    let native = crate::mir_lowering::lower_program(&program).unwrap();
    crate::mir_validation::validate_program(&native).unwrap();
    let root = native
        .classes
        .iter()
        .find(|class| class.declaration_name == "Root")
        .unwrap();
    for class in native
        .classes
        .iter()
        .filter(|class| class.id == root.id || class.ancestors.contains(&root.id))
    {
        for (contract_slot, slot) in root.virtual_methods.iter().zip(&class.virtual_methods) {
            let contract = &native.functions[contract_slot.implementation.unwrap().0];
            let expected = crate::native_closure_abi::NativeCallableSignaturePlan::direct(contract);
            assert!(expected.checked);
            let entry = &native.functions[slot.implementation.unwrap().0];
            let actual = crate::native_closure_abi::NativeCallableSignaturePlan::direct(entry);
            assert_eq!(
                actual.checked, expected.checked,
                "{} changes checked transport for {}",
                class.name, contract.name
            );
            assert_eq!(
                actual.hidden_inputs, expected.hidden_inputs,
                "{} changes hidden inputs for {}",
                class.name, contract.name
            );
        }
    }
    let caller = program
        .items
        .iter()
        .find_map(|item| match item {
            hir::Item::Function(function) if function.name == "throughChild" => Some(function),
            _ => None,
        })
        .unwrap();
    assert!(
        caller.required_checked_effects.is_empty(),
        "physical transport must preserve source error narrowing"
    );
    let child = native
        .classes
        .iter()
        .find(|class| class.declaration_name == "Child")
        .unwrap();
    let child_value = child.virtual_methods[0].implementation.unwrap();
    assert!(native.functions[child_value.0].checked_effects.is_empty());
    let mut changed_transport = native.clone();
    changed_transport.functions[child_value.0]
        .method
        .as_mut()
        .unwrap()
        .virtual_family
        .as_mut()
        .unwrap()
        .checked_transport = false;
    assert!(crate::mir_validation::validate_program(&changed_transport).is_err());

    let adapter = native
        .functions
        .iter()
        .find(|function| {
            function
                .method
                .as_ref()
                .is_some_and(|method| method.class == child.id)
                && matches!(
                    function.diagnostic_frame,
                    crate::mir::DiagnosticFrame::Forwarded(_)
                )
        })
        .unwrap();
    assert!(adapter.checked_effects.is_empty());
    let mut widened = native.clone();
    let failure = native
        .error_descriptors
        .iter()
        .find(|error| error.type_name == "Failure")
        .unwrap()
        .id;
    widened.functions[adapter.id.0].required_checked_effects =
        vec![crate::mir::CheckedEffect::Concrete(failure)];
    widened.functions[adapter.id.0].checked_effects =
        vec![crate::mir::CheckedEffect::Concrete(failure)];
    let error = crate::mir_validation::validate_program(&widened).unwrap_err();
    assert!(
        error
            .message
            .contains("widens its static checked-error contract"),
        "{error:?}"
    );
    #[cfg(feature = "llvm-backend")]
    {
        let ir = crate::codegen_llvm::lower_mir_to_llvm_ir(&native).unwrap();
        let calls = ir
            .lines()
            .filter(|line| line.contains("call ") && line.contains("virtual.entry"))
            .collect::<Vec<_>>();
        assert!(!calls.is_empty());
        for call in calls {
            assert!(
                call.contains("call i8"),
                "virtual call lost checked transport: {call}"
            );
        }
    }
    assert_dispatch_output(&program, b"7tn59z7tn59z");
    assert_native_dispatch_output(&program, "7tn59z7tn59z");
}

#[test]
fn narrowed_virtual_errors_preserve_direct_calls_and_owned_argument_cleanup() {
    let program = checked_hir(
        r#"
class Failure implements Error { function __construct(string $message) {} }
class Token { function __destruct() { try { echo "T"; } catch (Error) {} } }
open class Root {
    open function value(take Token $token, bool $fail): int throws Failure {
        if ($fail) { throw new Failure("x"); }
        return 1;
    }
}
class Child extends Root {
    override function value(take Token $token, bool $fail): int { return 7; }
    function exact(take Token $token, bool $fail): int throws Failure {
        return parent::value($token, $fail);
    }
}
function invoke(Root $source, bool $fail): int throws Failure {
    return $source->value(new Token(), $fail);
}
function main(): void {
    let $root = new Root();
    try { echo invoke($root, true); } catch (Failure) { echo "e"; }
    let $child = new Child();
    echo invoke($child, true);
    echo $child->value(new Token(), true);
    try { echo $child->exact(new Token(), true); } catch (Failure) { echo "p"; }
}
"#,
    );
    assert_dispatch_output(&program, b"TeT7T7Tp");
    assert_native_dispatch_output(&program, "TeT7T7Tp");
}

#[test]
fn inactive_virtual_roots_retain_checked_transport_contracts() {
    let mut program = checked_hir(
        r#"
class Failure implements Error { function __construct(string $message) {} }
open class Root { open function value(): int throws Failure { return 1; } }
class Child extends Root { override function value(): int { return 7; } }
function main(): void { let $child = new Child(); echo $child->value(); }
"#,
    );
    let root = program
        .semantic_info
        .method_hierarchy
        .values()
        .find(|method| method.declaring_class == "Root" && method.name == "value")
        .unwrap()
        .declaration;
    program
        .semantic_info
        .contracts
        .callable_conditions
        .insert(root, root);
    program.semantic_info.callable_admissions = CallableAdmissions::checked([]);
    let native = crate::mir_lowering::lower_program(&program).unwrap();
    crate::mir_validation::validate_program(&native).unwrap();
    let root = native
        .classes
        .iter()
        .find(|class| class.declaration_name == "Root")
        .unwrap();
    assert!(root.virtual_methods[0].implementation.is_none());
    assert!(root.virtual_methods[0].family.checked_transport);
    assert_dispatch_output(&program, b"7");
    assert_native_dispatch_output(&program, "7");
}

#[test]
fn virtual_error_narrowing_keeps_nominal_subtypes() {
    let program = checked_hir(include_str!(
        "../../tests/fixtures/virtual_nominal_error_narrowing.doria"
    ));
    assert_dispatch_output(&program, b"cc");
    assert_native_dispatch_output(&program, "cc");
}

#[test]
fn virtual_error_narrowing_keeps_error_subinterface_witnesses() {
    let program = checked_hir(include_str!(
        "../../tests/fixtures/virtual_error_interface_narrowing.doria"
    ));
    assert_dispatch_output(&program, b"ic");
    assert_native_dispatch_output(&program, "ic");
}

#[test]
fn exact_parent_calls_keep_their_own_ambient_transport_profile() {
    let program = checked_hir(include_str!(
        "../../tests/fixtures/virtual_exact_parent_ambient.doria"
    ));
    let native = crate::mir_lowering::lower_program(&program).unwrap();
    crate::mir_validation::validate_program(&native).unwrap();
    let root = native
        .classes
        .iter()
        .find(|class| class.declaration_name == "Root")
        .unwrap();
    let direct = native
        .functions
        .iter()
        .find(|function| {
            function.virtual_slot.is_none()
                && function
                    .method
                    .as_ref()
                    .is_some_and(|method| method.class == root.id && method.name == "value")
        })
        .unwrap();
    assert!(
        !crate::native_closure_abi::NativeCallableSignaturePlan::direct(direct).checked,
        "pure exact parent implementation inherited a descendant's ambient transport"
    );
    let caller = native
        .functions
        .iter()
        .find(|function| function.name == "Child::exact")
        .unwrap();
    assert!(
        caller.checked_effects.is_empty(),
        "exact parent call inherited a virtual family's ambient effects"
    );
    for class in native
        .classes
        .iter()
        .filter(|class| class.id == root.id || class.ancestors.contains(&root.id))
    {
        let entry = &native.functions[class.virtual_methods[0].implementation.unwrap().0];
        assert!(
            crate::native_closure_abi::NativeCallableSignaturePlan::direct(entry).checked,
            "{} lost virtual ambient-error transport",
            class.name
        );
    }
    assert_dispatch_output(&program, b"3");
    assert_native_dispatch_output(&program, "3");
}

#[test]
fn exact_and_virtual_ambient_profiles_preserve_scopes_catches_and_lifecycle() {
    let program = checked_hir(include_str!(
        "../../tests/fixtures/virtual_ambient_effect_profiles.doria"
    ));
    let native = crate::mir_lowering::lower_program(&program).unwrap();
    crate::mir_validation::validate_program(&native).unwrap();
    for name in [
        "Root::value::<direct>",
        "Root::__construct",
        "Root::__destruct",
        "Middle::exact",
        "Leaf::value::<direct>",
        "Sibling::value::<direct>",
        "catches",
    ] {
        let function = native
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap();
        assert!(
            function.checked_effects.is_empty(),
            "{name}: {:?}",
            function.checked_effects
        );
        assert!(!function.has_checked_transport(), "{name}");
    }
    for name in ["Root::value", "Middle::value", "Middle::value::<direct>"] {
        let function = native
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap();
        assert!(!function.ambient_checked_effects.is_empty(), "{name}");
        assert!(function.has_checked_transport(), "{name}");
    }
    let leaf = native
        .functions
        .iter()
        .find(|function| function.name == "Leaf::value")
        .unwrap();
    assert!(leaf.checked_effects.is_empty());
    assert!(leaf.has_checked_transport());
    assert_dispatch_output(&program, b"m793953m4");
    assert_native_dispatch_output(&program, "m793953m4");

    let root = program
        .semantic_info
        .method_hierarchy
        .values()
        .find(|method| method.declaring_class == "Root" && method.name == "value")
        .unwrap()
        .declaration;
    let mut missing = program.clone();
    missing
        .semantic_info
        .virtual_ambient_checked_effects
        .remove(&root);
    let errors = crate::mir_lowering::lower_program(&missing).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.code == "I2401"
            && error.message.contains("checked ambient profile")),
        "{errors:?}"
    );
}

#[test]
fn exact_parent_ambient_profiles_preserve_owner_method_trait_and_callback_bindings() {
    let program = checked_hir(include_str!(
        "../../tests/fixtures/virtual_generic_parent_ambient.doria"
    ));
    let native = crate::mir_lowering::lower_program(&program).unwrap();
    crate::mir_validation::validate_program(&native).unwrap();
    for function in native.functions.iter().filter(|function| {
        function.name.contains("::exact")
            || function.name.contains("::relay")
            || function.name == "viaCallback"
    }) {
        assert!(
            function.checked_effects.is_empty(),
            "{}: {:?}",
            function.name,
            function.checked_effects
        );
    }
    assert_dispatch_output(&program, b"789");
    assert_native_dispatch_output(&program, "789");
}

#[test]
fn virtual_receiver_weakening_preserves_source_and_mir_substitutability() {
    let program = checked_hir(include_str!(
        "../../tests/fixtures/virtual_receiver_weakening.doria"
    ));
    let native = crate::mir_lowering::lower_program(&program).unwrap();
    crate::mir_validation::validate_program(&native).unwrap();
    assert_dispatch_output(&program, b"77cr");
    assert_native_dispatch_output(&program, "77cr");

    let child = native
        .classes
        .iter()
        .find(|class| class.declaration_name == "Child")
        .unwrap();
    assert_eq!(
        child.virtual_methods[0].family.receiver_mode,
        crate::mir::ReceiverMode::Writable,
        "the slot must retain the root's receiver contract"
    );
    let entry = child.virtual_methods[0].implementation.unwrap();
    let mut consuming = native.clone();
    let entry = &mut consuming.functions[entry.0];
    entry.receiver_mode = Some(crate::mir::ReceiverMode::Consuming);
    entry.parameter_modes[0] = crate::mir::FunctionParameterMode::Take;
    entry.locals[entry.params[0].0].owned = true;
    assert!(
        crate::mir_validation::validate_program(&consuming).is_err(),
        "a borrowed virtual slot must not consume its receiver"
    );

    let Err(errors) = crate::check_source("receiver-strengthening.doria",
        "open class Root { open function value(): int { return 1; } } class Child extends Root { override writable function value(): int { return 7; } } function main(): void {}") else {
        panic!("a readonly receiver contract must reject writable strengthening");
    };
    assert!(
        errors.iter().any(|error| error.code == "E0729"),
        "{errors:?}"
    );
}

#[test]
fn receiver_substitution_uses_one_semantic_and_mir_contract() {
    use crate::symbols::ReceiverMode::{Consuming, Readonly, Writable};
    for required in [Readonly, Writable, Consuming] {
        for actual in [Readonly, Writable, Consuming] {
            let native: crate::mir::ReceiverMode = actual;
            assert_eq!(
                native.can_override(required),
                matches!(
                    (actual, required),
                    (Readonly, Readonly)
                        | (Readonly, Writable)
                        | (Writable, Writable)
                        | (Consuming, Consuming)
                ),
                "{actual:?} overriding {required:?}"
            );
        }
    }
}

#[test]
fn inactive_virtual_roots_retain_receiver_ownership_contracts() {
    let mut program = checked_hir(
        r#"
open class Root { open writable function value(): int { return 1; } }
class Child extends Root { override function value(): int { return 7; } }
function invoke(Child $source): int { return $source->value(); }
function main(): void { echo invoke(new Child()); }
"#,
    );
    let root = program
        .semantic_info
        .method_hierarchy
        .values()
        .find(|method| method.declaring_class == "Root" && method.name == "value")
        .unwrap()
        .declaration;
    program
        .semantic_info
        .contracts
        .callable_conditions
        .insert(root, root);
    program.semantic_info.callable_admissions = CallableAdmissions::checked([]);
    let native = crate::mir_lowering::lower_program(&program).unwrap();
    let root = native
        .classes
        .iter()
        .find(|class| class.declaration_name == "Root")
        .unwrap();
    assert!(root.virtual_methods[0].implementation.is_none());
    assert_eq!(
        root.virtual_methods[0].family.receiver_mode,
        crate::mir::ReceiverMode::Writable
    );
    let mut weakened_root = native.clone();
    weakened_root.classes[root.id.0].virtual_methods[0].receiver_mode =
        crate::mir::ReceiverMode::Readonly;
    let error = crate::mir_validation::validate_program(&weakened_root).unwrap_err();
    assert!(
        error.message.contains("root receiver contract"),
        "{error:?}"
    );
    crate::mir_validation::validate_program(&native).unwrap();
    assert_dispatch_output(&program, b"7");
    assert_native_dispatch_output(&program, "7");

    let child = native
        .classes
        .iter()
        .find(|class| class.declaration_name == "Child")
        .unwrap();
    let entry = child.virtual_methods[0].implementation.unwrap();
    let mut consuming = native.clone();
    let entry = &mut consuming.functions[entry.0];
    entry.receiver_mode = Some(crate::mir::ReceiverMode::Consuming);
    entry.parameter_modes[0] = crate::mir::FunctionParameterMode::Take;
    entry.locals[entry.params[0].0].owned = true;
    let error = crate::mir_validation::validate_program(&consuming).unwrap_err();
    assert!(
        error.message.contains("canonical receiver contract"),
        "{error:?}"
    );
}

#[test]
fn virtual_receiver_weakening_does_not_weaken_other_parameter_contracts() {
    let program = checked_hir(
        r#"
open class Root { open writable function value(int $number): int { return $number; } }
open class Middle extends Root { override function value(int $number): int { return $number; } }
class Leaf extends Middle { override function value(int $number): int { return $number; } }
function invoke(Middle $source): int { return $source->value(7); }
function main(): void { echo invoke(new Leaf()); }
"#,
    );
    let native = crate::mir_lowering::lower_program(&program).unwrap();
    crate::mir_validation::validate_program(&native).unwrap();
    assert_dispatch_output(&program, b"7");
    assert_native_dispatch_output(&program, "7");
    let leaf = native
        .classes
        .iter()
        .find(|class| class.declaration_name == "Leaf")
        .unwrap();
    let entry = leaf.virtual_methods[0].implementation.unwrap();
    let mut changed_parameter = native.clone();
    let function = &mut changed_parameter.functions[entry.0];
    function.parameter_modes[1] = crate::mir::FunctionParameterMode::Writable;
    function.locals[function.params[1].0].writable = true;
    let error = crate::mir_validation::validate_program(&changed_parameter).unwrap_err();
    assert!(
        error.message.contains("parameter ownership contract"),
        "{error:?}"
    );

    let mut strengthened_receiver = native.clone();
    let function = &mut strengthened_receiver.functions[entry.0];
    function.receiver_mode = Some(crate::mir::ReceiverMode::Writable);
    function.parameter_modes[0] = crate::mir::FunctionParameterMode::Writable;
    function.locals[function.params[0].0].writable = true;
    let error = crate::mir_validation::validate_program(&strengthened_receiver).unwrap_err();
    assert!(
        error.message.contains("logical receiver contract"),
        "{error:?}"
    );
}

#[test]
fn inactive_intermediate_slots_preserve_their_narrowed_receiver_contract() {
    let mut program = checked_hir(
        r#"
open class Root { open writable function value(): int { return 1; } }
open class Middle extends Root { override function value(): int { return 7; } }
class Leaf extends Middle { override function value(): int { return 9; } }
function invoke(Leaf $source): int { return $source->value(); }
function main(): void { echo invoke(new Leaf()); }
"#,
    );
    let middle = program
        .semantic_info
        .method_hierarchy
        .values()
        .find(|method| method.declaring_class == "Middle" && method.name == "value")
        .unwrap()
        .declaration;
    program
        .semantic_info
        .contracts
        .callable_conditions
        .insert(middle, middle);
    program.semantic_info.callable_admissions = CallableAdmissions::checked([]);
    let native = crate::mir_lowering::lower_program(&program).unwrap();
    let middle = native
        .classes
        .iter()
        .find(|class| class.declaration_name == "Middle")
        .unwrap();
    assert!(middle.virtual_methods[0].implementation.is_none());
    assert_eq!(
        middle.virtual_methods[0].receiver_mode,
        crate::mir::ReceiverMode::Readonly
    );
    assert_eq!(
        middle.virtual_methods[0].family.receiver_mode,
        crate::mir::ReceiverMode::Writable
    );
    crate::mir_validation::validate_program(&native).unwrap();
    assert_dispatch_output(&program, b"9");
    assert_native_dispatch_output(&program, "9");

    let leaf = native
        .classes
        .iter()
        .find(|class| class.declaration_name == "Leaf")
        .unwrap();
    let entry = leaf.virtual_methods[0].implementation.unwrap();
    let mut strengthened = native.clone();
    strengthened.classes[leaf.id.0].virtual_methods[0].receiver_mode =
        crate::mir::ReceiverMode::Writable;
    let function = &mut strengthened.functions[entry.0];
    function.receiver_mode = Some(crate::mir::ReceiverMode::Writable);
    function.parameter_modes[0] = crate::mir::FunctionParameterMode::Writable;
    function.locals[function.params[0].0].writable = true;
    let error = crate::mir_validation::validate_program(&strengthened).unwrap_err();
    assert!(
        error.message.contains("inherited receiver contract"),
        "{error:?}"
    );
}

#[test]
fn covariant_interface_returns_preserve_concrete_and_dynamic_class_views() {
    let program = checked_hir(include_str!(
        "../../tests/fixtures/virtual_interface_return_covariance.doria"
    ));
    let native = crate::mir_lowering::lower_program(&program).unwrap();
    crate::mir_validation::validate_program(&native).unwrap();
    let implementation = native
        .functions
        .iter()
        .find(|function| function.name == "Child::closed")
        .unwrap();
    assert!(matches!(
        implementation.return_type,
        crate::mir::ReturnType::Value(crate::mir::Type::Class(_))
    ));
    assert!(matches!(
        implementation
            .method
            .as_ref()
            .unwrap()
            .virtual_family
            .as_ref()
            .unwrap()
            .return_type,
        crate::mir::ReturnType::Value(crate::mir::Type::Interface(_))
    ));
    let mut forged = native.clone();
    let root = native
        .classes
        .iter()
        .find(|class| class.declaration_name == "Root")
        .unwrap()
        .id;
    forged.functions[implementation.id.0].return_type =
        crate::mir::ReturnType::Value(crate::mir::Type::Class(root));
    let error = crate::mir_validation::validate_program(&forged).unwrap_err();
    assert!(
        error.message.contains("canonical return contract")
            || error
                .message
                .contains("return adapter changes its implementation contract"),
        "{error:?}"
    );
    let closed_adapter = native
        .functions
        .iter()
        .find(|function| {
            function
                .method
                .as_ref()
                .is_some_and(|method| method.name == "closed" && method.return_adapter.is_some())
        })
        .unwrap();
    let open_adapter = native
        .functions
        .iter()
        .find(|function| {
            function
                .method
                .as_ref()
                .is_some_and(|method| method.name == "openView" && method.return_adapter.is_some())
        })
        .unwrap();
    let mut forged = native.clone();
    forged.functions[closed_adapter.id.0].blocks = open_adapter.blocks.clone();
    let error = crate::mir_validation::validate_program(&forged).unwrap_err();
    assert!(
        error
            .message
            .contains("adapter body disagrees with its checked conversion"),
        "{error:?}"
    );
    let plan = closed_adapter
        .method
        .as_ref()
        .unwrap()
        .return_adapter
        .unwrap();
    let mut forged = native.clone();
    for id in [plan.implementation, plan.direct] {
        let function = &mut forged.functions[id.0];
        function.locals[function.params[0].0].ty = crate::mir::Type::ClosureEnvironment(None);
    }
    let error = crate::mir_validation::validate_program(&forged).unwrap_err();
    assert!(
        error.message.contains("internal environment parameter"),
        "{error:?}"
    );
    assert_dispatch_output(&program, b"79oc79oc");
    assert_native_dispatch_output(&program, "79oc79oc");
}

#[test]
fn covariant_interface_returns_preserve_checked_nullable_and_borrowed_results() {
    let program = checked_hir(include_str!(
        "../../tests/fixtures/virtual_interface_return_contracts.doria"
    ));
    assert_dispatch_output(&program, b"t9otf9o07c7t9otf9o08c7c");
    assert_native_dispatch_output(&program, "t9otf9o07c7t9otf9o08c7c");
}

#[test]
fn covariant_nullable_returns_preserve_each_existing_value_carrier() {
    let program = checked_hir(include_str!(
        "../../tests/fixtures/virtual_nullable_return_contracts.doria"
    ));
    assert_dispatch_output(&program, b"7truetruestruetrue267truetruestruetrue26");
    assert_native_dispatch_output(&program, "7truetruestruetrue267truetruestruetrue26");
}

#[test]
fn covariant_interface_returns_preserve_owner_and_method_specializations() {
    let program = checked_hir(include_str!(
        "../../tests/fixtures/virtual_generic_interface_returns.doria"
    ));
    let mut native = crate::mir_lowering::lower_program(&program).unwrap();
    let child = native
        .classes
        .iter()
        .find(|class| class.declaration_name == "Child")
        .unwrap()
        .id;
    let root = native
        .classes
        .iter()
        .find(|class| class.declaration_name == "Root")
        .unwrap()
        .id;
    let leaf = native
        .classes
        .iter()
        .find(|class| class.declaration_name == "Leaf")
        .unwrap()
        .id;
    native.classes[root.0].virtual_methods[0].implementation = None;
    native.classes[child.0].virtual_methods[0].implementation = None;
    native.classes[leaf.0].virtual_methods[0].return_type =
        native.classes[root.0].virtual_methods[0].family.return_type;
    let error = crate::mir_validation::validate_program(&native).unwrap_err();
    assert!(
        error
            .message
            .contains("widens its inherited return contract"),
        "{error:?}"
    );
    assert_dispatch_output(&program, b"7");
    assert_native_dispatch_output(&program, "7");
}

#[test]
fn covariant_interface_returns_preserve_collection_and_inline_copy_ownership() {
    let program = checked_hir(include_str!(
        "../../tests/fixtures/virtual_nominal_value_returns.doria"
    ));
    assert_dispatch_output(&program, b"truetrue78truefalsetruetrue2truefalse");
}

#[test]
fn covariant_nominal_value_returns_preserve_native_profiles() {
    let program = checked_hir(include_str!(
        "../../tests/fixtures/virtual_nominal_value_returns.doria"
    ));
    assert_native_dispatch_output(&program, "truetrue78truefalsetruetrue2truefalse");
}

#[test]
fn covariant_collection_views_preserve_borrowed_storage_and_nominal_witnesses() {
    let program = checked_hir(include_str!(
        "../../tests/fixtures/virtual_borrowed_collection_views.doria"
    ));
    assert_dispatch_output(&program, b"12787823456");
    assert_native_dispatch_output(&program, "12787823456");
}

#[test]
fn covariant_php_views_preserve_checked_source_without_native_ir() {
    let program = checked_hir(include_str!(
        "../../tests/fixtures/virtual_borrowed_collection_views.doria"
    ));
    assert_php_dispatch_output(&program, None, b"12787823456");
}

#[test]
fn covariant_php_views_preserve_inherited_witnesses_defaults_and_core_protocols() {
    let program = checked_hir(include_str!(
        "../../tests/fixtures/covariant_dispatch_protocols.doria"
    ));
    assert_dispatch_output(&program, b"5516p978");
    assert_php_dispatch_output(&program, None, b"5516p978");
    assert_native_dispatch_output(&program, "5516p978");
}

#[test]
fn inline_nominal_interface_tests_preserve_zero_payloads_and_single_evaluation() {
    let program = checked_hir(include_str!(
        "../../tests/fixtures/inline_nominal_interface_type_tests.doria"
    ));
    assert_dispatch_output(&program, b"truefalseitrueotrueofalse");
    assert_native_dispatch_output(&program, "truefalseitrueotrueofalse");
}

#[test]
fn covariant_return_contracts_reject_constraint_only_primitive_erasure() {
    let Err(errors) = crate::check_source(
        "primitive-return-erasure.doria",
        r#"
class Label implements Displayable { function toString(): string { return "label"; } }
open class Root { open function value(): Displayable { return new Label(); } }
class Child extends Root { override function value(): string { return "child"; } }
function main(): void {}
"#,
    ) else {
        panic!("primitive capability cannot authorize an erased return");
    };
    assert!(
        errors.iter().any(|error| error.code == "E0729"),
        "{errors:?}"
    );
}
