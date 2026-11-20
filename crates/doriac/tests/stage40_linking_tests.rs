//! Stage 40 linking (Decision 0148 sections 18 to 21, amended 2026-11-20):
//! typed link inputs per logical library from build-plan schema 2 and the
//! direct flags, the missing-input diagnostic, and native links that use
//! them.

#[path = "common/execution_directory.rs"]
mod execution_directory;

use doriac::backend::{BackendOutput, NativeProfile};
use doriac::c_layout::{CTarget, TargetArchitecture, TargetOperatingSystem};
use doriac::native_link::NativeLinking;
use execution_directory::ExecutionDirectory;
use std::path::Path;
use std::process::Command;

/// `cbrt` from the logical library `math`: libm on Linux and macOS, the
/// Universal CRT on Windows.
const CBRT: &str = r#"
use Doria\Std\Ffi\Library;

#[Library("math")]
extern "C"
{
    /** @safety Any value. */
    function cbrt(float64 $value): float64;
}

function main(): void
{
    echo unsafe(cbrt(27.0)) == 3.0;
}
"#;

const MATH_LIBRARY: &str = if cfg!(windows) { "ucrt" } else { "m" };

fn program(source: &str) -> doriac::mir::Program {
    doriac::lower_source_to_mir("linking.doria", source.to_string())
        .unwrap_or_else(|diagnostics| panic!("the program should build: {diagnostics:#?}"))
}

fn linking(flags: &[(&str, String)]) -> NativeLinking {
    let mut linking = NativeLinking::default();
    for (flag, value) in flags {
        linking
            .apply_flag(flag, value, Path::new("."))
            .unwrap_or_else(|error| panic!("{flag} {value}: {error}"));
    }
    linking.finish_flags().unwrap();
    linking
}

fn executable_name() -> &'static str {
    if cfg!(windows) {
        "program.exe"
    } else {
        "program"
    }
}

fn run(executable: &Path) -> std::process::Output {
    Command::new(executable)
        .output()
        .unwrap_or_else(|error| panic!("{}: {error}", executable.display()))
}

#[test]
fn a_native_library_links_from_its_flags_and_publishes_its_runtime_files() {
    let mir = program(CBRT);
    for profile in NativeProfile::AVAILABLE {
        let directory = ExecutionDirectory::new();
        let inputs = directory.path().join("inputs");
        std::fs::create_dir(&inputs).unwrap();
        let notice = inputs.join("notice.txt");
        std::fs::write(&notice, "runtime file").unwrap();
        let linking = linking(&[
            ("--link-library", format!("math={MATH_LIBRARY}")),
            ("--runtime-file", format!("math={}", notice.display())),
        ]);
        let executable =
            doriac::codegen_native::generate_linked_executable(&mir, *profile, &linking)
                .unwrap_or_else(|error| panic!("{profile:?}: {error:?}"));
        assert_eq!(executable.runtime_files, vec![notice.clone()]);
        let out = directory.path().join("bin").join(executable_name());
        doriac::backend::publish_output(
            &out,
            BackendOutput::Executable {
                extension: String::new(),
                bytes: executable.bytes,
                runtime_files: executable.runtime_files,
            },
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(directory.path().join("bin").join("notice.txt")).unwrap(),
            "runtime file"
        );
        let output = run(&out);
        assert_eq!(output.status.code(), Some(0), "{profile:?}: {output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "true",
            "{profile:?}"
        );
    }
}

#[test]
fn a_library_without_link_inputs_names_the_manifest_entry_and_the_flag() {
    let mir = program(CBRT);
    let error = doriac::codegen_native::generate_linked_executable(
        &mir,
        NativeProfile::Fast,
        &NativeLinking::default(),
    )
    .expect_err("`math` has no link input");
    let diagnostics = error.diagnostics.expect("structured diagnostics");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.code, "E0804");
    assert_eq!(diagnostic.title, "Native Library Has No Link Input");
    assert!(
        diagnostic.message.contains("`math`"),
        "{}",
        diagnostic.message
    );
    let help = diagnostic.helps.join("\n");
    assert!(help.contains("[native-libraries.math]"), "{help}");
    assert!(help.contains("--link-library math=<library>"), "{help}");
    assert_eq!(
        &CBRT[diagnostic.span.start..diagnostic.span.end]
            .split('(')
            .next(),
        &Some("function cbrt"),
        "the diagnostic points at the binding"
    );

    // A library no emitted code references needs no input.
    let unused = linking(&[("--link-library", "zlib=z".to_string())]);
    let without_math = program("function main(): void { echo 1; }");
    assert!(unused.resolve(&without_math, CTarget::host()).is_ok());
}

fn target(operating_system: TargetOperatingSystem) -> CTarget {
    CTarget {
        operating_system,
        architecture: TargetArchitecture::X64,
    }
}

#[test]
fn inputs_a_target_cannot_use_are_reported_where_the_library_is_used() {
    let mir = program(CBRT);
    let framework = linking(&[("--link-framework", "math=Accelerate".to_string())]);
    let diagnostics = framework
        .resolve(&mir, target(TargetOperatingSystem::Linux))
        .expect_err("frameworks exist only on macOS");
    assert_eq!(
        diagnostics
            .iter()
            .map(|diagnostic| (diagnostic.code, diagnostic.title.as_str()))
            .collect::<Vec<_>>(),
        [("E0805", "Frameworks Link Only On macOS")]
    );
    assert!(framework
        .resolve(&mir, target(TargetOperatingSystem::MacOs))
        .is_ok());

    let directory = ExecutionDirectory::new();
    let missing = linking(&[
        ("--link-library", "math=m".to_string()),
        (
            "--runtime-file",
            format!("math={}", directory.path().join("absent.dll").display()),
        ),
    ]);
    let diagnostics = missing.resolve(&mir, CTarget::host()).unwrap_err();
    assert_eq!(diagnostics[0].title, "Runtime File Was Not Found");

    // ld64 cannot prefer an archive, so a static library on macOS links by
    // the file a search path holds.
    let static_math = linking(&[
        ("--link-library", "math=static:m".to_string()),
        (
            "--link-search",
            format!("math={}", directory.path().display()),
        ),
    ]);
    let diagnostics = static_math
        .resolve(&mir, target(TargetOperatingSystem::MacOs))
        .unwrap_err();
    assert_eq!(diagnostics[0].title, "Static Library Was Not Found");
    std::fs::write(directory.path().join("libm.a"), "").unwrap();
    let plan = static_math
        .resolve(&mir, target(TargetOperatingSystem::MacOs))
        .unwrap();
    assert!(plan.items.contains(&doriac::native_link::LinkItem::File(
        directory.path().join("libm.a")
    )));

    let gui = linking(&[
        ("--link-library", "math=m".to_string()),
        ("--windows-subsystem", "gui".to_string()),
    ]);
    let diagnostics = gui
        .resolve(&mir, target(TargetOperatingSystem::Windows))
        .unwrap_err();
    assert_eq!(
        (diagnostics[0].code, diagnostics[0].title.as_str()),
        ("E0784", "Windows GUI Subsystem Is Not Available Yet")
    );
    assert!(gui
        .resolve(&mir, target(TargetOperatingSystem::Linux))
        .is_ok());
}

/// A schema-2 plan whose one package names `math` in `nativeLibraries`
/// with a package-relative search path.
fn schema_2_plan(native_libraries: &str, unsafe_policy: &str) -> String {
    format!(
        r#"{{
  "schemaVersion": 2,
  "edition": "2026",
  "rootPackage": "acme/linking",
  "selectedTarget": {{
    "package": "acme/linking",
    "name": "linking",
    "kind": "binary",
    "entrySource": "acme/linking:src/main.doria",
    "activeScopes": ["main"],
    "windowsSubsystem": "console"
  }},
  "packages": [
    {{
      "identity": "acme/linking",
      "root": ".",
      "namespaceMappings": [
        {{ "prefix": "", "path": "src/", "scope": "main" }}
      ],
      "sources": [
        {{
          "identity": "acme/linking:src/main.doria",
          "path": "src/main.doria",
          "scope": "main",
          "origin": "entry"
        }}
      ],
      "nativeLibraries": {native_libraries},
      "unsafePolicy": "{unsafe_policy}"
    }}
  ],
  "compiler": {{
    "target": "native",
    "nativeProfile": "fast",
    "targetTriple": null
  }}
}}"#
    )
}

fn math_libraries() -> String {
    format!(
        r#"[
        {{
          "name": "math",
          "link": "dynamic",
          "inputs": [
            {{ "kind": "searchPath", "path": "native/lib" }},
            {{ "kind": "library", "name": "{MATH_LIBRARY}" }}
          ]
        }}
      ]"#
    )
}

fn plan_errors(text: &str) -> Vec<String> {
    doriac::build_plan::parse_build_plan("plan.json", text)
        .expect_err("the plan is invalid")
        .into_iter()
        .map(|diagnostic| diagnostic.message.clone())
        .collect()
}

#[test]
fn build_plan_schema_2_carries_typed_link_inputs_and_keeps_schema_1() {
    let text = schema_2_plan(&math_libraries(), "allow");
    let plan = doriac::build_plan::parse_build_plan("plan.json", &text).unwrap();
    assert_eq!(plan.packages[0].native_libraries[0].name, "math");
    let encoded = doriac::build_plan::encode_build_plan(&plan).unwrap();
    assert_eq!(
        doriac::build_plan::parse_build_plan("plan.json", &encoded).unwrap(),
        plan
    );

    let schema_1 = text.replace("\"schemaVersion\": 2", "\"schemaVersion\": 1");
    let errors = plan_errors(&schema_1);
    for fragment in [
        "selectedTarget.windowsSubsystem requires build-plan schema 2",
        "declares unsafePolicy, which requires build-plan schema 2",
        "declares nativeLibraries, which requires build-plan schema 2",
    ] {
        assert!(
            errors.iter().any(|error| error.contains(fragment)),
            "{fragment}: {errors:#?}"
        );
    }

    let errors = plan_errors(
        &text
            .replace(",\n    \"windowsSubsystem\": \"console\"", "")
            .replace(",\n      \"unsafePolicy\": \"allow\"", ""),
    );
    for fragment in [
        "a binary selected target requires a windowsSubsystem",
        "requires an unsafePolicy in build-plan schema 2",
    ] {
        assert!(
            errors.iter().any(|error| error.contains(fragment)),
            "{fragment}: {errors:#?}"
        );
    }

    for (libraries, fragment) in [
        (
            math_libraries().replace("searchPath", "linkerArgument"),
            "unknown variant `linkerArgument`",
        ),
        (
            math_libraries().replace("\"path\": \"native/lib\"", "\"path\": \"../outside\""),
            "without parent traversal",
        ),
        (
            math_libraries().replace(
                &format!("\"name\": \"{MATH_LIBRARY}\""),
                "\"name\": \"-Wl,-undefined\"",
            ),
            "not a library name",
        ),
        (
            math_libraries().replace("\"dynamic\"", "\"shared\""),
            "unknown variant `shared`",
        ),
    ] {
        let errors = plan_errors(&schema_2_plan(&libraries, "allow"));
        assert!(
            errors.iter().any(|error| error.contains(fragment)),
            "{fragment}: {errors:#?}"
        );
    }
}

#[test]
fn a_build_plan_links_its_package_libraries_and_refuses_an_unenforced_deny_policy() {
    let directory = ExecutionDirectory::new();
    std::fs::create_dir_all(directory.path().join("src")).unwrap();
    std::fs::create_dir_all(directory.path().join("native/lib")).unwrap();
    std::fs::write(directory.path().join("src/main.doria"), CBRT).unwrap();
    let plan_path = directory.path().join("plan.json");
    std::fs::write(&plan_path, schema_2_plan(&math_libraries(), "allow")).unwrap();

    let (_, graph) = doriac::load_build_plan_file(&plan_path).unwrap();
    let mir = doriac::lower_compilation_graph_to_mir(&graph).unwrap();
    let plan = doriac::compilation_graph::native_linking(&graph)
        .resolve(&mir, CTarget::host())
        .unwrap();
    let root = std::fs::canonicalize(directory.path()).unwrap();
    assert_eq!(
        plan.items[0],
        doriac::native_link::LinkItem::SearchPath(root.join("native/lib"))
    );
    let BackendOutput::Executable { bytes, .. } =
        doriac::compile_compilation_graph(&graph).unwrap()
    else {
        panic!("a native plan builds an executable");
    };
    let out = directory.path().join(executable_name());
    doriac::backend::publish_output(
        &out,
        BackendOutput::Executable {
            extension: String::new(),
            bytes,
            runtime_files: Vec::new(),
        },
    )
    .unwrap();
    let output = run(&out);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "true");

    // Libraries belong to the package that declares them.
    std::fs::write(&plan_path, schema_2_plan("[]", "allow")).unwrap();
    let (_, graph) = doriac::load_build_plan_file(&plan_path).unwrap();
    let diagnostics = doriac::compile_compilation_graph(&graph).unwrap_err();
    assert_eq!(diagnostics[0].code, "E0804", "{diagnostics:#?}");
    assert!(diagnostics[0]
        .message
        .contains("`math` in package `acme/linking`"));

    std::fs::write(&plan_path, schema_2_plan(&math_libraries(), "deny")).unwrap();
    let diagnostics = doriac::load_build_plan_file(&plan_path).unwrap_err();
    assert_eq!(
        (diagnostics[0].code, diagnostics[0].title.as_str()),
        ("E0784", "Package Unsafe Policies Are Not Available Yet")
    );
}
