use cranelift_object::object::{Object, ObjectSection, ObjectSymbol};
use doriac::backend::NativeProfile;
use doriac::build_plan::{
    BuildPlan, BuildPlanDocument, CompilerOptions, CompilerTarget, NamespaceMapping, Package,
    SelectedTarget, Source, SourceOrigin, SourceScope, TargetKind,
};
use doriac::mir::{ImplementingType, Program};
use doriac::native_abi::{
    ASSERTION_ERROR_DESCRIPTOR_MAGIC, CHECKED_IO, INTERFACE_VTABLE_HEADER_WORDS,
    INTERFACE_VTABLE_IO_PROGRESS_MAGIC_WORD, INTERFACE_VTABLE_IO_TRANSFERRED_COUNT_WORD,
    IO_ERROR_PROGRESS_DESCRIPTOR_MAGIC,
};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

const SOURCE: &str = r#"
internal interface Reader { function read(): int; }
internal class Number implements Reader {
    function read(): int { return 7; }
}
internal class Failure implements Error {
    function __construct(string $message) {}
}
function read(Reader $reader): int { return $reader->read(); }
function values(Iterable<int> $values): void {
    foreach ($values as int $value) { echo $value; }
}
function assertionMessage(Doria\Std\Test\AssertionError $error): string {
    return $error->message;
}
function main(): void {
    echo read(new Number());
    List<int> $items = [1, 2];
    values($items);
    try { read_file("missing"); }
    catch (Doria\Std\Io\IoError $error) {
        echo $error->transferredByteCount ?? -1;
    }
}
"#;

fn lower() -> Program {
    lower_development_source(SOURCE)
}

fn lower_development_source(source: &str) -> Program {
    const PACKAGE: &str = "test/io-progress";
    const PATH: &str = "main.doria";
    let identity = format!("{PACKAGE}:{PATH}");
    let document = BuildPlanDocument {
        path: "build-plan.json".to_owned(),
        directory: std::env::current_dir().unwrap(),
        text: String::new(),
        plan: BuildPlan {
            schema_version: 1,
            edition: "2026".to_owned(),
            root_package: PACKAGE.to_owned(),
            selected_target: SelectedTarget {
                package: PACKAGE.to_owned(),
                name: "io-progress".to_owned(),
                kind: TargetKind::Binary,
                entry_source: Some(identity.clone()),
                active_scopes: vec![SourceScope::Main, SourceScope::Development],
            },
            packages: vec![Package {
                identity: PACKAGE.to_owned(),
                root: ".".to_owned(),
                namespace_mappings: vec![NamespaceMapping {
                    prefix: String::new(),
                    path: String::new(),
                    scope: SourceScope::Development,
                    generated_for: None,
                }],
                sources: vec![Source {
                    identity,
                    path: PATH.to_owned(),
                    scope: SourceScope::Development,
                    origin: SourceOrigin::Entry,
                    generated_for: None,
                }],
                dependencies: Vec::new(),
            }],
            compiler: CompilerOptions {
                target: CompilerTarget::Debug,
                native_profile: None,
                target_triple: None,
            },
        },
    };
    let mut provider = doriac::source_provider::InMemorySourceProvider::new();
    provider.insert(PACKAGE, PATH, source);
    let graph = doriac::compilation_graph::load_compilation_graph(&document, &provider).unwrap();
    doriac::lower_compilation_graph_to_mir(&graph)
        .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"))
}

fn assert_headers(program: &Program, bytes: &[u8]) {
    let object = cranelift_object::object::File::parse(bytes).unwrap();
    // `is_64` is false for every COFF object, x86-64 included.
    let width = usize::from(object.architecture().address_size().unwrap().bytes());
    let word = |bytes: &[u8], index: usize| {
        let mut value = [0_u8; 8];
        let raw = &bytes[index * width..(index + 1) * width];
        if object.is_little_endian() {
            value[..width].copy_from_slice(raw);
            u64::from_le_bytes(value)
        } else {
            value[8 - width..].copy_from_slice(raw);
            u64::from_be_bytes(value)
        }
    };
    let mut saw_io = false;
    let mut saw_assertion = false;
    let mut saw_nonclass = false;
    let mut saw_methods = false;
    for table in &program.interface_vtables {
        let name = format!("__doria_interface_vtable_{}", table.id.0);
        let symbol = object
            .symbols()
            .find(|symbol| {
                symbol.name().is_ok_and(|actual| {
                    actual.trim_start_matches('_') == name.trim_start_matches('_')
                })
            })
            .unwrap_or_else(|| panic!("missing {name}"));
        let section = object
            .section_by_index(symbol.section_index().unwrap())
            .unwrap();
        let data = section
            .data_range(
                symbol.address(),
                (width * (INTERFACE_VTABLE_HEADER_WORDS + table.methods.len())) as u64,
            )
            .unwrap()
            .unwrap();
        let descriptor = table
            .error_descriptor
            .map(|id| &program.error_descriptors[id.0]);
        let progress_offset = descriptor
            .and_then(|descriptor| descriptor.io_transferred_byte_count_property)
            .map(|property| {
                let class = &program.classes[property.class.0];
                class
                    .layout
                    .properties
                    .iter()
                    .find(|layout| layout.id == property)
                    .unwrap()
                    .offset
            });
        assert_eq!(
            word(data, INTERFACE_VTABLE_IO_TRANSFERRED_COUNT_WORD as usize),
            u64::from(progress_offset.unwrap_or(0)),
            "{name} progress projection",
        );
        assert_eq!(
            word(data, INTERFACE_VTABLE_IO_PROGRESS_MAGIC_WORD as usize),
            progress_offset.map_or(0, |_| IO_ERROR_PROGRESS_DESCRIPTOR_MAGIC),
            "{name} progress discriminator",
        );
        assert_eq!(
            word(data, 9),
            table.interface.0 as u64,
            "{name} interface identity"
        );
        assert_eq!(
            word(data, 7),
            descriptor
                .and_then(|descriptor| descriptor.assertion.as_ref())
                .map_or(0, |_| ASSERTION_ERROR_DESCRIPTOR_MAGIC),
            "{name} assertion discriminator",
        );
        let start = symbol.address() - section.address();
        for slot in 0..table.methods.len() {
            let offset = start + (width * (INTERFACE_VTABLE_HEADER_WORDS + slot)) as u64;
            assert!(
                section.relocations().any(|(actual, _)| actual == offset),
                "{name} method#{slot} has no relocation after the complete header"
            );
            saw_methods = true;
        }
        saw_io |= progress_offset.is_some();
        saw_assertion |= descriptor.is_some_and(|descriptor| descriptor.assertion.is_some());
        saw_nonclass |= !matches!(table.implementing_type, ImplementingType::Class(_));
    }
    assert!(saw_io && saw_assertion && saw_nonclass && saw_methods);
    assert!(
        object.symbols().any(|symbol| {
            symbol
                .name()
                .is_ok_and(|name| name.trim_start_matches('_') == CHECKED_IO)
        }),
        "checked I/O must reference the progress-capable ABI"
    );
    assert!(!object.symbols().any(|symbol| {
        symbol
            .name()
            .is_ok_and(|name| name.trim_start_matches('_') == "dr_v3_checked_io")
    }));
}

#[test]
fn cranelift_error_progress_metadata_preserves_all_interface_headers() {
    let program = lower();
    let object = doriac::codegen_cranelift::lower_mir_to_object(&program).unwrap();
    assert_headers(&program, &object);
}

#[cfg(feature = "llvm-backend")]
#[test]
fn llvm_error_progress_metadata_preserves_all_interface_headers() {
    let program = lower();
    let ir = doriac::codegen_llvm::lower_mir_to_llvm_ir(&program).unwrap();
    let word = format!("i{}", usize::BITS);
    for table in &program.interface_vtables {
        let name = format!("@__doria_interface_vtable_{} =", table.id.0);
        let line = ir.lines().find(|line| line.starts_with(&name)).unwrap();
        let descriptor = table
            .error_descriptor
            .map(|id| &program.error_descriptors[id.0]);
        let progress_offset = descriptor
            .and_then(|descriptor| descriptor.io_transferred_byte_count_property)
            .map(|property| {
                program.classes[property.class.0]
                    .layout
                    .properties
                    .iter()
                    .find(|layout| layout.id == property)
                    .unwrap()
                    .offset
            });
        let expected = format!(
            "{word} {}, {word} {}, {word} {}, [{} x ptr]",
            table.interface.0,
            progress_offset.unwrap_or(0),
            progress_offset.map_or(0, |_| IO_ERROR_PROGRESS_DESCRIPTOR_MAGIC),
            table.methods.len(),
        );
        assert!(line.contains(&expected), "{line}\nexpected {expected}");
        if descriptor.is_some_and(|descriptor| descriptor.assertion.is_some()) {
            assert!(
                line.contains(&format!("{word} {ASSERTION_ERROR_DESCRIPTOR_MAGIC}, ptr")),
                "{line}"
            );
        }
    }
    let call = ir
        .lines()
        .find(|line| line.contains("call i8 @dr_v4_checked_io("))
        .unwrap();
    assert_eq!(call.matches(", ").count(), 12, "{call}");
    assert!(ir.contains("checked.io.transferred-count"));
}

struct NativeProgram {
    directory: PathBuf,
    executable: PathBuf,
}

impl NativeProgram {
    fn new(program: &Program, profile: NativeProfile) -> Self {
        static NEXT_RUN: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "doria-io-progress-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_RUN.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir(&directory).unwrap();
        let executable = directory.join(if cfg!(windows) {
            "program.exe"
        } else {
            "program"
        });
        let files = Self {
            directory,
            executable,
        };
        let bytes = doriac::codegen_native::generate_executable(program, profile)
            .unwrap_or_else(|error| panic!("{profile:?}: {error:?}"));
        std::fs::write(&files.executable, bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&files.executable, std::fs::Permissions::from_mode(0o700))
                .unwrap();
        }
        files
    }

    fn run(&self, v3: Option<&str>, v4: Option<&str>, v5: Option<&str>) -> Output {
        let mut command = Command::new(&self.executable);
        command
            .current_dir(&self.directory)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for key in [
            "DORIA_RUNTIME_OUTCOME_V2",
            "DORIA_RUNTIME_OUTCOME_V3",
            "DORIA_RUNTIME_OUTCOME_V4",
            "DORIA_RUNTIME_OUTCOME_V5",
        ] {
            command.env_remove(key);
        }
        for (key, path) in [
            ("DORIA_RUNTIME_OUTCOME_V3", v3),
            ("DORIA_RUNTIME_OUTCOME_V4", v4),
            ("DORIA_RUNTIME_OUTCOME_V5", v5),
        ] {
            if let Some(path) = path {
                command.env(key, self.directory.join(path));
            }
        }
        doriac::native_process::spawn(&mut command)
            .unwrap()
            .wait_with_output()
            .unwrap()
    }

    fn record(&self, name: &str) -> Vec<u8> {
        std::fs::read(self.directory.join(name)).unwrap()
    }
}

impl Drop for NativeProgram {
    fn drop(&mut self) {
        for path in [
            &self.executable,
            &self.directory.join("v3"),
            &self.directory.join("v4"),
            &self.directory.join("v5"),
        ] {
            let _ = std::fs::remove_file(path);
        }
        let _ = std::fs::remove_dir(&self.directory);
    }
}

fn assert_error_record(payload: &[u8], version: u16, error_type: &str, count: Option<i64>) {
    let header_length = if version == 5 { 60 } else { 51 };
    assert_eq!(
        &payload[..8],
        if version == 5 {
            b"DORIAO5\0"
        } else {
            b"DORIAO3\0"
        }
    );
    assert_eq!(
        u16::from_le_bytes(payload[8..10].try_into().unwrap()),
        version
    );
    let type_length = u32::from_le_bytes(payload[10..14].try_into().unwrap()) as usize;
    let message_length = u64::from_le_bytes(payload[14..22].try_into().unwrap()) as usize;
    let path_length = u32::from_le_bytes(payload[22..26].try_into().unwrap()) as usize;
    let source_length = u32::from_le_bytes(payload[26..30].try_into().unwrap()) as usize;
    let function_length = u32::from_le_bytes(payload[30..34].try_into().unwrap()) as usize;
    assert_eq!(
        payload.len(),
        header_length
            + type_length
            + message_length
            + path_length
            + source_length
            + function_length
    );
    assert_eq!(
        &payload[header_length..header_length + type_length],
        error_type.as_bytes()
    );
    assert_eq!(payload[34], 1, "throw origin must remain available");
    assert!(
        u64::from_le_bytes(payload[43..51].try_into().unwrap())
            > u64::from_le_bytes(payload[35..43].try_into().unwrap())
    );
    if version == 5 {
        assert_eq!(payload[51], u8::from(count.is_some()));
        assert_eq!(
            i64::from_le_bytes(payload[52..60].try_into().unwrap()),
            count.unwrap_or(0)
        );
    }
}

fn escaping_io_source(count: &str) -> String {
    format!(
        r#"
class Cleanup {{
    function __destruct() {{ try {{ echo "cleanup;"; }} catch (Error) {{}} }}
}}
function main(): void {{
    let $cleanup = new Cleanup();
    echo "before;";
    throw new Doria\Std\Io\IoError(
        message: "transfer failed",
        operation: Doria\Std\Io\IoOperation::Write,
        target: Doria\Std\Io\IoTarget::StandardOutput,
        reason: Doria\Std\Io\IoErrorReason::Other,
        transferredByteCount: {count},
    );
}}
"#
    )
}

#[test]
fn native_escaping_io_errors_preserve_optional_signed_progress_and_v3_compatibility() {
    for count in [None, Some(0), Some(2048), Some(-3), Some(i64::MAX)] {
        let source =
            escaping_io_source(&count.map_or_else(|| "null".to_owned(), |value| value.to_string()));
        let program = doriac::lower_source_to_mir("io-progress-escape.doria", &source).unwrap();
        for &profile in NativeProfile::AVAILABLE {
            let native = NativeProgram::new(&program, profile);
            let result = native.run(Some("v3"), None, Some("v5"));
            assert_eq!(result.status.code(), Some(70), "{profile:?}: {result:?}");
            assert_eq!(result.stdout, b"before;cleanup;");
            assert!(result.stderr.is_empty(), "{profile:?}: {result:?}");
            assert_error_record(
                &native.record("v5"),
                5,
                doriac::compiler_known_io::IO_ERROR,
                count,
            );
            assert!(
                !native.directory.join("v3").exists(),
                "v5 must have precedence"
            );

            let result = native.run(Some("v3"), None, None);
            assert_eq!(result.status.code(), Some(70), "{profile:?}: {result:?}");
            assert_eq!(result.stdout, b"before;cleanup;");
            assert!(result.stderr.is_empty(), "{profile:?}: {result:?}");
            assert_error_record(
                &native.record("v3"),
                3,
                doriac::compiler_known_io::IO_ERROR,
                None,
            );
        }
    }
}

#[test]
fn native_failed_progress_publication_keeps_human_facts_and_does_not_fall_back_to_v3() {
    for count in [None, Some(0), Some(2048), Some(-3), Some(i64::MAX)] {
        let source =
            escaping_io_source(&count.map_or_else(|| "null".to_owned(), |value| value.to_string()));
        let program = doriac::lower_source_to_mir("io-progress-fallback.doria", &source).unwrap();
        for &profile in NativeProfile::AVAILABLE {
            let native = NativeProgram::new(&program, profile);
            let result = native.run(Some("v3"), None, Some("missing/v5"));
            assert_eq!(result.status.code(), Some(70), "{profile:?}: {result:?}");
            assert_eq!(result.stdout, b"before;cleanup;");
            assert!(!native.directory.join("v3").exists());
            let stderr = String::from_utf8(result.stderr).unwrap();
            assert!(
                stderr.starts_with("Error[R1000]: Unhandled Doria\\Std\\Io\\IoError\n"),
                "{stderr}"
            );
            if let Some(count) = count {
                assert!(
                    stderr.contains(&format!("\n\nTransferred Byte Count\n  {count}\n")),
                    "{stderr}"
                );
            } else {
                assert!(!stderr.contains("Transferred Byte Count"), "{stderr}");
            }
            assert!(
                stderr.ends_with("\n\nProcess Exited With Status 70\n"),
                "{stderr}"
            );
        }
    }
}

#[test]
fn native_non_io_error_with_a_similar_property_has_no_io_progress_fact() {
    let source = r#"
class Failure implements Error {
    function __construct(string $message, ?int $transferredByteCount) {}
}
function main(): void { throw new Failure("user failed", 2048); }
"#;
    let program = doriac::lower_source_to_mir("non-io-progress.doria", source).unwrap();
    for &profile in NativeProfile::AVAILABLE {
        let native = NativeProgram::new(&program, profile);
        let result = native.run(None, None, Some("v5"));
        assert_eq!(result.status.code(), Some(70), "{profile:?}: {result:?}");
        assert!(result.stdout.is_empty());
        assert!(result.stderr.is_empty());
        assert_error_record(&native.record("v5"), 5, "Failure", None);
        let result = native.run(None, None, Some("missing/v5"));
        assert_eq!(result.status.code(), Some(70));
        let stderr = String::from_utf8(result.stderr).unwrap();
        assert!(!stderr.contains("Transferred Byte Count"), "{stderr}");
    }
}

#[test]
fn native_assertion_outcomes_remain_v4_when_generic_progress_transport_is_enabled() {
    let program = lower_development_source(
        r#"
use Doria\Std\Test\fail;
function main(): void { fail("assertion failed"); }
"#,
    );
    for &profile in NativeProfile::AVAILABLE {
        let native = NativeProgram::new(&program, profile);
        let result = native.run(Some("v3"), Some("v4"), Some("v5"));
        assert_eq!(result.status.code(), Some(70), "{profile:?}: {result:?}");
        assert!(result.stdout.is_empty());
        assert!(result.stderr.is_empty());
        let record = native.record("v4");
        assert!(record.starts_with(b"DORIAO4\0\x04\0"));
        assert!(record
            .windows(b"Doria\\Std\\Test\\AssertionError".len())
            .any(|value| value == b"Doria\\Std\\Test\\AssertionError"));
        assert!(!native.directory.join("v3").exists());
        assert!(!native.directory.join("v5").exists());
    }
}
