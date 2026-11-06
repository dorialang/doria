#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution.rs"]
mod native_execution;

#[path = "common/php_execution.rs"]
mod php_execution;

fn assert_returned_closure_execution(source: &str, stdout: &str) {
    let hir = doriac::lower_source("returned-closure-transport.doria", source)
        .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
    let mir = doriac::mir_lowering::lower_program(&hir)
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

#[test]
fn typed_owned_callbacks_allow_nullable_forwarding_and_loan_free_results() {
    assert_callback_hook_execution(
        r#"
class Cursor implements Iterator<int> {
    function __construct(borrow List<int> $source) {}
    function hasCurrent(): bool { return true; }
    function getCurrent(): int { return $this->source[0]; }
    writable function advance(): void {}
    function __destruct() { try { echo "drop;"; } catch (Error) {} }
}
class Factory<T> {
    function(take T): ?T $identity = function(take T $value): ?T { return $value; };
}
function main(): void {
    writable List<int> $source = [7];
    {
        function(take Cursor): ?Cursor $forward = function(take Cursor $value): ?Cursor { return $value; };
        let $cursor = $forward(new Cursor($source));
        if ($cursor != null) { echo $cursor->getCurrent(); }
        let $factory = new Factory<Cursor>();
        let $generic = ($factory->identity)(new Cursor($source));
        if ($generic != null) { echo $generic->getCurrent(); }
    }
    function(take Cursor): ?Cursor $discard = function(take Cursor $value): ?Cursor { return null; };
    let $absent = $discard(new Cursor($source));
    $source->add(8);
    echo $absent == null;
    function(take List<Cursor>): List<Cursor> $clear = function(take List<Cursor> $values): List<Cursor> { return []; };
    let $empty = $clear([new Cursor($source)]);
    $source->add(9);
    echo $empty->count;
    echo $source->count;
}
"#,
        "77drop;drop;drop;truedrop;03",
    );
}

#[test]
fn narrowed_mixed_payloads_keep_original_homes_through_borrowed_parameters() {
    let source = r#"
interface Named { function getName(): string; }
open class Item implements Named {
    function __construct(string $name) {}
    function getName(): string { return $this->name; }
}
class Child extends Item {
    function __construct(string $label) { parent::__construct($label); }
}
enum Packet { case Value(Item $item); }
function invoke(function(): int $callback): int { return $callback(); }
function lendCallback(function(): int $callback): function(): int {
    return fn() with ($callback) => $callback();
}
function lendItem(Item $item): function(): string {
    return fn() with ($item) => $item->getName();
}
function lendNamed(Named $item): function(): string {
    return fn() with ($item) => $item->getName();
}
function lendPacket(Packet $packet): function(): string {
    return fn() with ($packet) => match ($packet) { Packet::Value($item) => $item->getName() };
}
function main(): void {
    mixed $callback = fn() => 42;
    if ($callback is function(): int) {
        echo invoke($callback);
        let $later = lendCallback($callback);
        echo $later();
    }
    ?mixed $optional = fn() => 9;
    if ($optional is function(): int) { echo invoke($optional); }
    mixed $object = new Child("child;");
    if ($object is Item) { let $later = lendItem($object); echo $later(); }
    if ($object is Named) { let $later = lendNamed($object); echo $later(); }
    Named $view = new Child("view;");
    mixed $erased = $view;
    if ($erased is Item) { let $later = lendItem($erased); echo $later(); }
    if ($erased is Named) { let $later = lendNamed($erased); echo $later(); }
    mixed $packet = Packet::Value(new Child("packet;"));
    if ($packet is Packet) { let $later = lendPacket($packet); echo $later(); }
}
"#;
    assert_returned_closure_execution(source, "42429child;child;view;view;packet;");
}

fn lower_callback_hook_hir(source: &str) -> doriac::hir::Program {
    doriac::lower_source("hooks.doria", source)
        .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"))
}

fn assert_callback_hook_execution(source: &str, stdout: &str) -> doriac::mir::Program {
    let hir = lower_callback_hook_hir(source);
    let mir = doriac::mir_lowering::lower_program(&hir)
        .unwrap_or_else(|diagnostics| panic!("{source}\n{diagnostics:#?}"));
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
    let result = php_execution::Script::new(&script)
        .command(&[])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert!(result.stderr.is_empty(), "{result:?}");
    assert_eq!(result.stdout, stdout.as_bytes(), "{source}");
    mir
}

#[test]
fn collection_callback_storage_preserves_scalar_capture_scope_through_aliases() {
    let declarations = r#"
function bind(int $value): function(): int {
    let $callback = fn() with ($value) => $value;
    let $alias = $callback;
    return $alias;
}
"#;
    for callback in ["fn() with ($root) => $root", "bind($root)"] {
        assert_scalar_callback_factory_execution(declarations, callback);
    }
}

#[test]
fn generic_scalar_capture_factories_preserve_parameter_homes() {
    assert_scalar_callback_factory_execution(
        r#"
function bindGeneric<T>(int $value, T $marker): function(): int {
    let $callback = fn() with ($value) => $value;
    let $alias = $callback;
    return $alias;
}
"#,
        "bindGeneric($root, true)",
    );
}

#[test]
fn indirect_scalar_capture_factories_preserve_parameter_homes() {
    assert_scalar_callback_factory_execution(
        r#"
function bindIndirect(int $value): function(): int {
    let $factory = function(int $input): function(): int {
        return fn() with ($input) => $input;
    };
    return $factory($value);
}
"#,
        "bindIndirect($root)",
    );
}

fn assert_scalar_callback_factory_execution(declarations: &str, callback: &str) {
    let source = format!(
        r#"{declarations}
function main(): void {{
    let $root = 1;
    let $callback = {callback};
    List<function(): int> $callbacks = [$callback];
    echo $callbacks[0]();
}}
"#
    );
    assert_callback_hook_execution(&source, "1");

    let invalid = format!(
        r#"{declarations}
function main(): void {{
    writable List<function(): int> $callbacks = [];
    {{
        let $root = 1;
        let $callback = {callback};
        $callbacks->add($callback);
    }}
    echo $callbacks[0]();
}}
"#
    );
    let diagnostics = doriac::check_source("escaping-collection-capture.doria", &invalid)
        .expect_err("the collection must not outlive the captured scalar");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0658"),
        "{invalid}\n{diagnostics:#?}"
    );
}

#[test]
fn collection_borrow_only_callbacks_release_loans_at_last_use() {
    for (ty, literal, observation) in [
        (
            "List<function(): int>",
            "[capture($source)]",
            "echo $items[0]();",
        ),
        (
            "List<List<function(): int>>",
            "[[capture($source)]]",
            "echo $items[0][0]();",
        ),
    ] {
        let source = format!(
            r#"
class Source {{ writable int $value = 7; }}
function capture(Source $source): function(): int {{ return fn() with ($source) => $source->value; }}
function main(): void {{
    let writable $source = new Source();
    {ty} $items = {literal};
    {observation}
    $source->value = 8;
    echo $source->value;
}}
"#
        );
        assert_callback_hook_execution(&source, "78");
    }
}

#[test]
fn collection_literals_retain_sources_through_factories_and_extraction() {
    let declarations = r#"
class Cursor implements Iterator<int> {
    function __construct(borrow List<int> $source) {}
    function hasCurrent(): bool { return true; }
    function getCurrent(): int { return $this->source[0]; }
    writable function advance(): void {}
    function __destruct() { try { echo "drop{$this->source[0]};"; } catch (Error) {} }
}

function capture(List<int> $source): function(): int {
    let $cursor = new Cursor($source);
    return fn() with (take $cursor) => $cursor->getCurrent();
}
"#;
    for (ty, literal, extraction) in [
        ("List<Iterator<int>>", "[new Cursor($source)]", "let $item = $items->pop(); if ($item != null) { echo $item->getCurrent(); }"),
        ("Iterator<int>[]", "[new Cursor($source)]", "echo $items[0]->getCurrent();"),
        ("List<List<Iterator<int>>>", "[[new Cursor($source)]]", "let writable $inner = $items->removeAt(0); let $item = $inner->pop(); if ($item != null) { echo $item->getCurrent(); }"),
        ("List<?function(): int>", "[capture($source)]", "let $item = $items->removeAt(0); if ($item != null) { echo $item(); }"),
        ("Dictionary<string, ?function(): int>", "[\"callback\" => capture($source)]", "let $item = $items->remove(\"callback\"); if ($item != null) { echo $item(); }"),
    ] {
        let source = format!("{declarations} function make(List<int> $source): {ty} {{ return {literal}; }} function relay(take {ty} $items): {ty} {{ return $items; }} function main(): void {{ writable List<int> $source = [7]; {{ writable {ty} $items = relay(make($source)); {extraction} echo \"alive;\"; }} $source->add(8); echo $source->count; }}");
        assert_callback_hook_execution(&source, "7alive;drop7;2");
    }
}

#[test]
fn collection_mutations_retain_sources_through_helpers_and_extraction() {
    let declarations = r#"
class Key implements Comparable<Key>, Equatable<Key>, Hashable {
    function __construct(int $value) {}
    function compare(Key $other): Ordering {
        if ($this->value < $other->value) { return Ordering::Less; }
        if ($this->value > $other->value) { return Ordering::Greater; }
        return Ordering::Equal;
    }
    function equals(Key $other): bool { return $this->value == $other->value; }
    function hash(): uint64 { return 1; }
}
class Cursor implements Iterator<int> {
    function __construct(borrow List<int> $source) {}
    function hasCurrent(): bool { return true; }
    function getCurrent(): int { return $this->source[0]; }
    writable function advance(): void {}
    function __destruct() { try { echo "drop{$this->source[0]};"; } catch (Error) {} }
}
function capture(List<int> $source): function(): int {
    let $cursor = new Cursor($source);
    return fn() with (take $cursor) => $cursor->getCurrent();
}
"#;
    for (ty, initializer, store, extraction) in [
        (
            "List<Iterator<int>>",
            "[]",
            "$items->add(new Cursor($source));",
            "let $item = $items->pop(); if ($item != null) { echo $item->getCurrent(); }",
        ),
        (
            "List<Iterator<int>>",
            "[]",
            "$items->insertAt(0, new Cursor($source));",
            "echo $items[0]->getCurrent();",
        ),
        (
            "List<List<Iterator<int>>>",
            "[]",
            "$items->add([new Cursor($source)]);",
            "let $inner = $items->removeAt(0); echo $inner[0]->getCurrent();",
        ),
        (
            "List<?function(): int>",
            "[]",
            "$items->add(capture($source));",
            "let $item = $items->pop(); if ($item != null) { echo $item(); }",
        ),
        (
            "Dictionary<string, ?function(): int>",
            "[]",
            "$items->set(\"callback\", capture($source));",
            "let $item = $items->remove(\"callback\"); if ($item != null) { echo $item(); }",
        ),
        (
            "Dictionary<string, ?function(): int>",
            "[]",
            "$items[\"callback\"] = capture($source);",
            "let $item = $items->remove(\"callback\"); if ($item != null) { echo $item(); }",
        ),
        (
            "Deque<Iterator<int>>",
            "Deque::from([])",
            "$items->pushBack(new Cursor($source));",
            "let $item = $items->popFront(); if ($item != null) { echo $item->getCurrent(); }",
        ),
        (
            "Dictionary<Key, ?function(): int>",
            "[]",
            "$items->set(new Key(1), capture($source));",
            "let $item = $items->remove(new Key(1)); if ($item != null) { echo $item(); }",
        ),
        (
            "SortedDictionary<Key, ?function(): int>",
            "SortedDictionary::from([])",
            "$items[new Key(1)] = capture($source);",
            "let $item = $items->remove(new Key(1)); if ($item != null) { echo $item(); }",
        ),
    ] {
        let source = format!("{declarations} function install(writable {ty} $items, List<int> $source): void {{ {store} }} function make(List<int> $source): {ty} {{ writable {ty} $items = {initializer}; install($items, $source); return $items; }} function main(): void {{ writable List<int> $source = [7]; {{ writable {ty} $items = make($source); {extraction} echo \"alive;\"; }} $source->add(8); echo $source->count; }}");
        assert_callback_hook_execution(&source, "7alive;drop7;2");
    }
}

#[test]
fn collection_mutations_preserve_caller_owned_borrows_until_the_callers_last_use() {
    let declarations = r#"
class Source { writable int $value = 7; }
function capture(Source $source): function(): int { return fn() with ($source) => $source->value; }
function install(writable List<function(): int> $items, Source $source): void {
    $items->add(capture($source));
    echo "stored;";
}
interface InstallerContract {
    function install(writable List<function(): int> $items, Source $source): void;
}
class Installer implements InstallerContract {
    function install(writable List<function(): int> $items, Source $source): void {
        install($items, $source);
    }
}
"#;
    for installation in [
        "install($items, $source);",
        "let $installer = new Installer(); $installer->install($items, $source);",
        "InstallerContract $installer = new Installer(); $installer->install($items, $source);",
        "let $installer = function (writable List<function(): int> $items, Source $source): void { install($items, $source); }; $installer($items, $source);",
    ] {
        let source = format!("{declarations} function main(): void {{ let writable $source = new Source(); writable List<function(): int> $items = []; {installation} echo $items[0](); $source->value = 8; echo $source->value; }}");
        assert_callback_hook_execution(&source, "stored;78");
        let invalid = source.replace(
            "echo $items[0](); $source->value = 8;",
            "$source->value = 8; echo $items[0]();",
        );
        let diagnostics = doriac::lower_source("stored-callback.doria", &invalid).unwrap_err();
        assert!(diagnostics.iter().any(|d| d.code == "E0763"), "{installation}: {diagnostics:#?}");
    }
}

#[test]
fn closure_returns_are_independent_of_enclosing_whens_and_finalizers() {
    let source = r#"
function choose(bool $first): function(): int {
    return when ($first) {
        return function(): int {
            return when (true) { return 1; } else { return 0; };
        };
    } else {
        return fn() => 2;
    };
}
function main(): void {
    let $first = choose(true);
    let $second = choose(false);
    echo "{$first()};{$second()};";
    if (true) {} finally {
        let $callback = function(): int { return 3; };
        echo "{$callback()};";
    }
}
"#;
    assert_callback_hook_execution(source, "1;2;3;");
}

#[test]
fn nullable_indexed_views_keep_their_stored_type_through_callbacks() {
    for (ty, initializer, value) in [
        ("List<Book>", "[new Book(7)]", "$source[0]"),
        ("Book[]", "[new Book(7)]", "$source[0]"),
        (
            "Dictionary<string, Book>",
            "[\"slot\" => new Book(7)]",
            "$source[\"slot\"]",
        ),
        ("List<?Book>", "[new Book(7)]", "$source[0]"),
    ] {
        let source = format!(
            r#"
class Book {{
    function __construct(int $value) {{}}
    function __destruct() {{ try {{ echo "book{{$this->value}};"; }} catch (Error) {{}} }}
}}
function first({ty} $source): ?Book {{ return {value}; }}
function main(): void {{
    {ty} $books = {initializer};
    let $callback = fn({ty} $source) => first($source);
    let $view = $callback($books);
    if ($view != null) {{ echo "{{$view->value}};"; }}
}}
"#
        );
        assert_callback_hook_execution(&source, "7;book7;");
    }
}

#[test]
fn removed_callbacks_own_their_environments_and_release_them_once() {
    let source = r#"
class Token {
    function __construct(int $value) {}
    function __destruct() { try { echo "token{$this->value};"; } catch (Error) {} }
}
function takeLast(writable List<function(): int> $callbacks): ?function(): int {
    return $callbacks->pop();
}
function takeAt(writable List<function(): int> $callbacks): function(): int {
    return $callbacks->removeAt(0);
}
function main(): void {
    let $first = new Token(1);
    let $second = new Token(2);
    writable List<function(): int> $callbacks = [
        fn() with (take $first) => $first->value,
        fn() with (take $second) => $second->value
    ];
    {
        let $last = takeLast($callbacks);
        if ($last != null) { echo "{$last()};"; }
        let $at = takeAt($callbacks);
        echo "{$at()};alive;";
    }
    echo "done;";
}
"#;
    assert_callback_hook_execution(source, "2;1;alive;token1;token2;done;");
}

#[test]
fn nullable_collection_accessors_preserve_nested_and_mixed_values() {
    for (element, value, inspect) in [
        (
            "List<Token>",
            "[new Token(7)]",
            "if ($value != null) { echo \"{$value->count};\"; }",
        ),
        (
            "?List<Token>",
            "[new Token(7)]",
            "if ($value != null) { echo \"{$value->count};\"; }",
        ),
        (
            "mixed",
            "new Token(7)",
            "if ($value is Token) { echo \"{$value->value};\"; }",
        ),
        (
            "?mixed",
            "new Token(7)",
            "if ($value is Token) { echo \"{$value->value};\"; }",
        ),
    ] {
        for (setup, read, remove) in [
            (
                format!("writable List<{element}> $source = [{value}];"),
                "first",
                "pop()",
            ),
            (
                format!("writable Dictionary<string, {element}> $source = [\"slot\" => {value}];"),
                "get(\"slot\")",
                "remove(\"slot\")",
            ),
            (
                format!(
                    "writable SortedDictionary<string, {element}> $source = SortedDictionary::from([]); \
                    $source->set(\"slot\", {value});"
                ),
                "get(\"slot\")",
                "remove(\"slot\")",
            ),
            (
                format!(
                    "writable Deque<{element}> $source = Deque::from([]); $source->pushBack({value});"
                ),
                "peekFront",
                "popFront()",
            ),
        ] {
            let null_check = if element.starts_with('?') {
                let null_setup = setup.replace("$source", "$nulls").replace(value, "null");
                format!(
                    r#"
    {null_setup}
    if ($nulls->{read} == null) {{ echo "null;"; }}
    if ($nulls->{remove} == null and $nulls->count == 0) {{ echo "cleared;"; }}
"#
                )
            } else {
                String::new()
            };
            let source = format!(
                r#"
class Token {{
    function __construct(int $value) {{}}
    function __destruct() {{ try {{ echo "token;"; }} catch (Error) {{}} }}
}}
function main(): void {{
    {setup}
    {{ let $value = $source->{read}; {inspect} }}
    {{ let $value = $source->{remove}; {inspect} echo "alive;"; }}
    if ($source->{remove} == null) {{ echo "empty;"; }}
    {null_check}
    echo "done;";
}}
"#
            );
            let printed = if element.contains("List") { "1" } else { "7" };
            let null_output = if element.starts_with('?') {
                "null;cleared;"
            } else {
                ""
            };
            assert_callback_hook_execution(
                &source,
                &format!("{printed};{printed};alive;token;empty;{null_output}done;"),
            );
            let invalid = source.replace(
                &format!("{{ let $value = $source->{read}; {inspect} }}"),
                &format!("{{ let $value = $source->{read}; $source->clear(); {inspect} }}"),
            );
            let diagnostics = doriac::check_source("collection-read-loan.doria", &invalid)
                .expect_err("a live element borrow must protect its collection");
            assert!(
                diagnostics.iter().any(|diagnostic| diagnostic.code == "E0477"),
                "{invalid}\n{diagnostics:#?}"
            );
        }
    }
}

#[test]
fn nullable_mixed_presence_checks_evaluate_general_expressions_once() {
    let source = r#"
class Holder {
    function __construct(take ?mixed $value) {}
}
function produce(): ?mixed {
    echo "call;";
    return 7;
}
function main(): void {
    let $holder = new Holder(7);
    if ($holder->value != null) { echo "stored;"; }
    if (produce() is int) { echo "typed;"; }
    writable List<mixed> $values = [7];
    if ($values->pop() is int) { echo "removed;"; }
    if ($values->count == 0) { echo "empty;"; }
}
"#;
    assert_callback_hook_execution(source, "stored;call;typed;removed;empty;");
}

#[test]
fn collection_ingestion_through_shared_access_transfers_owned_values() {
    let source = r#"
class Token {
    function __construct(int $value) {}
    function __destruct() { try { echo "token{$this->value};"; } catch (Error) {} }
}
interface TokenQueue {
    writable function pushBack(take List<Token> $value): void;
    writable function popBack(): ?List<Token>;
}
class Adapter implements TokenQueue {
    writable Deque<List<Token>> $values = Deque::from([]);
    writable function pushBack(take List<Token> $value): void { $this->values->pushBack($value); }
    writable function popBack(): ?List<Token> { return $this->values->popBack(); }
}
function main(): void {
    let $owner = new WritableSharedReference<TokenQueue>(new Adapter());
    {
        let writable $access = $owner->acquireWritableAccess();
        let $first = [new Token(1)];
        $access->pushBack($first);
        $access->pushBack([new Token(2)]);
        echo "alive;";
        { let $value = $access->popBack(); if ($value != null) { echo "{$value->count};"; } }
        { let $value = $access->popBack(); if ($value != null) { echo "{$value->count};"; } }
    }
    echo "done;";
}
"#;
    assert_callback_hook_execution(source, "alive;1;token2;1;token1;done;");
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

fn assert_last_use_capture_cleanup(body: &str, expected: &str) {
    let declarations = r#"
class Token {
    function __construct(int $value) {}
    function __destruct() { try { echo "token{$this->value};"; } catch (Error) {} }
}
interface Reader { function(): int $callback { get; } }
class Source implements Reader {
    int $value = 42;
    function(): int $callback {
        get {
            let $first = new Token(1);
            let $second = new Token(2);
            return function (): int with ($this, take $first, take $second) {
                if ($first->value == 0 || $second->value == 0) { return 0; }
                return $this->value;
            };
        }
    }
    ?function(): int $optional { get => $this->callback; }
    function __destruct() { try { echo "source;"; } catch (Error) {} }
}
function forward(Reader $reader): function(): int { return $reader->callback; }
function optional(Source $source): ?Source { return $source; }
function wrap(take function(): int $callback): function(): int {
    return fn() with (take $callback) => $callback();
}
function lend(function(): int $callback): function(): int {
    return fn() with ($callback) => $callback();
}
function consume(take Source $source): void {}
"#;
    let source = format!(
        "{declarations} function exercise(bool $choose): void {{ let $source = new Source(); {{ {body} echo \"alive;\"; }} }} function main(): void {{ exercise(true); exercise(false); }}"
    );
    assert_callback_hook_execution(&source, &expected.repeat(2));
}

#[test]
fn direct_callback_last_use_preserves_owned_capture_cleanup() {
    assert_last_use_capture_cleanup(
        "let $callback = $source->callback; echo $callback(); consume($source);",
        "42source;alive;token2;token1;",
    );
}

#[test]
fn dispatched_callback_alias_last_use_preserves_owned_capture_cleanup() {
    assert_last_use_capture_cleanup(
        "let $callback = forward($source); let $alias = $callback; echo $alias(); consume($source);",
        "42source;alive;token2;token1;",
    );
}

#[test]
fn nullable_callback_last_use_preserves_owned_capture_cleanup() {
    assert_last_use_capture_cleanup(
        "let $callback = $source->optional; if ($callback != null) { echo $callback(); } consume($source);",
        "42source;alive;token2;token1;",
    );
}

#[test]
fn null_safe_callback_last_use_preserves_owned_capture_cleanup() {
    assert_last_use_capture_cleanup(
        "let $view = optional($source); let $callback = $view?->callback; if ($callback != null) { echo $callback(); } consume($source);",
        "42source;alive;token2;token1;",
    );
}

#[test]
fn indirect_callback_factory_last_use_preserves_owned_capture_cleanup() {
    assert_last_use_capture_cleanup(
        "let $factory = function (Source $input): function(): int { return $input->callback; }; let $callback = $factory($source); echo $callback(); consume($source);",
        "42source;alive;token2;token1;",
    );
}

#[test]
fn branched_callback_last_use_preserves_owned_capture_cleanup() {
    assert_last_use_capture_cleanup(
        "let $callback = $source->callback; if ($choose) { echo $callback(); consume($source); } else { echo $callback(); consume($source); }",
        "42source;alive;token2;token1;",
    );
}

#[test]
fn callback_last_use_keeps_reachable_join_loop_alias_and_error_path_uses_live() {
    let mut failures = Vec::new();
    for (case, body) in [
        (
            "join",
            "if ($choose) { echo $callback(); consume($source); } echo $callback();",
        ),
        (
            "loop",
            "while ($choose) { echo $callback(); consume($source); }",
        ),
        (
            "borrowed wrapper",
            "let $wrapper = fn() with ($callback) => $callback(); if ($choose) { consume($source); } echo $wrapper();",
        ),
        (
            "finally",
            "try { echo $callback(); consume($source); } finally { echo $callback(); }",
        ),
        (
            "checked catch",
            "try { echo $callback(); consume($source); echo \"after\"; } catch (Error) { echo $callback(); }",
        ),
    ] {
        let source = format!(
            r#"
class Source {{
    int $value = 42;
    function(): int $callback {{ get => fn() with ($this) => $this->value; }}
}}
function consume(take Source $source): void {{}}
function exercise(bool $choose): void {{
    let $source = new Source();
    let $callback = $source->callback;
    {body}
}}
function main(): void {{ exercise(true); }}
"#
        );
        let (_, analysis) =
            doriac::analyze_source_for_ide("callback-future-use.doria", &source).unwrap();
        if !analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0654")
        {
            failures.push(format!(
                "the callback's {case} use must retain its source: {:?}",
                analysis.diagnostics,
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn taking_callback_wrapper_last_use_preserves_owned_capture_cleanup() {
    assert_last_use_capture_cleanup(
        "let $callback = wrap($source->callback); echo $callback(); consume($source);",
        "42source;alive;token2;token1;",
    );
}

#[test]
fn borrowing_callback_wrapper_last_use_preserves_owned_capture_cleanup() {
    assert_last_use_capture_cleanup(
        "let $inner = $source->callback; let $callback = lend($inner); echo $callback(); consume($source);",
        "42source;alive;token2;token1;",
    );
}

#[test]
fn unused_callback_last_use_preserves_owned_capture_cleanup() {
    assert_last_use_capture_cleanup(
        "let $callback = $source->callback; consume($source);",
        "source;alive;token2;token1;",
    );
}

#[test]
fn checked_callback_last_use_preserves_capture_cleanup_on_both_outcomes() {
    assert_callback_hook_execution(
        r#"
class Failure implements Error { string $message = "failure"; }
class Token {
    int $value = 0;
    function __destruct() { try { echo "token;"; } catch (Error) {} }
}
class Source {
    function __construct(bool $fail) {}
    function(): int throws Failure $callback {
        get {
            let $token = new Token();
            return function (): int with ($this, take $token) {
                if ($this->fail) { throw new Failure(); }
                if ($token->value != 0) { return 0; }
                return 42;
            };
        }
    }
    function __destruct() { try { echo "source;"; } catch (Error) {} }
}
function consume(take Source $source): void {}
function exercise(bool $fail): void {
    let $source = new Source($fail);
    {
        let $callback = $source->callback;
        try { echo $callback(); } catch (Failure) { echo "caught;"; }
        consume($source);
        echo "alive;";
    }
}
function main(): void { exercise(false); exercise(true); }
"#,
        "42source;alive;token;caught;source;alive;token;",
    );
}

#[test]
fn owned_iterator_captures_keep_cleanup_loans_after_the_last_callback_call() {
    let declarations = r#"
class Cursor implements Iterator<int> {
    function __construct(borrow List<int> $source) {}
    function hasCurrent(): bool { return true; }
    function getCurrent(): int { return $this->source[0]; }
    writable function advance(): void {}
    function __destruct() {
        try { echo "cursor{$this->source[0]};"; } catch (Error) {}
    }
}
function capture(List<int> $source): function(): int {
    let $cursor = new Cursor($source);
    return fn() with (take $cursor) => $cursor->getCurrent();
}
"#;
    let source = format!(
        r#"{declarations}
function main(): void {{
    writable List<int> $source = [7];
    {{
        let $callback = capture($source);
        echo $callback();
        echo "alive;";
    }}
    $source->add(8);
    echo $source->count;
}}
"#
    );
    let mir = assert_callback_hook_execution(&source, "7alive;cursor7;2");
    let mut invalid = mir.clone();
    let main = invalid
        .functions
        .iter_mut()
        .find(|function| function.name == "main")
        .unwrap();
    let callback = main
        .locals
        .iter()
        .find(|local| local.name == "callback")
        .unwrap()
        .id;
    let mutation = main
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find(|statement| matches!(statement, doriac::mir::Statement::CollectionAdd { .. }))
        .expect("source mutation after the callback's lexical cleanup")
        .clone();
    let (block, index) = main
        .blocks
        .iter()
        .find_map(|block| {
            block
                .statements
                .iter()
                .position(|statement| {
                    matches!(statement,
                        doriac::mir::Statement::DropFunction { local, .. } if *local == callback
                    )
                })
                .map(|index| (block.id, index))
        })
        .expect("the owned iterator is dropped through callback cleanup");
    main.blocks[block.0].statements.insert(index, mutation);
    let error = doriac::mir_validation::validate_program(&invalid).expect_err(
        "the owned iterator destructor still needs its source after the final invocation",
    );
    assert!(error.message.contains("retains it"), "{error:?}");

    for factory in ["capture($source)", "$factory($source)"] {
        let invalid_source = format!(
            r#"{declarations}
function main(): void {{
    writable List<int> $source = [7];
    let $factory = function (List<int> $input): function(): int {{ return capture($input); }};
    let $callback = {factory};
    let $observed = $callback();
    $source->add(8);
}}
"#
        );
        let (_, analysis) =
            doriac::analyze_source_for_ide("cleanup-loan.doria", &invalid_source).unwrap();
        let retained_sources = analysis
            .info
            .retained_callables
            .iter()
            .filter(|(span, _)| span.source == doriac::source::SourceId::default())
            .collect::<Vec<_>>();
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0654"),
            "the frontend must retain the owned iterator's cleanup dependency for {factory}: {:?}; retained sources: {retained_sources:?}; return borrows: {:?}",
            analysis.diagnostics, analysis.info.return_borrows
        );
    }
}

#[test]
fn generic_owned_callbacks_preserve_concrete_source_loans_without_inventing_them() {
    for callback in [
        "fn(take T $item) => $item",
        "function(take T $item): T { return $item; }",
    ] {
        for declaration in ["let $identity", "function(take T): T $identity"] {
            let declarations = format!(
                r#"
class Item {{ function __construct(int $value) {{}} }}
class Envelope<T> {{ function __construct(take T $value) {{}} }}
class Cursor implements Iterator<int> {{
    function __construct(borrow List<int> $source) {{}}
    function hasCurrent(): bool {{ return true; }}
    function getCurrent(): int {{ return $this->source[0]; }}
    writable function advance(): void {{}}
}}
function relay<T>(take T $value): T {{
    {declaration} = {callback};
    return $identity($value);
}}
"#
            );
            let source = format!(
                r#"{declarations}
function main(): void {{
    echo relay(42);
    echo relay(new Item(3))->value;
    echo relay([4, 5])[0];
    writable List<int> $source = [7];
    {{
        let $cursor = relay(new Cursor($source));
        echo $cursor->getCurrent();
        let $owner = relay(new Envelope<Cursor>(new Cursor($source)));
        echo $owner->value->getCurrent();
        let $cursors = relay([new Cursor($source)]);
        echo $cursors[0]->getCurrent();
    }}
    $source->add(8);
    echo $source->count;
}}
"#
            );
            let program = assert_callback_hook_execution(&source, "42347772");
            let mut proved_owned_source = false;
            for signature in &program.function_types {
                if signature.parameters.iter().any(|parameter| {
                matches!(parameter.ty, doriac::mir::Type::Class(class) if program.classes[class.0].declaration_name == "Item")
            }) {
                assert!(signature.return_borrow.is_none());
            }
                if let Some(borrow) = signature.return_borrow {
                    let doriac::mir::BorrowSource::Parameter(index) = borrow.source else {
                        continue;
                    };
                    if signature.parameters[index].mode == doriac::mir::FunctionParameterMode::Take
                    {
                        assert!(signature.parameters[index].ty.has_move_ownership());
                        proved_owned_source = true;
                    }
                }
            }
            assert!(proved_owned_source);
            let mut forged_inputs = 0;
            for function in program
                .functions
                .iter()
                .filter(|function| function.name == "relay")
            {
                let mut forged = program.clone();
                let mut changed = false;
                for statement in forged.functions[function.id.0]
                    .blocks
                    .iter_mut()
                    .flat_map(|block| &mut block.statements)
                {
                    if let doriac::mir::Statement::ControlFlowPlan(
                        doriac::mir::ControlFlowPlan::RetainedSources(plan),
                    ) = statement
                    {
                        if let Some(storage) = plan
                            .returned_storage
                            .as_mut()
                            .filter(|storage| !storage.inputs.is_empty())
                        {
                            storage.inputs.clear();
                            changed = true;
                        }
                    }
                }
                if changed {
                    forged_inputs += 1;
                    assert!(doriac::mir_validation::validate_program(&forged).is_err());
                }
            }
            assert_eq!(forged_inputs, 3);
            let invalid = source.replace(
                "echo $cursor->getCurrent();",
                "echo $cursor->getCurrent(); $source->add(9);",
            );
            assert!(
                doriac::check_source("generic-callback-mutation.doria", invalid).is_err(),
                "{callback} must preserve the live source loan"
            );
            let escaping = format!(
            "{declarations} function escape(): Cursor {{ let $source = [7]; return relay(new Cursor($source)); }} function main(): void {{}}"
        );
            assert!(doriac::check_source("generic-callback-escape.doria", escaping).is_err());
        }
    }
}

#[test]
fn constructed_owners_forward_inherited_cleanup_loans_without_borrowed_fields() {
    let cursor = r#"
class Cursor implements Iterator<int> {
    function __construct(borrow List<int> $source) {}
    function hasCurrent(): bool { return true; }
    function getCurrent(): int { return $this->source[0]; }
    writable function advance(): void {}
    function __destruct() {
        try { echo "cursor{$this->source[0]};"; } catch (Error) {}
    }
}
"#;
    for owner in [
        r#"class Envelope {
            Cursor $cursor;
            function __construct(parameter take Cursor $cursor) { $this->cursor = $cursor; }
            function getCurrent(): int { return $this->cursor->getCurrent(); }
        }"#,
        r#"class Envelope {
            function __construct(take Cursor $cursor) {}
            function getCurrent(): int { return $this->cursor->getCurrent(); }
        }"#,
        r#"class Inner { function __construct(take Cursor $cursor) {} }
        class Envelope {
            Inner $inner;
            function __construct(parameter take Cursor $cursor) { $this->inner = new Inner($cursor); }
            function getCurrent(): int { return $this->inner->cursor->getCurrent(); }
        }"#,
        r#"open class Parent {
            function __construct(take Cursor $cursor) {}
            function getCurrent(): int { return $this->cursor->getCurrent(); }
        }
        class Envelope extends Parent {
            function __construct(parameter take Cursor $cursor) { parent::__construct($cursor); }
        }"#,
        r#"class Inner<T> { function __construct(take T $value) {} }
        class Envelope {
            Inner<Cursor> $inner;
            function __construct(parameter take Cursor $cursor) { $this->inner = new Inner<Cursor>($cursor); }
            function getCurrent(): int { return $this->inner->value->getCurrent(); }
        }"#,
        r#"class Inner<T> { function __construct(take List<T> $values) {} }
        class Envelope {
            Inner<Cursor> $inner;
            function __construct(parameter take Cursor $cursor) { $this->inner = new Inner<Cursor>([$cursor]); }
            function getCurrent(): int { return $this->inner->values[0]->getCurrent(); }
        }"#,
    ] {
        let declarations = format!(
            "{cursor}{owner} function wrap(take Cursor $input): Envelope {{ return new Envelope($input); }}"
        );
        let source = format!(
            r#"{declarations}
function main(): void {{
    writable List<int> $source = [7];
    {{
        let $owner = wrap(new Cursor($source));
        echo $owner->getCurrent();
        echo "alive;";
    }}
    $source->add(8);
    echo $source->count;
}}
"#
        );
        let program = assert_callback_hook_execution(&source, "7alive;cursor7;2");
        for edit in ["$source->add(8);", "let $taken = $source;"] {
            let invalid = source.replace("echo \"alive;\";", &format!("echo \"alive;\"; {edit}"));
            assert!(doriac::check_source("constructed-owner-loan.doria", invalid).is_err());
        }
        let escaping = format!(
            "{declarations} function escape(): Envelope {{ let $source = [7]; return wrap(new Cursor($source)); }} function main(): void {{}}"
        );
        assert!(doriac::check_source("constructed-owner-escape.doria", escaping).is_err());
        let constructors = program
            .functions
            .iter()
            .filter(|function| {
                function.blocks.iter().flat_map(|block| &block.statements).any(|statement| {
                    matches!(statement, doriac::mir::Statement::ControlFlowPlan(doriac::mir::ControlFlowPlan::RetainedSources(plan)) if plan.constructs.iter().any(|source| source.inherited))
                })
            })
            .map(|function| function.id)
            .collect::<Vec<_>>();
        assert!(!constructors.is_empty());
        for constructor in constructors {
            for forgery in 0..3 {
                let mut invalid = program.clone();
                let plan = invalid.functions[constructor.0]
                    .blocks
                    .iter_mut()
                    .flat_map(|block| &mut block.statements)
                    .find_map(|statement| match statement {
                        doriac::mir::Statement::ControlFlowPlan(
                            doriac::mir::ControlFlowPlan::RetainedSources(plan),
                        ) if plan.constructs.iter().any(|source| source.inherited) => Some(plan),
                        _ => None,
                    })
                    .expect("owning constructors forward inherited cleanup loans");
                match forgery {
                    0 => plan.constructs.clear(),
                    1 => plan
                        .constructs
                        .iter_mut()
                        .for_each(|source| source.inherited = false),
                    _ => plan.constructs.iter_mut().for_each(|source| {
                        source.lifetime = doriac::types::RetainedLifetime::LastUse
                    }),
                }
                assert!(
                    doriac::mir_validation::validate_program(&invalid).is_err(),
                    "constructor {constructor:?}, forgery {forgery}"
                );
            }
        }
    }
}

#[test]
fn constructed_generic_owners_do_not_invent_loans_for_plain_specializations() {
    let source = r#"
class Item { function __construct(int $value) {} }
class Envelope<T> { function __construct(take T $value) {} }
function identity<T>(take T $value): T { return $value; }
function wrap<T>(take T $value): Envelope<T> { return new Envelope<T>($value); }
function main(): void {
    let $number = wrap(identity(42));
    let $item = wrap(identity(new Item(7)));
    let $numbers = wrap(identity([3, 4]));
    let $items = wrap(identity([new Item(5)]));
    echo $number->value;
    echo $item->value->value;
    echo $numbers->value[0];
    echo $items->value[0]->value;
}
"#;
    let program = assert_callback_hook_execution(source, "42735");
    for function in &program.functions {
        for statement in function.blocks.iter().flat_map(|block| &block.statements) {
            if let doriac::mir::Statement::ControlFlowPlan(
                doriac::mir::ControlFlowPlan::RetainedSources(plan),
            ) = statement
            {
                assert!(plan.constructs.is_empty(), "{}: {plan:?}", function.name);
                assert!(plan.returns.is_empty(), "{}: {plan:?}", function.name);
                assert!(
                    plan.returned_storage.is_none(),
                    "{}: {plan:?}",
                    function.name
                );
            }
        }
    }
}

#[test]
fn constructed_generic_specializations_keep_their_loan_contracts_separate() {
    let source = r#"
class Cursor implements Iterator<int> {
    function __construct(borrow List<int> $source) {}
    function hasCurrent(): bool { return true; }
    function getCurrent(): int { return $this->source[0]; }
    writable function advance(): void {}
}
class Envelope<T> { function __construct(take T $value) {} }
function relay<T>(take Envelope<T> $value): Envelope<T> { return $value; }
function main(): void {
    writable List<int> $source = [7];
    {
        let $cursor = relay(new Envelope<Cursor>(new Cursor($source)));
        let $number = relay(new Envelope<int>(42));
        echo $cursor->value->getCurrent();
        echo $number->value;
    }
    $source->add(8);
    echo $source->count;
}
"#;
    assert_callback_hook_execution(source, "7422");
    let invalid = source.replace(
        "echo $number->value;",
        "$source->add(8); echo $number->value;",
    );
    assert!(doriac::check_source("mixed-generic-owner-loans.doria", invalid).is_err());
}

#[test]
fn constructing_an_owner_does_not_allow_storing_its_loans_in_another_receiver() {
    let cursor = r#"
class Cursor implements Iterator<int> {
    function __construct(borrow List<int> $source) {}
    function hasCurrent(): bool { return true; }
    function getCurrent(): int { return $this->source[0]; }
    writable function advance(): void {}
}
class Target { writable ?Cursor $cursor = null; }
"#;
    for (storage, target) in [
        ("class Envelope { function __construct(parameter writable Target $target, parameter take Cursor $cursor) { $target->cursor = $cursor; } } function store(writable Target $target, take Cursor $cursor): void { let $owner = new Envelope($target, $cursor); }", "new Target()"),
        ("function store(writable Target $target, take Cursor $cursor): void { $target->cursor = $cursor; }", "new Target()"),
        ("class GenericTarget<T> { writable ?T $cursor = null; } function store<T>(writable GenericTarget<T> $target, take T $cursor): void { $target->cursor = $cursor; }", "new GenericTarget<Cursor>()"),
    ] {
        let source = format!("{cursor}{storage} function main(): void {{ let writable $target = {target}; let $source = [7]; store($target, new Cursor($source)); }}");
        let diagnostics = doriac::check_source("unrelated-owner-storage.doria", source).unwrap_err();
        assert!(diagnostics.iter().any(|diagnostic| diagnostic.code == "E0762"), "{diagnostics:#?}");
    }
}

#[test]
fn taking_source_carriers_into_returned_callbacks_preserves_cleanup_contracts() {
    for callback in [
        "fn() with (take $input) => $input->getCurrent()",
        "function (): int with (take $input) { return $input->getCurrent(); }",
    ] {
        let source = format!(
            r#"
class Cursor implements Iterator<int> {{
    function __construct(borrow List<int> $source) {{}}
    function hasCurrent(): bool {{ return true; }}
    function getCurrent(): int {{ return $this->source[0]; }}
    writable function advance(): void {{}}
    function __destruct() {{
        try {{ echo "cursor{{$this->source[0]}};"; }} catch (Error) {{}}
    }}
}}
function keep(take Cursor $input): function(): int {{ return {callback}; }}
function main(): void {{
    writable List<int> $source = [7];
    {{
        let $callback = keep(new Cursor($source));
        echo $callback();
        echo "alive;";
    }}
    $source->add(8);
    echo $source->count;
}}
"#
        );
        let program = assert_callback_hook_execution(&source, "7alive;cursor7;2");
        let keep = program
            .functions
            .iter()
            .find(|function| function.name == "keep")
            .unwrap()
            .id;
        for forgery in 0..4 {
            let mut invalid = program.clone();
            let function = &mut invalid.functions[keep.0];
            if forgery == 2 {
                function.return_borrow.as_mut().unwrap().kind =
                    doriac::mir::ReturnBorrowKind::Value;
            } else if forgery == 3 {
                function.return_borrow.as_mut().unwrap().lifetime =
                    doriac::types::RetainedLifetime::LastUse;
            } else {
                let plan = function
                    .blocks
                    .iter_mut()
                    .flat_map(|block| &mut block.statements)
                    .find_map(|statement| match statement {
                        doriac::mir::Statement::ControlFlowPlan(
                            doriac::mir::ControlFlowPlan::RetainedSources(plan),
                        ) => Some(plan),
                        _ => None,
                    })
                    .unwrap();
                if forgery == 0 {
                    plan.returns.clear();
                } else {
                    for source in &mut plan.returns {
                        source.inherited = false;
                    }
                }
            }
            doriac::mir_validation::validate_program(&invalid)
                .expect_err("a carrier type alone cannot prove inherited cleanup loans");
        }
    }
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
fn inline_collection_callable_loan_is_a_materialized_index_place() {
    assert_callback_hook_execution(
        r#"
function main(): void {
    writable List<int> $values = [0];
    let $read = fn(List<int> $source) => $source;
    echo $read($values)[0];
    $values[0] = 1;
    echo $values[0];
}
"#,
        "01",
    );
}

#[test]
fn contextual_collection_places_keep_checked_storage_types_in_nested_consumers() {
    assert_callback_hook_execution(
        r#"
class Holder { function __construct(take Bytes $bytes) {} }
function inspect(Bytes $bytes): void { echo "{$bytes->length}:{$bytes[0]};"; }
function main(): void {
    inspect(Bytes::fromArray([1, 2, 3]));
    let $holder = new Holder(Bytes::fromArray(([128, 255])));
    inspect($holder->bytes);
    let $make = fn() => Bytes::fromArray([255]);
    inspect($make());
    List<Bytes> $nested = [Bytes::fromArray([8])];
    inspect($nested[0]);
    echo Bytes::fromArray([9])[0];
}
"#,
        "3:1;2:128;1:255;1:8;9",
    );
}

#[test]
fn contextual_collection_places_preserve_the_durable_writable_shared_domains_fixture() {
    assert_returned_closure_execution(
        include_str!("../../../examples/native/main_stage25a_writable_shared_domains.doria"),
        include_str!("fixtures/native_io/main_stage25a_writable_shared_domains/expected_stdout"),
    );
}

#[test]
fn shared_collection_projections_keep_read_and_write_borrow_modes_distinct() {
    let mut mir = doriac::lower_source_to_mir(
        "shared-collection-projection.doria",
        r#"
function main(): void {
    let $owner = new WritableSharedReference(Bytes::fromArray([1, 2]));
    let writable $access = $owner->acquireWritableAccess();
    $access[0] = 7;
    echo "{$access[0]}:{$access->length}";
}
"#,
    )
    .unwrap();
    let function = mir
        .functions
        .iter_mut()
        .find(|function| function.name == "main")
        .unwrap();
    let mut reads = 0;
    let mut writes = 0;
    for statement in function
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.statements)
    {
        if let doriac::mir::Statement::AssignLocal {
            target,
            value:
                doriac::mir::Rvalue::Collection(
                    doriac::mir::CollectionExpression::SharedAccessPayload { writable, .. },
                ),
        } = statement
        {
            if function.locals[target.0].writable {
                writes += 1;
                *writable = false;
            } else {
                reads += 1;
            }
        }
    }
    assert_eq!((reads, writes), (2, 1));
    doriac::mir_validation::validate_program(&mir)
        .expect_err("a writable projection must still prove writable source access");
}

#[test]
fn nullable_stored_properties_share_guarded_loan_projection() {
    assert_callback_hook_execution(
        r#"
class Item { int $number = 9; }
class Store {
    List<int> $values = [7];
    int[] $array = [8];
    ?List<int> $optional = [6];
    ?List<int> $missing = null;
    Dictionary<string, int> $dictionary = ["key" => 5];
    function(): int $callback = fn() => 4;
    Item $item = new Item();
    string $text = "text;";
    int $number = 3;
}
function lend(?Store $source): ?List<int> { return $source?->values; }
function lendArray(?Store $source): ?int[] { return $source?->array; }
function lendOptional(?Store $source): ?List<int> { return $source?->optional; }
function lendDictionary(?Store $source): ?Dictionary<string, int> {
    return $source?->dictionary;
}
function lendCallback(?Store $source): ?function(): int { return $source?->callback; }
function lendItem(?Store $source): ?Item { return $source?->item; }
function receiver(Store $source): ?Store { echo "receiver;"; return $source; }
function main(): void {
    let $store = new Store();
    let $values = lend($store);
    let $array = lendArray($store);
    let $optional = lendOptional($store);
    let $dictionary = lendDictionary($store);
    let $callback = lendCallback($store);
    if ($values != null) { echo $values[0]; }
    if ($array != null) { echo $array[0]; }
    if ($optional != null) { echo $optional[0]; }
    if ($dictionary != null) { echo $dictionary["key"]; }
    if ($callback != null) { echo $callback(); }
    echo lend(null) == null;
    echo lendArray(null) == null;
    echo lendOptional(null) == null;
    echo lendDictionary(null) == null;
    echo lendCallback(null) == null;
    let $view = receiver($store);
    echo $view?->missing == null;
    echo $view?->text ?? "absent;";
    echo $view?->number ?? 0;
    let $item = lendItem($view);
    if ($item != null) { echo $item->number; }
}
"#,
        "78654truetruetruetruetruereceiver;truetext;39",
    );
}

#[test]
fn nullable_stored_property_loans_keep_temporary_shared_owners_until_statement_end() {
    let source = r#"
class Store {
    List<int> $values = [7];
    int $referencedValue = 11;
    function __destruct() { try { echo "drop;"; } catch (Error) {} }
}
function make(bool $present): ?SharedReference<Store> {
    if ($present) { return shared new Store(); }
    return null;
}
function makePresent(): SharedReference<Store> { return shared new Store(); }
function main(): void {
    echo (make(true)?->values ?? [0])[0];
    echo (make(false)?->values ?? [0])[0];
    echo makePresent()->referencedValue->values[0];
    echo (make(true)?->referencedValue?->values ?? [0])[0];
    echo makePresent()->referencedValue->referencedValue;
}
"#;
    assert_returned_closure_execution(source, "7drop;07drop;7drop;11drop;");
    let hir = lower_callback_hook_hir(source);
    let mir = doriac::mir_lowering::lower_program(&hir).unwrap();
    let error = doriac::codegen_php::generate(&hir, Some(&mir))
        .expect_err("PHP keeps its explicit shared-ownership boundary");
    assert!(error
        .diagnostics
        .as_ref()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic.code == "B2301"));
}

#[test]
fn nullable_stored_property_projection_requires_real_presence_proof() {
    let mut mir = doriac::lower_source_to_mir(
        "nullable-stored-property-proof.doria",
        r#"
class Store { List<int> $values = [7]; }
function lend(?Store $source): ?List<int> { return $source?->values; }
function main(): void { let $store = new Store(); echo lend($store) == null; }
"#,
    )
    .unwrap();
    let function = mir
        .functions
        .iter_mut()
        .find(|function| function.name == "lend")
        .unwrap();
    let condition = function
        .blocks
        .iter_mut()
        .find_map(|block| match &mut block.terminator {
            doriac::mir::Terminator::Branch {
                condition: condition @ doriac::mir::BoolExpression::NullableClassIsPresent(_),
                ..
            } => Some(condition),
            _ => None,
        })
        .expect("stored property projection must guard its nullable receiver");
    *condition = doriac::mir::BoolExpression::Use {
        operand: doriac::mir::Operand::Scalar(doriac::mir::ScalarValue::Bool(true)),
    };
    let error = doriac::mir_validation::validate_program(&mir)
        .expect_err("a constant true branch is not a presence proof");
    assert!(
        error
            .message
            .contains("without a dominating presence proof"),
        "{error:?}"
    );
}

#[test]
fn inline_collection_writable_call_results_keep_the_original_source_place() {
    assert_callback_hook_execution(
        r#"
function loan(writable List<int> $source): List<int> { return $source; }
function main(): void {
    writable List<int> $values = [0];
    let $write = fn(writable List<int> $source) => $source;
    $write($values)[0] = 3;
    echo $values[0];
    loan($values)[0] += 2;
    echo $values[0];
}
"#,
        "35",
    );
}

#[test]
fn inline_collection_places_preserve_direct_callable_nested_and_nullable_loans() {
    assert_callback_hook_execution(
        r#"
function listLoan(List<int> $source): List<int> { return $source; }
function maybe(List<int> $source, bool $present): ?List<int> {
    if ($present) { return $source; }
    return null;
}
function main(): void {
    writable List<int> $values = [4];
    writable int[] $array = [5];
    writable Dictionary<string, int> $dictionary = ["key" => 6];
    writable List<List<int>> $matrix = [[7]];
    writable Bytes $bytes = Bytes::allocate(1);
    $bytes[0] = 8;
    let $read = fn(List<int> $source) => $source;
    let $readArray = fn(int[] $source) => $source;
    let $readDictionary = fn(Dictionary<string, int> $source) => $source;
    let $readMatrix = fn(List<List<int>> $source) => $source;
    let $readBytes = fn(Bytes $source) => $source;
    echo listLoan($read($values))[0];
    echo (($readArray($array)))[0];
    echo $readDictionary($dictionary)["key"];
    echo $readMatrix($matrix)[0][0];
    echo $readBytes($bytes)[0];
    echo (maybe($values, true) ?? $values)[0];
    echo (maybe($values, false) ?? $read($values))[0];
}
"#,
        "4567844",
    );
}

#[test]
fn inline_collection_owned_results_clean_up_once_after_the_indexed_use() {
    assert_callback_hook_execution(
        r#"
class Item {
    function __construct(int $number) {}
    function __destruct() { try { echo "drop{$this->number};"; } catch (Error) {} }
}
function make(): List<Item> { return [new Item(8)]; }
function main(): void {
    let $create = function(): List<Item> { return [new Item(9)]; };
    echo "{make()[0]->number};";
    echo "{$create()[0]->number};";
}
"#,
        "8;drop8;9;drop9;",
    );
}

#[test]
fn inline_collection_nested_places_preserve_argument_evaluation_order() {
    assert_callback_hook_execution(
        r#"
function before(): int { echo "A;"; return 1; }
function matrixLoan(List<List<int>> $source): List<List<int>> {
    echo "M;";
    return $source;
}
function index(): int { echo "I;"; return 0; }
function consume(int $first, int $second): void { echo "{$first}:{$second};"; }
function main(): void {
    List<List<int>> $matrix = [[7]];
    consume(before(), matrixLoan($matrix)[index()][index()]);
}
"#,
        "A;M;I;I;1:7;",
    );
}

#[test]
fn inline_collection_nullable_places_keep_fallback_evaluation_lazy() {
    assert_callback_hook_execution(
        r#"
function maybe(List<int> $source, bool $present): ?List<int> {
    if ($present) { return $source; }
    return null;
}
function matrixLoan(List<List<int>> $source): List<List<int>> {
    echo "fallback;";
    return $source;
}
function main(): void {
    List<int> $values = [4];
    List<List<int>> $matrix = [[7]];
    echo (maybe($values, true) ?? matrixLoan($matrix)[0])[0];
    echo (maybe($values, false) ?? matrixLoan($matrix)[0])[0];
}
"#,
        "4fallback;7",
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
