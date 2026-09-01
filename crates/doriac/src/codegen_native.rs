use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
use std::process::Output;

use crate::backend::{BackendError, NativeProfile};
use crate::{codegen_cranelift, mir, runtime_artifact};

#[derive(Debug, Clone)]
pub struct NativePerformance {
    pub mir_validation: Duration,
    pub code_generation: Duration,
    pub runtime_selection: Duration,
    pub runtime_artifact: PathBuf,
    pub runtime: crate::runtime_artifact::RuntimeArtifact,
    pub linker: String,
    pub link_command: Vec<String>,
    pub linking: Duration,
}

pub fn generate_executable(
    program: &mir::Program,
    profile: NativeProfile,
) -> Result<Vec<u8>, BackendError> {
    let object_bytes = match profile {
        NativeProfile::Fast => codegen_cranelift::lower_mir_to_object(program)?,
        NativeProfile::Release => lower_release_object(program)?,
    };
    let runtime = runtime_artifact::locate(profile)?;
    let runtime_path = runtime.path;
    link_object(&object_bytes, &runtime_path)
}

pub(crate) fn generate_executable_with_performance(
    program: &mir::Program,
    profile: NativeProfile,
) -> Result<(Vec<u8>, NativePerformance), BackendError> {
    let started = Instant::now();
    crate::mir_validation::validate_program(program)?;
    let mir_validation = started.elapsed();

    let started = Instant::now();
    let object_bytes = match profile {
        NativeProfile::Fast => codegen_cranelift::lower_validated_mir_to_object(program)?,
        NativeProfile::Release => lower_validated_release_object(program)?,
    };
    let code_generation = started.elapsed();

    let started = Instant::now();
    let runtime = runtime_artifact::locate(profile)?;
    let runtime_path = runtime.path.clone();
    let runtime_selection = started.elapsed();

    let started = Instant::now();
    let (executable, linker, link_command) =
        link_object_with_metadata(&object_bytes, &runtime_path)?;
    let linking = started.elapsed();
    Ok((
        executable,
        NativePerformance {
            mir_validation,
            code_generation,
            runtime_selection,
            runtime_artifact: runtime_path,
            runtime,
            linker,
            link_command,
            linking,
        },
    ))
}

#[cfg(feature = "llvm-backend")]
fn lower_release_object(program: &mir::Program) -> Result<Vec<u8>, BackendError> {
    crate::codegen_llvm::lower_mir_to_object(program)
}

#[cfg(feature = "llvm-backend")]
fn lower_validated_release_object(program: &mir::Program) -> Result<Vec<u8>, BackendError> {
    crate::codegen_llvm::lower_validated_mir_to_object(program)
}

#[cfg(not(feature = "llvm-backend"))]
fn lower_validated_release_object(_program: &mir::Program) -> Result<Vec<u8>, BackendError> {
    Err(BackendError::new(
        "LLVM release support is not available in this doriac build\nhelp: rebuild doriac with the llvm-backend feature",
    ))
}

#[cfg(not(feature = "llvm-backend"))]
fn lower_release_object(_program: &mir::Program) -> Result<Vec<u8>, BackendError> {
    Err(BackendError::new(
        "LLVM release support is not available in this doriac build\nhelp: rebuild doriac with the llvm-backend feature",
    ))
}

fn link_object(object_bytes: &[u8], runtime_path: &Path) -> Result<Vec<u8>, BackendError> {
    link_object_with_metadata(object_bytes, runtime_path).map(|(bytes, _, _)| bytes)
}

fn link_object_with_metadata(
    object_bytes: &[u8],
    runtime_path: &Path,
) -> Result<(Vec<u8>, String, Vec<String>), BackendError> {
    let temp_stem = unique_temp_stem();
    let object_path = temp_stem.with_extension(object_extension());
    let executable_path = temp_stem.with_extension(executable_extension());

    fs::write(&object_path, object_bytes)
        .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))?;

    let (linker, arguments) = match invoke_linker(&object_path, runtime_path, &executable_path) {
        Ok(metadata) => metadata,
        Err(error) => {
            cleanup_temp_artifacts(&object_path, &executable_path);
            return Err(error);
        }
    };
    let executable_bytes = match fs::read(&executable_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            cleanup_temp_artifacts(&object_path, &executable_path);
            return Err(BackendError::new(format!(
                "backend emission failure: {error}"
            )));
        }
    };

    cleanup_temp_artifacts(&object_path, &executable_path);
    let mut command = Vec::with_capacity(arguments.len() + 1);
    command.push(linker.clone());
    command.extend(arguments);
    Ok((executable_bytes, linker, command))
}

fn invoke_linker(
    object_path: &Path,
    runtime_path: &Path,
    executable_path: &Path,
) -> Result<(String, Vec<String>), BackendError> {
    // Cranelift emits a host object from MIR, then the host toolchain links it.
    // Doria does not generate C source or use C semantics as an oracle.
    let cc_is_set = env::var_os("CC").is_some();
    let msvc_host = cfg!(all(windows, target_env = "msvc"));
    let linker = env::var("CC").unwrap_or_else(|_| default_linker(msvc_host).to_string());
    let arguments = linker_arguments(
        &linker,
        cc_is_set,
        cfg!(windows),
        msvc_host,
        object_path,
        runtime_path,
        executable_path,
    );
    let mut command = Command::new(&linker);
    command.args(&arguments);

    let output = match command.output() {
        Ok(output) => output,
        Err(error) => {
            #[cfg(windows)]
            if error.kind() == std::io::ErrorKind::NotFound
                && msvc_host
                && !cc_is_set
                && is_msvc_style_compiler_driver(&linker)
            {
                match invoke_with_discovered_msvc(&linker, &arguments) {
                    Ok(Some(output)) => output,
                    Ok(None) => {
                        return Err(BackendError::new(format!(
                            "linker/toolchain failure: failed to run `{linker}`: {error}\n\
                             help: install the Visual Studio C++ build tools or run from a Visual Studio developer shell"
                        )));
                    }
                    Err(discovery_error) => {
                        return Err(BackendError::new(format!(
                            "linker/toolchain failure: failed to run `{linker}`: {error}\n\
                             Visual Studio toolchain discovery also failed: {discovery_error}"
                        )));
                    }
                }
            } else {
                return Err(BackendError::new(format!(
                    "linker/toolchain failure: failed to run `{linker}`: {error}"
                )));
            }

            #[cfg(not(windows))]
            return Err(BackendError::new(format!(
                "linker/toolchain failure: failed to run `{linker}`: {error}"
            )));
        }
    };

    if output.status.success() {
        return Ok((
            linker,
            arguments
                .into_iter()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect(),
        ));
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

#[cfg(windows)]
fn invoke_with_discovered_msvc(
    linker: &str,
    arguments: &[OsString],
) -> Result<Option<Output>, String> {
    let Some(environment) = discover_msvc_environment()? else {
        return Ok(None);
    };
    let executable = executable_from_environment(linker, &environment)
        .ok_or_else(|| format!("`{linker}` was absent from the discovered Visual Studio PATH"))?;

    let mut command = Command::new(&executable);
    command.args(arguments).envs(environment);
    command.output().map(Some).map_err(|error| {
        format!(
            "failed to run discovered linker `{}`: {error}",
            executable.display()
        )
    })
}

#[cfg(windows)]
fn executable_from_environment(
    executable: &str,
    environment: &[(OsString, OsString)],
) -> Option<PathBuf> {
    environment
        .iter()
        .find(|(name, _)| name.to_string_lossy().eq_ignore_ascii_case("PATH"))
        .and_then(|(_, path)| {
            env::split_paths(path)
                .map(|directory| directory.join(executable))
                .find(|candidate| candidate.is_file())
        })
}

#[cfg(windows)]
fn discover_msvc_environment() -> Result<Option<Vec<(OsString, OsString)>>, String> {
    let Some(vsdevcmd) = locate_vsdevcmd()? else {
        return Ok(None);
    };
    let architecture = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "amd64"
    };
    let command_line = format!(
        "call \"{}\" -no_logo -arch={architecture} -host_arch={architecture} >nul && set",
        vsdevcmd.display()
    );
    let command_interpreter = env::var_os("COMSPEC").unwrap_or_else(|| OsString::from("cmd.exe"));
    let mut command = Command::new(command_interpreter);
    command.args(["/d", "/u", "/c"]).raw_arg(&command_line);
    let output = command
        .output()
        .map_err(|error| format!("failed to query `{}`: {error}", vsdevcmd.display()))?;

    if !output.status.success() {
        let stderr = decode_utf16le(&output.stderr)
            .unwrap_or_else(|_| String::from_utf8_lossy(&output.stderr).into_owned());
        return Err(format!(
            "`{}` exited with status {}{}{}",
            vsdevcmd.display(),
            output.status,
            if stderr.trim().is_empty() { "" } else { ": " },
            stderr.trim()
        ));
    }

    parse_utf16le_environment(&output.stdout).map(Some)
}

#[cfg(windows)]
fn locate_vsdevcmd() -> Result<Option<PathBuf>, String> {
    if let Some(install_dir) = env::var_os("VSINSTALLDIR") {
        let candidate = PathBuf::from(install_dir)
            .join("Common7")
            .join("Tools")
            .join("VsDevCmd.bat");
        if candidate.is_file() {
            return Ok(Some(candidate));
        }
    }

    let mut vswhere_candidates = Vec::new();
    for variable in ["ProgramFiles(x86)", "ProgramFiles"] {
        let Some(program_files) = env::var_os(variable) else {
            continue;
        };
        let candidate = PathBuf::from(program_files)
            .join("Microsoft Visual Studio")
            .join("Installer")
            .join("vswhere.exe");
        if !vswhere_candidates.contains(&candidate) {
            vswhere_candidates.push(candidate);
        }
    }

    let required_component = if cfg!(target_arch = "aarch64") {
        "Microsoft.VisualStudio.Component.VC.Tools.ARM64"
    } else {
        "Microsoft.VisualStudio.Component.VC.Tools.x86.x64"
    };
    for vswhere in vswhere_candidates {
        if !vswhere.is_file() {
            continue;
        }
        let output = Command::new(&vswhere)
            .args([
                "-latest",
                "-products",
                "*",
                "-requires",
                required_component,
                "-property",
                "installationPath",
            ])
            .output()
            .map_err(|error| format!("failed to run `{}`: {error}", vswhere.display()))?;
        if !output.status.success() {
            continue;
        }
        let installation = String::from_utf8_lossy(&output.stdout);
        let Some(installation) = installation.lines().find(|line| !line.trim().is_empty()) else {
            continue;
        };
        let candidate = PathBuf::from(installation.trim())
            .join("Common7")
            .join("Tools")
            .join("VsDevCmd.bat");
        if candidate.is_file() {
            return Ok(Some(candidate));
        }
    }

    Ok(None)
}

#[cfg(windows)]
fn parse_utf16le_environment(bytes: &[u8]) -> Result<Vec<(OsString, OsString)>, String> {
    let text = decode_utf16le(bytes)?;

    Ok(text
        .lines()
        .filter_map(|line| {
            let (name, value) = line.split_once('=')?;
            (!name.is_empty()).then(|| (OsString::from(name), OsString::from(value)))
        })
        .collect())
}

#[cfg(windows)]
fn decode_utf16le(bytes: &[u8]) -> Result<String, String> {
    if !bytes.len().is_multiple_of(2) {
        return Err("Visual Studio environment output was not valid UTF-16LE".to_string());
    }
    let mut words = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>();
    if words.first() == Some(&0xfeff) {
        words.remove(0);
    }
    String::from_utf16(&words)
        .map_err(|error| format!("Visual Studio environment output was not valid UTF-16: {error}"))
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
        // Make Doria's process wrapper the executable entrypoint instead of
        // relying on CRT startup. On normal completion the wrapper calls
        // dr_v1_exit_process, so Windows does not rely on return-from-entrypoint
        // behavior to preserve the Doria process status.
        // doria-rt owns the small MSVC support surface used by generated code,
        // including LLVM's x86-64 stack-probe helper, so no C runtime is linked.
        return vec![
            OsString::from("/nologo"),
            object_path.as_os_str().to_os_string(),
            runtime_path.as_os_str().to_os_string(),
            OsString::from(format!("/Fe:{}", executable_path.display())),
            OsString::from("/link"),
            OsString::from("/ENTRY:main"),
            OsString::from("/SUBSYSTEM:CONSOLE"),
            OsString::from("kernel32.lib"),
            // `doria_rt.lib` is a staticlib, so it carries archived objects
            // without the crate metadata that records a `#[link]` dependency.
            // The entry glue's `CommandLineToArgvW` therefore has to be named
            // here; shell32 is not in the default library set.
            OsString::from("shell32.lib"),
        ];
    }

    let mut arguments = vec![
        object_path.as_os_str().to_os_string(),
        runtime_path.as_os_str().to_os_string(),
        OsString::from("-o"),
        executable_path.as_os_str().to_os_string(),
    ];
    if windows {
        arguments.push(OsString::from("-lshell32"));
    }
    arguments
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
                OsString::from("kernel32.lib"),
                OsString::from("shell32.lib"),
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
                OsString::from("kernel32.lib"),
                OsString::from("shell32.lib"),
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
                OsString::from("-lshell32"),
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
                OsString::from("-lshell32"),
            ]
        );
        assert_eq!(default_linker(false), "cc");
        assert_eq!(default_linker(true), "cl.exe");
    }

    #[cfg(windows)]
    #[test]
    fn visual_studio_environment_parser_preserves_values_and_skips_drive_entries() {
        let text = "PATH=C:\\Tools\r\nLIB=C:\\SDK=Preview\r\n=C:=C:\\repo\r\n";
        let bytes = text
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();

        assert_eq!(
            parse_utf16le_environment(&bytes).expect("parse environment"),
            vec![
                (OsString::from("PATH"), OsString::from("C:\\Tools")),
                (OsString::from("LIB"), OsString::from("C:\\SDK=Preview")),
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn executable_lookup_uses_the_discovered_path_case_insensitively() {
        let temporary = unique_temp_stem();
        fs::create_dir(&temporary).expect("create executable probe directory");
        let executable = temporary.join("doriac-discovery-test.exe");
        fs::write(&executable, []).expect("write executable probe");
        let environment = vec![(OsString::from("Path"), temporary.clone().into_os_string())];

        assert_eq!(
            executable_from_environment("doriac-discovery-test.exe", &environment),
            Some(executable.clone())
        );

        fs::remove_dir_all(temporary).expect("remove executable probe directory");
    }

    #[cfg(windows)]
    #[test]
    fn installed_visual_studio_environment_contains_the_default_linker() {
        let environment = match discover_msvc_environment() {
            Ok(Some(environment)) => environment,
            Ok(None) => {
                eprintln!("Visual Studio C++ build tools are not installed; skipping discovery");
                return;
            }
            Err(error) => panic!("discover Visual Studio C++ build tools: {error}"),
        };

        assert!(
            executable_from_environment("cl.exe", &environment).is_some(),
            "the discovered Visual Studio PATH should contain cl.exe"
        );
    }
}
