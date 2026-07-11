use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::backend::{BackendError, NativeProfile};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArchiveFormat {
    Gnu,
    Msvc,
}

pub fn locate(profile: NativeProfile) -> Result<PathBuf, BackendError> {
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
    )
}

const fn profile_directory(profile: NativeProfile) -> &'static str {
    match profile {
        NativeProfile::Fast => "debug",
        NativeProfile::Release => "release",
    }
}

fn resolve(
    explicit: Option<&OsStr>,
    current_executable: &Path,
    compiler_built_runtime: Option<&Path>,
    workspace: &Path,
    target_override: Option<&OsStr>,
    archive_format: ArchiveFormat,
    profile: &str,
) -> Result<PathBuf, BackendError> {
    let filename = runtime_filename(archive_format);
    if let Some(explicit) = explicit {
        let explicit = PathBuf::from(explicit);
        let candidate = if explicit.is_dir() {
            explicit.join(filename)
        } else {
            explicit
        };
        if candidate.is_file() {
            return Ok(candidate);
        }
        return Err(not_found_error(Some(&candidate)));
    }

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
    let preferred_profile_runtime = target_root.join(profile).join(filename);
    let mut candidates = Vec::new();
    if profile == "release" {
        candidates.push(preferred_profile_runtime.clone());
    }
    if let Some(compiler_built_runtime) = compiler_built_runtime {
        candidates.push(compiler_built_runtime.to_path_buf());
    }
    if let Some(parent) = current_executable.parent() {
        candidates.push(parent.join(filename));
        candidates.push(parent.join("../lib/doria").join(filename));
        if let Some(profile_directory) = parent.parent() {
            candidates.push(profile_directory.join(filename));
        }
    }
    if profile != "release" {
        candidates.push(preferred_profile_runtime);
    }
    let alternate_profile = if profile == "debug" {
        "release"
    } else {
        "debug"
    };
    candidates.push(target_root.join(alternate_profile).join(filename));

    candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| not_found_error(None))
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

    #[test]
    fn explicit_runtime_path_wins() {
        let directory = temp_directory("override");
        let runtime = directory.join(runtime_filename(ArchiveFormat::Gnu));
        fs::write(&runtime, b"archive").expect("runtime fixture should be written");
        let resolved = resolve(
            Some(runtime.as_os_str()),
            &directory.join("bin/doriac"),
            None,
            &directory,
            None,
            ArchiveFormat::Gnu,
            "debug",
        )
        .expect("explicit runtime should resolve");
        assert_eq!(resolved, runtime);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn development_target_directory_is_a_fallback() {
        let directory = temp_directory("target");
        let runtime = directory
            .join("target/debug")
            .join(runtime_filename(ArchiveFormat::Gnu));
        fs::create_dir_all(runtime.parent().expect("runtime should have parent"))
            .expect("target directory should be created");
        fs::write(&runtime, b"archive").expect("runtime fixture should be written");
        let resolved = resolve(
            None,
            &directory.join("elsewhere/doriac"),
            None,
            &directory,
            None,
            ArchiveFormat::Gnu,
            "debug",
        )
        .expect("workspace runtime should resolve");
        assert_eq!(resolved, runtime);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn missing_runtime_has_build_help() {
        let directory = temp_directory("missing");
        let error = resolve(
            None,
            &directory.join("bin/doriac"),
            None,
            &directory,
            None,
            ArchiveFormat::Gnu,
            "debug",
        )
        .expect_err("missing runtime should fail");
        assert!(error
            .message
            .contains("doria-rt static library was not found"));
        assert!(error.message.contains("cargo build -p doria-rt"));
        assert!(error.message.contains("DORIA_RT_PATH"));
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
        fs::write(&runtime, b"archive").expect("runtime fixture should be written");
        let resolved = resolve(
            Some(directory.as_os_str()),
            &directory.join("bin/doriac.exe"),
            None,
            &directory,
            None,
            ArchiveFormat::Gnu,
            "debug",
        )
        .expect("MinGW runtime should resolve");
        assert_eq!(resolved, runtime);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn compiler_built_archive_precedes_passive_fallbacks() {
        let directory = temp_directory("compiler-build");
        let runtime = directory.join("build/libdoria_rt.a");
        let adjacent = directory.join("bin/libdoria_rt.a");
        fs::create_dir_all(runtime.parent().expect("runtime should have parent"))
            .expect("compiler build directory should be created");
        fs::create_dir_all(
            adjacent
                .parent()
                .expect("adjacent runtime should have parent"),
        )
        .expect("compiler bin directory should be created");
        fs::write(&runtime, b"archive").expect("runtime fixture should be written");
        fs::write(adjacent, b"stale archive").expect("adjacent fixture should be written");
        let resolved = resolve(
            None,
            &directory.join("bin/doriac"),
            Some(&runtime),
            &directory,
            None,
            ArchiveFormat::Gnu,
            "debug",
        )
        .expect("compiler-built runtime should resolve before passive fallbacks");
        assert_eq!(resolved, runtime);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn release_profile_prefers_release_runtime_over_compiler_built_fallback() {
        let directory = temp_directory("release-preference");
        let compiler_built = directory.join("build/libdoria_rt.a");
        let release = directory.join("target/release/libdoria_rt.a");
        fs::create_dir_all(compiler_built.parent().unwrap()).unwrap();
        fs::create_dir_all(release.parent().unwrap()).unwrap();
        fs::write(&compiler_built, b"debug").unwrap();
        fs::write(&release, b"release").unwrap();

        let resolved = resolve(
            None,
            &directory.join("bin/doriac"),
            Some(&compiler_built),
            &directory,
            None,
            ArchiveFormat::Gnu,
            "release",
        )
        .expect("release runtime should resolve");
        assert_eq!(resolved, release);
        let _ = fs::remove_dir_all(directory);
    }
}
