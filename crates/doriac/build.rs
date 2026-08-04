use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

// One SHA-256 implementation shared with the compiler, so the digest recorded
// in the sidecar here and the digest verified at compile time cannot diverge.
include!("src/runtime_digest.rs");

/// Version of the sidecar document itself, independent of the runtime ABI.
const RUNTIME_METADATA_SCHEMA_VERSION: u32 = 1;

fn main() {
    let package_version = env::var("CARGO_PKG_VERSION").expect("Cargo package version");
    let toolchain_version = canonical_toolchain_version(&package_version);
    println!("cargo:rustc-env=DORIA_TOOLCHAIN_VERSION={toolchain_version}");

    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let repository_dir = manifest_dir.join("../..");
    println!("cargo:rerun-if-env-changed=DORIA_BUILD_COMMIT");
    watch_git_identity(&repository_dir);
    let build_commit = env::var("DORIA_BUILD_COMMIT")
        .ok()
        .or_else(|| git_output(&repository_dir, &["rev-parse", "--verify", "HEAD"]))
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=DORIA_BUILD_COMMIT={build_commit}",);

    let runtime_dir = manifest_dir.join("../doria-rt");
    let runtime_source_dir = runtime_dir.join("src");
    let runtime_source = runtime_source_dir.join("lib.rs");
    let runtime_manifest = runtime_dir.join("Cargo.toml");
    println!("cargo:rerun-if-changed={}", runtime_source_dir.display());
    println!("cargo:rerun-if-changed={}", runtime_manifest.display());

    // The ABI version is a single plain-text authority read here rather than a
    // constant duplicated between the runtime and the compiler. It is emitted
    // before the bundled-runtime early return so the compiler always knows
    // which ABI it expects, even in builds that carry no bundled archive.
    let abi_path = runtime_dir.join("RUNTIME_ABI_VERSION");
    println!("cargo:rerun-if-changed={}", abi_path.display());
    let runtime_abi_version = std::fs::read_to_string(&abi_path)
        .unwrap_or_else(|error| panic!("runtime ABI version at {}: {error}", abi_path.display()));
    let runtime_abi_version = runtime_abi_version.trim().to_string();
    assert!(
        !runtime_abi_version.is_empty()
            && runtime_abi_version
                .bytes()
                .all(|byte| byte.is_ascii_digit()),
        "runtime ABI version must be a non-empty decimal number"
    );
    println!("cargo:rustc-env=DORIA_RT_ABI_VERSION={runtime_abi_version}");

    let target = env::var("TARGET").expect("Cargo target triple");
    let profile = env::var("PROFILE").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=DORIA_RT_EXPECTED_TARGET={target}");
    println!("cargo:rustc-env=DORIA_RT_EXPECTED_PROFILE={profile}");
    println!("cargo:rustc-env=DORIA_RT_EXPECTED_REVISION={build_commit}");

    if env::var_os("CARGO_FEATURE_BUNDLED_RUNTIME").is_none() {
        return;
    }

    let filename = if target.ends_with("windows-msvc") {
        "doria_rt.lib"
    } else {
        "libdoria_rt.a"
    };
    let output =
        PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory")).join(filename);
    let dependency_dir = output
        .parent()
        .and_then(|path| path.parent())
        .and_then(|path| path.parent())
        .and_then(|path| path.parent())
        .expect("Cargo build directory")
        .join("deps");
    let ryu = find_rlib(&dependency_dir, "ryu");
    let doria_diagnostic_catalogue = find_rlib(&dependency_dir, "doria_diagnostic_catalogue");
    let doria_unicode = find_rlib(&dependency_dir, "doria_unicode");
    let unicode_width = find_rlib(&dependency_dir, "unicode_width");
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc"));
    let mut command = Command::new(rustc);
    command
        .arg("--crate-name")
        .arg("doria_rt")
        .arg("--edition=2021")
        .arg("--crate-type=staticlib")
        .arg("--target")
        .arg(&target)
        .arg("--cfg")
        .arg("feature=\"standalone-windows-support\"")
        .arg("-L")
        .arg(format!("dependency={}", dependency_dir.display()))
        .arg("--extern")
        .arg(format!("ryu={}", ryu.display()))
        .arg("--extern")
        .arg(format!(
            "doria_diagnostic_catalogue={}",
            doria_diagnostic_catalogue.display()
        ))
        .arg("--extern")
        .arg(format!("doria_unicode={}", doria_unicode.display()))
        .arg("--extern")
        .arg(format!("unicode_width={}", unicode_width.display()))
        .arg("-C")
        .arg(format!(
            "opt-level={}",
            env::var("OPT_LEVEL").unwrap_or_else(|_| "0".to_string())
        ));
    if let Some(encoded_flags) = env::var_os("CARGO_ENCODED_RUSTFLAGS") {
        for flag in encoded_flags.to_string_lossy().split('\u{1f}') {
            if !flag.is_empty() {
                command.arg(flag);
            }
        }
    }
    let status = command
        .arg("-C")
        .arg("panic=abort")
        .arg("-o")
        .arg(&output)
        .arg(&runtime_source)
        .status()
        .expect("failed to invoke rustc for doria-rt");
    assert!(status.success(), "failed to build doria-rt static library");

    println!("cargo:rustc-env=DORIA_RT_BUILT_PATH={}", output.display());

    // Identity for the archive just produced. Without this the compiler can
    // only compare paths and modification times, which is how a stale archive
    // from an unrelated build previously shadowed the correct runtime and
    // failed the link with an unexplained missing symbol.
    let archive = std::fs::read(&output)
        .unwrap_or_else(|error| panic!("runtime archive at {}: {error}", output.display()));
    let metadata_path = runtime_metadata_path(&output);
    let document = runtime_metadata_document(
        &runtime_abi_version,
        &build_commit,
        &target,
        &profile,
        archive.len(),
        &sha256_hex(&archive),
    );
    std::fs::write(&metadata_path, document).unwrap_or_else(|error| {
        panic!(
            "runtime metadata sidecar at {}: {error}",
            metadata_path.display()
        )
    });
    println!(
        "cargo:rustc-env=DORIA_RT_BUILT_METADATA_PATH={}",
        metadata_path.display()
    );
}

/// Sidecar path for a runtime archive: the archive path plus a fixed suffix, so
/// the two travel together and an archive copied without its metadata is
/// detectably unidentified rather than silently trusted.
fn runtime_metadata_path(archive: &Path) -> PathBuf {
    let mut name = archive.file_name().unwrap_or_default().to_os_string();
    name.push(".doria-runtime.json");
    archive.with_file_name(name)
}

fn runtime_metadata_document(
    abi_version: &str,
    runtime_revision: &str,
    target_triple: &str,
    profile: &str,
    bytes: usize,
    sha256: &str,
) -> String {
    format!(
        concat!(
            "{{\n",
            "  \"schemaVersion\": {},\n",
            "  \"abiVersion\": \"{}\",\n",
            "  \"runtimeRevision\": \"{}\",\n",
            "  \"targetTriple\": \"{}\",\n",
            "  \"profile\": \"{}\",\n",
            "  \"featureSet\": [\"standalone-windows-support\"],\n",
            "  \"bytes\": {},\n",
            "  \"sha256\": \"{}\"\n",
            "}}\n"
        ),
        RUNTIME_METADATA_SCHEMA_VERSION,
        escape_json(abi_version),
        escape_json(runtime_revision),
        escape_json(target_triple),
        escape_json(profile),
        bytes,
        escape_json(sha256),
    )
}

/// The values written here are commit ids, target triples, profile names, and
/// hex digests, but they arrive from the environment and are escaped rather
/// than trusted to stay well behaved.
fn escape_json(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if (character as u32) < 0x20 => {
                escaped.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped
}

fn find_rlib(dependency_dir: &Path, crate_name: &str) -> PathBuf {
    let prefix = format!("lib{crate_name}-");
    std::fs::read_dir(dependency_dir)
        .expect("Cargo dependency directory")
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, entry.path()))
        })
        .filter(|(_, path)| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".rlib"))
        })
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
        .unwrap_or_else(|| panic!("compiled {crate_name} dependency"))
}

fn watch_git_identity(repository_dir: &Path) {
    let mut identities = vec!["HEAD".to_string()];
    if let Some(reference) = git_output(repository_dir, &["symbolic-ref", "-q", "HEAD"]) {
        identities.push(reference);
    }
    for identity in identities {
        let Some(path) = git_output(
            repository_dir,
            &["rev-parse", "--git-path", identity.as_str()],
        ) else {
            continue;
        };
        let path = PathBuf::from(path);
        let path = if path.is_absolute() {
            path
        } else {
            repository_dir.join(path)
        };
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

fn git_output(repository_dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository_dir)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn canonical_toolchain_version(package_version: &str) -> String {
    let mut components = package_version.splitn(3, '.');
    let year = components.next().expect("toolchain version year");
    let month = components
        .next()
        .expect("toolchain version month")
        .parse::<u8>()
        .expect("numeric toolchain version month");
    let release = components.next().expect("toolchain version release");
    assert!(
        year.len() == 4 && year.bytes().all(|byte| byte.is_ascii_digit()),
        "toolchain version year must use four digits"
    );
    assert!(
        (1..=12).contains(&month),
        "toolchain version month must be between 1 and 12"
    );
    let release_number = release
        .split_once('-')
        .map_or(release, |(number, _)| number);
    assert!(
        !release_number.is_empty() && release_number.bytes().all(|byte| byte.is_ascii_digit()),
        "toolchain release number must be numeric"
    );
    format!("{year}.{month:02}.{release}")
}
