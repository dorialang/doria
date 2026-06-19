use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::str::FromStr;

use doriac::backend::{BackendOutput, BackendTarget};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        print_help();
        return Ok(());
    }

    match args[0].as_str() {
        "check" => {
            let input = args
                .get(1)
                .ok_or_else(|| "missing input file".to_string())?;
            let (path, text) = read_source(input)?;
            match doriac::check_source(path.clone(), text.clone()) {
                Ok(_) => {
                    println!("OK");
                    Ok(())
                }
                Err(diagnostics) => Err(doriac::render_diagnostics(path, text, &diagnostics)),
            }
        }
        "ast" => ast_command(&args[1..]),
        "hir" => hir_command(&args[1..]),
        "compile" => compile_command(&args[1..]),
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
    let mut target = None::<BackendTarget>;
    let mut out = None::<String>;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--target" => {
                let target_value = args
                    .get(index + 1)
                    .ok_or_else(|| "missing value for --target".to_string())?
                    .clone();
                target = Some(BackendTarget::from_str(&target_value)?);
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
            flag => return Err(format!("unknown compile option `{flag}`")),
        }
    }

    let target = target.ok_or_else(|| {
        "missing --target <target>; available targets are `native` and `php`".to_string()
    })?;

    if !target.is_available() {
        return Err(format!(
            "target `{}` ({}) is planned but not implemented yet; available targets are `native` and `php`",
            target.name(),
            target.description()
        ));
    }

    let out = out.ok_or_else(|| "missing --out <file>".to_string())?;
    let (path, text) = read_source(input)?;
    let output = doriac::compile_source(path.clone(), text.clone(), target)
        .map_err(|diagnostics| doriac::render_diagnostics(path, text, &diagnostics))?;

    let out_path = PathBuf::from(out);
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

fn run_command(args: &[String]) -> Result<(), String> {
    let input = args
        .first()
        .ok_or_else(|| "missing input file".to_string())?;
    let (path, text) = read_source(input)?;
    let php = doriac::compile_source_to_php(path.clone(), text.clone())
        .map_err(|diagnostics| doriac::render_diagnostics(path, text, &diagnostics))?;

    let temp_path = env::temp_dir().join("doriac-run.php");
    fs::write(&temp_path, php)
        .map_err(|error| format!("failed to write temp PHP file: {error}"))?;

    let status = Command::new("php")
        .arg(&temp_path)
        .status()
        .map_err(|error| format!("failed to run `php`: {error}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("php exited with status {status}"))
    }
}

fn read_source(path: impl AsRef<Path>) -> Result<(String, String), String> {
    let path = path.as_ref();
    let text = fs::read_to_string(path)
        .map_err(|error| format!("failed to read `{}`: {error}", path.display()))?;
    Ok((path.display().to_string(), text))
}

fn print_help() {
    println!(
        "doriac 0.1.0\n\nUSAGE:\n    doriac check <file>\n    doriac ast <file>\n    doriac hir <file>\n    doriac compile <file> --target <target> --out <file>\n    doriac run <file>\n\nTARGETS:\n    native    available Stage 1 Cranelift-backed fast native smoke backend\n    php       available compatibility backend\n    debug     planned interpreter/debug backend\n    wasm      planned WebAssembly backend"
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
