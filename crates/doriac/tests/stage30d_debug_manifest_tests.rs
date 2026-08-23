use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/debug_closures")
}

fn manifest_entries() -> Vec<String> {
    include_str!("fixtures/debug_closures/manifest.txt")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

#[test]
fn durable_debug_closure_manifest_covers_every_fixture() {
    let root = fixture_root();
    let manifest = manifest_entries().into_iter().collect::<BTreeSet<_>>();
    let disk = fs::read_dir(&root)
        .expect("debug closure fixture directory should exist")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect::<BTreeSet<_>>();

    assert_eq!(manifest, disk);
}

#[test]
fn durable_debug_closure_manifest_executes_through_the_interpreter() {
    let root = fixture_root();
    for name in manifest_entries() {
        let fixture = root.join(&name);
        let source = fs::read_to_string(fixture.join("source.doria"))
            .unwrap_or_else(|error| panic!("failed to read {name} source: {error}"));
        let mut expected = fs::read_to_string(fixture.join("expected_debug"))
            .unwrap_or_else(|error| panic!("failed to read {name} expectation: {error}"));
        expected.push('\n');
        let actual = doriac::compile_source_to_debug(
            fixture.join("source.doria").to_string_lossy().as_ref(),
            &source,
        )
        .unwrap_or_else(|diagnostics| panic!("{name} should execute: {diagnostics:#?}"));

        assert_eq!(actual, expected, "debug closure fixture {name}");
    }
}
