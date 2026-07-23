use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::backend::{BackendError, NativeProfile};
use crate::{codegen_cranelift, mir, runtime_artifact};

pub fn generate_executable(
    program: &mir::Program,
    profile: NativeProfile,
) -> Result<Vec<u8>, BackendError> {
    let object_bytes = match profile {
        NativeProfile::Fast => codegen_cranelift::lower_mir_to_object(program)?,
        NativeProfile::Release => lower_release_object(program)?,
    };
    let runtime_path = runtime_artifact::locate(profile)?;
    link_object(&object_bytes, &runtime_path)
}

#[cfg(feature = "llvm-backend")]
fn lower_release_object(program: &mir::Program) -> Result<Vec<u8>, BackendError> {
    crate::codegen_llvm::lower_mir_to_object(program)
}

#[cfg(not(feature = "llvm-backend"))]
fn lower_release_object(_program: &mir::Program) -> Result<Vec<u8>, BackendError> {
    Err(BackendError::new(
        "LLVM release support is not available in this doriac build\nhelp: rebuild doriac with the llvm-backend feature",
    ))
}

fn link_object(object_bytes: &[u8], runtime_path: &Path) -> Result<Vec<u8>, BackendError> {
    let temp_stem = unique_temp_stem();
    let object_path = temp_stem.with_extension(object_extension());
    let executable_path = temp_stem.with_extension(executable_extension());

    fs::write(&object_path, object_bytes)
        .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))?;

    let link_result = invoke_linker(&object_path, runtime_path, &executable_path);
    let executable_bytes = match link_result {
        Ok(()) => fs::read(&executable_path)
            .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))?,
        Err(error) => {
            cleanup_temp_artifacts(&object_path, &executable_path);
            return Err(error);
        }
    };

    cleanup_temp_artifacts(&object_path, &executable_path);
    Ok(executable_bytes)
}

fn invoke_linker(
    object_path: &Path,
    runtime_path: &Path,
    executable_path: &Path,
) -> Result<(), BackendError> {
    // Cranelift emits a host object from MIR, then the host toolchain links it.
    // Doria does not generate C source or use C semantics as an oracle.
    let cc_is_set = env::var_os("CC").is_some();
    let msvc_host = cfg!(all(windows, target_env = "msvc"));
    let linker = env::var("CC").unwrap_or_else(|_| default_linker(msvc_host).to_string());
    let mut command = Command::new(&linker);
    command.args(linker_arguments(
        &linker,
        cc_is_set,
        cfg!(windows),
        msvc_host,
        object_path,
        runtime_path,
        executable_path,
    ));

    let output = command.output().map_err(|error| {
        BackendError::new(format!(
            "linker/toolchain failure: failed to run `{linker}`: {error}"
        ))
    })?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let details = [stderr.trim(), stdout.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    if details.is_empty() {
        Err(BackendError::new(format!(
            "linker/toolchain failure: `{linker}` exited with status {}",
            output.status
        )))
    } else {
        Err(BackendError::new(format!(
            "linker/toolchain failure: `{linker}` exited with status {}\n{}",
            output.status, details
        )))
    }
}

fn cleanup_temp_artifacts(object_path: &Path, executable_path: &Path) {
    let _ = fs::remove_file(object_path);
    let _ = fs::remove_file(executable_path);
}

fn unique_temp_stem() -> PathBuf {
    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    let sequence = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    env::temp_dir().join(format!(
        "doriac-native-{}-{nanos}-{sequence}",
        std::process::id()
    ))
}

fn object_extension() -> &'static str {
    if cfg!(windows) {
        "obj"
    } else {
        "o"
    }
}

fn executable_extension() -> &'static str {
    if cfg!(windows) {
        "exe"
    } else {
        "out"
    }
}

fn default_linker(msvc_host: bool) -> &'static str {
    if msvc_host {
        "cl.exe"
    } else {
        "cc"
    }
}

fn linker_arguments(
    linker: &str,
    cc_is_set: bool,
    windows: bool,
    msvc_host: bool,
    object_path: &Path,
    runtime_path: &Path,
    executable_path: &Path,
) -> Vec<OsString> {
    if windows && ((msvc_host && !cc_is_set) || is_msvc_style_compiler_driver(linker)) {
        // Cranelift-generated objects do not carry MSVC /DEFAULTLIB directives.
        // For the current generated process wrapper, make Doria's main the executable
        // entrypoint instead of relying on CRT startup to discover and call it.
        // LLVM may emit __chkstk for any function whose frame crosses the Windows
        // stack-probe threshold. Link the static CRT archive that owns the
        // stack-probe support object because generated objects carry no
        // /DEFAULTLIB directives.
        return vec![
            OsString::from("/nologo"),
            object_path.as_os_str().to_os_string(),
            runtime_path.as_os_str().to_os_string(),
            OsString::from(format!("/Fe:{}", executable_path.display())),
            OsString::from("/link"),
            OsString::from("/ENTRY:main"),
            OsString::from("/SUBSYSTEM:CONSOLE"),
            OsString::from("libcmt.lib"),
            OsString::from("kernel32.lib"),
        ];
    }

    vec![
        object_path.as_os_str().to_os_string(),
        runtime_path.as_os_str().to_os_string(),
        OsString::from("-o"),
        executable_path.as_os_str().to_os_string(),
    ]
}

fn is_msvc_style_compiler_driver(linker: &str) -> bool {
    let Some(name) = Path::new(linker).file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    matches!(
        name.to_ascii_lowercase().as_str(),
        "cl" | "cl.exe" | "clang-cl" | "clang-cl.exe"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_default_uses_msvc_compiler_driver_arguments() {
        let args = linker_arguments(
            "cl.exe",
            false,
            true,
            true,
            Path::new("main.obj"),
            Path::new("doria_rt.lib"),
            Path::new("main.exe"),
        );

        assert_eq!(
            args,
            vec![
                OsString::from("/nologo"),
                OsString::from("main.obj"),
                OsString::from("doria_rt.lib"),
                OsString::from("/Fe:main.exe"),
                OsString::from("/link"),
                OsString::from("/ENTRY:main"),
                OsString::from("/SUBSYSTEM:CONSOLE"),
                OsString::from("libcmt.lib"),
                OsString::from("kernel32.lib"),
            ]
        );
    }

    #[test]
    fn windows_clang_cl_uses_msvc_compiler_driver_arguments() {
        let args = linker_arguments(
            "clang-cl.exe",
            true,
            true,
            true,
            Path::new("main.obj"),
            Path::new("doria_rt.lib"),
            Path::new("main.exe"),
        );

        assert_eq!(
            args,
            vec![
                OsString::from("/nologo"),
                OsString::from("main.obj"),
                OsString::from("doria_rt.lib"),
                OsString::from("/Fe:main.exe"),
                OsString::from("/link"),
                OsString::from("/ENTRY:main"),
                OsString::from("/SUBSYSTEM:CONSOLE"),
                OsString::from("libcmt.lib"),
                OsString::from("kernel32.lib"),
            ]
        );
    }

    #[test]
    fn unix_style_compiler_driver_uses_dash_o() {
        let args = linker_arguments(
            "clang",
            true,
            true,
            true,
            Path::new("main.obj"),
            Path::new("doria_rt.lib"),
            Path::new("main.exe"),
        );

        assert_eq!(
            args,
            vec![
                OsString::from("main.obj"),
                OsString::from("doria_rt.lib"),
                OsString::from("-o"),
                OsString::from("main.exe"),
            ]
        );
    }

    #[test]
    fn windows_gnu_default_uses_gnu_compiler_driver_arguments() {
        let args = linker_arguments(
            "cc",
            false,
            true,
            false,
            Path::new("main.obj"),
            Path::new("libdoria_rt.a"),
            Path::new("main.exe"),
        );

        assert_eq!(
            args,
            vec![
                OsString::from("main.obj"),
                OsString::from("libdoria_rt.a"),
                OsString::from("-o"),
                OsString::from("main.exe"),
            ]
        );
        assert_eq!(default_linker(false), "cc");
        assert_eq!(default_linker(true), "cl.exe");
    }
}
