#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution.rs"]
mod native_execution;

#[path = "common/php_execution.rs"]
mod php_execution;

use doriac::backend::NativeProfile;

const PATH: &str = "byte-region-transport.doria";
const IMPORTS: &str = "use Doria\\Std\\Io\\{ByteRegion, WritableByteRegion};\n";

fn executes(body: &str, stdout: &str) {
    let source = format!("{IMPORTS}{body}");
    let mir = doriac::lower_source_to_mir(PATH, &source)
        .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
    let result =
        doriac::mir_interpreter::interpret(&mir).unwrap_or_else(|error| panic!("{error:?}\n{mir}"));
    assert_eq!(result.stdout, stdout.as_bytes(), "{source}");
    assert_eq!(result.stderr, b"", "{source}");
    assert_eq!(result.exit_status, 0, "{source}");
    assert!(result.runtime_diagnostic.is_none());

    #[cfg(not(feature = "llvm-backend"))]
    let profiles = [NativeProfile::Fast];
    #[cfg(feature = "llvm-backend")]
    let profiles = [NativeProfile::Fast, NativeProfile::Release];
    for profile in profiles {
        native_execution::assert_native_execution(&mir, profile, stdout);
    }

    // Lowering is checked even on a host without PHP. Only execution is skipped.
    let php = doriac::compile_source_to_php(PATH, &source)
        .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
    let Ok(version) = std::process::Command::new("php").arg("--version").output() else {
        eprintln!("PHP unavailable; byte-region transport compatibility execution skipped");
        return;
    };
    assert!(version.status.success());
    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").unwrap()
    );
    let output = php_execution::Script::new(&script)
        .command(&[])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{source}\n{output:?}");
    assert_eq!(output.stdout, stdout.as_bytes(), "{source}");
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[test]
fn retained_region_return_parameters_preserve_coalesced_values_across_dispatch_forms() {
    executes(
        r#"
interface Relay {
    function identity<T>(take T $view): T;
}
class Echo implements Relay {
    function relay(ByteRegion $view): ByteRegion { return $view; }
    function generic<T>(ByteRegion $view, T $extra): ByteRegion { return $view; }
    function identity<T>(take T $view): T { return $view; }
    function borrowed<T>(T $view): T { return $view; }
}
enum Packet { case Data(int $count); case Empty; }
function relay(ByteRegion $view): ByteRegion { return $view; }
function generic<T>(ByteRegion $view, T $extra): ByteRegion { return $view; }
function identity<T>(T $view): T { return $view; }
function optional(Bytes $source): ?ByteRegion { return $source->getRegion(0, 1); }
function main(): void {
    let $bytes = Bytes::fromArray([71]);
    ?ByteRegion $missing = null;
    ?ByteRegion $present = $bytes->getRegion(0, 1);
    let $concrete = new Echo();
    Relay $erased = new Echo();
    let $a = relay($missing ?? $bytes->getRegion(0, 1));
    let $b = generic($present ?? $bytes->getRegion(0, 0), 1);
    let $c = $concrete->relay($missing ?? $bytes->getRegion(0, 1));
    let $d = $concrete->generic($present ?? $bytes->getRegion(0, 0), 1);
    let $e = $erased->identity($missing ?? $bytes->getRegion(0, 1));
    let $f = $erased->identity($present ?? $bytes->getRegion(0, 0));
    let $g = identity($missing ?? $bytes->getRegion(0, 1));
    let $h = $concrete->borrowed($missing ?? $bytes->getRegion(0, 1));
    let $i = $erased->identity(optional($bytes));
    ?Packet $missingPacket = null;
    if (identity($missingPacket ?? Packet::Data(5)) != Packet::Data(5)) { panic("free Copy payload"); }
    if ($concrete->borrowed($missingPacket ?? Packet::Data(5)) != Packet::Data(5)) { panic("method Copy payload"); }
    if ($erased->identity($missingPacket ?? Packet::Data(5)) != Packet::Data(5)) { panic("erased Copy payload"); }
    if ($i == null) { panic("missing nullable region"); }
    echo "{$a[0]} {$b[0]} {$c[0]} {$d[0]} {$e[0]} {$f[0]} {$g[0]} {$h[0]} {$i[0]}\n";
}
"#,
        "71 71 71 71 71 71 71 71 71\n",
    );
}

#[test]
fn erased_generic_owned_region_returns_use_concrete_inherited_class_facts() {
    executes(
        r#"
interface Relay { function own<T>(take T $view): T; }
open class Base<K> implements Relay {
    function __construct(take K $marker) {}
    function own<V>(take V $view): V { return $view; }
}
class Derived extends Base<int> {
    function __construct() { parent::__construct(2); }
}
function main(): void {
    let $bytes = Bytes::fromArray([71]);
    Relay $first = new Base<int>(1);
    Relay $second = new Derived();
    let $a = $first->own($bytes->getRegion(0, 1));
    let $b = $second->own($bytes->getRegion(0, 1));
    echo "{$a[0]} {$b[0]}\n";
}
"#,
        "71 71\n",
    );
}

#[test]
fn generic_owned_writable_regions_preserve_exclusive_storage_and_mutability() {
    executes(
        r#"
interface Relay { function own<T>(take T $view): T; }
class Echo implements Relay { function own<T>(take T $view): T { return $view; } }
open class Base<K> implements Relay {
    function __construct(take K $marker) {}
    function own<V>(take V $view): V { return $view; }
}
class Derived extends Base<int> { function __construct() { parent::__construct(2); } }
function own<T>(take T $view): T { return $view; }
function main(): void {
    writable Bytes $bytes = Bytes::allocate(1);
    let $concrete = new Echo();
    Relay $erased = new Echo();
    Relay $inherited = new Derived();
    { let writable $view = own($bytes->getWritableRegion(0, 1)); $view[0] = 71; }
    echo "{$bytes[0]} ";
    { let writable $view = $concrete->own($bytes->getWritableRegion(0, 1)); $view[0] = 72; }
    echo "{$bytes[0]} ";
    { let writable $view = $erased->own($bytes->getWritableRegion(0, 1)); $view[0] = 73; }
    echo "{$bytes[0]} ";
    { let writable $view = $inherited->own($bytes->getWritableRegion(0, 1)); $view[0] = 74; }
    echo "{$bytes[0]}\n";
}
"#,
        "71 72 73 74\n",
    );
}

#[test]
fn checked_region_returns_preserve_storage_on_success_and_error() {
    executes(
        r#"
class Failure implements Error { function __construct(string $message) {} }
function read(Bytes $source, bool $fail): ByteRegion throws Failure {
    if ($fail) { throw new Failure("read"); }
    return $source->getRegion(0, 1);
}
function write(writable Bytes $source, bool $fail): WritableByteRegion throws Failure {
    if ($fail) { throw new Failure("write"); }
    return $source->getWritableRegion(0, 1);
}
function main(): void {
    writable Bytes $bytes = Bytes::allocate(1);
    $bytes[0] = 21;
    try { let $view = read($bytes, false); echo "read {$view[0]}\n"; }
    catch (Failure) { echo "unexpected read failure\n"; }
    try { let $never = read($bytes, true); }
    catch (Failure $error) { echo "caught {$error->message}\n"; }
    try { let writable $view = write($bytes, false); $view[0] = 33; echo "write {$view[0]}\n"; }
    catch (Failure) { echo "unexpected write failure\n"; }
    try { let $never = write($bytes, true); }
    catch (Failure $error) { echo "caught {$error->message}\n"; }
    $bytes[0] = 44;
    echo "owner {$bytes[0]}\n";
}
"#,
        "read 21\ncaught read\nwrite 33\ncaught write\nowner 44\n",
    );
}

#[test]
fn checked_nullable_returns_distinguish_absent_from_empty_for_both_region_kinds() {
    executes(r#"
class Failure implements Error { function __construct(string $message) {} }
function read(Bytes $source, int $mode): ?ByteRegion throws Failure {
    if ($mode == 0) { return null; }
    if ($mode == 2) { throw new Failure("read"); }
    return $source->getRegion(0, 0);
}
function write(writable Bytes $source, int $mode): ?WritableByteRegion throws Failure {
    if ($mode == 0) { return null; }
    if ($mode == 2) { throw new Failure("write"); }
    return $source->getWritableRegion(0, 0);
}
function main(): void {
    writable Bytes $bytes = Bytes::allocate(0);
    try { let $view = read($bytes, 1); if ($view != null) { echo "read present {$view->length}\n"; } }
    catch (Failure) { echo "unexpected read failure\n"; }
    try { let $view = read($bytes, 0); if ($view == null) { echo "read absent\n"; } }
    catch (Failure) { echo "unexpected read failure\n"; }
    try { let $never = read($bytes, 2); }
    catch (Failure $error) { echo "caught {$error->message}\n"; }
    try { let $view = write($bytes, 1); if ($view != null) { echo "write present {$view->length}\n"; } }
    catch (Failure) { echo "unexpected write failure\n"; }
    try { let $view = write($bytes, 0); if ($view == null) { echo "write absent\n"; } }
    catch (Failure) { echo "unexpected write failure\n"; }
    try { let $never = write($bytes, 2); }
    catch (Failure $error) { echo "caught {$error->message}\n"; }
    echo "owner {$bytes->length}\n";
}
"#, "read present 0\nread absent\ncaught read\nwrite present 0\nwrite absent\ncaught write\nowner 0\n");
}

#[test]
fn structural_callbacks_transport_region_pairs_and_nullable_triples() {
    executes(
        r#"
function apply(function(ByteRegion): ByteRegion $call, ByteRegion $view): ByteRegion { return $call($view); }
function applyNullable(function(?ByteRegion): ?ByteRegion $call, ?ByteRegion $view): ?ByteRegion { return $call($view); }
function applyWritable(function(take WritableByteRegion): WritableByteRegion $call, take WritableByteRegion $view): WritableByteRegion { return $call($view); }
function applyNullableWritable(function(take ?WritableByteRegion): ?WritableByteRegion $call, take ?WritableByteRegion $view): ?WritableByteRegion { return $call($view); }
function main(): void {
    writable Bytes $bytes = Bytes::allocate(1);
    $bytes[0] = 17;
    {
        let $call = fn(ByteRegion $source) => $source;
        let $view = apply($call, $bytes->getRegion(0, 1));
        echo "read {$view[0]}\n";
        let $optionalCall = fn(?ByteRegion $source) => $source;
        let $present = applyNullable($optionalCall, $bytes->getRegion(1, 0));
        if ($present != null) { echo "read empty {$present->length}\n"; }
        let $absent = applyNullable($optionalCall, null);
        if ($absent == null) { echo "read absent\n"; }
    }
    {
        let $call = fn(take WritableByteRegion $source) => $source;
        let writable $view = applyWritable($call, $bytes->getWritableRegion(0, 1));
        $view[0] = 29;
        echo "write {$view[0]}\n";
    }
    {
        let $call = fn(take ?WritableByteRegion $source) => $source;
        let $present = applyNullableWritable($call, $bytes->getWritableRegion(1, 0));
        if ($present != null) { echo "write empty {$present->length}\n"; }
    }
    {
        let $call = fn(take ?WritableByteRegion $source) => $source;
        let $absent = applyNullableWritable($call, null);
        if ($absent == null) { echo "write absent\n"; }
    }
    $bytes[0] = 31;
    echo "owner {$bytes[0]}\n";
}
"#,
        "read 17\nread empty 0\nread absent\nwrite 29\nwrite empty 0\nwrite absent\nowner 31\n",
    );
}

#[test]
fn checked_structural_callbacks_transport_region_results_and_errors() {
    executes(
        r#"
class Failure implements Error { function __construct(string $message) {} }
function invoke(function(ByteRegion): ByteRegion throws Failure $call, ByteRegion $view): ByteRegion throws Failure { return $call($view); }
function invokeNullable(function(?ByteRegion): ?ByteRegion throws Failure $call, ?ByteRegion $view): ?ByteRegion throws Failure { return $call($view); }
function invokeWritable(function(take WritableByteRegion): WritableByteRegion throws Failure $call, take WritableByteRegion $view): WritableByteRegion throws Failure { return $call($view); }
function invokeNullableWritable(function(take ?WritableByteRegion): ?WritableByteRegion throws Failure $call, take ?WritableByteRegion $view): ?WritableByteRegion throws Failure { return $call($view); }
function main(): void {
    writable Bytes $bytes = Bytes::allocate(1);
    $bytes[0] = 19;
    {
        let $call = function (ByteRegion $source): ByteRegion {
            if ($source->length == 0) { throw new Failure("empty"); }
            return $source;
        };
        try { let $view = invoke($call, $bytes->getRegion(0, 1)); echo "read {$view[0]}\n"; }
        catch (Failure) { echo "unexpected failure\n"; }
        try { let $never = invoke($call, $bytes->getRegion(0, 0)); }
        catch (Failure $error) { echo "caught {$error->message}\n"; }
    }
    {
        let $call = function (?ByteRegion $source): ?ByteRegion {
            if ($source == null) { throw new Failure("absent"); }
            return $source;
        };
        try { let $view = invokeNullable($call, $bytes->getRegion(1, 0)); if ($view != null) { echo "present {$view->length}\n"; } }
        catch (Failure) { echo "unexpected failure\n"; }
        try { let $never = invokeNullable($call, null); }
        catch (Failure $error) { echo "caught {$error->message}\n"; }
    }
    {
        let $call = function (take WritableByteRegion $source): WritableByteRegion {
            if ($source->length == 0) { throw new Failure("empty writable"); }
            return $source;
        };
        try {
            let writable $view = invokeWritable($call, $bytes->getWritableRegion(0, 1));
            $view[0] = 31;
            echo "write {$view[0]}\n";
        } catch (Failure) { echo "unexpected failure\n"; }
        try { let $never = invokeWritable($call, $bytes->getWritableRegion(0, 0)); }
        catch (Failure $error) { echo "caught {$error->message}\n"; }
    }
    {
        let $call = function (take ?WritableByteRegion $source): ?WritableByteRegion {
            if ($source == null) { throw new Failure("absent writable"); }
            return $source;
        };
        try {
            let $view = invokeNullableWritable($call, $bytes->getWritableRegion(1, 0));
            if ($view != null) { echo "writable present {$view->length}\n"; }
        } catch (Failure) { echo "unexpected failure\n"; }
        try { let $never = invokeNullableWritable($call, null); }
        catch (Failure $error) { echo "caught {$error->message}\n"; }
    }
    $bytes[0] = 37;
    echo "owner {$bytes[0]}\n";
}
"#,
        "read 19\ncaught empty\npresent 0\ncaught absent\nwrite 31\ncaught empty writable\nwritable present 0\ncaught absent writable\nowner 37\n",
    );
}

#[test]
fn returned_readonly_callbacks_capture_regions_from_borrowed_sources() {
    executes(
        r#"
function fromBytes(Bytes $bytes): function(): uint8 {
    let $view = $bytes->getRegion(0, 1);
    return fn() with (take $view) => $view[0];
}
function fromRegion(ByteRegion $view): function(): uint8 { return fn() with (take $view) => $view[0]; }
function fromTakenRegion(take ByteRegion $view): function(): uint8 { return fn() with (take $view) => $view[0]; }
function main(): void {
    writable Bytes $bytes = Bytes::allocate(1);
    $bytes[0] = 41;
    { let $read = fromBytes($bytes); echo "bytes {$read()}:{$read()}\n"; }
    $bytes[0] = 43;
    { let $read = fromRegion($bytes->getRegion(0, 1)); echo "region {$read()}:{$read()}\n"; }
    $bytes[0] = 47;
    { let $read = fromTakenRegion($bytes->getRegion(0, 1)); echo "taken {$read()}:{$read()}\n"; }
    $bytes[0] = 53;
    echo "owner {$bytes[0]}\n";
}
"#,
        "bytes 41:41\nregion 43:43\ntaken 47:47\nowner 53\n",
    );
}

#[test]
fn returned_callback_owns_writable_descriptor_until_its_scope_ends() {
    executes(
        r#"
function make(take WritableByteRegion $view): function(): uint8 {
    return fn() with (take $view) => $view[0];
}
function main(): void {
    writable Bytes $bytes = Bytes::allocate(1);
    $bytes[0] = 61;
    { let $read = make($bytes->getWritableRegion(0, 1)); echo "captured {$read()}:{$read()}\n"; }
    $bytes[0] = 67;
    echo "owner {$bytes[0]}\n";
}
"#,
        "captured 61:61\nowner 67\n",
    );
}
