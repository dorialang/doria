fn source(reading: bool, preparation: &str, helpers: &str) -> String {
    let argument = if reading { "destination" } else { "source" };
    let preparation = preparation.replace("$bytes", &format!("${argument}"));
    let (capability, file, request, region, outcome, phase, method) = if reading {
        (
            "PendingReadableStream",
            "ReadableFile",
            "ReadRequest",
            "writable WritableByteRegion",
            "ReadOutcome",
            "Read",
            "read",
        )
    } else {
        (
            "PendingWritableStream",
            "WritableFile",
            "WriteRequest",
            "ByteRegion",
            "WriteOutcome",
            "Write",
            "write",
        )
    };
    format!(
        r#"
use Doria\Std\Io\{{PendingReadableStream, PendingWritableStream, ReadableFile, WritableFile,
    ReadRequest, WriteRequest, ReadOutcome, WriteOutcome, IoError, ReadinessSource,
    ByteRegion, WritableByteRegion}};
{helpers}
class Adapter implements {capability} {{
    internal writable {file} $file;
    internal writable Bytes $buffer = Bytes::allocate(8);
    internal writable int $count = 0;
    function __construct(parameter take {file} $file) {{ $this->file = $file; }}
    ReadinessSource $readinessSource {{ borrowed get => $this->file->readinessSource; }}
    writable function {method}({region} ${argument}): {outcome} throws IoError {{ return $this->file->{method}(${argument}); }}
    writable function prepare{phase}(writable {request} $request, {region} ${argument}): void throws IoError {{
        {preparation}
    }}
    writable function complete{phase}({request} $request, {region} ${argument}): ?{outcome} throws IoError {{ return null; }}
}}
function main(): void {{}}
"#
    )
}

fn diagnostics(source: &str) -> Vec<doriac::diagnostics::Diagnostic> {
    doriac::analyze_source_for_ide("selected-storage.doria", source)
        .unwrap()
        .1
        .diagnostics
}

#[test]
fn duplex_preparation_keeps_distinct_requests_on_one_file() {
    let read = "$file->prepareRead($readRequest, $destination);";
    let write = "$file->prepareWrite($writeRequest, $source);";
    for preparation in [format!("{read} {write}"), format!("{write} {read}")] {
        let source = format!(
            r#"
use Doria\Std\Io\{{DuplexFile, ReadRequest, WriteRequest, WritableByteRegion, ByteRegion, IoError}};
function prepare(writable DuplexFile $file, writable ReadRequest $readRequest,
    writable WriteRequest $writeRequest, writable WritableByteRegion $destination,
    ByteRegion $source): void throws IoError {{
    {preparation}
}}
function main(): void {{}}
"#
        );
        let found = diagnostics(&source);
        assert!(found.is_empty(), "{source}\n{found:#?}");
        let program = doriac::lower_source_to_mir("duplex-preparation.doria", &source).unwrap();
        doriac::mir_validation::validate_program(&program).unwrap();
    }
}

#[test]
fn preparation_keeps_group_regions_and_adapter_owned_storage() {
    for reading in [false, true] {
        let phase = if reading { "Read" } else { "Write" };
        let region = if reading {
            "getWritableRegion"
        } else {
            "getRegion"
        };
        for selected in [
            "$bytes".to_string(),
            format!("$this->buffer->{region}(0, 4)"),
        ] {
            let source = source(
                reading,
                &format!("$this->file->prepare{phase}($request, {selected});"),
                "",
            );
            let found = diagnostics(&source);
            assert!(found.is_empty(), "{source}\n{found:#?}");
        }
    }
}

#[test]
fn selected_storage_effects_follow_named_multihop_helpers_and_returned_aliases() {
    for reading in [false, true] {
        let (phase, request, region, file) = if reading {
            (
                "Read",
                "ReadRequest",
                "writable WritableByteRegion",
                "ReadableFile",
            )
        } else {
            ("Write", "WriteRequest", "ByteRegion", "WritableFile")
        };
        let helpers = format!(
            r#"
function select(writable {file} $file, writable {request} $request, {region} $bytes): void throws IoError {{
    $file->prepare{phase}($request, $bytes);
}}
function forward(writable {file} $file, writable {request} $request, {region} $bytes): void throws IoError {{
    select(bytes: alias($bytes), request: $request, file: $file);
}}
function alias({region} $bytes): {return_region} {{ {returned} }}
"#,
            return_region = if reading {
                "WritableByteRegion"
            } else {
                "ByteRegion"
            },
            returned = if reading {
                "let $length = $bytes->length; return $bytes->getWritableRegion(0, $length);"
            } else {
                "return $bytes;"
            }
        );
        let safe = source(reading, "forward($this->file, $request, $bytes);", &helpers);
        assert!(
            diagnostics(&safe).is_empty(),
            "{safe}\n{:#?}",
            diagnostics(&safe)
        );
        let method = if reading {
            "getWritableRegion"
        } else {
            "getRegion"
        };
        let unsafe_source = source(reading,
            &format!("writable Bytes $local = Bytes::allocate(4); forward($this->file, $request, $local->{method}(0, 4));"), &helpers);
        let found = diagnostics(&unsafe_source);
        assert!(
            found.iter().any(|diagnostic| diagnostic.code == "E0773"),
            "{unsafe_source}\n{found:#?}"
        );
    }
}

#[test]
fn selected_storage_follows_erased_and_constrained_dispatch() {
    for reading in [false, true] {
        let (phase, capability, request, region, accessor) = if reading {
            (
                "Read",
                "PendingReadableStream",
                "ReadRequest",
                "writable WritableByteRegion",
                "getWritableRegion",
            )
        } else {
            (
                "Write",
                "PendingWritableStream",
                "WriteRequest",
                "ByteRegion",
                "getRegion",
            )
        };
        for receiver in [capability, "T"] {
            let generics = if receiver == "T" {
                format!("<T implements {capability}>")
            } else {
                String::new()
            };
            let helpers = format!(
                r#"
function forward{generics}(writable {receiver} $stream, writable {request} $request, {region} $bytes): void throws IoError {{
    $stream->prepare{phase}($request, $bytes);
}}
"#
            );
            let safe = source(reading, "forward($this->file, $request, $bytes);", &helpers);
            let found = diagnostics(&safe);
            assert!(found.is_empty(), "{safe}\n{found:#?}");
            let unsafe_source = source(reading, &format!("writable Bytes $local = Bytes::allocate(4); forward($this->file, $request, $local->{accessor}(0, 4));"), &helpers);
            let found = diagnostics(&unsafe_source);
            assert!(
                found.iter().any(|diagnostic| diagnostic.code == "E0773"),
                "{unsafe_source}\n{found:#?}"
            );
        }
    }
}

#[test]
fn selected_storage_follows_direct_and_returned_callbacks_without_joining_callers() {
    for reading in [false, true] {
        let (phase, request, file, region, capture, accessor) = if reading {
            (
                "Read",
                "ReadRequest",
                "ReadableFile",
                "WritableByteRegion",
                "writable $bytes",
                "getWritableRegion",
            )
        } else {
            (
                "Write",
                "WriteRequest",
                "WritableFile",
                "ByteRegion",
                "$bytes",
                "getRegion",
            )
        };
        let mode = if reading { "writable " } else { "" };
        let helpers = format!(
            r#"
function invoke(writable function writable(writable {file}, writable {request}): void throws IoError $callback, writable {file} $file, writable {request} $request): void throws IoError {{
    $callback($file, $request);
}}
function make({mode}{region} $bytes): function writable(writable {file}, writable {request}): void throws IoError {{
    return function(writable {file} $file, writable {request} $request): void with ({capture}) {{
        $file->prepare{phase}($request, $bytes);
    }};
}}
"#
        );
        for factory in [
            format!("function(writable {file} $file, writable {request} $request): void with ({capture}) {{ $file->prepare{phase}($request, $bytes); }}"),
            "make($bytes)".to_owned(),
        ] {
            let preparation = format!("let writable $callback = {factory}; invoke($callback, $this->file, $request);");
            let safe = source(reading, &preparation, &helpers);
            let found = diagnostics(&safe);
            assert!(found.is_empty(), "{safe}\n{found:#?}");
            let local = format!("writable Bytes $local = Bytes::allocate(4); let writable $bytes = $local->{accessor}(0, 4); {preparation}");
            // The fixture's method parameter must remain distinct from this local carrier.
            let local = local.replace("$bytes", "$selected");
            let unsafe_source = source(reading, &local, &helpers);
            let found = diagnostics(&unsafe_source);
            assert!(found.iter().any(|diagnostic| diagnostic.code == "E0773"), "{unsafe_source}\n{found:#?}");
        }
        for invoked in ["safe", "unsafe"] {
            let preparation = format!(
                r#"
let writable $safe = make($bytes);
writable Bytes $local = Bytes::allocate(4);
let writable $selected = $local->{accessor}(0, 4);
let writable $unsafe = make($selected);
invoke(${invoked}, $this->file, $request);
"#
            );
            let source = source(reading, &preparation, &helpers);
            let found = diagnostics(&source);
            if invoked == "safe" {
                assert!(found.is_empty(), "{source}\n{found:#?}");
            } else {
                assert!(
                    found.iter().any(|diagnostic| diagnostic.code == "E0773"),
                    "{source}\n{found:#?}"
                );
            }
        }
    }
}

#[test]
fn descriptor_metadata_does_not_read_selected_bytes() {
    for reading in [false, true] {
        let phase = if reading { "Read" } else { "Write" };
        let selected = format!("$this->file->prepare{phase}($request, $bytes);");
        let metadata_source = source(
            reading,
            &format!("{selected} let $length = $bytes->length;"),
            "",
        );
        let found = diagnostics(&metadata_source);
        assert!(found.is_empty(), "{metadata_source}\n{found:#?}");
        if reading {
            let byte_source = source(reading, &format!("{selected} let $byte = $bytes[0];"), "");
            let found = diagnostics(&byte_source);
            assert!(
                found.iter().any(|diagnostic| diagnostic.code == "E0774"),
                "{byte_source}\n{found:#?}"
            );
        }
    }
}

#[test]
fn selected_owned_fields_do_not_freeze_unrelated_adapter_state() {
    for reading in [false, true] {
        let phase = if reading { "Read" } else { "Write" };
        let accessor = if reading {
            "getWritableRegion"
        } else {
            "getRegion"
        };
        let source = source(reading, &format!("$this->file->prepare{phase}($request, $this->buffer->{accessor}(0, 4)); $this->count += 1;"), "");
        let found = diagnostics(&source);
        assert!(found.is_empty(), "{source}\n{found:#?}");
    }
}

#[test]
fn selection_survives_control_flow_and_caught_failure_and_complete_is_not_retirement() {
    for reading in [false, true] {
        let phase = if reading { "Read" } else { "Write" };
        let accessor = if reading {
            "getWritableRegion"
        } else {
            "getRegion"
        };
        let outcome = if reading {
            "ReadOutcome"
        } else {
            "WriteOutcome"
        };
        for selection in [
            format!("if ($this->count == 0) {{ $this->file->prepare{phase}($request, $this->buffer->{accessor}(0, 4)); }}"),
            format!("try {{ $this->file->prepare{phase}($request, $this->buffer->{accessor}(0, 4)); }} catch (IoError) {{}}"),
            format!("$this->file->prepare{phase}($request, $this->buffer->{accessor}(0, 4)); $request->complete({outcome}::WouldBlock);"),
        ] {
            let source = source(reading, &format!("{selection} $this->buffer = Bytes::allocate(8);"), "");
            let found = diagnostics(&source);
            assert!(found.iter().any(|diagnostic| diagnostic.code == "E0774"), "{source}\n{found:#?}");
        }
    }
}

#[test]
fn temporary_and_helper_owned_storage_cannot_be_selected() {
    for reading in [false, true] {
        let (phase, request, file, region) = if reading {
            ("Read", "ReadRequest", "ReadableFile", "getWritableRegion")
        } else {
            ("Write", "WriteRequest", "WritableFile", "getRegion")
        };
        let helpers = format!(
            r#"
function select(writable {file} $file, writable {request} $request, take Bytes $bytes): void throws IoError {{
    let writable $owned = $bytes;
    $file->prepare{phase}($request, $owned->{region}(0, 4));
}}
"#
        );
        let source = source(
            reading,
            "select($this->file, $request, Bytes::allocate(4));",
            &helpers,
        );
        let found = diagnostics(&source);
        assert!(
            found.iter().any(|diagnostic| diagnostic.code == "E0773"),
            "{source}\n{found:#?}"
        );
    }
}

#[test]
fn pending_selection_does_not_end_at_the_descriptor_last_use() {
    for reading in [false, true] {
        let phase = if reading { "Read" } else { "Write" };
        let region = if reading {
            "getWritableRegion"
        } else {
            "getRegion"
        };
        let preparation = format!(
            r#"
{{ let $selected = $this->buffer->{region}(0, 4); $this->file->prepare{phase}($request, $selected); }}
$this->buffer = Bytes::allocate(8);
"#
        );
        let source = source(reading, &preparation, "");
        let found = diagnostics(&source);
        assert!(
            found.iter().any(|diagnostic| diagnostic.code == "E0774"),
            "{source}\n{found:#?}"
        );
    }
}

#[test]
fn requirement_calls_select_storage_without_any_implementation_in_the_program() {
    // Programs include only the std declarations they reach (decision log
    // entry 41), so these reach the pending interfaces and none of their
    // implementations. The requirement's own contract still selects storage.
    for (capability, request, region, phase, outcome) in [
        (
            "PendingReadableStream",
            "ReadRequest",
            "writable WritableByteRegion",
            "Read",
            "ReadOutcome",
        ),
        (
            "PendingWritableStream",
            "WriteRequest",
            "ByteRegion",
            "Write",
            "WriteOutcome",
        ),
    ] {
        for generic in [false, true] {
            let (parameters, receiver) = if generic {
                (format!("<T implements {capability}>"), "T")
            } else {
                (String::new(), capability)
            };
            let source = format!(
                "use Doria\\Std\\Io\\{{{capability}, {request}, ByteRegion, WritableByteRegion, {outcome}, IoError}};\n\
                 function inspect{parameters}(writable {receiver} $stream, writable {request} $request, {region} $region): ?{outcome} throws IoError {{\n\
                 $stream->prepare{phase}($request, $region);\n\
                 return $stream->complete{phase}($request, $region);\n}}\nfunction main(): void {{}}\n"
            );
            let found = diagnostics(&source);
            if phase == "Read" {
                assert!(
                    found.iter().any(|diagnostic| diagnostic.code == "E0774"),
                    "{source}\n{found:#?}"
                );
            } else {
                // A readonly source region can be reused before retirement.
                assert!(
                    found.iter().all(|diagnostic| diagnostic.code != "E0774"),
                    "{source}\n{found:#?}"
                );
            }
        }
    }
}
