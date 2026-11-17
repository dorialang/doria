//! Stage 40 execution (Decision 0148): the unsafe and foreign forms each
//! execution target builds, run through the interpreter and every native
//! profile this compiler build has.

#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution_runner.rs"]
mod native_execution;

use doriac::backend::NativeProfile;
use doriac::mir_interpreter::interpret;
use native_execution::execute_native;

fn program(source: &str) -> doriac::mir::Program {
    doriac::lower_source_to_mir("stage40.doria", source.to_string())
        .unwrap_or_else(|diagnostics| panic!("the program should build: {diagnostics:#?}"))
}

/// The program prints `stdout` and exits 0 in the interpreter and on every
/// native profile.
fn assert_runs_everywhere(source: &str, stdout: &str) {
    let mir = program(source);
    let interpreted = interpret(&mir).expect("the interpreter should run the program");
    assert_eq!(
        (
            String::from_utf8_lossy(&interpreted.stdout).into_owned(),
            interpreted.exit_status
        ),
        (stdout.to_string(), 0),
        "interpreter: {:?}",
        interpreted.runtime_diagnostic
    );
    for profile in NativeProfile::AVAILABLE {
        let run = execute_native(&mir, *profile);
        assert_eq!(run.status.code(), Some(0), "{profile:?}: {run:?}");
        assert_eq!(String::from_utf8_lossy(&run.stdout), stdout, "{profile:?}");
        assert!(run.stderr.is_empty(), "{profile:?}: {run:?}");
    }
}

/// The program panics with `code` after printing `stdout`, in the
/// interpreter and on every native profile.
fn assert_panics_everywhere(source: &str, stdout: &str, code: &str) {
    let mir = program(source);
    let interpreted = interpret(&mir).expect("the interpreter should run the program");
    assert_eq!(String::from_utf8_lossy(&interpreted.stdout), stdout);
    assert_eq!(interpreted.exit_status, 101);
    assert_eq!(
        interpreted
            .runtime_diagnostic
            .as_ref()
            .map(|diagnostic| diagnostic.code),
        Some(code)
    );
    for profile in NativeProfile::AVAILABLE {
        let run = execute_native(&mir, *profile);
        assert_eq!(run.status.code(), Some(101), "{profile:?}: {run:?}");
        assert_eq!(String::from_utf8_lossy(&run.stdout), stdout, "{profile:?}");
        let stderr = String::from_utf8_lossy(&run.stderr);
        assert!(stderr.contains(code), "{profile:?}: {stderr}");
    }
}

const BOUNDARY: &str = "use Doria\\Std\\Ffi\\{CChar, CLong, CPointerDifference, CSize, CUnsignedLong, CWideChar};\n";

#[test]
fn boundary_values_convert_compare_and_convert_back() {
    let source = format!(
        "{BOUNDARY}
function widen(CSize $size): int64
{{
    return Int64::from($size);
}}

function main(): void
{{
    let $size = CSize::from(42);
    echo widen($size);
    echo \" \";
    echo $size == CSize::from(42);
    echo \" \";
    echo $size != CSize::from(41);
    echo \" \";
    let $long = CLong::from(-7);
    echo Int64::from($long);
    echo \" \";
    let $difference = CPointerDifference::from(-9);
    echo Int64::from($difference);
    echo \" \";
    let $character = CChar::from(65);
    echo UInt8::from($character);
    echo \" \";
    let $wide = CWideChar::from(9731);
    echo UInt32::from($wide);
    echo \" \";
    let $unsigned = CUnsignedLong::from(UInt32::from(4000000000));
    echo UInt64::from($unsigned);
}}
"
    );
    assert_runs_everywhere(&source, "42 true true -7 -9 65 9731 4000000000");
}

#[test]
fn try_from_gives_null_outside_the_range_on_this_target() {
    let source = format!(
        "{BOUNDARY}
function main(): void
{{
    ?CSize $negative = CSize::tryFrom(-1);
    echo $negative == null;
    echo \" \";
    ?CSize $present = CSize::tryFrom(3);
    echo ($present ?? CSize::from(0)) == CSize::from(3);
    echo \" \";
    ?CChar $character = CChar::tryFrom(300);
    echo $character == null;
    echo \" \";
    ?CLong $long = CLong::tryFrom(-5);
    echo Int64::from($long ?? CLong::from(0));
}}
"
    );
    assert_runs_everywhere(&source, "true true true -5");
}

#[test]
fn a_value_outside_the_boundary_range_panics_with_p1701() {
    let source = format!(
        "{BOUNDARY}
function main(): void
{{
    echo \"before \";
    let $size = CSize::from(-1);
    echo $size == CSize::from(0);
}}
"
    );
    assert_panics_everywhere(&source, "before ", "P1701");

    // The fixed-width companion's conversion from a boundary value stays
    // P1109, as for any other integer.
    let narrowing = format!(
        "{BOUNDARY}
function main(): void
{{
    let $size = CSize::from(300);
    echo UInt8::from($size);
}}
"
    );
    assert_panics_everywhere(&narrowing, "", "P1109");
}

#[test]
fn php_output_reports_that_it_does_not_build_boundary_values_yet() {
    let source = format!("{BOUNDARY}function main(): void {{ let $size = CSize::from(1); }}");
    let diagnostics = doriac::compile_source(
        "stage40.doria",
        source,
        doriac::backend::BackendTarget::Php,
    )
    .expect_err("PHP output does not build boundary values yet");
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic.code == "E0784"
            && diagnostic.title == "C Boundary Values Cannot Be Built For PHP Output Yet"),
        "{diagnostics:#?}"
    );
}

/// The program prints `stdout` and exits 0 on every native profile; the
/// interpreter reports that it runs pointer operations through the native
/// companion library.
fn assert_runs_natively(source: &str, stdout: &str) {
    let mir = program(source);
    assert_interpreter_reports_companion(&mir);
    for profile in NativeProfile::AVAILABLE {
        let run = execute_native(&mir, *profile);
        assert_eq!(run.status.code(), Some(0), "{profile:?}: {run:?}");
        assert_eq!(String::from_utf8_lossy(&run.stdout), stdout, "{profile:?}");
        assert!(run.stderr.is_empty(), "{profile:?}: {run:?}");
    }
}

/// The program panics with `code` and `title` on every native profile.
fn assert_panics_natively(source: &str, code: &str, title: &str) {
    let mir = program(source);
    assert_interpreter_reports_companion(&mir);
    for profile in NativeProfile::AVAILABLE {
        let run = execute_native(&mir, *profile);
        assert_eq!(run.status.code(), Some(101), "{profile:?}: {run:?}");
        let stderr = String::from_utf8_lossy(&run.stderr);
        assert!(
            stderr.contains(code) && stderr.contains(title),
            "{profile:?}: {stderr}"
        );
    }
}

fn assert_interpreter_reports_companion(mir: &doriac::mir::Program) {
    let companion = doriac::unsafe_code::companion_operations(mir);
    assert!(
        companion.iter().all(|diagnostic| diagnostic.code == "E0784"
            && diagnostic.title == "Pointer Operations Cannot Run In The Interpreter Yet"),
        "{companion:#?}"
    );
    assert!(!companion.is_empty());
    let error = interpret(mir).expect_err("the interpreter runs pointers through the companion");
    assert!(error.to_string().contains("pointer operations"), "{error}");
}

const POINTER: &str = "use Doria\\Std\\Ffi\\{MutPtr, Ptr};\n";

#[test]
fn pointer_addresses_offset_cast_and_compare() {
    let source = format!(
        "{POINTER}
function step(Ptr<int32> $pointer, int $count): Ptr<int32>
{{
    return unsafe($pointer->offset($count));
}}

function main(): void
{{
    unsafe {{
        ?Ptr<int32> $absent = Ptr::fromAddress(0);
        echo $absent == null;
        echo \" \";
        ?Ptr<int32> $maybe = Ptr::fromAddress(4096);
        if ($maybe != null) {{
            let $base = $maybe;
            echo $base->address;
            echo \" \";
            echo step($base, 3)->address;
            echo \" \";
            echo $base->offset(-2)->address;
            echo \" \";
            echo $base->byteOffset(6)->address;
            echo \" \";
            Ptr<uint8> $bytes = Ptr::cast($base);
            echo $bytes->offset(5)->address;
            echo \" \";
            MutPtr<int64> $wide = MutPtr::cast($base);
            echo $wide->offset(1)->address;
            echo \" \";
            Ptr<int64> $readonly = Ptr::from($wide);
            echo $readonly->address == $base->address;
            echo \" \";
            echo $base == step($base, 0);
            echo \" \";
            echo $base != step($base, 1);
        }}
    }}
}}
"
    );
    assert_runs_natively(
        &source,
        "true 4096 4108 4088 4102 4101 4104 true true true",
    );
}

#[test]
fn an_aligned_access_at_a_misaligned_address_panics_with_p1702() {
    for body in [
        "?Ptr<int32> $pointer = Ptr::fromAddress(4098); if ($pointer != null) { echo $pointer->read(); }",
        "?Ptr<float64> $pointer = Ptr::fromAddress(4100); if ($pointer != null) { MutPtr<float64> $mutable = MutPtr::cast($pointer); $mutable->write(1.5); }",
    ] {
        let source = format!("{POINTER}function main(): void {{ unsafe {{ {body} }} }}");
        assert_panics_natively(&source, "P1702", "Pointer Access Is Misaligned");
    }
}

#[test]
fn an_offset_that_leaves_the_address_space_panics_with_p1703() {
    for body in [
        // Below zero.
        "?Ptr<uint8> $pointer = Ptr::fromAddress(8); if ($pointer != null) { echo $pointer->byteOffset(-16)->address; }",
        // To zero, which no `Ptr<T>` holds.
        "?Ptr<int64> $pointer = Ptr::fromAddress(16); if ($pointer != null) { echo $pointer->offset(-2)->address; }",
        // Past the largest address.
        "?Ptr<uint8> $pointer = Ptr::fromAddress(18446744073709551608); if ($pointer != null) { echo $pointer->byteOffset(8)->address; }",
        // The scaled count overflows before the address moves.
        "?Ptr<int64> $pointer = Ptr::fromAddress(4096); if ($pointer != null) { echo $pointer->offset(2305843009213693952)->address; }",
    ] {
        let source = format!("{POINTER}function main(): void {{ unsafe {{ {body} }} }}");
        assert_panics_natively(&source, "P1703", "Pointer Arithmetic Left The Address Space");
    }
}

#[test]
fn the_interpreter_and_php_output_report_pointer_operations() {
    let source = format!(
        "{POINTER}function main(): void {{ unsafe {{ ?Ptr<int32> $pointer = Ptr::fromAddress(4096); echo $pointer == null; }} }}"
    );
    let debug = doriac::compile_source(
        "stage40.doria",
        source.clone(),
        doriac::backend::BackendTarget::Debug,
    )
    .expect_err("the interpreter runs pointers through the companion");
    assert!(
        debug.iter().any(|diagnostic| diagnostic.code == "E0784"
            && diagnostic.title == "Pointer Operations Cannot Run In The Interpreter Yet"),
        "{debug:#?}"
    );
    let php = doriac::compile_source(
        "stage40.doria",
        source,
        doriac::backend::BackendTarget::Php,
    )
    .expect_err("PHP output runs pointers through the companion");
    assert!(
        php.iter().any(|diagnostic| diagnostic.code == "E0784"
            && diagnostic.title == "Pointers Cannot Be Built For PHP Output Yet"),
        "{php:#?}"
    );
}

#[test]
fn a_pointer_to_a_local_reads_and_writes_its_place() {
    let source = format!(
        "{POINTER}
function main(): void
{{
    writable int32 $value = 41;
    unsafe {{
        let $pointer = MutPtr::to($value);
        $pointer->write(7);
        echo $pointer->read();
        echo \" \";
        echo $value;
    }}
    echo \" \";
    writable float64 $ratio = 0.0;
    unsafe {{
        let $place = MutPtr::to($ratio);
        $place->writeUnaligned(2.5);
        echo Float::toInt($place->readUnaligned() * 2.0);
    }}
    echo \" \";
    writable bool $flag = false;
    unsafe {{
        MutPtr::to($flag)->write(true);
    }}
    echo $flag;
    echo \" \";
    writable int64 $wide = 0;
    unsafe {{
        MutPtr<uint8> $low = MutPtr::cast(MutPtr::to($wide));
        $low->write(UInt8::from(255));
        $low->byteOffset(1)->write(UInt8::from(1));
    }}
    echo $wide;
}}
"
    );
    // Every tier-1 target is little-endian.
    assert_runs_natively(&source, "7 7 5 true 511");

    // A writable Copy parameter of a function is the callee's own copy
    // (SPEC: Copy arguments keep their Copy behavior), so a pointer to it
    // addresses that copy. A writable closure parameter and a writable
    // capture name the caller's place, so a write through a pointer to
    // either is what the caller reads next.
    let parameters = format!(
        "{POINTER}function bump(writable int32 $count): void {{ unsafe {{ MutPtr::to($count)->write(8); }} echo $count . \" \"; }}
function main(): void {{
    writable int32 $value = 1;
    bump($value);
    echo $value . \" \";
    let $set = function(writable int32 $target): void {{ unsafe {{ MutPtr::to($target)->write(5); }} echo $target . \" \"; }};
    $set($value);
    echo $value . \" \";
    writable int32 $captured = 2;
    writable function writable(): void $raise = function(): void with (writable $captured) {{
        unsafe {{ MutPtr::to($captured)->write(9); }}
        echo $captured . \" \";
        $captured = $captured + 1;
    }};
    $raise();
    echo $captured;
}}
"
    );
    assert_runs_natively(&parameters, "8 1 5 5 9 10");
}
