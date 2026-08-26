use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const MANIFEST: &str = include_str!("fixtures/php_closures/manifest.txt");

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn manifest_entries() -> Vec<&'static str> {
    MANIFEST
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect()
}

fn php_available() -> bool {
    Command::new("php")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn expected_status(root: &Path) -> i32 {
    fs::read_to_string(root.join("expected_status"))
        .ok()
        .map(|status| {
            status
                .trim()
                .parse()
                .expect("fixture status should be numeric")
        })
        .unwrap_or(0)
}

fn expected_bytes(root: &Path, name: &str) -> Vec<u8> {
    fs::read(root.join(name)).unwrap_or_default()
}

fn generated_script(name: &str, php: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should follow the Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "doria-stage30f-{name}-{}-{nonce}.php",
        std::process::id()
    ));
    fs::write(&path, php).expect("generated Stage 30f PHP should be writable");
    path
}

#[test]
fn php_closure_manifest_is_unique_and_resolves_every_fixture() {
    let entries = manifest_entries();
    assert_eq!(
        entries.iter().copied().collect::<BTreeSet<_>>().len(),
        entries.len(),
        "PHP closure manifest entries must be unique"
    );
    for entry in entries {
        let root = fixture_root().join(entry);
        assert!(
            root.join("source.doria").is_file(),
            "missing {entry} source"
        );
        assert!(
            root.join("expected_status").is_file() || expected_status(&root) == 0,
            "invalid {entry} status sidecar"
        );
    }
}

#[test]
fn php_closure_manifest_matches_the_interpreter_and_generated_php() {
    if !php_available() {
        if std::env::var_os("CI").is_some() {
            panic!("Stage 30f PHP parity requires a PHP interpreter in CI");
        }
        eprintln!("PHP is unavailable; skipping local Stage 30f execution parity");
        return;
    }

    for entry in manifest_entries() {
        let root = fixture_root().join(entry);
        let source_path = root.join("source.doria");
        let source = fs::read_to_string(&source_path)
            .unwrap_or_else(|error| panic!("failed to read {entry}: {error}"));
        let source_name = source_path.to_string_lossy();
        let expected_stdout = expected_bytes(&root, "expected_stdout");
        let expected_stderr = expected_bytes(&root, "expected_stderr");
        let expected_stderr_contains = expected_bytes(&root, "expected_stderr_contains");
        let status = expected_status(&root);

        let mir = doriac::lower_source_to_mir(source_name.as_ref(), &source)
            .unwrap_or_else(|diagnostics| panic!("{entry} should lower: {diagnostics:#?}"));
        let interpreted = doriac::mir_interpreter::interpret(&mir)
            .unwrap_or_else(|error| panic!("{entry} should interpret: {error:?}"));
        assert_eq!(
            interpreted.stdout, expected_stdout,
            "{entry} interpreter stdout"
        );
        assert_eq!(
            interpreted.exit_status, status,
            "{entry} interpreter status"
        );
        if expected_stderr_contains.is_empty() {
            assert_eq!(
                interpreted.stderr, expected_stderr,
                "{entry} interpreter stderr"
            );
        } else {
            assert!(
                interpreted
                    .stderr
                    .windows(expected_stderr_contains.len())
                    .any(|window| window == expected_stderr_contains),
                "{entry} interpreter stderr did not contain {:?}: {}",
                String::from_utf8_lossy(&expected_stderr_contains),
                String::from_utf8_lossy(&interpreted.stderr)
            );
        }

        let php = doriac::compile_source_to_php(source_name.as_ref(), &source).unwrap_or_else(
            |diagnostics| panic!("{entry} should compile to PHP: {diagnostics:#?}"),
        );
        assert!(
            !php.contains("E0641"),
            "{entry} retained the Stage 30f boundary"
        );
        assert!(
            !php.contains("call_user_func"),
            "{entry} used host callable dispatch"
        );
        assert!(php.contains("__DoriaFunctionValue"));

        let name = entry.replace('/', "-");
        let script = generated_script(&name, &php);
        let lint = Command::new("php")
            .arg("-l")
            .arg(&script)
            .output()
            .unwrap_or_else(|error| panic!("failed to lint {entry}: {error}"));
        assert!(
            lint.status.success(),
            "{entry} PHP syntax: {}",
            String::from_utf8_lossy(&lint.stderr)
        );
        let run = Command::new("php")
            .arg("-d")
            .arg("display_errors=0")
            .arg(&script)
            .output()
            .unwrap_or_else(|error| panic!("failed to execute {entry}: {error}"));
        let _ = fs::remove_file(&script);

        assert_eq!(run.stdout, expected_stdout, "{entry} PHP stdout");
        assert_eq!(run.status.code(), Some(status), "{entry} PHP status");
        assert!(
            !run.stderr
                .windows("__doriaClosureEntry".len())
                .any(|window| { window == "__doriaClosureEntry".as_bytes() })
                && !run
                    .stderr
                    .windows("__DoriaClosureValue".len())
                    .any(|window| { window == "__DoriaClosureValue".as_bytes() }),
            "{entry} leaked generated PHP closure names: {}",
            String::from_utf8_lossy(&run.stderr)
        );
        if expected_stderr_contains.is_empty() {
            assert_eq!(run.stderr, expected_stderr, "{entry} PHP stderr");
        } else {
            assert!(
                run.stderr
                    .windows(expected_stderr_contains.len())
                    .any(|window| window == expected_stderr_contains),
                "{entry} PHP stderr did not contain {:?}: {}",
                String::from_utf8_lossy(&expected_stderr_contains),
                String::from_utf8_lossy(&run.stderr)
            );
        }
    }
}

#[test]
fn generated_php_uses_explicit_environments_cells_and_exact_carriers() {
    let source = r#"
function main(): void
{
    let $left = "left";
    let writable $right = "right";
    let writable $join = function (): string with ($left, writable $right) {
        $right = $right . "!";
        return $left . $right;
    };
    echo $join() . "\n";
}
"#;
    let php = doriac::compile_source_to_php("stage30f-structure.doria", source)
        .expect("capturing closure should emit PHP");

    assert!(php.contains("interface __DoriaFunctionValue"));
    assert!(php.contains("final class __DoriaClosureValue"));
    assert!(php.contains("final class __DoriaClosureEnvironment"));
    assert!(php.contains("public __DoriaCell $field0;"));
    assert!(php.contains("public __DoriaCell $field1;"));
    assert!(php.contains("__doria_take_cell"));
    assert!(php.contains("__doria_drop_cell"));
    assert!(!php.contains("call_user_func"));
    assert!(!php.contains("call_user_func_array"));
    assert!(!php.contains("mixed ...$arguments"));
    assert!(!php.contains("function accept(callable"));
}

#[test]
fn no_capture_php_closure_has_no_environment_and_property_drop_uses_a_temporary() {
    let source = r#"
class Runner
{
    writable function(): string $callback = fn() => "old";
    function run(): string { return $this->callback(); }
}
function main(): void
{
    let $runner = new Runner();
    echo $runner->run() . "\n";
}
"#;
    let php = doriac::compile_source_to_php("stage30f-no-capture.doria", source)
        .expect("no-capture function property should emit PHP");

    assert!(php.contains("final class __DoriaClosureValue"));
    assert!(!php.contains("final class __DoriaClosureEnvironment"));
    assert!(!php.contains("callable $callback"));
    assert!(php.contains("__DoriaFunctionValue $callback"));
    assert!(php.contains("$__doriaPropertyValue = $this->callback;"));
    assert!(!php.contains("__doria_drop_value($this->callback)"));
}
