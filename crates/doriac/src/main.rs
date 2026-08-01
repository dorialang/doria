use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, ExitStatus};
use std::str::FromStr;

use doriac::backend::{BackendOutput, BackendTarget, CompileOptions, NativeProfile};
use doriac::diagnostics::{
    ColorChoice, Diagnostic, DiagnosticFormat, DiagnosticSource, RenderOptions, RuntimeFact,
    RuntimeFactValue, RuntimeOutcomeDetails, RuntimeOutcomeFrame, RuntimeOutcomeOrigin,
    TerminationBehavior,
};
use doriac::source::Span;

#[derive(Debug)]
enum CliError {
    Message(String),
    Diagnostics {
        path: String,
        text: String,
        diagnostics: Vec<Diagnostic>,
        options: RenderOptions,
    },
}

impl CliError {
    fn diagnostics(
        path: String,
        text: String,
        diagnostics: Vec<Diagnostic>,
        options: RenderOptions,
    ) -> Self {
        Self::Diagnostics {
            path,
            text,
            diagnostics,
            options,
        }
    }

    fn emit(self) {
        match self {
            Self::Message(message) => eprintln!("Error: {message}"),
            Self::Diagnostics {
                path,
                text,
                diagnostics,
                options,
            } => {
                let rendered =
                    doriac::render_diagnostics_with_options(path, text, &diagnostics, options);
                if options.format == DiagnosticFormat::Json {
                    println!("{rendered}");
                } else {
                    eprintln!("{rendered}");
                }
            }
        }
    }
}

impl From<String> for CliError {
    fn from(message: String) -> Self {
        Self::Message(message)
    }
}

impl From<&str> for CliError {
    fn from(message: &str) -> Self {
        Self::Message(message.to_string())
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(exit_code) => exit_code,
        Err(error) => {
            error.emit();
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode, CliError> {
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    let Some(command) = args.first() else {
        print_help();
        return Ok(ExitCode::SUCCESS);
    };
    let command = command
        .to_str()
        .ok_or_else(|| "doriac command must be valid UTF-8".to_string())?;
    if command == "--help" || command == "-h" {
        print_help();
        return Ok(ExitCode::SUCCESS);
    }
    if command == "--version" || command == "-V" {
        return version_command(&args[1..]).map_err(Into::into);
    }
    if command == "run" {
        return run_command(&args[1..]);
    }

    let args = utf8_cli_arguments(&args)?;
    match command {
        "check" => {
            let (args, options) = parse_diagnostic_options(&args[1..])?;
            let input = args
                .first()
                .ok_or_else(|| "missing input file".to_string())?;
            if let Some(option) = args.get(1) {
                return Err(format!("unknown check option `{option}`").into());
            }
            let (path, text) = read_source(input)?;
            match doriac::check_source(path.clone(), text.clone()) {
                Ok(_) => {
                    if options.format == DiagnosticFormat::Json {
                        println!(
                            "{}",
                            doriac::render_diagnostics_with_options(path, text, &[], options)
                        );
                    } else {
                        println!("OK");
                    }
                    Ok(ExitCode::SUCCESS)
                }
                Err(diagnostics) => Err(CliError::diagnostics(path, text, diagnostics, options)),
            }
        }
        "ast" => ast_command(&args[1..]).map(|()| ExitCode::SUCCESS),
        "hir" => hir_command(&args[1..]).map(|()| ExitCode::SUCCESS),
        "mir" => mir_command(&args[1..]).map(|()| ExitCode::SUCCESS),
        "compile" => compile_command(&args[1..]).map(|()| ExitCode::SUCCESS),
        command if command.ends_with(".doria") || Path::new(command).is_file() => Err(format!(
            "unknown command `{command}`\n\n\
             `{command}` looks like a source file, and the command comes first. Did you mean:\n    \
             doriac compile {command} --out <file>   # build a native executable\n    \
             doriac run {command}                    # compile and run it\n\n\
             Run `doriac --help` for all commands."
        )
        .into()),
        command => Err(format!("unknown command `{command}`\n\nRun `doriac --help`.").into()),
    }
}

fn version_command(args: &[OsString]) -> Result<ExitCode, String> {
    let args = utf8_cli_arguments(args)?;
    match args.as_slice() {
        [] => println!("doriac {}", doriac::TOOLCHAIN_VERSION),
        [option] if option == "--json" => {
            let target = format!(
                "{}-{}",
                normalized_platform(std::env::consts::OS),
                normalized_architecture(std::env::consts::ARCH)
            );
            let identity = serde_json::json!({
                "schema": 1,
                "component": "doriac",
                "toolchainVersion": doriac::TOOLCHAIN_VERSION,
                "target": target,
                "commit": doriac::BUILD_COMMIT,
            });
            println!(
                "{}",
                serde_json::to_string(&identity)
                    .map_err(|error| format!("failed to encode version metadata: {error}"))?
            );
        }
        [option] => return Err(format!("unknown version option `{option}`")),
        _ => return Err("too many version options".to_string()),
    }

    Ok(ExitCode::SUCCESS)
}

fn normalized_platform(platform: &str) -> &str {
    match platform {
        "macos" => "macos",
        "windows" => "windows",
        "linux" => "linux",
        other => other,
    }
}

fn normalized_architecture(architecture: &str) -> &str {
    match architecture {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => other,
    }
}

fn utf8_cli_arguments(args: &[OsString]) -> Result<Vec<String>, String> {
    args.iter()
        .map(|argument| {
            argument
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| "doriac commands and options must be valid UTF-8".to_string())
        })
        .collect()
}

fn compile_command(args: &[String]) -> Result<(), CliError> {
    let (args, diagnostic_options) = parse_diagnostic_options(args)?;
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
            flag => return Err(format!("unknown compile option `{flag}`").into()),
        }
    }

    if release && target != BackendTarget::Native {
        return Err("--release is only valid for the native target".into());
    }

    if !target.is_available() {
        return Err(format!(
            "target `{}` ({}) is planned but not implemented yet; available targets are `native`, `php`, and `debug`",
            target.name(),
            target.description()
        )
        .into());
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
    let output = doriac::compile_source_with_options(path.clone(), text.clone(), options).map_err(
        |diagnostics| CliError::diagnostics(path, text, diagnostics, diagnostic_options),
    )?;

    write_backend_output(&out_path, output)?;
    println!("{}", out_path.display());
    Ok(())
}

fn ast_command(args: &[String]) -> Result<(), CliError> {
    let (args, diagnostic_options) = parse_diagnostic_options(args)?;
    let input = args
        .first()
        .ok_or_else(|| "missing input file".to_string())?;
    let (path, text) = read_source(input)?;
    let ast = doriac::parse_source(path.clone(), text.clone()).map_err(|diagnostics| {
        CliError::diagnostics(path, text, diagnostics, diagnostic_options)
    })?;
    println!("{ast:#?}");
    Ok(())
}

fn hir_command(args: &[String]) -> Result<(), CliError> {
    let (args, diagnostic_options) = parse_diagnostic_options(args)?;
    let input = args
        .first()
        .ok_or_else(|| "missing input file".to_string())?;
    let (path, text) = read_source(input)?;
    let hir = doriac::lower_source(path.clone(), text.clone()).map_err(|diagnostics| {
        CliError::diagnostics(path, text, diagnostics, diagnostic_options)
    })?;
    println!("{hir:#?}");
    Ok(())
}

fn mir_command(args: &[String]) -> Result<(), CliError> {
    let (args, diagnostic_options) = parse_diagnostic_options(args)?;
    let input = args
        .first()
        .ok_or_else(|| "missing input file".to_string())?;
    let (path, text) = read_source(input)?;
    let mir = doriac::lower_source_to_mir(path.clone(), text.clone()).map_err(|diagnostics| {
        CliError::diagnostics(path, text, diagnostics, diagnostic_options)
    })?;
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

fn run_command(args: &[OsString]) -> Result<ExitCode, CliError> {
    let separator = args
        .iter()
        .position(|argument| argument == OsStr::new("--"));
    let (compiler_args, program_args) = match separator {
        Some(index) => (&args[..index], &args[index + 1..]),
        None => (args, &[][..]),
    };
    let compiler_args = utf8_cli_arguments(compiler_args)?;
    let (compiler_args, diagnostic_options) = parse_diagnostic_options(&compiler_args)?;
    let input = compiler_args
        .first()
        .ok_or_else(|| "missing input file".to_string())?;
    let mut release = false;
    // Everything after `--` belongs to the program, not to `doriac`, so a
    // program argument can safely look like a compiler option (decision 0099).
    for option in &compiler_args[1..] {
        match option.as_str() {
            "--release" => release = true,
            option => return Err(format!("unknown run option `{option}`").into()),
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
    .map_err(|diagnostics| {
        CliError::diagnostics(path.clone(), text.clone(), diagnostics, diagnostic_options)
    })?;

    let temp_path = temp_run_executable_path(input);
    let outcome_path = temp_path.with_extension("doria-outcome-v2");
    write_backend_output(&temp_path, output)
        .map_err(|error| format!("failed to write temp native executable: {error}"))?;

    let status = Command::new(&temp_path)
        .env("DORIA_RUNTIME_OUTCOME_V2", &outcome_path)
        .args(program_args)
        .status()
        .map_err(|error| {
            format!(
                "failed to run native executable `{}`: {error}",
                temp_path.display()
            )
        })?;

    let runtime_payload = fs::read(&outcome_path).ok();
    let _ = fs::remove_file(&outcome_path);
    let _ = fs::remove_file(&temp_path);
    let runtime_diagnostic = runtime_payload
        .map(|payload| decode_runtime_outcome(&payload, &path))
        .transpose()?;
    if let Some(diagnostic) = runtime_diagnostic {
        let diagnostic_options = RenderOptions {
            context_lines: 0,
            ..diagnostic_options
        };
        let rendered =
            doriac::render_diagnostics_with_options(path, text, &[diagnostic], diagnostic_options);
        if diagnostic_options.format == DiagnosticFormat::Json {
            println!("{rendered}");
        } else {
            eprintln!("{rendered}");
        }
    }
    Ok(exit_code_from_status(status)?)
}

fn decode_runtime_outcome(payload: &[u8], source_path: &str) -> Result<Diagnostic, CliError> {
    let mut decoder = RuntimeOutcomeDecoder::new(payload);
    if decoder.take(8)? != b"DORIAO2\0" || decoder.u16()? != 2 {
        return Err("native program returned an unsupported runtime outcome record".into());
    }
    let code_length = usize::from(decoder.u16()?);
    let message_length = decoder.u32()? as usize;
    let path_length = decoder.u32()? as usize;
    let source_length = decoder.u32()? as usize;
    let function_length = usize::from(decoder.u16()?);
    let frame_count = usize::from(decoder.u16()?);
    let fact_count = usize::from(decoder.u16()?);
    if code_length > 16
        || message_length > 64 * 1024
        || path_length > 4096
        || source_length > 4 * 1024 * 1024
        || function_length > 1024
        || frame_count > 128
        || fact_count > 32
    {
        return Err("native program returned an oversized runtime outcome record".into());
    }
    let primary_span = Span::new(decoder.u64()? as usize, decoder.u64()? as usize);
    let code = decoder.text(code_length)?;
    let message = decoder.text(message_length)?;
    let record_path = decoder.text(path_length)?;
    let _embedded_source = decoder.text(source_length)?;
    let function = decoder.text(function_length)?;
    let catalogue_entry = doriac::diagnostics::runtime_catalogue_entry(&code)
        .ok_or_else(|| "native program returned an unknown runtime diagnostic code".to_string())?;
    let mut facts = Vec::with_capacity(fact_count);
    for _ in 0..fact_count {
        let name_length = usize::from(decoder.u16()?);
        let kind = decoder.byte()?;
        let value = decoder.u64()?;
        let value_length = decoder.u32()? as usize;
        if name_length > 1024 || value_length > 64 * 1024 {
            return Err("native program returned an oversized runtime fact".into());
        }
        let name = decoder.text(name_length)?;
        let value = match kind {
            1 if value_length == 0 => RuntimeFactValue::Signed(value as i64),
            2 if value_length == 0 => RuntimeFactValue::Unsigned(value),
            3 if value_length == 0 && value <= 1 => RuntimeFactValue::Boolean(value != 0),
            4 => RuntimeFactValue::StaticString(decoder.text(value_length)?),
            1..=3 => return Err("native program returned a malformed runtime fact".into()),
            _ => return Err("native program returned an unknown runtime fact type".into()),
        };
        facts.push(RuntimeFact { name, value });
    }
    if code == "P1501"
        && !facts.iter().any(|fact| {
            fact.name == doria_diagnostic_catalogue::SHARED_ACCESS_CONFLICT_REASON_FACT
                && matches!(
                    &fact.value,
                    RuntimeFactValue::StaticString(value)
                        if doria_diagnostic_catalogue::is_shared_access_conflict_reason(value)
                )
        })
    {
        return Err("native program returned an invalid shared-access conflict reason".into());
    }
    if code == "P1203"
        && !facts.iter().any(|fact| {
            fact.name == doria_diagnostic_catalogue::STRING_PADDING_OPERATION_FACT
                && matches!(
                    &fact.value,
                    RuntimeFactValue::StaticString(value)
                        if doria_diagnostic_catalogue::is_string_padding_operation(value)
                )
        })
    {
        return Err("native program returned an invalid string-padding operation".into());
    }
    let transport_fact_names = if code == "P1000" {
        catalogue_entry
            .fact_names
            .strip_prefix(&["message"])
            .expect("P1000 message fact schema")
    } else {
        catalogue_entry.fact_names
    };
    if facts.len() != transport_fact_names.len()
        || facts
            .iter()
            .zip(transport_fact_names)
            .any(|(actual, expected)| actual.name != *expected)
    {
        return Err(
            "native program returned facts that do not match the diagnostic catalogue".into(),
        );
    }
    let mut frames = Vec::with_capacity(frame_count);
    for _ in 0..frame_count {
        let frame_function_length = usize::from(decoder.u16()?);
        let frame_path_length = decoder.u32()? as usize;
        let span = Span::new(decoder.u64()? as usize, decoder.u64()? as usize);
        if frame_function_length > 1024 || frame_path_length > 4096 {
            return Err("native program returned an oversized runtime frame".into());
        }
        let frame_function = decoder.text(frame_function_length)?;
        let frame_path = decoder.text(frame_path_length)?;
        frames.push(RuntimeOutcomeFrame {
            function: frame_function,
            source: diagnostic_source(&frame_path, source_path),
            span,
        });
    }
    if !decoder.is_empty() {
        return Err("native program returned trailing runtime outcome bytes".into());
    }

    let code = catalogue_entry.code;
    if code == "P1000" && !message.is_empty() {
        facts.push(RuntimeFact {
            name: "message".to_string(),
            value: RuntimeFactValue::StaticString(message.clone()),
        });
    }
    let outcome = RuntimeOutcomeDetails {
        process_status: 101,
        termination_behavior: TerminationBehavior::AbortWithoutCleanup,
        origin: RuntimeOutcomeOrigin {
            source: diagnostic_source(&record_path, source_path),
            span: primary_span,
            function: Some(function),
        },
        path: frames,
        facts,
    };
    let mut diagnostic = Diagnostic::runtime_panic(code, primary_span, outcome);
    if code == "P1203" {
        let operation = diagnostic
            .runtime_outcome
            .as_ref()
            .and_then(|outcome| {
                outcome.facts.iter().find(|fact| {
                    fact.name == doria_diagnostic_catalogue::STRING_PADDING_OPERATION_FACT
                })
            })
            .and_then(|fact| match &fact.value {
                RuntimeFactValue::StaticString(value) => Some(value.as_str()),
                _ => None,
            });
        let text = diagnostic
            .runtime_outcome
            .as_ref()
            .and_then(|outcome| {
                outcome
                    .facts
                    .iter()
                    .find(|fact| fact.name == doria_diagnostic_catalogue::STRING_PADDING_VALUE_FACT)
            })
            .and_then(|fact| match &fact.value {
                RuntimeFactValue::StaticString(value) => Some(value.as_str()),
                _ => None,
            });
        let unsigned = |name: &str| {
            diagnostic
                .runtime_outcome
                .as_ref()
                .and_then(|outcome| outcome.facts.iter().find(|fact| fact.name == name))
                .and_then(|fact| match fact.value {
                    RuntimeFactValue::Unsigned(value) => Some(value),
                    RuntimeFactValue::Signed(value) => u64::try_from(value).ok(),
                    _ => None,
                })
        };
        if let (Some(operation), Some(value), Some(current), Some(requested), Some(_padding)) = (
            operation,
            text,
            unsigned(doria_diagnostic_catalogue::STRING_PADDING_CURRENT_LENGTH_FACT),
            unsigned(doria_diagnostic_catalogue::STRING_PADDING_REQUESTED_GRAPHEME_LENGTH_FACT),
            unsigned(doria_diagnostic_catalogue::STRING_PADDING_PADDING_LENGTH_FACT),
        ) {
            diagnostic.explanation = Some(format!(
                "`{operation}` was asked to extend `\"{value}\"` from {current} to {requested} graphemes,\nbut an empty padding string cannot add any graphemes."
            ));
        }
    }
    if code == "P1000" && !message.is_empty() {
        diagnostic.notes.push(message);
    }
    Ok(diagnostic)
}

fn diagnostic_source(path: &str, current: &str) -> DiagnosticSource {
    if path == current {
        DiagnosticSource::Current
    } else {
        DiagnosticSource::Path(path.to_string())
    }
}

struct RuntimeOutcomeDecoder<'a> {
    remaining: &'a [u8],
}

impl<'a> RuntimeOutcomeDecoder<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { remaining: bytes }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], CliError> {
        if length > self.remaining.len() {
            return Err("native program returned a truncated runtime outcome record".into());
        }
        let (value, remaining) = self.remaining.split_at(length);
        self.remaining = remaining;
        Ok(value)
    }

    fn u16(&mut self) -> Result<u16, CliError> {
        let bytes: [u8; 2] = self.take(2)?.try_into().expect("fixed length");
        Ok(u16::from_le_bytes(bytes))
    }

    fn byte(&mut self) -> Result<u8, CliError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, CliError> {
        let bytes: [u8; 4] = self.take(4)?.try_into().expect("fixed length");
        Ok(u32::from_le_bytes(bytes))
    }

    fn u64(&mut self) -> Result<u64, CliError> {
        let bytes: [u8; 8] = self.take(8)?.try_into().expect("fixed length");
        Ok(u64::from_le_bytes(bytes))
    }

    fn text(&mut self, length: usize) -> Result<String, CliError> {
        String::from_utf8(self.take(length)?.to_vec())
            .map_err(|_| "native program returned invalid UTF-8 in a runtime outcome".into())
    }

    fn is_empty(&self) -> bool {
        self.remaining.is_empty()
    }
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
        "doriac {}\n\nUSAGE:\n    doriac check <source.doria> [diagnostic options]\n    doriac ast <source.doria> [diagnostic options]\n    doriac hir <source.doria> [diagnostic options]\n    doriac mir <source.doria> [diagnostic options]\n    doriac compile <source.doria> [--release] [--out <file>] [diagnostic options]\n    doriac compile <source.doria> --target php [--out <file>] [diagnostic options]\n    doriac run <source.doria> [--release] [diagnostic options] [-- <program args>...]\n\nDIAGNOSTIC OPTIONS:\n    --diagnostic-format human|concise|json    default: human\n    --diagnostic-color auto|always|never      default: auto; NO_COLOR disables auto color\n\nHuman and concise diagnostics are written to stderr. Versioned JSON diagnostics are written to stdout.\n\nNATIVE PROFILES:\n    fast       default Cranelift profile for rapid local feedback\n    release    LLVM optimized profile selected with --release\n\nTARGETS:\n    native    default target for standalone executables\n    php       compatibility and inspection backend\n    debug     MIR interpreter debug artifact\n    wasm      planned WebAssembly backend",
        doriac::TOOLCHAIN_VERSION
    );
}

fn parse_diagnostic_options(args: &[String]) -> Result<(Vec<String>, RenderOptions), String> {
    let mut options = RenderOptions {
        terminal_width: env::var("COLUMNS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(100),
        ..RenderOptions::default()
    };
    let mut positional = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => {
                options.format = DiagnosticFormat::Json;
                index += 1;
            }
            "--diagnostic-format" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "missing value for --diagnostic-format".to_string())?;
                options.format = DiagnosticFormat::parse(value).ok_or_else(|| {
                    format!(
                        "unknown diagnostic format `{value}`; expected `human`, `concise`, or `json`"
                    )
                })?;
                index += 2;
            }
            "--diagnostic-color" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "missing value for --diagnostic-color".to_string())?;
                options.color = ColorChoice::parse(value).ok_or_else(|| {
                    format!(
                        "unknown diagnostic color `{value}`; expected `auto`, `always`, or `never`"
                    )
                })?;
                index += 2;
            }
            argument => {
                positional.push(argument.to_string());
                index += 1;
            }
        }
    }
    Ok((positional, options))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime_record(code: &str, facts: &[(&str, u8, u64, &str)]) -> Vec<u8> {
        let path = "main.doria";
        let source = "function main(): void {}\n";
        let function = "main";
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"DORIAO2\0");
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&(code.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&(path.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(source.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(function.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&(facts.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&0_u64.to_le_bytes());
        bytes.extend_from_slice(&8_u64.to_le_bytes());
        bytes.extend_from_slice(code.as_bytes());
        bytes.extend_from_slice(path.as_bytes());
        bytes.extend_from_slice(source.as_bytes());
        bytes.extend_from_slice(function.as_bytes());
        for (name, kind, scalar, text) in facts {
            bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
            bytes.push(*kind);
            bytes.extend_from_slice(&scalar.to_le_bytes());
            bytes.extend_from_slice(&(text.len() as u32).to_le_bytes());
            bytes.extend_from_slice(name.as_bytes());
            bytes.extend_from_slice(text.as_bytes());
        }
        bytes
    }

    fn decode_error(payload: &[u8]) -> String {
        match decode_runtime_outcome(payload, "main.doria") {
            Err(CliError::Message(message)) => message,
            Err(CliError::Diagnostics { .. }) => {
                panic!("transport decoding must not create a compilation diagnostic")
            }
            Ok(_) => panic!("malformed transport unexpectedly decoded"),
        }
    }

    #[test]
    fn runtime_transport_decodes_catalogued_facts() {
        let payload = runtime_record(
            "P1203",
            &[
                ("operation", 4, 0, "padStart"),
                ("value", 4, 0, "Doria"),
                ("currentGraphemeLength", 2, 5, ""),
                ("requestedGraphemeLength", 2, 8, ""),
                ("paddingGraphemeLength", 2, 0, ""),
            ],
        );
        let diagnostic = decode_runtime_outcome(&payload, "main.doria").expect("valid record");

        assert_eq!(diagnostic.code, "P1203");
        assert_eq!(diagnostic.title, "String Padding Text Cannot Be Empty");
        assert_eq!(
            diagnostic
                .runtime_outcome
                .as_ref()
                .expect("runtime outcome")
                .facts
                .len(),
            5
        );
        assert!(diagnostic
            .explanation
            .as_deref()
            .is_some_and(|explanation| explanation.starts_with("`padStart` was asked")));
    }

    #[test]
    fn runtime_transport_rejects_truncated_unknown_and_trailing_records() {
        assert!(decode_error(b"DORIA").contains("truncated"));

        let unknown = runtime_record("PX999", &[]);
        assert!(decode_error(&unknown).contains("unknown runtime diagnostic code"));

        let mut trailing = runtime_record("P1101", &[]);
        trailing.push(0);
        assert!(decode_error(&trailing).contains("trailing"));
    }

    #[test]
    fn runtime_transport_rejects_malformed_fact_schemas() {
        let missing_status = runtime_record("P1111", &[]);
        assert!(decode_error(&missing_status).contains("do not match"));

        let wrong_name = runtime_record("P1204", &[("length", 1, 1, "")]);
        assert!(decode_error(&wrong_name).contains("do not match"));

        let invalid_boolean = runtime_record("P1204", &[("count", 3, 2, "")]);
        assert!(decode_error(&invalid_boolean).contains("malformed runtime fact"));

        let unknown_kind = runtime_record("P1204", &[("count", 9, 0, "")]);
        assert!(decode_error(&unknown_kind).contains("unknown runtime fact type"));

        let missing_conflict_reason = runtime_record("P1501", &[]);
        assert!(decode_error(&missing_conflict_reason)
            .contains("invalid shared-access conflict reason"));

        let unknown_conflict_reason =
            runtime_record("P1501", &[("conflictReason", 4, 0, "Unknown Conflict")]);
        assert!(decode_error(&unknown_conflict_reason)
            .contains("invalid shared-access conflict reason"));
    }
}
