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

#[cfg(test)]
#[path = "../tests/common/execution_directory.rs"]
mod execution_directory;

#[cfg(test)]
#[path = "../tests/common/native_execution_runner.rs"]
mod panic_native_execution;

#[cfg(test)]
#[path = "../tests/common/php_execution.rs"]
mod php_execution;

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
    #[cfg(feature = "bundled-runtime")]
    doriac::runtime_artifact::register_embedded_runtimes(
        include_bytes!(env!("DORIA_RT_BUILT_DEBUG_PATH")),
        include_str!(concat!(
            env!("DORIA_RT_BUILT_DEBUG_PATH"),
            ".doria-runtime.json"
        )),
        include_bytes!(env!("DORIA_RT_BUILT_RELEASE_PATH")),
        include_str!(concat!(
            env!("DORIA_RT_BUILT_RELEASE_PATH"),
            ".doria-runtime.json"
        )),
    );

    #[cfg(feature = "bundled-runtime")]
    doriac::runtime_artifact::register_embedded_shared_runtime(
        include_bytes!(env!("DORIA_RT_BUILT_SHARED_PATH")),
        include_str!(concat!(
            env!("DORIA_RT_BUILT_SHARED_PATH"),
            ".doria-runtime.json"
        )),
    );

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
        let (document, graph) = load_cli_graph(plan_path, diagnostic_options)?;
        let graph_load = source_load_started.elapsed();
        let target = backend_target_for_plan(&graph.build_plan)?;
        if performance_report.is_some() && target != BackendTarget::Native {
            return Err("--performance-report requires a native build-plan target".into());
        }
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
            performance_report.as_deref().map(Path::new),
        )?;
        let (output, report) = if performance_report.is_some() {
            let compilation = doriac::performance::compile_native_graph(
                &graph,
                plan_path,
                graph_load,
                utf8_cli_arguments(&invocation)?,
            )
            .map_err(|diagnostics| {
                CliError::graph_diagnostics(
                    graph.source_map.clone(),
                    diagnostics,
                    diagnostic_options,
                )
            })?;
            (compilation.output, Some(compilation.report))
        } else {
            let output = doriac::compile_compilation_graph(&graph).map_err(|diagnostics| {
                CliError::graph_diagnostics(
                    graph.source_map.clone(),
                    diagnostics,
                    diagnostic_options,
                )
            })?;
            (output, None)
        };
        write_backend_output(&out_path, output).map_err(|error| {
            CliError::graph_diagnostics(
                graph.source_map.clone(),
                publication_diagnostics(error),
                diagnostic_options,
            )
        })?;
        if let (Some(report_path), Some(mut report)) = (performance_report, report) {
            report["totalDurationNs"] = serde_json::Value::from(
                u64::try_from(source_load_started.elapsed().as_nanos()).unwrap_or(u64::MAX),
            );
            write_performance_report(
                Path::new(&report_path),
                &report,
                &document.path,
                &document.text,
                diagnostic_options,
            )?;
        }
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

    write_backend_output(&out_path, output).map_err(|error| {
        CliError::diagnostics(
            path.clone(),
            text.clone(),
            publication_diagnostics(error),
            diagnostic_options,
        )
    })?;
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
    // Plan diagnostics locate the plan text, which is not a graph source.
    // Give their spans the plan's identity; preparation names the source.
    let plan_source_id = SourceId(u32::MAX);
    for diagnostic in &mut failure.diagnostics {
        let primary_is_plan = diagnostic.labels.iter().any(|label| {
            label.role == LabelRole::Primary && label.source == DiagnosticSource::Current
        });
        for label in &mut diagnostic.labels {
            if label.source == DiagnosticSource::Current {
                label.span.source = plan_source_id;
            }
        }
        for fix in &mut diagnostic.fixes {
            for edit in &mut fix.edits {
                if edit.source == DiagnosticSource::Current {
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

fn write_backend_output(
    out_path: &Path,
    output: BackendOutput,
) -> Result<(), doriac::backend::BackendError> {
    doriac::backend::publish_output(out_path, output)
}

fn publication_diagnostics(error: doriac::backend::BackendError) -> Vec<Diagnostic> {
    error
        .diagnostics
        .unwrap_or_else(|| vec![Diagnostic::new("B0001", error.message, Span::default())])
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
            let diagnostic = decode_runtime_outcome_with_sources(
                &payload,
                fallback_path,
                Some(&graph.source_map),
            )?;
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
        .map_err(|error| format!("failed to write temp native executable: {}", error.message))?;
    let status = doriac::native_process::spawn(
        Command::new(&temp_path)
            .env("DORIA_RUNTIME_OUTCOME_V2", &outcome_path)
            .env("DORIA_RUNTIME_OUTCOME_V3", &outcome_path)
            .env("DORIA_RUNTIME_OUTCOME_V4", &outcome_path)
            .env("DORIA_RUNTIME_OUTCOME_V5", &outcome_path)
            .args(program_args),
    )
    .and_then(|mut child| child.wait())
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
    decode_runtime_outcome_with_sources(payload, source_path, None)
}

fn decode_runtime_outcome_with_sources(
    payload: &[u8],
    source_path: &str,
    sources: Option<&doriac::source_map::SourceMap>,
) -> Result<Diagnostic, CliError> {
    if payload.starts_with(b"DORIAO4\0") {
        return decode_runtime_assertion_outcome(payload, source_path, sources);
    }
    if payload.starts_with(b"DORIAO3\0") || payload.starts_with(b"DORIAO5\0") {
        return decode_runtime_error_outcome(payload, source_path, sources);
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
    let primary_span = runtime_source_span(sources, &record_path, primary_span)?;
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
        let span = runtime_source_span(sources, &frame_path, span)?;
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
    // A user panic carries its message; a failed runtime check carries the
    // check that failed.
    if (code == "P1000" || code == "P1001") && !message.is_empty() {
        diagnostic.notes.push(message);
    }
    Ok(diagnostic)
}

fn decode_runtime_assertion_outcome(
    payload: &[u8],
    source_path: &str,
    sources: Option<&doriac::source_map::SourceMap>,
) -> Result<Diagnostic, CliError> {
    const ASSERTION_ERROR: &str = "Doria\\Std\\Test\\AssertionError";
    let mut decoder = RuntimeOutcomeDecoder::new(payload);
    if decoder.take(8)? != b"DORIAO4\0" || decoder.u16()? != 4 {
        return Err("native program returned an unsupported runtime assertion record".into());
    }
    let error_type_length = decoder.u32()? as usize;
    if decoder.u32()? != 70 || decoder.byte()? != 1 {
        return Err("native program returned an invalid assertion termination contract".into());
    }
    let matcher_length = decoder.u32()? as usize;
    let flag = |decoder: &mut RuntimeOutcomeDecoder<'_>| -> Result<bool, CliError> {
        match decoder.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err("native program returned an invalid assertion presence flag".into()),
        }
    };
    let negated = flag(&mut decoder)?;
    let actual_present = flag(&mut decoder)?;
    let actual_type_length = decoder.u32()? as usize;
    let actual_presentation_length = decoder.u32()? as usize;
    let expected_present = flag(&mut decoder)?;
    let expected_type_length = decoder.u32()? as usize;
    let expected_presentation_length = decoder.u32()? as usize;
    let difference_present = flag(&mut decoder)?;
    let difference_length = decoder.u32()? as usize;
    let user_message_present = flag(&mut decoder)?;
    let user_message_length = decoder.u32()? as usize;
    let path_length = decoder.u32()? as usize;
    let source_length = decoder.u32()? as usize;
    let function_length = decoder.u32()? as usize;
    let origin_known = flag(&mut decoder)?;
    let span = Span::new(decoder.u64()? as usize, decoder.u64()? as usize);
    let frame_count = usize::from(decoder.u16()?);
    if error_type_length > 4096
        || matcher_length > 64
        || actual_type_length > 4096
        || actual_presentation_length > doriac::assertions::PRESENTATION_LIMIT
        || expected_type_length > 4096
        || expected_presentation_length > doriac::assertions::PRESENTATION_LIMIT
        || difference_length > doriac::assertions::PRESENTATION_LIMIT
        || user_message_length > 64 * 1024
        || path_length > 4096
        || source_length > 4 * 1024 * 1024
        || function_length > 1024
        || frame_count > 128
    {
        return Err("native program returned an oversized runtime assertion record".into());
    }
    if (!actual_present && (actual_type_length != 0 || actual_presentation_length != 0))
        || (!expected_present && (expected_type_length != 0 || expected_presentation_length != 0))
        || (!difference_present && difference_length != 0)
        || (!user_message_present && user_message_length != 0)
    {
        return Err("native program returned values for absent assertion facts".into());
    }

    let error_type = decoder.text(error_type_length)?;
    let matcher_name = decoder.text(matcher_length)?;
    let actual_type = decoder.text(actual_type_length)?;
    let actual_presentation = decoder.text(actual_presentation_length)?;
    let expected_type = decoder.text(expected_type_length)?;
    let expected_presentation = decoder.text(expected_presentation_length)?;
    let difference = decoder.text(difference_length)?;
    let user_message = decoder.text(user_message_length)?;
    if error_type != ASSERTION_ERROR {
        return Err("native program returned an invalid Slice-2 assertion identity".into());
    }
    let matcher = doriac::assertions::matcher_from_fact_name(&matcher_name)
        .ok_or_else(|| "native program returned an unknown assertion matcher".to_string())?;
    use doriac::assertions::AssertionMatcher as Matcher;
    match matcher {
        Matcher::Fail if negated || actual_present || expected_present || !user_message_present => {
            return Err("native program returned malformed explicit-failure facts".into());
        }
        Matcher::Null
            if !expected_present || expected_type != "null" || expected_presentation != "null" =>
        {
            return Err("native program returned malformed null-expectation facts".into());
        }
        Matcher::True
            if !expected_present || expected_type != "bool" || expected_presentation != "true" =>
        {
            return Err("native program returned malformed true-expectation facts".into());
        }
        Matcher::False
            if !expected_present || expected_type != "bool" || expected_presentation != "false" =>
        {
            return Err("native program returned malformed false-expectation facts".into());
        }
        Matcher::StringEmpty
            if !expected_present
                || expected_type != "string"
                || expected_presentation != "\"\"" =>
        {
            return Err("native program returned malformed empty-string expectation facts".into());
        }
        Matcher::Fail => {}
        _ if !actual_present => {
            return Err("native program returned an expectation without an actual value".into());
        }
        _ => {}
    }

    let (record_path, function) = if origin_known {
        let path = decoder.text(path_length)?;
        let _embedded_source = decoder.text(source_length)?;
        let function = decoder.text(function_length)?;
        (path, Some(function))
    } else {
        if path_length != 0
            || source_length != 0
            || function_length != 0
            || span != Span::default()
            || frame_count != 0
        {
            return Err("native program returned facts for an unavailable assertion origin".into());
        }
        (String::new(), None)
    };
    let span = if origin_known {
        runtime_source_span(sources, &record_path, span)?
    } else {
        span
    };
    let mut frames = Vec::with_capacity(frame_count);
    for _ in 0..frame_count {
        let frame_function_length = usize::from(decoder.u16()?);
        let frame_path_length = decoder.u32()? as usize;
        let frame_span = Span::new(decoder.u64()? as usize, decoder.u64()? as usize);
        if frame_function_length > 1024 || frame_path_length > 4096 {
            return Err("native program returned an oversized assertion frame".into());
        }
        let frame_function = decoder.text(frame_function_length)?;
        let frame_path = decoder.text(frame_path_length)?;
        frames.push(RuntimeOutcomeFrame {
            function: frame_function,
            source: diagnostic_source(&frame_path, source_path),
            span: runtime_source_span(sources, &frame_path, frame_span)?,
        });
    }
    if !decoder.is_empty() {
        return Err("native program returned trailing runtime assertion bytes".into());
    }

    let facts = vec![
        RuntimeFact {
            name: doria_diagnostic_catalogue::ASSERTION_MATCHER_FACT.to_string(),
            value: RuntimeFactValue::StaticString(matcher_name),
        },
        RuntimeFact {
            name: doria_diagnostic_catalogue::ASSERTION_NEGATED_FACT.to_string(),
            value: RuntimeFactValue::Boolean(negated),
        },
        RuntimeFact {
            name: doria_diagnostic_catalogue::ASSERTION_ACTUAL_PRESENT_FACT.to_string(),
            value: RuntimeFactValue::Boolean(actual_present),
        },
        RuntimeFact {
            name: doria_diagnostic_catalogue::ASSERTION_ACTUAL_TYPE_FACT.to_string(),
            value: RuntimeFactValue::StaticString(actual_type),
        },
        RuntimeFact {
            name: doria_diagnostic_catalogue::ASSERTION_ACTUAL_PRESENTATION_FACT.to_string(),
            value: RuntimeFactValue::StaticString(actual_presentation),
        },
        RuntimeFact {
            name: doria_diagnostic_catalogue::ASSERTION_EXPECTED_PRESENT_FACT.to_string(),
            value: RuntimeFactValue::Boolean(expected_present),
        },
        RuntimeFact {
            name: doria_diagnostic_catalogue::ASSERTION_EXPECTED_TYPE_FACT.to_string(),
            value: RuntimeFactValue::StaticString(expected_type),
        },
        RuntimeFact {
            name: doria_diagnostic_catalogue::ASSERTION_EXPECTED_PRESENTATION_FACT.to_string(),
            value: RuntimeFactValue::StaticString(expected_presentation),
        },
        RuntimeFact {
            name: doria_diagnostic_catalogue::ASSERTION_DIFFERENCE_PRESENT_FACT.to_string(),
            value: RuntimeFactValue::Boolean(difference_present),
        },
        RuntimeFact {
            name: doria_diagnostic_catalogue::ASSERTION_DIFFERENCE_FACT.to_string(),
            value: RuntimeFactValue::StaticString(difference),
        },
        RuntimeFact {
            name: doria_diagnostic_catalogue::ASSERTION_USER_MESSAGE_PRESENT_FACT.to_string(),
            value: RuntimeFactValue::Boolean(user_message_present),
        },
        RuntimeFact {
            name: doria_diagnostic_catalogue::ASSERTION_USER_MESSAGE_FACT.to_string(),
            value: RuntimeFactValue::StaticString(user_message.clone()),
        },
    ];
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
        path: frames,
        facts,
        error_type: Some(error_type),
    };
    let message = if user_message_present {
        user_message
    } else {
        doriac::assertions::stable_message(matcher, negated).to_string()
    };
    Ok(Diagnostic::runtime_assertion(message, span, outcome))
}

fn decode_runtime_error_outcome(
    payload: &[u8],
    source_path: &str,
    sources: Option<&doriac::source_map::SourceMap>,
) -> Result<Diagnostic, CliError> {
    let mut decoder = RuntimeOutcomeDecoder::new(payload);
    let magic = decoder.take(8)?;
    let version = decoder.u16()?;
    if !((magic == b"DORIAO3\0" && version == 3) || (magic == b"DORIAO5\0" && version == 5)) {
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
    let transferred_byte_count = if version == 5 {
        let present = decoder.byte()?;
        let count = decoder.u64()? as i64;
        match present {
            0 if count == 0 => None,
            1 => Some(count),
            _ => {
                return Err("native program returned invalid I/O progress metadata".into());
            }
        }
    } else {
        None
    };
    if error_type_length > 4096
        || path_length > 4096
        || source_length > 4 * 1024 * 1024
        || function_length > 1024
        || message_length > decoder.remaining_len()
    {
        return Err("native program returned an oversized runtime Error record".into());
    }
    let error_type = decoder.text(error_type_length)?;
    if transferred_byte_count.is_some() && error_type != doriac::compiler_known_io::IO_ERROR {
        return Err("native program returned I/O progress for a non-IoError type".into());
    }
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
    let span = if origin_known {
        runtime_source_span(sources, &record_path, span)?
    } else {
        span
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
        facts: transferred_byte_count
            .map(|value| RuntimeFact {
                name: doria_diagnostic_catalogue::IO_TRANSFERRED_BYTE_COUNT_FACT.to_string(),
                value: RuntimeFactValue::Signed(value),
            })
            .into_iter()
            .collect(),
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

fn runtime_source_span(
    sources: Option<&doriac::source_map::SourceMap>,
    path: &str,
    offsets: Span,
) -> Result<Span, CliError> {
    let Some(sources) = sources else {
        // Standalone compilation owns source zero. Runtime records carry paths
        // and offsets, not process-local SourceIds from a compilation graph.
        return Ok(offsets);
    };
    let record = sources.by_path(path).ok_or_else(|| {
        CliError::from(
            "native program returned a runtime source absent from the compilation inventory",
        )
    })?;
    if offsets.start > offsets.end || offsets.end > record.source.text.len() {
        return Err("native program returned a runtime span outside its source".into());
    }
    Ok(Span::in_source(
        record.source.id,
        offsets.start,
        offsets.end,
    ))
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
        "doriac {}\n\nUSAGE:\n    doriac check <source.doria> [diagnostic options]\n    doriac check --build-plan <plan.json> [diagnostic options]\n    doriac ast|hir|mir <source.doria> [diagnostic options]\n    doriac ast|hir|mir --build-plan <plan.json> [diagnostic options]\n    doriac metadata <source.doria> [--schema-version 1|2|3] [diagnostic options]\n    doriac metadata --build-plan <plan.json> [--schema-version 1|2|3] [diagnostic options]\n    doriac compile <source.doria> [--release] [--out <file>] [--performance-report <file>] [diagnostic options]\n    doriac compile <source.doria> --target php [--out <file>] [diagnostic options]\n    doriac compile --build-plan <plan.json> [--out <file>] [--performance-report <file>] [diagnostic options]\n    doriac run <source.doria> [--release] [diagnostic options] [-- <program args>...]\n    doriac run --build-plan <plan.json> [diagnostic options] [-- <program args>...]\n\nDIAGNOSTIC OPTIONS:\n    --diagnostic-format human|concise|json    default: human\n    --diagnostic-color auto|always|never      default: auto; NO_COLOR disables auto color\n\nHuman and concise diagnostics are written to stderr. Versioned JSON diagnostics are written to stdout.\n\nNATIVE PROFILES:\n    fast       default Cranelift profile for rapid local feedback\n    release    LLVM optimized profile selected with --release\n\nTARGETS:\n    native    default target for standalone executables\n    php       compatibility and inspection backend\n    debug     MIR interpreter debug artifact\n    wasm      planned WebAssembly backend",
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

#[cfg(test)]
mod tests {
    use super::panic_native_execution::ExecutionDirectory;
    use super::*;

    fn with_panic_fixture_context<T>(
        path: &str,
        source: &str,
        run: impl FnOnce() -> T + std::panic::UnwindSafe,
    ) -> T {
        match std::panic::catch_unwind(run) {
            Ok(value) => value,
            Err(payload) => {
                // Add repro context to compiler panics and failed assertions,
                // then resume the original failure without changing its result.
                eprintln!("Panic parity fixture: {path}\n{source}");
                std::panic::resume_unwind(payload)
            }
        }
    }

    // Keep constructor and cleanup parity strict across the real V2 decoder,
    // human PHP output, the interpreter, and each enabled native profile.
    const PHP_LIFECYCLE_PANIC_SOURCES: &[&str] = &[
        // Ordinary constructors retain the authored `new`, not a declaration.
        r#"class Value { function __construct(int $value) { panic("constructor"); } }
function argument(): int { return 7; }
function main(): void { let $value = new Value(argument()); }"#,
        r#"function seed(): int { panic("generated constructor initializer"); }
class Value { int $number = seed(); }
function main(): void { let $value = new Value(); }"#,
        r#"open class Base { function __construct() { panic("implicit parent constructor"); } }
class Value extends Base {}
function main(): void { let $value = new Value(); }"#,
        // An intrinsic panic in an initializer has no intervening user call.
        r#"class Value { Bytes $bytes = Bytes::allocate(-1); }
function main(): void { let $value = new Value(); }"#,
        r#"class Marker { function __destruct() { panic("normal cleanup"); } }
function finish(): void { let $marker = new Marker(); }
function main(): void { finish(); }"#,
        r#"class Failure implements Error { function __construct(string $message) {} }
class Marker { function __destruct() { panic("checked cleanup"); } }
function attempt(): void throws Failure {
    let $marker = new Marker();
    throw new Failure("retry");
}
function main(): void throws Failure { attempt(); }"#,
        // Incomplete construction retains its frame through field cleanup.
        r#"class Failure implements Error { function __construct(string $message) {} }
class Marker { function __destruct() { panic("initializer failure cleanup"); } }
function seed(): int throws Failure { throw new Failure("initialize"); }
class Value {
    Marker $marker = new Marker();
    int $number = seed();
    function __construct() throws Failure {}
}
function main(): void throws Failure { let $value = new Value(); }"#,
        // A parent without executable construction still owns completed-phase
        // cleanup when either kind of child constructor subsequently fails.
        r#"class Failure implements Error { function __construct(string $message) {} }
open class Base {
    function __destruct() { try { echo "base;"; } catch (Error) {} }
}
class Value extends Base {
    function __construct() throws Failure { throw new Failure("child"); }
}
function main(): void {
    try { let $value = new Value(); } catch (Failure) {}
    panic("after explicit child construction failure");
}"#,
        // Generated constructors may transport automatic I/O effects, but
        // E0635 still requires explicit contracts for required user errors.
        // A literal NUL rejects the path before any filesystem access.
        concat!(
            r#"open class Base {
    function __destruct() { try { echo "base;"; } catch (Error) {} }
}
function seed(): string { return read_file("invalid"#,
            "\0",
            r#"path"); }
class Value extends Base { string $contents = seed(); }
function main(): void {
    try { let $value = new Value(); } catch (Doria\Std\Io\IoError) {}
    panic("after generated child construction failure");
}"#
        ),
        // Caught construction failure and later normal property destruction
        // must remove every lifecycle frame before the subsequent panic.
        r#"class Failure implements Error { function __construct(string $message) {} }
class Marker {
    function __destruct() { try { echo "field;"; } catch (Error) {} }
}
class Value {
    Marker $marker = new Marker();
    function __construct(bool $fail) throws Failure {
        if ($fail) { throw new Failure("initialize"); }
    }
    function __destruct() { try { echo "value;"; } catch (Error) {} }
}
function main(): void throws Failure {
    try { let $failed = new Value(true); } catch (Failure) {}
    { let $completed = new Value(false); }
    panic("after lifecycle cleanup");
}"#,
    ];

    #[test]
    fn php_panic_transport_preserves_typed_facts_and_exact_human_projection() {
        let path = "php-panic-transport.doria";
        // Reuse the interface payload exercised by
        // collection_ingestion_through_shared_access_transfers_owned_values.
        let shared_payload = r#"
class Token {
    function __construct(int $value) {}
    function __destruct() { try { echo "token{$this->value};"; } catch (Error) {} }
}
interface TokenQueue {
    writable function pushBack(take List<Token> $value): void;
    writable function popBack(): ?List<Token>;
}
class Adapter implements TokenQueue {
    writable Deque<List<Token>> $values = Deque::from([]);
    writable function pushBack(take List<Token> $value): void { $this->values->pushBack($value); }
    writable function popBack(): ?List<Token> { return $this->values->popBack(); }
}
"#;
        let shared_conflicts = [
            (
                r#"function main(): void {
    let $shared = new WritableSharedReference<TokenQueue>(new Adapter());
    let $readonly = $shared->acquireReadonlyAccess();
    let writable $writable = $shared->acquireWritableAccess();
}"#,
                doria_diagnostic_catalogue::READONLY_THEN_WRITABLE_CONFLICT,
            ),
            (
                r#"function main(): void {
    let $shared = new WritableSharedReference<TokenQueue>(new Adapter());
    let writable $writable = $shared->acquireWritableAccess();
    let $readonly = $shared->acquireReadonlyAccess();
}"#,
                doria_diagnostic_catalogue::WRITABLE_THEN_READONLY_CONFLICT,
            ),
            (
                r#"function main(): void {
    let $shared = new WritableSharedReference<TokenQueue>(new Adapter());
    let writable $first = $shared->acquireWritableAccess();
    let writable $second = $shared->acquireWritableAccess();
}"#,
                doria_diagnostic_catalogue::WRITABLE_THEN_WRITABLE_CONFLICT,
            ),
        ];
        for (source, conflict_reason) in [
            "function main(): void { panic(\"stop 界\"); }",
            "function main(): void { int[] $values = [7]; let $value = $values[-1]; }",
            "function main(): void { List<int> $values = [7]; let $value = $values[1]; }",
            "function main(): void { Dictionary<string, int> $values = [\"present\" => 7]; let $value = $values[\"missing\"]; }",
            "function main(): void { let $values = Bytes::allocate(-7); }",
            "function main(): void { let $values = Bytes::allocate(1); let $value = $values[-1]; }",
            "function main(): void { let $values = Bytes::allocate(1); let $view = $values->getRegion(-1, 1); }",
            include_str!("../../../examples/native/main_stage23c_negative_fill_panic.doria"),
            // Recursion and repeated same-line calls retain each full caller span.
            r#"function descend(int $depth): void {
    if ($depth > 0) { descend($depth - 1); return; }
    panic("recursive stop");
}
function main(): void { descend(2); }"#,
            r#"function choose(bool $stop): void { if ($stop) { panic("same line"); } }
function main(): void { choose(false); choose(true); }"#,
            // Written named-argument order and exactly-once evaluation survive
            // metadata scopes; the outer call must replace nested call sites.
            r#"function argument(int $value): int {
    try { echo "argument{$value};"; } catch (Error) {}
    return $value;
}
function fail(int $first, int $second): void { panic("arguments"); }
function main(): void { fail(second: argument(2), first: argument(1)); }"#,
            r#"function fail<T>(T $value): void { panic("specialized"); }
class Runner { function __doria_list_insert_at(): void { fail(1); } }
function main(): void { let $runner = new Runner(); $runner->__doria_list_insert_at(); }"#,
            r#"interface Runner { function run(int $value): void; }
class Task implements Runner { function run(int $value): void { panic("interface"); } }
function argument(): int { return 7; }
function forward(Runner $runner): void { $runner->run(argument()); }
function main(): void { let $runner = new Task(); forward($runner); }"#,
            r#"open class Base { open function run(): void { panic("parent"); } }
class Task extends Base { override function run(): void { parent::run(); } }
function main(): void { let $runner = new Task(); $runner->run(); }"#,
            r#"class Runner { static function run(int $value): void { panic("static"); } }
function argument(): int { return 7; }
function main(): void { Runner::run(argument()); }"#,
            r#"class Failure implements Error { function __construct(string $message) {} }
class Task { function __construct(int $value) throws Failure { panic("constructor"); } }
function argument(): int { return 7; }
function main(): void throws Failure { let $task = new Task(argument()); }"#,
            r#"function argument(): int { return 7; }
function main(): void {
    let $callback = function (int $value): void { panic("closure"); };
    $callback(argument());
}"#,
            r#"function main(): void {
    List<int> $values = [1, 2];
    let $callback = function (int $value): int { panic("map callback"); };
    let $mapped = $values->map($callback);
}"#,
            r#"class Value { int $number { get { panic("getter"); } } }
function main(): void { let $value = new Value(); let $number = $value->number; }"#,
            r#"class Value { writable int $number { set (int $next) { panic("setter"); } } }
function argument(): int { return 7; }
function main(): void { let writable $value = new Value(); $value->number = argument(); }"#,
            r#"class Value implements Displayable { function toString(): string { panic("display"); } }
function main(): void { let $value = new Value(); echo "value={$value}"; }"#,
            // Collection helpers invoke authored contracts at the checked
            // insertion site, not the encompassing assignment/declaration.
            r#"class Key implements Hashable, Equatable<Key> {
    function hash(): uint64 { panic("indexed hash"); }
    function equals(Key $other): bool { return true; }
}
function argument(): int { return 7; }
function main(): void {
    writable Dictionary<Key, int> $values = [];
    $values[new Key()] = argument();
}"#,
            r#"class Key implements Hashable, Equatable<Key> {
    function hash(): uint64 { panic("literal hash"); }
    function equals(Key $other): bool { return true; }
}
function argument(): int { return 7; }
function main(): void { Dictionary<Key, int> $values = [new Key() => argument()]; }"#,
            r#"class Key implements Hashable, Equatable<Key> {
    function hash(): uint64 { return 1; }
    function equals(Key $other): bool { panic("indexed equality"); }
}
function argument(): int { return 7; }
function main(): void {
    writable Dictionary<Key, int> $values = [new Key() => 1];
    $values[new Key()] = argument();
}"#,
            r#"class Key implements Comparable<Key> {
    function compare(Key $other): Ordering { panic("indexed comparison"); }
}
function argument(): int { return 7; }
function main(): void {
    writable SortedDictionary<Key, int> $values = SortedDictionary::from([]);
    $values[new Key()] = 1;
    $values[new Key()] = argument();
}"#,
            // Checked propagation must run cleanup and remove the failed frame
            // before a later panic; abort must not run remaining cleanup.
            r#"class Failure implements Error { function __construct(string $message) {} }
class Marker { function __destruct() { try { echo "cleanup;"; } catch (Error) {} } }
function attempt(): int throws Failure {
    let $marker = new Marker();
    throw new Failure("retry");
}
function main(): void {
    try { let $result = attempt(); } catch (Failure) {}
    let $remaining = new Marker();
    panic("after checked cleanup");
}"#,
            // The primary divisor label differs from the complete RMW operation.
            r#"function main(): void {
    let writable $bytes = Bytes::allocate(1);
    uint8 $zero = 0;
    $bytes[0] /= $zero;
}"#,
        ]
        .into_iter()
        .map(|source| (source.to_string(), None))
        .chain(
            PHP_LIFECYCLE_PANIC_SOURCES
                .iter()
                .map(|source| (source.to_string(), None)),
        )
        .chain(shared_conflicts.map(|(source, reason)| {
            (format!("{shared_payload}\n{source}"), Some(reason))
        }))
        {
            let source = source.as_str();
            with_panic_fixture_context(path, source, || {
                let mir = doriac::lower_source_to_mir(path, source).unwrap();
                let expected = doriac::mir_interpreter::interpret(&mir).unwrap();
                let expected_diagnostic = expected.runtime_diagnostic.as_ref().unwrap();
                if let Some(reason) = conflict_reason {
                    assert_eq!(expected_diagnostic.code, "P1501", "{source}");
                    assert_eq!(expected_diagnostic.explanation.as_deref(), Some(reason));
                    assert_eq!(
                        expected_diagnostic.runtime_outcome.as_ref().unwrap().facts,
                        [RuntimeFact {
                            name: doria_diagnostic_catalogue::SHARED_ACCESS_CONFLICT_REASON_FACT
                                .to_string(),
                            value: RuntimeFactValue::StaticString(reason.to_string()),
                        }],
                        "{source}"
                    );
                }
                let php = doriac::compile_source_to_php(path, source).unwrap();
                let script = format!(
                    "{}\n__DoriaFunction_6d61696e();",
                    php.strip_prefix("<?php").unwrap()
                );
                let directory = ExecutionDirectory::new();
                let channel = directory.path().join("panic.outcome");
                let execution = php_execution::Script::new(&script);
                let execute = |transport: bool| {
                    let mut command = execution.command(&["-d", "ffi.enable=0"]);
                    if transport {
                        command.env("DORIA_RUNTIME_OUTCOME_V2", &channel);
                    } else {
                        command.env_remove("DORIA_RUNTIME_OUTCOME_V2");
                    }
                    command
                        .output()
                        .expect("PHP is required for panic transport compatibility parity")
                };
                let human = execute(false);
                assert_eq!(human.status.code(), Some(101), "{source}\n{human:?}");
                assert_eq!(human.stdout, expected.stdout, "{source}");
                assert_eq!(human.stderr, expected.stderr, "{source}");
                let transported = execute(true);
                assert_eq!(
                    transported.status.code(),
                    Some(101),
                    "{source}\n{transported:?}"
                );
                assert_eq!(transported.stdout, expected.stdout, "{source}");
                assert!(transported.stderr.is_empty(), "{source}\n{transported:?}");
                let record = fs::read(&channel).expect("typed panic record must be preserved");
                let decoded = decode_runtime_outcome(&record, path).unwrap();
                assert_eq!(decoded.code, expected_diagnostic.code, "{source}");
                assert_eq!(decoded.span, expected_diagnostic.span, "{source}");
                assert_eq!(
                    decoded.runtime_outcome,
                    expected_diagnostic.runtime_outcome,
                    "{source}"
                );
                let rendered = doriac::render_diagnostics_with_options(
                    path,
                    source,
                    &[decoded],
                    RenderOptions {
                        color: doriac::diagnostics::ColorChoice::Never,
                        context_lines: 0,
                        ..RenderOptions::default()
                    },
                );
                assert_eq!((rendered + "\n").as_bytes(), expected.stderr, "{source}");
            });
        }
    }

    #[test]
    fn php_panic_transport_failure_preserves_canonical_stderr_and_no_cleanup() {
        let path = "php-panic-channel-failure.doria";
        let source = r#"
class Marker {
    function __destruct() {
        try { echo "unexpected cleanup\n"; } catch (Doria\Std\Io\IoError) {}
    }
}
function main(): void {
    let $marker = new Marker();
    int[] $values = [7];
    let $value = $values[-1];
}
"#;
        let mir = doriac::lower_source_to_mir(path, source).unwrap();
        let expected = doriac::mir_interpreter::interpret(&mir).unwrap();
        let php = doriac::compile_source_to_php(path, source).unwrap();
        let script = format!(
            "{}\n__DoriaFunction_6d61696e();",
            php.strip_prefix("<?php").unwrap()
        );
        let directory = ExecutionDirectory::new();
        // A directory cannot receive a record; this fails on every platform
        // without relying on permissions or writing outside the test's root.
        let output = php_execution::Script::new(&script)
            .command(&["-d", "ffi.enable=0"])
            .env("DORIA_RUNTIME_OUTCOME_V2", directory.path())
            .output()
            .expect("PHP is required for panic transport compatibility parity");
        assert_eq!(output.status.code(), Some(101), "{output:?}");
        assert_eq!(output.stdout, expected.stdout);
        assert_eq!(output.stderr, expected.stderr);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[test]
    fn php_authored_caller_sites_match_interpreter_and_native_profiles() {
        for source in [
            r#"function argument(): int { return 7; }
function fail(int $value): void { panic("nested argument"); }
function main(): void { fail(argument()); }"#,
            r#"function descend(int $depth): void {
    if ($depth > 0) { descend($depth - 1); return; }
    panic("recursive stop");
}
function main(): void { descend(2); }"#,
            r#"class Value { int $number { get { panic("getter"); } } }
function main(): void { let $value = new Value(); let $number = $value->number; }"#,
            r#"function main(): void {
    List<int> $values = [1];
    let $callback = function (int $value): int { panic("callback"); };
    let $mapped = $values->map($callback);
}"#,
            r#"class Key implements Hashable, Equatable<Key> {
    function hash(): uint64 { panic("indexed hash"); }
    function equals(Key $other): bool { return true; }
}
function argument(): int { return 7; }
function main(): void {
    writable Dictionary<Key, int> $values = [];
    $values[new Key()] = argument();
}"#,
            r#"class Key implements Hashable, Equatable<Key> {
    function hash(): uint64 { panic("literal hash"); }
    function equals(Key $other): bool { return true; }
}
function argument(): int { return 7; }
function main(): void { Dictionary<Key, int> $values = [new Key() => argument()]; }"#,
        ]
        .into_iter()
        .chain(PHP_LIFECYCLE_PANIC_SOURCES.iter().copied())
        {
            let path = "caller-sites.doria";
            with_panic_fixture_context(path, source, || {
                let mir = doriac::lower_source_to_mir(path, source).unwrap();
                let expected = doriac::mir_interpreter::interpret(&mir).unwrap();
                let php = doriac::compile_source_to_php(path, source).unwrap();
                let output = php_execution::Script::new(&format!(
                    "{}\n__DoriaFunction_6d61696e();",
                    php.strip_prefix("<?php").unwrap(),
                ))
                .command(&["-d", "ffi.enable=0"])
                .env_remove("DORIA_RUNTIME_OUTCOME_V2")
                .output()
                .expect("PHP caller-site parity");
                assert_eq!(output.status.code(), Some(101), "{source}\n{output:?}");
                assert_eq!(output.stdout, expected.stdout, "{source}");
                assert_eq!(output.stderr, expected.stderr, "{source}");
                for &profile in NativeProfile::AVAILABLE {
                    let output = crate::panic_native_execution::execute_native(&mir, profile);
                    assert_eq!(
                        output.status.code(),
                        Some(101),
                        "{source}\n{profile:?}: {output:?}"
                    );
                    assert_eq!(output.stdout, expected.stdout, "{source}\n{profile:?}");
                    assert_eq!(output.stderr, expected.stderr, "{source}\n{profile:?}");
                }
            });
        }
    }

    #[test]
    fn php_multifile_panic_transport_matches_canonical_diagnostic_json() {
        let text = serde_json::json!({
            "schemaVersion": 1, "edition": "2026", "rootPackage": "acme/application",
            "selectedTarget": { "package": "acme/application", "name": "application",
                "kind": "binary", "entrySource": "acme/application:main.doria",
                "activeScopes": ["main"] },
            "packages": [{ "identity": "acme/application", "root": ".",
                "namespaceMappings": [{ "prefix": "", "path": "", "scope": "main" }],
                "sources": [
                { "identity": "acme/application:main.doria", "path": "main.doria",
                    "scope": "main", "origin": "entry" },
                { "identity": "acme/application:helpers.doria", "path": "helpers.doria",
                    "scope": "main", "origin": "explicit" },
                { "identity": "acme/application:leaf.doria", "path": "leaf.doria",
                    "scope": "main", "origin": "explicit" }
            ] }],
            "compiler": { "target": "php", "nativeProfile": null, "targetTriple": null }
        })
        .to_string();
        let document = doriac::build_plan::BuildPlanDocument {
            path: "plan.json".into(),
            directory: env::current_dir().unwrap(),
            plan: doriac::build_plan::parse_build_plan("plan.json", &text).unwrap(),
            text,
        };
        let mut provider = doriac::source_provider::InMemorySourceProvider::new();
        provider.insert(
            "acme/application",
            "main.doria",
            "function main(): void { relay(2); }",
        );
        provider.insert(
            "acme/application",
            "helpers.doria",
            r#"
function argument(): int { return 7; }
function relay(int $depth): void {
    if ($depth > 0) { relay($depth - 1); return; }
    leaf(argument());
}"#,
        );
        provider.insert(
            "acme/application",
            "leaf.doria",
            "function leaf(int $value): void { panic(\"multi-file\"); }",
        );
        let graph =
            doriac::compilation_graph::load_compilation_graph(&document, &provider).unwrap();
        let mir = doriac::lower_compilation_graph_to_mir(&graph).unwrap();
        let expected = doriac::mir_interpreter::interpret(&mir).unwrap();
        let BackendOutput::Text { contents: php, .. } =
            doriac::compile_compilation_graph(&graph).unwrap()
        else {
            panic!("this fixture requires no native PHP runtime dependency");
        };
        let directory = ExecutionDirectory::new();
        let channel = directory.path().join("panic.outcome");
        let output = php_execution::Script::new(&format!(
            "{}\n__DoriaFunction_6d61696e();",
            php.strip_prefix("<?php").unwrap(),
        ))
        .command(&["-d", "ffi.enable=0"])
        .env("DORIA_RUNTIME_OUTCOME_V2", &channel)
        .output()
        .expect("PHP multi-source transport");
        assert_eq!(output.status.code(), Some(101), "{output:?}");
        assert_eq!(output.stdout, expected.stdout);
        assert!(output.stderr.is_empty(), "{output:?}");
        let path = &graph
            .source_map
            .iter()
            .find(|source| source.identity.0 == "acme/application:main.doria")
            .unwrap()
            .display_path;
        let decoded = decode_runtime_outcome_with_sources(
            &fs::read(channel).unwrap(),
            path,
            Some(&graph.source_map),
        )
        .unwrap();
        let options = RenderOptions {
            format: DiagnosticFormat::Json,
            color: ColorChoice::Never,
            ..RenderOptions::default()
        };
        // Compare the complete public projection through the real renderer:
        // no span rewriting, frame filtering or rendered-output normalization.
        assert_eq!(
            doriac::diagnostics::render_diagnostics_with_source_map(
                &graph.source_map,
                &[decoded],
                options
            ),
            doriac::diagnostics::render_diagnostics_with_source_map(
                &graph.source_map,
                &[expected.runtime_diagnostic.unwrap()],
                options
            ),
        );
    }

    fn error_record(version: u16, error_type: &str, progress: Option<i64>) -> Vec<u8> {
        error_record_with_origin(version, error_type, progress, None)
    }

    fn error_record_with_origin(
        version: u16,
        error_type: &str,
        progress: Option<i64>,
        origin: Option<(&str, &str, &str, Span)>,
    ) -> Vec<u8> {
        let message = "failed transfer";
        let (path, source, function, span) = origin.unwrap_or(("", "", "", Span::default()));
        let mut bytes = Vec::new();
        bytes.extend_from_slice(if version == 5 {
            b"DORIAO5\0"
        } else {
            b"DORIAO3\0"
        });
        bytes.extend_from_slice(&version.to_le_bytes());
        bytes.extend_from_slice(&(error_type.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(message.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&(path.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(source.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(function.len() as u32).to_le_bytes());
        bytes.push(u8::from(origin.is_some()));
        bytes.extend_from_slice(&(span.start as u64).to_le_bytes());
        bytes.extend_from_slice(&(span.end as u64).to_le_bytes());
        if version == 5 {
            bytes.push(u8::from(progress.is_some()));
            bytes.extend_from_slice(&progress.unwrap_or(0).to_le_bytes());
        }
        bytes.extend_from_slice(error_type.as_bytes());
        bytes.extend_from_slice(message.as_bytes());
        bytes.extend_from_slice(path.as_bytes());
        bytes.extend_from_slice(source.as_bytes());
        bytes.extend_from_slice(function.as_bytes());
        bytes
    }

    #[test]
    fn runtime_error_transport_preserves_absent_zero_and_positive_progress() {
        for count in [None, Some(0), Some(2048)] {
            let payload = error_record(5, doriac::compiler_known_io::IO_ERROR, count);
            let diagnostic = decode_runtime_outcome(&payload, "main.doria").unwrap();
            let outcome = diagnostic.runtime_outcome.as_ref().unwrap();
            assert_eq!(outcome.process_status, 70);
            assert_eq!(
                outcome.termination_behavior,
                TerminationBehavior::PropagateWithCleanup
            );
            let expected = count
                .map(|value| RuntimeFact {
                    name: doria_diagnostic_catalogue::IO_TRANSFERRED_BYTE_COUNT_FACT.to_string(),
                    value: RuntimeFactValue::Signed(value),
                })
                .into_iter()
                .collect::<Vec<_>>();
            assert_eq!(outcome.facts, expected);
        }
        let legacy = error_record(3, doriac::compiler_known_io::IO_ERROR, None);
        assert!(decode_runtime_outcome(&legacy, "main.doria")
            .unwrap()
            .runtime_outcome
            .as_ref()
            .unwrap()
            .facts
            .is_empty());
    }

    #[test]
    fn runtime_error_transport_rejects_forged_or_ambiguous_progress() {
        let mut invalid_presence = error_record(5, doriac::compiler_known_io::IO_ERROR, None);
        invalid_presence[51] = 2;
        assert!(decode_error(&invalid_presence).contains("invalid I/O progress"));
        let mut hidden_count = error_record(5, doriac::compiler_known_io::IO_ERROR, None);
        hidden_count[52] = 1;
        assert!(decode_error(&hidden_count).contains("invalid I/O progress"));
        assert!(decode_error(&error_record(5, "OtherFailure", Some(0)))
            .contains("progress for a non-IoError"));
        let valid = error_record(5, doriac::compiler_known_io::IO_ERROR, Some(4));
        for length in 0..valid.len() {
            assert!(decode_runtime_outcome(&valid[..length], "main.doria").is_err());
        }
        let mut trailing = valid;
        trailing.push(0);
        assert!(decode_error(&trailing).contains("trailing runtime Error"));
    }

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

    fn assertion_record(
        matcher: &str,
        negated: bool,
        actual: Option<(&str, &str)>,
        expected: Option<(&str, &str)>,
        user_message: Option<&str>,
    ) -> Vec<u8> {
        let error_type = "Doria\\Std\\Test\\AssertionError";
        let path = "main.doria";
        let source = "function main(): void {}\n";
        let function = "main";
        let (actual_type, actual_presentation) = actual.unwrap_or_default();
        let (expected_type, expected_presentation) = expected.unwrap_or_default();
        let user_message_present = user_message.is_some();
        let user_message = user_message.unwrap_or_default();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"DORIAO4\0");
        bytes.extend_from_slice(&4_u16.to_le_bytes());
        bytes.extend_from_slice(&(error_type.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&70_u32.to_le_bytes());
        bytes.push(1);
        bytes.extend_from_slice(&(matcher.len() as u32).to_le_bytes());
        bytes.push(u8::from(negated));
        bytes.push(u8::from(actual.is_some()));
        bytes.extend_from_slice(&(actual_type.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(actual_presentation.len() as u32).to_le_bytes());
        bytes.push(u8::from(expected.is_some()));
        bytes.extend_from_slice(&(expected_type.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(expected_presentation.len() as u32).to_le_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.push(u8::from(user_message_present));
        bytes.extend_from_slice(&(user_message.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(path.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(source.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(function.len() as u32).to_le_bytes());
        bytes.push(1);
        bytes.extend_from_slice(&0_u64.to_le_bytes());
        bytes.extend_from_slice(&8_u64.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(error_type.as_bytes());
        bytes.extend_from_slice(matcher.as_bytes());
        bytes.extend_from_slice(actual_type.as_bytes());
        bytes.extend_from_slice(actual_presentation.as_bytes());
        bytes.extend_from_slice(expected_type.as_bytes());
        bytes.extend_from_slice(expected_presentation.as_bytes());
        bytes.extend_from_slice(user_message.as_bytes());
        bytes.extend_from_slice(path.as_bytes());
        bytes.extend_from_slice(source.as_bytes());
        bytes.extend_from_slice(function.as_bytes());
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

    fn runtime_transport_source_map(main_id: SourceId) -> doriac::source_map::SourceMap {
        doriac::source_map::SourceMap::from_ordered_records(
            [
                (SourceId(0), "helpers.doria", "function helper(): void {}\n"),
                (main_id, "main.doria", "function main(): void {}\n"),
            ]
            .into_iter()
            .map(|(id, path, text)| doriac::source_map::SourceRecord {
                identity: doriac::names::SourceIdentity(format!("acme/application:{path}")),
                package: doriac::names::PackageIdentity::Named("acme/application".into()),
                display_path: path.into(),
                canonical_path: None,
                content_fingerprint: String::new(),
                source: SourceFile::with_id(id, path, text),
            })
            .collect(),
        )
    }

    #[test]
    fn runtime_transport_binds_every_known_origin_to_its_compilation_source() {
        let text = "function main(): void {}\n";
        let payloads = [
            runtime_record("P1101", &[]),
            error_record_with_origin(
                3,
                doriac::compiler_known_io::IO_ERROR,
                None,
                Some(("main.doria", text, "main", Span::new(0, 8))),
            ),
            assertion_record(
                "Equal",
                false,
                Some(("int", "41")),
                Some(("int", "42")),
                None,
            ),
            error_record_with_origin(
                5,
                doriac::compiler_known_io::IO_ERROR,
                Some(4),
                Some(("main.doria", text, "main", Span::new(0, 8))),
            ),
        ];
        for main_id in [SourceId(1), SourceId(17)] {
            let sources = runtime_transport_source_map(main_id);
            for payload in &payloads {
                for current in ["main.doria", "helpers.doria"] {
                    let diagnostic =
                        decode_runtime_outcome_with_sources(payload, current, Some(&sources))
                            .unwrap();
                    let span = Span::in_source(main_id, 0, 8);
                    assert_eq!(diagnostic.span, span);
                    assert_eq!(diagnostic.labels[0].span, span);
                    assert_eq!(
                        diagnostic.runtime_outcome.as_ref().unwrap().origin.span,
                        span
                    );
                    let rendered = doriac::diagnostics::render_diagnostics_with_source_map(
                        &sources,
                        &[diagnostic],
                        RenderOptions {
                            format: DiagnosticFormat::Json,
                            color: ColorChoice::Never,
                            ..RenderOptions::default()
                        },
                    );
                    let projection: serde_json::Value = serde_json::from_str(&rendered).unwrap();
                    let diagnostic = &projection["diagnostics"][0];
                    for location in [
                        &diagnostic["labels"][0],
                        &diagnostic["runtimeOutcome"]["origin"],
                    ] {
                        assert_eq!(location["source"], "main.doria");
                        assert_eq!(location["range"]["start"]["line"], 1);
                        assert_eq!(location["range"]["start"]["column"], 1);
                        assert_eq!(location["range"]["end"]["line"], 1);
                        assert_eq!(location["range"]["end"]["column"], 9);
                    }
                }
            }
        }
    }

    #[test]
    fn runtime_transport_rejects_missing_or_invalid_graph_locations() {
        let sources = runtime_transport_source_map(SourceId(17));
        for (path, span) in [
            ("missing.doria", Span::new(0, 8)),
            ("main.doria", Span::new(9, 8)),
            ("main.doria", Span::new(0, 100)),
        ] {
            assert!(runtime_source_span(Some(&sources), path, span).is_err());
        }
        let empty = doriac::source_map::SourceMap::default();
        assert!(decode_runtime_outcome_with_sources(
            &runtime_record("P1101", &[]),
            "main.doria",
            Some(&empty),
        )
        .is_err());
        for version in [3, 5] {
            let diagnostic = decode_runtime_outcome_with_sources(
                &error_record(version, doriac::compiler_known_io::IO_ERROR, None),
                "main.doria",
                Some(&empty),
            )
            .unwrap();
            assert_eq!(
                diagnostic.runtime_outcome.as_ref().unwrap().origin.source,
                DiagnosticSource::Unavailable,
            );
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

    #[test]
    fn runtime_assertion_transport_is_strict_and_catalogued() {
        let payload = assertion_record(
            "Equal",
            false,
            Some(("int", "41")),
            Some(("int", "42")),
            None,
        );
        let diagnostic = decode_runtime_outcome(&payload, "main.doria").expect("valid V4 record");
        assert_eq!(diagnostic.code, "R1001");
        assert_eq!(
            diagnostic.kind,
            doriac::diagnostics::DiagnosticKind::RuntimeAssertion
        );
        let outcome = diagnostic
            .runtime_outcome
            .as_ref()
            .expect("runtime assertion outcome");
        assert_eq!(outcome.process_status, 70);
        assert_eq!(
            outcome.termination_behavior,
            TerminationBehavior::PropagateWithCleanup
        );
        assert_eq!(outcome.facts.len(), 12);
        assert!(outcome.facts.iter().map(|fact| fact.name.as_str()).eq(
            doria_diagnostic_catalogue::ASSERTION_FACT_NAMES
                .iter()
                .copied()
        ));

        let mut invalid_flag = payload.clone();
        invalid_flag[23] = 2;
        assert!(decode_error(&invalid_flag).contains("invalid assertion presence flag"));

        let mut trailing = payload.clone();
        trailing.push(0);
        assert!(decode_error(&trailing).contains("trailing runtime assertion bytes"));

        let mut oversized = payload.clone();
        oversized[29..33].copy_from_slice(&4097_u32.to_le_bytes());
        assert!(decode_error(&oversized).contains("oversized runtime assertion record"));

        let mut invalid_identity = payload;
        let error_type = b"Doria\\Std\\Test\\AssertionError";
        let error_type_offset = invalid_identity
            .windows(error_type.len())
            .position(|window| window == error_type)
            .expect("encoded assertion error type");
        invalid_identity[error_type_offset] = b'X';
        assert!(decode_error(&invalid_identity).contains("invalid Slice-2 assertion identity"));
    }
}
