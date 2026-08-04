use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::backend::{BackendError, NativeProfile};
use crate::diagnostics::Diagnostic;
use crate::source::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArchiveFormat {
    Gnu,
    Msvc,
}

/// Where a selected runtime archive came from.
///
/// The origin is recorded rather than inferred later, because "which archive
/// did this executable link" is the question benchmark provenance has to answer
/// and a path alone does not answer it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeOrigin {
    /// `DORIA_RT_PATH`, set deliberately by an operator or a tool.
    ExplicitOverride,
    /// Built and bundled by this compiler's own build script.
    CompilerBundled,
    /// Shipped beside an installed `doriac`.
    InstalledToolchain,
    /// An ambient archive in a workspace target directory.
    Workspace,
}

impl RuntimeOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitOverride => "explicit-override",
            Self::CompilerBundled => "compiler-bundled",
            Self::InstalledToolchain => "installed-development-toolchain",
            Self::Workspace => "workspace",
        }
    }

    /// Whether an archive from this origin was chosen on purpose.
    ///
    /// A deliberate origin may be used without identity metadata, because
    /// somebody selected it. An ambient one may not: that is precisely how a
    /// stale archive from an unrelated build previously won.
    const fn is_deliberate(self) -> bool {
        matches!(self, Self::ExplicitOverride | Self::CompilerBundled)
    }
}

/// Identity recorded beside a runtime archive by the build script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeMetadata {
    pub schema_version: u32,
    pub abi_version: String,
    pub runtime_revision: String,
    pub target_triple: String,
    pub profile: String,
    pub bytes: u64,
    pub sha256: String,
}

/// What this compiler build requires of a runtime archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeExpectation {
    pub abi_version: String,
    pub target_triple: String,
    pub profile: String,
    pub revision: String,
}

impl RuntimeExpectation {
    /// The expectation baked in when this compiler was built.
    ///
    /// Both native profiles share one expectation because both link the same
    /// bundled archive. Cranelift and LLVM deliberately do not get separate
    /// identity rules.
    pub fn current() -> Self {
        Self {
            abi_version: option_env!("DORIA_RT_ABI_VERSION")
                .unwrap_or("")
                .to_string(),
            target_triple: option_env!("DORIA_RT_EXPECTED_TARGET")
                .unwrap_or("")
                .to_string(),
            profile: option_env!("DORIA_RT_EXPECTED_PROFILE")
                .unwrap_or("")
                .to_string(),
            revision: option_env!("DORIA_RT_EXPECTED_REVISION")
                .unwrap_or("")
                .to_string(),
        }
    }
}

/// A runtime archive the compiler has selected and, where identity was
/// available, verified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeArtifact {
    pub path: PathBuf,
    pub origin: RuntimeOrigin,
    pub metadata: Option<RuntimeMetadata>,
}

pub fn locate(profile: NativeProfile) -> Result<RuntimeArtifact, BackendError> {
    let current_executable = env::current_exe().map_err(|error| {
        BackendError::new(format!(
            "doria-rt static library was not found: failed to locate doriac: {error}\nhelp: build it with `cargo build -p doria-rt` or set DORIA_RT_PATH"
        ))
    })?;
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("doriac must live under the workspace crates directory");
    let target_override = env::var_os("CARGO_TARGET_DIR");
    resolve(
        env::var_os("DORIA_RT_PATH").as_deref(),
        &current_executable,
        option_env!("DORIA_RT_BUILT_PATH").map(Path::new),
        workspace,
        target_override.as_deref(),
        if cfg!(all(windows, target_env = "msvc")) {
            ArchiveFormat::Msvc
        } else {
            ArchiveFormat::Gnu
        },
        profile_directory(profile),
        &RuntimeExpectation::current(),
    )
}

const fn profile_directory(profile: NativeProfile) -> &'static str {
    match profile {
        NativeProfile::Fast => "debug",
        NativeProfile::Release => "release",
    }
}

/// Candidate archives in priority order.
///
/// The compiler-owned archive is preferred on every profile. An earlier version
/// of this function pushed the ambient workspace `target/release` archive ahead
/// of it for the release profile only, which let a stale file from an unrelated
/// build shadow the correct runtime and fail the link with an unexplained
/// missing symbol. Worse, it allowed a release build to link a runtime produced
/// from a different compiler revision than the one generating the code.
fn candidates(
    current_executable: &Path,
    compiler_built_runtime: Option<&Path>,
    workspace: &Path,
    target_override: Option<&OsStr>,
    filename: &str,
    profile: &str,
) -> Vec<(PathBuf, RuntimeOrigin)> {
    let target_root = target_override.map_or_else(
        || workspace.join("target"),
        |target| {
            let target = PathBuf::from(target);
            if target.is_absolute() {
                target
            } else {
                workspace.join(target)
            }
        },
    );
    let mut candidates = Vec::new();
    if let Some(compiler_built_runtime) = compiler_built_runtime {
        candidates.push((
            compiler_built_runtime.to_path_buf(),
            RuntimeOrigin::CompilerBundled,
        ));
    }
    if let Some(parent) = current_executable.parent() {
        candidates.push((parent.join(filename), RuntimeOrigin::InstalledToolchain));
        candidates.push((
            parent.join("../lib/doria").join(filename),
            RuntimeOrigin::InstalledToolchain,
        ));
        if let Some(profile_directory) = parent.parent() {
            candidates.push((
                profile_directory.join(filename),
                RuntimeOrigin::InstalledToolchain,
            ));
        }
    }
    candidates.push((
        target_root.join(profile).join(filename),
        RuntimeOrigin::Workspace,
    ));
    let alternate_profile = if profile == "debug" {
        "release"
    } else {
        "debug"
    };
    candidates.push((
        target_root.join(alternate_profile).join(filename),
        RuntimeOrigin::Workspace,
    ));
    candidates
}

#[allow(clippy::too_many_arguments)]
fn resolve(
    explicit: Option<&OsStr>,
    current_executable: &Path,
    compiler_built_runtime: Option<&Path>,
    workspace: &Path,
    target_override: Option<&OsStr>,
    archive_format: ArchiveFormat,
    profile: &str,
    expectation: &RuntimeExpectation,
) -> Result<RuntimeArtifact, BackendError> {
    let filename = runtime_filename(archive_format);

    if let Some(explicit) = explicit {
        let explicit = PathBuf::from(explicit);
        let candidate = if explicit.is_dir() {
            explicit.join(filename)
        } else {
            explicit
        };
        if !candidate.is_file() {
            return Err(not_found_error(Some(&candidate)));
        }
        return accept(candidate, RuntimeOrigin::ExplicitOverride, expectation);
    }

    for (candidate, origin) in candidates(
        current_executable,
        compiler_built_runtime,
        workspace,
        target_override,
        filename,
        profile,
    ) {
        if !candidate.is_file() {
            continue;
        }
        let metadata = read_metadata(&candidate);
        // An ambient archive with no identity never outranks the
        // compiler-owned runtime; it is skipped rather than trusted.
        if metadata.is_none() && !origin.is_deliberate() {
            continue;
        }
        return accept(candidate, origin, expectation);
    }
    Err(not_found_error(None))
}

/// Validate a candidate and take it, or fail.
///
/// There is no fallback after a validation failure. Continuing down the
/// candidate list after finding an incompatible archive would reintroduce the
/// original defect in a quieter form: the compiler would silently link
/// something else instead of reporting the mismatch it just detected.
fn accept(
    path: PathBuf,
    origin: RuntimeOrigin,
    expectation: &RuntimeExpectation,
) -> Result<RuntimeArtifact, BackendError> {
    let metadata = read_metadata(&path);
    if let Some(metadata) = &metadata {
        if let Some(mismatch) = incompatibility(metadata, expectation) {
            return Err(incompatible_runtime_error(&path, origin, &mismatch));
        }
    }
    Ok(RuntimeArtifact {
        path,
        origin,
        metadata,
    })
}

/// Fields whose disagreement makes an archive unusable, in report order.
///
/// A blank expectation means this compiler was built without that fact and
/// cannot judge it; it is skipped rather than treated as a mismatch, so a
/// compiler built outside the normal build script does not reject every
/// runtime.
fn incompatibility(
    metadata: &RuntimeMetadata,
    expectation: &RuntimeExpectation,
) -> Option<Vec<Mismatch>> {
    let comparisons = [
        ("ABI", &expectation.abi_version, &metadata.abi_version),
        (
            "target",
            &expectation.target_triple,
            &metadata.target_triple,
        ),
        ("profile", &expectation.profile, &metadata.profile),
        (
            "revision",
            &expectation.revision,
            &metadata.runtime_revision,
        ),
    ];
    let mismatches: Vec<Mismatch> = comparisons
        .into_iter()
        .filter(|(_, expected, _)| !expected.is_empty())
        .filter(|(_, expected, found)| expected != found)
        .map(|(field, expected, found)| Mismatch {
            field,
            expected: expected.clone(),
            found: found.clone(),
        })
        .collect();
    (!mismatches.is_empty()).then_some(mismatches)
}

#[derive(Debug, Clone)]
struct Mismatch {
    field: &'static str,
    expected: String,
    found: String,
}

fn incompatible_runtime_error(
    path: &Path,
    origin: RuntimeOrigin,
    mismatches: &[Mismatch],
) -> BackendError {
    let mut message = format!(
        "the selected runtime artifact does not match this compiler\nselected runtime: {}\norigin: {}",
        path.display(),
        origin.as_str()
    );
    for mismatch in mismatches {
        message.push_str(&format!(
            "\nexpected {}: {}\nfound {}: {}",
            mismatch.field, mismatch.expected, mismatch.field, mismatch.found
        ));
    }
    let summary = mismatches
        .iter()
        .map(|mismatch| mismatch.field)
        .collect::<Vec<_>>()
        .join(", ");
    BackendError::from_diagnostics(vec![Diagnostic::new("B0003", message, Span::default())
        .with_note(format!(
            "the runtime artifact disagrees with this compiler on: {summary}"
        ))
        .with_help(
            "rebuild the compiler so it bundles a matching runtime, or set DORIA_RT_PATH to an archive built from this revision",
        )])
}

/// Read the sidecar written beside an archive by the build script.
///
/// A missing or unreadable sidecar is absence of identity, not an error: some
/// archives legitimately predate the sidecar. Callers decide what absence
/// means, and an ambient archive without identity is skipped rather than used.
fn read_metadata(archive: &Path) -> Option<RuntimeMetadata> {
    let raw = std::fs::read_to_string(metadata_path(archive)).ok()?;
    parse_metadata(&raw)
}

pub fn metadata_path(archive: &Path) -> PathBuf {
    let mut name = archive.file_name().unwrap_or_default().to_os_string();
    name.push(".doria-runtime.json");
    archive.with_file_name(name)
}

/// Parse the flat sidecar document.
///
/// Deliberately minimal: the document is written by this repository's build
/// script with a fixed shape, and a full JSON reader is not worth adding to the
/// compiler for it. Anything unexpected yields `None`, which is treated as
/// absent identity rather than as a compatible archive.
fn parse_metadata(raw: &str) -> Option<RuntimeMetadata> {
    let schema_version = field(raw, "schemaVersion")?.parse().ok()?;
    let bytes = field(raw, "bytes")?.parse().ok()?;
    Some(RuntimeMetadata {
        schema_version,
        abi_version: field(raw, "abiVersion")?,
        runtime_revision: field(raw, "runtimeRevision")?,
        target_triple: field(raw, "targetTriple")?,
        profile: field(raw, "profile")?,
        bytes,
        sha256: field(raw, "sha256")?,
    })
}

fn field(raw: &str, name: &str) -> Option<String> {
    let key = format!("\"{name}\"");
    let start = raw.find(&key)? + key.len();
    let rest = raw.get(start..)?.trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    if let Some(quoted) = rest.strip_prefix('"') {
        let end = quoted.find('"')?;
        return Some(quoted[..end].to_string());
    }
    let end = rest
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(rest.len());
    (end > 0).then(|| rest[..end].to_string())
}

fn runtime_filename(archive_format: ArchiveFormat) -> &'static str {
    match archive_format {
        ArchiveFormat::Gnu => "libdoria_rt.a",
        ArchiveFormat::Msvc => "doria_rt.lib",
    }
}

fn not_found_error(path: Option<&Path>) -> BackendError {
    let detail = path
        .map(|path| format!(" at `{}`", path.display()))
        .unwrap_or_default();
    BackendError::new(format!(
        "doria-rt static library was not found{detail}\nhelp: build it with `cargo build -p doria-rt` or set DORIA_RT_PATH"
    ))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temp_directory(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "doriac-runtime-artifact-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("temporary directory should be created");
        path
    }

    fn expectation() -> RuntimeExpectation {
        RuntimeExpectation {
            abi_version: "1".to_string(),
            target_triple: "aarch64-apple-darwin".to_string(),
            profile: "release".to_string(),
            revision: "cafebabe".to_string(),
        }
    }

    /// Write an archive plus a sidecar, overriding chosen identity fields.
    fn write_archive(path: &Path, overrides: &[(&str, &str)]) {
        fs::create_dir_all(path.parent().expect("archive should have a parent"))
            .expect("archive directory should be created");
        fs::write(path, b"archive").expect("archive fixture should be written");
        let expectation = expectation();
        let mut abi = expectation.abi_version;
        let mut target = expectation.target_triple;
        let mut profile = expectation.profile;
        let mut revision = expectation.revision;
        for (field, value) in overrides {
            match *field {
                "abiVersion" => abi = (*value).to_string(),
                "targetTriple" => target = (*value).to_string(),
                "profile" => profile = (*value).to_string(),
                "runtimeRevision" => revision = (*value).to_string(),
                other => panic!("unknown override {other}"),
            }
        }
        let document = format!(
            "{{\n  \"schemaVersion\": 1,\n  \"abiVersion\": \"{abi}\",\n  \"runtimeRevision\": \"{revision}\",\n  \"targetTriple\": \"{target}\",\n  \"profile\": \"{profile}\",\n  \"featureSet\": [\"standalone-windows-support\"],\n  \"bytes\": 7,\n  \"sha256\": \"abc123\"\n}}\n"
        );
        fs::write(metadata_path(path), document).expect("sidecar should be written");
    }

    /// An archive with no sidecar at all.
    fn write_unidentified_archive(path: &Path) {
        fs::create_dir_all(path.parent().expect("archive should have a parent"))
            .expect("archive directory should be created");
        fs::write(path, b"archive").expect("archive fixture should be written");
    }

    fn resolve_in(
        directory: &Path,
        explicit: Option<&OsStr>,
        compiler_built: Option<&Path>,
        profile: &str,
    ) -> Result<RuntimeArtifact, BackendError> {
        resolve(
            explicit,
            &directory.join("bin/doriac"),
            compiler_built,
            directory,
            None,
            ArchiveFormat::Gnu,
            profile,
            &expectation(),
        )
    }

    #[test]
    fn compiler_owned_runtime_wins_on_the_release_profile() {
        // The original defect: an ambient workspace release archive outranked
        // the compiler's own runtime, so a stale file broke the link.
        let directory = temp_directory("release-compiler-owned");
        let compiler_built = directory.join("build/libdoria_rt.a");
        let workspace = directory.join("target/release/libdoria_rt.a");
        write_archive(&compiler_built, &[]);
        write_archive(&workspace, &[("runtimeRevision", "stale")]);

        let resolved = resolve_in(&directory, None, Some(&compiler_built), "release")
            .expect("compiler-owned runtime should resolve");
        assert_eq!(resolved.path, compiler_built);
        assert_eq!(resolved.origin, RuntimeOrigin::CompilerBundled);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn compiler_owned_runtime_wins_on_the_development_profile() {
        let directory = temp_directory("debug-compiler-owned");
        let compiler_built = directory.join("build/libdoria_rt.a");
        let workspace = directory.join("target/debug/libdoria_rt.a");
        write_archive(&compiler_built, &[]);
        write_archive(&workspace, &[("runtimeRevision", "stale")]);

        let resolved = resolve_in(&directory, None, Some(&compiler_built), "debug")
            .expect("compiler-owned runtime should resolve");
        assert_eq!(resolved.path, compiler_built);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn stale_workspace_release_runtime_does_not_win() {
        let directory = temp_directory("stale-release");
        let workspace = directory.join("target/release/libdoria_rt.a");
        write_archive(&workspace, &[("runtimeRevision", "stale")]);

        let error = resolve_in(&directory, None, None, "release")
            .expect_err("a stale workspace runtime must not be selected silently");
        let diagnostics = error.diagnostics.expect("mismatch should be structured");
        assert_eq!(diagnostics[0].code, "B0003");
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn stale_workspace_development_runtime_does_not_win() {
        let directory = temp_directory("stale-debug");
        let workspace = directory.join("target/debug/libdoria_rt.a");
        write_archive(&workspace, &[("runtimeRevision", "stale")]);

        let error = resolve_in(&directory, None, None, "debug")
            .expect_err("a stale workspace runtime must not be selected silently");
        assert_eq!(
            error
                .diagnostics
                .expect("mismatch should be structured")
                .first()
                .expect("one diagnostic")
                .code,
            "B0003"
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn wrong_abi_is_rejected() {
        let directory = temp_directory("abi");
        let compiler_built = directory.join("build/libdoria_rt.a");
        write_archive(&compiler_built, &[("abiVersion", "99")]);
        let error = resolve_in(&directory, None, Some(&compiler_built), "release")
            .expect_err("an ABI mismatch must be rejected");
        assert!(error.message.contains("expected ABI: 1"));
        assert!(error.message.contains("found ABI: 99"));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn wrong_target_is_rejected() {
        let directory = temp_directory("target");
        let compiler_built = directory.join("build/libdoria_rt.a");
        write_archive(
            &compiler_built,
            &[("targetTriple", "x86_64-unknown-linux-gnu")],
        );
        let error = resolve_in(&directory, None, Some(&compiler_built), "release")
            .expect_err("a target mismatch must be rejected");
        assert!(error.message.contains("x86_64-unknown-linux-gnu"));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn wrong_profile_is_rejected() {
        let directory = temp_directory("profile");
        let compiler_built = directory.join("build/libdoria_rt.a");
        write_archive(&compiler_built, &[("profile", "debug")]);
        let error = resolve_in(&directory, None, Some(&compiler_built), "release")
            .expect_err("a profile mismatch must be rejected");
        assert!(error.message.contains("expected profile: release"));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn wrong_revision_is_rejected() {
        let directory = temp_directory("revision");
        let compiler_built = directory.join("build/libdoria_rt.a");
        write_archive(&compiler_built, &[("runtimeRevision", "deadbeef")]);
        let error = resolve_in(&directory, None, Some(&compiler_built), "release")
            .expect_err("a revision mismatch must be rejected");
        assert!(error.message.contains("deadbeef"));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn an_unidentified_ambient_archive_is_skipped() {
        // Absence of identity is not evidence of compatibility. An ambient
        // archive without a sidecar is passed over entirely.
        let directory = temp_directory("unidentified-ambient");
        write_unidentified_archive(&directory.join("target/release/libdoria_rt.a"));
        let error = resolve_in(&directory, None, None, "release")
            .expect_err("an unidentified ambient archive must not be selected");
        assert!(error
            .message
            .contains("doria-rt static library was not found"));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn an_unidentified_compiler_owned_archive_is_accepted_without_identity() {
        // A compiler-owned archive predating the sidecar is still ours. It is
        // accepted, but reports no metadata so provenance can mark evidence
        // produced with it as unidentified.
        let directory = temp_directory("unidentified-owned");
        let compiler_built = directory.join("build/libdoria_rt.a");
        write_unidentified_archive(&compiler_built);
        let resolved = resolve_in(&directory, None, Some(&compiler_built), "release")
            .expect("a compiler-owned archive should resolve");
        assert_eq!(resolved.origin, RuntimeOrigin::CompilerBundled);
        assert!(resolved.metadata.is_none());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn an_explicit_override_is_validated() {
        let directory = temp_directory("override-validated");
        let override_archive = directory.join("elsewhere/libdoria_rt.a");
        write_archive(&override_archive, &[("abiVersion", "42")]);
        let error = resolve_in(
            &directory,
            Some(override_archive.as_os_str()),
            None,
            "release",
        )
        .expect_err("an explicit override must still be validated");
        assert!(error.message.contains("explicit-override"));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn a_matching_explicit_override_is_accepted() {
        let directory = temp_directory("override-accepted");
        let override_archive = directory.join("elsewhere/libdoria_rt.a");
        write_archive(&override_archive, &[]);
        let resolved = resolve_in(
            &directory,
            Some(override_archive.as_os_str()),
            None,
            "release",
        )
        .expect("a matching override should resolve");
        assert_eq!(resolved.origin, RuntimeOrigin::ExplicitOverride);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn no_silent_fallback_after_a_validation_failure() {
        // A good archive sits later in the candidate list. The incompatible
        // compiler-owned archive must still fail rather than quietly yielding
        // to it, or the mismatch becomes invisible again.
        let directory = temp_directory("no-fallback");
        let compiler_built = directory.join("build/libdoria_rt.a");
        let workspace = directory.join("target/release/libdoria_rt.a");
        write_archive(&compiler_built, &[("abiVersion", "99")]);
        write_archive(&workspace, &[]);

        let error = resolve_in(&directory, None, Some(&compiler_built), "release")
            .expect_err("validation failure must not fall through to another archive");
        assert!(error.message.contains("build/libdoria_rt.a"));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn a_blank_expectation_cannot_reject_a_runtime() {
        // A compiler built outside the normal build script knows nothing about
        // the ABI and must not reject every archive on that basis.
        let directory = temp_directory("blank-expectation");
        let compiler_built = directory.join("build/libdoria_rt.a");
        write_archive(&compiler_built, &[("abiVersion", "77")]);
        let resolved = resolve(
            None,
            &directory.join("bin/doriac"),
            Some(&compiler_built),
            &directory,
            None,
            ArchiveFormat::Gnu,
            "release",
            &RuntimeExpectation {
                abi_version: String::new(),
                target_triple: String::new(),
                profile: String::new(),
                revision: String::new(),
            },
        )
        .expect("a compiler without expectations should not reject a runtime");
        assert_eq!(resolved.path, compiler_built);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn missing_runtime_has_build_help() {
        let directory = temp_directory("missing");
        let error =
            resolve_in(&directory, None, None, "debug").expect_err("missing runtime should fail");
        assert!(error
            .message
            .contains("doria-rt static library was not found"));
        assert!(error.message.contains("cargo build -p doria-rt"));
        assert!(error.message.contains("DORIA_RT_PATH"));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn a_missing_explicit_override_reports_its_path() {
        let directory = temp_directory("override-missing");
        let absent = directory.join("absent/libdoria_rt.a");
        let error = resolve_in(&directory, Some(absent.as_os_str()), None, "release")
            .expect_err("a missing override should fail");
        assert!(error.message.contains("absent"));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn runtime_archive_name_matches_the_rust_target_environment() {
        assert_eq!(runtime_filename(ArchiveFormat::Msvc), "doria_rt.lib");
        assert_eq!(runtime_filename(ArchiveFormat::Gnu), "libdoria_rt.a");
    }

    #[test]
    fn mingw_directory_override_uses_the_gnu_archive_name() {
        let directory = temp_directory("mingw");
        let runtime = directory.join("libdoria_rt.a");
        write_archive(&runtime, &[]);
        let resolved = resolve_in(&directory, Some(directory.as_os_str()), None, "debug")
            .expect("MinGW runtime should resolve");
        assert_eq!(resolved.path, runtime);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn an_installed_toolchain_runtime_resolves() {
        let directory = temp_directory("installed");
        let installed = directory.join("bin/libdoria_rt.a");
        write_archive(&installed, &[]);
        let resolved = resolve_in(&directory, None, None, "release")
            .expect("an installed toolchain runtime should resolve");
        assert_eq!(resolved.origin, RuntimeOrigin::InstalledToolchain);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn sidecar_documents_round_trip() {
        let parsed = parse_metadata(
            "{\n  \"schemaVersion\": 1,\n  \"abiVersion\": \"1\",\n  \"runtimeRevision\": \"abc\",\n  \"targetTriple\": \"t\",\n  \"profile\": \"release\",\n  \"featureSet\": [\"x\"],\n  \"bytes\": 42,\n  \"sha256\": \"ff\"\n}\n",
        )
        .expect("a well-formed sidecar should parse");
        assert_eq!(parsed.schema_version, 1);
        assert_eq!(parsed.bytes, 42);
        assert_eq!(parsed.sha256, "ff");
        assert_eq!(parsed.profile, "release");
        assert!(parse_metadata("not json").is_none());
    }
}
