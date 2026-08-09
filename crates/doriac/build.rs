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
    let runtime_manifest = runtime_dir.join("Cargo.toml");
    println!("cargo:rerun-if-changed={}", runtime_source_dir.display());
    println!("cargo:rerun-if-changed={}", runtime_manifest.display());

    // The bundled archive is now built from `doria-rt`'s whole dependency graph,
    // so every input to that graph has to retrigger this script. Watching only
    // the runtime's own sources would leave a stale archive bundled after an edit
    // to a crate the runtime links, which is the same class of mistake as
    // selecting a stale rlib.
    for sibling in ["doria-unicode", "doria-diagnostic-catalogue"] {
        println!(
            "cargo:rerun-if-changed={}",
            manifest_dir.join("..").join(sibling).join("src").display()
        );
        println!(
            "cargo:rerun-if-changed={}",
            manifest_dir
                .join("..")
                .join(sibling)
                .join("Cargo.toml")
                .display()
        );
    }
    println!(
        "cargo:rerun-if-changed={}",
        repository_dir.join("Cargo.lock").display()
    );

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
    println!("cargo:rustc-env=DORIA_RT_EXPECTED_TARGET={target}");
    println!("cargo:rustc-env=DORIA_RT_EXPECTED_REVISION={build_commit}");

    if env::var_os("CARGO_FEATURE_BUNDLED_RUNTIME").is_none() {
        return;
    }

    let filename = if target.ends_with("windows-msvc") {
        "doria_rt.lib"
    } else {
        "libdoria_rt.a"
    };
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"));

    // Both native profiles are built by cargo from `doria-rt`'s own manifest,
    // rather than by invoking `rustc` here with hand-picked dependency rlibs.
    // The profile used to build doriac cannot decide which runtime a later
    // `doriac compile --release` invocation needs, so a compiler carries one
    // identified archive for each selectable Doria native profile.
    //
    // The previous approach could not be made correct. A build script has no
    // supported way to depend on, order, or locate its own package's *normal*
    // dependencies: the `[build-dependencies]` that existed to make the runtime's
    // externs "ready" only ever ordered the **host** copies of those crates,
    // which cargo builds with the build-override profile — `opt-level=0` and
    // `panic=unwind`. Cargo therefore left two rlibs for every runtime
    // dependency in one directory, differing in optimisation level, panic
    // strategy, and feature set, and the selector picked between them by
    // modification time. Which runtime a compiler bundled was decided by which
    // rlib cargo happened to finish writing last: a scheduling race, not a build
    // input. Picking the host copies produced an 18.0 MB archive whose
    // dependencies were entirely unoptimised; picking the target copies produced
    // the correct 12.7 MB one.
    //
    // Letting cargo resolve the runtime removes the guess rather than improving
    // it. Cargo knows the features `doria-rt` declares and the explicitly
    // requested profile, and it writes each archive to exactly one path.
    let runtime_target_dir = out_dir.join("doria-rt-target");
    let cargo = env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    for profile in ["debug", "release"] {
        let output = out_dir.join("doria-rt").join(profile).join(filename);
        build_runtime_profile(
            &cargo,
            &runtime_manifest,
            &runtime_target_dir,
            &target,
            profile,
            filename,
            &output,
            &runtime_abi_version,
            &build_commit,
        );
        println!(
            "cargo:rustc-env=DORIA_RT_BUILT_{}_PATH={}",
            profile.to_ascii_uppercase(),
            output.display()
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn build_runtime_profile(
    cargo: &OsString,
    runtime_manifest: &Path,
    runtime_target_dir: &Path,
    target: &str,
    profile: &str,
    filename: &str,
    output: &Path,
    runtime_abi_version: &str,
    build_commit: &str,
) {
    let mut command = Command::new(cargo);
    command
        .arg("build")
        .arg("--manifest-path")
        .arg(runtime_manifest)
        .arg("--package")
        .arg("doria-rt")
        .arg("--locked")
        .arg("--target")
        .arg(target)
        // A nested cargo must not share the target directory of the build that
        // invoked it: cargo takes an exclusive lock on it and the two would
        // deadlock waiting for each other.
        .env("CARGO_TARGET_DIR", runtime_target_dir)
        // `cargo clippy` sets this to `clippy-driver` for workspace members, and
        // `doria-rt` is a workspace member. Inherited, it builds the runtime
        // through a lint driver and yields a different archive than `cargo
        // build` does from the same revision — measured, not feared. Nobody
        // asked for the shipped runtime to depend on which cargo subcommand ran
        // last, so the wrapper is dropped. `RUSTC_WRAPPER` is deliberately kept:
        // it is set by the operator, applies to every unit alike, and is how
        // caches such as sccache are configured.
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        // Incremental compilation makes the archive nondeterministic: two dev
        // builds of one revision, from scratch, in one directory, produce
        // different bytes. It exists to speed up editing the code being built,
        // which is not what this is — the runtime is an artifact whose digest is
        // recorded and compared. Release was already unaffected because cargo
        // leaves incremental off there; this extends the guarantee to every
        // profile rather than to the one that happened to be safe.
        .env("CARGO_INCREMENTAL", "0");
    if profile == "release" {
        command.arg("--release");
    }
    if env::var("CARGO_NET_OFFLINE").is_ok_and(|value| value == "true") {
        command.arg("--offline");
    }
    let status = command
        .status()
        .expect("failed to invoke cargo for doria-rt");
    assert!(status.success(), "failed to build doria-rt static library");

    // Exactly one path, derived from what was asked for rather than searched
    // for. If cargo did not put the archive here, that is a fact worth failing
    // on: the alternative is to go looking, which is the defect this replaced.
    let built = runtime_target_dir.join(target).join(profile).join(filename);
    assert!(
        built.is_file(),
        "cargo did not produce the doria-rt static library at {}",
        built.display()
    );
    std::fs::create_dir_all(
        output
            .parent()
            .expect("bundled runtime output must have a parent"),
    )
    .unwrap_or_else(|error| {
        panic!(
            "creating bundled runtime directory for {}: {error}",
            output.display()
        )
    });
    std::fs::copy(&built, output).unwrap_or_else(|error| {
        panic!(
            "copying runtime archive {} to {}: {error}",
            built.display(),
            output.display()
        )
    });

    // Identity for the archive just produced. Without this the compiler can
    // only compare paths and modification times, which is how a stale archive
    // from an unrelated build previously shadowed the correct runtime and
    // failed the link with an unexplained missing symbol.
    let archive = std::fs::read(output)
        .unwrap_or_else(|error| panic!("runtime archive at {}: {error}", output.display()));
    let metadata_path = runtime_metadata_path(output);
    let document = runtime_metadata_document(
        runtime_abi_version,
        build_commit,
        target,
        profile,
        archive.len(),
        &sha256_hex(&archive),
    );
    std::fs::write(&metadata_path, document).unwrap_or_else(|error| {
        panic!(
            "runtime metadata sidecar at {}: {error}",
            metadata_path.display()
        )
    });
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
