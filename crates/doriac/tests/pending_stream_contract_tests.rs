use doriac::semantics::SemanticAnalysis;

fn analyze(source: &str) -> SemanticAnalysis {
    doriac::analyze_source_for_ide("pending-stream.doria", source)
        .unwrap_or_else(|diagnostics| panic!("{source}\n{diagnostics:#?}"))
        .1
}

fn adapter(reading: bool, preparation: &str, completion: &str, helpers: &str) -> String {
    format!(
        "{IMPORTS}\n{helpers}\n{}",
        adapter_declaration(reading, preparation, completion)
    )
}

const IMPORTS: &str = r#"
use Doria\Std\Io\{PendingReadableStream, ReadableFile, ReadRequest, ReadOutcome,
    PendingWritableStream, WritableFile, WriteRequest, WriteOutcome,
    ReadinessSource, WritableByteRegion, ByteRegion, IoError};
"#;

fn adapter_declaration(reading: bool, preparation: &str, completion: &str) -> String {
    let (capability, file, request, outcome, region, method, phase) = if reading {
        (
            "PendingReadableStream",
            "ReadableFile",
            "ReadRequest",
            "ReadOutcome",
            "writable WritableByteRegion",
            "read",
            "Read",
        )
    } else {
        (
            "PendingWritableStream",
            "WritableFile",
            "WriteRequest",
            "WriteOutcome",
            "ByteRegion",
            "write",
            "Write",
        )
    };
    let argument = if reading { "destination" } else { "source" };
    format!(
        r#"
class Adapter implements {capability} {{
    internal writable {file} $source;
    function __construct(parameter take {file} $source) {{ $this->source = $source; }}
    ReadinessSource $readinessSource {{ borrowed get => $this->source->readinessSource; }}
    writable function {method}({region} ${argument}): {outcome} throws IoError {{
        return $this->source->{method}(${argument});
    }}
    writable function prepare{phase}(writable {request} $request,
        {region} ${argument}): void throws IoError {{ {preparation} }}
    writable function complete{phase}({request} $request,
        {region} ${argument}): ?{outcome} throws IoError {{
        {completion}
        return {outcome}::WouldBlock;
    }}
}}
"#
    )
}

#[test]
fn pending_capabilities_use_nominal_requirements_without_restricting_synchronous_reads() {
    for reading in [false, true] {
        let source = adapter(reading, "", "", "");
        let analysis = analyze(&source);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:#?}",
            analysis.diagnostics
        );
        let conformance = analysis
            .info
            .contracts
            .conformances
            .iter()
            .find(|conformance| {
                conformance.interface.name
                    == if reading {
                        doriac::compiler_known_stream::PENDING_READABLE_STREAM
                    } else {
                        doriac::compiler_known_stream::PENDING_WRITABLE_STREAM
                    }
            })
            .unwrap();
        assert!(conformance
            .implementations
            .iter()
            .all(|member| { member.implementation.is_some() && member.failures.is_empty() }));
        // Recognition is a semantic checkpoint, not a claim that a group can yet
        // create scoped requests. Unused declarations must still lower normally.
        doriac::lower_source_to_mir(
            "pending-stream.doria",
            format!("{source}\nfunction main(): void {{}}"),
        )
        .unwrap();
    }
}

fn request_mir() -> doriac::mir::Program {
    doriac::lower_source_to_mir(
        "request-mir.doria",
        format!(
            "{IMPORTS}\n{}\n{}\nfunction main(): void {{}}",
            adapter_declaration(true, "", "").replace("class Adapter", "class ReadAdapter"),
            adapter_declaration(false, "", "").replace("class Adapter", "class WriteAdapter"),
        ),
    )
    .unwrap()
}

#[test]
fn pending_file_and_request_intrinsics_lower_without_native_layout_knowledge() {
    use doriac::mir::{CheckedIoOperation, Statement, Terminator};
    let program = request_mir();
    doriac::mir_validation::validate_program(&program).unwrap();
    for name in ["prepareRead", "prepareWrite", "complete"] {
        assert!(
            program
                .functions
                .iter()
                .any(|function| function.blocks.iter().any(|block| {
                    function.name.ends_with(&format!("::{name}"))
                        && block.statements.iter().any(|statement| {
                            matches!(statement, Statement::CompletePendingRequest { .. })
                        })
                        || function.name.ends_with(&format!("::{name}"))
                            && matches!(
                                block.terminator,
                                Terminator::CheckedIo {
                                    operation: CheckedIoOperation::PreparePendingFile { .. },
                                    ..
                                }
                            )
                })),
            "missing typed {name} intrinsic"
        );
    }
    assert!(program
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .any(|block| matches!(
            block.terminator,
            Terminator::CheckedIo {
                operation: CheckedIoOperation::PendingRequestOutcome { .. },
                ..
            }
        )));
    doriac::codegen_cranelift::lower_mir_to_object(&program).unwrap();
    #[cfg(feature = "llvm-backend")]
    doriac::codegen_llvm::lower_mir_to_object(&program).unwrap();
}

#[test]
fn pending_request_mir_rejects_ownership_mutability_and_direction_forgery() {
    use doriac::mir::{CheckedIoOperation, Terminator, Type};
    let original = request_mir();
    let (function, request) = original
        .functions
        .iter()
        .find_map(|function| {
            function.blocks.iter().find_map(|block| {
                if let Terminator::CheckedIo {
                    operation: CheckedIoOperation::PreparePendingFile { request, .. },
                    ..
                } = &block.terminator
                {
                    Some((function.id, *request))
                } else {
                    None
                }
            })
        })
        .expect("fixture lowers a pending file preparation");
    for own in [false, true] {
        let mut forged = original.clone();
        let local = &mut forged.functions[function.0].locals[request.0];
        if own {
            local.owned = true;
        } else {
            local.writable = false;
        }
        let error = doriac::mir_validation::validate_program(&forged).unwrap_err();
        let expected = if own {
            "call-scoped pending request cannot enter owning or escaping storage"
        } else {
            "call-scoped borrow mode"
        };
        assert!(error.message.contains(expected), "{error:?}");
    }
    let mut forged = original.clone();
    let local = &mut forged.functions[function.0].locals[request.0];
    let Type::Class(class) = local.ty else {
        unreachable!()
    };
    let replacement = forged
        .classes
        .iter()
        .find(|candidate| {
            candidate.name
                == if forged.classes[class.0].name == doriac::compiler_known_stream::READ_REQUEST {
                    doriac::compiler_known_stream::WRITE_REQUEST
                } else {
                    doriac::compiler_known_stream::READ_REQUEST
                }
        })
        .unwrap()
        .id;
    local.ty = Type::Class(replacement);
    assert!(doriac::mir_validation::validate_program(&forged).is_err());
}

#[test]
fn pending_completion_accepts_absence_but_immediate_completion_requires_an_outcome() {
    for (reading, request, outcome) in [
        (true, "ReadRequest", "ReadOutcome"),
        (false, "WriteRequest", "WriteOutcome"),
    ] {
        let source = adapter(reading, "", "", "")
            .replace(&format!("return {outcome}::WouldBlock;"), "return null;");
        let analysis = analyze(&source);
        assert!(
            analysis.diagnostics.is_empty(),
            "{source}\n{:#?}",
            analysis.diagnostics
        );
        doriac::lower_source_to_mir(
            "nullable-completion.doria",
            format!("{source}\nfunction main(): void {{}}"),
        )
        .unwrap();

        let source = format!(
            "use Doria\\Std\\Io\\{{{request}, IoError}};\nfunction prepare(writable {request} $request): void throws IoError {{ $request->complete(null); }}"
        );
        let analysis = analyze(&source);
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.starts_with('E')),
            "{source}\n{:#?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn pending_metadata_preserves_nullable_results_without_changing_synchronous_transfers() {
    use doriac::source::Span;
    use doriac::types::ResolvedType;
    for (reading, request, outcome, capability, region, phase, method, argument) in [
        (
            true,
            "ReadRequest",
            "ReadOutcome",
            "PendingReadableStream",
            "writable WritableByteRegion",
            "Read",
            "read",
            "destination",
        ),
        (
            false,
            "WriteRequest",
            "WriteOutcome",
            "PendingWritableStream",
            "ByteRegion",
            "Write",
            "write",
            "source",
        ),
    ] {
        for receiver in ["Adapter", capability, "T"] {
            let generics = if receiver == "T" {
                format!("<T implements {capability}>")
            } else {
                String::new()
            };
            let helpers = format!(
                "function inspect{generics}(writable {receiver} $adapter, {request} $request, {region} ${argument}): void throws IoError {{\nlet $pending = $request->outcome;\nlet $completed = $adapter->complete{phase}($request, ${argument});\nlet $ordinary = $adapter->{method}(${argument});\n}}"
            );
            let source = adapter(reading, "", "", &helpers);
            let analysis = analyze(&source);
            assert!(
                analysis.diagnostics.is_empty(),
                "{source}\n{:#?}",
                analysis.diagnostics
            );
            for (expression, nullable) in [
                ("$request->outcome".to_owned(), true),
                (
                    format!("$adapter->complete{phase}($request, ${argument})"),
                    true,
                ),
                (format!("$adapter->{method}(${argument})"), false),
            ] {
                let start = source.find(&expression).unwrap();
                let ty = analysis
                    .info
                    .expression_type(Span::new(start, start + expression.len()))
                    .unwrap();
                let ty = if nullable {
                    let ResolvedType::Nullable(inner) = ty else {
                        panic!("{expression}: {ty:?}");
                    };
                    inner.as_ref()
                } else {
                    ty
                };
                assert!(
                    matches!(ty, ResolvedType::Enum(ty) if ty.name == format!("Doria\\Std\\Io\\{outcome}")),
                    "{expression}: {ty:?}"
                );
            }
        }
    }
}

#[test]
fn preparation_and_completion_share_transitive_nonblocking_analysis() {
    let cases = [
        (r#"echo "blocking";"#, ""),
        (
            "observe();",
            "use Doria\\Std\\Io\\{TransferGroup, Wait}; function group(): TransferGroup { panic(\"unreachable\"); } function observe(): void throws IoError { let writable $owner = group(); $owner->getNextCompletion(Wait::Immediate); }",
        ),
        (
            "helper();",
            "function helper(): void throws IoError { read_line(); }",
        ),
        ("try { read_line(); } catch (IoError) {}", ""),
        ("let $callback = fn() => read_line(); $callback();", ""),
        (
            "let $marker = new Marker();",
            r#"class Marker {
            function __destruct() { try { echo "cleanup"; } catch (IoError) {} }
        }"#,
        ),
    ];
    for reading in [false, true] {
        for (body, helpers) in cases {
            for completion in [false, true] {
                let source = if completion {
                    adapter(reading, "", body, helpers)
                } else {
                    adapter(reading, body, "", helpers)
                };
                let analysis = analyze(&source);
                assert!(
                    analysis
                        .diagnostics
                        .iter()
                        .any(|diagnostic| diagnostic.code == "E0771"),
                    "{source}\n{:#?}",
                    analysis.diagnostics
                );
                assert!(
                    analysis
                        .diagnostics
                        .iter()
                        .all(|diagnostic| diagnostic.code == "E0771"),
                    "{source}\n{:#?}",
                    analysis.diagnostics
                );
            }
        }
    }
}

#[test]
fn similarly_named_ordinary_methods_do_not_acquire_a_pending_contract() {
    let analysis = analyze(
        r#"
use Doria\Std\Io\IoError;
class Ordinary {
    writable function prepareRead(): void throws IoError { read_line(); }
    writable function completeRead(): void throws IoError { read_line(); }
}
"#,
    );
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn request_members_are_accepted_after_removing_the_temporary_execution_boundary() {
    for (request, outcome, body) in [
        (
            "ReadRequest",
            "ReadOutcome",
            "$request->complete(ReadOutcome::WouldBlock);",
        ),
        (
            "WriteRequest",
            "WriteOutcome",
            "$request->complete(WriteOutcome::WouldBlock);",
        ),
        (
            "ReadRequest",
            "ReadOutcome",
            "let $outcome = $request->outcome;",
        ),
        (
            "WriteRequest",
            "WriteOutcome",
            "let $outcome = $request->outcome;",
        ),
    ] {
        let source = format!(
            r#"
use Doria\Std\Io\{{{request}, {outcome}, IoError}};
function inspect(writable {request} $request): void throws IoError {{ {body} }}
"#
        );
        let analysis = analyze(&source);
        assert!(
            analysis.diagnostics.is_empty(),
            "{source}\n{:#?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn pending_calls_are_accepted_through_erasure_and_constraints_after_boundary_removal() {
    for (reading, capability, request, region, phase) in [
        (
            true,
            "PendingReadableStream",
            "ReadRequest",
            "writable WritableByteRegion",
            "Read",
        ),
        (
            false,
            "PendingWritableStream",
            "WriteRequest",
            "ByteRegion",
            "Write",
        ),
    ] {
        for receiver in ["Adapter", capability, "T"] {
            let generics = if receiver == "T" {
                format!("<T implements {capability}>")
            } else {
                String::new()
            };
            let source = adapter(
                reading,
                "",
                "",
                &format!(
                    r#"
function inspect{generics}(writable {receiver} $reader,
    writable {request} $request, {region} $bytes, bool $preparing): void throws IoError {{
    if ($preparing) {{
        $reader->prepare{phase}($request, $bytes);
    }} else {{
        $reader->complete{phase}($request, $bytes);
    }}
}}
"#
                ),
            );
            let analysis = analyze(&source);
            assert!(
                analysis.diagnostics.is_empty(),
                "{source}\n{:#?}",
                analysis.diagnostics
            );
        }
    }
}

#[test]
fn requests_cannot_be_constructed_or_consumed_from_borrowed_parameters() {
    for request in ["ReadRequest", "WriteRequest"] {
        for body in [
            format!("function main(): void {{ let $request = new {request}(); }}"),
            format!(
                "function consume(take {request} $request): void {{}}\nfunction inspect({request} $request): void {{ consume($request); }}"
            ),
        ] {
            let source = format!("use Doria\\Std\\Io\\{request}; {body}");
            let analysis = analyze(&source);
            assert!(!analysis.diagnostics.is_empty(), "{source}");
            assert!(
                analysis
                    .diagnostics
                    .iter()
                    .all(|diagnostic| diagnostic.code.starts_with('E')),
                "{source}\n{:#?}",
                analysis.diagnostics
            );
        }
    }
}

#[test]
fn call_scoped_requests_cannot_enter_owned_or_returned_storage() {
    for request in ["ReadRequest", "WriteRequest"] {
        for body in [
            format!("function consume(take {request} $request): void {{}}"),
            format!("class Holder {{ ?{request} $saved = null; }}"),
            format!("trait Storage {{ ?{request} $saved = null; }}"),
            format!("interface Owning {{ function consume(take {request} $request): void; }}"),
            format!("interface Factory {{ function create(): {request}; }}"),
            format!("interface Returning {{ {request} $request {{ get; }} }}"),
            format!("enum Saved {{ case Request({request} $request); }}"),
            format!("function collect({request} $request): void {{ let $requests = [$request]; }}"),
            format!("function collect(List<{request}> $requests): void {{}}"),
            format!(
                "class Holder<T> {{}} function collect(Holder<{request}> $requests): void {{}}"
            ),
            format!("function erase({request} $request): mixed {{ return $request; }}"),
            format!(
                "function capture({request} $request): function(): void {{ return function(): void with ($request) {{}}; }}"
            ),
            format!(
                "function inspect({request} $request): void {{}} function capture({request} $request): void {{ let $callback = function(): void with (take $request) {{ inspect($request); }}; }}"
            ),
            format!("function inspect(function(take {request}): void $callback): void {{}}"),
            format!(
                "function consume<T>(take T $value): void {{}} function inspect({request} $request): void {{ consume($request); }}"
            ),
            format!(
                "function make<T>(T $value): function(): void {{ return function(): void with ($value) {{}}; }} function escape({request} $request): function(): void {{ return make($request); }}"
            ),
        ] {
            let source = format!("use Doria\\Std\\Io\\{request}; {body}");
            let analysis = analyze(&source);
            assert!(
                analysis
                    .diagnostics
                    .iter()
                    .any(|diagnostic| matches!(diagnostic.code, "E0772" | "E0645")),
                "{source}\n{:#?}",
                analysis.diagnostics
            );
            assert!(
                analysis
                    .diagnostics
                    .iter()
                    .all(|diagnostic| diagnostic.code != "E0642"),
                "{source}\n{:#?}",
                analysis.diagnostics
            );
        }
    }
}

#[test]
fn borrowed_helpers_aliases_and_local_callbacks_stay_within_the_request_call() {
    for request in ["ReadRequest", "WriteRequest"] {
        let source = format!(
            r#"
use Doria\Std\Io\{request};
function inspect({request} $request): void {{}}
function inspectGeneric<T>(T $value): void {{}}
function identity({request} $request): {request} {{ return $request; }}
function identityGeneric<T>(T $value): T {{ return $value; }}
function phase({request} $request): void {{
    let $alias = $request;
    inspect($alias);
    inspectGeneric($request);
    let $returned = identity($alias);
    inspect($returned);
    let $generic = identityGeneric($request);
    inspect($generic);
    let $callback = function(): void with ($request) {{ inspect($request); }};
    $callback();
}}
function main(): void {{}}
"#
        );
        let analysis = analyze(&source);
        assert!(
            analysis.diagnostics.is_empty(),
            "{source}\n{:#?}",
            analysis.diagnostics
        );
        let mir = doriac::lower_source_to_mir("request-loans.doria", source).unwrap();
        doriac::mir_validation::validate_program(&mir).unwrap();
    }
}

#[test]
fn unrelated_request_names_retain_ordinary_class_ownership() {
    for request in ["ReadRequest", "WriteRequest"] {
        let source = format!(
            r#"
class {request} {{ int $value = 1; }}
function identity(take {request} $request): {request} {{ return $request; }}
function main(): void {{ let $request = identity(new {request}()); }}
"#
        );
        let analysis = analyze(&source);
        assert!(
            analysis.diagnostics.is_empty(),
            "{source}\n{:#?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn mir_preserves_call_scoped_request_ownership_and_storage() {
    use doriac::mir::{
        ClassExpression, ClosureEnvironmentPlacement, ClosureEnvironmentStorage,
        FunctionParameterMode, MixedExpression, ReturnType, Rvalue, SharedPayload, Statement, Type,
    };
    for request in ["ReadRequest", "WriteRequest"] {
        let source = format!(
            r#"
use Doria\Std\Io\{request};
class Holder {{ int $value = 1; }}
enum Saved {{ case Value(int $value); }}
function observe({request} $request): void {{}}
function phase({request} $request): void {{
    let $alias = $request;
    mixed $boxed = new Holder();
    let $callback = function(): void with ($request) {{ observe($request); }};
    $callback();
}}
function inspect(function({request}): void $callback): void {{}}
function main(): void {{ let $values = [1, 2]; }}
"#
        );
        let original = doriac::lower_source_to_mir("request-mir.doria", source).unwrap();
        doriac::mir_validation::validate_program(&original).unwrap();
        let class = original
            .classes
            .iter()
            .find(|class| class.name == format!("Doria\\Std\\Io\\{request}"))
            .unwrap()
            .id;
        let ty = Type::Class(class);
        for mutation in 0..13 {
            let mut forged = original.clone();
            match mutation {
                0 | 1 | 2 | 8 => {
                    let function = forged
                        .functions
                        .iter_mut()
                        .find(|function| function.name == "phase")
                        .unwrap();
                    let parameter = function.params[0];
                    match mutation {
                        0 => function.locals[parameter.0].owned = true,
                        1 => function.parameter_modes[0] = FunctionParameterMode::Take,
                        2 => function.return_type = ReturnType::Value(ty),
                        8 => {
                            function.locals[parameter.0].ty =
                                Type::SharedReference(SharedPayload::Class(class))
                        }
                        _ => unreachable!(),
                    }
                }
                3 => {
                    forged
                        .classes
                        .iter_mut()
                        .find(|class| class.name == "Holder")
                        .unwrap()
                        .properties[0]
                        .ty = ty
                }
                4 => {
                    forged
                        .enums
                        .iter_mut()
                        .find(|enumeration| enumeration.name == "Saved")
                        .unwrap()
                        .cases[0]
                        .payload[0]
                        .ty = ty
                }
                5 => {
                    forged
                        .collection_types
                        .iter_mut()
                        .find(|collection| collection.kind == doriac::mir::CollectionKind::List)
                        .unwrap()
                        .value = ty
                }
                6 => {
                    forged
                        .closure_environment_layouts
                        .iter_mut()
                        .find(|layout| layout.fields.iter().any(|field| field.ty == ty))
                        .unwrap()
                        .fields[0]
                        .storage = ClosureEnvironmentStorage::Owned
                }
                7 => {
                    forged
                        .closure_descriptors
                        .iter_mut()
                        .find(|descriptor| descriptor.environment_layout.is_some())
                        .unwrap()
                        .environment_placement = ClosureEnvironmentPlacement::Heap
                }
                9 | 10 => {
                    let signature = forged
                        .function_types
                        .iter_mut()
                        .find(|signature| {
                            signature
                                .parameters
                                .iter()
                                .any(|parameter| parameter.ty == ty)
                        })
                        .unwrap();
                    if mutation == 9 {
                        signature.parameters[0].mode = FunctionParameterMode::Take;
                    } else {
                        signature.return_type = ReturnType::Value(ty);
                    }
                }
                11 | 12 => {
                    let function = forged
                        .functions
                        .iter_mut()
                        .find(|function| function.name == "phase")
                        .unwrap();
                    let parameter = function.params[0];
                    let (value, payload_owned) = function
                        .blocks
                        .iter_mut()
                        .flat_map(|block| &mut block.statements)
                        .find_map(|statement| match statement {
                            Statement::AssignLocal {
                                value:
                                    Rvalue::Mixed(MixedExpression::BoxClass {
                                        value,
                                        payload_owned,
                                    }),
                                ..
                            } => Some((value, payload_owned)),
                            _ => None,
                        })
                        .unwrap();
                    *value = ClassExpression::Local {
                        class,
                        local: parameter,
                        transfer: false,
                    };
                    *payload_owned = mutation == 11;
                }
                _ => unreachable!(),
            }
            let error = doriac::mir_validation::validate_program(&forged)
                .expect_err("request ownership must remain checked in MIR");
            assert!(
                error.message.contains("call-scoped pending request"),
                "{request}, mutation {mutation}: {error:?}"
            );
        }
    }
}

#[test]
fn inherited_pending_requirements_keep_the_nonblocking_contract() {
    let source = adapter(
        true,
        r#"echo "blocking";"#,
        "",
        "interface UserPending extends PendingReadableStream {}",
    )
    .replace("implements PendingReadableStream", "implements UserPending");
    let analysis = analyze(&source);
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0771"),
        "{source}\n{:#?}",
        analysis.diagnostics
    );
}

/// A blocking operation reached inside a compiler-owned standard source is
/// reported at the authored call that entered it, the same way on every run.
#[test]
fn blocking_sites_inside_standard_sources_are_reported_at_the_authored_call() {
    let forwarding = r#"
class ForwardingReader implements PendingReadableStream {
    internal writable PendingReadableStream $source;
    function __construct(parameter take PendingReadableStream $source) { $this->source = $source; }
    ReadinessSource $readinessSource { borrowed get => $this->source->readinessSource; }
    writable function read(writable WritableByteRegion $destination): ReadOutcome throws IoError {
        return $this->source->read($destination);
    }
    writable function prepareRead(writable ReadRequest $request, writable WritableByteRegion $destination): void throws IoError {
        $this->source->read($destination);
    }
    writable function completeRead(ReadRequest $request, writable WritableByteRegion $destination): ?ReadOutcome throws IoError {
        return $this->source->completeRead($request, $destination);
    }
}
"#;
    let helper = adapter(
        true,
        "helper();",
        "",
        "function helper(): void throws IoError { read_line(); }",
    );
    for (source, call) in [
        (
            format!("{IMPORTS}\n{forwarding}"),
            "$this->source->read($destination)",
        ),
        (helper, "read_line()"),
    ] {
        let mut sites = Vec::new();
        for _ in 0..8 {
            let diagnostics = doriac::check_source("pending-stream.doria", source.as_str())
                .expect_err("a blocking pending phase must be rejected");
            let diagnostic = diagnostics
                .iter()
                .find(|diagnostic| diagnostic.code == "E0771")
                .unwrap_or_else(|| panic!("{source}\n{diagnostics:#?}"));
            let site = diagnostic
                .labels
                .iter()
                .find(|label| label.role == doriac::diagnostics::LabelRole::Secondary)
                .unwrap_or_else(|| panic!("{diagnostic:#?}"))
                .span;
            assert!(!site.source.is_compiler_owned(), "{diagnostic:#?}");
            assert_eq!(&source[site.start..site.end], call, "{diagnostic:#?}");
            sites.push(site);
        }
        sites.dedup();
        assert_eq!(sites.len(), 1, "{source}\n{sites:#?}");
    }
}
