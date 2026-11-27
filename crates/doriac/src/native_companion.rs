//! The native companion library artifact (Decision 0148 sections 22 and
//! 23): the platform shared library a program's unsafe regions run in under
//! the interpreter and PHP output, its metadata sidecar, and its place in
//! the compiler cache.
//!
//! The library is `<stem>.native.<so|dylib|dll>`. It exports one
//! `doria_region_v1_<id>` entry per region, a `doria_callback_v1_<n>` entry
//! per C-callable function and a `doria_binding_v1_<n>` entry per extern
//! declaration's function a region reaches, depends on the hosted shared
//! runtime, which sits at `.doria-runtime/<identity>/` beside it, and links
//! the foreign libraries its own code calls. The runtime files those
//! libraries load sit beside it. `<stem>.native.json` records what a host
//! checks before it loads the library.
//!
//! The companion follows the build's native profile: Cranelift by default
//! and LLVM for `--release`, as the program it serves would be built.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::backend::{BackendError, NativeProfile};
use crate::mir;
use crate::mir::companion::CompanionLibrary;
use crate::native_link::NativeLinking;
use crate::runtime_artifact::{self, RuntimeExpectation, SharedRuntimeDependency};
use crate::runtime_digest::sha256_hex;

/// The version of the `<stem>.native.json` document.
pub const SIDECAR_SCHEMA_VERSION: u32 = 1;

/// The directory under the compiler cache root that holds built companions.
pub const CACHE_DIRECTORY: &str = "doria-companion-v1";

/// The file name of the companion library of `stem`.
pub fn library_file_name(stem: &str) -> String {
    format!(
        "{stem}.native.{}",
        crate::codegen_native::shared_library_extension()
    )
}

/// The file name of the sidecar of the companion library of `stem`.
pub fn sidecar_file_name(stem: &str) -> String {
    format!("{stem}.native.json")
}

/// A built companion: where its library is, and the sidecar describing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedCompanion {
    pub library: PathBuf,
    pub sidecar: PathBuf,
    /// The shared runtime the library depends on, relative to its directory.
    pub runtime: PathBuf,
}

/// Builds the companion library of `program` into `directory` as `stem`:
/// the shared runtime, the library, its runtime files and its sidecar.
/// `program` has regions; a program without them has no companion.
pub fn publish(
    program: &mir::Program,
    profile: NativeProfile,
    linking: &NativeLinking,
    directory: &Path,
    stem: &str,
) -> Result<PublishedCompanion, BackendError> {
    let library = mir::companion::library(program);
    let runtime = runtime_artifact::shared_runtime_dependency()?;
    publish_library(&library, &runtime, profile, linking, directory, stem)
}

fn publish_library(
    library: &CompanionLibrary,
    runtime: &SharedRuntimeDependency,
    profile: NativeProfile,
    linking: &NativeLinking,
    directory: &Path,
    stem: &str,
) -> Result<PublishedCompanion, BackendError> {
    let emitted = || {
        library
            .emitted
            .iter()
            .map(|id| &library.program.functions[id.0])
    };
    let plan = linking
        .resolve_functions(emitted(), crate::c_layout::CTarget::host())
        .map_err(BackendError::from_diagnostics)?;
    let object = match profile {
        NativeProfile::Fast => crate::codegen_cranelift::lower_companion_object(library)?,
        NativeProfile::Release => release_object(library)?,
    };
    fs::create_dir_all(directory).map_err(|error| {
        BackendError::new(format!(
            "could not create the companion's directory `{}`: {error}",
            directory.display()
        ))
    })?;
    runtime.publish(directory)?;
    let runtime_path = runtime.relative_library_path().to_path_buf();
    let runtime_directory = runtime_path.parent().unwrap_or(Path::new("."));
    let bytes = crate::codegen_native::link_shared_library(
        &object,
        &directory.join(&runtime_path),
        runtime_directory,
        &library_file_name(stem),
        &plan,
    )?;
    let library_path = directory.join(library_file_name(stem));
    write(&library_path, &bytes)?;
    for file in &plan.runtime_files {
        crate::backend::publish_runtime_file(file, directory)?;
    }
    let sidecar = sidecar(library, runtime, profile, &bytes, stem);
    let sidecar_path = directory.join(sidecar_file_name(stem));
    let document = serde_json::to_string_pretty(&sidecar)
        .map_err(|error| BackendError::new(format!("companion sidecar failure: {error}")))?;
    write(&sidecar_path, document.as_bytes())?;
    Ok(PublishedCompanion {
        library: library_path,
        sidecar: sidecar_path,
        runtime: runtime_path,
    })
}

/// The companion library of `program` in the compiler cache, built there
/// on a miss: `doria-companion-v1/<key>/` under the root of the embedded
/// runtime cache, where the interpreter loads it.
///
/// The key hashes the compiler identity, the shared runtime's digest, the
/// MIR of every region entry and every function the library emits with the
/// companion plan (the host operations and their slot records), the
/// resolved link inputs, the target triple and the profile.
pub fn cached(
    program: &mir::Program,
    profile: NativeProfile,
    linking: &NativeLinking,
    stem: &str,
) -> Result<PublishedCompanion, BackendError> {
    let library = mir::companion::library(program);
    let runtime = runtime_artifact::shared_runtime_dependency()?;
    let plan = linking
        .resolve_functions(
            library
                .emitted
                .iter()
                .map(|id| &library.program.functions[id.0]),
            crate::c_layout::CTarget::host(),
        )
        .map_err(BackendError::from_diagnostics)?;
    let expectation = RuntimeExpectation::current(profile);
    // The compiler identity is part of the key, so the MIR's debug
    // rendering is a stable input for one compiler.
    let mut identity = format!(
        "{}\n{}\n{}\n{:?}\n{:?}\n{:?}\n",
        expectation.revision,
        runtime.metadata().sha256,
        expectation.target_triple,
        profile,
        plan,
        program.companion,
    );
    for id in &library.emitted {
        identity.push_str(&format!("{:?}\n", library.program.functions[id.0]));
    }
    let key = sha256_hex(identity.as_bytes());
    let root = std::env::temp_dir().join(CACHE_DIRECTORY);
    let directory = root.join(&key);
    let published = PublishedCompanion {
        library: directory.join(library_file_name(stem)),
        sidecar: directory.join(sidecar_file_name(stem)),
        runtime: runtime.relative_library_path().to_path_buf(),
    };
    if matches_sidecar(&published) {
        return Ok(published);
    }
    // Build beside the final directory, then move it into place, so a
    // directory under its key is always complete.
    let staging = root.join(format!(
        "{key}.{}.{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0)
    ));
    publish_library(&library, &runtime, profile, linking, &staging, stem)?;
    if fs::rename(&staging, &directory).is_err() {
        let _ = fs::remove_dir_all(&staging);
        if !matches_sidecar(&published) {
            return Err(BackendError::new(format!(
                "could not place the companion library in `{}`",
                directory.display()
            )));
        }
    }
    Ok(published)
}

/// Whether the companion at `published` is complete: its sidecar records
/// the digest of its library.
fn matches_sidecar(published: &PublishedCompanion) -> bool {
    let (Ok(library), Ok(document)) = (
        fs::read(&published.library),
        fs::read_to_string(&published.sidecar),
    ) else {
        return false;
    };
    serde_json::from_str::<Value>(&document)
        .ok()
        .and_then(|sidecar| sidecar["sha256"].as_str().map(str::to_string))
        .is_some_and(|digest| digest == sha256_hex(&library))
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), BackendError> {
    fs::write(path, bytes).map_err(|error| {
        BackendError::new(format!(
            "could not write the companion file `{}`: {error}",
            path.display()
        ))
    })
}

#[cfg(feature = "llvm-backend")]
fn release_object(library: &CompanionLibrary) -> Result<Vec<u8>, BackendError> {
    crate::codegen_llvm::lower_companion_object(library)
}

#[cfg(not(feature = "llvm-backend"))]
fn release_object(_library: &CompanionLibrary) -> Result<Vec<u8>, BackendError> {
    Err(BackendError::new(
        "LLVM release support is not available in this doriac build\nhelp: rebuild doriac with the llvm-backend feature",
    ))
}

/// The sidecar document (Decision 0148 section 23): its schema version,
/// the compiler and runtime identity, the library with its size and
/// SHA-256, the region entries with their slot records, the host
/// operations, the callbacks, the binding entries and the foreign
/// libraries the library links.
fn sidecar(
    library: &CompanionLibrary,
    runtime: &SharedRuntimeDependency,
    profile: NativeProfile,
    bytes: &[u8],
    stem: &str,
) -> Value {
    let mut sidecar = expected_sidecar(library, runtime, profile);
    sidecar["library"] = json!(library_file_name(stem));
    sidecar["bytes"] = json!(bytes.len());
    sidecar["sha256"] = json!(sha256_hex(bytes));
    sidecar
}

/// The sidecar of the companion library of `program` without the fields
/// that name and digest the built file: what a host generated for this
/// program checks a sidecar against before it loads the library.
pub fn expected_sidecar_of(
    program: &mir::Program,
    profile: NativeProfile,
) -> Result<Value, BackendError> {
    let library = mir::companion::library(program);
    let runtime = runtime_artifact::shared_runtime_dependency()?;
    Ok(expected_sidecar(&library, &runtime, profile))
}

fn expected_sidecar(
    library: &CompanionLibrary,
    runtime: &SharedRuntimeDependency,
    profile: NativeProfile,
) -> Value {
    let expectation = RuntimeExpectation::current(profile);
    let companion = &library.program.companion;
    let mut foreign_libraries = Vec::<String>::new();
    for id in &library.emitted {
        for block in &library.program.functions[id.0].blocks {
            for statement in &block.statements {
                if let mir::Statement::ForeignCall { function, .. } = statement {
                    if let mir::ForeignCallee::Symbol {
                        library: Some(name),
                        ..
                    } = &function.callee
                    {
                        if !foreign_libraries.contains(name) {
                            foreign_libraries.push(name.clone());
                        }
                    }
                }
            }
        }
    }
    json!({
        "schemaVersion": SIDECAR_SCHEMA_VERSION,
        "entryAbiVersion": 1,
        "compiler": {
            "revision": expectation.revision,
            "targetTriple": expectation.target_triple,
            "profile": match profile {
                NativeProfile::Fast => "fast",
                NativeProfile::Release => "release",
            },
        },
        "runtime": {
            "abiVersion": runtime.metadata().abi_version,
            "sha256": runtime.metadata().sha256,
            "path": runtime.relative_library_path().to_string_lossy().replace('\\', "/"),
        },
        "regions": companion.regions.iter().map(|region| json!({
            "id": region.id,
            "symbol": region.entry_symbol(),
            "inputs": record(&region.inputs),
            "outputs": record(&region.outputs),
        })).collect::<Vec<_>>(),
        "hostOperations": companion.host_operations.iter().map(|operation| json!({
            "id": operation.id,
            "region": companion.regions[operation.region].id,
            "arguments": record(&operation.arguments),
            "results": record(&operation.results),
        })).collect::<Vec<_>>(),
        "callbacks": companion.callbacks.iter().map(|callback| json!({
            "operation": callback.operation,
            "function": library.program.functions[callback.function.0].name,
            "symbol": callback.entry_symbol(),
            "arguments": record(&callback.arguments),
            "results": record(&callback.results),
        })).collect::<Vec<_>>(),
        "bindings": companion.bindings.iter().map(|binding| json!({
            "function": library.program.functions[binding.function.0].name,
            "symbol": binding.entry_symbol(),
            "arguments": record(&binding.arguments),
            "results": record(&binding.results),
        })).collect::<Vec<_>>(),
        "foreignLibraries": foreign_libraries,
    })
}

fn record(record: &mir::SlotRecord) -> Value {
    json!({
        "size": record.size,
        "align": record.align,
        "index": record.index,
        "slots": record.slots.iter().map(|slot| json!({
            "offset": slot.offset,
            "kind": match slot.value.kind {
                mir::UnsafeRegionValueKind::Native => "native",
                mir::UnsafeRegionValueKind::Text => "text",
                mir::UnsafeRegionValueKind::Buffer => "buffer",
                mir::UnsafeRegionValueKind::Region => "region",
                mir::UnsafeRegionValueKind::ForeignString => "foreignString",
                mir::UnsafeRegionValueKind::HostToken => "hostToken",
            },
            "nullable": slot.value.nullable,
        })).collect::<Vec<_>>(),
    })
}
