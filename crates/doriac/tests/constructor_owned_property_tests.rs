use doriac::mir;

fn diagnostics(source: &str) -> Vec<doriac::diagnostics::Diagnostic> {
    doriac::check_source("constructor-owned-property.doria", source)
        .expect_err("source should be rejected")
}

fn assert_diagnostic(source: &str, code: &str) {
    let found = diagnostics(source);
    assert!(
        found.iter().any(|diagnostic| diagnostic.code == code),
        "expected {code}, got {found:#?}"
    );
}

fn lower(source: &str) -> mir::Program {
    doriac::lower_source_to_mir("constructor-owned-property.doria", source)
        .expect("source should lower to MIR")
}

fn interpret(source: &str) -> doriac::mir_interpreter::InterpreterOutput {
    doriac::mir_interpreter::interpret(&lower(source)).expect("MIR should interpret")
}

#[test]
fn constructor_root_derives_writable_access_through_initialized_properties() {
    let source = r#"
class Counter
{
    writable int $value = 0;
    writable List<int> $items = [1];

    writable function add(int $amount): void
    {
        $this->value += $amount;
    }
}

class Layer
{
    writable Counter $counter = new Counter();
}

class Application
{
    internal writable Layer $layer = new Layer();

    function __construct()
    {
        $this->layer->counter->value = 10;
        $this->layer->counter->value++;
        $this->layer->counter->value += 20;
        $this->layer->counter->add(10);
        $this->layer->counter->items->add(2);
        $this->layer->counter->items[0] = 42;
    }

    function value(): int
    {
        return $this->layer->counter->value;
    }

    function first(): int
    {
        return $this->layer->counter->items[0];
    }
}

function main(): void throws Doria\Std\Io\IoError
{
    let $application = new Application();
    echo "{$application->value()}:{$application->first()}";
}
"#;

    let output = interpret(source);
    assert_eq!(output.stdout, b"41:42");
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
}

#[test]
fn constructor_can_initialize_owned_property_and_writable_method_can_replace_it() {
    let source = r#"
class Window
{
    string $title;

    function __construct(string $input)
    {
        $this->title = $input;
    }
}

class Application
{
    internal writable Window $window;

    function __construct(string $initialTitle)
    {
        $this->window = new Window($initialTitle);
    }

    writable function replace(take Window $window): void
    {
        $this->window = $window;
    }

    function title(): string
    {
        return $this->window->title;
    }
}

function main(): void throws Doria\Std\Io\IoError
{
    let writable $application = new Application("Doria");
    echo $application->title() . "\n";
    let $replacement = new Window("Ready");
    $application->replace($replacement);
    echo $application->title() . "\n";
}
"#;

    let output = interpret(source);
    assert_eq!(output.stdout, b"Doria\nReady\n");
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
}

#[test]
fn constructor_root_does_not_bypass_readonly_or_initialization_rules() {
    assert_diagnostic(
        r#"
class Window { writable string $title = ""; }
class Application
{
    Window $window = new Window();
    function __construct() { $this->window->title = "Doria"; }
}
"#,
        "E0201",
    );

    assert_diagnostic(
        r#"
class Window { string $title = ""; }
class Application
{
    writable Window $window = new Window();
    function __construct() { $this->window->title = "Doria"; }
}
"#,
        "E0202",
    );

    assert_diagnostic(
        r#"
class Window { writable string $title = ""; }
class Application
{
    writable Window $window;
    function __construct() { $this->window->title = "Doria"; }
}
"#,
        "E0501",
    );
}

#[test]
fn constructor_root_respects_branch_nullable_and_shared_path_provenance() {
    doriac::check_source(
        "constructor-branch-path.doria",
        r#"
class Window { writable string $title = ""; }
class Application
{
    writable Window $window;

    function __construct(bool $enabled)
    {
        if ($enabled) {
            $this->window = new Window();
            $this->window->title = "enabled";
        } else {
            $this->window = new Window();
        }
    }
}
"#,
    )
    .expect("a branch-local direct initialization should enable nested writable access");

    assert_diagnostic(
        r#"
class Window { writable string $title = ""; }
class Application
{
    writable Window $window;

    function __construct(bool $enabled)
    {
        if ($enabled) { $this->window = new Window(); }
        $this->window->title = "not definite";
        $this->window = new Window();
    }
}
"#,
        "E0501",
    );

    assert_diagnostic(
        r#"
class Window { writable string $title = ""; }
class Application
{
    writable ?Window $window = null;
    function __construct(bool $enabled)
    {
        if ($enabled) { $this->window = new Window(); }
        $this->window->title = "not narrowed";
    }
}
"#,
        "E0506",
    );

    assert_diagnostic(
        r#"
class Counter { writable int $value = 0; }
class Application
{
    writable WritableSharedReference<Counter> $counter =
        new WritableSharedReference(new Counter());

    function __construct() { $this->counter->value++; }
}
"#,
        "E0548",
    );
}

#[test]
fn constructor_root_is_not_an_ordinary_writable_receiver() {
    assert_diagnostic(
        r#"
class Application
{
    writable function mutate(): void {}
    function __construct() { $this->mutate(); }
}
"#,
        "E0203",
    );
}

#[test]
fn property_move_out_and_self_move_remain_rejected() {
    assert_diagnostic(
        r#"
class Window {}
class Application
{
    writable Window $window = new Window();
    writable function replace(): void { $this->window = $this->window; }
}
"#,
        "E0471",
    );

    assert_diagnostic(
        r#"
class Window {}
class Application { Window $window = new Window(); }
function consume(take Window $window): void {}
function main(): void
{
    let $application = new Application();
    consume($application->window);
}
"#,
        "E0472",
    );
}

#[test]
fn constructor_write_kinds_preserve_conditional_initialization_and_replacement_order() {
    let source = r#"
class Token
{
    string $name;

    function __construct(string $label)
    {
        $this->name = $label;
    }

    function __destruct()
    {
        try { echo "drop {$this->name}\n"; }
        catch (Doria\Std\Io\IoError $error) {}
    }
}

class Holder
{
    writable Token $token;

    function __construct(bool $seeded)
    {
        if ($seeded) {
            $this->token = new Token("old");
        }
        $this->token = new Token("new");
    }
}

function main(): void throws Doria\Std\Io\IoError
{
    let $first = new Holder(true);
    let $second = new Holder(false);
    echo "ready\n";
}
"#;

    let program = lower(source);
    let constructor = program
        .functions
        .iter()
        .find(|function| function.name == "Holder::__construct")
        .expect("Holder constructor should lower");
    let kinds = constructor
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| match statement {
            mir::Statement::AssignProperty { kind, .. } => Some(*kind),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(kinds.contains(&mir::PropertyWriteKind::Initialize));
    assert!(kinds.contains(&mir::PropertyWriteKind::InitializeOrReplace));

    let output = doriac::mir_interpreter::interpret(&program).expect("MIR should interpret");
    assert_eq!(output.stdout, b"drop old\nready\ndrop new\ndrop new\n");
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
}

#[test]
fn property_transfer_consumes_the_source_and_borrowed_results_stay_rejected() {
    assert_diagnostic(
        r#"
class Window {}
function consume(take Window $window): void {}
class Application
{
    writable Window $window = new Window();
    writable function replace(take Window $replacement): void
    {
        $this->window = $replacement;
        consume($replacement);
    }
}
"#,
        "E0470",
    );

    assert_diagnostic(
        r#"
class Window {}
function inspect(Window $window): Window { return $window; }
class Application
{
    writable Window $window = new Window();
    writable function replace(Window $candidate): void
    {
        $this->window = inspect($candidate);
    }
}
"#,
        "E0478",
    );
}
