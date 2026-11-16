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
