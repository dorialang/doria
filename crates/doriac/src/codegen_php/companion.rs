//! PHP output as host of the native companion library (Decision 0148
//! sections 22 and 23, entry ABI version 1).
//!
//! A program with unsafe regions runs each region as one call into its
//! companion, `<stem>.native.<ext>` beside the generated PHP. The PHP
//! checks the companion's sidecar against the plan it was generated for,
//! loads the library with `FFI::cdef` after the shared runtime, and
//! attaches its host record before the first region runs. Each region call
//! keeps a frame of the region function's MIR locals: a local that lowers a
//! source binding takes that binding's PHP value before the call and gives
//! it back after, and the exit the region took continues as the source
//! statement at the unsafe site would.

use super::*;
use crate::foreign::UnsafeContextFact;

/// The companion plan PHP emission reads at each unsafe site.
#[derive(Debug)]
pub(super) struct Sites {
    program: mir::Program,
    compilation_context: crate::names::CompilationContext,
    compilation_contexts: HashMap<crate::source::SourceId, crate::names::CompilationContext>,
    unsafe_contexts: HashMap<Span, UnsafeContextFact>,
}

impl Sites {
    /// The plan of a program whose MIR has a companion; `None` without one.
    pub(super) fn build(
        program: &Program,
        mir: Option<&mir::Program>,
    ) -> Result<Option<Rc<Self>>, BackendError> {
        let Some(mir) = mir.filter(|mir| !mir.companion.regions.is_empty()) else {
            return Ok(None);
        };
        let sites = Self {
            program: mir.clone(),
            compilation_context: program.semantic_info.compilation_context.clone(),
            compilation_contexts: program.semantic_info.compilation_contexts.clone(),
            unsafe_contexts: program.semantic_info.unsafe_contexts.clone(),
        };
        sites.check_supported()?;
        Ok(Some(Rc::new(sites)))
    }

    pub(super) fn program(&self) -> &mir::Program {
        &self.program
    }

    /// The forms of a companion this PHP host does not run yet: host
    /// operations, callbacks, region edges other than fallthrough, `break`
    /// and `continue`, and slots other than numbers, `bool` and strings.
    fn check_supported(&self) -> Result<(), BackendError> {
        let companion = &self.program.companion;
        let mut diagnostics = Vec::new();
        for region in &companion.regions {
            let (function, unsafe_region) = self.lowering(region);
            let keyword = unsafe_region.keyword;
            if companion
                .host_operations
                .iter()
                .any(|operation| companion.regions[operation.region].id == region.id)
            {
                diagnostics.push(crate::unsafe_code::companion_form(
                    keyword,
                    "Host Operations In Unsafe Code Cannot Be Built For PHP Output Yet",
                    "this unsafe code calls Doria code, writes output or uses a value that stays in PHP, which is part of Doria; PHP output does not run those operations inside unsafe code yet",
                ));
                continue;
            }
            if unsafe_region.exits.iter().any(|exit| {
                !matches!(
                    exit.kind,
                    mir::UnsafeRegionExitKind::Break | mir::UnsafeRegionExitKind::Continue
                )
            }) {
                diagnostics.push(crate::unsafe_code::companion_form(
                    keyword,
                    "Returns And Errors Leaving Unsafe Code Cannot Be Built For PHP Output Yet",
                    "a `return` or a checked error that leaves unsafe code is part of Doria; PHP output does not build them yet",
                ));
                continue;
            }
            for slot in region.inputs.slots.iter().chain(&region.outputs.slots) {
                if slot.value.kind == mir::UnsafeRegionValueKind::Native
                    && native_codec(slot.value, function.locals[slot.value.local.0].ty).is_none()
                    || !matches!(
                        slot.value.kind,
                        mir::UnsafeRegionValueKind::Native | mir::UnsafeRegionValueKind::Text
                    )
                {
                    diagnostics.push(crate::unsafe_code::companion_form(
                        keyword,
                        "These Values Cannot Cross Into Unsafe Code In PHP Output Yet",
                        format!(
                            "`${}` crossing into unsafe code is part of Doria; PHP output passes numbers, `bool` values and strings to unsafe code, and not this value yet",
                            function.locals[slot.value.local.0].name.trim_start_matches('$')
                        ),
                    ));
                    break;
                }
            }
        }
        if !companion.callbacks.is_empty() {
            let span = self.program.functions[companion.callbacks[0].function.0].source_span;
            diagnostics.push(crate::unsafe_code::companion_form(
                span,
                "C-Callable Functions Cannot Be Built For PHP Output Yet",
                "a C-callable function is part of Doria; PHP output does not build them yet",
            ));
        }
        if diagnostics.is_empty() {
            Ok(())
        } else {
            Err(BackendError::from_diagnostics(diagnostics))
        }
    }

    fn lowering(&self, region: &mir::CompanionRegion) -> (&mir::Function, &mir::UnsafeRegion) {
        let lowering = region.lowerings[0];
        let function = &self.program.functions[lowering.function.0];
        (function, &function.unsafe_regions[lowering.region])
    }

    /// The companion region of the unsafe context at `span` in the
    /// specialization with `substitutions`, with its first lowering.
    fn region_at(
        &self,
        span: Span,
        form: crate::foreign::UnsafeContextForm,
        substitutions: &HashMap<String, ResolvedType>,
    ) -> Option<(usize, &mir::Function, &mir::UnsafeRegion)> {
        let fact = self
            .unsafe_contexts
            .get(&span)
            .filter(|fact| fact.form == form)?;
        let compilation = self
            .compilation_contexts
            .get(&fact.span.source)
            .unwrap_or(&self.compilation_context);
        let id = crate::foreign::region_identity(compilation, fact.span, substitutions);
        let index = self
            .program
            .companion
            .regions
            .iter()
            .position(|region| region.id == id)?;
        let (function, region) = self.lowering(&self.program.companion.regions[index]);
        Some((index, function, region))
    }
}

/// The PHP pack format, value size and nullable form of a native slot, as
/// `[format, size, slotSize, nullForm, payloadOffset, isBool]`: `nullForm`
/// is 0 for a value with no presence, 1 for a presence word then the value
/// at `payloadOffset`, and 2 for a value whose all-zero form is null.
fn native_codec(value: mir::UnsafeRegionValue, ty: mir::Type) -> Option<String> {
    let scalar = match ty {
        mir::Type::Scalar(scalar) | mir::Type::NullableScalar(scalar) => scalar,
        _ => return None,
    };
    let (format, size, is_bool) = match scalar {
        mir::ScalarType::Integer(integer) => {
            let format = match integer {
                IntegerType::Int8 => "c",
                IntegerType::UInt8 => "C",
                IntegerType::Int16 => "s",
                IntegerType::UInt16 => "S",
                IntegerType::Int32 => "l",
                IntegerType::UInt32 => "L",
                IntegerType::Int64 => "q",
                IntegerType::UInt64 => "Q",
            };
            (format, integer.storage_bytes(), false)
        }
        mir::ScalarType::Float(FloatType::Float32) => ("f", 4, false),
        mir::ScalarType::Float(FloatType::Float64) => ("d", 8, false),
        mir::ScalarType::Bool => ("C", 1, true),
        _ => return None,
    };
    let (slot_size, _) = crate::native_abi::companion_slot_layout(value, ty)?;
    let (null_form, payload) = if value.nullable {
        match crate::native_closure_abi::nullable_scalar_form(scalar, 8) {
            crate::native_closure_abi::NullableScalarForm::Tagged { payload_offset } => {
                (1, payload_offset)
            }
            crate::native_closure_abi::NullableScalarForm::ZeroIsNull => (2, 0),
        }
    } else {
        (0, 0)
    };
    Some(format!(
        "[{}, {size}, {slot_size}, {null_form}, {payload}, {}]",
        emit_php_string_literal(format),
        if is_bool { "true" } else { "false" }
    ))
}

/// One slot record as PHP: `[[local, offset, kind, codec], ...]`.
fn slots(function: &mir::Function, record: &mir::SlotRecord) -> String {
    let slots = record
        .slots
        .iter()
        .map(|slot| {
            let (kind, codec) = match slot.value.kind {
                mir::UnsafeRegionValueKind::Native => (
                    "n",
                    native_codec(slot.value, function.locals[slot.value.local.0].ty)
                        .unwrap_or_else(|| "null".to_string()),
                ),
                mir::UnsafeRegionValueKind::Text => ("t", "null".to_string()),
                mir::UnsafeRegionValueKind::Buffer => ("b", "null".to_string()),
                mir::UnsafeRegionValueKind::Region => ("r", "null".to_string()),
                mir::UnsafeRegionValueKind::ForeignString => ("f", "null".to_string()),
                mir::UnsafeRegionValueKind::HostToken => ("h", "null".to_string()),
            };
            format!(
                "[{}, {}, {}, {codec}]",
                slot.value.local.0,
                slot.offset,
                emit_php_string_literal(kind)
            )
        })
        .collect::<Vec<_>>();
    format!("[{}]", slots.join(", "))
}

/// The companion's declarations for `FFI::cdef`: the host record and the
/// native frame and fact records its panic sink reads, then every region
/// entry. The records cross as `void *`, so the library's declarations
/// and the runtime's never need to name one type.
fn declarations(program: &mir::Program) -> String {
    let mut declarations = String::from(
        "typedef struct DrStackFrameV2 {\n    const struct DrStackFrameV2 *parent;\n    const uint8_t *function_name; size_t function_name_length;\n    const uint8_t *source_path; size_t source_path_length;\n    const uint8_t *source_text; size_t source_text_length;\n    size_t function_span_start; size_t function_span_end;\n    size_t active_span_start; size_t active_span_end;\n} DrStackFrameV2;\ntypedef struct DrRuntimeFactV2 {\n    const uint8_t *name; size_t name_length; size_t kind; uint64_t value; size_t value_length;\n} DrRuntimeFactV2;\ntypedef struct DrRegionHostV1 {\n    int32_t (*call)(void *host, uint32_t operation, void *arguments, void *results);\n    void (*panic)(void *host, const DrStackFrameV2 *frame, const uint8_t *code, size_t code_length,\n        const uint8_t *message, size_t message_length, const DrRuntimeFactV2 *facts, size_t fact_count);\n} DrRegionHostV1;\n",
    );
    for region in &program.companion.regions {
        declarations.push_str(&format!(
            "int32_t {}(void *host, const void *caller_frame, void *inputs, void *outputs);\n",
            region.entry_symbol()
        ));
    }
    declarations
}

/// Emits the companion's loader and host after the shared runtime's.
pub(super) fn emit_runtime(
    output: &mut String,
    program: &Program,
    sites: &Sites,
) -> Result<(), BackendError> {
    let source = crate::source::SourceFile::new(&program.source_path, &program.source_text);
    output.push_str("$__doria_native_failures += [\n");
    for (key, explanation) in [
        ("companionMissing", "The native companion library or its sidecar is missing or unreadable."),
        ("companionIdentity", "The native companion library or its sidecar does not match this generated PHP program."),
        ("companionLoad", "PHP could not load the verified native companion library on this host. Its architecture and the native libraries it links must be present."),
    ] {
        let diagnostic = Diagnostic::new(
            "B3605",
            "PHP native companion library is unavailable",
            Span::default(),
        )
        .with_title("PHP Native Companion Library Is Unavailable")
        .with_explanation(explanation)
        .with_help("Deploy the generated PHP output together with the `.native` library, its `.native.json` sidecar and the `.doria-runtime` directory the compiler wrote beside it, on a matching platform.");
        output.push_str(&format!(
            "    {} => {},\n",
            emit_php_string_literal(key),
            emit_php_string_literal(&(diagnostic.render(&source) + "\n"))
        ));
    }
    output.push_str("];\n");
    let expected = crate::native_companion::expected_sidecar_of(
        sites.program(),
        crate::backend::NativeProfile::Fast,
    )?;
    output.push_str(&format!(
        "$__doria_companion_expected = json_decode(hex2bin({}), true);\n",
        emit_php_string_literal(&hex_bytes(expected.to_string().as_bytes()))
    ));
    output.push_str("$__doria_companion_regions = [\n");
    for region in &sites.program().companion.regions {
        let (function, _) = sites.lowering(region);
        output.push_str(&format!(
            "    [{}, {}, {}, {}, {}, {}],\n",
            emit_php_string_literal(&region.entry_symbol()),
            region.inputs.size,
            region.outputs.size,
            region
                .outputs
                .index
                .map_or_else(|| "-1".to_string(), |index| index.to_string()),
            slots(function, &region.inputs),
            slots(function, &region.outputs),
        ));
    }
    output.push_str("];\n");
    output.push_str(
        &RUNTIME
            .replace(
                "__DORIA_COMPANION_EXTENSION__",
                &emit_php_string_literal(crate::codegen_native::shared_library_extension()),
            )
            .replace(
                "__DORIA_COMPANION_DECLARATIONS__",
                &declarations(sites.program()),
            ),
    );
    Ok(())
}

/// The PHP frame entries a region call starts with: every local it takes
/// from its surroundings that lowers a source binding, from that binding's
/// place, as `[local => value, ...]`; and the places each output that lowers a
/// binding goes back to after the call, as `(local, place)`.
fn frame_exchange(
    function: &mir::Function,
    region: &mir::UnsafeRegion,
    scopes: &PhpNameScopes,
) -> (String, Vec<(mir::LocalId, String)>) {
    let place = |local: mir::LocalId| {
        function.locals[local.0].binding.map(|binding| {
            scopes.place(binding).map_or_else(
                || {
                    let declaration = scopes
                        .closure_plan
                        .binding_resolution
                        .declarations_by_id
                        .get(&binding)
                        .expect("a MIR local names a declared binding");
                    format!("${}", scopes.php_name(&declaration.name))
                },
                PhpBindingPlace::read,
            )
        })
    };
    // A region reads only the locals its own code names, which lower
    // source bindings. An input that lowers none (its result, or a
    // temporary the statement around it destroys) gets its value inside
    // the region, so it starts with none.
    let seeds = region
        .inputs
        .iter()
        .filter_map(|input| place(input.local).map(|read| format!("{} => {read}", input.local.0)))
        .collect::<Vec<_>>();
    let writes = region
        .outputs
        .iter()
        .filter_map(|output| place(output.local).map(|place| (output.local, place)))
        .collect();
    (format!("[{}]", seeds.join(", ")), writes)
}

/// Emits `unsafe { }` at `block` as a region call, when the block is one.
pub(super) fn emit_region_block(
    block: &Block,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> bool {
    emit_region_statement(
        block.span,
        crate::foreign::UnsafeContextForm::Block,
        output,
        indent,
        scopes,
    )
}

/// Emits `unsafe(<expression>);` as a region call, when `expr` is one.
pub(super) fn emit_region_expression_statement(
    expr: &Expr,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> bool {
    let Expr::Grouped { span, .. } = expr else {
        return false;
    };
    emit_region_statement(
        *span,
        crate::foreign::UnsafeContextForm::Expression,
        output,
        indent,
        scopes,
    )
}

fn emit_region_statement(
    span: Span,
    form: crate::foreign::UnsafeContextForm,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> bool {
    let Some(sites) = scopes.companion.clone() else {
        return false;
    };
    let Some((index, function, region)) = sites.region_at(span, form, &scopes.substitutions) else {
        return false;
    };
    let frame = scopes.fresh_temp("__doria_region");
    let (seeds, writes) = frame_exchange(function, region, scopes);
    writeln(
        output,
        indent,
        &format!("${frame} = __DoriaCompanion::run({index}, {seeds});"),
    );
    for (local, place) in writes {
        writeln(output, indent, &format!("{place} = ${frame}[{}];", local.0));
    }
    for (position, exit) in region.exits.iter().enumerate() {
        let transfer = match exit.kind {
            mir::UnsafeRegionExitKind::Break => "break;",
            mir::UnsafeRegionExitKind::Continue => "continue;",
            mir::UnsafeRegionExitKind::Return | mir::UnsafeRegionExitKind::CheckedError { .. } => {
                unreachable!("the companion plan check admits only break and continue exits")
            }
        };
        writeln(
            output,
            indent,
            &format!("if (${frame}[-1] === {position}) {{ {transfer} }}"),
        );
    }
    true
}

/// `unsafe(<expression>)` as a value: one PHP expression that calls the
/// region, gives its outputs back to their bindings' places and reads its
/// result.
pub(super) fn emit_region_value(expr: &Expr, scopes: &PhpNameScopes) -> Option<String> {
    let Expr::Grouped { span, .. } = expr else {
        return None;
    };
    let sites = scopes.companion.clone()?;
    let (index, function, region) = sites.region_at(
        *span,
        crate::foreign::UnsafeContextForm::Expression,
        &scopes.substitutions,
    )?;
    let result = region.result?;
    let (seeds, writes) = frame_exchange(function, region, scopes);
    let (locals, places): (Vec<_>, Vec<_>) = writes
        .into_iter()
        .map(|(local, place)| (local.0.to_string(), place))
        .unzip();
    let mut arguments = vec![
        format!("__DoriaCompanion::run({index}, {seeds})"),
        result.0.to_string(),
        format!("[{}]", locals.join(", ")),
    ];
    arguments.extend(places);
    Some(format!("__DoriaCompanion::value({})", arguments.join(", ")))
}

const RUNTIME: &str = r#"
final class __DoriaCompanion
{
    private static ?FFI $library = null;
    private static ?FFI\CData $host = null;
    /** @var list<array> The region calls in progress, innermost last. */
    private static array $calls = [];

    public static function get(): FFI
    {
        if (self::$library !== null) { return self::$library; }
        // The companion links the shared runtime, which loads first.
        $runtime = __DoriaNativeIo::get();
        global $__doria_companion_expected;
        $stem = pathinfo(__FILE__, PATHINFO_FILENAME);
        $name = $stem . ".native." . __DORIA_COMPANION_EXTENSION__;
        $path = __DIR__ . DIRECTORY_SEPARATOR . $name;
        $sidecar = __DIR__ . DIRECTORY_SEPARATOR . $stem . ".native.json";
        $document = @file_get_contents($sidecar);
        if ($document === false || !is_file($path) || !is_readable($path)) {
            __DoriaNativeIo::fail("companionMissing");
        }
        $metadata = json_decode($document, true);
        if (!is_array($metadata) || ($metadata["library"] ?? null) !== $name) {
            __DoriaNativeIo::fail("companionIdentity");
        }
        foreach ($__doria_companion_expected as $field => $expected) {
            if (!array_key_exists($field, $metadata) || $metadata[$field] !== $expected) {
                __DoriaNativeIo::fail("companionIdentity");
            }
        }
        if (@filesize($path) !== ($metadata["bytes"] ?? null) ||
            @hash_file("sha256", $path) !== ($metadata["sha256"] ?? null)) {
            __DoriaNativeIo::fail("companionIdentity");
        }
        try {
            $library = FFI::cdef(<<<'DORIA_COMPANION_ABI'
__DORIA_COMPANION_DECLARATIONS__DORIA_COMPANION_ABI, $path);
        } catch (Throwable) {
            __DoriaNativeIo::fail("companionLoad");
        }
        // The record lives for the process: the runtime keeps its address.
        $host = $library->new("DrRegionHostV1", false);
        $host->call = static function ($record, int $operation, $arguments, $results): int {
            return __DoriaCompanion::call($operation, $arguments, $results);
        };
        $host->panic = static function ($record, $frame, $code, int $codeLength, $message,
            int $messageLength, $facts, int $factCount): void {
            __DoriaCompanion::panic($frame, $code, $codeLength, $message, $messageLength, $facts, $factCount);
        };
        $runtime->dr_v1_attach_region_host(FFI::addr($host));
        self::$host = $host;
        return self::$library = $library;
    }

    /** Whether a region call is in progress, so nothing may leave through PHP. */
    public static function active(): bool
    {
        return self::$calls !== [];
    }

    /**
     * Runs region `$region` with the frame `$frame` of the locals it takes
     * from its surroundings. Returns the frame after the call, its outputs
     * written, with the exit taken at key -1 (-1 for fallthrough).
     */
    public static function run(int $region, array $frame): array
    {
        global $__doria_companion_regions;
        $library = self::get();
        [$symbol, $inputSize, $outputSize, $exitOffset, $inputs, $outputs] =
            $__doria_companion_regions[$region];
        $lent = [];
        $record = $library->new("uint8_t[" . max($inputSize, 1) . "]");
        $base = self::address($library, FFI::addr($record));
        foreach ($inputs as [$local, $offset, $kind, $codec]) {
            $bytes = self::store($library, $kind, $codec, $frame[$local] ?? null, $lent);
            FFI::memcpy(self::pointer($library, $base + $offset), $bytes, strlen($bytes));
        }
        $result = $library->new("uint8_t[" . max($outputSize, 1) . "]");
        $kept = [];
        $caller = self::callerFrame($library, $kept);
        self::$calls[] = [
            "region" => $region,
            "outputs" => self::address($library, FFI::addr($result)),
            "accepted" => [],
            "frame" => $frame,
        ];
        $status = $library->$symbol(FFI::addr(self::$host), $caller, FFI::addr($record), FFI::addr($result));
        $call = array_pop(self::$calls);
        $frame = $call["frame"];
        $bytes = FFI::string($result, max($outputSize, 1));
        foreach ($outputs as [$local, $offset, $kind, $codec]) {
            if ($kind === "n") {
                $frame[$local] = self::decode($codec, substr($bytes, $offset, $codec[2]));
            } else {
                $token = unpack("Q", substr($bytes, $offset, 8))[1];
                $frame[$local] = $token === 0 ? null : $call["accepted"][$token - 1];
            }
        }
        $frame[-1] = $status === 0 ? -1 : unpack("l", substr($bytes, $exitOffset, 4))[1];
        return $frame;
    }

    /** The result of an expression region, after giving each output back to its place. */
    public static function value(array $frame, int $result, array $locals, mixed &...$places): mixed
    {
        foreach ($locals as $position => $local) { $places[$position] = $frame[$local]; }
        return $frame[$result];
    }

    /** `host->call`: operation 0 accepts an output the companion owns. */
    public static function call(int $operation, $arguments, $results): int
    {
        global $__doria_companion_regions;
        $library = self::$library;
        if ($operation === 0) {
            $top = count(self::$calls) - 1;
            $address = self::address($library, $arguments);
            $offset = $address - self::$calls[$top]["outputs"];
            $kind = null;
            foreach ($__doria_companion_regions[self::$calls[$top]["region"]][5] as $slot) {
                if ($slot[1] === $offset) { $kind = $slot[2]; }
            }
            [, $data, $length] = unpack("Q2", FFI::string(self::pointer($library, $address), 16));
            $value = $data === 0 ? null :
                ($length === 0 ? "" : FFI::string(self::pointer($library, $data), $length));
            $token = 0;
            if ($kind === "t" && $value !== null) {
                self::$calls[$top]["accepted"][] = $value;
                $token = count(self::$calls[$top]["accepted"]);
            }
            FFI::memcpy(self::pointer($library, self::address($library, $results)), pack("Q", $token), 8);
            return 0;
        }
        __DoriaCompanion::end("the companion called host operation " . $operation);
    }

    /** The panic sink: renders the companion's panic as a native one does and ends the process. */
    public static function panic($frame, $code, int $codeLength, $message, int $messageLength,
        $facts, int $factCount): void
    {
        global $__doria_sources;
        $text = static fn ($data, int $length): string =>
            ($data === null || FFI::isNull($data) || $length === 0) ? "" : FFI::string($data, $length);
        $frames = [];
        $site = null;
        while ($frame !== null && !FFI::isNull($frame)) {
            $record = $frame[0];
            $path = $text($record->source_path, $record->source_path_length);
            $source = $text($record->source_text, $record->source_text_length);
            $start = $record->active_span_start;
            $end = $record->active_span_end;
            $line = substr_count(substr($source, 0, $start), "\n") + 1;
            $frames[] = [$text($record->function_name, $record->function_name_length), $path, $start, $end, $line];
            if ($site === null) {
                $site = [$path, $start, $end];
            }
            $frame = $record->parent;
        }
        $location = static function (string $path, int $offset) use ($__doria_sources): int {
            foreach ($__doria_sources as $id => [$sourcePath]) {
                if ($sourcePath === $path) { return $id * 4294967296 + $offset; }
            }
            return 0;
        };
        $values = [];
        for ($index = 0; $index < $factCount; $index++) {
            $fact = $facts[$index];
            $kind = $fact->kind;
            $values[$text($fact->name, $fact->name_length)] = [match ($kind) { 1, 3, 4 => $kind, default => 2 }, match ($kind) {
                1 => (int) $fact->value,
                3 => $fact->value !== 0,
                4 => $text(self::pointer(self::$library, $fact->value), $fact->value_length),
                default => $fact->value,
            }];
        }
        $note = ($message === null || FFI::isNull($message)) ? null : $text($message, $messageLength);
        __doria_panic(
            $text($code, $codeLength),
            $site === null ? 0 : $location($site[0], $site[1]),
            $site === null ? 0 : $location($site[0], $site[2]),
            $note,
            $frames[0][0] ?? null,
            $values,
            null,
            $frames,
        );
    }

    /** Ends the process from inside a region call, where PHP cannot unwind. */
    public static function endProcess(int $status): never
    {
        fflush(__DORIA_STDOUT);
        fflush(__DORIA_STDERR);
        __DoriaNativeIo::get()->dr_v1_exit_process($status);
    }

    /** A host failure inside a region call, which cannot return to the companion. */
    private static function end(string $failure): never
    {
        fwrite(__DORIA_STDERR, "PHP companion host failure: " . $failure . "\n");
        self::endProcess(70);
    }

    /** The bytes of an input slot of `$kind`, keeping alive in `$lent` what they point at. */
    private static function store(FFI $library, string $kind, ?array $codec, mixed $value, array &$lent): string
    {
        if ($kind === "n") { return self::encode($codec, $value); }
        if ($value === null) { return str_repeat("\0", 16); }
        $length = strlen($value);
        $buffer = $library->new("uint8_t[" . max($length, 1) . "]");
        if ($length !== 0) { FFI::memcpy($buffer, $value, $length); }
        $lent[] = $buffer;
        return pack("QQ", self::address($library, FFI::addr($buffer)), $length);
    }

    private static function encode(array $codec, mixed $value): string
    {
        [$format, $size, $slotSize, $nullForm, $payload, $isBool] = $codec;
        if ($value === null) { return str_repeat("\0", $slotSize); }
        $bytes = $isBool ? pack("C", $value ? 1 : 0) : pack($format, $value);
        if ($nullForm === 1) { $bytes = pack("Q", 1) . str_repeat("\0", $payload - 8) . $bytes; }
        return str_pad($bytes, $slotSize, "\0");
    }

    private static function decode(array $codec, string $bytes): mixed
    {
        [$format, $size, $slotSize, $nullForm, $payload, $isBool] = $codec;
        if ($nullForm === 1) {
            if (substr($bytes, 0, 8) === str_repeat("\0", 8)) { return null; }
            $bytes = substr($bytes, $payload);
        } elseif ($nullForm === 2 && $bytes === str_repeat("\0", strlen($bytes))) {
            return null;
        }
        $value = unpack($format, substr($bytes, 0, $size))[1];
        return $isBool ? $value !== 0 : $value;
    }

    /**
     * The host's frames as the native frame chain a region entry links to,
     * innermost last in `$kept`, which keeps every record and string alive.
     */
    private static function callerFrame(FFI $library, array &$kept): ?FFI\CData
    {
        global $__doria_frames, $__doria_frame_top, $__doria_diagnostic_sites;
        $parent = null;
        $bytes = static function (string $text) use ($library, &$kept): FFI\CData {
            $buffer = $library->new("uint8_t[" . max(strlen($text), 1) . "]");
            if ($text !== "") { FFI::memcpy($buffer, $text, strlen($text)); }
            $kept[] = $buffer;
            return $library->cast("uint8_t *", FFI::addr($buffer));
        };
        for ($base = 0; $base < ($__doria_frame_top ?? 0); $base += 3) {
            $origin = $base + 3 < $__doria_frame_top ? $__doria_frames[$base + 5] : $__doria_frames[$base + 1];
            $site = $__doria_diagnostic_sites[$origin] ?? ["start" => 0, "end" => 0];
            [$path, $source, $start] = __doria_source_location($site["start"]);
            [, , $end] = __doria_source_location($site["end"]);
            $record = $library->new("DrStackFrameV2");
            $record->parent = $parent;
            $name = $__doria_frames[$base];
            $record->function_name = $bytes($name);
            $record->function_name_length = strlen($name);
            $record->source_path = $bytes($path);
            $record->source_path_length = strlen($path);
            $record->source_text = $bytes($source);
            $record->source_text_length = strlen($source);
            $record->active_span_start = $start;
            $record->active_span_end = $end;
            $kept[] = $record;
            $parent = FFI::addr($record);
        }
        return $parent;
    }

    /** The address a pointer holds. */
    private static function address(FFI $library, FFI\CData $pointer): int
    {
        $cell = $library->new("void *[1]");
        $cell[0] = $pointer;
        return unpack("Q", FFI::string($cell, 8))[1];
    }

    /** A byte pointer to `$address`. */
    private static function pointer(FFI $library, int $address): FFI\CData
    {
        $cell = $library->new("uint8_t *[1]");
        FFI::memcpy($cell, pack("Q", $address), 8);
        return $cell[0];
    }
}
"#;
