use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, ExitStatus};
use std::str::FromStr;

use doriac::backend::{BackendOutput, BackendTarget, CompileOptions, NativeProfile};

fn main() -> ExitCode {
    match run() {
        Ok(exit_code) => exit_code,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        print_help();
        return Ok(ExitCode::SUCCESS);
    }
    if args[0] == "--version" || args[0] == "-V" {
        println!("doriac {}", doriac::TOOLCHAIN_VERSION);
        return Ok(ExitCode::SUCCESS);
    }

    match args[0].as_str() {
        "check" => {
            let input = args
                .get(1)
                .ok_or_else(|| "missing input file".to_string())?;
            let json = match args.get(2).map(String::as_str) {
                None => false,
                Some("--json") => true,
                Some(option) => return Err(format!("unknown check option `{option}`")),
            };
            if let Some(option) = args.get(3) {
                return Err(format!("unknown check option `{option}`"));
            }
            let (path, text) = read_source(input)?;
            match doriac::check_source(path.clone(), text.clone()) {
                Ok(_) => {
                    if json {
                        println!("[]");
                    } else {
                        println!("OK");
                    }
                    Ok(ExitCode::SUCCESS)
                }
                Err(diagnostics) if json => Err(doriac::diagnostics_json(&diagnostics)),
                Err(diagnostics) => Err(doriac::render_diagnostics(path, text, &diagnostics)),
            }
        }
        "ast" => ast_command(&args[1..]).map(|()| ExitCode::SUCCESS),
        "hir" => hir_command(&args[1..]).map(|()| ExitCode::SUCCESS),
        "mir" => mir_command(&args[1..]).map(|()| ExitCode::SUCCESS),
        "compile" => compile_command(&args[1..]).map(|()| ExitCode::SUCCESS),
        "run" => run_command(&args[1..]),
        command => Err(format!(
            "unknown command `{command}`\n\nRun `doriac --help`."
        )),
    }
}

fn compile_command(args: &[String]) -> Result<(), String> {
    let input = args
        .first()
        .ok_or_else(|| "missing input file".to_string())?;
    let mut target = BackendTarget::Native;
    let mut release = false;
    let mut out = None::<String>;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--target" => {
                let target_value = args
                    .get(index + 1)
                    .ok_or_else(|| "missing value for --target".to_string())?
                    .clone();
                target = BackendTarget::from_str(&target_value)?;
                index += 2;
            }
            "--out" => {
                out = Some(
                    args.get(index + 1)
                        .ok_or_else(|| "missing value for --out".to_string())?
                        .clone(),
                );
                index += 2;
            }
            "--release" => {
                release = true;
                index += 1;
            }
            flag => return Err(format!("unknown compile option `{flag}`")),
        }
    }

    if release && target != BackendTarget::Native {
        return Err("--release is only valid for the native target".to_string());
    }

    if !target.is_available() {
        return Err(format!(
            "target `{}` ({}) is planned but not implemented yet; available targets are `native`, `php`, and `debug`",
            target.name(),
            target.description()
        ));
    }

    let (path, text) = read_source(input)?;
    let out_path = match out {
        Some(out) => PathBuf::from(out),
        None => default_output_path(input, target)?,
    };
    let options = CompileOptions {
        target,
        native_profile: if release {
            NativeProfile::Release
        } else {
            NativeProfile::Fast
        },
    };
    let output = doriac::compile_source_with_options(path.clone(), text.clone(), options)
        .map_err(|diagnostics| doriac::render_diagnostics(path, text, &diagnostics))?;

    write_backend_output(&out_path, output)?;
    println!("{}", out_path.display());
    Ok(())
}

fn ast_command(args: &[String]) -> Result<(), String> {
    let input = args
        .first()
        .ok_or_else(|| "missing input file".to_string())?;
    let (path, text) = read_source(input)?;
    let ast = doriac::parse_source(path.clone(), text.clone())
        .map_err(|diagnostics| doriac::render_diagnostics(path, text, &diagnostics))?;
    println!("{ast:#?}");
    Ok(())
}

fn hir_command(args: &[String]) -> Result<(), String> {
    let input = args
        .first()
        .ok_or_else(|| "missing input file".to_string())?;
    let (path, text) = read_source(input)?;
    let hir = doriac::lower_source(path.clone(), text.clone())
        .map_err(|diagnostics| doriac::render_diagnostics(path, text, &diagnostics))?;
    println!("{hir:#?}");
    Ok(())
}

fn mir_command(args: &[String]) -> Result<(), String> {
    let input = args
        .first()
        .ok_or_else(|| "missing input file".to_string())?;
    let (path, text) = read_source(input)?;
    let mir = doriac::lower_source_to_mir(path.clone(), text.clone())
        .map_err(|diagnostics| doriac::render_diagnostics(path, text, &diagnostics))?;
    print!("{mir}");
    Ok(())
}

fn write_backend_output(out_path: &Path, output: BackendOutput) -> Result<(), String> {
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create output directory: {error}"))?;
        }
    }

    match output {
        BackendOutput::Text { contents, .. } => fs::write(out_path, contents)
            .map_err(|error| format!("failed to write output file: {error}")),
        BackendOutput::Binary { bytes, .. } => fs::write(out_path, bytes)
            .map_err(|error| format!("failed to write output file: {error}")),
        BackendOutput::Executable { bytes, .. } => {
            fs::write(out_path, bytes)
                .map_err(|error| format!("failed to write output file: {error}"))?;
            make_executable(out_path)
        }
        BackendOutput::Artifact { path } => {
            fs::copy(&path, out_path)
                .map_err(|error| format!("failed to copy backend artifact: {error}"))?;
            Ok(())
        }
    }
}

fn default_output_path(input: &str, target: BackendTarget) -> Result<PathBuf, String> {
    let stem = Path::new(input)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .ok_or_else(|| format!("cannot infer output file name from `{input}`"))?;

    let extension = default_output_extension(target);
    let mut file_name = stem.to_string();
    if !extension.is_empty() {
        file_name.push('.');
        file_name.push_str(extension);
    }

    let output_path = PathBuf::from(file_name);
    if inferred_output_aliases_input(input, &output_path)? {
        return Err(format!(
            "inferred output path `{}` would overwrite input `{}`; pass --out <file> to choose a different output path",
            output_path.display(),
            input
        ));
    }

    Ok(output_path)
}

fn inferred_output_aliases_input(input: &str, output_path: &Path) -> Result<bool, String> {
    let input_path = Path::new(input);
    let input_canonical = fs::canonicalize(input_path)
        .map_err(|error| format!("failed to resolve input path `{input}`: {error}"))?;

    if let Ok(output_canonical) = fs::canonicalize(output_path) {
        return Ok(output_canonical == input_canonical);
    }

    let output_absolute = if output_path.is_absolute() {
        output_path.to_path_buf()
    } else {
        env::current_dir()
            .map_err(|error| format!("failed to resolve current directory: {error}"))?
            .join(output_path)
    };

    Ok(output_absolute == input_canonical)
}

fn default_output_extension(target: BackendTarget) -> &'static str {
    match target {
        BackendTarget::Native => {
            if cfg!(windows) {
                "exe"
            } else {
                ""
            }
        }
        BackendTarget::Php => "php",
        BackendTarget::Debug => "debug",
        BackendTarget::Wasm => "wasm",
    }
}

fn run_command(args: &[String]) -> Result<ExitCode, String> {
    let input = args
        .first()
        .ok_or_else(|| "missing input file".to_string())?;
    let mut release = false;
    for option in &args[1..] {
        match option.as_str() {
            "--release" => release = true,
            option => return Err(format!("unknown run option `{option}`")),
        }
    }

    let (path, text) = read_source(input)?;
    let profile = if release {
        NativeProfile::Release
    } else {
        NativeProfile::Fast
    };
    let output = doriac::compile_source_with_options(
        path.clone(),
        text.clone(),
        CompileOptions::native(profile),
    )
    .map_err(|diagnostics| doriac::render_diagnostics(path, text, &diagnostics))?;

    let temp_path = temp_run_executable_path(input);
    write_backend_output(&temp_path, output)
        .map_err(|error| format!("failed to write temp native executable: {error}"))?;

    let status = Command::new(&temp_path).status().map_err(|error| {
        format!(
            "failed to run native executable `{}`: {error}",
            temp_path.display()
        )
    })?;

    let _ = fs::remove_file(&temp_path);
    exit_code_from_status(status)
}

fn temp_run_executable_path(input: &str) -> PathBuf {
    let stem = Path::new(input)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("doriac-run");
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let extension = if cfg!(windows) { ".exe" } else { "" };
    env::temp_dir().join(format!(
        "doriac-run-{stem}-{}-{nanos}{extension}",
        std::process::id()
    ))
}

fn exit_code_from_status(status: ExitStatus) -> Result<ExitCode, String> {
    if let Some(code) = status.code() {
        let code = u8::try_from(code).unwrap_or(1);
        Ok(ExitCode::from(code))
    } else {
        Err(format!(
            "native executable terminated without an exit code: {status}"
        ))
    }
}

fn read_source(path: impl AsRef<Path>) -> Result<(String, String), String> {
    let path = path.as_ref();
    let text = fs::read_to_string(path).map_err(|error| {
        if error.kind() == ErrorKind::InvalidData {
            format!(
                "failed to read `{}` as Doria source: Doria source files must be valid UTF-8.\n`doriac run` expects a `.doria` source file. To run a compiled executable, run it directly: `{}`",
                path.display(),
                direct_executable_hint(path)
            )
        } else {
            format!("failed to read `{}`: {error}", path.display())
        }
    })?;
    Ok((path.display().to_string(), text))
}

fn direct_executable_hint(path: &Path) -> String {
    let display = path.display().to_string();
    if !path.is_absolute() && path.components().count() == 1 {
        format!(".{}{display}", std::path::MAIN_SEPARATOR)
    } else {
        display
    }
}

fn print_help() {
    println!(
        "doriac {}\n\nUSAGE:\n    doriac check <source.doria> [--json]\n    doriac ast <source.doria>\n    doriac hir <source.doria>\n    doriac mir <source.doria>\n    doriac compile <source.doria> [--release] [--out <file>]\n    doriac compile <source.doria> --target php [--out <file>]\n    doriac run <source.doria> [--release]\n\nNATIVE PROFILES:\n    fast       default Cranelift profile for rapid local feedback\n    release    LLVM optimized profile selected with --release\n\nTARGETS:\n    native    default target for standalone executables\n    php       compatibility and inspection backend\n    debug     MIR interpreter debug artifact\n    wasm      planned WebAssembly backend",
        doriac::TOOLCHAIN_VERSION
    );
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .map_err(|error| format!("failed to read output permissions: {error}"))?
        .permissions();
    permissions.set_mode(permissions.mode() | 0o111);
    fs::set_permissions(path, permissions)
        .map_err(|error| format!("failed to mark output executable: {error}"))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}
