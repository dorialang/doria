#[path = "common/native_execution.rs"]
mod native_execution;

const OBSERVATIONS: &str = r#"
class Events { static writable string $text = ""; }
function note(string $event): void { Events::text = Events::text . $event; }
function mark(string $event, int $value): int { note($event); return $value; }
class Token {
    function __construct(string $label) { note("new " . $this->label . ";"); }
    function __destruct() { note("drop " . $this->label . ";"); }
}
"#;

fn assert_initializer_execution(body: &str, stdout: &str) -> doriac::mir::Program {
    let source = format!("{OBSERVATIONS}\n{body}");
    assert_complete_initializer_execution(&source, stdout)
}

fn assert_complete_initializer_execution(source: &str, stdout: &str) -> doriac::mir::Program {
    let mir = doriac::lower_source_to_mir("property-hook-initializers.doria", source)
        .unwrap_or_else(|diagnostics| panic!("{source}\n{diagnostics:#?}"));
    doriac::mir_validation::validate_program(&mir)
        .unwrap_or_else(|diagnostic| panic!("{diagnostic:?}\n{mir}"));
    let result = doriac::mir_interpreter::interpret(&mir)
        .unwrap_or_else(|diagnostic| panic!("{diagnostic:?}\n{mir}"));
    assert_eq!(result.stdout, stdout.as_bytes());
    assert_eq!(result.stderr, b"");
    assert_eq!(result.exit_status, 0);
    assert!(result.runtime_diagnostic.is_none());
    native_execution::assert_native_execution(&mir, doriac::backend::NativeProfile::Fast, stdout);
    #[cfg(feature = "llvm-backend")]
    native_execution::assert_native_execution(
        &mir,
        doriac::backend::NativeProfile::Release,
        stdout,
    );

    let php = doriac::compile_source_to_php("property-hook-initializers.doria", source)
        .unwrap_or_else(|diagnostics| panic!("{source}\n{diagnostics:#?}"));
    let Ok(version) = std::process::Command::new("php").arg("--version").output() else {
        eprintln!("PHP unavailable; initializer compatibility execution skipped");
        return mir;
    };
    assert!(version.status.success());
    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").unwrap()
    );
    let result = std::process::Command::new("php")
        .args(["-r", &script])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert!(result.stderr.is_empty(), "{result:?}");
    assert_eq!(result.stdout, stdout.as_bytes());
    mir
}

fn assert_one_backing_slot(mir: &doriac::mir::Program, owner: &str, name: &str) {
    let class = mir
        .classes
        .iter()
        .find(|class| class.name == owner)
        .unwrap();
    assert_eq!(
        class
            .properties
            .iter()
            .filter(|property| property.name == name)
            .count(),
        1,
        "{owner} must reuse one physical backing slot: {class:#?}"
    );
}

#[test]
fn implicit_constructors_run_readonly_parent_then_child_initializers_per_object() {
    let mir = assert_initializer_execution(
        r#"
open class Base {
    open int $value = mark("parent;", 1) { get => $this->value; }
}
class Child extends Base {
    override int $value = mark("child;", 2) { get => $this->value; }
}
function readBase(Base $value): int { return $value->value; }
function main(): void {
    let $first = new Child();
    let $second = new Child();
    echo Events::text . "\n";
    echo "{$first->value} {readBase($first)} {$second->value}\n";
}
"#,
        "parent;child;parent;child;\n2 2 2\n",
    );
    assert_one_backing_slot(&mir, "Base", "value");
    assert_one_backing_slot(&mir, "Child", "value");
}

#[test]
fn overriding_initializers_bypass_setters_and_keep_separate_object_storage() {
    let mir = assert_initializer_execution(
        r#"
open class Base {
    open writable int $value = mark("parent;", 1) {
        get => $this->value;
        set (int $next) { note("parent setter;"); $this->value = $next; }
    }
}
class Child extends Base {
    override writable int $value = mark("child;", 2) {
        get => $this->value;
        set (int $next) { note("child setter;"); $this->value = $next; }
    }
}
function readBase(Base $value): int { return $value->value; }
function main(): void {
    let writable $first = new Child();
    let $second = new Child();
    $first->value = 7;
    echo Events::text . "\n";
    echo "{$first->value} {readBase($first)} {$second->value}\n";
}
"#,
        "parent;child;parent;child;child setter;\n7 7 2\n",
    );
    assert_one_backing_slot(&mir, "Child", "value");
}

#[test]
fn owned_backing_replacement_acquires_child_before_releasing_parent() {
    let mir = assert_complete_initializer_execution(
        include_str!("../../../examples/native/main_backed_override_initializer_order.doria"),
        include_str!("fixtures/native_io/main_backed_override_initializer_order/expected_stdout"),
    );
    assert_one_backing_slot(&mir, "Child", "item");
}

#[test]
fn explicit_parent_calls_preserve_initializer_and_constructor_body_phases() {
    let mir = assert_initializer_execution(
        r#"
function argument(): int { note("argument;"); return 42; }
open class Base {
    open Token $item = new Token("parent") { borrowed get => $this->item; }
    function __construct(parameter int $seed) {
        note("parent body " . $seed . " " . $this->item->label . ";");
    }
    function __destruct() { note("base destructor;"); }
}
class Child extends Base {
    override Token $item = new Token("child") { borrowed get => $this->item; }
    function __construct(parameter int $seed) {
        parent::__construct($seed);
        note("child body " . $this->item->label . ";");
    }
    function __destruct() { note("child destructor;"); }
}
function main(): void {
    {
        let $value = new Child(argument());
        note("live " . $value->item->label . ";");
    }
    echo Events::text . "\n";
}
"#,
        "argument;new parent;parent body 42 parent;new child;drop parent;child body child;live child;child destructor;base destructor;drop child;\n",
    );
    assert_one_backing_slot(&mir, "Child", "item");
}

#[test]
fn parent_failure_releases_parent_storage_without_running_child_initializers() {
    assert_initializer_execution(
        r#"
class Fault implements Error { function __construct(string $message) {} }
open class Base {
    open Token $item = new Token("parent") { borrowed get => $this->item; }
    function __construct() throws Fault {
        note("parent failure;");
        throw new Fault("parent");
    }
    function __destruct() { note("unexpected base destructor;"); }
}
class Child extends Base {
    override Token $item = new Token("child") { borrowed get => $this->item; }
    function __construct() throws Fault { parent::__construct(); note("unexpected child body;"); }
    function __destruct() { note("unexpected child destructor;"); }
}
function main(): void {
    try { let $value = new Child(); }
    catch (Fault) { note("caught;"); }
    echo Events::text . "\n";
}
"#,
        "new parent;parent failure;drop parent;caught;\n",
    );
}

#[test]
fn failed_child_replacement_keeps_parent_value_until_completed_parent_cleanup() {
    assert_complete_initializer_execution(
        include_str!("../../../examples/native/main_backed_override_initializer_failure.doria"),
        include_str!("fixtures/native_io/main_backed_override_initializer_failure/expected_stdout"),
    );
}

#[test]
fn failure_after_child_replacement_drops_the_current_backing_value_only_once() {
    assert_initializer_execution(
        r#"
class Fault implements Error { function __construct(string $message) {} }
function fail(): int throws Fault { note("failure;"); throw new Fault("child"); }
open class Base {
    open Token $item = new Token("parent") { borrowed get => $this->item; }
    function __destruct() { note("base destructor;"); }
}
class Child extends Base {
    override Token $item = new Token("child") { borrowed get => $this->item; }
    Token $own = new Token("own");
    int $later = fail();
    function __construct() throws Fault { note("unexpected child body;"); }
    function __destruct() { note("unexpected child destructor;"); }
}
function main(): void {
    try { let $value = new Child(); }
    catch (Fault) { note("caught;"); }
    echo Events::text . "\n";
}
"#,
        "new parent;new child;drop parent;new own;failure;drop own;base destructor;drop child;caught;\n",
    );
}

#[test]
fn generic_multilevel_overrides_initialize_one_specialized_backing_slot() {
    let mir = assert_initializer_execution(
        r#"
function markNull<T>(string $event): ?T { note($event); return null; }
open class Base<T> {
    open ?T $value = markNull("base;") { get => $this->value; }
}
open class Middle<T> extends Base<T> {
    override ?T $value = markNull("middle;") { get => $this->value; }
}
class Leaf extends Middle<int> {
    override ?int $value = mark("leaf;", 9) { get => $this->value; }
}
function readBase(Base<int> $value): ?int { return $value->value; }
function main(): void {
    let $first = new Leaf();
    let $second = new Leaf();
    let $firstValue = $first->value ?? 0;
    let $baseValue = readBase($first) ?? 0;
    let $secondValue = $second->value ?? 0;
    echo Events::text . "\n";
    echo "{$firstValue} {$baseValue} {$secondValue}\n";
}
"#,
        "base;middle;leaf;base;middle;leaf;\n9 9 9\n",
    );
    assert_one_backing_slot(&mir, "Leaf", "value");
}

#[test]
fn callback_initializers_replace_and_release_each_phases_owned_environment() {
    let mir = assert_initializer_execution(
        r#"
function makeCallback(string $label): function(): string {
    let $token = new Token($label);
    return fn() with (take $token) => $token->label;
}
open class Base {
    open function(): string $callback = makeCallback("parent") {
        borrowed get => $this->callback;
    }
}
class Child extends Base {
    override function(): string $callback = makeCallback("child") {
        borrowed get => $this->callback;
    }
}
function main(): void {
    {
        let $value = new Child();
        let $callback = $value->callback;
        note("live " . $callback() . ";");
    }
    echo Events::text . "\n";
}
"#,
        "new parent;new child;drop parent;live child;drop child;\n",
    );
    assert_one_backing_slot(&mir, "Child", "callback");
}

fn readonly_override_mir() -> doriac::mir::Program {
    let source = r#"
open class Base { open int $value = 1 { get => $this->value; } }
class Child extends Base {
    override int $value = 2 { get => $this->value; }
    int $own = 3;
}
function main(): void { let $value = new Child(); }
"#;
    let program = doriac::lower_source_to_mir("initializer-validation.doria", source)
        .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
    doriac::mir_validation::validate_program(&program).unwrap();
    program
}

fn override_statement(program: &mut doriac::mir::Program) -> &mut doriac::mir::Statement {
    program
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .flat_map(|block| &mut block.statements)
        .find(|statement| {
            matches!(
                statement,
                doriac::mir::Statement::AssignProperty {
                    kind: doriac::mir::PropertyWriteKind::InitializeOverride,
                    ..
                }
            )
        })
        .expect("child declaration emits one override-initialization write")
}

fn assert_invalid_initializer_mir(program: &doriac::mir::Program, expected: &str) {
    let error = doriac::mir_validation::validate_program(program)
        .expect_err("invalid initializer ownership must be rejected before execution");
    assert!(
        error.message.contains(expected),
        "expected {expected:?}, got {error:?}"
    );
}

#[test]
fn shared_validator_rejects_plain_initialization_of_parent_initialized_backing() {
    let mut program = readonly_override_mir();
    let doriac::mir::Statement::AssignProperty { kind, .. } = override_statement(&mut program)
    else {
        unreachable!();
    };
    *kind = doriac::mir::PropertyWriteKind::Initialize;
    assert_invalid_initializer_mir(&program, "more than once on one path");
}

#[test]
fn shared_validator_rejects_override_initialization_of_child_owned_storage() {
    let mut program = readonly_override_mir();
    let ordinary = program
        .classes
        .iter()
        .find(|class| class.name == "Child")
        .unwrap()
        .properties
        .iter()
        .find(|property| property.name == "own")
        .unwrap()
        .id;
    let doriac::mir::Statement::AssignProperty { property, .. } = override_statement(&mut program)
    else {
        unreachable!();
    };
    *property = ordinary;
    assert_invalid_initializer_mir(&program, "must target inherited storage");
}

#[test]
fn shared_validator_rejects_repeated_readonly_override_initialization() {
    let mut program = readonly_override_mir();
    let (block, index) = program
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .find_map(|block| {
            block
                .statements
                .iter()
                .position(|statement| {
                    matches!(
                        statement,
                        doriac::mir::Statement::AssignProperty {
                            kind: doriac::mir::PropertyWriteKind::InitializeOverride,
                            ..
                        }
                    )
                })
                .map(|index| (block, index))
        })
        .expect("child declaration emits an override-initialization write");
    block
        .statements
        .insert(index + 1, block.statements[index].clone());
    assert_invalid_initializer_mir(
        &program,
        "without one completed parent phase or more than once",
    );
}

#[test]
fn child_declaration_order_precedes_inherited_backing_layout_order() {
    let mir = assert_initializer_execution(
        r#"
open class Base {
    open Token $item = new Token("parent") { borrowed get => $this->item; }
    function __destruct() { note("base destructor;"); }
}
class Child extends Base {
    Token $own = new Token("own");
    override Token $item = new Token("child") { borrowed get => $this->item; }
    function __destruct() { note("child destructor;"); }
}
function main(): void {
    {
        let $value = new Child();
        note("live " . $value->own->label . " " . $value->item->label . ";");
    }
    echo Events::text . "\n";
}
"#,
        "new parent;new own;new child;drop parent;live own child;child destructor;drop own;base destructor;drop child;\n",
    );
    assert_one_backing_slot(&mir, "Child", "item");
}

#[test]
fn failed_parent_phase_cleans_own_fields_then_only_the_completed_grandparent() {
    assert_initializer_execution(
        r#"
class Fault implements Error { function __construct(string $message) {} }
open class Grandparent {
    Token $grand = new Token("grand");
    function __construct() { note("grandparent body;"); }
    function __destruct() { note("grandparent destructor;"); }
}
open class MiddlePhase extends Grandparent {
    Token $first = new Token("first");
    Token $second = new Token("second");
    function __construct() throws Fault {
        parent::__construct();
        note("parent failure;");
        throw new Fault("parent");
    }
    function __destruct() { note("unexpected parent destructor;"); }
}
class Child extends MiddlePhase {
    Token $child = new Token("child");
    function __construct() throws Fault {
        parent::__construct();
        note("unexpected child body;");
    }
    function __destruct() { note("unexpected child destructor;"); }
}
function main(): void {
    try { let $value = new Child(); }
    catch (Fault) { note("caught;"); }
    echo Events::text . "\n";
}
"#,
        "new grand;grandparent body;new first;new second;parent failure;drop second;drop first;grandparent destructor;drop grand;caught;\n",
    );
}

#[test]
fn shared_validator_rejects_missing_failed_constructor_phase_cleanup() {
    let mut program = doriac::lower_source_to_mir(
        "initializer-failure-cleanup-validation.doria",
        include_str!("../../../examples/native/main_backed_override_initializer_failure.doria"),
    )
    .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
    let constructors = program
        .classes
        .iter()
        .filter_map(|class| class.constructor)
        .collect::<Vec<_>>();
    let mut removed = 0;
    for function in &mut program.functions {
        if !constructors.contains(&function.id) {
            continue;
        }
        for block in &mut function.blocks {
            block.statements.retain(|statement| {
                if matches!(
                    statement,
                    doriac::mir::Statement::CleanupConstructorPhase { .. }
                ) {
                    removed += 1;
                    false
                } else {
                    true
                }
            });
        }
    }
    assert!(
        removed > 0,
        "fixture must contain constructor failure cleanup"
    );
    assert_invalid_initializer_mir(
        &program,
        "propagates failure without constructor phase cleanup",
    );
}

#[test]
fn shared_validator_rejects_duplicate_failed_constructor_phase_cleanup() {
    let mut program = doriac::lower_source_to_mir(
        "initializer-failure-cleanup-validation.doria",
        include_str!("../../../examples/native/main_backed_override_initializer_failure.doria"),
    )
    .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
    let constructor = program
        .classes
        .iter()
        .find(|class| class.name == "Child")
        .and_then(|class| class.constructor)
        .expect("child initializer has a checked constructor phase");
    let (block, index) = program.functions[constructor.0]
        .blocks
        .iter_mut()
        .find_map(|block| {
            block
                .statements
                .iter()
                .position(|statement| {
                    matches!(
                        statement,
                        doriac::mir::Statement::CleanupConstructorPhase { .. }
                    )
                })
                .map(|index| (block, index))
        })
        .expect("checked child replacement has a reachable phase cleanup");
    block
        .statements
        .insert(index + 1, block.statements[index].clone());
    assert_invalid_initializer_mir(
        &program,
        "constructor phase cleanup requires an active own phase",
    );
}
