#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution.rs"]
mod native_execution;

use doriac::backend::{BackendTarget, NativeProfile};

fn assert_native_execution(source: &str, expected: &str) {
    let mir = doriac::lower_source_to_mir("nullable-defaults.doria", source).unwrap();
    let output = doriac::mir_interpreter::interpret(&mir).unwrap();
    assert_eq!(output.stdout, expected.as_bytes());
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
    native_execution::assert_native_execution(&mir, NativeProfile::Fast, expected);
    #[cfg(feature = "llvm-backend")]
    native_execution::assert_native_execution(&mir, NativeProfile::Release, expected);
}

fn assert_execution(source: &str, expected: &str) {
    assert_native_execution(source, expected);
    let artifact =
        doriac::compile_source("nullable-defaults.doria", source, BackendTarget::Php).unwrap();
    let directory = native_execution::ExecutionDirectory::new();
    let path = directory.path().join("main.php");
    doriac::backend::publish_output(&path, artifact).unwrap();
    let output = std::process::Command::new("php")
        .arg(path)
        .current_dir(directory.path())
        .output()
        .expect("PHP is required for nullable default parity");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, expected.as_bytes());
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[test]
fn every_copy_scalar_width_preserves_null_and_present_defaults() {
    for include_float32 in [true, false] {
        let mut source = String::new();
        let mut main = String::from("function main(): void {");
        let mut expected = String::new();
        for (index, (ty, present, absent)) in [
            ("int", "7", "0"),
            ("int8", "7", "0"),
            ("int16", "7", "0"),
            ("int32", "7", "0"),
            ("int64", "7", "0"),
            ("uint8", "7", "0"),
            ("uint16", "7", "0"),
            ("uint32", "7", "0"),
            ("uint64", "7", "0"),
            ("float", "1.5", "0.0"),
            ("float32", "1.5", "0.0"),
            ("float64", "1.5", "0.0"),
            ("bool", "true", "false"),
        ]
        .into_iter()
        .enumerate()
        .filter(|(_, (ty, _, _))| include_float32 || *ty != "float32")
        {
            source.push_str(&format!(
                "const ?{ty} EMPTY_{index} = null;\n\
                 const ?{ty} COPY_{index} = EMPTY_{index};\n\
                 function present{index}(?{ty} $value = {present}): bool {{ return ($value ?? {absent}) == {present}; }}\n\
                 function absent{index}(?{ty} $value = null): bool {{ return $value == null; }}\n"
            ));
            main.push_str(&format!(
                "echo present{index}() . \"/\" . absent{index}() . \"/\" . absent{index}(COPY_{index}) . \";\";"
            ));
            expected.push_str("true/true/true;");
        }
        source.push_str(&main);
        source.push('}');
        if include_float32 {
            assert_native_execution(&source, &expected);
            let diagnostics =
                doriac::compile_source("float32-defaults.doria", source, BackendTarget::Php)
                    .unwrap_err();
            assert!(
                diagnostics
                    .iter()
                    .any(|error| error.code == "B1301" && error.message.contains("float32")),
                "{diagnostics:?}"
            );
        } else {
            assert_execution(&source, &expected);
        }
    }
}

#[test]
fn callable_kinds_named_omissions_and_copy_enums_share_default_materialization() {
    let source = r#"
enum Mode { case Ready; case Stopped; }
function combine(int $first, ?int $middle = 3, int $last = 9): int {
    return $first + ($middle ?? 0) + $last;
}
class Defaults {
    function __construct(writable ?int $value = null) {}
    static function getNumber(?float $value = 2.5): float { return $value ?? 0.0; }
    function isReady(?Mode $mode = Mode::Ready): bool { return ($mode ?? Mode::Stopped) == Mode::Ready; }
    function isAbsent(?Mode $mode = null): bool { return $mode == null; }
    writable function update(writable ?int $value = 0): void {
        $value = 4;
        $this->value = $value;
    }
}
function main(): void {
    let writable $defaults = new Defaults();
    echo ($defaults->value ?? -1) . ":";
    $defaults->update();
    echo ($defaults->value ?? -1) . ":" . (Defaults::getNumber() == 2.5) . ":";
    echo $defaults->isReady() . ":" . $defaults->isAbsent() . ":";
    echo combine(first: 1, last: 5) . ":" . combine(1, null, 5) . "\n";
}
"#;
    assert_execution(source, "-1:4:true:true:true:9:6\n");
}

#[test]
fn nullable_defaults_preserve_constant_typing_and_existing_ownership_boundaries() {
    for source in [
        "function runtimeValue(): int { return 1; } function value(?int $value = runtimeValue()): void {}",
        "function value(?int $value = 1.5): void {}",
        "function value(?uint8 $value = 256): void {}",
        "function value(?string $value = null): void {}",
        "function value(take ?int $value = null): void {}",
        "class Owner {} function value(?Owner $value = null): void {}",
    ] {
        assert!(doriac::check_source("invalid-nullable-default.doria", source).is_err(), "{source}");
    }
}
