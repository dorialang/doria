use doriac::hir;
use std::fs;
use std::path::{Path, PathBuf};

fn error_class(name: &str) -> String {
    format!(
        r#"
class {name} implements Error
{{
    function __construct(string $message)
    {{
    }}
}}
"#
    )
}

fn diagnostics(source: &str) -> Vec<doriac::diagnostics::Diagnostic> {
    doriac::check_source("checked_error.doria", source)
        .expect_err("checked-error source should be rejected")
}

fn assert_code(source: &str, code: &str) {
    let diagnostics = diagnostics(source);
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic.code == code),
        "expected {code}, got {diagnostics:?}"
    );
}

fn collect_doria_sources(root: &Path, sources: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", root.display()))
    {
        let path = entry
            .expect("source directory entry should be readable")
            .path();
        if path.is_dir() {
            collect_doria_sources(&path, sources);
        } else if path
            .extension()
            .is_some_and(|extension| extension == "doria")
        {
            sources.push(path);
        }
    }
}

#[test]
fn repository_executable_sources_cover_checked_io_effects() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("doriac should live below the workspace");
    let roots = [
        "examples/compile-only",
        "examples/debug",
        "examples/native",
        "examples/php",
        "crates/doriac/tests/fixtures/native_io",
        "crates/doriac/tests/fixtures/native_stack",
    ];
    let mut sources = Vec::new();
    for root in roots {
        collect_doria_sources(&workspace.join(root), &mut sources);
    }
    sources.sort();

    let mut uncovered = Vec::new();
    for path in sources {
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        if let Err(diagnostics) = doriac::check_source(path.to_string_lossy(), source) {
            for diagnostic in diagnostics {
                if diagnostic.code == "E0631" {
                    uncovered.push(format!("{}: {}", path.display(), diagnostic.message));
                }
            }
        }
    }

    assert!(
        uncovered.is_empty(),
        "executable sources have uncovered checked effects:\n{}",
        uncovered.join("\n")
    );
}

#[test]
fn error_conformance_accepts_both_property_forms_and_rejects_invalid_contracts() {
    doriac::check_source(
        "checked_error.doria",
        r#"
class PromotedError implements Error
{
    function __construct(string $message)
    {
    }
}

class ExplicitError<T> implements Error
{
    string $message = "explicit";
    int $code = 7;
}
"#,
    )
    .expect("valid Error contracts should pass");

    for (member, code) in [
        ("int $code = 1;", "E0613"),
        ("internal string $message = \"x\";", "E0616"),
        ("writable string $message = \"x\";", "E0615"),
        ("int $message = 1;", "E0614"),
    ] {
        assert_code(
            &format!("class InvalidError implements Error {{ {member} }}"),
            code,
        );
    }
}

#[test]
fn error_is_available_in_every_settled_type_position() {
    let source = format!(
        r#"
{}

class Holder
{{
    Error $error = new Failure("property");
}}

function accept(Error $error): void
{{
    let $message = $error->message;
}}

function same(Error $left, Error $right): bool
{{
    return $left == $right;
}}

function create(): Error
{{
    return new Failure("return");
}}

Error $local = new Failure("local");
?Error $optional = null;
List<Error> $list = [new Failure("list")];
Dictionary<string, Error> $map = ["failure" => new Failure("map")];
mixed $boundary = new Failure("mixed");
accept(new Failure("parameter"));
"#,
        error_class("Failure")
    );
    doriac::check_source("checked_error.doria", source)
        .expect("Error should resolve as the compiler-known interface type");
}

#[test]
fn throws_entries_are_error_types_unique_and_source_ordered() {
    let source = format!(
        r#"
{}
{}

function load(bool $first): void throws FirstError, SecondError
{{
    if ($first) {{
        throw new FirstError("first");
    }}
    throw new SecondError("second");
}}
"#,
        error_class("FirstError"),
        error_class("SecondError")
    );
    let program = doriac::lower_source("checked_error.doria", source)
        .expect("declared effects should cover direct throws");
    let hir::Item::Function(function) = &program.items[2] else {
        panic!("expected function HIR");
    };
    let effects = &function.throws.as_ref().unwrap().entries;
    assert_eq!(effects.len(), 2);
    assert!(matches!(
        &effects[0].resolved,
        doriac::types::ResolvedType::Class(class) if class.name == "FirstError"
    ));
    assert!(matches!(
        &effects[1].resolved,
        doriac::types::ResolvedType::Class(class) if class.name == "SecondError"
    ));

    let first = error_class("FirstError");
    for (source, code, title) in [
        (
            format!("{first} function f(): void throws FirstError, FirstError {{}}"),
            "E0620",
            "Remove Duplicate Throws Entry",
        ),
        (
            format!("{first} function f(): void throws Error, FirstError {{}}"),
            "E0621",
            "Remove Redundant Throws Entry",
        ),
    ] {
        let diagnostics = diagnostics(&source);
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == code)
            .unwrap_or_else(|| panic!("expected {code}, got {diagnostics:?}"));
        let fix = diagnostic
            .fixes
            .first()
            .unwrap_or_else(|| panic!("{code} should carry a structured fix"));
        assert_eq!(fix.title, title);
        assert_eq!(
            fix.applicability,
            doriac::diagnostics::FixApplicability::MachineApplicable
        );
        let mut fixed = source.clone();
        fixed.replace_range(fix.edits[0].span.start..fix.edits[0].span.end, "");
        doriac::check_source("checked_error.doria", fixed).unwrap_or_else(|diagnostics| {
            panic!("{code} fix must produce valid source: {diagnostics:?}")
        });
    }

    let commented =
        format!("{first} function f(): void throws FirstError, /* keep context */ FirstError {{}}");
    let commented_diagnostics = diagnostics(&commented);
    let commented_fix = &commented_diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "E0620")
        .expect("commented duplicate")
        .fixes[0];
    assert_eq!(
        commented_fix.applicability,
        doriac::diagnostics::FixApplicability::RequiresReview,
        "automatic cleanup must not silently discard comments"
    );
    assert_code("function f(): void throws int {}", "E0618");
    assert_code("function f(): void throws ?Error {}", "E0619");
    assert_code("function f(): void throws mixed {}", "E0618");
    assert_code("function f(): void throws List<int> {}", "E0618");
    assert_code("class Value {} function f(): void throws Value {}", "E0618");
    assert_code("function f() throws Error {}", "E0617");
    assert_code(
        &format!("{first} class C {{ function __destruct(): void throws FirstError {{}} }}"),
        "E0622",
    );
}

#[test]
fn direct_throw_requires_an_owned_explicit_error_value() {
    let failure = error_class("Failure");
    doriac::check_source(
        "checked_error.doria",
        format!(
            r#"
{failure}
function relay(take Failure $failure): void throws Failure
{{
    throw $failure;
}}
"#
        ),
    )
    .expect("an owned Error value may be thrown");

    assert_code(
        &format!(
            r#"
{failure}
function relay(Failure $failure): void throws Failure
{{
    throw $failure;
}}
"#
        ),
        "E0474",
    );
    assert_code(
        "class NotAnError {} function f(): void { throw new NotAnError(); }",
        "E0623",
    );
    assert_code(
        "class LooksLikeError { string $message = \"x\"; } function f(): void { throw new LooksLikeError(); }",
        "E0623",
    );
    let borrowed = diagnostics(&format!(
        "{failure} function relay(Failure $failure): void throws Failure {{ throw $failure; }}"
    ));
    assert!(borrowed
        .iter()
        .any(|diagnostic| diagnostic.title == "Throw Requires Ownership"));
    assert_code(
        &format!(
            r#"
{failure}
function consume(take Failure $failure): void
{{
    try {{
        throw $failure;
    }} catch (Failure) {{}}
    let $message = $failure->message;
}}
"#
        ),
        "E0470",
    );
}

#[test]
fn catches_subtract_only_protected_effects_and_catch_bodies_are_independent() {
    let source = format!(
        r#"
{}
{}

function first(): void throws FirstError
{{
    throw new FirstError("first");
}}

function handled(): void
{{
    try {{
        first();
    }} catch (FirstError $error) {{
        let $message = $error->message;
    }}
}}

function open(): void throws Error
{{
    try {{
        first();
    }} catch (Error) {{
        let $handled = true;
    }}
}}
"#,
        error_class("FirstError"),
        error_class("SecondError")
    );
    doriac::check_source("checked_error.doria", source)
        .expect("exact and Error catch-all coverage should pass");

    let first = error_class("FirstError");
    let second = error_class("SecondError");
    assert_code(
        &format!("{first} function f(): void {{ try {{}} catch (FirstError) {{}} }}"),
        "E0629",
    );
    assert_code(
        &format!(
            "{first} function f(): void {{ try {{ throw new FirstError(\"x\"); }} catch (Error) {{}} catch (FirstError) {{}} }}"
        ),
        "E0628",
    );
    assert_code(
        &format!(
            "{first}{second} function f(): void throws SecondError {{ try {{ throw new FirstError(\"x\"); }} catch (FirstError) {{ throw new SecondError(\"y\"); }} catch (SecondError) {{}} }}"
        ),
        "E0629",
    );
}

#[test]
fn partial_coverage_propagates_the_complete_uncovered_set() {
    let source = format!(
        r#"
{}
{}

function load(bool $first): void throws FirstError, SecondError
{{
    if ($first) {{
        throw new FirstError("first");
    }}
    throw new SecondError("second");
}}

function caller(): void throws SecondError
{{
    try {{
        load(true);
    }} catch (FirstError) {{
        let $handled = true;
    }}
}}
"#,
        error_class("FirstError"),
        error_class("SecondError")
    );
    let program = doriac::lower_source("checked_error.doria", source)
        .expect("the remaining exact effect is declared by the caller");
    let hir::Item::Function(caller) = &program.items[3] else {
        panic!("expected caller function");
    };
    let hir::Stmt::Try(try_statement) = &caller.body.statements[0] else {
        panic!("expected try HIR");
    };
    assert!(matches!(
        try_statement.uncovered_effects.as_slice(),
        [doriac::types::ResolvedType::Class(class)] if class.name == "SecondError"
    ));
}

#[test]
fn uncovered_effect_diagnostics_report_the_complete_set_once() {
    let source = format!(
        r#"
{}
{}
function load(): void throws FirstError, SecondError
{{
    throw new FirstError("first");
}}
function caller(): void
{{
    load();
}}
"#,
        error_class("FirstError"),
        error_class("SecondError")
    );
    let diagnostics = diagnostics(&source);
    let uncovered = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "E0631")
        .collect::<Vec<_>>();
    assert_eq!(uncovered.len(), 1);
    assert!(uncovered[0].message.contains("FirstError"));
    assert!(uncovered[0].message.contains("SecondError"));

    let broad_contract = format!(
        r#"
{}
function concrete(): void throws FirstError
{{
    throw new FirstError("first");
}}
function broad(): void throws Error
{{
    concrete();
}}
"#,
        error_class("FirstError")
    );
    doriac::check_source("checked_error.doria", broad_contract)
        .expect("the broad Error effect must cover each concrete checked error");
}

#[test]
fn checked_effects_propagate_through_every_callable_form() {
    let source = format!(
        r#"
{}

class Service
{{
    function __construct() throws Failure
    {{
        throw new Failure("construct");
    }}

    function load(): void throws Failure
    {{
        throw new Failure("method");
    }}

    static function open(): void throws Failure
    {{
        throw new Failure("static");
    }}
}}

function run(): void throws Failure
{{
    let $service = new Service();
    $service->load();
    Service::open();
}}
"#,
        error_class("Failure")
    );
    doriac::check_source("checked_error.doria", source)
        .expect("constructor, method, and static effects should share call propagation");
}

#[test]
fn construction_effects_cannot_hide_in_initialization() {
    let valid = format!(
        r#"
{}

function value(): string throws Failure
{{
    throw new Failure("value");
}}

class Explicit
{{
    string $value = value();

    function __construct() throws Failure
    {{
    }}
}}

function create(): Explicit throws Failure
{{
    return new Explicit();
}}
"#,
        error_class("Failure")
    );
    doriac::check_source("checked_error.doria", valid)
        .expect("explicit constructor effects should include property initialization");

    let invalid = format!(
        r#"
{}
function value(): string throws Failure {{ throw new Failure("value"); }}
class Implicit {{ string $value = value(); }}
"#,
        error_class("Failure")
    );
    assert_code(&invalid, "E0635");
}

#[test]
fn checked_errors_cannot_escape_finally_or_static_initialization() {
    let failure = error_class("Failure");
    assert_code(
        &format!(
            r#"
{failure}
function f(): void throws Failure
{{
    try {{ echo "body"; }} finally {{ throw new Failure("finally"); }}
}}
"#
        ),
        "E0632",
    );
    assert_code(
        &format!(
            r#"
{failure}
function value(): string throws Failure {{ throw new Failure("value"); }}
const string VALUE = value();
"#
        ),
        "E0633",
    );
    assert_code(
        &format!(
            r#"
{failure}
function value(): string throws Failure {{ throw new Failure("value"); }}
class C {{ static string $value = value(); }}
"#
        ),
        "E0634",
    );
}

#[test]
fn checked_error_scopes_and_optional_bindings_follow_lexical_blocks() {
    let failure = error_class("Failure");
    doriac::check_source(
        "checked_error.doria",
        format!(
            r#"
{failure}
function work(): void throws Failure {{ throw new Failure("work"); }}
function handled(): void
{{
    try {{ work(); }} catch (Failure) {{ let $handled = true; }}
    try {{ work(); }} catch (Failure $failure) {{ let $message = $failure->message; }}
}}
"#
        ),
    )
    .expect("bound and omitted catches should both be valid");

    assert_code(
        &format!(
            r#"
{failure}
function work(): void throws Failure {{ throw new Failure("work"); }}
function invalid(): void
{{
    try {{ let $temporary = "try"; work(); }} catch (Failure $caught) {{ let $outside = $temporary; }} finally {{ let $message = $caught->message; }}
}}
"#
        ),
        "E0101",
    );
}

#[test]
fn destructors_must_handle_effects_locally_and_nested_finally_handling_is_valid() {
    let source = format!(
        r#"
{}
function cleanup(): void throws CleanupError {{ throw new CleanupError("cleanup"); }}
class Resource
{{
    function __destruct(): void
    {{
        try {{ cleanup(); }} catch (CleanupError) {{ let $recorded = true; }}
    }}
}}
function complete(): void
{{
    try {{ let $body = true; }} finally {{
        try {{ cleanup(); }} catch (CleanupError) {{ let $recorded = true; }}
    }}
}}
"#,
        error_class("CleanupError")
    );
    doriac::check_source("checked_error.doria", source)
        .expect("destructors and finalizers may handle checked errors locally");
}

#[test]
fn generic_specializations_preserve_declared_checked_effects() {
    let source = format!(
        r#"
{}
function failWith<T>(T $value): T throws Failure
{{
    throw new Failure("generic");
}}
function caller(): int throws Failure
{{
    return failWith(42);
}}
"#,
        error_class("Failure")
    );
    doriac::check_source("checked_error.doria", source)
        .expect("generic specialization should preserve the callable effect set");
}

#[test]
fn constructors_and_property_initializers_share_one_declared_effect_contract() {
    let source = format!(
        r#"
{}
function label(): string throws Failure {{ throw new Failure("label"); }}
class Box
{{
    string $label = label();
    function __construct() throws Failure {{ throw new Failure("construct"); }}
}}
function create(): Box throws Failure {{ return new Box(); }}
"#,
        error_class("Failure")
    );
    doriac::check_source("checked_error.doria", source)
        .expect("construction should expose property and body effects through one contract");
}

#[test]
fn ownership_catches_join_the_state_at_each_matching_throw_site() {
    let source = format!(
        r#"
{}
class Payload {{}}
function consume(take Payload $payload): void {{}}
function fail(): void throws Failure {{ throw new Failure("failure"); }}
function inspect(Payload $payload): void {{}}
function invalid(): void
{{
    let writable $payload = new Payload();
    try {{
        consume($payload);
        fail();
        $payload = new Payload();
    }} catch (Failure) {{}}
    inspect($payload);
}}
"#,
        error_class("Failure")
    );
    assert_code(&source, "E0470");
}

#[test]
fn constructor_catches_observe_completed_writes_before_throw() {
    let source = format!(
        r#"
{}
function fail(): void throws Failure {{ throw new Failure("failure"); }}
class Ready
{{
    string $value;
    function __construct()
    {{
        try {{
            $this->value = "ready";
            fail();
        }} catch (Failure) {{}}
    }}
}}
"#,
        error_class("Failure")
    );
    doriac::check_source("checked_error.doria", source)
        .expect("a caught throw must preserve constructor writes completed before it");
}

#[test]
fn constructor_catches_do_not_observe_writes_after_a_throwing_rhs() {
    let source = format!(
        r#"
{}
function load(): string throws Failure {{ throw new Failure("failure"); }}
class NotReady
{{
    string $value;
    function __construct()
    {{
        try {{
            $this->value = load();
        }} catch (Failure) {{}}
    }}
}}
"#,
        error_class("Failure")
    );
    assert_code(&source, "E0500");
}

#[test]
fn concrete_catches_receive_only_matching_exceptional_states() {
    let source = format!(
        r#"
{}
{}
class Payload {{}}
function failFirst(): void throws FirstError {{ throw new FirstError("first"); }}
function failSecond(): void throws SecondError {{ throw new SecondError("second"); }}
function consume(take Payload $payload): void {{}}
function inspect(Payload $payload): void {{}}
function valid(bool $first): void throws SecondError
{{
    let writable $payload = new Payload();
    try {{
        if ($first) {{
            failFirst();
        }}
        consume($payload);
        failSecond();
    }} catch (FirstError) {{
        inspect($payload);
    }}
}}
"#,
        error_class("FirstError"),
        error_class("SecondError")
    );
    doriac::check_source("checked_error.doria", source)
        .expect("a concrete catch must not receive a different error type's ownership state");
}

#[test]
fn stage29_slice3_executes_handled_and_escaping_main_errors() {
    let source = format!(
        r#"
{}
function fail(): void throws Failure {{ throw new Failure("x"); }}
function main(): void throws Doria\Std\Io\IoError
{{
    try {{ fail(); }} catch (Failure) {{ echo "handled"; }}
}}
"#,
        error_class("Failure")
    );
    doriac::check_source("checked_error.doria", &source).expect("semantic checking should succeed");
    doriac::parse_source("checked_error.doria", &source).expect("AST should succeed");
    doriac::lower_source("checked_error.doria", &source).expect("HIR should succeed");
    doriac::lower_source_to_mir("checked_error.doria", &source)
        .expect("handled checked errors should reach MIR");

    for target in [
        doriac::backend::BackendTarget::Debug,
        doriac::backend::BackendTarget::Native,
        doriac::backend::BackendTarget::Php,
    ] {
        doriac::compile_source("checked_error.doria", &source, target)
            .expect("handled checked errors should reach every executable backend");
    }

    let escaping = format!(
        r#"
{}
function fail(): void throws Failure {{ throw new Failure("x"); }}
function main(): void throws Failure {{ fail(); }}
"#,
        error_class("Failure")
    );
    doriac::lower_source_to_mir("escaping_error.doria", &escaping)
        .expect("escaping-main reporting is a backend boundary, not a MIR boundary");
    for target in [
        doriac::backend::BackendTarget::Debug,
        doriac::backend::BackendTarget::Native,
        doriac::backend::BackendTarget::Php,
    ] {
        doriac::compile_source("escaping_error.doria", &escaping, target)
            .expect("an Error escaping main should be handled by the runtime boundary");
    }
    let program = doriac::lower_source_to_mir("escaping_error.doria", &escaping)
        .expect("escaping Error source should lower to MIR");
    let interpreted = doriac::mir_interpreter::interpret(&program)
        .expect("interpreter should report an escaping Error");
    assert_eq!(interpreted.exit_status, 70);
    assert_eq!(
        interpreted
            .runtime_diagnostic
            .as_ref()
            .map(|diagnostic| diagnostic.code),
        Some("R1000")
    );

    doriac::compile_source(
        "plain.doria",
        "function main(): void throws Doria\\Std\\Io\\IoError { echo \"ok\"; }",
        doriac::backend::BackendTarget::Debug,
    )
    .expect("nonthrowing programs must remain executable");

    doriac::compile_source(
        "unused_error.doria",
        format!(
            "{} function main(): void throws Doria\\Std\\Io\\IoError {{ echo \"ok\"; }}",
            error_class("UnusedError")
        ),
        doriac::backend::BackendTarget::Debug,
    )
    .expect("an unused concrete Error declaration must not activate the execution boundary");
}

#[test]
fn compiler_known_io_types_are_nominal_and_expose_typed_fields() {
    let source = r#"
function inspect(Error $error): string
{
    return $error->message;
}

function main(): int
{
    let $operation = Doria\Std\Io\IoOperation::Read;
    let $target = Doria\Std\Io\IoTarget::File(path: "data.txt");
    let $reason = Doria\Std\Io\IoErrorReason::NotFound;
    let $error = new Doria\Std\Io\IoError(
        message: "failed to read file `data.txt`: not found",
        operation: $operation,
        target: $target,
        reason: $reason,
    );
    let $message = inspect($error);

    let $source = Doria\Std\Io\Utf8InputSource::StandardInput;
    let $encoding = new Doria\Std\Io\InvalidUtf8Error(
        message: "invalid UTF-8 in standard input",
        source: $source,
        validByteCount: 3,
    );
    mixed $boxed = $encoding;

    if (!($boxed is Doria\Std\Io\InvalidUtf8Error)) {
        return 1;
    }
    if ($message != $error->message) {
        return 2;
    }
    return match ($error->operation) {
        Doria\Std\Io\IoOperation::Read => 0,
        default => 3,
    };
}
"#;

    doriac::check_source("canonical_io.doria", source)
        .expect("canonical I/O types should use the ordinary nominal type model");
    doriac::lower_source_to_mir("canonical_io.doria", source)
        .expect("canonical I/O types should lower through ordinary MIR");
    for target in [
        doriac::backend::BackendTarget::Debug,
        doriac::backend::BackendTarget::Native,
        doriac::backend::BackendTarget::Php,
    ] {
        doriac::compile_source("canonical_io.doria", source, target)
            .expect("canonical I/O types should compile for every backend");
    }

    let diagnostics = doriac::check_source(
        "short_alias.doria",
        "function inspect(IoError $error): void {}",
    )
    .expect_err("unqualified temporary I/O aliases must not enter the prelude");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message.contains("unknown type `IoError`")));
}
