#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution.rs"]
mod native_execution;

use doriac::backend::{BackendTarget, NativeProfile};
use doriac::class_layout::ClassId;
use doriac::inline_value::{InlineValue, InlineValueType};
use doriac::mir;

const SOURCE: &str = r#"
use Doria\Std\Io\{RegistrationKey, ReadableFile, WritableFile};
function forward(RegistrationKey<ReadableFile> $key): RegistrationKey<ReadableFile> {
    let $copy = $key;
    let $returned = identity($copy);
    let $again = identity($key);
    return $returned;
}
function identity<T>(T $key): T {
    return $key;
}
function probe(?RegistrationKey<ReadableFile> $key = null): bool {
    let $copy = $key;
    if ($copy != null) {
        let $returned = forward($copy);
        if ($key != null) {
            let $again = forward($key);
            return true;
        }
    }
    return $key == null;
}
function other(?RegistrationKey<WritableFile> $key): ?RegistrationKey<WritableFile> {
    let $copy = $key;
    return $key;
}
function main(): void {
    let $ready = probe();
    echo $ready . ":" . (other(null) == null) . "\n";
}
"#;

fn program() -> mir::Program {
    doriac::lower_source_to_mir("registration-key.doria", SOURCE).unwrap()
}

fn key_types(program: &mir::Program) -> Vec<InlineValueType> {
    program
        .classes
        .iter()
        .filter_map(|class| {
            class
                .inline_copy_type()
                .filter(|ty| matches!(ty, InlineValueType::RegistrationKey(_)))
        })
        .collect()
}

#[test]
fn keys_and_nullable_keys_are_copy_with_exact_generic_identity() {
    doriac::check_source("registration-key.doria", SOURCE).unwrap();
    let program = program();
    let keys = key_types(&program);
    assert_eq!(keys.len(), 2);
    assert_ne!(keys[0], keys[1]);
    assert_ne!(keys[0].type_id(), keys[1].type_id());
    for ty in keys {
        assert_eq!(ty.storage_bytes(), 16);
        let InlineValueType::RegistrationKey(id) = ty else {
            unreachable!()
        };
        let class = &program.classes[id.0];
        let [mir::Type::Class(stream)] = class.arguments.as_slice() else {
            panic!("registration key must retain its exact stream type")
        };
        assert!(matches!(
            program.classes[stream.0].declaration_name.as_str(),
            doriac::compiler_known_stream::READABLE_FILE
                | doriac::compiler_known_stream::WRITABLE_FILE
        ));
        assert!(class.constructor.is_none() && class.destructor.is_none());
        assert!(class.properties.is_empty() && class.runtime_payload.is_none());
        for local in program
            .functions
            .iter()
            .flat_map(|function| &function.locals)
        {
            if matches!(local.ty, mir::Type::Scalar(mir::ScalarType::Inline(actual))
                | mir::Type::NullableScalar(mir::ScalarType::Inline(actual)) if actual == ty)
            {
                assert!(!local.owned);
            }
        }
    }
}

#[test]
fn keys_do_not_convert_between_stream_types_or_to_integers() {
    for invalid in [
        SOURCE.replace("return $returned;", "return other($returned);"),
        SOURCE.replace("return $returned;", "return 1;"),
        SOURCE.replace(
            "return $returned;",
            "int $bits = $returned; return $returned;",
        ),
        SOURCE.replace(
            "return $returned;",
            "return new RegistrationKey<ReadableFile>();",
        ),
        SOURCE.replace(
            "?RegistrationKey<ReadableFile> $key = null",
            "?RegistrationKey<ReadableFile> $key = 1",
        ),
        SOURCE.replace("RegistrationKey<ReadableFile>", "RegistrationKey"),
    ] {
        assert!(
            doriac::check_source("invalid-key.doria", &invalid).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn specialization_arguments_retain_nested_and_erased_types_without_name_decoding() {
    let source = r#"
use Doria\Std\Io\RegistrationKey;
interface Named { function getName(): string; }
class Leaf {}
class Envelope<T, U> {}
function observe(?Iterable<Leaf> $source): void {}
function observeIterator(?Iterator<Leaf> $source): void {}
function probe(
    ?RegistrationKey<Envelope<Leaf, List<Leaf>>> $nested,
    ?RegistrationKey<Named> $erased,
    ?RegistrationKey<?int> $nullable,
    ?RegistrationKey<function(int): string> $callback
): void {}
function main(): void { probe(null, null, null, null); observe(null); observeIterator(null); }
"#;
    let mut program = doriac::lower_source_to_mir("key-arguments.doria", source).unwrap();
    let leaf = program
        .classes
        .iter()
        .find(|class| class.declaration_name == "Leaf")
        .unwrap()
        .id;
    let envelope = program
        .classes
        .iter()
        .find(|class| class.declaration_name == "Envelope")
        .unwrap();
    let [mir::Type::Class(element), mir::Type::Collection(list)] = envelope.arguments.as_slice()
    else {
        panic!("ordinary generic classes must retain both concrete arguments")
    };
    assert_eq!(*element, leaf);
    assert_eq!(
        program.collection_types[list.0].value,
        mir::Type::Class(leaf)
    );
    assert!(program.interface_vtables.iter().any(|table| {
        table.implementing_type == mir::ImplementingType::Collection(*list)
            && program.interface_types[table.interface.0].name == "Iterable<Leaf>"
    }));
    assert!(program.interface_vtables.iter().any(|table| {
        table.implementing_type == mir::ImplementingType::CollectionIterator(*list)
            && program.interface_types[table.interface.0].name == "Iterator<Leaf>"
    }));
    let envelope = envelope.id;
    let named = program
        .interface_types
        .iter()
        .find(|interface| interface.name == "Named")
        .unwrap()
        .id;
    let arguments = key_types(&program)
        .iter()
        .map(|ty| {
            let InlineValueType::RegistrationKey(id) = ty else {
                unreachable!()
            };
            program.classes[id.0].arguments.clone()
        })
        .collect::<Vec<_>>();
    let callback = program
        .function_types
        .iter()
        .find(|signature| {
            signature.parameters.len() == 1
                && signature.parameters[0].ty
                    == mir::Type::Scalar(mir::ScalarType::Integer(
                        doriac::numeric::IntegerType::Int64,
                    ))
                && signature.return_type == mir::ReturnType::Value(mir::Type::String)
        })
        .unwrap()
        .id;
    assert_eq!(arguments.len(), 4);
    for expected in [
        mir::Type::Class(envelope),
        mir::Type::Interface(named),
        mir::Type::NullableScalar(mir::ScalarType::Integer(
            doriac::numeric::IntegerType::Int64,
        )),
        mir::Type::Function(callback),
    ] {
        assert!(arguments.contains(&vec![expected]));
    }
    for class in &mut program.classes {
        if !class.arguments.is_empty() {
            class.name = format!("opaque-specialization-{}", class.id.0);
        }
    }
    doriac::mir_validation::validate_program(&program).unwrap();
    assert_eq!(
        doriac::mir_interpreter::interpret(&program)
            .unwrap()
            .exit_status,
        0
    );
    doriac::codegen_cranelift::lower_mir_to_object(&program).unwrap();
    #[cfg(feature = "llvm-backend")]
    doriac::codegen_llvm::lower_mir_to_object(&program).unwrap();
}

#[test]
fn folded_null_constants_preserve_the_declared_registration_type() {
    let source = r#"
use Doria\Std\Io\{RegistrationKey, ReadableFile, WritableFile};
const ?RegistrationKey<ReadableFile> EMPTY = null;
const ?RegistrationKey<ReadableFile> COPY = EMPTY;
function accepts(?RegistrationKey<ReadableFile> $key = COPY): void {}
function main(): void { accepts(COPY); accepts(); }
"#;
    doriac::check_source("key-constant.doria", source).unwrap();
    let wrong = source.replace(
        "function accepts(?RegistrationKey<ReadableFile>",
        "function accepts(?RegistrationKey<WritableFile>",
    );
    assert!(doriac::check_source("wrong-key-constant.doria", wrong).is_err());
    let wrong = source.replace(
        "const ?RegistrationKey<ReadableFile> COPY",
        "const ?RegistrationKey<WritableFile> COPY",
    );
    assert!(doriac::check_source("wrong-key-constant.doria", wrong).is_err());
    doriac::lower_source_to_mir("key-constant.doria", source).unwrap();
}

#[test]
fn generic_copy_keys_keep_identity_in_constant_enum_payloads_and_defaults() {
    let source = r#"
use Doria\Std\Io\{RegistrationKey, ReadableFile};
enum State { case Empty(?RegistrationKey<ReadableFile> $key); }
const State EMPTY = State::Empty(null);
class Settings { static State $initial = EMPTY; }
function empty(State $state = EMPTY): bool {
    return match ($state) { State::Empty($key) => $key == null };
}
function main(): void {
    echo empty() . ":" . empty(EMPTY) . ":" . empty(Settings::initial) . "\n";
}
"#;
    assert_backend_parity("key-enum.doria", source, "true:true:true\n");
}

#[test]
fn nullable_key_copy_transport_executes_on_every_backend() {
    assert_backend_parity("registration-key.doria", SOURCE, "true:true\n");
}

#[test]
fn nullable_key_storage_and_generic_captures_share_copy_transport() {
    let source = r#"
use Doria\Std\Io\{RegistrationKey, ReadableFile};
class Holder { function __construct(?RegistrationKey<ReadableFile> $key) {} }
function identity<T>(T $value): T { return $value; }
function capture<T>(?RegistrationKey<T> $value): function(): ?RegistrationKey<T> {
    return fn() with (take $value) => $value;
}
function main(): void {
    ?RegistrationKey<ReadableFile> $key = null;
    let $holder = new Holder(identity($key));
    List<?RegistrationKey<ReadableFile>> $keys = [$key, $holder->key];
    let $read = capture($key);
    echo ($key == null) . ":" . ($holder->key == null) . ":" . ($keys[0] == null);
    echo ":" . ($read() == null) . ":" . ($read() == null) . "\n";
}
"#;
    assert_backend_parity("key-storage.doria", source, "true:true:true:true:true\n");
}

#[test]
fn generic_key_summaries_name_specialized_parameters_and_capture_inputs() {
    let source = r#"
use Doria\Std\Io\{RegistrationKey, ReadableFile};
class Item { function __construct(int $value) {} }
function identity<T>(T $value): T { return $value; }
function second<T>(T $first, T $second): T { return identity($second); }
function capture<T>(?RegistrationKey<T> $first, ?RegistrationKey<T> $second): function(): ?RegistrationKey<T> {
    return function(): ?RegistrationKey<T> with (take $first, take $second) {
        if ($first == null) { return identity($second); }
        return identity($second);
    };
}
function main(): void {
    ?RegistrationKey<ReadableFile> $key = null;
    let $selected = second($key, $key);
    let $read = capture($key, $selected);
    let $copy = identity(7);
    let $item = new Item($copy);
    let $view = identity($item);
    echo $view->value . ":" . ($read() == null) . "\n";
}
"#;
    let program = doriac::lower_source_to_mir("generic-key-summary.doria", source).unwrap();
    let plan = |function: &mir::Function| {
        function.blocks[0]
            .statements
            .iter()
            .find_map(|statement| match statement {
                mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(plan)) => {
                    Some(plan)
                }
                _ => None,
            })
            .map(|plan| plan.returned_selectors.clone())
    };
    for function in &program.functions {
        if matches!(
            function.return_type,
            mir::ReturnType::Value(mir::Type::NullableScalar(mir::ScalarType::Inline(
                InlineValueType::RegistrationKey(_)
            )))
        ) {
            let expected = if let Some(closure) = &function.closure {
                closure.capture_locals[1].1
            } else if function.name == "second" {
                function.params[1]
            } else {
                assert_eq!(function.name, "identity");
                function.params[0]
            };
            assert_eq!(
                plan(function).expect("returned registration keys require selector facts"),
                vec![mir::RetainedStorageSelector::Input(expected)]
            );
        } else if function.name == "identity" {
            assert!(plan(function).is_none_or(|selectors| selectors.is_empty()));
        }
    }
    let closure = program
        .functions
        .iter()
        .find(|function| function.closure.is_some())
        .unwrap();
    for wrong in [
        None,
        Some(closure.closure.as_ref().unwrap().capture_locals[0].1),
        Some(closure.params[0]),
    ] {
        let mut invalid = program.clone();
        let function = &mut invalid.functions[closure.id.0];
        let mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(plan)) =
            &mut function.blocks[0].statements[0]
        else {
            unreachable!()
        };
        plan.returned_selectors = wrong
            .into_iter()
            .map(mir::RetainedStorageSelector::Input)
            .collect();
        assert!(doriac::mir_validation::validate_program(&invalid).is_err());
    }
    assert_backend_parity("generic-key-summary.doria", source, "7:true\n");
}

#[test]
fn generic_nullable_values_keep_null_through_function_and_method_helpers() {
    let source = r#"
function relay<T>(T $value): T { T $copy = $value; return $copy; }
class Holder<T> {
    function __construct(take T $value) {}
    function getValue(): T { return $this->value; }
    function relayClass(T $value): T { T $copy = $value; return $copy; }
    function relay<U>(U $value): U { U $copy = $value; return $copy; }
}
class Utility {
    static function relay<T>(T $value): T { T $copy = $value; return $copy; }
}
function main(): void {
    ?int $value = null;
    let $holder = new Holder<?int>($value);
    let $direct = relay($value);
    let $stored = $holder->getValue();
    let $method = $holder->relay($value);
    let $class = $holder->relayClass($value);
    let $static = Utility::relay($value);
    echo (relay($direct) == null) . ":" . (relay($stored) == null);
    echo ":" . (relay($method) == null) . ":" . (relay($class) == null);
    echo ":" . (relay($static) == null) . "\n";
    echo relay(7) . ":" . (relay(true)) . "\n";
}
"#;
    assert_backend_parity(
        "generic-nullable-flow.doria",
        source,
        "true:true:true:true:true\n7:true\n",
    );
}

#[test]
fn grouped_nullable_values_keep_presence_through_helpers_and_promoted_fields() {
    let source = r#"
function relay((?int) $value): ((?int)) { (?int) $copy = $value; return $copy; }
class Holder {
    function __construct((?int) $value) {}
    function getValue(): (?int) { (?int) $copy = $this->value; return $copy; }
}
function main(): void {
    (?int) $value = null;
    let $holder = new Holder($value);
    let $direct = relay($value);
    let $stored = $holder->getValue();
    echo "" . (relay($direct) == null) . ":" . (relay($stored) == null) . ":" . (relay(7) ?? 0) . "\n";
}
"#;
    assert_backend_parity("grouped-nullable-flow.doria", source, "true:true:7\n");
}

#[test]
fn grouped_primitive_return_signatures_preserve_nullable_and_present_values() {
    for (ty, literal) in [
        ("int", "7"),
        ("float", "2.5"),
        ("bool", "true"),
        ("string", "\"text\""),
    ] {
        let source = format!(
            r#"
function optional((?{ty}) $value): ((?{ty})) {{ return $value; }}
function present(({ty}) $value): (({ty})) {{ return $value; }}
function main(): void {{
    let $missing = optional(null);
    let $value = optional({literal});
    echo "" . ($missing == null) . ":" . ($value != null);
    echo ":" . (($value ?? {literal}) == {literal});
    echo ":" . (present({literal}) == {literal}) . "\n";
}}
"#
        );
        assert_backend_parity(
            "grouped-primitive-return.doria",
            &source,
            "true:true:true:true\n",
        );
    }
}

fn assert_backend_parity(name: &str, source: &str, expected: &str) {
    let program = doriac::lower_source_to_mir(name, source).unwrap();
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(output.stdout, expected.as_bytes());
    assert_eq!(output.exit_status, 0);
    assert!(output.stderr.is_empty());
    native_execution::assert_native_execution(&program, NativeProfile::Fast, expected);
    #[cfg(feature = "llvm-backend")]
    native_execution::assert_native_execution(&program, NativeProfile::Release, expected);
    let artifact = doriac::compile_source(name, source, BackendTarget::Php).unwrap();
    let directory = native_execution::ExecutionDirectory::new();
    let path = directory.path().join("key.php");
    doriac::backend::publish_output(&path, artifact).unwrap();
    let output = std::process::Command::new("php")
        .args(["-d", "ffi.enable=0"])
        .arg(path)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, expected.as_bytes());
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[test]
fn present_keys_use_the_same_inline_copy_transport_without_a_public_constructor() {
    for bits in [0, (17_u128 << 64) | 23, u128::MAX] {
        assert_present_key_transport(bits, false, "RegistrationKey<ReadableFile>");
    }
}

#[test]
fn present_static_keys_preserve_nullability_and_all_payload_bits() {
    for bits in [0, (17_u128 << 64) | 23, u128::MAX] {
        assert_present_key_transport(bits, true, "RegistrationKey<ReadableFile>");
    }
}

#[test]
fn typed_transfer_keys_preserve_all_bits_through_nullable_copy_transport() {
    for name in ["ReadTransferKey", "WriteTransferKey"] {
        for bits in [0, (17_u128 << 64) | 23, u128::MAX] {
            for static_storage in [false, true] {
                assert_present_key_transport(bits, static_storage, name);
            }
        }
    }
}

#[test]
fn transfer_keys_are_distinct_from_each_other_registrations_and_integers() {
    let source = r#"
use Doria\Std\Io\{ReadTransferKey, WriteTransferKey, RegistrationKey, ReadableFile};
const ?ReadTransferKey EMPTY = null;
function forward(ReadTransferKey $key): ReadTransferKey { let $copy = $key; return $key; }
function optional(?ReadTransferKey $key = EMPTY): ?ReadTransferKey { return $key; }
function write(?WriteTransferKey $key): void {}
function main(): void { echo optional() == null; }
"#;
    doriac::check_source("transfer-keys.doria", source).unwrap();
    for invalid in [
        source.replace("return $key;", "return 1;"),
        source.replace("return $key;", "int $bits = $key; return $key;"),
        source.replace("return $key;", "write($key); return $key;"),
        source.replace("return $key;", "return new ReadTransferKey();"),
        source.replace("return $key;", "return new WriteTransferKey();"),
        source.replace(
            "?ReadTransferKey $key = EMPTY",
            "?WriteTransferKey $key = EMPTY",
        ),
        source.replace(
            "?ReadTransferKey $key = EMPTY",
            "?RegistrationKey<ReadableFile> $key = EMPTY",
        ),
    ] {
        assert!(
            doriac::check_source("wrong-transfer-key.doria", &invalid).is_err(),
            "{invalid}"
        );
    }
    for name in ["ReadTransferKey", "WriteTransferKey"] {
        let program = doriac::lower_source_to_mir(
            "transfer-key-shape.doria",
            format!("use Doria\\Std\\Io\\{name}; function forward({name} $key): {name} {{ return $key; }} function main(): void {{}}"),
        )
        .unwrap();
        let class = program
            .classes
            .iter()
            .find(|class| class.declaration_name.ends_with(name))
            .unwrap();
        let ty = class.inline_copy_type().unwrap();
        assert_eq!(ty.storage_bytes(), 16);
        assert!(class.constructor.is_none() && class.destructor.is_none());
        for wrong in [
            InlineValueType::RegistrationKey(class.id),
            if name == "ReadTransferKey" {
                InlineValueType::WriteTransferKey(class.id)
            } else {
                InlineValueType::ReadTransferKey(class.id)
            },
            InlineValueType::ReadTransferKey(ClassId(usize::MAX)),
            InlineValueType::WriteTransferKey(ClassId(usize::MAX)),
        ] {
            let mut invalid = program.clone();
            let local = invalid
                .functions
                .iter_mut()
                .flat_map(|function| &mut function.locals)
                .find(|local| local.ty == mir::Type::Scalar(mir::ScalarType::Inline(ty)))
                .unwrap();
            assert!(!local.owned);
            local.ty = mir::Type::Scalar(mir::ScalarType::Inline(wrong));
            assert!(doriac::mir_validation::validate_program(&invalid).is_err());
        }
    }
}

fn assert_present_key_transport(bits: u128, static_storage: bool, key_name: &str) {
    let mut source = SOURCE.replace("return $key == null;", "return false;");
    if static_storage {
        source.push_str(
            r#"
enum State { case Key(?RegistrationKey<ReadableFile> $key); }
class Settings { static State $initial = State::Key(null); }
function stored(): ?RegistrationKey<ReadableFile> {
    let $key = match (Settings::initial) { State::Key($key) => $key };
    echo ($key == null) . ":";
    return $key;
}
"#,
        );
        source = source.replace("let $ready = probe();", "let $ready = probe(stored());");
    }
    source = source
        .replace(
            "RegistrationKey, ReadableFile",
            "RegistrationKey, ReadTransferKey, WriteTransferKey, ReadableFile",
        )
        .replace("RegistrationKey<ReadableFile>", key_name);
    let mut program = doriac::lower_source_to_mir("present-key.doria", &source).unwrap();
    let probe = program
        .functions
        .iter()
        .find(|function| function.name == "probe")
        .unwrap()
        .id;
    let ty = program.functions[probe.0].locals[0].ty;
    let mir::Type::NullableScalar(mir::ScalarType::Inline(key)) = ty else {
        panic!("{ty:?}")
    };
    let expected = mir::ValueExpression::Inline(mir::InlineValueExpression::Use {
        ty: key,
        mode: mir::InlineValueUseMode::Borrow,
        operand: mir::Operand::Scalar(mir::ScalarValue::Inline(InlineValue { ty: key, bits })),
    });
    let returned = program.functions[probe.0]
        .locals
        .iter()
        .find(|local| local.name == "returned")
        .unwrap()
        .id;
    let mut compared = false;
    for block in &mut program.functions[probe.0].blocks {
        if matches!(
            block.terminator,
            mir::Terminator::Return(mir::Rvalue::Value(mir::ValueExpression::Bool(
                mir::BoolExpression::Use {
                    operand: mir::Operand::Scalar(mir::ScalarValue::Bool(true))
                }
            )))
        ) {
            block.terminator = mir::Terminator::Return(mir::Rvalue::Value(
                mir::ValueExpression::Bool(mir::BoolExpression::Compare {
                    op: mir::CompareOp::Equal,
                    left: Box::new(mir::ValueExpression::Inline(
                        mir::InlineValueExpression::Use {
                            ty: key,
                            mode: mir::InlineValueUseMode::Borrow,
                            operand: mir::Operand::Local(returned),
                        },
                    )),
                    right: Box::new(expected.clone()),
                }),
            ));
            compared = true;
        }
    }
    assert!(compared);
    let mut replaced = static_storage;
    if static_storage {
        let mir::StaticValue::PayloadEnum(initial) = &mut program.statics[0].initializer else {
            panic!("expected a static enum payload")
        };
        assert_eq!(initial.fields, [mir::StaticValue::Null]);
        initial.fields[0] =
            mir::StaticValue::Scalar(mir::ScalarValue::Inline(InlineValue { ty: key, bits }));
    }
    for block in &mut program.functions[program.entry.0].blocks {
        for statement in &mut block.statements {
            if let mir::Statement::AssignLocal {
                value:
                    mir::Rvalue::Value(mir::ValueExpression::Bool(mir::BoolExpression::Call {
                        function,
                        args,
                        ..
                    })),
                ..
            } = statement
            {
                if *function == probe && !static_storage {
                    args[0] = mir::Rvalue::NullableScalar(mir::NullableScalarExpression::Value(
                        expected.clone(),
                    ));
                    replaced = true;
                }
            }
        }
    }
    assert!(replaced);
    doriac::mir_validation::validate_program(&program).unwrap();
    let expected_stdout = if static_storage {
        "false:true:true\n"
    } else {
        "true:true\n"
    };
    assert_eq!(
        doriac::mir_interpreter::interpret(&program).unwrap().stdout,
        expected_stdout.as_bytes()
    );
    native_execution::assert_native_execution(&program, NativeProfile::Fast, expected_stdout);
    #[cfg(feature = "llvm-backend")]
    native_execution::assert_native_execution(&program, NativeProfile::Release, expected_stdout);
}

#[test]
fn shared_mir_rejects_heap_and_forged_registration_key_identities() {
    let valid = program();
    let InlineValueType::RegistrationKey(id) = key_types(&valid)[0] else {
        unreachable!()
    };
    for fault in 0..12 {
        let mut invalid = valid.clone();
        match fault {
            0 => invalid.classes[id.0].source_span = Default::default(),
            1 => invalid.classes[id.0].declaration_name = "Forged".into(),
            2 | 3 => {
                let local = invalid
                    .functions
                    .iter_mut()
                    .flat_map(|function| &mut function.locals)
                    .find(|local| {
                        local.ty
                            == mir::Type::Scalar(mir::ScalarType::Inline(
                                InlineValueType::RegistrationKey(id),
                            ))
                    })
                    .unwrap();
                local.ty = if fault == 2 {
                    mir::Type::Class(id)
                } else {
                    mir::Type::Scalar(mir::ScalarType::Inline(InlineValueType::RegistrationKey(
                        ClassId(usize::MAX),
                    )))
                };
            }
            4 => invalid.classes[id.0].arguments.clear(),
            5 => invalid.classes[id.0].arguments.push(mir::Type::String),
            6 => invalid.classes[id.0].arguments[0] = mir::Type::Class(ClassId(usize::MAX)),
            7 => {
                let other = key_types(&valid)
                    .into_iter()
                    .find_map(|ty| match ty {
                        InlineValueType::RegistrationKey(other) if other != id => Some(other),
                        _ => None,
                    })
                    .unwrap();
                invalid.classes[id.0].arguments = valid.classes[other.0].arguments.clone();
            }
            8 => invalid.classes[id.0].arguments[0] = mir::Type::ClosureEnvironment(None),
            9 => {
                invalid.classes[id.0].arguments[0] =
                    mir::Type::Function(mir::FunctionTypeId(usize::MAX));
            }
            10 => {
                invalid.classes[id.0].arguments[0] =
                    mir::Type::Interface(mir::InterfaceTypeId(usize::MAX));
            }
            11 => {
                invalid.classes[id.0].arguments[0] =
                    mir::Type::Collection(mir::CollectionTypeId(usize::MAX));
            }
            _ => unreachable!(),
        }
        assert!(doriac::mir_validation::validate_program(&invalid).is_err());
        assert!(doriac::mir_interpreter::interpret(&invalid).is_err());
        assert!(doriac::codegen_cranelift::lower_mir_to_object(&invalid).is_err());
        #[cfg(feature = "llvm-backend")]
        assert!(doriac::codegen_llvm::lower_mir_to_object(&invalid).is_err());
    }
}

#[test]
fn shared_mir_rejects_enum_fields_that_cannot_hold_their_typed_values() {
    let source = r#"
use Doria\Std\Io\{RegistrationKey, ReadableFile};
enum State { case Key(?RegistrationKey<ReadableFile> $key); }
function main(): void { let $state = State::Key(null); }
"#;
    let valid = doriac::lower_source_to_mir("key-layout.doria", source).unwrap();
    let id = valid
        .enums
        .iter()
        .find(|value| value.name == "State")
        .unwrap()
        .id;
    for fault in 0..8 {
        let mut invalid = valid.clone();
        let definition = &mut invalid.enums[id.0];
        let field = &mut definition.layout.cases[0].fields[0];
        match fault {
            0 => field.size -= 1,
            1 => field.align = 1,
            2 => field.offset = definition.layout.size,
            3 => field.index = 1,
            4 => definition.layout.align = 8,
            5 => definition.layout.payload_offset = 0,
            6 => field.offset = 0,
            7 => definition.layout.size += 16,
            _ => unreachable!(),
        }
        assert!(doriac::mir_validation::validate_program(&invalid).is_err());
        assert!(doriac::mir_interpreter::interpret(&invalid).is_err());
        assert!(doriac::codegen_cranelift::lower_mir_to_object(&invalid).is_err());
        #[cfg(feature = "llvm-backend")]
        assert!(doriac::codegen_llvm::lower_mir_to_object(&invalid).is_err());
    }
}
