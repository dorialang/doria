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
fn repository_doria_sources_cover_checked_io_effects_and_contain_finalizers() {
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
        "crates/doriac/tests/fixtures/stage28a_pending",
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
                if matches!(diagnostic.code, "E0631" | "E0632") {
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
fn checked_errors_escape_finally_but_not_static_initialization() {
    let failure = error_class("Failure");
    doriac::check_source(
        "checked_error.doria",
        format!(
            r#"
{failure}
function f(): void throws Failure
{{
    try {{ echo "body"; }} finally {{ throw new Failure("finally"); }}
}}
"#
        ),
    )
    .expect("a checked Error may escape a finalizer through its callable contract");
    doriac::check_source(
        "checked_error.doria",
        r#"
function main(): void
{
    if (true) {} finally { echo "cleanup"; }
}
"#,
    )
    .expect("ambient I/O may propagate from a finalizer without source boilerplate");
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
fn selected_main_infers_exact_uncovered_effects_without_changing_source_syntax() {
    let source = format!(
        r#"
{}
{}
function failFirst(): void throws FirstError {{ throw new FirstError("first"); }}
function failSecond<T>(T $value): void throws SecondError {{ throw new SecondError("second"); }}
function main(): void
{{
    failFirst();
    try {{ failSecond(42); }} catch (SecondError) {{ echo "caught"; }}
    failSecond("again");
}}
"#,
        error_class("FirstError"),
        error_class("SecondError")
    );

    let parsed = doriac::parse_source("inferred_main.doria", &source)
        .expect("clause-free main should parse");
    let parsed_main = parsed.items.iter().find_map(|item| match item {
        doriac::ast::Item::Function(function) if function.name == "main" => Some(function),
        _ => None,
    });
    assert!(
        parsed_main.is_some_and(|function| function.throws.is_none()),
        "inference must not synthesize source syntax"
    );

    let hir = doriac::lower_source("inferred_main.doria", &source)
        .expect("clause-free main should lower");
    let main = hir.items.iter().find_map(|item| match item {
        hir::Item::Function(function) if function.name == "main" => Some(function),
        _ => None,
    });
    let main = main.expect("main should exist in HIR");
    assert!(main.throws.is_none(), "HIR must preserve source omission");
    let effect_names = main
        .checked_effects
        .iter()
        .map(|effect| match effect {
            doriac::types::ResolvedType::Class(class) => class.name.as_str(),
            other => panic!("expected concrete error class, got {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        effect_names,
        ["FirstError", "Doria\\Std\\Io\\IoError", "SecondError"]
    );

    let mir = doriac::lower_source_to_mir("inferred_main.doria", &source)
        .expect("inferred effects should lower to MIR");
    let main = &mir.functions[mir.entry.0];
    let mir_effect_names = main
        .checked_effects
        .iter()
        .map(|effect| match effect {
            doriac::mir::CheckedEffect::Concrete(id) => {
                mir.error_descriptors[id.0].type_name.as_str()
            }
            doriac::mir::CheckedEffect::Any => "Error",
        })
        .collect::<Vec<_>>();
    assert_eq!(
        mir_effect_names,
        ["FirstError", "Doria\\Std\\Io\\IoError", "SecondError"]
    );
}

#[test]
fn selected_main_inference_covers_direct_io_entry_shapes_and_construction() {
    fn main_effect_names(source: &str) -> Vec<String> {
        let hir = doriac::lower_source("inferred_main_surface.doria", source)
            .expect("accepted clause-free main should lower");
        hir.items
            .iter()
            .find_map(|item| match item {
                hir::Item::Function(function) if function.name == "main" => Some(
                    function
                        .checked_effects
                        .iter()
                        .map(|effect| match effect {
                            doriac::types::ResolvedType::Class(class) => class.name.clone(),
                            other => panic!("expected concrete error class, got {other:?}"),
                        })
                        .collect(),
                ),
                _ => None,
            })
            .expect("main should exist in HIR")
    }

    assert_eq!(
        main_effect_names(r#"function main(): void { echo "hello"; }"#),
        ["Doria\\Std\\Io\\IoError"]
    );
    assert_eq!(
        main_effect_names(
            r#"function main(List<string> $args): int { echo $args->count; return 0; }"#,
        ),
        ["Doria\\Std\\Io\\IoError"]
    );

    let failure = error_class("Failure");
    assert_eq!(
        main_effect_names(&format!(
            r#"
{failure}
function main(): void {{ throw new Failure("direct"); }}
"#
        )),
        ["Failure"]
    );
    assert_eq!(
        main_effect_names(&format!(
            r#"
{failure}
class Application
{{
    function __construct() throws Failure {{ throw new Failure("construct"); }}
}}
function main(): void {{ let $application = new Application(); }}
"#
        )),
        ["Failure"]
    );
}

#[test]
fn selected_main_inference_covers_entry_shapes_and_nested_catch_subtraction() {
    for source in [
        r#"
function main(): void
{
    try { echo "handled"; } catch (Doria\Std\Io\IoError) {}
}
"#,
        r#"
function main(List<string> $args): int
{
    try {
        if ($args->count > 0) { echo $args[0]; }
    } catch (Doria\Std\Io\IoError) {}
    return 0;
}
"#,
    ] {
        let hir = doriac::lower_source("handled_main.doria", source)
            .expect("accepted main shape should infer after nested handling");
        let main = hir.items.iter().find_map(|item| match item {
            hir::Item::Function(function) if function.name == "main" => Some(function),
            _ => None,
        });
        assert!(
            main.is_some_and(|function| function.checked_effects.is_empty()),
            "fully handled effects must not select the checked-result ABI"
        );
        let mir = doriac::lower_source_to_mir("handled_main.doria", source)
            .expect("nonthrowing inferred main should reach MIR");
        assert!(mir.functions[mir.entry.0].checked_effects.is_empty());
    }
}

#[test]
fn inferred_main_contract_is_available_to_recursive_and_source_calls() {
    let source = format!(
        r#"
{}
function caller(): void {{ main(); }}
function main(): void
{{
    try {{ main(); }} catch (Failure) {{}}
    throw new Failure("escape");
}}
"#,
        error_class("Failure")
    );
    let diagnostics = doriac::check_source("recursive_main.doria", source)
        .expect_err("an ordinary caller must declare main's inferred effect");
    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "E0631")
            .count(),
        1,
        "the inferred contract must be established before checking source callers"
    );
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "E0631" && diagnostic.message.contains("`caller`")
    }));
}

#[test]
fn inferred_main_diagnostics_remain_in_source_order() {
    let source = r#"
function earlier(): void
{
    int $value = "not an integer";
}
function main(): void
{
    int $value = "also not an integer";
}
"#;

    let diagnostics = doriac::check_source("inferred_main_order.doria", source)
        .expect_err("both invalid declarations should be diagnosed");
    let assignment_diagnostics = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "E0403")
        .collect::<Vec<_>>();

    assert_eq!(assignment_diagnostics.len(), 2);
    assert!(
        assignment_diagnostics[0].span.start < assignment_diagnostics[1].span.start,
        "the ordinary source-order pass must be the only pass that publishes diagnostics"
    );
}

#[test]
fn explicit_main_contract_remains_checked_and_ordinary_callables_remain_explicit() {
    let failure = error_class("Failure");
    doriac::check_source(
        "explicit_main.doria",
        format!(
            r#"
{failure}
function fail(): void throws Failure {{ throw new Failure("failure"); }}
function main(): void throws Failure {{ fail(); }}
"#
        ),
    )
    .expect("an explicit complete main contract remains valid");

    for source in [
        format!(
            r#"
{failure}
function fail(): void throws Failure {{ throw new Failure("failure"); }}
function main(): void throws Doria\Std\Io\IoError {{ fail(); }}
"#
        ),
        format!(
            r#"
{failure}
function helper(): void {{ throw new Failure("failure"); }}
function main(): void {{}}
"#
        ),
        format!(
            r#"
{failure}
class Worker {{ function main(): void {{ throw new Failure("failure"); }} }}
function main(): void {{}}
"#
        ),
    ] {
        assert_code(&source, "E0631");
    }
}

#[test]
fn inferred_main_diagnostics_are_not_duplicated_by_contract_discovery() {
    let diagnostics = doriac::check_source(
        "invalid_main.doria",
        "function main(): void { echo $missing; }",
    )
    .expect_err("invalid main should be rejected");
    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "E0101")
            .count(),
        1,
        "the inference pass must not duplicate user diagnostics"
    );
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

#[test]
fn ide_analysis_uses_the_compiler_known_io_pipeline_without_exposing_synthetic_items() {
    let source = r#"function main(): void throws Doria\Std\Io\IoError
{
    echo "ready";
}
"#;

    let (program, analysis) = doriac::analyze_source_for_ide("ide_io.doria", source)
        .expect("IDE analysis should parse compiler-known I/O types");

    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    assert_eq!(
        program.items.len(),
        1,
        "synthetic declarations stay compiler-owned"
    );
}

#[test]
fn canonical_io_is_ambient_in_source_but_retained_in_hir_and_mir() {
    let source = r#"
function writeMessage(string $message): void
{
    echo $message;
}

function forward(string $message): void
{
    writeMessage($message);
}

function main(): void
{
    forward("ambient");
}
"#;

    doriac::check_source("ambient_io.doria", source)
        .expect("canonical I/O must not create a source throws obligation");
    let hir = doriac::lower_source("ambient_io.doria", source)
        .expect("ambient I/O should retain a runtime profile in HIR");
    for name in ["writeMessage", "forward", "main"] {
        let function = hir.items.iter().find_map(|item| match item {
            hir::Item::Function(function) if function.name == name => Some(function),
            _ => None,
        });
        let function = function.unwrap_or_else(|| panic!("{name} should exist in HIR"));
        assert!(function.required_checked_effects.is_empty());
        assert_eq!(
            function
                .ambient_checked_effects
                .iter()
                .map(|effect| match effect {
                    doriac::types::ResolvedType::Class(class) => class.name.clone(),
                    other => panic!("expected concrete ambient Error, got {other:?}"),
                })
                .collect::<Vec<_>>(),
            ["Doria\\Std\\Io\\IoError"]
        );
        assert_eq!(function.checked_effects.len(), 1);
    }

    let mir = doriac::lower_source_to_mir("ambient_io.doria", source)
        .expect("ambient I/O should retain checked transport in MIR");
    for name in ["writeMessage", "forward", "main"] {
        let function = mir
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap_or_else(|| panic!("{name} should exist in MIR"));
        assert!(function.required_checked_effects.is_empty());
        assert_eq!(function.ambient_checked_effects.len(), 1);
        assert_eq!(function.checked_effects, function.ambient_checked_effects);
    }
}

#[test]
fn ambient_effect_fixpoint_preserves_catch_coverage_through_call_chains() {
    let source = r#"
function writeMessage(): void
{
    echo "handled";
}

function recover(): void
{
    try {
        writeMessage();
    } catch (Doria\Std\Io\IoError) {
    }
}

function forward(): void
{
    recover();
}

function main(): void
{
    forward();
}
"#;

    let hir = doriac::lower_source("ambient_catch_fixpoint.doria", source)
        .expect("a locally caught ambient effect must not escape its callable");
    let ambient_count = |name: &str| {
        hir.items
            .iter()
            .find_map(|item| match item {
                hir::Item::Function(function) if function.name == name => {
                    Some(function.ambient_checked_effects.len())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("{name} should exist in HIR"))
    };
    assert_eq!(ambient_count("writeMessage"), 1);
    assert_eq!(ambient_count("recover"), 0);
    assert_eq!(ambient_count("forward"), 0);
    assert_eq!(ambient_count("main"), 0);

    let unreachable = source.replace(
        "forward();\n}",
        "try { forward(); } catch (Doria\\Std\\Io\\IoError) {}\n}",
    );
    let diagnostics = doriac::check_source("ambient_catch_unreachable.doria", &unreachable)
        .expect_err("a caller cannot catch an ambient effect already handled by its callee");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0629"));
}

#[test]
fn ambient_property_initializers_use_the_implicit_constructor_transport() {
    let source = r#"
function initialValue(): string
{
    echo "init";
    return "ready";
}

class Report
{
    string $value = initialValue();
}

function main(): void
{
    let $report = new Report();
}
"#;

    let program = doriac::lower_source_to_mir("ambient_implicit_constructor.doria", source)
        .expect("ambient-only property initialization must not require a boilerplate constructor");
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("the implicit constructor must retain ambient checked transport");
    assert_eq!(output.stdout, b"init");
    assert_eq!(output.exit_status, 0);
}

#[test]
fn top_level_ambient_effects_do_not_create_source_obligations() {
    let source = r#"
echo "direct";

function writeMessage(string $message): void
{
    echo $message;
}

writeMessage("helper");

let $callback = function (): void {
    echo "closure";
};
$callback();
"#;

    doriac::check_source("top_level_ambient.doria", source)
        .expect("top-level ambient I/O remains runtime-checked without source boilerplate");
}

#[test]
fn top_level_nonambient_effects_still_require_handling() {
    let source = format!(
        r#"
{}

function fail(): void throws Failure
{{
    throw new Failure("required");
}}

fail();
"#,
        error_class("Failure")
    );

    let diagnostics = doriac::check_source("top_level_required.doria", source)
        .expect_err("top-level required errors still need an explicit handling boundary");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "E0630" && diagnostic.message.contains("`Failure`")
    }));
}

#[test]
fn only_exact_compiler_known_io_errors_are_ambient() {
    let source = format!(
        r#"
{}
function fail(): void {{ throw new IoError("user"); }}
function main(): void {{}}
"#,
        error_class("IoError")
    );
    let diagnostics = doriac::check_source("user_io_error.doria", source)
        .expect_err("a user class with the short IoError spelling is still required");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "E0631" && diagnostic.message.contains("`IoError`")
    }));

    doriac::check_source(
        "explicit_ambient.doria",
        r#"
function helper(): void throws Doria\Std\Io\IoError
{
    echo "explicit";
}
function main(): void { helper(); }
"#,
    )
    .expect("an explicitly authored ambient throws entry remains accepted");
}

#[test]
fn structural_callables_ignore_ambient_identity_but_keep_ambient_transport() {
    let source = r#"
function invoke(function(): void $callback): void
{
    $callback();
}

function main(): void
{
    function(): void throws Doria\Std\Io\IoError $callback = function (): void {
        echo "closure";
    };
    invoke($callback);
}
"#;
    doriac::check_source("ambient_closure.doria", source)
        .expect("ambient-only function types have the ordinary structural identity");
    let mir = doriac::lower_source_to_mir("ambient_closure.doria", source)
        .expect("ambient callable values need a checked runtime route");
    assert!(mir
        .function_types
        .iter()
        .all(doriac::mir::FunctionType::has_checked_transport));
    assert!(mir
        .function_types
        .iter()
        .all(|function| function.checked_effects.is_empty()));
    assert!(mir
        .function_types
        .iter()
        .all(|function| function.ambient_checked_effects.len() == 2));
}

#[test]
fn every_structural_invocation_mode_receives_ambient_transport_without_source_effects() {
    for (name, source) in [
        (
            "readonly",
            "function accept(function(): void $callback): void {}",
        ),
        (
            "writable",
            "function accept(function writable(): void $callback): void {}",
        ),
        (
            "once",
            "function accept(function once(): void $callback): void {}",
        ),
    ] {
        doriac::check_source(format!("{name}_function_type.doria"), source)
            .unwrap_or_else(|diagnostics| panic!("{name}: {diagnostics:#?}"));
    }
}

#[test]
fn fallible_finalizers_escape_same_try_catches_and_reach_outer_catches() {
    let source = format!(
        r#"
{}

function failCleanup(): void throws CleanupError
{{
    throw new CleanupError("cleanup");
}}

function main(): void
{{
    try {{
        try {{
            if (false) {{ failCleanup(); }}
            echo "body ";
        }} catch (CleanupError) {{
            echo "wrong ";
        }} finally {{
            failCleanup();
        }}
    }} catch (CleanupError $error) {{
        echo "outer {{$error->message}}";
    }}
}}
"#,
        error_class("CleanupError")
    );

    let program = doriac::lower_source_to_mir("fallible_finally.doria", &source)
        .expect("a finalizer Error may flow to an outer catch");
    let interpreted = doriac::mir_interpreter::interpret(&program)
        .expect("fallible finalizer source should execute");
    assert_eq!(interpreted.stdout, b"body outer cleanup");
    assert_eq!(interpreted.exit_status, 0);
}

#[test]
fn finalizer_error_replaces_pending_return_and_earlier_error() {
    let source = format!(
        r#"
{}
{}

function returnThenFail(): int throws FinalError
{{
    try {{
        return 42;
    }} finally {{
        throw new FinalError("return replaced");
    }}
}}

function failTwice(): void throws FirstError, FinalError
{{
    try {{
        throw new FirstError("first");
    }} finally {{
        throw new FinalError("error replaced");
    }}
}}

function main(): void
{{
    try {{ int $value = returnThenFail(); }} catch (FinalError $error) {{ echo "{{$error->message}} "; }}
    try {{ failTwice(); }} catch (FinalError $error) {{ echo $error->message; }}
}}
"#,
        error_class("FirstError"),
        error_class("FinalError")
    );

    let program = doriac::lower_source_to_mir("finalizer_precedence.doria", &source)
        .expect("finalizer replacement must be represented in MIR");
    let replacements = program
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.statements)
        .filter_map(|statement| match statement {
            doriac::mir::Statement::ControlFlowPlan(doriac::mir::ControlFlowPlan::Finalizer(
                plan,
            )) => Some(plan),
            _ => None,
        })
        .flat_map(|plan| &plan.replacements)
        .count();
    assert!(
        replacements >= 2,
        "both finalizers should carry replacement plans"
    );

    let interpreted = doriac::mir_interpreter::interpret(&program)
        .expect("finalizer replacement should execute through shared MIR");
    assert_eq!(interpreted.stdout, b"return replaced error replaced");
    assert_eq!(interpreted.exit_status, 0);
}

#[test]
fn every_canonical_io_builtin_is_ambient_at_the_source_boundary() {
    let source = r#"
class Device
{
    string $label = marker();

    function __construct()
    {
        printf("constructed");
    }
}

function marker(): string
{
    write_stderr("marker");
    return "device";
}

function textIo(string $path): void
{
    ?string $line = read_line();
    string $contents = read_file($path);
    write_file($path, $contents);
    append_file($path, $line ?? "");
}

function byteIo(string $path): void
{
    Bytes $stdin = read_stdin_bytes();
    Bytes $file = read_file_bytes($path);
    write_file_bytes($path, $stdin);
    append_file_bytes($path, $file);
    write_stdout_bytes($stdin);
    write_stderr_bytes($file);
}

function main(): void
{
    let $device = new Device();
}
"#;

    doriac::check_source("ambient_builtins.doria", source)
        .expect("canonical I/O must not require source throws clauses");
    let hir = doriac::lower_source("ambient_builtins.doria", source)
        .expect("canonical I/O profiles should lower to HIR");
    for name in ["marker", "textIo", "byteIo", "main"] {
        let function = hir.items.iter().find_map(|item| match item {
            hir::Item::Function(function) if function.name == name => Some(function),
            _ => None,
        });
        let function = function.unwrap_or_else(|| panic!("{name} should exist in HIR"));
        assert!(function.required_checked_effects.is_empty(), "{name}");
        assert!(!function.ambient_checked_effects.is_empty(), "{name}");
        assert_eq!(
            function.checked_effects, function.ambient_checked_effects,
            "{name}"
        );
    }
}

#[test]
fn ambient_catches_aliases_and_required_contracts_remain_exact() {
    doriac::check_source(
        "ambient_catches.doria",
        r#"
use Doria\Std\Io\IoError as OutputError;

function recover(): void
{
    try { echo "value"; } catch (OutputError) {}
    try { read_file("missing"); } catch (Error) {}
}

function main(): void { recover(); }
"#,
    )
    .expect("ambient effects must remain visible to exact and broad catches");

    let source = format!(
        r#"
{}

function fail(): void
{{
    echo "before";
    throw new StorageError("failed");
}}

function main(): void {{}}
"#,
        error_class("StorageError")
    );
    let diagnostics = doriac::check_source("required_plus_ambient.doria", source)
        .expect_err("the nonambient StorageError still requires a contract");
    let contract = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "E0631")
        .expect("missing required-effect diagnostic");
    assert!(contract.message.contains("StorageError"));
    assert!(!contract.message.contains("Doria\\Std\\Io\\IoError"));
    assert!(!contract.message.contains("InvalidUtf8Error"));

    doriac::check_source(
        "explicit_mixed_contract.doria",
        format!(
            r#"
{}
function fail(): void throws Doria\Std\Io\IoError, StorageError
{{
    echo "before";
    throw new StorageError("failed");
}}
function main(): void {{ try {{ fail(); }} catch (StorageError) {{}} }}
"#,
            error_class("StorageError")
        ),
    )
    .expect("explicit ambient entries and required entries may coexist");

    let broad = doriac::check_source(
        "broad_error.doria",
        r#"
function broad(): void throws Error {}
function caller(): void { broad(); }
function main(): void {}
"#,
    )
    .expect_err("the broad Error contract must never become ambient");
    assert!(broad.iter().any(|diagnostic| {
        diagnostic.code == "E0631" && diagnostic.message.contains("`Error`")
    }));
}

#[test]
fn ambient_io_does_not_relax_destructor_or_static_initializer_boundaries() {
    let destructor = doriac::check_source(
        "ambient_destructor.doria",
        r#"
class Device
{
    function __destruct() { echo "closing"; }
}
function main(): void {}
"#,
    )
    .expect_err("destructors must still absorb every checked runtime effect");
    assert!(destructor
        .iter()
        .any(|diagnostic| diagnostic.code == "E0631"));
    assert!(destructor
        .iter()
        .all(|diagnostic| diagnostic.code != "E0632"));

    let initializer = doriac::check_source(
        "ambient_static.doria",
        r#"
function marker(): string { echo "marker"; return "value"; }
class Device { static string $label = marker(); }
function main(): void {}
"#,
    )
    .expect_err("static initialization remains nonthrowing");
    assert!(initializer
        .iter()
        .any(|diagnostic| diagnostic.code == "E0634"));
}
