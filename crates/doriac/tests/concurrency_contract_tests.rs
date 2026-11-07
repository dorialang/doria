#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution.rs"]
mod native_execution;
#[path = "common/native_structure.rs"]
mod native_structure;
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "common/unix_failure_injection.rs"]
mod unix_failure_injection;

use doriac::backend::{BackendTarget, NativeProfile};
use doriac::compiler_known_concurrency::CANCELLATION_ERROR;
use doriac::names::{CompilerKnownProvenance, CompilerSymbolIdentity, GlobalSymbolOwner};

const SOURCE: &str = r#"
use Doria\Std\Concurrency\CancellationError as CancellationFailure;

function failCancellation(?int $code = null): void throws CancellationFailure {
    throw new CancellationFailure("notification failed", systemCode: $code);
}
function forwardCancellation(): void throws CancellationFailure {
    try { failCancellation(0); }
    catch (CancellationFailure $error) { throw $error; }
}
function describeError(Error $error): string { return $error->message; }
function main(): void {
    try { forwardCancellation(); }
    catch (CancellationFailure $error) {
        echo describeError($error) . ": " . ($error->systemCode ?? -1) . "\n";
    }
    try { failCancellation(); }
    catch (CancellationFailure $error) {
        if ($error->systemCode == null) { echo "no system code\n"; }
    }
}
"#;

fn assert_cancellation_execution(source: &str, expected: &str) {
    let hir = doriac::lower_source("cancellation-source.doria", source).unwrap();
    let program = doriac::mir_lowering::lower_program(&hir).unwrap();
    doriac::mir_validation::validate_program(&program)
        .unwrap_or_else(|error| panic!("{error:?}\n{program}"));
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(output.stdout, expected.as_bytes());
    assert_eq!(output.exit_status, 0);
    assert!(output.stderr.is_empty());
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    let artifact =
        doriac::compile_source("cancellation-source.doria", source, BackendTarget::Php).unwrap();
    assert!(matches!(
        &artifact,
        doriac::backend::BackendOutput::TextWithRuntime { .. }
    ));
    let directory = native_execution::ExecutionDirectory::new();
    let path = directory.path().join("main.php");
    doriac::backend::publish_output(&path, artifact).unwrap();
    let output = std::process::Command::new("php")
        .args(["-d", "ffi.enable=1"])
        .arg(path)
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, expected.as_bytes());
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[test]
fn source_properties_cancel_and_outliving_tokens_execute_across_backends() {
    assert_cancellation_execution(
        r#"
use Doria\Std\Concurrency\CancellationSource;
use Doria\Std\Concurrency\CancellationToken;
use Doria\Std\Concurrency\CancellationError;
function survivingToken(): CancellationToken {
    let $source = new CancellationSource();
    return $source->token;
}
function inspect(CancellationToken $token): void {}
function consume(take CancellationToken $token): void {}
function main(): void {
    let writable $source = new CancellationSource();
    echo "before: {$source->cancellationRequested}\n";
    let $first, $second = $source->token;
    consume($first);
    inspect($first); inspect($second);
    try { $source->cancel(); $source->cancel(); }
    catch (CancellationError $error) { echo $error->message . "\n"; }
    echo "after: {$source->cancellationRequested}\n";
    let $survivor = survivingToken();
    let $copy = $survivor;
    consume($copy); inspect($copy); inspect($survivor);
    echo "survived\n";
}
"#,
        "before: false\nafter: true\nsurvived\n",
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn native_notification_failure_preserves_owned_error_requested_state_and_group_reuse() {
    let source = r#"
use Doria\Std\Concurrency\{CancellationSource, CancellationError};
use Doria\Std\Io\{ReadinessSet, TransferPlan, File};
function main(): void {
    write_file("input", "abc");
    let writable $set = new ReadinessSet();
    let $registration = $set->register(File::openRead("input"));
    writable Bytes $buffer = Bytes::allocate(1);
    let writable $source = new CancellationSource();
    let $token = $source->token;
    {
        let $plan = TransferPlan::read($registration, $buffer->getWritableRegion(0, 1));
        let $group = $set->begin($plan, cancellation: $token);
        try { $source->cancel(); echo "notified\n"; }
        catch (CancellationError $error) {
            if (($error->systemCode ?? -1) != 5) { panic("notification code lost"); }
            echo $error->message . "\n";
        }
        if (!$source->cancellationRequested || $token != $source->token) {
            panic("notification failure reset cancellation or token identity");
        }
        $source->cancel();
    }
    {
        let writable $fresh = new CancellationSource();
        let $plan = TransferPlan::read($registration, $buffer->getWritableRegion(0, 1));
        let $group = $set->begin($plan, cancellation: $fresh->token);
        $fresh->cancel();
        if (!$fresh->cancellationRequested) { panic("reused group lost cancellation"); }
    }
    let $file = $set->unregister($registration);
    $file->close();
    echo "requested state and group reuse preserved\n";
}
"#;
    unix_failure_injection::assert_execution(
        source,
        "cancellation",
        "notified\nrequested state and group reuse preserved\n",
        &format!(
            "{}\nrequested state and group reuse preserved\n",
            doriac::compiler_known_concurrency::CANCELLATION_NOTIFICATION_FAILURE_MESSAGE
        ),
    );
}

#[test]
fn token_equality_compares_stable_shared_state_across_backends() {
    assert_cancellation_execution(
        r#"
use Doria\Std\Concurrency\{CancellationSource, CancellationToken, CancellationError};
enum Held { case Token(?CancellationToken $token); }
function equal(CancellationToken $first, CancellationToken $second): bool {
    return $first == $second;
}
function survivingTokens(): List<CancellationToken> {
    let $source = new CancellationSource();
    return [$source->token, $source->token];
}
function main(): void {
    let writable $source = new CancellationSource();
    let $first = $source->token;
    let $second = $source->token;
    let $copy = $first;
    let $other = new CancellationSource();
    let $different = $other->token;
    echo "same: {equal($first, $second)}, copy: {$first == $copy}\n";
    echo "different: {$first != $different}, reverse: {$different != $first}\n";
    try { $source->cancel(); }
    catch (CancellationError $error) { echo $error->message; }
    echo "cancelled: {$source->cancellationRequested}, stable: {equal($first, $second)}\n";
    ?CancellationToken $present = $first;
    ?CancellationToken $absent = null;
    echo "nullable: {$present == $second}, absent: {$absent != $present}\n";
    echo "wrapped: {Held::Token($first) == Held::Token($second)}\n";
    let $survivors = survivingTokens();
    echo "surviving: {equal($survivors[0], $survivors[1])}\n";
}
"#,
        "same: true, copy: true\ndifferent: true, reverse: true\ncancelled: true, stable: true\nnullable: true, absent: true\nwrapped: true\nsurviving: true\n",
    );
}

#[test]
fn borrowed_token_equality_acquires_no_claim_or_native_resource() {
    let program = doriac::lower_source_to_mir(
        "token-equality-cost.doria",
        r#"
use Doria\Std\Concurrency\CancellationToken;
function equal(CancellationToken $first, CancellationToken $second): bool { return $first == $second; }
function different(CancellationToken $first, CancellationToken $second): bool { return $first != $second; }
function optional(?CancellationToken $first, ?CancellationToken $second): bool { return $first == $second; }
function main(): void {}
"#,
    )
    .unwrap();
    let roots = native_structure::authored_symbols(&program, &["equal", "different", "optional"]);
    let fast = native_structure::cranelift_references(&program);
    let references = native_structure::reachable_references(&fast, roots.clone());
    assert!(
        references
            .iter()
            .all(|symbol| !symbol.contains("cancellation")
                && !symbol.contains("clock")
                && !symbol.contains("class_alloc")
                && !symbol.contains("class_free")),
        "{references:?}"
    );
    #[cfg(feature = "llvm-backend")]
    {
        let (_, release) = native_structure::llvm_references(&program);
        let references = native_structure::reachable_references(&release, roots);
        assert!(
            references
                .iter()
                .all(|symbol| !symbol.contains("cancellation")
                    && !symbol.contains("clock")
                    && !symbol.contains("class_alloc")
                    && !symbol.contains("class_free")),
            "{references:?}"
        );
    }
}

#[test]
fn php_token_equality_ignores_distinct_carriers_for_the_same_native_state() {
    let source = r#"
use Doria\Std\Concurrency\CancellationSource;
function main(): void { let $source = new CancellationSource(); let $token = $source->token; }
"#;
    let mut artifact =
        doriac::compile_source("token-carrier-equality.doria", source, BackendTarget::Php).unwrap();
    let doriac::backend::BackendOutput::TextWithRuntime { contents, .. } = &mut artifact else {
        panic!("token execution requires the shared runtime");
    };
    let token = doriac::compiler_known_concurrency::CANCELLATION_TOKEN
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    contents.push_str(&format!(
        r#"
$type = '__DoriaQualified_{token}';
$runtime = __DoriaNativeIo::get();
$state = $runtime->dr_v1_cancellation_new();
$first = $type::__doriaFromState($state);
$second = $type::__doriaFromState($runtime->dr_v1_cancellation_retain($state));
$different = $type::__doriaFromState($runtime->dr_v1_cancellation_new());
try {{
    if ($first === $second || $first->__doriaCancellationState() === $second->__doriaCancellationState()) {{ exit(1); }}
    if (!__doria_equal($first, $second) || !__doria_equal($second, $first)) {{ exit(2); }}
    if (__doria_equal($first, $different) || __doria_equal($different, $second)) {{ exit(3); }}
    if (__doria_equal($first, null) || __doria_equal(null, $first)) {{ exit(4); }}
    echo "carrier-independent state identity\n";
}} finally {{
    $first->__doriaRelease(); $second->__doriaRelease(); $different->__doriaRelease();
}}
"#
    ));
    let directory = native_execution::ExecutionDirectory::new();
    let path = directory.path().join("main.php");
    doriac::backend::publish_output(&path, artifact).unwrap();
    let output = std::process::Command::new("php")
        .args(["-d", "ffi.enable=1"])
        .arg(path)
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, b"carrier-independent state identity\n");
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[test]
fn source_and_token_contracts_reject_forgery_writes_and_wrong_effects() {
    for declaration in [
        "class CancellationSource {}",
        "class CancellationToken {}",
        "interface CancellationToken {}",
        "trait CancellationToken {}",
        "enum CancellationToken { case Value; }",
    ] {
        let source = format!("namespace Doria\\Std\\Concurrency; {declaration}");
        let diagnostics = doriac::check_source("reserved-cancellation.doria", source).unwrap_err();
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0640"),
            "{diagnostics:?}"
        );
    }
    for body in [
        "let $token = new CancellationToken();",
        "let $source = new CancellationSource(); $source->cancel();",
        "let writable $source = new CancellationSource(); $source->token = $source->token;",
        "let writable $source = new CancellationSource(); $source->cancellationRequested = false;",
        "let writable $source = new CancellationSource(); $source->getToken();",
        "let writable $source = new CancellationSource(); $source->requestCancellation();",
        "let $source = new CancellationSource(); let $token = $source->token; echo $token == $source;",
        "let $source = new CancellationSource(); let $token = $source->token; echo $token < $token;",
        "let $source = new CancellationSource(); let $token = $source->token; echo $token == 0;",
    ] {
        let source = format!("use Doria\\Std\\Concurrency\\CancellationSource; use Doria\\Std\\Concurrency\\CancellationToken; function main(): void {{ {body} }}");
        assert!(
            doriac::check_source("invalid-cancellation.doria", source).is_err(),
            "{body}"
        );
    }
    let wrong_effect = "use Doria\\Std\\Concurrency\\CancellationSource; function cancel(writable CancellationSource $source): void throws Doria\\Std\\Io\\IoError { $source->cancel(); }";
    assert!(doriac::check_source("wrong-cancellation-effect.doria", wrong_effect).is_err());
    let user = "class CancellationToken {} function consume(take CancellationToken $token): void {} function main(): void { let $token = new CancellationToken(); consume($token); consume($token); }";
    assert!(doriac::check_source("ordinary-token-move.doria", user).is_err());
}

#[test]
fn token_claims_survive_nested_copies_captures_and_exceptional_cleanup() {
    assert_cancellation_execution(
        r#"
use Doria\Std\Concurrency\{CancellationSource, CancellationToken, CancellationError};
enum Held { case Token(?CancellationToken $token); }
class Holder { function __construct(CancellationToken $token) {} }
class TokenKey implements Hashable, Equatable<TokenKey> {
    function __construct(int $value) {}
    function hash(): uint64 { return 0; }
    function equals(TokenKey $other): bool { return $this->value == $other->value; }
}
function create(): CancellationToken { let $source = new CancellationSource(); return $source->token; }
function inspect(CancellationToken $token): void {}
function consume(take CancellationToken $token): void {}
function fail(): int throws CancellationError { throw new CancellationError("argument failed"); }
function arguments(take CancellationToken $token, int $number): void { echo "not reached\n"; }
function visit(Iterable<CancellationToken> $tokens): void {
    foreach ($tokens as CancellationToken $value) { consume($value); }
}
function visitNullable(Iterable<?CancellationToken> $tokens): void {
    foreach ($tokens as ?CancellationToken $value) {
        if ($value != null) { consume($value); }
    }
}
function visitHeld(Iterable<Held> $tokens): void {
    foreach ($tokens as Held $value) {
        let $token = match ($value) { Held::Token($item) => $item, };
        if ($token != null) { consume($token); }
    }
}
function abandoned(CancellationToken $token): CancellationToken throws CancellationError {
    try { return $token; } finally { throw new CancellationError("return replaced"); }
}
function main(): void {
    let $token = create();
    let $held = Held::Token($token);
    let $copy = $held;
    let $selected = match ($copy) { Held::Token($value) => $value, };
    if ($selected != null) { consume($selected); inspect($selected); }
    let $holder = new Holder($token);
    let $property = $holder->token;
    consume($property); inspect($property);
    let $capture = function(): CancellationToken with ($token) { return $token; };
    let $captured = $capture(); consume($captured); inspect($captured);
    let $callback = function(take CancellationToken $input): CancellationToken { return $input; };
    let $called = $callback($token); consume($called); inspect($called);
    try { arguments($token, fail()); }
    catch (CancellationError $error) { echo $error->message . "\n"; }
    try { abandoned($token); }
    catch (CancellationError $error) { echo $error->message . "\n"; }
    ?CancellationToken $absent = null;
    List<?CancellationToken> $missing = [$absent; 2];
    List<CancellationToken> $filled = [$token; 3];
    let $deque = Deque::from($filled);
    let $filtered = $filled->filter(function(CancellationToken $value): bool { return true; });
    visit($filled); visit($filtered); visit($deque);
    visitNullable($missing);
    List<?CancellationToken> $present = [$token; 2];
    let $filteredNullable = $present->filter(function(?CancellationToken $value): bool { return true; });
    visitNullable($present); visitNullable($filteredNullable);
    List<Held> $wrapped = [$held; 2];
    let $filteredHeld = $wrapped->filter(function(Held $value): bool { return true; });
    visitHeld($wrapped); visitHeld($filteredHeld);
    Dictionary<TokenKey, CancellationToken> $indexed = [new TokenKey(1) => $token];
    Dictionary<TokenKey, ?CancellationToken> $indexedNullable = [new TokenKey(1) => $token, new TokenKey(2) => null];
    Dictionary<TokenKey, Held> $indexedHeld = [new TokenKey(1) => $held];
    visit($indexed); visitNullable($indexedNullable); visitHeld($indexedHeld);
    consume($filled[0]); inspect($filled[1]);
    let $front = $deque->peekFront;
    if ($front != null) { consume($front); inspect($front); }
    if ($missing[1] == null) { echo "absent\n"; }
    inspect($token);
    echo "claims preserved\n";
}
"#,
        "argument failed\nreturn replaced\nabsent\nclaims preserved\n",
    );
}

#[test]
fn promoted_copy_fields_preserve_parameter_claims_and_generic_inherited_storage() {
    assert_cancellation_execution(
        r#"
use Doria\Std\Concurrency\{CancellationSource, CancellationToken};
enum Held { case Token(?CancellationToken $token); }
open class Parent { function __construct(CancellationToken $token) {} }
class Child extends Parent {
    function __construct(parameter CancellationToken $input, ?CancellationToken $optional, Held $held) {
        parent::__construct($input);
    }
}
class GenericHolder<T> { function __construct(take T $value) {} }
class Taken {
    function __construct(take CancellationToken $token) { consume($token); }
}
function consume(take CancellationToken $token): void {}
function create(): CancellationToken { let $source = new CancellationSource(); return $source->token; }
function exercise(CancellationToken $token): void {
    let $parent = new Parent($token);
    let $child = new Child($token, $token, Held::Token($token));
    let $generic = new GenericHolder<CancellationToken>($token);
    let $taken = new Taken($token);
    consume($generic->value); consume($child->token); consume($taken->token);
}
function main(): void {
    let writable $index = 0;
    while ($index < 100) {
        let $token = create();
        exercise($token); consume($token);
        $index++;
    }
    echo "stored claims preserved\n";
}
"#,
        "stored claims preserved\n",
    );
}

#[test]
fn cancellation_error_preserves_owned_checked_transport_across_backends() {
    let expected = "notification failed: 0\nno system code\n";
    let program = doriac::lower_source_to_mir("cancellation-error.doria", SOURCE).unwrap();
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(output.stdout, expected.as_bytes());
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    let artifact =
        doriac::compile_source("cancellation-error.doria", SOURCE, BackendTarget::Php).unwrap();
    assert!(!matches!(
        artifact,
        doriac::backend::BackendOutput::TextWithRuntime { .. }
    ));
    let directory = native_execution::ExecutionDirectory::new();
    let path = directory.path().join("main.php");
    doriac::backend::publish_output(&path, artifact).unwrap();
    let output = std::process::Command::new("php")
        .args(["-d", "ffi.enable=0"])
        .arg(path)
        .current_dir(directory.path())
        .output()
        .expect("PHP is required for cancellation parity");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, expected.as_bytes());
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[test]
fn cancellation_error_has_qualified_concurrency_identity_without_prelude_alias() {
    let facts = doriac::names::compiler_known_symbol_facts(doriac::names::Edition::Doria2026);
    let fact = facts
        .iter()
        .find(|fact| fact.source_name == CANCELLATION_ERROR)
        .unwrap();
    assert_eq!(
        fact.provenance,
        CompilerKnownProvenance::StandardConcurrency
    );
    assert_eq!(
        fact.id.owner,
        GlobalSymbolOwner::CompilerKnown(CompilerSymbolIdentity::StandardConcurrency(
            CANCELLATION_ERROR.to_string()
        ))
    );
    assert!(doriac::check_source(
        "unqualified-cancellation.doria",
        r#"
function main(): void { let $error = new CancellationError("failed"); }
"#
    )
    .is_err());
    doriac::check_source(
        "qualified-cancellation.doria",
        r#"
function main(): void {
    let $error = new Doria\Std\Concurrency\CancellationError("failed");
    echo $error->message;
}
"#,
    )
    .unwrap();
}

#[test]
fn cancellation_error_keeps_exact_effect_and_readonly_typed_facts() {
    for source in [
        SOURCE.replace(
            "throws CancellationFailure",
            "throws Doria\\Std\\Io\\IoError",
        ),
        SOURCE.replace(
            "throws CancellationFailure",
            "throws Doria\\Std\\Time\\ClockError",
        ),
        SOURCE.replace("throws CancellationFailure", ""),
        SOURCE.replace("systemCode: $code", "systemCode: \"zero\""),
        SOURCE.replace(
            "echo describeError($error)",
            "$error->systemCode = 1; echo describeError($error)",
        ),
        SOURCE.replace(
            "echo describeError($error)",
            "$error->message = \"replaced\"; echo describeError($error)",
        ),
    ] {
        assert!(doriac::check_source("invalid-cancellation-error.doria", &source).is_err());
    }
}

#[test]
fn concurrency_ide_facts_preserve_authored_items_and_compiler_source_identity() {
    let authored = doriac::parse_source("cancellation-ide.doria", SOURCE).unwrap();
    let (program, analysis) =
        doriac::analyze_source_for_ide("cancellation-ide.doria", SOURCE).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert_eq!(program.items.len(), authored.items.len());
    let known = analysis
        .info
        .global_symbols
        .compiler_known
        .iter()
        .find(|fact| fact.id.qualified_name == CANCELLATION_ERROR)
        .unwrap();
    assert_eq!(
        known.provenance,
        CompilerKnownProvenance::StandardConcurrency
    );
    let hir = doriac::lower_source("cancellation-ide.doria", SOURCE).unwrap();
    assert!(hir.sources.iter().any(|source| {
        source.id == doriac::compiler_known_concurrency::SOURCE_ID
            && source.package == doriac::names::PackageIdentity::CompilerKnown
    }));
}
