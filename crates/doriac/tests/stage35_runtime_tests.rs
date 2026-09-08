use doriac::semantics::CatchCoverage;
use doriac::{
    mir,
    semantics::CallableTarget,
    types::{InterfaceType, ResolvedType},
};

const ERROR_CARRIER_SOURCE: &str =
    include_str!("../../../examples/native/main_stage35_interface_error_carrier.doria");

#[test]
fn interface_erasure_keeps_headerless_layout_and_constrained_calls_direct() {
    let program = doriac::lower_source_to_mir(
        "interface-structure.doria",
        include_str!("../../../examples/native/main_stage35_interface_structure.doria"),
    )
    .unwrap();
    let number = program
        .classes
        .iter()
        .find(|class| class.name == "Number")
        .unwrap();
    assert_eq!(number.layout.size, 8);
    assert_eq!(number.layout.properties.len(), 1);
    assert_eq!(number.layout.properties[0].offset, 0);
    assert_eq!(
        program
            .interface_vtables
            .iter()
            .filter(|table| table.implementing_type == mir::ImplementingType::Class(number.id))
            .count(),
        1
    );
    assert!(program.closure_descriptors.is_empty());
    for name in ["concrete", "constrained", "erased"] {
        let function = program
            .functions
            .iter()
            .find(|function| function.name.starts_with(name))
            .unwrap();
        let indirect = function.blocks.iter().any(|block| {
            matches!(
                block.terminator,
                mir::Terminator::IndirectCall { .. } | mir::Terminator::CheckedIndirectCall { .. }
            )
        });
        assert_eq!(indirect, name == "erased", "{function}");
    }
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(output.stdout, b"7\n");
}

#[test]
fn durable_interface_fixtures_preserve_php_execution_and_cleanup() {
    use std::io::Write;
    use std::process::Command;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for path in include_str!("fixtures/native_parity_examples.txt")
        .lines()
        .filter(|path| path.starts_with("examples/native/main_stage35_interface_"))
    {
        let source = std::fs::read_to_string(root.join(path)).unwrap();
        let hir = doriac::lower_source(path, source).unwrap();
        let mir = doriac::mir_lowering::lower_program(&hir).unwrap();
        let php = doriac::codegen_php::generate(&hir, Some(&mir)).unwrap();
        let stem = std::path::Path::new(path).file_stem().unwrap();
        let fixture = root
            .join("crates/doriac/tests/fixtures/native_io")
            .join(stem);
        let file = std::env::temp_dir().join(format!(
            "doria-interface-{}-{}.php",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        std::fs::File::create_new(&file)
            .unwrap()
            .write_all(php.as_bytes())
            .unwrap();
        let lint = Command::new("php").arg("-l").arg(&file).output();
        let run = Command::new("php").arg(&file).output();
        std::fs::remove_file(&file).unwrap();
        for (lint, output) in [(true, lint), (false, run)] {
            let output = output.expect("PHP is required for interface compatibility parity");
            assert!(output.status.success(), "{path}: {output:?}");
            assert!(output.stderr.is_empty(), "{path}: {output:?}");
            if !lint {
                assert_eq!(
                    output.stdout,
                    std::fs::read(fixture.join("expected_stdout")).unwrap(),
                    "{path}"
                );
            }
        }
    }
}

fn interface_dispatch_mir() -> mir::Program {
    let mut program = doriac::lower_source_to_mir(
        "interface-entry.doria",
        r#"
interface Marker {}
class Box implements Marker { function read(): int { return 42; } }
function main(): int { return 42; }
"#,
    )
    .unwrap();
    let interface = program
        .interface_types
        .iter()
        .find(|ty| ty.name == "Marker")
        .unwrap()
        .id;
    let class = program
        .classes
        .iter()
        .find(|class| class.name == "Box")
        .unwrap()
        .id;
    let constructor = program.classes[class.0].constructor;
    let implementation = program
        .functions
        .iter()
        .find(|function| {
            function
                .method
                .as_ref()
                .is_some_and(|method| method.class == class && method.name == "read")
        })
        .unwrap()
        .id;
    let vtable = program
        .interface_vtable(mir::ImplementingType::Class(class), interface)
        .unwrap();
    let signature = mir::FunctionTypeId(program.function_types.len());
    let integer = mir::Type::Scalar(mir::ScalarType::Integer(
        doriac::numeric::IntegerType::Int64,
    ));
    program.function_types.push(mir::FunctionType {
        id: signature,
        invocation_mode: mir::FunctionInvocationMode::Readonly,
        parameters: vec![mir::FunctionParameter {
            mode: mir::FunctionParameterMode::Readonly,
            ty: mir::Type::Interface(interface),
        }],
        return_type: mir::ReturnType::Value(integer),
        return_borrow: None,
        checked_effects: vec![],
        ambient_checked_effects: vec![],
        test_assertion_checked_effects: vec![],
    });
    program.interface_types[interface.0]
        .methods
        .push(mir::InterfaceMethod {
            requirement: Default::default(),
            name: "read".into(),
            arguments: vec![],
            signature,
            writable_receiver: false,
            exact_dynamic_return: false,
        });
    let receiver = mir::Local {
        id: mir::LocalId(0),
        name: "receiver".into(),
        ty: mir::Type::Interface(interface),
        writable: false,
        owned: false,
        synthetic: true,
    };
    let mut entry = program.functions[program.entry.0].clone();
    entry.id = mir::FunctionId(program.functions.len());
    entry.name = "interface-entry".into();
    entry.params = vec![receiver.id];
    entry.parameter_modes = vec![mir::FunctionParameterMode::Readonly];
    entry.locals = vec![receiver.clone()];
    entry.blocks[0].terminator = mir::Terminator::Return(mir::Rvalue::Value(
        mir::ValueExpression::Integer(mir::IntegerExpression::Call {
            ty: doriac::numeric::IntegerType::Int64,
            function: implementation,
            args: vec![mir::Rvalue::Class(
                mir::ClassExpression::InterfaceReceiver {
                    class,
                    receiver: receiver.id,
                    vtable,
                },
            )],
        }),
    ));
    program.interface_vtables[vtable.0].methods.push(entry.id);
    program.functions.push(entry);

    let main = &mut program.functions[program.entry.0];
    main.locals = vec![
        mir::Local {
            owned: true,
            ..receiver
        },
        mir::Local {
            id: mir::LocalId(1),
            name: "result".into(),
            ty: integer,
            writable: false,
            owned: false,
            synthetic: true,
        },
    ];
    main.blocks = vec![
        mir::BasicBlock {
            id: mir::BlockId(0),
            statements: vec![mir::Statement::AssignLocal {
                target: mir::LocalId(0),
                value: mir::Rvalue::Interface(mir::InterfaceExpression {
                    interface,
                    value: mir::InterfaceValue::FromClass {
                        object: Box::new(mir::ClassExpression::New {
                            class,
                            concrete_class: class,
                            properties: vec![],
                            constructor,
                            args: vec![],
                        }),
                        vtable,
                    },
                }),
            }],
            terminator: mir::Terminator::IndirectCall {
                callee: mir::IndirectCallee::InterfaceMethod {
                    receiver: mir::LocalId(0),
                    interface,
                    slot: 0,
                },
                function_type: signature,
                invocation_mode: mir::FunctionInvocationMode::Readonly,
                args: vec![mir::Rvalue::Interface(mir::InterfaceExpression {
                    interface,
                    value: mir::InterfaceValue::Local {
                        local: mir::LocalId(0),
                        transfer: false,
                    },
                })],
                result: Some(mir::LocalId(1)),
                continuation: mir::BlockId(1),
                span: Default::default(),
            },
        },
        mir::BasicBlock {
            id: mir::BlockId(1),
            statements: vec![mir::Statement::DropError {
                local: mir::LocalId(0),
            }],
            terminator: mir::Terminator::Return(mir::Rvalue::Value(mir::ValueExpression::Integer(
                mir::IntegerExpression::Use {
                    ty: doriac::numeric::IntegerType::Int64,
                    operand: mir::Operand::Local(mir::LocalId(1)),
                },
            ))),
        },
    ];
    main.entry_block = mir::BlockId(0);
    program
}

#[test]
fn interface_dispatch_uses_an_ordinary_entry_without_a_closure_environment() {
    let program = interface_dispatch_mir();
    doriac::mir_validation::validate_program(&program).unwrap();
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(output.exit_status, 42);
    assert!(output.stdout.is_empty());
    assert!(program.closure_descriptors.is_empty());
    assert!(!doriac::codegen_cranelift::lower_mir_to_object(&program)
        .unwrap()
        .is_empty());
    #[cfg(feature = "llvm-backend")]
    assert!(!doriac::codegen_llvm::lower_mir_to_object(&program)
        .unwrap()
        .is_empty());
}

#[test]
fn interface_dispatch_rejects_slot_receiver_and_entry_abi_mismatches() {
    let program = interface_dispatch_mir();
    let mut bad_slot = program.clone();
    let mir::Terminator::IndirectCall {
        callee: mir::IndirectCallee::InterfaceMethod { slot, .. },
        ..
    } = &mut bad_slot.functions[program.entry.0].blocks[0].terminator
    else {
        unreachable!()
    };
    *slot = 1;
    assert!(doriac::mir_validation::validate_program(&bad_slot).is_err());
    let mut bad_receiver = program.clone();
    let mir::Terminator::IndirectCall { args, .. } =
        &mut bad_receiver.functions[program.entry.0].blocks[0].terminator
    else {
        unreachable!()
    };
    let mir::Rvalue::Interface(mir::InterfaceExpression {
        value: mir::InterfaceValue::Local { transfer, .. },
        ..
    }) = &mut args[0]
    else {
        unreachable!()
    };
    *transfer = true;
    assert!(doriac::mir_validation::validate_program(&bad_receiver).is_err());
    let mut unproved = program.clone();
    let mut unregistered = unproved.functions.last().unwrap().clone();
    unregistered.id = mir::FunctionId(unproved.functions.len());
    unproved.functions.push(unregistered);
    assert!(doriac::mir_validation::validate_program(&unproved)
        .unwrap_err()
        .message
        .contains("exact entry-vtable proof"));
    let mut expired = program.clone();
    expired.functions[program.entry.0].blocks[0]
        .statements
        .push(mir::Statement::DropError {
            local: mir::LocalId(0),
        });
    assert!(doriac::mir_validation::validate_program(&expired)
        .unwrap_err()
        .message
        .contains("after its ownership ended"));
    let mut bad_entry = program;
    bad_entry.functions.last_mut().unwrap().parameter_modes[0] = mir::FunctionParameterMode::Take;
    assert!(doriac::mir_validation::validate_program(&bad_entry).is_err());
}

#[test]
fn error_interface_preserves_dynamic_identity_and_nullable_pairs() {
    let program =
        doriac::lower_source_to_mir("interface-error.doria", ERROR_CARRIER_SOURCE).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(
        output.stdout,
        include_bytes!("fixtures/native_io/main_stage35_interface_error_carrier/expected_stdout")
    );
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
}

#[test]
fn interface_vtable_validation_rejects_incompatible_payload_and_metadata() {
    let program =
        doriac::lower_source_to_mir("interface-error.doria", ERROR_CARRIER_SOURCE).unwrap();
    let mut duplicate = program.clone();
    duplicate
        .interface_vtables
        .push(duplicate.interface_vtables[0].clone());
    assert!(doriac::mir_validation::validate_program(&duplicate).is_err());

    let mut missing = program.clone();
    missing.interface_vtables.clear();
    assert!(doriac::mir_validation::validate_program(&missing).is_err());

    let mut wrong = program.clone();
    wrong.interface_vtables[0].error_descriptor = None;
    assert!(doriac::mir_validation::validate_program(&wrong).is_err());

    let mut premature_collection = program;
    premature_collection.interface_vtables[0].implementing_type =
        mir::ImplementingType::Collection(mir::CollectionTypeId(0));
    assert!(doriac::mir_validation::validate_program(&premature_collection).is_err());
}

#[test]
fn erased_calls_publish_the_requirement_and_specialized_graph() {
    let source = r#"
interface Root<T> { function read(T $fallback): T; }
interface Child extends Root<int> {}
function read(Child $value): int { return $value->read(1); }
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("interface-facts.doria", source).unwrap();
    let interface = analysis
        .info
        .contracts
        .interface_specializations
        .iter()
        .find(|fact| fact.specialization.name == "Child")
        .unwrap();
    assert!(interface.valid);
    assert_eq!(interface.ancestors.len(), 1);
    assert_eq!(interface.ancestors[0].name, "Root");
    let requirement = &interface.requirements[0];
    assert_eq!(
        requirement.signature.parameters[0].r#type,
        ResolvedType::Integer(doriac::numeric::IntegerType::Int64)
    );
    assert_eq!(
        requirement.signature.return_type,
        ResolvedType::Integer(doriac::numeric::IntegerType::Int64)
    );
    assert!(analysis.info.call_targets.values().any(|target| {
        matches!(target, CallableTarget::InterfaceMethod { interface, requirement: origin, .. }
            if interface.name == "Child" && *origin == requirement.origins[0].declaration)
    }));
}

#[test]
fn constrained_interface_specialization_stays_an_erased_requirement_call() {
    let target = CallableTarget::ConstrainedMethod {
        receiver: ResolvedType::TypeParameter("T".into()),
        method_name: "read".into(),
        requirement: doriac::source::Span::default(),
        implementations: Vec::new(),
    };
    let specialized =
        target.specialize(|_| ResolvedType::Interface(InterfaceType::new("Readable", vec![])));
    assert!(
        matches!(specialized, Some(CallableTarget::InterfaceMethod { interface, .. }) if interface.name == "Readable")
    );
}

#[test]
fn interface_carrier_validation_rejects_unknown_and_mismatched_views() {
    let program = doriac::lower_source_to_mir(
        "interface-mir.doria",
        "function main(): void { ?Error $value = null; }",
    )
    .unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    for register_view in [false, true] {
        let mut invalid = program.clone();
        if register_view {
            invalid.interface_types.push(mir::InterfaceType {
                id: mir::InterfaceTypeId(1),
                name: "Other".into(),
                ancestors: vec![],
                methods: vec![],
            });
        }
        let function = &mut invalid.functions[invalid.entry.0];
        let value = function
            .blocks
            .iter_mut()
            .flat_map(|block| &mut block.statements)
            .find_map(|statement| match statement {
                mir::Statement::AssignLocal {
                    value: mir::Rvalue::NullableInterface(value),
                    ..
                } => Some(value),
                _ => None,
            })
            .unwrap();
        value.interface = mir::InterfaceTypeId(1);
        assert!(doriac::mir_validation::validate_program(&invalid).is_err());
    }
}

const ERROR_HIERARCHY: &str = r#"
open class Failure implements Error {
    function __construct(string $message) {}
}
class Missing extends Failure {
    function __construct() { parent::__construct("missing"); }
}
"#;

#[test]
fn nominal_catch_facts_distinguish_partial_and_complete_coverage() {
    let source = format!(
        r#"{ERROR_HIERARCHY}
function fail(): void throws Failure {{ throw new Missing(); }}
function main(): void {{
    try {{ fail(); }} catch (Missing) {{}} catch (Failure) {{}}
}}
"#
    );
    let hir = doriac::lower_source("catch-coverage.doria", source)
        .expect("a descendant catch may handle part of an open parent effect");
    let mut catches = hir.semantic_info.catch_coverage.iter().collect::<Vec<_>>();
    catches.sort_by_key(|(span, _)| span.start);
    assert_eq!(catches.len(), 2);
    assert_eq!(catches[0].1.len(), 1);
    assert_eq!(catches[1].1.len(), 1);
    assert_eq!(catches[0].1.values().next(), Some(&CatchCoverage::Partial));
    assert_eq!(catches[1].1.values().next(), Some(&CatchCoverage::Complete));
}

#[test]
fn parent_catches_preserve_exceptional_ownership_states() {
    let source = format!(
        r#"{ERROR_HIERARCHY}
class Payload {{}}
function consume(take Payload $payload): void {{}}
function inspect(Payload $payload): void {{}}
function fail(): void throws Missing {{ throw new Missing(); }}
function invalid(): void {{
    let writable $payload = new Payload();
    try {{
        consume($payload);
        fail();
        $payload = new Payload();
    }} catch (Failure) {{}}
    inspect($payload);
}}
"#
    );
    let errors = doriac::check_source("parent-catch.doria", source)
        .expect_err("the parent catch observes the move before the child Error");
    assert!(
        errors.iter().any(|error| error.code == "E0470"),
        "{errors:#?}"
    );
}

#[test]
fn partial_catches_preserve_both_ownership_paths() {
    for effect in ["Failure", "Error"] {
        let source = format!(
            r#"{ERROR_HIERARCHY}
class Payload {{}}
function consume(take Payload $payload): void {{}}
function inspect(Payload $payload): void {{}}
function fail(): void throws {effect} {{ throw new Missing(); }}
function invalid(): void {{
    let $payload = new Payload();
    try {{ fail(); }}
    catch (Missing) {{ consume($payload); }}
    catch (Error) {{}}
    inspect($payload);
}}
"#
        );
        let errors = doriac::check_source("partial-catch.doria", source)
            .expect_err("a partial catch can consume the value before the join");
        assert!(
            errors.iter().any(|error| error.code == "E0470"),
            "{errors:#?}"
        );
    }
}

#[test]
fn nominal_catches_preserve_constructor_definite_initialization() {
    for (effect, catch) in [
        ("Missing", "Failure"),
        ("Failure", "Missing"),
        ("Error", "Missing"),
    ] {
        let source = format!(
            r#"{ERROR_HIERARCHY}
function load(): string throws {effect} {{ throw new Missing(); }}
class NotReady {{
    string $value;
    function __construct() throws {effect} {{
        try {{ $this->value = load(); }} catch ({catch}) {{}}
    }}
}}
"#
        );
        let errors = doriac::check_source("catch-initialization.doria", source)
            .expect_err("a caught RHS failure leaves the property uninitialized");
        assert!(
            errors.iter().any(|error| error.code == "E0500"),
            "{errors:#?}"
        );
    }
}
