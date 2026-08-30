use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, ExitStatus};
use std::str::FromStr;
use std::time::Instant;

use doriac::backend::{BackendOutput, BackendTarget, CompileOptions, NativeProfile};
use doriac::diagnostics::{
    ColorChoice, Diagnostic, DiagnosticFormat, DiagnosticSource, LabelRole, RenderOptions,
    RuntimeFact, RuntimeFactValue, RuntimeOutcomeDetails, RuntimeOutcomeFrame,
    RuntimeOutcomeOrigin, TerminationBehavior,
};
use doriac::source::{SourceFile, SourceId, Span};

#[derive(Debug)]
enum CliError {
    Message(String),
    Diagnostics {
        path: String,
        text: String,
        diagnostics: Vec<Diagnostic>,
        options: RenderOptions,
    },
    GraphDiagnostics {
        sources: Box<doriac::source_map::SourceMap>,
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

    fn graph_diagnostics(
        sources: doriac::source_map::SourceMap,
        diagnostics: Vec<Diagnostic>,
        options: RenderOptions,
    ) -> Self {
        Self::GraphDiagnostics {
            sources: Box::new(sources),
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
            Self::GraphDiagnostics {
                sources,
                diagnostics,
                options,
            } => {
                let rendered = doriac::diagnostics::render_diagnostics_with_source_map(
                    &sources,
                    &diagnostics,
                    options,
                );
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
    let mut process_args = env::args_os();
    let executable = process_args
        .next()
        .unwrap_or_else(|| OsString::from("doriac"));
    let args = process_args.collect::<Vec<_>>();
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
            if let Some(plan_path) = build_plan_argument(&args)? {
                let (_, graph) = load_cli_graph(plan_path, options)?;
                let analysis = doriac::analyze_compilation_graph_for_ide(&graph);
                if analysis.diagnostics.is_empty() {
                    if options.format == DiagnosticFormat::Json {
                        println!(
                            "{}",
                            doriac::diagnostics::render_diagnostics_with_source_map(
                                &graph.source_map,
                                &[],
                                options,
                            )
                        );
                    } else {
                        println!("OK");
                    }
                    return Ok(ExitCode::SUCCESS);
                }
                return Err(CliError::graph_diagnostics(
                    graph.source_map,
                    analysis.diagnostics,
                    options,
                ));
            }
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
        "metadata" => metadata_command(&args[1..]).map(|()| ExitCode::SUCCESS),
        "compile" => compile_command(&executable, &args[1..]).map(|()| ExitCode::SUCCESS),
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
            let target = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
            // Which backends this binary was compiled with. Tooling that
            // orchestrates doriac (the benchmark harness, editors) reads this
            // to decide up front whether a build can serve `--release`,
            // instead of discovering the answer from a failed compile. The
            // LLVM backend is a build-time feature, so the binary itself is
            // the only authority on whether it is present.
            let backends = [
                "interpreter",
                "cranelift",
                "php",
                #[cfg(feature = "llvm-backend")]
                "llvm",
            ];
            let identity = serde_json::json!({
                "schema": 1,
                "component": "doriac",
                "toolchainVersion": doriac::TOOLCHAIN_VERSION,
                "target": target,
                "commit": doriac::BUILD_COMMIT,
                "backends": backends,
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

fn compile_command(executable: &OsStr, args: &[String]) -> Result<(), CliError> {
    let invocation = std::iter::once(executable.to_os_string())
        .chain(std::iter::once(OsString::from("compile")))
        .chain(args.iter().map(OsString::from))
        .collect::<Vec<_>>();
    let (args, diagnostic_options) = parse_diagnostic_options(args)?;
    let plan_path = build_plan_argument(&args)?;
    let input = if plan_path.is_some() {
        None
    } else {
        Some(
            args.first()
                .ok_or_else(|| "missing input file".to_string())?
                .as_str(),
        )
    };
    let mut target = BackendTarget::Native;
    let mut target_override = false;
    let mut release = false;
    let mut out = None::<String>;
    let mut performance_report = None::<String>;
    let mut index = if plan_path.is_some() { 2 } else { 1 };
    while index < args.len() {
        match args[index].as_str() {
            "--target" => {
                let target_value = args
                    .get(index + 1)
                    .ok_or_else(|| "missing value for --target".to_string())?
                    .clone();
                target = BackendTarget::from_str(&target_value)?;
                target_override = true;
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
            "--performance-report" => {
                performance_report = Some(
                    args.get(index + 1)
                        .ok_or_else(|| "missing value for --performance-report".to_string())?
                        .clone(),
                );
                index += 2;
            }
            flag => return Err(format!("unknown compile option `{flag}`").into()),
        }
    }

    if release && target != BackendTarget::Native {
        return Err("--release is only valid for the native target".into());
    }
    if performance_report.is_some() && target != BackendTarget::Native {
        return Err(
            "--performance-report is currently available only for the native target".into(),
        );
    }

    if !target.is_available() {
        return Err(format!(
            "target `{}` ({}) is planned but not implemented yet; available targets are `native`, `php`, and `debug`",
            target.name(),
            target.description()
        )
        .into());
    }

    if plan_path.is_some() && (release || target_override) {
        return Err(
            "--target and --release cannot override compiler settings from a build plan".into(),
        );
    }

    let source_load_started = Instant::now();
    if let Some(plan_path) = plan_path {
        if performance_report.is_some() {
            return Err("--performance-report is not available with --build-plan".into());
        }
        let (_, graph) = load_cli_graph(plan_path, diagnostic_options)?;
        let output = doriac::compile_compilation_graph(&graph).map_err(|diagnostics| {
            CliError::graph_diagnostics(graph.source_map.clone(), diagnostics, diagnostic_options)
        })?;
        let target = backend_target_for_plan(&graph.build_plan)?;
        let out_path = out.map_or_else(
            || default_build_plan_output_path(&graph.build_plan, target),
            PathBuf::from,
        );
        validate_compile_destinations(
            std::iter::once(Path::new(plan_path)).chain(graph.sources.values().map(|source| {
                source
                    .canonical_path
                    .as_deref()
                    .unwrap_or_else(|| Path::new(&source.display_path))
            })),
            &out_path,
            None,
        )?;
        write_backend_output(&out_path, output)?;
        println!("{}", out_path.display());
        return Ok(());
    }
    let input = input.expect("source input exists without a build plan");
    let (path, text) = read_source(input)?;
    let source_load = source_load_started.elapsed();
    let out_path = match out {
        Some(out) => PathBuf::from(out),
        None => default_output_path(input, target)?,
    };
    validate_compile_destinations(
        std::iter::once(Path::new(input)),
        &out_path,
        performance_report.as_deref().map(Path::new),
    )?;
    let options = CompileOptions {
        target,
        native_profile: if release {
            NativeProfile::Release
        } else {
            NativeProfile::Fast
        },
    };
    let (output, mut report) = if performance_report.is_some() {
        let command = utf8_cli_arguments(&invocation)?;
        let compilation = doriac::performance::compile_native(
            path.clone(),
            text.clone(),
            options,
            source_load,
            command,
        )
        .map_err(|diagnostics| {
            CliError::diagnostics(path.clone(), text.clone(), diagnostics, diagnostic_options)
        })?;
        (compilation.output, Some(compilation.report))
    } else {
        let output = doriac::compile_source_with_options(path.clone(), text.clone(), options)
            .map_err(|diagnostics| {
                CliError::diagnostics(path.clone(), text.clone(), diagnostics, diagnostic_options)
            })?;
        (output, None)
    };

    write_backend_output(&out_path, output)?;
    if let Some(report) = report.as_mut() {
        report["totalDurationNs"] = serde_json::Value::from(
            u64::try_from(source_load_started.elapsed().as_nanos()).unwrap_or(u64::MAX),
        );
    }
    if let (Some(report_path), Some(report)) = (performance_report, report) {
        write_performance_report(
            Path::new(&report_path),
            &report,
            &path,
            &text,
            diagnostic_options,
        )?;
    }
    println!("{}", out_path.display());
    Ok(())
}

fn write_performance_report(
    path: &Path,
    report: &serde_json::Value,
    source_path: &str,
    source_text: &str,
    diagnostic_options: RenderOptions,
) -> Result<(), CliError> {
    let encoded = serde_json::to_vec_pretty(report).map_err(|error| {
        performance_report_error(
            path,
            error.to_string(),
            source_path,
            source_text,
            diagnostic_options,
        )
    })?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| {
        performance_report_error(
            path,
            error.to_string(),
            source_path,
            source_text,
            diagnostic_options,
        )
    })?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("performance-report"),
        std::process::id()
    ));
    fs::write(&temporary, encoded)
        .and_then(|()| replace_file_atomically(&temporary, path))
        .map_err(|error| {
            let _ = fs::remove_file(&temporary);
            performance_report_error(
                path,
                error.to_string(),
                source_path,
                source_text,
                diagnostic_options,
            )
        })
}

fn performance_report_error(
    report_path: &Path,
    details: String,
    source_path: &str,
    source_text: &str,
    options: RenderOptions,
) -> CliError {
    let diagnostic = Diagnostic::new(
        "B2601",
        format!(
            "performance report write failed for `{}`: {details}",
            report_path.display()
        ),
        Span::default(),
    )
    .with_title("Performance Report Could Not Be Written")
    .with_primary_label("Performance Report Write Failed")
    .with_explanation(
        "Compilation completed, but the requested performance evidence could not be written atomically.",
    )
    .with_help("choose a writable report path that is separate from the source and compiler output")
    .with_developer_details(details);
    CliError::diagnostics(
        source_path.to_string(),
        source_text.to_string(),
        vec![diagnostic],
        options,
    )
}

fn build_plan_argument(args: &[String]) -> Result<Option<&str>, String> {
    if args.first().map(String::as_str) != Some("--build-plan") {
        return Ok(None);
    }
    args.get(1)
        .map(String::as_str)
        .map(Some)
        .ok_or_else(|| "missing value for --build-plan".to_string())
}

fn load_cli_graph(
    path: &str,
    options: RenderOptions,
) -> Result<
    (
        doriac::build_plan::BuildPlanDocument,
        doriac::compilation_graph::CompilationGraph,
    ),
    CliError,
> {
    let plan_text = fs::read_to_string(path)
        .map_err(|error| format!("failed to read build plan `{path}`: {error}"))?;
    let document = doriac::build_plan::parse_build_plan_document(path, plan_text.clone()).map_err(
        |diagnostics| {
            CliError::diagnostics(path.to_string(), plan_text.clone(), diagnostics, options)
        },
    )?;
    let graph = doriac::compilation_graph::load_compilation_graph_detailed(
        &document,
        &doriac::source_provider::FileSystemSourceProvider,
    )
    .map_err(|failure| graph_load_error(path, plan_text, failure, options))?;
    Ok((document, graph))
}

fn graph_load_error(
    path: &str,
    plan_text: String,
    mut failure: doriac::compilation_graph::GraphLoadFailure,
    options: RenderOptions,
) -> CliError {
    let plan_source_id = SourceId(u32::MAX);
    for diagnostic in &mut failure.diagnostics {
        let primary_is_plan = diagnostic.labels.iter().any(|label| {
            label.role == LabelRole::Primary && label.source == DiagnosticSource::Current
        });
        for label in &mut diagnostic.labels {
            if label.source == DiagnosticSource::Current {
                label.source = DiagnosticSource::Path(path.to_string());
                label.span.source = plan_source_id;
            }
        }
        for fix in &mut diagnostic.fixes {
            for edit in &mut fix.edits {
                if edit.source == DiagnosticSource::Current {
                    edit.source = DiagnosticSource::Path(path.to_string());
                    edit.span.source = plan_source_id;
                }
            }
        }
        if primary_is_plan {
            diagnostic.span.source = plan_source_id;
        }
    }
    failure.source_map.insert(doriac::source_map::SourceRecord {
        identity: doriac::names::SourceIdentity(format!("!build-plan:{path}")),
        package: doriac::names::PackageIdentity::SyntheticTooling("build-plan".to_string()),
        display_path: path.to_string(),
        canonical_path: fs::canonicalize(path)
            .ok()
            .map(|path| path.display().to_string()),
        content_fingerprint: String::new(),
        source: SourceFile::with_id(plan_source_id, path, plan_text),
    });
    CliError::graph_diagnostics(*failure.source_map, failure.diagnostics, options)
}

fn backend_target_for_plan(
    plan: &doriac::build_plan::BuildPlan,
) -> Result<BackendTarget, CliError> {
    match plan.compiler.target {
        doriac::build_plan::CompilerTarget::Debug => Ok(BackendTarget::Debug),
        doriac::build_plan::CompilerTarget::Native => Ok(BackendTarget::Native),
        doriac::build_plan::CompilerTarget::Php => Ok(BackendTarget::Php),
    }
}

fn default_build_plan_output_path(
    plan: &doriac::build_plan::BuildPlan,
    target: BackendTarget,
) -> PathBuf {
    let mut path = PathBuf::from(&plan.selected_target.name);
    let extension = default_output_extension(target);
    if !extension.is_empty() {
        path.set_extension(extension);
    }
    path
}

fn ast_command(args: &[String]) -> Result<(), CliError> {
    let (args, diagnostic_options) = parse_diagnostic_options(args)?;
    if let Some(plan_path) = build_plan_argument(&args)? {
        let (_, graph) = load_cli_graph(plan_path, diagnostic_options)?;
        for (identity, source) in &graph.sources {
            println!("Source {identity}\n{:#?}", source.authored);
        }
        return Ok(());
    }
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

fn metadata_command(args: &[String]) -> Result<(), CliError> {
    let (args, diagnostic_options) = parse_diagnostic_options(args)?;
    let (args, schema_version) = metadata_schema_version(args)?;
    let document = (if let Some(plan_path) = build_plan_argument(&args)? {
        if args.len() != 2 {
            return Err(format!("unknown metadata option `{}`", args[2]).into());
        }
        let (_, graph) = load_cli_graph(plan_path, diagnostic_options)?;
        match schema_version {
            1 => encode_metadata(&doriac::metadata_compilation_graph(&graph).map_err(
                |diagnostics| {
                    CliError::graph_diagnostics(
                        graph.source_map.clone(),
                        diagnostics,
                        diagnostic_options,
                    )
                },
            )?),
            2 => encode_metadata(&doriac::metadata_compilation_graph_v2(&graph).map_err(
                |diagnostics| {
                    CliError::graph_diagnostics(
                        graph.source_map.clone(),
                        diagnostics,
                        diagnostic_options,
                    )
                },
            )?),
            3 => encode_metadata(&doriac::metadata_compilation_graph_v3(&graph).map_err(
                |diagnostics| {
                    CliError::graph_diagnostics(
                        graph.source_map.clone(),
                        diagnostics,
                        diagnostic_options,
                    )
                },
            )?),
            _ => unreachable!(),
        }
    } else {
        let input = args
            .first()
            .ok_or_else(|| "missing input file".to_string())?;
        if let Some(option) = args.get(1) {
            return Err(format!("unknown metadata option `{option}`").into());
        }
        let (path, text) = read_source(input)?;
        match schema_version {
            1 => encode_metadata(
                &doriac::metadata_source(path.clone(), text.clone()).map_err(|diagnostics| {
                    CliError::diagnostics(
                        path.clone(),
                        text.clone(),
                        diagnostics,
                        diagnostic_options,
                    )
                })?,
            ),
            2 => encode_metadata(
                &doriac::metadata_source_v2(path.clone(), text.clone()).map_err(|diagnostics| {
                    CliError::diagnostics(
                        path.clone(),
                        text.clone(),
                        diagnostics,
                        diagnostic_options,
                    )
                })?,
            ),
            3 => encode_metadata(
                &doriac::metadata_source_v3(path.clone(), text.clone()).map_err(|diagnostics| {
                    CliError::diagnostics(
                        path.clone(),
                        text.clone(),
                        diagnostics,
                        diagnostic_options,
                    )
                })?,
            ),
            _ => unreachable!(),
        }
    })?;
    println!("{document}");
    Ok(())
}

fn encode_metadata(document: &impl serde::Serialize) -> Result<String, CliError> {
    serde_json::to_string_pretty(document)
        .map_err(|error| format!("failed to encode attribute metadata: {error}").into())
}

fn metadata_schema_version(args: Vec<String>) -> Result<(Vec<String>, u32), CliError> {
    let mut filtered = Vec::with_capacity(args.len());
    let mut schema_version = None;
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--schema-version" {
            if schema_version.is_some() {
                return Err("metadata schema version was supplied more than once".into());
            }
            let value = args
                .get(index + 1)
                .ok_or_else(|| "missing value for `--schema-version`".to_string())?;
            let parsed = value
                .parse::<u32>()
                .map_err(|_| format!("invalid metadata schema version `{value}`"))?;
            if !matches!(parsed, 1..=3) {
                return Err(format!("unsupported metadata schema version `{parsed}`").into());
            }
            schema_version = Some(parsed);
            index += 2;
            continue;
        }
        filtered.push(args[index].clone());
        index += 1;
    }
    Ok((filtered, schema_version.unwrap_or(1)))
}

fn hir_command(args: &[String]) -> Result<(), CliError> {
    let (args, diagnostic_options) = parse_diagnostic_options(args)?;
    if let Some(plan_path) = build_plan_argument(&args)? {
        let (_, graph) = load_cli_graph(plan_path, diagnostic_options)?;
        let hir = doriac::lower_compilation_graph(&graph).map_err(|diagnostics| {
            CliError::graph_diagnostics(graph.source_map.clone(), diagnostics, diagnostic_options)
        })?;
        println!("{hir:#?}");
        return Ok(());
    }
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
    if let Some(plan_path) = build_plan_argument(&args)? {
        let (_, graph) = load_cli_graph(plan_path, diagnostic_options)?;
        let mir = doriac::lower_compilation_graph_to_mir(&graph).map_err(|diagnostics| {
            CliError::graph_diagnostics(graph.source_map.clone(), diagnostics, diagnostic_options)
        })?;
        print!("{mir}");
        return Ok(());
    }
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

    Ok(PathBuf::from(file_name))
}

fn validate_compile_destinations<'a>(
    input_paths: impl IntoIterator<Item = &'a Path>,
    output_path: &Path,
    performance_report_path: Option<&Path>,
) -> Result<(), String> {
    let input_paths = input_paths.into_iter().collect::<Vec<_>>();
    for input_path in &input_paths {
        if paths_alias(input_path, output_path)? {
            return Err(format!(
                "output path `{}` would overwrite input `{}`; pass --out <file> to choose a different output path",
                output_path.display(),
                input_path.display()
            ));
        }
    }
    let Some(report_path) = performance_report_path else {
        return Ok(());
    };
    for input_path in input_paths {
        if paths_alias(report_path, input_path)? {
            return Err(format!(
                "performance report path `{}` would overwrite input `{}`; choose a separate report path",
                report_path.display(),
                input_path.display()
            ));
        }
    }
    if paths_alias(report_path, output_path)? {
        return Err(format!(
            "performance report path `{}` would overwrite compiler output `{}`; choose a separate report path",
            report_path.display(),
            output_path.display()
        ));
    }
    Ok(())
}

fn paths_alias(left: &Path, right: &Path) -> Result<bool, String> {
    let left_resolved = resolve_path_identity(left)?;
    let right_resolved = resolve_path_identity(right)?;
    if resolved_paths_equal(&left_resolved, &right_resolved) {
        return Ok(true);
    }
    Ok(existing_paths_alias(left, right))
}

fn resolve_path_identity(path: &Path) -> Result<PathBuf, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .map_err(|error| format!("failed to resolve current directory: {error}"))?
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    let mut existing = normalized.as_path();
    let mut missing = Vec::new();
    while !existing.exists() {
        let name = existing
            .file_name()
            .ok_or_else(|| format!("failed to resolve path identity for `{}`", path.display()))?;
        missing.push(name.to_os_string());
        existing = existing
            .parent()
            .ok_or_else(|| format!("failed to resolve path identity for `{}`", path.display()))?;
    }
    let mut resolved = fs::canonicalize(existing)
        .map_err(|error| format!("failed to resolve `{}`: {error}", path.display()))?;
    for component in missing.iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

#[cfg(not(windows))]
fn resolved_paths_equal(left: &Path, right: &Path) -> bool {
    left == right
}

#[cfg(windows)]
fn resolved_paths_equal(left: &Path, right: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;

    const CSTR_EQUAL: i32 = 2;
    #[link(name = "Kernel32")]
    extern "system" {
        fn CompareStringOrdinal(
            string1: *const u16,
            count1: i32,
            string2: *const u16,
            count2: i32,
            ignore_case: i32,
        ) -> i32;
    }

    let left = left.as_os_str().encode_wide().collect::<Vec<_>>();
    let right = right.as_os_str().encode_wide().collect::<Vec<_>>();
    let (Ok(left_len), Ok(right_len)) = (i32::try_from(left.len()), i32::try_from(right.len()))
    else {
        return false;
    };
    // SAFETY: Both UTF-16 buffers remain alive for the call and their explicit
    // lengths bound every read. A case-insensitive ordinal comparison matches
    // the default Windows path identity rules for destinations that do not yet
    // exist, while conservatively rejecting collisions in case-sensitive trees.
    unsafe {
        CompareStringOrdinal(left.as_ptr(), left_len, right.as_ptr(), right_len, 1) == CSTR_EQUAL
    }
}

#[cfg(unix)]
fn existing_paths_alias(left: &Path, right: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let (Ok(left), Ok(right)) = (fs::metadata(left), fs::metadata(right)) else {
        return false;
    };
    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(windows)]
fn existing_paths_alias(left: &Path, right: &Path) -> bool {
    windows_file_identity(left)
        .is_some_and(|identity| windows_file_identity(right) == Some(identity))
}

#[cfg(not(any(unix, windows)))]
fn existing_paths_alias(_left: &Path, _right: &Path) -> bool {
    false
}

#[cfg(windows)]
fn windows_file_identity(path: &Path) -> Option<(u32, u64)> {
    use std::ffi::c_void;
    use std::mem::MaybeUninit;
    use std::os::windows::io::AsRawHandle;

    #[repr(C)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    #[repr(C)]
    struct ByHandleFileInformation {
        attributes: u32,
        creation_time: FileTime,
        last_access_time: FileTime,
        last_write_time: FileTime,
        volume_serial_number: u32,
        file_size_high: u32,
        file_size_low: u32,
        number_of_links: u32,
        file_index_high: u32,
        file_index_low: u32,
    }

    #[link(name = "Kernel32")]
    extern "system" {
        fn GetFileInformationByHandle(
            file: *mut c_void,
            information: *mut ByHandleFileInformation,
        ) -> i32;
    }

    let file = fs::File::open(path).ok()?;
    let mut information = MaybeUninit::<ByHandleFileInformation>::uninit();
    // SAFETY: `file` owns a valid handle for the duration of the call, and the
    // Windows API initializes the complete output structure when it succeeds.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) } == 0 {
        return None;
    }
    // SAFETY: A successful call initialized the full structure.
    let information = unsafe { information.assume_init() };
    let file_index =
        (u64::from(information.file_index_high) << 32) | u64::from(information.file_index_low);
    Some((information.volume_serial_number, file_index))
}

#[cfg(not(windows))]
fn replace_file_atomically(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn replace_file_atomically(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    #[link(name = "Kernel32")]
    extern "system" {
        fn MoveFileExW(
            existing_file_name: *const u16,
            new_file_name: *const u16,
            flags: u32,
        ) -> i32;
    }

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: Both path buffers are NUL-terminated and remain alive for the
    // duration of the call. The flags request same-volume atomic replacement
    // and synchronous persistence of the rename operation.
    let replaced = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if replaced == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
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
    let plan_path = build_plan_argument(&compiler_args)?;
    if let Some(plan_path) = plan_path {
        if compiler_args.len() != 2 {
            return Err("build-plan run accepts no compiler overrides".into());
        }
        let (_, graph) = load_cli_graph(plan_path, diagnostic_options)?;
        if graph.build_plan.selected_target.kind != doriac::build_plan::TargetKind::Binary {
            return Err("running a build plan requires a selected binary target".into());
        }
        if graph.build_plan.compiler.target != doriac::build_plan::CompilerTarget::Native {
            return Err("running a build plan requires compiler.target `native`".into());
        }
        let output = doriac::compile_compilation_graph(&graph).map_err(|diagnostics| {
            CliError::graph_diagnostics(graph.source_map.clone(), diagnostics, diagnostic_options)
        })?;
        let (status, runtime_payload) = execute_native_output(plan_path, output, program_args)?;
        if let Some(payload) = runtime_payload {
            let fallback_path = graph
                .selected_entry
                .as_ref()
                .and_then(|entry| graph.sources.get(&entry.0))
                .map(|source| source.display_path.as_str())
                .unwrap_or(plan_path);
            let diagnostic = decode_runtime_outcome(&payload, fallback_path)?;
            let options = RenderOptions {
                context_lines: 0,
                ..diagnostic_options
            };
            let rendered = doriac::diagnostics::render_diagnostics_with_source_map(
                &graph.source_map,
                &[diagnostic],
                options,
            );
            if options.format == DiagnosticFormat::Json {
                println!("{rendered}");
            } else {
                eprintln!("{rendered}");
            }
        }
        return Ok(exit_code_from_status(status)?);
    }
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

    let (status, runtime_payload) = execute_native_output(input, output, program_args)?;
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

fn execute_native_output(
    input: &str,
    output: BackendOutput,
    program_args: &[OsString],
) -> Result<(ExitStatus, Option<Vec<u8>>), CliError> {
    let temp_path = temp_run_executable_path(input);
    let outcome_path = temp_path.with_extension("doria-outcome");
    write_backend_output(&temp_path, output)
        .map_err(|error| format!("failed to write temp native executable: {error}"))?;
    let status = Command::new(&temp_path)
        .env("DORIA_RUNTIME_OUTCOME_V2", &outcome_path)
        .env("DORIA_RUNTIME_OUTCOME_V3", &outcome_path)
        .args(program_args)
        .status()
        .map_err(|error| {
            format!(
                "failed to run native executable `{}`: {error}",
                temp_path.display()
            )
        });
    let runtime_payload = fs::read(&outcome_path).ok();
    let _ = fs::remove_file(&outcome_path);
    let _ = fs::remove_file(&temp_path);
    Ok((status?, runtime_payload))
}

fn decode_runtime_outcome(payload: &[u8], source_path: &str) -> Result<Diagnostic, CliError> {
    if payload.starts_with(b"DORIAO3\0") {
        return decode_runtime_error_outcome(payload, source_path);
    }
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
        error_type: None,
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

fn decode_runtime_error_outcome(payload: &[u8], source_path: &str) -> Result<Diagnostic, CliError> {
    let mut decoder = RuntimeOutcomeDecoder::new(payload);
    if decoder.take(8)? != b"DORIAO3\0" || decoder.u16()? != 3 {
        return Err("native program returned an unsupported runtime Error record".into());
    }
    let error_type_length = decoder.u32()? as usize;
    let message_length = usize::try_from(decoder.u64()?)
        .map_err(|_| "native program returned an impossible runtime Error message length")?;
    let path_length = decoder.u32()? as usize;
    let source_length = decoder.u32()? as usize;
    let function_length = decoder.u32()? as usize;
    let origin_known = match decoder.byte()? {
        0 => false,
        1 => true,
        _ => return Err("native program returned an invalid runtime Error origin state".into()),
    };
    let span = Span::new(decoder.u64()? as usize, decoder.u64()? as usize);
    if error_type_length > 4096
        || path_length > 4096
        || source_length > 4 * 1024 * 1024
        || function_length > 1024
        || message_length > decoder.remaining_len()
    {
        return Err("native program returned an oversized runtime Error record".into());
    }
    let error_type = decoder.text(error_type_length)?;
    let message = decoder.text(message_length)?;
    let (record_path, function) = if origin_known {
        let path = decoder.text(path_length)?;
        let _embedded_source = decoder.text(source_length)?;
        let function = decoder.text(function_length)?;
        (path, Some(function))
    } else {
        if path_length != 0 || source_length != 0 || function_length != 0 || span != Span::default()
        {
            return Err("native program returned facts for an unavailable Error origin".into());
        }
        (String::new(), None)
    };
    if !decoder.is_empty() {
        return Err("native program returned trailing runtime Error bytes".into());
    }
    let source = if origin_known {
        diagnostic_source(&record_path, source_path)
    } else {
        DiagnosticSource::Unavailable
    };
    let outcome = RuntimeOutcomeDetails {
        process_status: 70,
        termination_behavior: TerminationBehavior::PropagateWithCleanup,
        origin: RuntimeOutcomeOrigin {
            source,
            span,
            function,
        },
        path: Vec::new(),
        facts: Vec::new(),
        error_type: Some(error_type.clone()),
    };
    Ok(Diagnostic::runtime_error(
        error_type, message, span, outcome,
    ))
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

    fn remaining_len(&self) -> usize {
        self.remaining.len()
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
        "doriac {}\n\nUSAGE:\n    doriac check <source.doria> [diagnostic options]\n    doriac check --build-plan <plan.json> [diagnostic options]\n    doriac ast|hir|mir <source.doria> [diagnostic options]\n    doriac ast|hir|mir --build-plan <plan.json> [diagnostic options]\n    doriac metadata <source.doria> [--schema-version 1|2|3] [diagnostic options]\n    doriac metadata --build-plan <plan.json> [--schema-version 1|2|3] [diagnostic options]\n    doriac compile <source.doria> [--release] [--out <file>] [--performance-report <file>] [diagnostic options]\n    doriac compile <source.doria> --target php [--out <file>] [diagnostic options]\n    doriac compile --build-plan <plan.json> [--out <file>] [diagnostic options]\n    doriac run <source.doria> [--release] [diagnostic options] [-- <program args>...]\n    doriac run --build-plan <plan.json> [diagnostic options] [-- <program args>...]\n\nDIAGNOSTIC OPTIONS:\n    --diagnostic-format human|concise|json    default: human\n    --diagnostic-color auto|always|never      default: auto; NO_COLOR disables auto color\n\nHuman and concise diagnostics are written to stderr. Versioned JSON diagnostics are written to stdout.\n\nNATIVE PROFILES:\n    fast       default Cranelift profile for rapid local feedback\n    release    LLVM optimized profile selected with --release\n\nTARGETS:\n    native    default target for standalone executables\n    php       compatibility and inspection backend\n    debug     MIR interpreter debug artifact\n    wasm      planned WebAssembly backend",
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
            Err(CliError::GraphDiagnostics { .. }) => {
                panic!("transport decoding must not create a graph diagnostic")
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
