#[path = "common/native_execution.rs"]
mod native_execution;

fn assert_returned_closure_execution(source: &str, stdout: &str) {
    let mir = doriac::lower_source_to_mir("returned-closure-transport.doria", source)
        .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
    assert_mir_execution(&mir, stdout);
}

fn assert_mir_execution(mir: &doriac::mir::Program, stdout: &str) {
    doriac::mir_validation::validate_program(mir)
        .unwrap_or_else(|error| panic!("{error:?}\n{mir}"));
    let result =
        doriac::mir_interpreter::interpret(mir).unwrap_or_else(|error| panic!("{error:?}\n{mir}"));
    assert_eq!(result.stdout, stdout.as_bytes());
    assert_eq!(result.stderr, b"");
    assert_eq!(result.exit_status, 0);
    assert!(result.runtime_diagnostic.is_none());
    assert!(!doriac::codegen_cranelift::lower_mir_to_object(mir)
        .unwrap_or_else(|error| panic!("Cranelift: {error:?}\n{mir}"))
        .is_empty());
    #[cfg(feature = "llvm-backend")]
    assert!(!doriac::codegen_llvm::lower_mir_to_object(mir)
        .unwrap_or_else(|error| panic!("LLVM: {error:?}\n{mir}"))
        .is_empty());

    #[cfg(not(feature = "llvm-backend"))]
    let profiles = [doriac::backend::NativeProfile::Fast];
    #[cfg(feature = "llvm-backend")]
    let profiles = [
        doriac::backend::NativeProfile::Fast,
        doriac::backend::NativeProfile::Release,
    ];
    for profile in profiles {
        native_execution::assert_native_execution(mir, profile, stdout);
    }
}

fn lower_callback_hook_hir(source: &str) -> doriac::hir::Program {
    doriac::lower_source("hooks.doria", source)
        .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"))
}

fn assert_callback_hook_execution(source: &str, stdout: &str) -> doriac::mir::Program {
    let hir = lower_callback_hook_hir(source);
    let mir = doriac::mir_lowering::lower_program(&hir).unwrap();
    assert_mir_execution(&mir, stdout);
    let php = doriac::codegen_php::generate(&hir, Some(&mir)).unwrap();
    let Ok(version) = std::process::Command::new("php").arg("--version").output() else {
        eprintln!("PHP unavailable; callback compatibility execution skipped");
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

#[test]
fn interface_callable_getters_keep_receiver_places_in_linked_programs() {
    let source = r#"
interface Reader { function(): int $callback { get; } }
open class Base implements Reader {
    int $value = 42;
    open function(): int $callback { get => fn() with ($this) => $this->value; }
    function __destruct() { try { echo "drop;"; } catch (Error) {} }
}
class Source extends Base {
    override function(): int $callback { get => fn() with ($this) => $this->value; }
}
function capture(Reader $reader): function(): int { return $reader->callback; }
function forward(Reader $reader): function(): int { return capture($reader); }
function main(): void {
    let $source = new Source();
    let $callback = forward($source);
    echo "{$callback()}\n";
}
"#;
    assert_callback_hook_execution(source, "42\ndrop;");
}

#[test]
fn fresh_callable_getters_own_environments_across_interface_and_override_dispatch() {
    let source = r#"
class Token {
    function __construct(int $value) {}
    function __destruct() { try { echo "token{$this->value};"; } catch (Error) {} }
}
interface Reader { function(): int $callback { get; } }
open class Base implements Reader {
    int $value = 42;
    open function(): int $callback {
        get {
            let $token = new Token(1);
            return function (): int with ($this, take $token) {
                if ($token->value == 0) { return 0; }
                return $this->value;
            };
        }
    }
    function __destruct() { try { echo "source;"; } catch (Error) {} }
}
class Source extends Base {
    override function(): int $callback {
        get {
            let $token = new Token(2);
            return function (): int with ($this, take $token) {
                if ($token->value == 0) { return 0; }
                return $this->value;
            };
        }
    }
}
class Independent implements Reader {
    function(): int $callback { get => fn() => 7; }
}
function fromInterface(Reader $reader): function(): int { return $reader->callback; }
function fromBase(Base $reader): function(): int { return $reader->callback; }
function main(): void {
    let $source = new Source();
    {
        let $direct = $source->callback;
        echo "{$direct()};";
    }
    {
        let $erased = fromInterface($source);
        echo "{$erased()};";
    }
    {
        let $virtual = fromBase($source);
        echo "{$virtual()};";
    }
    {
        let $independent = new Independent();
        let $callback = fromInterface($independent);
        echo "independent:{$callback()};";
    }
    echo "alive;";
}
"#;
    let mir = assert_callback_hook_execution(
        source,
        "42;token2;42;token2;42;token2;independent:7;alive;source;",
    );
    let main = mir
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    for name in ["direct", "erased", "virtual"] {
        let local = main.locals.iter().find(|local| local.name == name).unwrap();
        assert!(
            local.owned,
            "fresh callback {name} must own its environment"
        );
    }
}

#[test]
fn generic_callable_getters_preserve_owned_environments_and_retained_receiver_sources() {
    assert_callback_hook_execution(
        r#"
class Token {
    int $value = 42;
    function __destruct() { try { echo "token;"; } catch (Error) {} }
}
interface Factory<T> { T $callback { get; } }
class Source implements Factory<function(): int> {
    int $value = 42;
    function(): int $callback {
        get {
            let $token = new Token();
            return function (): int with ($this, take $token) {
                if ($token->value == 42) { return $this->value; }
                return 0;
            };
        }
    }
    function __destruct() { try { echo "source;"; } catch (Error) {} }
}
function erased(Factory<function(): int> $source): function(): int {
    return $source->callback;
}
function relay<T implements Factory<function(): int>>(T $source): function(): int {
    return $source->callback;
}
function optional(Source $source, bool $present): ?Source {
    if ($present) { return $source; }
    return null;
}
function main(): void {
    let $source = new Source();
    {
        let $callback = erased($source);
        echo "{$callback()};";
    }
    {
        let $callback = relay($source);
        echo "{$callback()};";
    }
    {
        let $present = optional($source, true);
        let $callback = $present?->callback;
        if ($callback != null) { echo "{$callback()};"; }
    }
    let $absent = optional($source, false);
    let $missing = $absent?->callback;
    if ($missing == null) { echo "none;"; }
    echo "alive;";
}
"#,
        "42;token;42;token;42;token;none;alive;source;",
    );
}

#[test]
fn stored_callable_getters_lend_environments_without_releasing_owned_captures() {
    let source = r#"
class Token {
    int $value = 42;
    function __destruct() { try { echo "token;"; } catch (Error) {} }
}
interface Reader { function(): int $callback { borrowed get; } }
class Source implements Reader {
    function __construct(take function(): int $stored) {}
    function(): int $callback { borrowed get => $this->stored; }
    ?function(): int $optional { borrowed get => $this->stored; }
    function __destruct() { try { echo "source;"; } catch (Error) {} }
}
function fromInterface(Reader $reader): function(): int { return $reader->callback; }
function main(): void {
    let $token = new Token();
    let $source = new Source(fn() with (take $token) => $token->value);
    {
        let $direct = $source->callback;
        echo "{$direct()};";
    }
    {
        let $erased = fromInterface($source);
        echo "{$erased()};";
    }
    {
        let $optional = $source->optional;
        if ($optional != null) { echo "{$optional()};"; }
    }
    echo "{$source->stored()};alive;";
}
"#;
    let mir = assert_callback_hook_execution(source, "42;42;42;42;alive;source;token;");
    let main = mir
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    for name in ["direct", "erased", "optional"] {
        let local = main.locals.iter().find(|local| local.name == name).unwrap();
        assert!(
            !local.owned,
            "stored callback {name} must remain a borrowed carrier"
        );
    }
}

#[test]
fn fresh_nullable_callable_getters_release_only_present_environments() {
    assert_callback_hook_execution(
        r#"
class Token {
    int $value = 2;
    function __destruct() { try { echo "token;"; } catch (Error) {} }
}
class Source {
    int $value = 42;
    function __construct(bool $present) {}
    ?function(): int $callback {
        get {
            if (!$this->present) { return null; }
            let $token = new Token();
            return function (): int with ($this, take $token) {
                if ($token->value == 0) { return 0; }
                return $this->value;
            };
        }
    }
    function __destruct() { try { echo "source;"; } catch (Error) {} }
}
function main(): void {
    let $source = new Source(true);
    let $empty = new Source(false);
    {
        let $callback = $source->callback;
        if ($callback != null) { echo "{$callback()};"; }
        let $missing = $empty->callback;
        if ($missing == null) { echo "none;"; }
    }
    echo "alive;";
}
"#,
        "42;none;token;alive;source;source;",
    );
}

#[test]
fn checked_callable_getters_transport_owned_environments_and_receiver_borrows_separately() {
    assert_callback_hook_execution(
        r#"
class Failure implements Error { string $message = "failed"; }
class Token {
    int $value = 2;
    function __destruct() { try { echo "token;"; } catch (Error) {} }
}
interface Reader { function(): int $callback { get throws Failure; } }
class Source implements Reader {
    int $value = 42;
    function __construct(bool $fail) {}
    function(): int $callback {
        get throws Failure {
            if ($this->fail) { throw new Failure(); }
            let $token = new Token();
            return function (): int with ($this, take $token) {
                if ($token->value == 0) { return 0; }
                return $this->value;
            };
        }
    }
    function __destruct() { try { echo "source;"; } catch (Error) {} }
}
function forward(Reader $reader): (function(): int) throws Failure {
    return $reader->callback;
}
function main(): void {
    let $source = new Source(false);
    let $failing = new Source(true);
    try {
        let $callback = forward($source);
        echo "{$callback()};";
    } catch (Failure) { echo "unexpected;"; }
    try {
        let $callback = forward($failing);
        echo "{$callback()};unexpected;";
    } catch (Failure) { echo "caught;"; }
    try {
        let $callback = forward($source);
        echo "{$callback()};";
    } catch (Failure) { echo "unexpected;"; }
    echo "alive;";
}
"#,
        "42;token;caught;42;token;alive;source;source;",
    );
}

#[test]
fn taking_a_retained_callback_preserves_its_sources_without_borrowing_the_local_carrier() {
    assert_callback_hook_execution(
        r#"
class Token {
    int $value = 2;
    function __destruct() { try { echo "token;"; } catch (Error) {} }
}
class Source {
    int $value = 42;
    function(): int $callback {
        get {
            let $token = new Token();
            return function (): int with ($this, take $token) {
                if ($token->value == 0) { return 0; }
                return $this->value;
            };
        }
    }
    function __destruct() { try { echo "source;"; } catch (Error) {} }
}
function wrap(Source $source): function(): int {
    let $inner = $source->callback;
    return fn() with (take $inner) => $inner();
}
function relay(Source $source): function(): int {
    let $middle = wrap($source);
    return fn() with (take $middle) => $middle();
}
function main(): void {
    let $source = new Source();
    {
        let $callback = relay($source);
        echo "{$callback()};";
    }
    echo "alive;";
}
"#,
        "42;token;alive;source;",
    );
}

#[test]
fn owning_a_fresh_callable_does_not_allow_its_borrowed_receiver_to_escape() {
    let declarations = r#"
interface Reader { function(): int $callback { get; } }
class Source implements Reader {
    int $value = 42;
    function(): int $callback { get => fn() with ($this) => $this->value; }
}
function consume(take Source $source): void {}
function fromInterface(Reader $reader): function(): int { return $reader->callback; }
"#;
    for (body, expected) in [
        (
            "function main(): void { let $source = new Source(); let $callback = $source->callback; consume($source); echo $callback(); }",
            "E0654",
        ),
        (
            "function main(): void { let $source = new Source(); let $callback = fromInterface($source); consume($source); echo $callback(); }",
            "E0654",
        ),
        (
            "function escape(): function(): int { let $source = new Source(); return $source->callback; }",
            "E0658",
        ),
        (
            "function escape(): function(): int { return fromInterface(new Source()); }",
            "E0658",
        ),
    ] {
        let source = format!("{declarations}\n{body}");
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", &source).unwrap();
        assert!(
            analysis.diagnostics.iter().any(|diagnostic| diagnostic.code == expected),
            "{body}: {:?}",
            analysis.diagnostics
        );
        assert!(
            analysis.diagnostics.iter().all(|diagnostic| diagnostic.code != "E0755"),
            "a retained receiver is not a borrowed callback carrier: {:?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn returned_closure_preserves_a_materialized_property_place() {
    assert_returned_closure_execution(
        r#"
class Child {
    int $value = 42;
    function make(): function(): int { return fn() with ($this) => $this->value; }
    function __destruct() { try { echo "drop;"; } catch (Error) {} }
}
class Holder { Child $child = new Child(); }
function capture(Holder $holder): function(): int { return $holder->child->make(); }
function main(): void {
    let $holder = new Holder();
    let $callback = capture($holder);
    echo "{$callback()}\n";
}
"#,
        "42\ndrop;",
    );
}

#[test]
fn returned_closure_preserves_a_borrowed_method_results_exact_place() {
    assert_returned_closure_execution(
        r#"
class Child {
    int $value = 42;
    function make(): function(): int { return fn() with ($this) => $this->value; }
    function __destruct() { try { echo "drop;"; } catch (Error) {} }
}
class Holder {
    Child $child = new Child();
    function childRef(): Child { return $this->child; }
}
function capture(Holder $holder): function(): int {
    let $view = $holder->childRef();
    return $view->make();
}
function main(): void {
    let $holder = new Holder();
    let $callback = capture($holder);
    echo "{$callback()}\n";
}
"#,
        "42\ndrop;",
    );
}

#[test]
fn returned_closures_preserve_runtime_selected_child_places_through_call_chains() {
    assert_returned_closure_execution(
        r#"
class Child {
    function __construct(int $value) {}
    function selfRef(): self { return $this; }
    function make(): function(): int { return fn() with ($this) => $this->value; }
    function __destruct() { try { echo "drop{$this->value};"; } catch (Error) {} }
}
class Holder {
    Child $first = new Child(11);
    Child $second = new Child(22);
    function select(bool $second): Child {
        if ($second) { return $this->second; }
        return $this->first;
    }
    function __destruct() { try { echo "holder;"; } catch (Error) {} }
}
function forward(Holder $holder, bool $second): Child { return $holder->select($second); }
function relay(Holder $holder, bool $second): Child { return forward($holder, $second); }
function capture(Holder $holder, bool $second): function(): int {
    let $view = relay($holder, $second);
    let $same = $view->selfRef();
    return $same->make();
}
function main(): void {
    let $holder = new Holder();
    let $first = capture($holder, false);
    let $second = capture($holder, true);
    echo "{$first()} {$second()}\n";
}
"#,
        "11 22\nholder;drop22;drop11;",
    );
}

#[test]
fn returned_closures_preserve_nullable_selected_child_places() {
    assert_returned_closure_execution(
        r#"
class Child {
    function __construct(int $value) {}
    function make(): function(): int { return fn() with ($this) => $this->value; }
    function __destruct() { try { echo "drop{$this->value};"; } catch (Error) {} }
}
class Holder {
    Child $first = new Child(11);
    Child $second = new Child(22);
    function select(bool $present, bool $second): ?Child {
        if (!$present) { return null; }
        if ($second) { return $this->second; }
        return $this->first;
    }
    function __destruct() { try { echo "holder;"; } catch (Error) {} }
}
function capture(Holder $holder, bool $present, bool $second): ?function(): int {
    let $view = $holder->select($present, $second);
    return $view?->make();
}
function main(): void {
    let $holder = new Holder();
    let $missing = capture($holder, false, true);
    if ($missing == null) { echo "absent;"; }
    let $first = capture($holder, true, false);
    if ($first != null) { echo "{$first()} "; }
    let $second = capture($holder, true, true);
    if ($second != null) { echo "{$second()}\n"; }
}
"#,
        "absent;11 22\nholder;drop22;drop11;",
    );
}

#[test]
fn checked_borrowed_calls_preserve_selected_child_places_after_errors() {
    assert_returned_closure_execution(
        r#"
class Failure implements Error { string $message = "failed"; }
class Child {
    function __construct(int $value) {}
    function make(): function(): int { return fn() with ($this) => $this->value; }
    function __destruct() { try { echo "drop{$this->value};"; } catch (Error) {} }
}
class Holder {
    Child $first = new Child(11);
    Child $second = new Child(22);
    function select(bool $second, bool $fail): Child throws Failure {
        if ($fail) { throw new Failure(); }
        if ($second) { return $this->second; }
        return $this->first;
    }
    function __destruct() { try { echo "holder;"; } catch (Error) {} }
}
function forward(Holder $holder, bool $second, bool $fail): Child throws Failure {
    return $holder->select($second, $fail);
}
function capture(Holder $holder, bool $second, bool $fail): (function(): int) throws Failure {
    let $view = forward($holder, $second, $fail);
    return $view->make();
}
function main(): void {
    let $holder = new Holder();
    try {
        let $first = capture($holder, false, false);
        echo "{$first()};";
    } catch (Failure) { echo "unexpected;"; }
    try {
        let $unused = capture($holder, true, true);
        echo "unexpected;";
    } catch (Failure) { echo "caught;"; }
    try {
        let $second = capture($holder, true, false);
        echo "{$second()}\n";
    } catch (Failure) { echo "unexpected;"; }
}
"#,
        "11;caught;22\nholder;drop22;drop11;",
    );
}

#[test]
fn virtual_borrowed_calls_preserve_the_overrides_selected_child_place() {
    assert_returned_closure_execution(
        r#"
class Child {
    function __construct(int $value) {}
    function make(): function(): int { return fn() with ($this) => $this->value; }
    function __destruct() { try { echo "drop{$this->value};"; } catch (Error) {} }
}
open class Holder {
    Child $first = new Child(11);
    Child $second = new Child(22);
    open function select(bool $second): Child {
        if ($second) { return $this->second; }
        return $this->first;
    }
    function __destruct() { try { echo "holder;"; } catch (Error) {} }
}
class Reversed extends Holder {
    override function select(bool $second): Child {
        if ($second) { return $this->first; }
        return $this->second;
    }
}
function capture(Holder $holder, bool $second): function(): int {
    let $view = $holder->select($second);
    return $view->make();
}
function main(): void {
    let $holder = new Reversed();
    let $first = capture($holder, false);
    let $second = capture($holder, true);
    echo "{$first()} {$second()}\n";
}
"#,
        "22 11\nholder;drop22;drop11;",
    );
}

#[test]
fn indirect_borrowed_calls_preserve_runtime_selected_child_places() {
    assert_returned_closure_execution(
        r#"
class Child {
    function __construct(int $value) {}
    function make(): function(): int { return fn() with ($this) => $this->value; }
    function __destruct() { try { echo "drop{$this->value};"; } catch (Error) {} }
}
class Holder {
    Child $first = new Child(11);
    Child $second = new Child(22);
    function select(bool $second): Child {
        if ($second) { return $this->second; }
        return $this->first;
    }
    function __destruct() { try { echo "holder;"; } catch (Error) {} }
}
function capture(Holder $holder, bool $second): function(): int {
    let $select = function (Holder $owner, bool $other): Child {
        return $owner->select($other);
    };
    let $view = $select($holder, $second);
    return $view->make();
}
function main(): void {
    let $holder = new Holder();
    let $first = capture($holder, false);
    let $second = capture($holder, true);
    echo "{$first()} {$second()}\n";
}
"#,
        "11 22\nholder;drop22;drop11;",
    );
}

#[test]
fn returned_closure_keeps_an_erased_interface_view_of_its_original_class_place() {
    assert_returned_closure_execution(
        r#"
interface Value { function read(): int; }
open class Base implements Value {
    int $value = 42;
    open function read(): int { return $this->value; }
    function __destruct() { try { echo "drop;"; } catch (Error) {} }
}
class Source extends Base {
    override function read(): int { return $this->value; }
}
function capture(Value $value): function(): int {
    return fn() with ($value) => $value->read();
}
function forward(Value $value): function(): int { return capture($value); }
function main(): void {
    let $source = new Source();
    let $callback = forward($source);
    echo "{$callback()}\n";
}
"#,
        "42\ndrop;",
    );
}

#[test]
fn returned_closure_receiver_places_survive_virtual_dispatch_and_forwarding() {
    assert_returned_closure_execution(
        r#"
open class Reader {
    function __construct(int $value) {}
    open function make(): function(): int {
        return fn() with ($this) => $this->value;
    }
    function __destruct() { try { echo "drop;"; } catch (Error) {} }
}
class Source extends Reader {
    function __construct(int $initial) { parent::__construct($initial); }
    override function make(): function(): int {
        return fn() with ($this) => $this->value;
    }
}
function forward(Reader $reader): function(): int { return $reader->make(); }
function relay(Reader $reader): function(): int { return forward($reader); }
function main(): void {
    let $source = new Source(42);
    let $direct = $source->make();
    let $forwarded = forward($source);
    let $relayed = relay($source);
    echo "{$direct()} {$forwarded()} {$relayed()}\n";
}
"#,
        "42 42 42\ndrop;",
    );
}

#[test]
fn returned_closures_project_nullable_concrete_and_base_class_receivers() {
    assert_returned_closure_execution(
        include_str!("../../../examples/native/main_nullable_returned_closures.doria"),
        include_str!("fixtures/native_io/main_nullable_returned_closures/expected_stdout"),
    );
}

#[test]
fn checked_virtual_calls_preserve_returned_closure_receiver_places() {
    assert_returned_closure_execution(
        r#"
class Failure implements Error { string $message = "failed"; }
open class Reader {
    int $value = 42;
    open function make(bool $fail): (function(): int) throws Failure {
        if ($fail) { throw new Failure(); }
        return fn() with ($this) => $this->value;
    }
    function __destruct() { try { echo "drop;"; } catch (Error) {} }
}
class Source extends Reader {
    override function make(bool $fail): (function(): int) throws Failure {
        if ($fail) { throw new Failure(); }
        return fn() with ($this) => $this->value;
    }
}
function forward(Reader $reader, bool $fail): (function(): int) throws Failure {
    return $reader->make($fail);
}
function main(): void {
    let $source = new Source();
    try {
        let $callback = forward($source, false);
        echo "{$callback()}\n";
    } catch (Failure) { echo "unexpected;"; }
    try {
        let $unused = forward($source, true);
        echo "unexpected;";
    } catch (Failure) { echo "caught;"; }
    try {
        let $callback = forward($source, false);
        echo "{$callback()}\n";
    } catch (Failure) { echo "unexpected;"; }
}
"#,
        "42\ncaught;42\ndrop;",
    );
}

#[test]
fn returned_writable_closure_updates_the_original_virtual_receiver() {
    assert_returned_closure_execution(
        r#"
open class Counter {
    writable int $value = 40;
    open writable function make(): function writable(): int {
        return function (): int with (writable $this) {
            $this->value += 1;
            return $this->value;
        };
    }
    function __destruct() { try { echo "drop{$this->value};"; } catch (Error) {} }
}
class Source extends Counter {
    override writable function make(): function writable(): int {
        return function (): int with (writable $this) {
            $this->value += 1;
            return $this->value;
        };
    }
}
function forward(writable Counter $counter): function writable(): int {
    return $counter->make();
}
function relay(writable Counter $counter): function writable(): int {
    return forward($counter);
}
function maybe(bool $present): ?Source {
    if ($present) { return new Source(); }
    return null;
}
function main(): void {
    let writable $source = maybe(true);
    if ($source != null) {
        {
            let writable $callback = relay($source);
            echo "{$callback()} {$callback()}\n";
        }
        if ($source != null) { echo "{$source->value}\n"; }
    }
}
"#,
        "41 42\n42\ndrop42;",
    );
}

#[test]
fn coalesced_borrowed_results_preserve_the_selected_child_place() {
    assert_returned_closure_execution(
        r#"
class Child {
    function __construct(int $value) {}
    function make(): function(): int { return fn() with ($this) => $this->value; }
    function __destruct() { try { echo "drop{$this->value};"; } catch (Error) {} }
}

class Holder {
    Child $first = new Child(11);
    Child $second = new Child(22);
    function maybeFirst(bool $present): ?Child {
        if ($present) { return $this->first; }
        return null;
    }
    function secondRef(): Child {
        try { echo "fallback;"; } catch (Error) {}
        return $this->second;
    }
    function __destruct() { try { echo "holder;"; } catch (Error) {} }
}
function select(Holder $holder, bool $present): Child {
    return $holder->maybeFirst($present) ?? $holder->secondRef();
}
function capture(Holder $holder, bool $present): function(): int {
    let $view = select($holder, $present);
    return $view->make();
}
function forward(Holder $holder, bool $present): function(): int {
    return capture($holder, $present);
}
function main(): void {
    let $holder = new Holder();
    let $first = forward($holder, true);
    echo "{$first()};";
    let $second = forward($holder, false);
    echo "{$second()}\n";
}
"#,
        "11;fallback;22\nholder;drop22;drop11;",
    );
}

#[test]
fn null_safe_borrowed_calls_preserve_dynamic_dispatch_and_returned_places() {
    assert_null_safe_borrowed_dispatch(
        "?Holder $absent = null; let $missing = capture($absent, false);",
    );
}

#[test]
fn literal_null_borrowed_calls_preserve_dynamic_dispatch_and_returned_places() {
    assert_null_safe_borrowed_dispatch("let $missing = capture(null, false);");
}

fn assert_null_safe_borrowed_dispatch(absent_call: &str) {
    let source = r#"
class Child {
    function __construct(int $value) {}
    function make(): function(): int { return fn() with ($this) => $this->value; }
    function __destruct() { try { echo "drop{$this->value};"; } catch (Error) {} }
}
open class Holder {
    Child $first = new Child(11);
    Child $second = new Child(22);
    open function select(bool $second): Child {
        if ($second) { return $this->second; }
        return $this->first;
    }
    function __destruct() { try { echo "holder;"; } catch (Error) {} }
}
class Reversed extends Holder {
    override function select(bool $second): Child {
        if ($second) { return $this->first; }
        return $this->second;
    }
}
function capture(?Holder $holder, bool $second): ?function(): int {
    let $view = $holder?->select($second);
    return $view?->make();
}
function main(): void {
    let $holder = new Reversed();
    ABSENT_CALL
    if ($missing == null) { echo "absent;"; }
    let $first = capture($holder, false);
    if ($first != null) { echo "{$first()} "; }
    let $second = capture($holder, true);
    if ($second != null) { echo "{$second()}\n"; }
}
"#
    .replace("ABSENT_CALL", absent_call);
    assert_returned_closure_execution(&source, "absent;22 11\nholder;drop22;drop11;");
}

#[test]
fn returned_readonly_closure_can_observe_literal_null_without_a_dead_parameter_place() {
    assert_returned_closure_execution(
        r#"
class Child {
    function __construct(int $value) {}
    function __destruct() { try { echo "drop{$this->value};"; } catch (Error) {} }
}
function capture(?Child $child): function(): bool {
    return fn() with ($child) => $child == null;
}
function forward(?Child $child): function(): bool { return capture($child); }
function main(): void {
    let $child = new Child(42);
    let $present = forward($child);
    let $direct = capture(null);
    let $forwarded = forward(null);
    echo "{$present()} {$direct()} {$forwarded()}\n";
}
"#,
        "false true true\ndrop42;",
    );
}

#[test]
fn durable_returned_collection_borrows_preserve_exact_element_places() {
    assert_returned_closure_execution(
        include_str!("../../../examples/native/main_returned_collection_borrows.doria"),
        include_str!("fixtures/native_io/main_returned_collection_borrows/expected_stdout"),
    );
}

#[test]
fn returned_readonly_closures_can_observe_literal_null_callable_parameters() {
    assert_returned_closure_execution(
        r#"
class Child {
    function __construct(int $value) {}
    function __destruct() { try { echo "drop{$this->value};"; } catch (Error) {} }
}
function capture(?function(): int $callback): function(): bool {
    return fn() with ($callback) => $callback == null;
}
function forward(?function(): int $callback): function(): bool { return capture($callback); }
function main(): void {
    let $child = new Child(42);
    let $callback = fn() with (take $child) => $child->value;
    {
        let $present = forward($callback);
        let $direct = capture(null);
        let $forwarded = forward(null);
        let $grouped = capture((null));
        echo "{$present()} {$direct()} {$forwarded()} {$grouped()}\n";
    }
    echo "{$callback()}\n";
}
"#,
        "false true true true\n42\ndrop42;",
    );
}

#[test]
fn repeatable_closures_keep_owned_capture_families_until_environment_destruction() {
    assert_returned_closure_execution(
        r#"
class Child {
    function __construct(int $value) {}
    function __destruct() { try { echo "drop{$this->value};"; } catch (Error) {} }
}
enum Packet { case Value(Child $child); }
function make(): function(): int {
    let $child = new Child(55);
    return fn() with (take $child) => $child->value;
}
function main(): void {
    {
        let $child = new Child(11);
        List<Child> $children = [new Child(22)];
        let $packet = Packet::Value(new Child(33));
        let $innerChild = new Child(44);
        let $inner = fn() with (take $innerChild) => $innerChild->value;
        let $read = function (): int with (take $child, take $children, take $packet, take $inner) {
            let $packetValue = match ($packet) { Packet::Value($item) => $item->value };
            return $child->value + $children[0]->value + $packetValue + $inner();
        };
        let $escaped = make();
        let $label = "owned{$escaped()}";
        let $readLabel = fn() with (take $label) => $label;
        echo "{$read()} {$read()} {$escaped()} {$escaped()} {$readLabel()} {$readLabel()}\n";
        echo "alive\n";
    }
    echo "after\n";
}
"#,
        "110 110 55 55 owned55 owned55\nalive\ndrop55;drop44;drop33;drop22;drop11;after\n",
    );
}

#[test]
fn durable_repeatable_closure_captures_preserve_environment_lifetimes() {
    assert_returned_closure_execution(
        include_str!("../../../examples/native/main_repeatable_closure_captures.doria"),
        include_str!("fixtures/native_io/main_repeatable_closure_captures/expected_stdout"),
    );
}

#[test]
fn writable_repeatable_closures_keep_owned_captures_between_calls() {
    assert_returned_closure_execution(
        r#"
class Child {
    function __construct(int $value) {}
    function __destruct() { try { echo "drop{$this->value};"; } catch (Error) {} }
}
function main(): void {
    let writable $count = 0;
    {
        let $child = new Child(40);
        let writable $next = function (): int with (take $child, writable $count) {
            $count += 1;
            return $child->value + $count;
        };
        echo "{$next()} {$next()}\n";
        echo "alive\n";
    }
    echo "count{$count}\n";
}
"#,
        "41 42\nalive\ndrop40;count2\n",
    );
}

#[test]
fn repeatable_closures_preserve_owned_captures_across_checked_errors() {
    assert_returned_closure_execution(
        r#"
class Child {
    function __construct(int $value) {}
    function __destruct() { try { echo "drop{$this->value};"; } catch (Error) {} }
}
class Failure implements Error { function __construct(string $message) {} }
function main(): void {
    {
        let $child = new Child(42);
        let $read = function (bool $fail): int with (take $child) {
            let $temporary = new Child(9);
            if ($fail) { throw new Failure("failed"); }
            return $child->value;
        };
        try { echo "{$read(false)}\n"; } catch (Failure) { echo "unexpected;"; }
        try { $read(true); } catch (Failure) { echo "caught;"; }
        try { echo "{$read(false)}\n"; } catch (Failure) { echo "unexpected;"; }
        echo "alive\n";
    }
    echo "after\n";
}
"#,
        "drop9;42\ndrop9;caught;drop9;42\nalive\ndrop42;after\n",
    );
}

#[test]
fn unused_and_once_closures_release_remaining_owned_captures_exactly_once() {
    assert_returned_closure_execution(
        r#"
class Child {
    function __construct(int $value) {}
    function __destruct() { try { echo "drop{$this->value};"; } catch (Error) {} }
}
class Failure implements Error { function __construct(string $message) {} }
function main(): void {
    {
        let $child = new Child(5);
        let $unused = fn() with (take $child) => $child->value;
        echo "unused;";
    }
    {
        let $child = new Child(6);
        let $remaining = new Child(7);
        let $consume = function (): Child with (take $child, take $remaining) {
            echo "remaining{$remaining->value};";
            return $child;
        };
        let $result = $consume();
        echo "result{$result->value};";
    }
    {
        let $child = new Child(8);
        let $remaining = new Child(9);
        let $consume = function (bool $fail): Child with (take $child, take $remaining) {
            echo "remaining{$remaining->value};";
            if ($fail) { throw new Failure("failed"); }
            return $child;
        };
        try { let $result = $consume(true); } catch (Failure) { echo "caught;"; }
    }
    echo "after\n";
}
"#,
        "unused;drop5;remaining7;drop7;result6;drop6;remaining9;drop9;drop8;caught;after\n",
    );
}

#[test]
fn returned_readonly_closures_can_observe_literal_null_mixed_parameters() {
    for ty in ["mixed", "?mixed"] {
        let source = r#"
class Child {
    function __construct(int $value) {}
    function __destruct() { try { echo "drop{$this->value};"; } catch (Error) {} }
}
function capture(PAYLOAD_TYPE $payload): function(): bool {
    return fn() with ($payload) => $payload is Child;
}
function forward(PAYLOAD_TYPE $payload): function(): bool { return capture($payload); }
function main(): void {
    PAYLOAD_TYPE $payload = new Child(42);
    let $present = forward($payload);
    let $direct = capture(null);
    let $forwarded = forward(null);
    echo "{$present()} {$direct()} {$forwarded()}\n";
}
"#
        .replace("PAYLOAD_TYPE", ty);
        assert_returned_closure_execution(&source, "true false false\ndrop42;");
    }
}

#[test]
fn returned_readonly_closures_preserve_optional_views_of_move_enum_slots() {
    assert_returned_closure_execution(
        r#"
class Item {
    function __construct(int $number) {}
    function __destruct() { try { echo "drop{$this->number};"; } catch (Error) {} }
}
enum Packet { case Value(Item $item); }
function selectList(List<Packet> $packets): ?Packet { return $packets->first; }
function selectArray(Packet[] $packets, bool $present): ?Packet {
    if ($present) { return $packets[0]; }
    return null;
}
function selectDictionary(Dictionary<string, Packet> $packets, string $key): ?Packet {
    return $packets->get($key);
}
function capture(?Packet $packet): function(): int {
    return function (): int with ($packet) {
        if ($packet == null) { return -1; }
        return match ($packet) { Packet::Value($item) => $item->number };
    };
}
function forward(?Packet $packet): function(): int { return capture($packet); }
function main(): void {
    List<Packet> $list = [Packet::Value(new Item(11))];
    Packet[] $array = [Packet::Value(new Item(22))];
    Dictionary<string, Packet> $dictionary = ["found" => Packet::Value(new Item(33))];
    List<Packet> $empty = [];
    List<?Packet> $nullable = [Packet::Value(new Item(44)), null];
    let $fromList = forward(selectList($list));
    let $fromArray = forward(selectArray($array, true));
    let $fromDictionary = forward(selectDictionary($dictionary, "found"));
    let $emptyList = forward(selectList($empty));
    let $absentArray = forward(selectArray($array, false));
    let $missingKey = forward(selectDictionary($dictionary, "missing"));
    let $nullablePresent = forward($nullable[0]);
    let $nullableAbsent = forward($nullable[1]);
    echo "{$fromList()} {$fromArray()} {$fromDictionary()} ";
    echo "{$emptyList()} {$absentArray()} {$missingKey()} ";
    echo "{$nullablePresent()} {$nullableAbsent()}\n";
}
"#,
        "11 22 33 -1 -1 -1 44 -1\ndrop44;drop33;drop22;drop11;",
    );
}
