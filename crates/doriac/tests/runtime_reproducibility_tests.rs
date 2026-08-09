//! Regression coverage for deterministic compiler-bundled runtime artifacts.
//!
//! The old build selected dependency rlibs by modification time. Host and
//! target copies shared a directory, so one revision could bundle materially
//! different runtimes depending on Cargo scheduling. These tests exercise the
//! replacement contract at its observable boundary: Cargo builds `doria-rt`
//! from its own manifest, and every clean build agrees with the archive bundled
//! into this compiler.

use std::path::{Path, PathBuf};
use std::process::Command;

use doriac::runtime_artifact::{RuntimeArtifact, RuntimeOrigin};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("doriac lives under the workspace crates directory")
        .to_path_buf()
}

fn target_triple() -> &'static str {
    env!("DORIA_RT_EXPECTED_TARGET")
}

fn archive_filename() -> &'static str {
    if target_triple().ends_with("windows-msvc") {
        "doria_rt.lib"
    } else {
        "libdoria_rt.a"
    }
}

fn build_runtime_into(target_dir: &Path, profile: &str) -> PathBuf {
    let root = workspace_root();
    let mut command = Command::new(env!("CARGO"));
    command
        .arg("build")
        .arg("--manifest-path")
        .arg(root.join("crates/doria-rt/Cargo.toml"))
        .arg("--package")
        .arg("doria-rt")
        .arg("--locked")
        .arg("--target")
        .arg(target_triple())
        .env("CARGO_TARGET_DIR", target_dir)
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env("CARGO_INCREMENTAL", "0");
    if profile == "release" {
        command.arg("--release");
    }
    if std::env::var("CARGO_NET_OFFLINE").is_ok_and(|value| value == "true") {
        command.arg("--offline");
    }
    let output = command
        .output()
        .expect("failed to invoke Cargo for doria-rt");
    assert!(
        output.status.success(),
        "building doria-rt failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let archive = target_dir
        .join(target_triple())
        .join(profile)
        .join(archive_filename());
    assert!(
        archive.is_file(),
        "Cargo did not produce the runtime archive at {}",
        archive.display()
    );
    archive
}

fn identity(archive: &Path) -> (Option<u64>, Option<String>) {
    let provenance = RuntimeArtifact {
        path: archive.to_path_buf(),
        origin: RuntimeOrigin::CompilerBundled,
        metadata: None,
    }
    .provenance();
    (provenance.bytes, provenance.sha256)
}

fn bundled_runtime(profile: &str) -> PathBuf {
    let path = match profile {
        "debug" => option_env!("DORIA_RT_BUILT_DEBUG_PATH"),
        "release" => option_env!("DORIA_RT_BUILT_RELEASE_PATH"),
        other => panic!("unknown runtime profile {other}"),
    };
    path.map(PathBuf::from)
        .unwrap_or_else(|| panic!("a bundled-runtime build must record its {profile} archive"))
}

#[test]
fn one_revision_produces_one_runtime_archive() {
    if !cfg!(feature = "bundled-runtime") {
        return;
    }

    let scratch = Path::new(env!("CARGO_TARGET_TMPDIR"));
    for profile in ["debug", "release"] {
        let bundled = bundled_runtime(profile);
        let first = build_runtime_into(
            &scratch.join(format!("runtime-reproducibility-{profile}-first")),
            profile,
        );
        let second = build_runtime_into(
            &scratch.join(format!("runtime-reproducibility-{profile}-second")),
            profile,
        );

        let bundled_identity = identity(&bundled);
        let first_identity = identity(&first);
        let second_identity = identity(&second);
        assert!(
            bundled_identity.1.is_some(),
            "bundled {profile} runtime was unreadable"
        );
        assert_eq!(
            bundled_identity, first_identity,
            "the compiler-bundled {profile} runtime differs from a clean build"
        );
        assert_eq!(
            first_identity, second_identity,
            "one revision produced different {profile} runtime archives in clean directories"
        );
    }
}

#[test]
fn bundled_runtime_sidecar_describes_the_archive() {
    if !cfg!(feature = "bundled-runtime") {
        return;
    }

    for profile in ["debug", "release"] {
        let bundled = bundled_runtime(profile);
        let mut metadata_name = bundled
            .file_name()
            .expect("runtime archive filename")
            .to_os_string();
        metadata_name.push(".doria-runtime.json");
        let metadata_path = bundled.with_file_name(metadata_name);
        let document = std::fs::read_to_string(&metadata_path)
            .unwrap_or_else(|error| panic!("reading {}: {error}", metadata_path.display()));
        let (bytes, sha256) = identity(&bundled);
        let bytes = bytes.expect("runtime archive size");
        let sha256 = sha256.expect("runtime archive digest");

        assert!(document.contains(&format!("\"profile\": \"{profile}\"")));
        assert!(document.contains(&format!("\"bytes\": {bytes}")));
        assert!(document.contains(&format!("\"sha256\": \"{sha256}\"")));
    }
}
