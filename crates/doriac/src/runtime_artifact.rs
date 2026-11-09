use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::backend::{BackendError, NativeProfile};
use crate::diagnostics::Diagnostic;
use crate::runtime_digest::sha256_hex;
use crate::source::Span;

const RUNTIME_METADATA_SCHEMA_VERSION: u32 = 1;
const EMBEDDED_RUNTIME_CACHE_DIRECTORY: &str = "doria-embedded-runtime-v1";

#[derive(Debug, Clone, Copy)]
struct EmbeddedRuntimeProfile {
    archive: &'static [u8],
    metadata: &'static str,
}

#[derive(Debug, Clone, Copy)]
struct EmbeddedRuntimeSet {
    debug: EmbeddedRuntimeProfile,
    release: EmbeddedRuntimeProfile,
}

static EMBEDDED_RUNTIMES: OnceLock<EmbeddedRuntimeSet> = OnceLock::new();
static EMBEDDED_SHARED_RUNTIME: OnceLock<EmbeddedRuntimeProfile> = OnceLock::new();
static NEXT_EMBEDDED_RUNTIME_ID: AtomicU64 = AtomicU64::new(0);

/// Register the runtime archives compiled into the standalone `doriac` binary.
///
/// The library remains usable without embedded archives by the language server
/// and compiler tests. The executable registers both profiles at startup so an
/// installed compiler never retains a dependency on Cargo's reclaimable target
/// directory.
pub fn register_embedded_runtimes(
    debug_archive: &'static [u8],
    debug_metadata: &'static str,
    release_archive: &'static [u8],
    release_metadata: &'static str,
) {
    let _ = EMBEDDED_RUNTIMES.set(EmbeddedRuntimeSet {
        debug: EmbeddedRuntimeProfile {
            archive: debug_archive,
            metadata: debug_metadata,
        },
        release: EmbeddedRuntimeProfile {
            archive: release_archive,
            metadata: release_metadata,
        },
    });
}

pub fn register_embedded_shared_runtime(library: &'static [u8], metadata: &'static str) {
    let _ = EMBEDDED_SHARED_RUNTIME.set(EmbeddedRuntimeProfile {
        archive: library,
        metadata,
    });
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeArtifactKind {
    #[default]
    Static,
    Shared,
}

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeMetadata {
    pub schema_version: u32,
    pub abi_version: String,
    pub runtime_revision: String,
    pub target_triple: String,
    pub profile: String,
    #[serde(default)]
    pub artifact_kind: RuntimeArtifactKind,
    #[serde(default)]
    pub feature_set: Vec<String>,
    pub bytes: u64,
    pub sha256: String,
}

/// Verified bytes and their portable publication location, never a build-cache reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedRuntimeDependency {
    metadata: RuntimeMetadata,
    metadata_document: String,
    library: Vec<u8>,
    relative_library_path: PathBuf,
}

impl SharedRuntimeDependency {
    pub fn metadata(&self) -> &RuntimeMetadata {
        &self.metadata
    }

    pub fn metadata_json(&self) -> &str {
        &self.metadata_document
    }

    pub fn relative_library_path(&self) -> &Path {
        &self.relative_library_path
    }

    pub fn relative_metadata_path(&self) -> PathBuf {
        metadata_path(&self.relative_library_path)
    }

    pub fn publish(&self, output_directory: &Path) -> Result<(), BackendError> {
        let library = materialize_runtime_asset(
            &self.library,
            &self.metadata_document,
            shared_runtime_filename(),
            &output_directory.join(".doria-runtime"),
            false,
        )?;
        if library != output_directory.join(&self.relative_library_path) {
            return Err(BackendError::new(
                "shared runtime publication changed its verified relative path",
            ));
        }
        Ok(())
    }
}

/// Shared artifacts are selected only from an explicit override or this compiler's own build.
pub fn shared_runtime_dependency() -> Result<SharedRuntimeDependency, BackendError> {
    if let Some(override_path) = env::var_os("DORIA_RT_PATH") {
        let path = PathBuf::from(override_path);
        let path = if path.is_dir() {
            path.join(shared_runtime_filename())
        } else {
            path
        };
        return read_shared_runtime(&path, RuntimeOrigin::ExplicitOverride);
    }
    if let Some(embedded) = EMBEDDED_SHARED_RUNTIME.get() {
        return verify_shared_runtime(
            Path::new("<compiler-bundled shared runtime>"),
            RuntimeOrigin::CompilerBundled,
            embedded.archive.to_vec(),
            embedded.metadata.to_string(),
            &RuntimeExpectation::current(NativeProfile::Release),
        );
    }
    let path = option_env!("DORIA_RT_BUILT_SHARED_PATH").ok_or_else(|| BackendError::new(
        "this compiler does not contain a shared Doria runtime; rebuild it with bundled-runtime support to publish native-backed PHP output",
    ))?;
    read_shared_runtime(Path::new(path), RuntimeOrigin::CompilerBundled)
}

fn read_shared_runtime(
    path: &Path,
    origin: RuntimeOrigin,
) -> Result<SharedRuntimeDependency, BackendError> {
    let library = fs::read(path).map_err(|error| {
        invalid_metadata_error(
            path,
            origin,
            &format!("shared library unavailable: {error}"),
        )
    })?;
    let document = fs::read_to_string(metadata_path(path)).map_err(|error| {
        invalid_metadata_error(
            path,
            origin,
            &format!("shared library identity sidecar unavailable: {error}"),
        )
    })?;
    verify_shared_runtime(
        path,
        origin,
        library,
        document,
        &RuntimeExpectation::current(NativeProfile::Release),
    )
}

fn verify_shared_runtime(
    path: &Path,
    origin: RuntimeOrigin,
    library: Vec<u8>,
    document: String,
    expectation: &RuntimeExpectation,
) -> Result<SharedRuntimeDependency, BackendError> {
    let metadata = parse_metadata(&document).ok_or_else(|| {
        invalid_metadata_error(
            path,
            origin,
            "shared library metadata is malformed or incomplete",
        )
    })?;
    let mut mismatches = incompatibility(&metadata, expectation).unwrap_or_default();
    if metadata.artifact_kind != RuntimeArtifactKind::Shared {
        mismatches.push(Mismatch {
            field: "artifact kind",
            expected: "shared".into(),
            found: "static".into(),
        });
    }
    if metadata.feature_set != ["hosted-shared-runtime"] {
        mismatches.push(Mismatch {
            field: "feature set",
            expected: "hosted-shared-runtime".into(),
            found: metadata.feature_set.join(", "),
        });
    }
    for (field, value) in [
        ("ABI", &expectation.abi_version),
        ("target", &expectation.target_triple),
        ("revision", &expectation.revision),
    ] {
        if value.is_empty() || value == "unknown" {
            return Err(invalid_metadata_error(
                path,
                origin,
                &format!("compiler has no verified {field} expectation for a shared runtime"),
            ));
        }
    }
    mismatches.extend(integrity_mismatches(&library, &metadata));
    if !mismatches.is_empty() {
        return Err(incompatible_runtime_error(path, origin, &mismatches));
    }
    // Integrity passed, so the recorded digest is the library's own.
    let identity = runtime_cache_identity(&metadata.sha256, &document);
    Ok(SharedRuntimeDependency {
        metadata,
        metadata_document: document,
        library,
        relative_library_path: Path::new(".doria-runtime")
            .join(identity)
            .join(shared_runtime_filename()),
    })
}

fn shared_runtime_filename() -> &'static str {
    if cfg!(windows) {
        "doria_rt.dll"
    } else if cfg!(target_os = "macos") {
        "libdoria_rt.dylib"
    } else {
        "libdoria_rt.so"
    }
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
    /// The identity this compiler requires for one requested native profile.
    /// Cranelift and LLVM share the ABI, target, and revision rules, while each
    /// selects the runtime built with its own optimization profile.
    pub fn current(profile: NativeProfile) -> Self {
        Self {
            abi_version: option_env!("DORIA_RT_ABI_VERSION")
                .unwrap_or("")
                .to_string(),
            target_triple: option_env!("DORIA_RT_EXPECTED_TARGET")
                .unwrap_or("")
                .to_string(),
            profile: profile_directory(profile).to_string(),
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

/// Complete identity of the archive an executable was linked against.
///
/// Every field is either a recorded fact or an explicit absence. A consumer can
/// tell "this archive is unidentified" from "this archive is identified as X",
/// which is the distinction the benchmark report previously could not make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeProvenance {
    pub path: String,
    pub origin: &'static str,
    pub metadata_path: Option<String>,
    pub bytes: Option<u64>,
    /// Digest of the bytes actually on disk, not the digest the sidecar claims.
    pub sha256: Option<String>,
    pub abi_version: Option<String>,
    pub runtime_revision: Option<String>,
    pub target_triple: Option<String>,
    pub profile: Option<String>,
    /// Whether the recorded digest agrees with the sidecar's claim. `None` when
    /// there was no claim to check.
    pub digest_matches_metadata: Option<bool>,
}

impl RuntimeArtifact {
    /// Read the archive and describe it completely.
    ///
    /// Selection already verifies identified archives before linking. A
    /// performance report reads the bytes again so it records the artifact that
    /// exists at reporting time rather than copying the sidecar's claim.
    pub fn provenance(&self) -> RuntimeProvenance {
        let archive = std::fs::read(&self.path).ok();
        let sha256 = archive.as_ref().map(|bytes| sha256_hex(bytes));
        let bytes = archive.as_ref().map(|bytes| bytes.len() as u64);
        let digest_matches_metadata = match (&self.metadata, &sha256) {
            (Some(metadata), Some(sha256)) => Some(&metadata.sha256 == sha256),
            _ => None,
        };
        RuntimeProvenance {
            path: self.path.display().to_string(),
            origin: self.origin.as_str(),
            metadata_path: self
                .metadata
                .is_some()
                .then(|| metadata_path(&self.path).display().to_string()),
            bytes,
            sha256,
            abi_version: self.metadata.as_ref().map(|m| m.abi_version.clone()),
            runtime_revision: self.metadata.as_ref().map(|m| m.runtime_revision.clone()),
            target_triple: self.metadata.as_ref().map(|m| m.target_triple.clone()),
            profile: self.metadata.as_ref().map(|m| m.profile.clone()),
            digest_matches_metadata,
        }
    }
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
    let explicit_override = env::var_os("DORIA_RT_PATH");
    let expectation = RuntimeExpectation::current(profile);
    if let Some((path, document)) =
        embedded_runtime_candidate(profile, explicit_override.as_deref())?
    {
        // Materialization proved these bytes are the embedded archive and
        // that they match this document; only compatibility remains to check.
        let metadata = parse_metadata(document).ok_or_else(|| {
            invalid_metadata_error(
                &path,
                RuntimeOrigin::CompilerBundled,
                "embedded identity metadata is malformed or incomplete",
            )
        })?;
        return accept_checked(
            path,
            RuntimeOrigin::CompilerBundled,
            &expectation,
            Some(metadata),
            false,
        );
    }
    resolve(
        explicit_override.as_deref(),
        &current_executable,
        compiler_built_runtime(profile),
        workspace,
        target_override.as_deref(),
        if cfg!(all(windows, target_env = "msvc")) {
            ArchiveFormat::Msvc
        } else {
            ArchiveFormat::Gnu
        },
        profile_directory(profile),
        &expectation,
    )
}

fn embedded_runtime_candidate(
    profile: NativeProfile,
    explicit_override: Option<&OsStr>,
) -> Result<Option<(PathBuf, &'static str)>, BackendError> {
    if explicit_override.is_some() {
        return Ok(None);
    }
    materialize_embedded_runtime(profile)
}

fn materialize_embedded_runtime(
    profile: NativeProfile,
) -> Result<Option<(PathBuf, &'static str)>, BackendError> {
    let Some(runtimes) = EMBEDDED_RUNTIMES.get() else {
        return Ok(None);
    };
    let embedded = match profile {
        NativeProfile::Fast => runtimes.debug,
        NativeProfile::Release => runtimes.release,
    };
    let cache_root = env::temp_dir().join(EMBEDDED_RUNTIME_CACHE_DIRECTORY);
    materialize_embedded_runtime_at(embedded, &cache_root)
        .map(|path| Some((path, embedded.metadata)))
}

fn materialize_embedded_runtime_at(
    embedded: EmbeddedRuntimeProfile,
    cache_root: &Path,
) -> Result<PathBuf, BackendError> {
    let filename = runtime_filename(if cfg!(all(windows, target_env = "msvc")) {
        ArchiveFormat::Msvc
    } else {
        ArchiveFormat::Gnu
    });
    materialize_runtime_asset(
        embedded.archive,
        embedded.metadata,
        filename,
        cache_root,
        true,
    )
}

/// The cache key of an archive whose SHA-256 is `digest`. Callers pass a
/// digest they have verified, so the archive is hashed at most once.
fn runtime_cache_identity(digest: &str, metadata: &str) -> String {
    sha256_hex(format!("{digest}\n{metadata}").as_bytes())
}

fn materialize_runtime_asset(
    bytes: &[u8],
    document: &str,
    filename: &str,
    cache_root: &Path,
    allow_private_copy: bool,
) -> Result<PathBuf, BackendError> {
    let metadata = parse_metadata(document).ok_or_else(|| {
        BackendError::new("doria-rt identity metadata is malformed or incomplete")
    })?;
    if metadata.bytes != bytes.len() as u64 {
        return Err(BackendError::new(
            "doria-rt bytes do not match their identity metadata",
        ));
    }

    fs::create_dir_all(cache_root).map_err(|error| {
        BackendError::new(format!(
            "doria-rt could not create its materialization directory: {error}"
        ))
    })?;
    if !allow_private_copy && !is_regular_directory(cache_root) {
        return Err(publication_conflict_error(cache_root));
    }
    // The sidecar is part of the artifact identity. Two compiler revisions can
    // legitimately produce identical archive bytes while recording different
    // revision/profile facts, so key the cache by both inputs.
    // Entries are named by the identity the document claims and published
    // only after their bytes were verified against it. An entry holding
    // exactly these bytes and this document therefore proves the pair, so
    // only a miss hashes the archive (24 MB for the debug runtime).
    let cache_identity = runtime_cache_identity(&metadata.sha256, document);
    let final_directory = cache_root.join(&cache_identity);
    let final_archive = final_directory.join(filename);
    if materialized_runtime_matches(&final_archive, bytes, document) {
        return Ok(final_archive);
    }
    if sha256_hex(bytes) != metadata.sha256 {
        return Err(BackendError::new(
            "doria-rt bytes do not match their identity metadata",
        ));
    }
    if !allow_private_copy {
        match fs::symlink_metadata(&final_directory) {
            Ok(_) => return Err(publication_conflict_error(&final_directory)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(BackendError::new(format!(
                    "could not inspect shared runtime destination {}: {error}",
                    final_directory.display()
                )));
            }
        }
    }

    let staging_directory = unique_materialization_directory(cache_root, &cache_identity);
    fs::create_dir(&staging_directory).map_err(|error| {
        BackendError::new(format!(
            "doria-rt could not create a private materialization directory: {error}"
        ))
    })?;
    let staging_archive = staging_directory.join(filename);
    let write_result = fs::write(&staging_archive, bytes)
        .and_then(|()| fs::write(metadata_path(&staging_archive), document));
    if let Err(error) = write_result {
        let _ = fs::remove_dir_all(&staging_directory);
        return Err(BackendError::new(format!(
            "doria-rt could not be materialized: {error}"
        )));
    }

    match fs::rename(&staging_directory, &final_directory) {
        Ok(()) => Ok(final_archive),
        Err(_) if materialized_runtime_matches(&final_archive, bytes, document) => {
            let _ = fs::remove_dir_all(&staging_directory);
            Ok(final_archive)
        }
        // Never replace an unexpected existing cache entry. The private
        // staging directory already contains verified compiler-owned bytes and
        // is safe to use for this invocation.
        Err(_) if allow_private_copy => Ok(staging_archive),
        Err(error) => {
            let _ = fs::remove_dir_all(&staging_directory);
            Err(BackendError::new(format!("could not publish verified shared runtime at {}: {error}; an existing incompatible companion is never replaced", final_directory.display())))
        }
    }
}

fn materialized_runtime_matches(path: &Path, archive: &[u8], metadata: &str) -> bool {
    let sidecar = metadata_path(path);
    path.parent().is_some_and(is_regular_directory)
        && [path, sidecar.as_path()].into_iter().all(|path| {
            fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_file())
        })
        && fs::read(path).is_ok_and(|bytes| bytes == archive)
        && fs::read_to_string(sidecar).is_ok_and(|contents| contents == metadata)
}

fn is_regular_directory(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_dir())
}

fn publication_conflict_error(path: &Path) -> BackendError {
    BackendError::new(format!(
        "could not publish verified shared runtime at {}; an existing incompatible companion is never replaced",
        path.display()
    ))
}

fn unique_materialization_directory(cache_root: &Path, digest: &str) -> PathBuf {
    let sequence = NEXT_EMBEDDED_RUNTIME_ID.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    cache_root.join(format!(
        ".{digest}-{}-{nanos}-{sequence}",
        std::process::id()
    ))
}

fn compiler_built_runtime(profile: NativeProfile) -> Option<&'static Path> {
    match profile {
        NativeProfile::Fast => option_env!("DORIA_RT_BUILT_DEBUG_PATH").map(Path::new),
        NativeProfile::Release => option_env!("DORIA_RT_BUILT_RELEASE_PATH").map(Path::new),
    }
}

const fn profile_directory(profile: NativeProfile) -> &'static str {
    match profile {
        NativeProfile::Fast => "debug",
        NativeProfile::Release => "release",
    }
}

/// Candidate archives in priority order.
///
/// The matching compiler-owned archive is preferred on every profile. An earlier version
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
        let origin = RuntimeOrigin::ExplicitOverride;
        let metadata = load_metadata(&candidate)
            .map_err(|detail| invalid_metadata_error(&candidate, origin, &detail))?;
        return accept(candidate, origin, expectation, metadata);
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
        let metadata = load_metadata(&candidate)
            .map_err(|detail| invalid_metadata_error(&candidate, origin, &detail))?;
        // An ambient archive with no identity is skipped rather than trusted.
        if metadata.is_none() && !origin.is_deliberate() {
            continue;
        }
        return accept(candidate, origin, expectation, metadata);
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
    metadata: Option<RuntimeMetadata>,
) -> Result<RuntimeArtifact, BackendError> {
    accept_checked(path, origin, expectation, metadata, true)
}

/// `verify_integrity` is false only for an archive whose bytes were already
/// proven to match `metadata`, so the archive is not read and hashed again.
fn accept_checked(
    path: PathBuf,
    origin: RuntimeOrigin,
    expectation: &RuntimeExpectation,
    metadata: Option<RuntimeMetadata>,
    verify_integrity: bool,
) -> Result<RuntimeArtifact, BackendError> {
    if let Some(metadata) = &metadata {
        let mut mismatches = incompatibility(metadata, expectation).unwrap_or_default();
        if metadata.artifact_kind != RuntimeArtifactKind::Static {
            mismatches.push(Mismatch {
                field: "artifact kind",
                expected: "static".into(),
                found: "shared".into(),
            });
        }
        if verify_integrity {
            mismatches.extend(archive_integrity_mismatches(&path, origin, metadata)?);
        }
        if !mismatches.is_empty() {
            return Err(incompatible_runtime_error(&path, origin, &mismatches));
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
    let expected_schema = RUNTIME_METADATA_SCHEMA_VERSION.to_string();
    let found_schema = metadata.schema_version.to_string();
    let comparisons = [
        ("metadata schema", &expected_schema, &found_schema),
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

fn archive_integrity_mismatches(
    path: &Path,
    origin: RuntimeOrigin,
    metadata: &RuntimeMetadata,
) -> Result<Vec<Mismatch>, BackendError> {
    let archive = std::fs::read(path).map_err(|error| {
        invalid_metadata_error(
            path,
            origin,
            &format!("failed to read archive bytes: {error}"),
        )
    })?;
    Ok(integrity_mismatches(&archive, metadata))
}

fn integrity_mismatches(archive: &[u8], metadata: &RuntimeMetadata) -> Vec<Mismatch> {
    let actual_bytes = archive.len() as u64;
    let actual_sha256 = sha256_hex(archive);
    let mut mismatches = Vec::new();
    if metadata.bytes != actual_bytes {
        mismatches.push(Mismatch {
            field: "archive bytes",
            expected: metadata.bytes.to_string(),
            found: actual_bytes.to_string(),
        });
    }
    if metadata.sha256 != actual_sha256 {
        mismatches.push(Mismatch {
            field: "archive SHA-256",
            expected: metadata.sha256.clone(),
            found: actual_sha256,
        });
    }
    mismatches
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
            "rebuild the compiler so it bundles a matching runtime, or set DORIA_RT_PATH to a runtime artifact built from this revision",
        )])
}

fn invalid_metadata_error(path: &Path, origin: RuntimeOrigin, detail: &str) -> BackendError {
    BackendError::from_diagnostics(vec![Diagnostic::new(
        "B0003",
        format!(
            "the selected runtime artifact has invalid identity metadata\nselected runtime: {}\norigin: {}\n{detail}",
            path.display(),
            origin.as_str()
        ),
        Span::default(),
    )
    .with_note("runtime identity must be readable and complete before the artifact is used")
    .with_help(
        "rebuild the compiler so it bundles a matching runtime, or replace the runtime artifact and its .doria-runtime.json sidecar together",
    )])
}

/// Read the sidecar written beside an archive by the build script.
///
/// A missing sidecar is absence of identity: deliberate archives may predate
/// the sidecar, while ambient archives without identity are skipped. A sidecar
/// that exists but cannot be read or parsed is different: it is broken identity
/// evidence and must never be silently treated as no evidence.
fn load_metadata(archive: &Path) -> Result<Option<RuntimeMetadata>, String> {
    let path = metadata_path(archive);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("failed to read {}: {error}", path.display())),
    };
    parse_metadata(&raw)
        .map(Some)
        .ok_or_else(|| format!("{} is malformed or incomplete", path.display()))
}

pub fn metadata_path(archive: &Path) -> PathBuf {
    let mut name = archive.file_name().unwrap_or_default().to_os_string();
    name.push(".doria-runtime.json");
    archive.with_file_name(name)
}

fn parse_metadata(raw: &str) -> Option<RuntimeMetadata> {
    serde_json::from_str(raw).ok()
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
        let archive = b"archive";
        fs::write(path, archive).expect("archive fixture should be written");
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
            "{{\n  \"schemaVersion\": 1,\n  \"abiVersion\": \"{abi}\",\n  \"runtimeRevision\": \"{revision}\",\n  \"targetTriple\": \"{target}\",\n  \"profile\": \"{profile}\",\n  \"featureSet\": [\"standalone-windows-support\"],\n  \"bytes\": {},\n  \"sha256\": \"{}\"\n}}\n",
            archive.len(),
            sha256_hex(archive),
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
    fn replaced_archive_bytes_are_rejected_before_linking() {
        let directory = temp_directory("integrity-size");
        let compiler_built = directory.join("build/libdoria_rt.a");
        write_archive(&compiler_built, &[]);
        fs::write(&compiler_built, b"different bytes entirely")
            .expect("archive should be replaced");

        let error = resolve_in(&directory, None, Some(&compiler_built), "release")
            .expect_err("a stale sidecar must not authenticate replaced bytes");
        assert!(error.message.contains("archive bytes"));
        assert!(error.message.contains("archive SHA-256"));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn same_size_replacement_is_rejected_by_digest_before_linking() {
        let directory = temp_directory("integrity-digest");
        let compiler_built = directory.join("build/libdoria_rt.a");
        write_archive(&compiler_built, &[]);
        fs::write(&compiler_built, b"ARCHIVE").expect("archive should be replaced");

        let error = resolve_in(&directory, None, Some(&compiler_built), "release")
            .expect_err("equal size must not bypass digest verification");
        assert!(!error.message.contains("archive bytes"));
        assert!(error.message.contains("archive SHA-256"));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn malformed_present_metadata_is_rejected() {
        let directory = temp_directory("malformed-metadata");
        let compiler_built = directory.join("build/libdoria_rt.a");
        write_archive(&compiler_built, &[]);
        fs::write(metadata_path(&compiler_built), "not metadata")
            .expect("sidecar should be replaced");

        let error = resolve_in(&directory, None, Some(&compiler_built), "release")
            .expect_err("present malformed identity must not become absent identity");
        assert!(error.message.contains("invalid identity metadata"));
        assert!(error.message.contains("malformed or incomplete"));
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
    fn embedded_runtime_materialization_is_independent_of_cargo_output() {
        let directory = temp_directory("embedded-materialization");
        let archive = b"embedded archive";
        let metadata = format!(
            "{{\n  \"schemaVersion\": 1,\n  \"abiVersion\": \"1\",\n  \"runtimeRevision\": \"cafebabe\",\n  \"targetTriple\": \"aarch64-apple-darwin\",\n  \"profile\": \"debug\",\n  \"featureSet\": [],\n  \"bytes\": {},\n  \"sha256\": \"{}\"\n}}\n",
            archive.len(),
            sha256_hex(archive),
        );
        let archive = materialize_embedded_runtime_at(
            EmbeddedRuntimeProfile {
                archive,
                metadata: Box::leak(metadata.into_boxed_str()),
            },
            &directory,
        )
        .expect("embedded runtime should materialize");

        assert_eq!(fs::read(&archive).unwrap(), b"embedded archive");
        assert!(load_metadata(&archive)
            .expect("metadata should parse")
            .is_some());
        assert!(archive.starts_with(&directory));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn materialization_hashes_on_a_miss_and_rejects_bytes_their_document_does_not_describe() {
        let directory = temp_directory("embedded-integrity");
        let document = |archive: &[u8], digest: String| -> &'static str {
            Box::leak(
                format!(
                    "{{\n  \"schemaVersion\": 1,\n  \"abiVersion\": \"1\",\n  \"runtimeRevision\": \"cafebabe\",\n  \"targetTriple\": \"aarch64-apple-darwin\",\n  \"profile\": \"debug\",\n  \"featureSet\": [],\n  \"bytes\": {},\n  \"sha256\": \"{digest}\"\n}}\n",
                    archive.len(),
                )
                .into_boxed_str(),
            )
        };
        let genuine = b"embedded archive";
        let metadata = document(genuine, sha256_hex(genuine));
        let first = materialize_embedded_runtime_at(
            EmbeddedRuntimeProfile {
                archive: genuine,
                metadata,
            },
            &directory,
        )
        .expect("verified bytes materialize");
        // The same pair reuses the published entry.
        let second = materialize_embedded_runtime_at(
            EmbeddedRuntimeProfile {
                archive: genuine,
                metadata,
            },
            &directory,
        )
        .expect("a verified entry is reused");
        assert_eq!(first, second);
        // Same length, same claimed digest, different bytes: the entry does
        // not hold these bytes, so the miss hashes them and rejects the pair.
        let corrupted = b"embedded archivE";
        let error = materialize_embedded_runtime_at(
            EmbeddedRuntimeProfile {
                archive: corrupted,
                metadata,
            },
            &directory,
        )
        .expect_err("bytes that disagree with their document are rejected");
        assert!(error
            .message
            .contains("do not match their identity metadata"));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn explicit_override_skips_embedded_runtime_materialization() {
        assert_eq!(
            embedded_runtime_candidate(NativeProfile::Fast, Some(OsStr::new("runtime")))
                .expect("explicit runtime selection must not materialize the embedded archive"),
            None
        );
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
    fn provenance_records_the_complete_identity() {
        let directory = temp_directory("provenance");
        let compiler_built = directory.join("build/libdoria_rt.a");
        write_archive(&compiler_built, &[]);
        let resolved = resolve_in(&directory, None, Some(&compiler_built), "release")
            .expect("runtime should resolve");
        let provenance = resolved.provenance();

        assert_eq!(provenance.origin, "compiler-bundled");
        assert_eq!(provenance.abi_version.as_deref(), Some("1"));
        assert_eq!(provenance.profile.as_deref(), Some("release"));
        assert_eq!(
            provenance.target_triple.as_deref(),
            Some("aarch64-apple-darwin")
        );
        assert_eq!(provenance.runtime_revision.as_deref(), Some("cafebabe"));
        assert!(provenance.metadata_path.is_some());
        assert_eq!(provenance.bytes, Some(7));
        assert_eq!(
            provenance.sha256.as_deref(),
            Some(sha256_hex(b"archive").as_str())
        );
        assert_eq!(provenance.digest_matches_metadata, Some(true));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn provenance_reports_an_unidentified_runtime_as_unidentified() {
        // Absence must be reportable. A consumer has to be able to tell "no
        // identity recorded" from "identity recorded as X" so it can mark the
        // evidence rather than quietly compare it.
        let directory = temp_directory("provenance-unidentified");
        let compiler_built = directory.join("build/libdoria_rt.a");
        write_unidentified_archive(&compiler_built);
        let resolved = resolve_in(&directory, None, Some(&compiler_built), "release")
            .expect("runtime should resolve");
        let provenance = resolved.provenance();

        assert!(provenance.abi_version.is_none());
        assert!(provenance.runtime_revision.is_none());
        assert!(provenance.metadata_path.is_none());
        assert!(provenance.digest_matches_metadata.is_none());
        // The bytes are still described even when identity is not.
        assert!(provenance.sha256.is_some());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn provenance_detects_a_sidecar_that_no_longer_matches_its_archive() {
        let directory = temp_directory("provenance-drift");
        let compiler_built = directory.join("build/libdoria_rt.a");
        write_archive(&compiler_built, &[]);
        let metadata = load_metadata(&compiler_built)
            .expect("metadata should be readable")
            .expect("metadata should exist");
        // Provenance still compares current disk bytes even though normal
        // selection would now reject this artifact before linking.
        fs::write(&compiler_built, b"different bytes entirely")
            .expect("archive should be rewritten");
        let artifact = RuntimeArtifact {
            path: compiler_built,
            origin: RuntimeOrigin::CompilerBundled,
            metadata: Some(metadata),
        };
        assert_eq!(artifact.provenance().digest_matches_metadata, Some(false));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn requested_native_profile_selects_its_matching_bundled_runtime() {
        assert_eq!(
            RuntimeExpectation::current(NativeProfile::Fast).profile,
            "debug"
        );
        assert_eq!(
            RuntimeExpectation::current(NativeProfile::Release).profile,
            "release"
        );
        if !cfg!(feature = "bundled-runtime") {
            return;
        }

        let debug = compiler_built_runtime(NativeProfile::Fast)
            .expect("a bundled build should carry a debug runtime");
        let release = compiler_built_runtime(NativeProfile::Release)
            .expect("a bundled build should carry a release runtime");
        assert_ne!(debug, release);
        assert_eq!(
            load_metadata(debug)
                .expect("debug metadata should parse")
                .expect("debug metadata should exist")
                .profile,
            "debug"
        );
        assert_eq!(
            load_metadata(release)
                .expect("release metadata should parse")
                .expect("release metadata should exist")
                .profile,
            "release"
        );
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

    fn shared_document(bytes: &[u8]) -> serde_json::Value {
        let expected = expectation();
        serde_json::json!({
            "schemaVersion": 1,
            "abiVersion": expected.abi_version,
            "runtimeRevision": expected.revision,
            "targetTriple": expected.target_triple,
            "profile": expected.profile,
            "artifactKind": "shared",
            "featureSet": ["hosted-shared-runtime"],
            "bytes": bytes.len(),
            "sha256": sha256_hex(bytes),
        })
    }

    fn shared_dependency(
        bytes: &[u8],
        document: &serde_json::Value,
    ) -> Result<SharedRuntimeDependency, BackendError> {
        verify_shared_runtime(
            Path::new("fixture-library"),
            RuntimeOrigin::ExplicitOverride,
            bytes.to_vec(),
            document.to_string(),
            &expectation(),
        )
    }

    #[test]
    fn shared_runtime_checks_every_identity_and_integrity_field() {
        let bytes = b"shared library fixture";
        let document = shared_document(bytes);
        assert!(shared_dependency(bytes, &document).is_ok());
        for (field, wrong) in [
            ("schemaVersion", serde_json::json!(2)),
            ("abiVersion", serde_json::json!("999")),
            ("runtimeRevision", serde_json::json!("other")),
            ("targetTriple", serde_json::json!("other-target")),
            ("profile", serde_json::json!("debug")),
            ("artifactKind", serde_json::json!("static")),
            (
                "featureSet",
                serde_json::json!(["standalone-windows-support"]),
            ),
            ("bytes", serde_json::json!(0)),
            ("sha256", serde_json::json!("bad-digest")),
        ] {
            let mut changed = document.clone();
            changed[field] = wrong;
            let error = shared_dependency(bytes, &changed).expect_err(field);
            assert_eq!(error.diagnostics.unwrap()[0].code, "B0003", "{field}");
            let mut missing = document.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(
                shared_dependency(bytes, &missing).is_err(),
                "missing {field}"
            );
        }
    }

    #[test]
    fn unidentified_shared_override_is_rejected() {
        let directory = temp_directory("shared-unidentified");
        let path = directory.join(shared_runtime_filename());
        fs::write(&path, b"unidentified library").unwrap();
        let error = read_shared_runtime(&path, RuntimeOrigin::ExplicitOverride).unwrap_err();
        assert!(error.message.contains("identity sidecar unavailable"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn published_php_and_shared_runtime_are_relocatable_and_self_contained() {
        let directory = temp_directory("shared-relocation");
        let bytes = b"verified shared library";
        let dependency = shared_dependency(bytes, &shared_document(bytes)).unwrap();
        assert!(!dependency.relative_library_path().is_absolute());
        let original = directory.join("original");
        crate::backend::publish_output(
            &original.join("main.php"),
            crate::backend::BackendOutput::TextWithRuntime {
                extension: "php".into(),
                contents: "<?php // portable fixture".into(),
                runtime: Box::new(dependency.clone()),
            },
        )
        .unwrap();
        let relocated = directory.join("relocated");
        fs::rename(&original, &relocated).unwrap();
        assert_eq!(
            fs::read(relocated.join(dependency.relative_library_path())).unwrap(),
            bytes
        );
        assert_eq!(
            fs::read_to_string(relocated.join(dependency.relative_metadata_path())).unwrap(),
            dependency.metadata_json()
        );
        dependency.publish(&relocated).unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn incompatible_companion_is_not_overwritten_and_php_is_not_published() {
        let directory = temp_directory("shared-conflict");
        let bytes = b"verified shared library";
        let dependency = shared_dependency(bytes, &shared_document(bytes)).unwrap();
        dependency.publish(&directory).unwrap();
        let path = directory.join(dependency.relative_library_path());
        fs::write(&path, b"unrelated existing contents").unwrap();
        let php = directory.join("main.php");
        let error = crate::backend::publish_output(
            &php,
            crate::backend::BackendOutput::TextWithRuntime {
                extension: "php".into(),
                contents: "<?php // must not appear".into(),
                runtime: Box::new(dependency),
            },
        )
        .unwrap_err();
        assert!(error.message.contains("never replaced"));
        assert!(!php.exists());
        assert_eq!(fs::read(path).unwrap(), b"unrelated existing contents");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn shared_publication_preserves_existing_non_runtime_entries() {
        for directory_entry in [false, true] {
            let directory = temp_directory("shared-existing-entry");
            let bytes = b"verified shared library";
            let dependency = shared_dependency(bytes, &shared_document(bytes)).unwrap();
            let destination = directory
                .join(dependency.relative_library_path())
                .parent()
                .unwrap()
                .to_path_buf();
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            if directory_entry {
                fs::create_dir(&destination).unwrap();
            } else {
                fs::write(&destination, b"existing file").unwrap();
            }
            assert!(dependency.publish(&directory).is_err());
            if directory_entry {
                assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
            } else {
                assert_eq!(fs::read(&destination).unwrap(), b"existing file");
            }
            fs::remove_dir_all(directory).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn shared_publication_does_not_reuse_symlinked_companions() {
        use std::os::unix::fs::symlink;
        for entry in ["root", "directory", "library", "metadata"] {
            let directory = temp_directory("shared-symlink");
            let output = directory.join("output");
            let bytes = b"verified shared library";
            let dependency = shared_dependency(bytes, &shared_document(bytes)).unwrap();
            dependency.publish(&output).unwrap();
            let library = output.join(dependency.relative_library_path());
            let path = match entry {
                "root" => output.join(".doria-runtime"),
                "directory" => library.parent().unwrap().to_path_buf(),
                "library" => library,
                "metadata" => output.join(dependency.relative_metadata_path()),
                _ => unreachable!(),
            };
            let external = directory.join("external");
            fs::rename(&path, &external).unwrap();
            symlink(&external, &path).unwrap();
            assert!(dependency.publish(&output).is_err(), "symlinked {entry}");
            assert!(fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink());
            fs::remove_dir_all(directory).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn output_publication_preserves_permissions_when_adding_executable_bits() {
        use std::os::unix::fs::PermissionsExt;
        let directory = temp_directory("executable-permissions");
        let output = directory.join("main");
        fs::write(&output, b"old executable").unwrap();
        fs::set_permissions(&output, fs::Permissions::from_mode(0o640)).unwrap();
        crate::backend::publish_output(
            &output,
            crate::backend::BackendOutput::Executable {
                extension: String::new(),
                bytes: b"new executable".to_vec(),
            },
        )
        .unwrap();
        assert_eq!(
            fs::metadata(output).unwrap().permissions().mode() & 0o777,
            0o751
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn output_publication_replaces_the_file_a_symlink_names() {
        // Outputs are renamed into place (decision log entry 50); a symlinked
        // destination still receives the bytes in the file it points to.
        use std::os::unix::fs::symlink;
        let directory = temp_directory("symlinked-output");
        let target = directory.join("real");
        let link = directory.join("main");
        fs::write(&target, b"old executable").unwrap();
        symlink(&target, &link).unwrap();
        crate::backend::publish_output(
            &link,
            crate::backend::BackendOutput::Executable {
                extension: String::new(),
                bytes: b"new executable".to_vec(),
            },
        )
        .unwrap();
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(&target).unwrap(), b"new executable");
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 2);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn pure_text_output_does_not_publish_a_runtime() {
        let directory = temp_directory("pure-text");
        let php = directory.join("pure.php");
        crate::backend::publish_output(
            &php,
            crate::backend::BackendOutput::Text {
                extension: "php".into(),
                contents: "<?php echo 'pure';".into(),
            },
        )
        .unwrap();
        assert!(php.is_file());
        assert!(!directory.join(".doria-runtime").exists());
        fs::remove_dir_all(directory).unwrap();
    }
}
