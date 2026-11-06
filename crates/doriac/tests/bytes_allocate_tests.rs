#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution.rs"]
mod native_execution;

#[path = "common/php_execution.rs"]
mod php_execution;

use cranelift_object::object::{
    Object, ObjectSection, ObjectSymbol, RelocationTarget, SectionKind,
};
use doriac::backend::NativeProfile;
use doriac::diagnostics::RuntimeFactValue;
use doriac::mir::{CollectionExpression, CollectionKind, IntegerExpression, Rvalue, Statement};
use doriac::numeric::{IntegerType, IntegerValue};

const NEGATIVE_PATH: &str = "examples/native/main_bytes_allocate_negative.doria";
const NEGATIVE: &str = include_str!("../../../examples/native/main_bytes_allocate_negative.doria");
const SIMPLE: &str = "function main(): void { let $bytes = Bytes::allocate(4); }";

fn profiles() -> Vec<NativeProfile> {
    #[cfg(feature = "llvm-backend")]
    {
        vec![NativeProfile::Fast, NativeProfile::Release]
    }
    #[cfg(not(feature = "llvm-backend"))]
    {
        vec![NativeProfile::Fast]
    }
}

fn lower(path: &str, source: &str) -> doriac::mir::Program {
    doriac::lower_source_to_mir(path, source)
        .unwrap_or_else(|diagnostics| panic!("{source}\n{diagnostics:#?}"))
}

fn assert_execution(source: &str, stdout: &str) {
    let mir = lower("bytes-allocate.doria", source);
    let output = doriac::mir_interpreter::interpret(&mir).unwrap();
    assert_eq!(output.stdout, stdout.as_bytes());
    assert_eq!(output.stderr, b"");
    assert_eq!(output.exit_status, 0);
    assert!(output.runtime_diagnostic.is_none());
    for profile in profiles() {
        native_execution::assert_native_execution(&mir, profile, stdout);
    }
    let Some(output) = execute_php(
        "bytes-allocate.doria",
        source,
        "__DoriaFunction_6d61696e();",
    ) else {
        return;
    };
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(output.stdout, stdout.as_bytes());
}

fn execute_php(path: &str, source: &str, entry: &str) -> Option<std::process::Output> {
    let php = doriac::compile_source_to_php(path, source)
        .unwrap_or_else(|diagnostics| panic!("{source}\n{diagnostics:#?}"));
    let Ok(version) = std::process::Command::new("php").arg("--version").output() else {
        eprintln!("PHP unavailable; byte allocation compatibility execution skipped");
        return None;
    };
    assert!(version.status.success());
    let script = format!("{}\n{entry}", php.strip_prefix("<?php").unwrap());
    Some(
        php_execution::Script::new(&script)
            .command(&["-d", "memory_limit=16M"])
            .output()
            .unwrap(),
    )
}

fn assert_error(source: &str, code: &str) {
    let (_, analysis) = doriac::analyze_source_for_ide("bytes-allocate.doria", source)
        .unwrap_or_else(|diagnostics| panic!("must parse: {source}\n{diagnostics:#?}"));
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == code),
        "expected {code}: {source}\n{:#?}",
        analysis.diagnostics
    );
}

fn assert_php_panic(path: &str, source: &str, stdout: &[u8], code: &str) -> Option<String> {
    let output = execute_php(path, source, "__DoriaFunction_6d61696e();")?;
    assert_eq!(output.status.code(), Some(101), "{output:?}");
    assert_eq!(output.stdout, stdout);
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains(&format!("Panic[{code}]")), "{stderr}");
    assert!(
        stderr.contains(path),
        "panic must retain its source: {stderr}"
    );
    assert!(
        !stderr.contains("__doria_") && !stderr.contains("__Doria"),
        "compiler helper leaked: {stderr}"
    );
    Some(stderr)
}

fn expected_php_main_panic(path: &str, source: &str, selected: &str, code: &str) -> String {
    let start = source.rfind(selected).unwrap();
    let span = doriac::source::Span::new(start, start + selected.len());
    let diagnostic = doriac::diagnostics::Diagnostic::runtime_panic(
        doria_diagnostic_catalogue::runtime_entry(code)
            .unwrap()
            .code,
        span,
        doriac::diagnostics::RuntimeOutcomeDetails {
            process_status: 101,
            termination_behavior: doriac::diagnostics::TerminationBehavior::AbortWithoutCleanup,
            origin: doriac::diagnostics::RuntimeOutcomeOrigin {
                source: doriac::diagnostics::DiagnosticSource::Current,
                span,
                function: Some("main".to_string()),
            },
            path: vec![doriac::diagnostics::RuntimeOutcomeFrame {
                function: "main".to_string(),
                source: doriac::diagnostics::DiagnosticSource::Current,
                span,
            }],
            facts: Vec::new(),
            error_type: None,
        },
    );
    doriac::render_diagnostics_with_options(
        path,
        source,
        &[diagnostic],
        doriac::diagnostics::RenderOptions {
            color: doriac::diagnostics::ColorChoice::Never,
            context_lines: 0,
            ..doriac::diagnostics::RenderOptions::default()
        },
    ) + "\n"
}

#[test]
fn allocated_bytes_are_zero_initialized_independent_mutable_and_move_owned() {
    assert_execution(
        include_str!("../../../examples/native/main_bytes_allocate.doria"),
        include_str!("fixtures/native_io/main_bytes_allocate/expected_stdout"),
    );
}

#[test]
fn allocation_evaluates_checked_length_once_and_preserves_normal_and_failure_cleanup() {
    assert_execution(
        include_str!("../../../examples/native/main_bytes_allocate_checked.doria"),
        include_str!("fixtures/native_io/main_bytes_allocate_checked/expected_stdout"),
    );
}

#[test]
fn allocate_accepts_named_length_and_rejects_wrong_arity_type_or_name() {
    doriac::check_source(
        "bytes-allocate.doria",
        "function main(): void { let $bytes = Bytes::allocate(length: 0); }",
    )
    .expect("canonical length parameter supports named arguments");
    for (expression, code) in [
        ("Bytes::allocate()", "E0409"),
        ("Bytes::allocate(1, 2)", "E0409"),
        ("Bytes::allocate(true)", "E0408"),
        ("Bytes::allocate(1.0)", "E0408"),
        ("Bytes::allocate(\"4\")", "E0408"),
        ("Bytes::allocate(size: 4)", "E0516"),
        ("Bytes::fromArray(source: [0])", "E0519"),
    ] {
        assert_error(
            &format!("function main(): void {{ let $bytes = {expression}; }}"),
            code,
        );
    }
    assert_error(
        "function wrong(uint32 $length): Bytes { return Bytes::allocate($length); }",
        "E0408",
    );
}

#[test]
fn allocation_does_not_change_readonly_access_or_use_after_move_rules() {
    assert_error(
        "function main(): void { let $bytes = Bytes::allocate(1); $bytes[0] = 7; }",
        "E0201",
    );
    assert_error("function takeBytes(take Bytes $bytes): void {} function main(): void { let $bytes = Bytes::allocate(1); takeBytes($bytes); echo $bytes->length; }", "E0470");
}

#[test]
fn negative_length_is_a_signed_catalogued_panic_and_runs_no_cleanup() {
    let mir = lower(NEGATIVE_PATH, NEGATIVE);
    let output = doriac::mir_interpreter::interpret(&mir).unwrap();
    assert_eq!(output.stdout, b"before\n");
    assert_eq!(output.exit_status, 101);
    assert_eq!(
        output.stderr,
        include_bytes!("fixtures/native_io/main_bytes_allocate_negative/expected_stderr")
    );
    let diagnostic = output
        .runtime_diagnostic
        .as_ref()
        .expect("structured panic");
    assert_eq!(diagnostic.code, "P1303");
    assert_eq!(diagnostic.span.start, NEGATIVE.find("-7").unwrap());
    assert_eq!(diagnostic.span.end, NEGATIVE.find("-7").unwrap() + 2);
    let facts = &diagnostic.runtime_outcome.as_ref().unwrap().facts;
    assert!(
        facts.iter().any(
            |fact| fact.name == doria_diagnostic_catalogue::BYTE_BUFFER_LENGTH_FACT
                && fact.value == RuntimeFactValue::Signed(-7)
        ),
        "{facts:?}"
    );
    for profile in profiles() {
        let native = native_execution::execute_native(&mir, profile);
        assert_eq!(native.status.code(), Some(101), "{profile:?}: {native:?}");
        assert_eq!(
            native.stdout, output.stdout,
            "panic must not unwind: {profile:?}"
        );
        assert_eq!(native.stderr, output.stderr, "{profile:?}");
    }
    if let Some(stderr) = assert_php_panic(NEGATIVE_PATH, NEGATIVE, b"before\n", "P1303") {
        assert_eq!(stderr.as_bytes(), output.stderr);
    }
}

#[test]
fn negative_length_after_nested_evaluation_points_back_to_the_length_argument() {
    let source = r#"
function requested(): int { echo "length once\n"; return -7; }
function main(): void { let $bytes = Bytes::allocate(requested()); }
"#;
    let mir = lower("bytes-allocate-length.doria", source);
    let output = doriac::mir_interpreter::interpret(&mir).unwrap();
    assert_eq!(output.stdout, b"length once\n");
    assert_eq!(output.exit_status, 101);
    let diagnostic = output.runtime_diagnostic.as_ref().unwrap();
    assert_eq!(diagnostic.code, "P1303");
    let start = source.rfind("requested()").unwrap();
    assert_eq!(diagnostic.span.start, start);
    assert_eq!(diagnostic.span.end, start + "requested()".len());
    for profile in profiles() {
        let native = native_execution::execute_native(&mir, profile);
        assert_eq!(native.status.code(), Some(101), "{profile:?}: {native:?}");
        assert_eq!(native.stdout, output.stdout);
        assert_eq!(native.stderr, output.stderr, "{profile:?}");
    }
    if let Some(stderr) = assert_php_panic(
        "bytes-allocate-length.doria",
        source,
        b"length once\n",
        "P1303",
    ) {
        assert_eq!(stderr.as_bytes(), output.stderr);
    }
}

#[test]
fn php_capacity_rejection_is_catalogued_without_exhausting_host_memory() {
    let source = r#"
class Marker {
    function __destruct() {
        try { echo "unexpected cleanup\n"; } catch (Doria\Std\Io\IoError) {}
    }
}
function main(): void {
    let $marker = new Marker();
    echo "before\n";
    let $bytes = Bytes::allocate(9223372036854775807);
    echo "unexpected continuation\n";
}
"#;
    // PHP rejects this count before allocating; this does not simulate host OOM.
    if let Some(stderr) = assert_php_panic(
        "bytes-allocate-capacity.doria",
        source,
        b"before\n",
        "P1302",
    ) {
        assert_eq!(
            stderr,
            expected_php_main_panic(
                "bytes-allocate-capacity.doria",
                source,
                "9223372036854775807",
                "P1302",
            )
        );
    }
}

fn allocation_mut(mir: &mut doriac::mir::Program) -> &mut CollectionExpression {
    for function in &mut mir.functions {
        for block in &mut function.blocks {
            for statement in &mut block.statements {
                if let Statement::AssignLocal {
                    value: Rvalue::Collection(value @ CollectionExpression::BytesAllocate { .. }),
                    ..
                } = statement
                {
                    return value;
                }
            }
        }
    }
    panic!("allocation operation not found");
}

#[test]
fn allocation_lowers_directly_without_typed_array_or_copy_staging() {
    let mut mir = lower("bytes-allocate.doria", SIMPLE);
    assert!(matches!(
        allocation_mut(&mut mir),
        CollectionExpression::BytesAllocate { .. }
    ));
    let mut allocations = 0;
    for function in &mir.functions {
        for block in &function.blocks {
            for statement in &block.statements {
                if let Statement::AssignLocal {
                    value: Rvalue::Collection(value),
                    ..
                } = statement
                {
                    match value {
                        CollectionExpression::BytesAllocate { collection, .. } => {
                            assert_eq!(
                                mir.collection_types[collection.0].kind,
                                CollectionKind::Bytes
                            );
                            allocations += 1;
                        }
                        CollectionExpression::Local { .. } => {}
                        _ => panic!("allocation unexpectedly stages another collection: {value:?}"),
                    }
                }
            }
        }
    }
    assert_eq!(allocations, 1, "expected one executed allocation: {mir:#?}");

    // Registered standard collection types are not executed allocation steps.
    // Code relocations prove which runtime operations are referenced, not how
    // many calls execute: one AArch64 PIC address load has page/offset relocation
    // pairs, and a loaded address may serve multiple calls. The MIR above owns
    // the exact operation-count assertion; do not count linker records as calls.
    let bytes = doriac::codegen_cranelift::lower_mir_to_object(&mir).unwrap();
    let object = cranelift_object::object::File::parse(bytes.as_slice()).unwrap();
    let mut references = Vec::new();
    let mut locations = Vec::new();
    for section in object
        .sections()
        .filter(|section| section.kind() == SectionKind::Text)
    {
        for (offset, relocation) in section.relocations() {
            if let RelocationTarget::Symbol(symbol) = relocation.target() {
                let symbol = object.symbol_by_index(symbol).unwrap();
                let name = symbol.name().unwrap().trim_start_matches('_').to_owned();
                locations.push(format!(
                    "{}+{offset:#x} {:?}: {name}",
                    section.name().unwrap(),
                    relocation.flags(),
                ));
                references.push(name);
            }
        }
    }
    assert_direct_allocation_runtime_references(
        references.iter().map(String::as_str),
        &format!(
            "Cranelift {:?} {:?} relocations: {locations:#?}",
            object.format(),
            object.architecture(),
        ),
    );

    #[cfg(feature = "llvm-backend")]
    {
        let ir = doriac::codegen_llvm::lower_mir_to_llvm_ir(&mir).unwrap();
        let calls = ir
            .lines()
            .filter(|line| line.contains("call "))
            .filter_map(|line| {
                line.split_once('@')?
                    .1
                    .split_once('(')
                    .map(|(name, _)| name)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            calls
                .iter()
                .filter(|name| **name == doriac::native_abi::BYTES_ALLOCATE)
                .count(),
            1,
            "expected one LLVM allocation call instruction: {ir}",
        );
        assert_direct_allocation_runtime_references(calls.into_iter(), &ir);
    }
}

fn assert_direct_allocation_runtime_references<'a>(
    references: impl Iterator<Item = &'a str>,
    context: &str,
) {
    use doriac::native_abi::{
        BYTES_ALLOCATE, BYTES_COPY, BYTES_FROM_COLLECTION, COLLECTION_NEW, CORE_COLLECTION_NEW,
    };
    let references = references.collect::<std::collections::BTreeSet<_>>();
    assert!(
        references.contains(BYTES_ALLOCATE),
        "direct allocation runtime is absent: {references:?}\n{context}",
    );
    for forbidden in [
        BYTES_COPY,
        BYTES_FROM_COLLECTION,
        COLLECTION_NEW,
        CORE_COLLECTION_NEW,
    ] {
        assert!(
            !references.contains(forbidden),
            "allocation staged through {forbidden}: {references:?}\n{context}"
        );
    }
}

fn assert_malformed(mir: &doriac::mir::Program, message: &str) {
    let error =
        doriac::mir_validation::validate_program(mir).expect_err("malformed allocation must fail");
    assert!(error.message.contains(message), "{error:?}");
    assert!(doriac::codegen_cranelift::lower_mir_to_object(mir).is_err());
    #[cfg(feature = "llvm-backend")]
    assert!(doriac::codegen_llvm::lower_mir_to_object(mir).is_err());
}

#[test]
fn malformed_allocation_rejects_non_bytes_destination() {
    let mut mir = lower("bytes-allocate.doria", SIMPLE);
    let CollectionExpression::BytesAllocate { collection, .. } = allocation_mut(&mut mir) else {
        unreachable!()
    };
    let collection = *collection;
    mir.collection_types[collection.0].kind = CollectionKind::TypedArray;
    assert_malformed(&mir, "Bytes::allocate destination is not Bytes");
}

#[test]
fn malformed_allocation_rejects_noncanonical_integer_width() {
    let mut mir = lower("bytes-allocate.doria", SIMPLE);
    let CollectionExpression::BytesAllocate { length, .. } = allocation_mut(&mut mir) else {
        unreachable!()
    };
    **length = IntegerExpression::constant(IntegerValue::from_bits(IntegerType::UInt32, 4));
    assert_malformed(&mir, "Bytes::allocate length is not int");
}

#[test]
fn php_bytes_indexing_and_length_preserve_writable_parameter_borrows() {
    assert_execution(
        r#"
function change(writable Bytes $bytes): void {
    $bytes[0] = 128;
    $bytes[2] = 255;
}
function first(Bytes $bytes): uint8 { return $bytes[0]; }
function main(): void {
    writable Bytes $bytes = Bytes::allocate(3);
    echo "before {$bytes->length} {$bytes[0]} {$bytes[1]} {$bytes[2]}\n";
    change($bytes);
    echo "after {$bytes->length} {first($bytes)} {$bytes[1]} {$bytes[2]}\n";
}
"#,
        "before 3 0 0 0\nafter 3 128 0 255\n",
    );
}

#[test]
fn php_bytes_explicit_array_copies_remain_independent_and_compare_by_value() {
    assert_execution(
        r#"
function main(): void {
    writable uint8[] $source = [0, 128, 255];
    writable Bytes $bytes = Bytes::fromArray($source);
    writable uint8[] $exported = $bytes->toArray();
    writable Bytes $copy = Bytes::fromArray($exported);
    echo "equal {$bytes == $copy}\n";
    $source[0] = 7;
    $exported[1] = 42;
    $copy[2] = 9;
    echo "source {$source[0]} {$source[1]} {$source[2]}\n";
    echo "bytes {$bytes->length} {$bytes[0]} {$bytes[1]} {$bytes[2]}\n";
    echo "exported {$exported[0]} {$exported[1]} {$exported[2]}\n";
    echo "copy {$copy->length} {$copy[0]} {$copy[1]} {$copy[2]}\n";
    echo "different {$bytes != $copy}\n";
}
"#,
        "equal true\nsource 7 128 255\nbytes 3 0 128 255\nexported 0 42 255\ncopy 3 0 128 9\ndifferent true\n",
    );
}

#[test]
fn php_bytes_read_and_write_bounds_panics_preserve_facts_location_and_abort_cleanup() {
    const PATH: &str = "bytes-index-bounds.doria";
    for (statement, index) in [
        ("let $value = $bytes[-1];", -1),
        ("let $value = $bytes[2];", 2),
        ("$bytes[-1] = 255;", -1),
        ("$bytes[2] = 255;", 2),
    ] {
        let source = format!(
            r#"class Marker {{
    function __destruct() {{
        try {{ echo "unexpected cleanup\n"; }} catch (Doria\Std\Io\IoError) {{}}
    }}
}}
function main(): void {{
    let $marker = new Marker();
    writable Bytes $bytes = Bytes::allocate(2);
    echo "before\n";
    {statement}
    echo "unexpected continuation\n";
}}
"#
        );
        let selected = format!("$bytes[{index}]");
        let mir = lower(PATH, &source);
        let output = doriac::mir_interpreter::interpret(&mir).unwrap();
        assert_eq!(output.exit_status, 101);
        assert_eq!(output.stdout, b"before\n");
        let diagnostic = output.runtime_diagnostic.as_ref().unwrap();
        assert_eq!(diagnostic.code, "P1301");
        let reported_source = source
            .get(diagnostic.span.start..diagnostic.span.end)
            .unwrap_or_else(|| {
                panic!(
                    "bounds panic has an invalid source range for `{statement}`: \
                     {diagnostic:#?}\nsource:\n{source}"
                )
            });
        // A preceding I/O operation or nested operand evaluation must not own
        // this access's bounds panic. Preserve this guard if attribution fails;
        // accepting the previous active panic site would mask a compiler bug.
        assert!(
            reported_source.contains(&selected),
            "bounds panic for `{statement}` must identify `{selected}`, \
             but identified {reported_source:?} at {:?}\n{diagnostic:#?}\n{}",
            diagnostic.span,
            String::from_utf8_lossy(&output.stderr),
        );
        let facts = &diagnostic.runtime_outcome.as_ref().unwrap().facts;
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0].name, doria_diagnostic_catalogue::INDEX_FACT);
        assert_eq!(facts[0].value, RuntimeFactValue::Signed(index));
        assert_eq!(
            facts[1].name,
            doria_diagnostic_catalogue::INDEXED_LENGTH_FACT
        );
        assert_eq!(facts[1].value, RuntimeFactValue::Unsigned(2));
        for profile in profiles() {
            let native = native_execution::execute_native(&mir, profile);
            assert_eq!(native.status.code(), Some(101), "{profile:?}: {native:?}");
            assert_eq!(native.stdout, output.stdout);
            assert_eq!(native.stderr, output.stderr, "{profile:?}");
        }
        if let Some(stderr) = assert_php_panic(PATH, &source, b"before\n", "P1301") {
            assert_eq!(stderr.as_bytes(), output.stderr);
        }
    }
}

#[test]
fn php_generated_runtime_catalogue_contains_every_canonical_entry() {
    let Some(output) = execute_php(
        "bytes-runtime-catalogue.doria",
        SIMPLE,
        "echo json_encode($__doria_catalogue, JSON_THROW_ON_ERROR);",
    ) else {
        return;
    };
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stderr, b"");
    let generated: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        generated.as_object().unwrap().len(),
        doria_diagnostic_catalogue::RUNTIME_CATALOGUE.len()
    );
    for entry in doria_diagnostic_catalogue::RUNTIME_CATALOGUE {
        assert_eq!(
            generated[entry.code],
            serde_json::json!([
                entry.title,
                entry.primary_label,
                entry.explanation,
                entry.fact_names
            ]),
            "{}",
            entry.code
        );
    }
}

#[test]
fn php_bytes_carrier_crosses_owned_properties_nullable_results_and_closure_signatures() {
    assert_execution(
        r#"
class Packet {
    function __construct(take Bytes $contents) {}
    function __destruct() {
        try { echo "drop property {$this->contents->length}\n"; }
        catch (Doria\Std\Io\IoError) {}
    }
}
function maybeBytes(bool $present): ?Bytes {
    if ($present) { return Bytes::allocate(2); }
    return null;
}
function main(): void {
    function(take Bytes): Bytes $identity = fn(take Bytes $input) => $input;
    function(Bytes): int $length = fn(Bytes $input) => $input->length;
    function(bool): ?Bytes $optional = fn(bool $present) => maybeBytes($present);
    let $packet = new Packet($identity(Bytes::allocate(3)));
    echo "property {$length($packet->contents)} {$packet->contents[0]}\n";
    let $present = $optional(true);
    if ($present != null) { echo "present {$length($present)} {$present[1]}\n"; }
    let $absent = $optional(false);
    if ($absent == null) { echo "absent\n"; }
}
"#,
        "property 3 0\npresent 2 0\nabsent\ndrop property 3\n",
    );
}
