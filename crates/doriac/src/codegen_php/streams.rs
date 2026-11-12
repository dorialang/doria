//! Typed file intrinsics over one private PHP file owner. Doria's ordinary
//! nominal class, enum, checked-error, and deterministic-drop paths stay in use.
use super::*;
use crate::compiler_known_stream::FileFunction;

mod hosted;
pub(super) use hosted::{emit_group_body, emit_group_members};

pub(super) fn has_payload(class: &ClassDecl) -> bool {
    mir::RuntimeClassPayload::for_declaration(&class.name, class.span.source)
        == Some(mir::RuntimeClassPayload::OwnedFile)
}

pub(super) fn has_readiness_payload(class: &ClassDecl) -> bool {
    class.name == crate::compiler_known_stream::READINESS_SOURCE
        && class.span.source == crate::compiler_known_stream::SOURCE_ID
}

pub(super) fn has_set_payload(class: &ClassDecl) -> bool {
    class.name == crate::compiler_known_stream::READINESS_SET
        && class.span.source == crate::compiler_known_stream::SOURCE_ID
}

pub(super) fn has_plan_payload(class: &ClassDecl) -> bool {
    class.name == crate::compiler_known_stream::TRANSFER_PLAN
        && class.span.source == crate::compiler_known_stream::SOURCE_ID
}

pub(super) fn has_group_payload(class: &ClassDecl) -> bool {
    class.name == crate::compiler_known_stream::TRANSFER_GROUP
        && class.span.source == crate::compiler_known_stream::SOURCE_ID
}

pub(super) fn uses_group_payload(scopes: &PhpNameScopes) -> bool {
    scopes.current_class.as_deref() == Some(crate::compiler_known_stream::TRANSFER_GROUP)
}

pub(super) fn uses_plan_payload(scopes: &PhpNameScopes) -> bool {
    scopes.current_class.as_deref() == Some(crate::compiler_known_stream::TRANSFER_PLAN)
}

pub(super) fn emit_plan_payload_members(output: &mut String, indent: usize) {
    for line in r#"private ?FFI\CData $__doriaTransferPlan;
private array $__doriaTransferOperations;
public function __doriaBegin(FFI\CData $set, array $members, mixed $deadline, mixed $cancellation): __DoriaHostedTransferGroup
{
    $plan = $this->__doriaTransferPlan;
    if ($plan === null) { __DoriaNativeIo::fail('protocol'); }
    $operations = $this->__doriaTransferOperations;
    $this->__doriaTransferPlan = null;
    $this->__doriaTransferOperations = [];
    return __DoriaHostedTransferGroup::begin($set, $plan, $operations, $members, $deadline, $cancellation);
}
// Each entry is [member identity, region, kind] at its plan position. Kind is
// the runtime's: 0 a write, 1 a read, 2 a watch, which has neither of the others.
private static function __doriaConfigure(?FFI\CData $previous, array $operations, FFI\CData $key,
    __DoriaByteRegion $region, bool $reading, string $identity): self
{
    $runtime = __DoriaNativeIo::get();
    $outputs = self::__doriaPlanOutputs($runtime);
    [$result, $message, $path, $system, $transferred, $meta] = $outputs;
    $source = $region->__doriaNativeStorage($reading);
    $base = $source->__doriaNativeBase();
    $capacity = count($source);
    $offset = $region->__doriaRegionOffset();
    $length = count($region);
    $code = $previous === null
        ? $runtime->dr_v1_host_transfer_plan_new(null, $key, $base, $capacity, $offset, $length,
            (int) $reading, $result, $message, $path, $system, $transferred, $meta)
        : $runtime->dr_v1_host_transfer_plan_append(null, $previous, $key, $base, $capacity, $offset, $length,
            (int) $reading, $result, $message, $path, $system, $transferred, $meta);
    return self::__doriaPlanned($runtime, $code, $outputs, $operations, [$identity, $region, (int) $reading]);
}
// Only the source's one-word identity reaches the runtime, which owns the wait.
private static function __doriaWatch(?FFI\CData $previous, array $operations, int $source): self
{
    $runtime = __DoriaNativeIo::get();
    $outputs = self::__doriaPlanOutputs($runtime);
    [$result, $message, $path, $system, $transferred, $meta] = $outputs;
    $code = $previous === null
        ? $runtime->dr_v1_checked_transfer_plan_watch_new(null, $source,
            $result, $message, $path, $system, $transferred, $meta)
        : $runtime->dr_v1_checked_transfer_plan_watch_append(null, $previous, $source,
            $result, $message, $path, $system, $transferred, $meta);
    return self::__doriaPlanned($runtime, $code, $outputs, $operations, [null, null, 2]);
}
private static function __doriaPlanOutputs(FFI $runtime): array
{
    return [$runtime->new('void *[1]'), $runtime->new('DrStringV1 *[1]'), $runtime->new('DrStringV1 *[1]'),
        $runtime->new('int64_t[1]'), $runtime->new('size_t[1]'), $runtime->new('uint64_t[1]')];
}
private static function __doriaPlanned(FFI $runtime, int $code, array $outputs, array $operations,
    array $operation): self
{
    [$result, $message, $path, $system, $transferred, $meta] = $outputs;
    try {
        if ($code === 0) {
            $operations[] = $operation;
            return new self($result[0], $operations);
        }
        if ($code !== 1) { __DoriaNativeIo::fail('protocol'); }
        __doria_throw_io_facts(__DoriaStdIoIoOperation::Configure,
            __DoriaStdIoIoTarget::__doriaCaseTransferPlan(),
            __doria_native_io_reason(($meta[0] >> 16) & 255),
            ($meta[0] & (1 << 32)) === 0 ? null : $system[0],
            ($meta[0] & (1 << 34)) === 0 ? null : $transferred[0], 0, '');
    } finally {
        foreach ([$message[0], $path[0]] as $string) {
            if ($string !== null && !FFI::isNull($string)) { $runtime->dr_v1_string_release($string); }
        }
    }
}"#.lines() {
        writeln(output, indent, line);
    }
}

pub(super) fn emit_plan_drop(output: &mut String, indent: usize) {
    for line in [
        "if ($this->__doriaTransferPlan !== null) {",
        "    $plan = $this->__doriaTransferPlan;",
        "    $this->__doriaTransferPlan = null;",
        "    __DoriaNativeIo::get()->dr_v1_transfer_plan_drop($plan);",
        "}",
    ] {
        writeln(output, indent, line);
    }
}

pub(super) fn emit_plan_body(
    intrinsic: crate::compiler_known_stream::TransferPlanFunction,
    function: &FunctionDecl,
    output: &mut String,
    indent: usize,
    scopes: &PhpNameScopes,
) {
    use crate::compiler_known_stream::TransferPlanFunction as Plan;
    use crate::native_abi::{TRANSFER_KIND_READ, TRANSFER_KIND_WATCH, TRANSFER_KIND_WRITE};
    let key_kind = match intrinsic {
        Plan::GetWriteKey => Some(TRANSFER_KIND_WRITE),
        Plan::GetReadKey => Some(TRANSFER_KIND_READ),
        Plan::GetWatchKey => Some(TRANSFER_KIND_WATCH),
        Plan::Read
        | Plan::AndReadFrom
        | Plan::WriteTo
        | Plan::AndWriteTo
        | Plan::StartWatch
        | Plan::AppendWatch => None,
    };
    if let Some(kind) = key_kind {
        let position = intrinsic_parameter(function, 0, scopes);
        let key_type = scopes
            .specialization
            .class_symbol_for_type(
                function
                    .return_type
                    .as_ref()
                    .expect("key lookup returns a value"),
                &scopes.substitutions,
            )
            .expect("key lookup returns its exact key");
        for line in [
            "$runtime = __DoriaNativeIo::get();".to_string(),
            "$key = $runtime->new('int64_t[2]');".into(),
            "$message = $runtime->new('DrStringV1 *[1]');".into(),
            "$path = $runtime->new('DrStringV1 *[1]');".into(),
            "$system = $runtime->new('int64_t[1]');".into(),
            "$transferred = $runtime->new('size_t[1]');".into(),
            "$meta = $runtime->new('uint64_t[1]');".into(),
            format!("$code = $runtime->dr_v1_checked_transfer_plan_key(null, $this->__doriaTransferPlan, {position}, {kind}, $key, $message, $path, $system, $transferred, $meta);"),
            "try {".into(),
            format!("    if ($code === 0) {{ return {key_type}::__doriaFromTransferKey($key[0], $key[1]); }}"),
            "    if ($code !== 1) { __DoriaNativeIo::fail('protocol'); }".into(),
            "    __doria_throw_io_facts(__DoriaStdIoIoOperation::Access, __DoriaStdIoIoTarget::__doriaCaseTransferPlan(),".into(),
            "        __doria_native_io_reason(($meta[0] >> 16) & 255),".into(),
            "        ($meta[0] & (1 << 32)) === 0 ? null : $system[0], null, 0, '');".into(),
            "} finally {".into(),
            "    foreach ([$message[0], $path[0]] as $string) {".into(),
            "        if ($string !== null && !FFI::isNull($string)) { $runtime->dr_v1_string_release($string); }".into(),
            "    }".into(),
            "}".into(),
        ] {
            writeln(output, indent, &line);
        }
        return;
    }
    let previous = if intrinsic.is_appending() {
        writeln(output, indent, "$previous = $this->__doriaTransferPlan;");
        writeln(output, indent, "if ($previous === null) { throw new LogicException('consumed transfer plan was used'); }");
        writeln(output, indent, "$this->__doriaTransferPlan = null;");
        writeln(
            output,
            indent,
            "$operations = $this->__doriaTransferOperations;",
        );
        writeln(output, indent, "$this->__doriaTransferOperations = [];");
        "$previous"
    } else {
        writeln(output, indent, "$operations = [];");
        "null"
    };
    if intrinsic.is_watch() {
        // Parameter 0 is the watched source, which only lends itself to the
        // plan; parameter 1 is the one-word identity the runtime waits on.
        let identity = intrinsic_parameter(function, 1, scopes);
        writeln(
            output,
            indent,
            &format!(
                "return self::__doriaWatch({previous}, $operations, {identity}->__doriaLowWord());"
            ),
        );
        return;
    }
    let key = intrinsic_parameter(function, 0, scopes);
    let region = intrinsic_parameter(function, 1, scopes);
    let reading = if intrinsic.is_read() { "true" } else { "false" };
    writeln(output, indent, &format!("return self::__doriaConfigure({previous}, $operations, {key}->__doriaNativeRegistration(), {region}, {reading}, {key}->__doriaRegistrationIdentity());"));
}

pub(super) fn uses_set_payload(scopes: &PhpNameScopes) -> bool {
    scopes.readiness_set_payload
}

pub(super) fn emit_set_payload_members(output: &mut String, indent: usize) {
    writeln(
        output,
        indent,
        "private ?FFI\\CData $__doriaReadinessSet = null;",
    );
    writeln(
        output,
        indent,
        "private ?FFI\\CData $__doriaReadinessStatus = null;",
    );
    writeln(
        output,
        indent,
        "private ?FFI\\CData $__doriaReadinessStatusAddress = null;",
    );
    writeln(
        output,
        indent,
        "private ?FFI\\CData $__doriaReadinessKey = null;",
    );
    writeln(
        output,
        indent,
        "private array $__doriaReadinessMembers = [];",
    );
    writeln(
        output,
        indent,
        "private array $__doriaReadinessPendingMembers = [];",
    );
}

pub(super) fn emit_set_drop(output: &mut String, indent: usize) {
    writeln(output, indent, "if ($this->__doriaReadinessSet !== null) {");
    writeln(
        output,
        indent + 1,
        "__DoriaNativeIo::get()->dr_v1_readiness_set_drop(null, $this->__doriaReadinessSet);",
    );
    writeln(output, indent + 1, "$this->__doriaReadinessSet = null;");
    writeln(output, indent, "}");
    writeln(
        output,
        indent,
        "$this->__doriaReadinessPendingMembers = [];",
    );
    writeln(
        output,
        indent,
        "while (($key = array_key_last($this->__doriaReadinessMembers)) !== null) {",
    );
    writeln(
        output,
        indent + 1,
        "__doria_drop_value($this->__doriaReadinessMembers[$key]);",
    );
    writeln(
        output,
        indent + 1,
        "unset($this->__doriaReadinessMembers[$key]);",
    );
    writeln(output, indent, "}");
    writeln(
        output,
        indent,
        "$this->__doriaReadinessStatusAddress = null;",
    );
    writeln(output, indent, "$this->__doriaReadinessStatus = null;");
    writeln(output, indent, "$this->__doriaReadinessKey = null;");
}

pub(super) fn emit_registration_key_members(output: &mut String, indent: usize) {
    for line in [
        "private readonly string $__doriaRegistrationIdentityBytes;",
        "public static function __doriaFromRegistration(int $low, int $high): self",
        "{",
        "    $key = new self($low, $high);",
        "    $key->__doriaRegistrationIdentityBytes = 'k' . pack('q2', $low, $high);",
        "    return $key;",
        "}",
        "public function __doriaNativeRegistration(): FFI\\CData",
        "{",
        "    $key = __DoriaNativeIo::get()->new('int64_t[2]');",
        "    $this->__doriaWriteNativeRegistration($key);",
        "    return $key;",
        "}",
        "public function __doriaWriteNativeRegistration(FFI\\CData $key): void",
        "{",
        "    $key[0] = $this->__doriaInlineBits;",
        "    $key[1] = $this->__doriaInlineHighBits;",
        "}",
        "public function __doriaRegistrationIdentity(): string",
        "{",
        "    return $this->__doriaRegistrationIdentityBytes;",
        "}",
    ] {
        writeln(output, indent, line);
    }
}

pub(super) fn emit_set_body(
    intrinsic: crate::compiler_known_stream::ReadinessFunction,
    function: &FunctionDecl,
    output: &mut String,
    indent: usize,
    scopes: &PhpNameScopes,
) {
    use crate::compiler_known_stream::ReadinessFunction;
    match intrinsic {
        ReadinessFunction::Begin => {
            let parameter = &function.params[0].name;
            let plan = scopes.closure_plan.callable_definition(function.span);
            let take = if callable_parameter_uses_cell(plan, 0) {
                format!("__doria_take_cell(${parameter})")
            } else {
                format!("${parameter}")
            };
            let result = php_type(function.return_type.as_ref().unwrap(), scopes);
            let deadline = intrinsic_parameter(function, 1, scopes);
            let cancellation = intrinsic_parameter(function, 2, scopes);
            for line in [format!("$plan = {take};"),
                "try {".into(),
                format!("    return {result}::__doriaFromHostedGroup($plan->__doriaBegin($this->__doriaReadinessSet, $this->__doriaReadinessPendingMembers, {deadline}, {cancellation}));"),
                "} finally {".into(),
                "    if (__doria_cleanup_enabled()) { __doria_drop_value($plan); }".into(),
                "}".into()] {
                writeln(output, indent, &line);
            }
        }
        ReadinessFunction::New => {
            for line in [
                "$runtime = __DoriaNativeIo::get();",
                "$this->__doriaReadinessStatus = $runtime->new('DrIoStatusV1');",
                "$this->__doriaReadinessStatusAddress = FFI::addr($this->__doriaReadinessStatus);",
                "$this->__doriaReadinessKey = $runtime->new('int64_t[2]');",
                "$status = $this->__doriaReadinessStatus;",
                "$owner = $runtime->dr_v1_readiness_set_new(null, $this->__doriaReadinessStatusAddress);",
                "if ($owner === null || FFI::isNull($owner)) {",
                "    __doria_throw_io_facts(__DoriaStdIoIoOperation::Register, __DoriaStdIoIoTarget::__doriaCaseReadinessSet(),",
                "        __doria_native_io_reason($status->reason), $status->has_system_code === 0 ? null : $status->system_code, null, 0, '');",
                "}",
                "$this->__doriaReadinessSet = $owner;",
            ] {
                writeln(output, indent, line);
            }
        }
        ReadinessFunction::Register => {
            let parameter = &function.params[0].name;
            let plan = scopes.closure_plan.callable_definition(function.span);
            let take = if callable_parameter_uses_cell(plan, 0) {
                format!("__doria_take_cell(${parameter})")
            } else {
                format!("${parameter}")
            };
            writeln(output, indent, &format!("$taken = {take};"));
            if !callable_parameter_uses_cell(plan, 0) {
                writeln(output, indent, &format!("${parameter} = null;"));
            }
            let receiver = scopes.specialization.resolve_type(&function.params[0].ty, &scopes.substitutions)
                .expect("checked registration has an exact stream specialization");
            let (getter, getter_plan) = scopes.specialization.stream_accessor(&receiver, crate::compiler_known_stream::StreamOperation::ReadinessSource)
                .expect("checked registration has a readiness-source accessor");
            let getter_arguments = if getter_plan.caller_receiver_home() {
                "__DoriaCell::borrowed($taken)"
            } else {
                ""
            };
            let source = if getter_plan.returns_borrow {
                format!("$taken->{getter}({getter_arguments})->value")
            } else {
                format!("$taken->{getter}({getter_arguments})")
            };
            let key_type = scopes.specialization.class_symbol_for_type(function.return_type.as_ref().unwrap(), &scopes.substitutions)
                .expect("checked registration returns an exact key specialization");
            let pending_read = scopes.specialization.stream_capability(&receiver,
                crate::compiler_known_stream::StreamOperation::PrepareRead);
            let pending_write = scopes.specialization.stream_capability(&receiver,
                crate::compiler_known_stream::StreamOperation::PrepareWrite);
            let admission = if pending_read || pending_write {
                format!("    $code = $runtime->dr_v1_readiness_set_add_host_pending_source($this->__doriaReadinessSet, $source, {}, {}, $nativeKey, $this->__doriaReadinessStatusAddress);",
                    u8::from(pending_read), u8::from(pending_write))
            } else {
                "    $code = $runtime->dr_v1_readiness_set_add_source($this->__doriaReadinessSet, $source, $nativeKey, $this->__doriaReadinessStatusAddress);".into()
            };
            let dispatch = hosted::member_dispatch(&receiver, scopes);
            for line in [
                "try {".to_string(),
                "    $runtime = __DoriaNativeIo::get();".into(),
                "    $status = $this->__doriaReadinessStatus;".into(),
                "    $nativeKey = $this->__doriaReadinessKey;".into(),
                format!("    $source = {source}->__doriaNativeReadinessSource();"),
                admission,
                "    if ($code !== 0 && $code !== 3) { __DoriaNativeIo::fail('protocol'); }".into(),
                "    if ($code === 3) {".into(),
                "        __doria_throw_io_facts(__DoriaStdIoIoOperation::Register, __DoriaStdIoIoTarget::__doriaCaseReadinessSet(),".into(),
                "            __doria_native_io_reason($status->reason), $status->has_system_code === 0 ? null : $status->system_code, null, 0, '');".into(),
                "    }".into(),
                format!("    $key = {key_type}::__doriaFromRegistration($nativeKey[0], $nativeKey[1]);"),
                "    $this->__doriaReadinessMembers[$key->__doriaRegistrationIdentity()] = $taken;".into(),
                format!("    $this->__doriaReadinessPendingMembers[$key->__doriaRegistrationIdentity()] = new __DoriaPendingMember(__DoriaCell::borrowed($this->__doriaReadinessMembers[$key->__doriaRegistrationIdentity()]), {}, {}, {dispatch});",
                    emit_php_string_literal(getter), if getter_plan.caller_receiver_home() { "true" } else { "false" }),
                "    $taken = null;".into(),
                "    return $key;".into(),
                "} finally {".into(),
                "    __doria_drop_value($taken);".into(),
                "}".into(),
            ] {
                writeln(output, indent, &line);
            }
        }
        ReadinessFunction::Unregister
        | ReadinessFunction::GetStream
        | ReadinessFunction::GetWritableStream => {
            let key = intrinsic_parameter(function, 0, scopes);
            let removing = intrinsic == ReadinessFunction::Unregister;
            let symbol = if removing { "remove_source" } else { "get_source" };
            let operation = if removing { "Unregister" } else { "Access" };
            for line in [
                "$runtime = __DoriaNativeIo::get();".to_string(),
                "$status = $this->__doriaReadinessStatus;".into(),
                "$nativeKey = $this->__doriaReadinessKey;".into(),
                format!("{key}->__doriaWriteNativeRegistration($nativeKey);"),
                format!("$code = $runtime->dr_v1_readiness_set_{symbol}($this->__doriaReadinessSet, $nativeKey, $this->__doriaReadinessStatusAddress);"),
                "if ($code !== 0 && $code !== 3) { __DoriaNativeIo::fail('protocol'); }".into(),
                "if ($code === 3) {".into(),
                format!("    __doria_throw_io_facts(__DoriaStdIoIoOperation::{operation}, __DoriaStdIoIoTarget::__doriaCaseReadinessSet(),"),
                "        __doria_native_io_reason($status->reason), $status->has_system_code === 0 ? null : $status->system_code, null, 0, '');".into(),
                "}".into(),
                format!("$identity = {key}->__doriaRegistrationIdentity();"),
                "if (!array_key_exists($identity, $this->__doriaReadinessMembers)) { __DoriaNativeIo::fail('protocol'); }".into(),
                "$returned = $this->__doriaReadinessMembers[$identity];".into(),
            ] {
                writeln(output, indent, &line);
            }
            if removing {
                writeln(output, indent, "unset($this->__doriaReadinessPendingMembers[$identity]);");
                writeln(output, indent, "unset($this->__doriaReadinessMembers[$identity]);");
                writeln(output, indent, "return $returned;");
            } else if scopes.closure_plan.callable_definition(function.span)
                .expect("readiness intrinsic has checked callable transport").returns_borrow
            {
                writeln(output, indent, "return $this->__doriaReadinessPendingMembers[$identity]->receiver;");
            } else {
                writeln(output, indent, "return $returned;");
            }
        }
    }
}

/// Decision 0110 `Child`: an FFI handle to the runtime owner and its pipes.
pub(super) fn has_child_payload(class: &ClassDecl) -> bool {
    class.name == crate::compiler_known_stream::CHILD
        && class.span.source == crate::compiler_known_stream::SOURCE_ID
}

pub(super) fn uses_child_payload(scopes: &PhpNameScopes) -> bool {
    scopes.current_class.as_deref() == Some(crate::compiler_known_stream::CHILD)
}

pub(super) fn emit_child_payload_members(output: &mut String, indent: usize) {
    for line in r#"private ?FFI\CData $__doriaChild;
private array $__doriaPipes = [null, null, null];
// Each pipe's home cell, stable for the child's life, like a set member's.
public function __doriaPipe(int $index, string $class): __DoriaCell
{
    return $this->__doriaPipes[$index] ??= new __DoriaCell(
        $class::__doriaFromFileOwner(__DoriaChildProcess::pipe($this->__doriaChild, $index)));
}"#
    .lines()
    {
        writeln(output, indent, line);
    }
}

pub(super) fn emit_child_drop(output: &mut String, indent: usize) {
    for line in [
        "if ($this->__doriaChild !== null) {",
        "    $child = $this->__doriaChild;",
        "    $this->__doriaChild = null;",
        "    $this->__doriaPipes = [null, null, null];",
        "    __DoriaChildProcess::drop($child);",
        "}",
    ] {
        writeln(output, indent, line);
    }
}

/// `Child` intrinsics: one shared FFI call per operation, as natively.
pub(super) fn emit_process_body(
    intrinsic: crate::compiler_known_stream::ProcessFunction,
    function: &FunctionDecl,
    output: &mut String,
    indent: usize,
    scopes: &PhpNameScopes,
) {
    use crate::compiler_known_stream::ProcessFunction as P;
    let parameter = |index: usize| intrinsic_parameter(function, index, scopes);
    let tag = intrinsic.tag();
    let line = match intrinsic {
        P::Launch => {
            let arguments = (0..8).map(parameter).collect::<Vec<_>>().join(", ");
            format!("return new self(__DoriaChildProcess::launch({arguments}));")
        }
        P::InputPipe | P::OutputPipe | P::ErrorsPipe => {
            let class = php_type(function.return_type.as_ref().unwrap(), scopes);
            let returns_home = scopes
                .closure_plan
                .callable_definition(function.span)
                .expect("pipe loan has checked callable transport")
                .returns_borrow;
            format!(
                "return $this->__doriaPipe({}, {class}::class){};",
                tag - P::InputPipe.tag(),
                if returns_home { "" } else { "->value" }
            )
        }
        P::Id | P::Finish | P::Status | P::Terminate => format!(
            "return __DoriaChildProcess::call($this->__doriaChild, {tag}, [{}]);",
            if intrinsic == P::Finish {
                parameter(0)
            } else {
                String::new()
            }
        ),
        P::WaitUntil => format!(
            "return __DoriaChildProcess::call($this->__doriaChild, {tag}, [{deadline}->__doriaLowWord(), {deadline}->__doriaHighWord()]) === 1;",
            deadline = parameter(0)
        ),
        P::Truncated => format!(
            "return __DoriaChildProcess::call($this->__doriaChild, {tag}, []) === 1;"
        ),
        P::Release => format!("__DoriaChildProcess::call($this->__doriaChild, {tag}, []);"),
        P::CapturedOutput | P::CapturedErrors => format!(
            "return __DoriaChildProcess::bytes($this->__doriaChild, {tag});"
        ),
    };
    writeln(output, indent, &line);
}

pub(super) fn uses_readiness_payload(scopes: &PhpNameScopes) -> bool {
    scopes.current_class.as_deref() == Some(crate::compiler_known_stream::READINESS_SOURCE)
}

pub(super) fn has_request_payload(class: &ClassDecl) -> bool {
    class.span.source == crate::compiler_known_stream::SOURCE_ID
        && crate::compiler_known_stream::is_pending_request(&class.name)
}

pub(super) fn uses_request_payload(scopes: &PhpNameScopes) -> bool {
    scopes
        .current_class
        .as_deref()
        .is_some_and(crate::compiler_known_stream::is_pending_request)
}

pub(super) fn emit_runtime(output: &mut String, reaches_child: bool) {
    let runtime = RUNTIME
        .replace(
            "__DoriaReadOutcome",
            &php_type_symbol("Doria\\Std\\Io\\ReadOutcome"),
        )
        .replace(
            "__DoriaWriteOutcome",
            &php_type_symbol("Doria\\Std\\Io\\WriteOutcome"),
        )
        .replace(
            "__DoriaFileCreation",
            &php_type_symbol("Doria\\Std\\Io\\FileCreation"),
        )
        .replace(
            "__DoriaFileWriteMode",
            &php_type_symbol("Doria\\Std\\Io\\FileWriteMode"),
        )
        .replace(
            "__DoriaSeekOrigin",
            &php_type_symbol("Doria\\Std\\Io\\SeekOrigin"),
        )
        .replace(
            "__DoriaFileLockMode",
            &php_type_symbol("Doria\\Std\\Io\\FileLockMode"),
        )
        .replace(
            "__DoriaReadinessSource",
            &php_type_symbol(crate::compiler_known_stream::READINESS_SOURCE),
        );
    output.push_str(&value_result::bind_native_results(runtime));
    if reaches_child {
        let child = CHILD_RUNTIME.replace(
            "__DoriaChildCleanup",
            &php_type_symbol("Doria\\Std\\Process\\ChildCleanup"),
        );
        output.push_str(&value_result::bind_native_results(child));
    }
    hosted::emit_runtime(output);
}

pub(super) fn emit_payload_members(output: &mut String, indent: usize) {
    writeln(
        output,
        indent,
        "private __DoriaFileOwner $__doriaFileOwner;",
    );
    writeln(
        output,
        indent,
        "public static function __doriaFromFileOwner(__DoriaFileOwner $owner): self",
    );
    writeln(output, indent, "{");
    writeln(output, indent + 1, "return new self($owner);");
    writeln(output, indent, "}");
    // Lock guards hold the file, not its private owner.
    writeln(
        output,
        indent,
        "public function __doriaUnlock(bool $abandon): void",
    );
    writeln(output, indent, "{");
    writeln(
        output,
        indent + 1,
        "$this->__doriaFileOwner->unlock($abandon);",
    );
    writeln(output, indent, "}");
}

pub(super) fn emit_readiness_payload_members(output: &mut String, indent: usize) {
    writeln(
        output,
        indent,
        "private FFI\\CData $__doriaReadinessSource;",
    );
    writeln(
        output,
        indent,
        "public static function __doriaFromReadinessSource(FFI\\CData $source): self",
    );
    writeln(output, indent, "{");
    writeln(output, indent + 1, "return new self($source);");
    writeln(output, indent, "}");
    writeln(
        output,
        indent,
        "public function __doriaNativeReadinessSource(): FFI\\CData",
    );
    writeln(output, indent, "{");
    writeln(output, indent + 1, "return $this->__doriaReadinessSource;");
    writeln(output, indent, "}");
}

pub(super) fn emit_request_payload_members(output: &mut String, indent: usize) {
    writeln(
        output,
        indent,
        "private __DoriaPendingRequest $__doriaPendingRequest;",
    );
    writeln(
        output,
        indent,
        "public static function __doriaFromPendingRequest(__DoriaPendingRequest $request): self",
    );
    writeln(output, indent, "{");
    writeln(output, indent + 1, "return new self($request);");
    writeln(output, indent, "}");
    writeln(
        output,
        indent,
        "public function __doriaNativePendingRequest(): __DoriaPendingRequest",
    );
    writeln(output, indent, "{");
    writeln(output, indent + 1, "return $this->__doriaPendingRequest;");
    writeln(output, indent, "}");
}

pub(super) fn emit_body(
    intrinsic: FileFunction,
    function: &FunctionDecl,
    parameters: &[Option<value_result::Place>],
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    let standard = scopes
        .current_class
        .as_deref()
        .and_then(crate::compiler_known_stream::standard_device);
    match intrinsic {
        FileFunction::StandardInput
        | FileFunction::StandardOutput
        | FileFunction::StandardError => {
            let result = php_type(function.return_type.as_ref().unwrap(), scopes);
            writeln(
                output,
                indent,
                &format!("return {result}::__doriaStandardView();"),
            );
        }
        FileFunction::PrepareRead | FileFunction::PrepareWrite => {
            let request = intrinsic_parameter(function, 0, scopes);
            let region = value_result::intrinsic_parameter(
                function,
                1,
                parameters[1].as_ref(),
                output,
                indent,
                scopes,
            );
            let [storage, offset, length] = region.region_parts().unwrap();
            let reading = intrinsic == FileFunction::PrepareRead;
            let body = if let Some(stream) = standard {
                format!("{request}->__doriaNativePendingRequest()->configureStandard({}, __DoriaByteRegion::__doriaNativeSource({storage}), {offset}, {length});", stream.runtime_tag())
            } else {
                format!("$this->__doriaFileOwner->prepareFields({request}->__doriaNativePendingRequest(), {storage}, {offset}, {length}, {});", if reading { "true" } else { "false" })
            };
            writeln(output, indent, &body);
        }
        FileFunction::CompleteRead | FileFunction::CompleteWrite => {
            let request = intrinsic_parameter(function, 0, scopes);
            let method = if intrinsic == FileFunction::CompleteRead {
                "readOutcome"
            } else {
                "writeOutcome"
            };
            if scopes.value_return.is_some() {
                writeln(
                    output,
                    indent,
                    &format!("{request}->__doriaNativePendingRequest()->{method}Result();"),
                );
                writeln(output, indent, "return;");
            } else {
                writeln(
                    output,
                    indent,
                    &format!("return {request}->__doriaNativePendingRequest()->{method}();"),
                );
            }
        }
        FileFunction::ReadRequestOutcome | FileFunction::WriteRequestOutcome => {
            let method = if intrinsic == FileFunction::ReadRequestOutcome {
                "readOutcome"
            } else {
                "writeOutcome"
            };
            if scopes.value_return.is_some() {
                writeln(
                    output,
                    indent,
                    &format!("$this->__doriaPendingRequest->{method}Result();"),
                );
                writeln(output, indent, "return;");
            } else {
                writeln(
                    output,
                    indent,
                    &format!("return $this->__doriaPendingRequest->{method}();"),
                );
            }
        }
        FileFunction::CompleteReadRequest | FileFunction::CompleteWriteRequest => {
            let outcome = intrinsic_parameter(function, 0, scopes);
            let method = if intrinsic == FileFunction::CompleteReadRequest {
                "completeRead"
            } else {
                "completeWrite"
            };
            writeln(
                output,
                indent,
                &format!("$this->__doriaPendingRequest->{method}({outcome});"),
            );
        }
        FileFunction::AskTerminal => term::emit_operate(output, indent),
        // The same inline carrier as the Term module's one-word values.
        FileFunction::WatchSourceFromBits => writeln(output, indent, "return new self($bits);"),
        FileFunction::UnavailableConstructor if scopes.file_payload => {
            // Only the backend factory calls this private payload constructor.
            // The compiler-owned source constructor remains unavailable.
            writeln(
                output,
                indent,
                "$this->__doriaFileOwner = $__doriaFileOwner;",
            );
        }
        FileFunction::UnavailableConstructor if uses_child_payload(scopes) => {
            writeln(output, indent, "$this->__doriaChild = $__doriaChild;");
        }
        FileFunction::UnavailableConstructor if uses_readiness_payload(scopes) => {
            writeln(
                output,
                indent,
                "$this->__doriaReadinessSource = $__doriaReadinessSource;",
            );
        }
        FileFunction::UnavailableConstructor if uses_request_payload(scopes) => {
            writeln(
                output,
                indent,
                "$this->__doriaPendingRequest = $__doriaPendingRequest;",
            );
        }
        FileFunction::UnavailableConstructor if uses_plan_payload(scopes) => {
            writeln(
                output,
                indent,
                "$this->__doriaTransferPlan = $__doriaTransferPlan;",
            );
            writeln(
                output,
                indent,
                "$this->__doriaTransferOperations = $__doriaTransferOperations;",
            );
        }
        FileFunction::UnavailableConstructor if uses_group_payload(scopes) => {
            writeln(
                output,
                indent,
                "$this->__doriaTransferGroup = $__doriaTransferGroup;",
            );
        }
        FileFunction::UnavailableConstructor => writeln(
            output,
            indent,
            "throw new LogicException('compiler-owned file companion cannot be constructed');",
        ),
        FileFunction::OpenRead | FileFunction::OpenWrite | FileFunction::OpenReadWrite => {
            let result = php_type(function.return_type.as_ref().unwrap(), scopes);
            let path = &function.params[0].name;
            let arguments = match intrinsic {
                FileFunction::OpenRead => format!("0, ${path}"),
                FileFunction::OpenWrite | FileFunction::OpenReadWrite => format!(
                    "{}, ${path}, ${}, ${}",
                    if intrinsic == FileFunction::OpenWrite {
                        1
                    } else {
                        2
                    },
                    function.params[1].name,
                    function.params[2].name,
                ),
                _ => unreachable!(),
            };
            writeln(
                output,
                indent,
                &format!(
                    "return {result}::__doriaFromFileOwner(__DoriaFileOwner::open({arguments}));"
                ),
            );
        }
        FileFunction::Read | FileFunction::Write => {
            let region = value_result::intrinsic_parameter(
                function,
                0,
                parameters[0].as_ref(),
                output,
                indent,
                scopes,
            );
            let [storage, offset, length] = region.region_parts().unwrap();
            assert!(
                scopes.value_return.is_some(),
                "stream reads/writes publish shared fields"
            );
            if let Some(stream) = standard {
                writeln(
                    output,
                    indent,
                    &format!(
                        "__DoriaNativeStandardIo::get()->transferFields({}, {}, {storage}, {offset}, {length});",
                        stream.runtime_tag(),
                        if intrinsic == FileFunction::Read {
                            "true"
                        } else {
                            "false"
                        }
                    ),
                );
            } else {
                writeln(
                    output,
                    indent,
                    &format!(
                        "$this->__doriaFileOwner->{}Fields({storage}, {offset}, {length});",
                        if intrinsic == FileFunction::Read {
                            "read"
                        } else {
                            "write"
                        }
                    ),
                );
            }
            writeln(output, indent, "return;");
        }
        FileFunction::Close => {
            // Intrinsics have typed implementation metadata, not authored
            // BindingIds. Register this transferred receiver with the existing
            // owned-scope cleanup without inventing an authored binding.
            scopes.push();
            let receiver = scopes.fresh_temp("__doria_file_receiver");
            emit_owned_scope(output, indent, scopes, |output, indent, scopes| {
                scopes.own_cell(receiver.clone());
                writeln(
                    output,
                    indent,
                    &format!("${receiver} = new __DoriaCell($this);"),
                );
                writeln(output, indent, "$this->__doriaFileOwner->close();");
            });
            scopes.pop();
        }
        FileFunction::Seek => {
            let offset = intrinsic_parameter(function, 0, scopes);
            let origin = intrinsic_parameter(function, 1, scopes);
            writeln(
                output,
                indent,
                &format!("return $this->__doriaFileOwner->seek({offset}, {origin});"),
            );
        }
        FileFunction::Position | FileFunction::Length => {
            let tag = if intrinsic == FileFunction::Position {
                4
            } else {
                5
            };
            writeln(
                output,
                indent,
                &format!("return $this->__doriaFileOwner->inspect({tag});"),
            );
        }
        FileFunction::SynchronizeData | FileFunction::SynchronizeMetadata => {
            let (tag, label) = if intrinsic == FileFunction::SynchronizeData {
                (1, "SynchronizeData")
            } else {
                (2, "SynchronizeMetadata")
            };
            writeln(
                output,
                indent,
                &format!(
                "$this->__doriaFileOwner->synchronize({tag}, __DoriaStdIoIoOperation::{label});"
            ),
            );
        }
        FileFunction::ReadinessSource => {
            if let Some(stream) = standard {
                let source = php_type_symbol(crate::compiler_known_stream::READINESS_SOURCE);
                writeln(
                    output,
                    indent,
                    &format!(
                        "return __DoriaNativeStandardIo::get()->source({}, {source}::class);",
                        stream.runtime_tag()
                    ),
                );
            } else {
                writeln(
                    output,
                    indent,
                    "return $this->__doriaFileOwner->readinessSource;",
                );
            }
        }
        FileFunction::SetLength => {
            let length = intrinsic_parameter(function, 0, scopes);
            writeln(
                output,
                indent,
                &format!("$this->__doriaFileOwner->setLength({length});"),
            );
        }
        FileFunction::AcquireLock | FileFunction::TryAcquireLock => {
            let mode = intrinsic_parameter(function, 0, scopes);
            let wait = intrinsic == FileFunction::AcquireLock;
            let call = format!("$this->__doriaFileOwner->lock({mode}, {wait})");
            writeln(
                output,
                indent,
                &if wait {
                    format!("{call};")
                } else {
                    format!("return {call};")
                },
            );
        }
        FileFunction::ReleaseLock | FileFunction::AbandonLock => {
            let file = intrinsic_parameter(function, 0, scopes);
            let abandon = intrinsic == FileFunction::AbandonLock;
            writeln(
                output,
                indent,
                &format!("{file}->__doriaUnlock({abandon});"),
            );
        }
        FileFunction::CloseInput => {
            writeln(output, indent, "$this->__doriaFileOwner->closeInput();");
        }
    }
}

fn intrinsic_parameter(function: &FunctionDecl, index: usize, scopes: &PhpNameScopes) -> String {
    let parameter = &function.params[index];
    let plan = scopes.closure_plan.callable_definition(function.span);
    if callable_parameter_uses_cell(plan, index) {
        format!("${}->value", parameter.name)
    } else {
        format!("${}", parameter.name)
    }
}

pub(super) const RUNTIME: &str = r#"
final class __DoriaHostErrors
{
    private readonly FFI $runtime;
    private readonly FFI\CData $slots;
    private readonly int $stride;
    private array $errors;

    public function __construct(int $count)
    {
        $this->runtime = __DoriaNativeIo::get();
        $layout = $this->runtime->new('size_t[4]');
        $this->runtime->dr_v1_host_adapter_error_layout(FFI::addr($layout[0]),
            FFI::addr($layout[1]), FFI::addr($layout[2]), FFI::addr($layout[3]));
        $word = $this->runtime->new('uint64_t');
        if ($count < 0 || $layout[0] < 1 || $layout[1] < 1 ||
            $layout[1] > FFI::alignof($word)) { __DoriaNativeIo::fail('layout'); }
        $this->stride = intdiv($layout[0] + FFI::sizeof($word) - 1, FFI::sizeof($word));
        if ($count > intdiv(PHP_INT_MAX, $this->stride)) { __DoriaNativeIo::fail('layout'); }
        $this->slots = $this->runtime->new('uint64_t[' . max(1, $count * $this->stride) . ']');
        $this->errors = array_fill(0, $count, null);
        for ($index = 0; $index < $count; ++$index) {
            $this->runtime->dr_v1_host_adapter_error_initialize($this->slot($index), $index);
        }
    }

    private function slot(int $index): FFI\CData
    {
        if (!array_key_exists($index, $this->errors)) { __DoriaNativeIo::fail('protocol'); }
        return FFI::addr($this->slots[$index * $this->stride]);
    }

    public function retain(int $index, __DoriaCheckedError $caught, int $bound,
        FFI\CData $terminal): void
    {
        $slot = $this->slot($index);
        if ($this->errors[$index] !== null || $bound < 0 ||
            $this->runtime->dr_v1_host_adapter_error_disposition($slot) !== 0) {
            __DoriaNativeIo::fail('protocol');
        }
        $error = __doria_detach_checked_error($caught);
        $progress = $error->descriptor->hasTransferredByteCount
            ? $error->error()->transferredByteCount : null;
        $known = $progress !== null && $progress >= 0 && $progress <= $bound;
        // Own the real payload before passing its move-only claim into C.
        $this->errors[$index] = $error;
        if ($this->runtime->dr_v1_host_adapter_error_claim($slot, $terminal,
            $known ? 1 : 0, $known ? $progress : 0, $bound) !== 0) {
            __DoriaNativeIo::fail('protocol');
        }
    }

    public function take(int $index): __DoriaCheckedError
    {
        $slot = $this->slot($index);
        if ($this->runtime->dr_v1_host_adapter_error_disposition($slot) !== 3 ||
            $this->errors[$index] === null) { __DoriaNativeIo::fail('protocol'); }
        $error = $this->errors[$index];
        $this->errors[$index] = null;
        $this->acknowledge($slot);
        return $error;
    }

    private function acknowledge(FFI\CData $slot): void
    {
        if ($this->runtime->dr_v1_host_adapter_error_acknowledge($slot) !== 0) {
            __DoriaNativeIo::fail('protocol');
        }
    }

    // Called after a core poll/drop returns, never from a C callback. Existing
    // checked-error cleanup owns the payload's exact-once Doria destruction.
    public function sweep(): void
    {
        foreach ($this->errors as $index => $error) {
            if ($error === null) { continue; }
            $slot = $this->slot($index);
            if ($this->runtime->dr_v1_host_adapter_error_disposition($slot) !== 2) { continue; }
            $error->dropError();
            $this->errors[$index] = null;
            $this->acknowledge($slot);
        }
    }

    public function __destruct()
    {
        if (!__doria_cleanup_enabled()) { return; }
        $this->sweep();
        foreach ($this->errors as $error) {
            // A group must finish shared-core cleanup BEFORE releasing these
            // pinned homes; ordinary PHP destruction cannot retire native I/O.
            if ($error !== null) { __DoriaNativeIo::fail('protocol'); }
        }
    }
}

final class __DoriaPendingRequest
{
    private readonly FFI\CData $tag;
    private readonly FFI\CData $count;
    private readonly FFI\CData $message;
    private readonly FFI\CData $path;
    private readonly FFI\CData $system;
    private readonly FFI\CData $transferred;
    private readonly FFI\CData $meta;
    private readonly FFI\CData $error;

    public function __construct(private FFI\CData $view, private readonly FFI $runtime,
        private readonly __DoriaCell $receiver, private readonly string $getter,
        private readonly bool $getterUsesHome, private readonly FFI\CData $group)
    {
        $this->tag = $runtime->new("uint64_t[1]");
        $this->count = $runtime->new("size_t[1]");
        $this->message = $runtime->new("DrStringV1 *[1]");
        $this->path = $runtime->new("DrStringV1 *[1]");
        $this->system = $runtime->new("int64_t[1]");
        $this->transferred = $runtime->new("size_t[1]");
        $this->meta = $runtime->new("uint64_t[1]");
        $this->error = $runtime->new("DrInterfaceCarrierV1[1]");
    }

    public function bind(FFI\CData $view): void { $this->view = $view; }

    public function configureFile(FFI\CData $file, __DoriaBytes $source, int $offset, int $length): void
    {
        $this->finishConfiguration($this->runtime->dr_v1_host_pending_request_configure_file(null,
            $this->group, $this->view, $file, $this->freshSource(), $source->__doriaNativeBase(),
            count($source), $offset, $length, $this->error,
            $this->message, $this->path, $this->system, $this->transferred, $this->meta));
    }

    public function configureStandard(int $stream, __DoriaBytes $source, int $offset, int $length): void
    {
        $this->finishConfiguration($this->runtime->dr_v1_host_pending_request_configure_standard(null,
            $this->group, $this->view, $stream, $this->freshSource(), $source->__doriaNativeBase(),
            count($source), $offset, $length, $this->error,
            $this->message, $this->path, $this->system, $this->transferred, $this->meta));
    }

    private function freshSource(): FFI\CData
    {
        // Ordinary getter dispatch precedes FFI, preserving its original Error.
        $owner = $this->receiver->value;
        $getter = $this->getter;
        $source = $this->getterUsesHome ? $owner->$getter($this->receiver) : $owner->$getter();
        if ($source instanceof __DoriaCell) { $source = $source->value; }
        return $source->__doriaNativeReadinessSource();
    }

    private function finishConfiguration(int $code): void
    {
        if ($code === 0) { return; }
        // External bindings cannot run native getter callbacks.
        if ($code !== 3) { __DoriaNativeIo::fail('protocol'); }
        $this->throwFailure();
    }

    public function completeRead(__DoriaReadOutcome $outcome): void
    {
        $tag = $outcome->__doriaMatchesCase(0) ? 0 :
            ($outcome->__doriaMatchesCase(1) ? 1 : ($outcome->__doriaMatchesCase(2) ? 2 : 3));
        $this->runtime->dr_v1_pending_request_complete($this->view, $tag,
            $tag === 0 ? $outcome->__doriaPayloadAt(0) : 0);
    }

    public function completeWrite(__DoriaWriteOutcome $outcome): void
    {
        $tag = $outcome->__doriaMatchesCase(0) ? 0 : 1;
        $this->runtime->dr_v1_pending_request_complete($this->view, $tag,
            $tag === 0 ? $outcome->__doriaPayloadAt(0) : 0);
    }

    private function observe(): bool
    {
        $code = $this->runtime->dr_v1_checked_pending_request_outcome(null, $this->view,
            $this->tag, $this->count, $this->message, $this->path,
            $this->system, $this->transferred, $this->meta);
        if ($code === 0) { return true; }
        if ($code === 4) { return false; }
        if ($code !== 1) { throw new LogicException('invalid native request status'); }
        $this->throwFailure();
    }

    private function throwFailure(): never
    {
        $meta = $this->meta[0];
        $path = $this->consumeString($this->path[0]);
        $this->consumeString($this->message[0]);
        // A pending transfer fails on whatever it transfers: a file, a
        // standard stream or a child's pipe.
        __doria_throw_io_facts(__doria_native_io_operation($meta & 255),
            __doria_native_io_target(($meta >> 8) & 255, $path),
            __doria_native_io_reason(($meta >> 16) & 255),
            ($meta & (1 << 32)) === 0 ? null : $this->system[0],
            ($meta & (1 << 34)) === 0 ? null : $this->transferred[0], 0, "");
    }

    private function consumeString(?FFI\CData $string): string
    {
        if ($string === null || FFI::isNull($string)) { return ""; }
        try {
            return FFI::string($this->runtime->dr_v1_string_data($string),
                $this->runtime->dr_v1_string_length($string));
        } finally {
            $this->runtime->dr_v1_string_release($string);
        }
    }

    public function readOutcome(): ?__DoriaReadOutcome
    {
        $this->readOutcomeResult();
        if (!__DoriaValueResult::$present) { return null; }
        $value = __DORIA_READ_BOX__(__DoriaValueResult::$tag, __DoriaValueResult::$field0);
        __DoriaValueResult::clear();
        return $value;
    }

    public function readOutcomeResult(): void
    {
        __DoriaValueResult::clear();
        if (!$this->observe()) { return; }
        match ($this->tag[0]) {
            0 => __DORIA_READ_DATA_RESULT__($this->count[0]),
            1 => __DORIA_READ_BLOCK_RESULT__(),
            2 => __DORIA_READ_EOF_RESULT__(),
            3 => __DORIA_READ_TIMEOUT_RESULT__(),
        };
    }

    public function writeOutcome(): ?__DoriaWriteOutcome
    {
        $this->writeOutcomeResult();
        if (!__DoriaValueResult::$present) { return null; }
        $value = __DORIA_WRITE_BOX__(__DoriaValueResult::$tag, __DoriaValueResult::$field0);
        __DoriaValueResult::clear();
        return $value;
    }

    public function writeOutcomeResult(): void
    {
        __DoriaValueResult::clear();
        if (!$this->observe()) { return; }
        match ($this->tag[0]) {
            0 => __DORIA_WRITE_DATA_RESULT__($this->count[0]),
            1 => __DORIA_WRITE_BLOCK_RESULT__(),
        };
    }
}

// What the native runtime says failed, decoded in one place: the operation,
// and the target with the path or program name the runtime returned for it.
function __doria_native_io_operation(int $code): __DoriaStdIoIoOperation
{
    return __DoriaStdIoIoOperation::cases()[$code]
        ?? __DoriaNativeIo::fail('protocol', "the runtime named unknown I/O operation {$code}");
}

function __doria_native_io_target(int $tag, string $name): __DoriaStdIoIoTarget
{
    return match ($tag) {
        0 => __DoriaStdIoIoTarget::File($name),
        1 => __DoriaStdIoIoTarget::__doriaCaseStandardInput(),
        2 => __DoriaStdIoIoTarget::__doriaCaseStandardOutput(),
        3 => __DoriaStdIoIoTarget::__doriaCaseStandardError(),
        4 => __DoriaStdIoIoTarget::__doriaCaseReadinessSet(),
        5 => __DoriaStdIoIoTarget::__doriaCaseTransferPlan(),
        6 => __DoriaStdIoIoTarget::__doriaCaseTransferGroup(),
        7 => __DoriaStdIoIoTarget::Process($name),
        8 => __DoriaStdIoIoTarget::ProcessInput($name),
        9 => __DoriaStdIoIoTarget::ProcessOutput($name),
        10 => __DoriaStdIoIoTarget::ProcessErrors($name),
        default => __DoriaNativeIo::fail('protocol', "the runtime named unknown I/O target {$tag}"),
    };
}

function __doria_native_io_reason(int $reason): __DoriaStdIoIoErrorReason
{
    return match ($reason) {
        0 => __DoriaStdIoIoErrorReason::NotFound,
        1 => __DoriaStdIoIoErrorReason::PermissionDenied,
        2 => __DoriaStdIoIoErrorReason::InvalidInput,
        3 => __DoriaStdIoIoErrorReason::Interrupted,
        4 => __DoriaStdIoIoErrorReason::ResourceExhausted,
        5 => __DoriaStdIoIoErrorReason::Unsupported,
        6 => __DoriaStdIoIoErrorReason::Closed,
        7 => __DoriaStdIoIoErrorReason::Other,
        default => __DoriaNativeIo::fail('protocol', "the runtime named unknown I/O failure reason {$reason}"),
    };
}

final class __DoriaFileOwner
{
    public readonly __DoriaCell $readinessSource;
    private ?FFI\CData $storageAddress;
    private readonly FFI\CData $statusAddress;

    private function __construct(private ?FFI\CData $storage, private readonly FFI $runtime,
        private readonly FFI\CData $status, private readonly string $path,
        ?FFI\CData $address = null, private readonly int $target = 0)
    {
        // A child's pipe lives in the runtime's child owner: borrow its address.
        $this->storageAddress = $address ?? FFI::addr($storage[0]);
        $this->statusAddress = FFI::addr($status);
        // Cache the non-owning view at file creation, never on a getter or transfer.
        $this->readinessSource = new __DoriaCell(__DoriaReadinessSource::__doriaFromReadinessSource(
            $runtime->dr_v1_file_object_readiness_source($this->storageAddress)));
    }

    public static function open(int $access, string $path,
        __DoriaFileCreation $creation = __DoriaFileCreation::OpenExisting,
        __DoriaFileWriteMode $mode = __DoriaFileWriteMode::Positioned): self
    {
        $runtime = __DoriaNativeIo::get();
        $layout = $runtime->new("size_t[2]");
        $runtime->dr_v1_file_object_layout(FFI::addr($layout[0]), FFI::addr($layout[1]));
        $word = $runtime->new("uint64_t");
        if ($layout[0] < 1 || $layout[1] < 1 || $layout[1] > FFI::alignof($word)) {
            __DoriaNativeIo::fail("layout");
        }
        $words = intdiv($layout[0] + FFI::sizeof($word) - 1, FFI::sizeof($word));
        $storage = $runtime->new("uint64_t[" . $words . "]");
        $status = $runtime->new("DrIoStatusV1");
        $creationTag = match ($creation) {
            __DoriaFileCreation::OpenExisting => 0,
            __DoriaFileCreation::OpenOrCreate => 1,
            __DoriaFileCreation::CreateNew => 2,
            __DoriaFileCreation::TruncateExisting => 3,
            __DoriaFileCreation::CreateOrTruncate => 4,
        };
        $nativePath = $runtime->dr_v1_string_from_utf8($path, strlen($path));
        try {
            $code = $runtime->dr_v1_file_object_open($nativePath, $access,
                $creationTag, $mode === __DoriaFileWriteMode::Append ? 1 : 0,
                FFI::addr($storage[0]), FFI::addr($status));
        } finally {
            $runtime->dr_v1_string_release($nativePath);
        }
        if ($code === 4) { __doria_panic("P1206", 0, 0, null, "", []); }
        if ($code !== 0) {
            self::throwFailure($status, __DoriaStdIoIoOperation::Open, $path);
        }
        return new self($storage, $runtime, $status, $path);
    }

    /** A pipe of a child process; the child owner frees it, never this. */
    public static function fromPipe(FFI\CData $address, int $target, string $program): self
    {
        $runtime = __DoriaNativeIo::get();
        return new self(null, $runtime, $runtime->new("DrIoStatusV1"), $program, $address, $target);
    }

    public function closeInput(): void
    {
        $this->control(11, 0, 0, __DoriaStdIoIoOperation::Close);
    }

    private function native(): FFI\CData
    {
        if ($this->storageAddress === null) {
            throw new LogicException('compiler invariant violated: consumed file was used');
        }
        return $this->storageAddress;
    }

    private static function throwFailure(FFI\CData $status,
        __DoriaStdIoIoOperation $operation, string $path, int $target = 0): void
    {
        $reason = __doria_native_io_reason($status->reason);
        $target = match ($target) {
            8 => __DoriaStdIoIoTarget::ProcessInput($path),
            9 => __DoriaStdIoIoTarget::ProcessOutput($path),
            10 => __DoriaStdIoIoTarget::ProcessErrors($path),
            default => __DoriaStdIoIoTarget::File($path),
        };
        __doria_throw_io_facts($operation, $target, $reason,
            $status->has_system_code === 0 ? null : $status->system_code,
            $status->has_transferred_byte_count === 0 ? null : $status->transferred_byte_count,
            0, "");
    }

    public function read(__DoriaByteRegion $destination): __DoriaReadOutcome
    {
        $this->readResult($destination);
        $value = __DORIA_READ_BOX__(__DoriaValueResult::$tag, __DoriaValueResult::$field0);
        __DoriaValueResult::clear();
        return $value;
    }

    public function readResult(__DoriaByteRegion $destination): void
    {
        $this->readRange($destination->__doriaNativeStorage(true),
            $destination->__doriaRegionOffset(), count($destination));
    }

    public function readFields(WeakReference $storage, int $offset, int $length): void
    {
        $this->readRange(__DoriaByteRegion::__doriaNativeSource($storage), $offset, $length);
    }

    private function readRange(__DoriaBytes $source, int $offset, int $length): void
    {
        __DoriaValueResult::clear();
        $code = $this->runtime->dr_v1_host_file_object_read($this->native(),
            $source->__doriaNativeBase(), count($source), $offset, $length, $this->statusAddress);
        match ($code) {
            0 => __DORIA_READ_DATA_RESULT__($this->status->value),
            1 => __DORIA_READ_BLOCK_RESULT__(),
            2 => __DORIA_READ_EOF_RESULT__(),
            default => self::throwFailure($this->status, __DoriaStdIoIoOperation::Read, $this->path, $this->target),
        };
    }

    public function write(__DoriaByteRegion $source): __DoriaWriteOutcome
    {
        $this->writeResult($source);
        $value = __DORIA_WRITE_BOX__(__DoriaValueResult::$tag, __DoriaValueResult::$field0);
        __DoriaValueResult::clear();
        return $value;
    }

    public function writeResult(__DoriaByteRegion $source): void
    {
        $this->writeRange($source->__doriaNativeStorage(false),
            $source->__doriaRegionOffset(), count($source));
    }

    public function writeFields(WeakReference $storage, int $offset, int $length): void
    {
        $this->writeRange(__DoriaByteRegion::__doriaNativeSource($storage), $offset, $length);
    }

    private function writeRange(__DoriaBytes $source, int $offset, int $length): void
    {
        __DoriaValueResult::clear();
        $code = $this->runtime->dr_v1_host_file_object_write($this->native(),
            $source->__doriaNativeBase(), count($source), $offset, $length, $this->statusAddress);
        match ($code) {
            0 => __DORIA_WRITE_DATA_RESULT__($this->status->value),
            1 => __DORIA_WRITE_BLOCK_RESULT__(),
            default => self::throwFailure($this->status, __DoriaStdIoIoOperation::Write, $this->path, $this->target),
        };
    }

    public function close(): void
    {
        // The native owner relinquishes its sole close obligation. Its retained
        // path remains live until ordinary receiver cleanup drops the payload.
        if ($this->runtime->dr_v1_file_object_close($this->native(), $this->statusAddress) !== 0) {
            self::throwFailure($this->status, __DoriaStdIoIoOperation::Close, $this->path, $this->target);
        }
    }

    public function seek(int $offset, __DoriaSeekOrigin $origin): int
    {
        $tag = match ($origin) {
            __DoriaSeekOrigin::Start => 0,
            __DoriaSeekOrigin::Current => 1,
            __DoriaSeekOrigin::End => 2,
        };
        return $this->control(3, $offset, $tag, __DoriaStdIoIoOperation::Seek);
    }

    public function inspect(int $operation): int
    {
        return $this->control($operation, 0, 0, __DoriaStdIoIoOperation::Access);
    }

    public function synchronize(int $operation, __DoriaStdIoIoOperation $label): void
    {
        $this->control($operation, 0, 0, $label);
    }

    public function setLength(int $length): void
    {
        $this->control(6, $length, 0, __DoriaStdIoIoOperation::Resize);
    }

    public function lock(__DoriaFileLockMode $mode, bool $wait): bool
    {
        $tag = match ($mode) {
            __DoriaFileLockMode::Shared => 0,
            __DoriaFileLockMode::Exclusive => 1,
        };
        return $this->control($wait ? 7 : 8, 0, $tag, __DoriaStdIoIoOperation::Lock) === 1;
    }

    public function unlock(bool $abandon): void
    {
        $this->control($abandon ? 10 : 9, 0, 0, __DoriaStdIoIoOperation::Unlock);
    }

    private function control(int $operation, int $offset, int $origin,
        __DoriaStdIoIoOperation $label): int
    {
        $code = $this->runtime->dr_v1_file_object_control_int($this->native(),
            $operation, $offset, $origin, $this->statusAddress);
        if ($code !== 0) { self::throwFailure($this->status, $label, $this->path, $this->target); }
        return $this->status->value;
    }

    public function prepare(__DoriaPendingRequest $request, __DoriaByteRegion $region, bool $reading): void
    {
        $request->configureFile($this->native(), $region->__doriaNativeStorage($reading),
            $region->__doriaRegionOffset(), count($region));
    }

    public function prepareFields(__DoriaPendingRequest $request, WeakReference $storage,
        int $offset, int $length, bool $reading): void
    {
        $request->configureFile($this->native(), __DoriaByteRegion::__doriaNativeSource($storage), $offset, $length);
    }

    public function drop(): void
    {
        // A pipe (no PHP storage) belongs to its child owner.
        if ($this->storage === null) { return; }
        $storage = $this->storage;
        $address = $this->storageAddress;
        $this->storage = null;
        $this->storageAddress = null;
        // The address view does not own the backing allocation during native drop.
        try { $this->runtime->dr_v1_file_object_drop(null, $address); }
        finally { unset($storage); }
    }
}

"#;

/// Emitted only for programs that reach `Doria\Std\Process\Child`; the
/// stream runtime above serves every program.
const CHILD_RUNTIME: &str = r#"
/** Decision 0110 child processes over the shared runtime's `Child` owner. */
final class __DoriaChildProcess
{
    private static function word(FFI $runtime, ?FFI\CData $pointer): int
    {
        return $pointer === null ? 0 : $runtime->cast('uintptr_t', $pointer)->cdata;
    }

    private static function pointer(FFI $runtime, int $word): FFI\CData
    {
        $cell = $runtime->new('uintptr_t[1]');
        $cell[0] = $word;
        return $runtime->cast('void **', FFI::addr($cell[0]))[0];
    }

    public static function launch(string $program, bool $shell, mixed $arguments, mixed $names,
        mixed $values, mixed $removed, ?string $directory, __DoriaChildCleanup $cleanup): FFI\CData
    {
        $runtime = __DoriaNativeIo::get();
        $strings = [];
        $lists = [];
        $string = function (string $text) use ($runtime, &$strings): int {
            $native = $runtime->dr_v1_string_from_utf8($text, strlen($text));
            $strings[] = $native;
            return self::word($runtime, $native);
        };
        $list = function (mixed $items) use ($runtime, $string, &$lists): int {
            $collection = $runtime->dr_v1_collection_new(0, 0, 0, 8);
            $lists[] = $collection;
            foreach ($items as $item) { $runtime->dr_v1_collection_push($collection, $string($item)); }
            return self::word($runtime, $collection);
        };
        try {
            $words = [$string($program), $shell ? 1 : 0, $list($arguments), $list($names),
                $list($values), $list($removed), $directory === null ? 0 : $string($directory),
                match ($cleanup) {
                    __DoriaChildCleanup::Wait => 0,
                    __DoriaChildCleanup::Terminate => 1,
                    __DoriaChildCleanup::Detach => 2,
                }];
            return self::pointer($runtime, self::call(null, 0, $words));
        } finally {
            foreach ($lists as $collection) { $runtime->dr_v1_collection_free($collection); }
            foreach ($strings as $native) { $runtime->dr_v1_string_release($native); }
        }
    }

    /** The pipe's home is the owner's slot; the file object lives there. */
    public static function pipe(FFI\CData $child, int $index): __DoriaFileOwner
    {
        $runtime = __DoriaNativeIo::get();
        $home = self::pointer($runtime, self::call($child, 1 + $index, []));
        $object = $runtime->cast('void **', $home)[0];
        // The pipe's retained path is the child's program, its error target.
        $name = $runtime->dr_v1_file_object_path($object);
        $program = FFI::string($runtime->dr_v1_string_data($name), $runtime->dr_v1_string_length($name));
        return __DoriaFileOwner::fromPipe($object, 8 + $index, $program);
    }

    public static function bytes(FFI\CData $child, int $operation): __DoriaBytes
    {
        $runtime = __DoriaNativeIo::get();
        return __DoriaBytes::__doriaFromNativeBytes(
            $runtime->cast('DrBytesV1 *', self::pointer($runtime, self::call($child, $operation, []))));
    }

    public static function call(?FFI\CData $child, int $operation, array $words): int
    {
        $runtime = __DoriaNativeIo::get();
        $count = count($words);
        // Signed words: PHP ints carry the same bits as the native u64 words.
        $operands = $runtime->new('int64_t[' . max(1, $count) . ']');
        foreach (array_values($words) as $index => $word) { $operands[$index] = $word; }
        $result = $runtime->new('int64_t[1]');
        $message = $runtime->new('DrStringV1 *[1]');
        $path = $runtime->new('DrStringV1 *[1]');
        $system = $runtime->new('int64_t[1]');
        $transferred = $runtime->new('size_t[1]');
        $meta = $runtime->new('uint64_t[1]');
        $code = $runtime->dr_v1_checked_process(null, $operation, $child,
            FFI::addr($operands[0]), $count, FFI::addr($result[0]), $message, $path, $system,
            $transferred, $meta);
        try {
            if ($code === 0) { return $result[0]; }
            if ($code !== 1) { __DoriaNativeIo::fail('protocol'); }
            $program = FFI::string($runtime->dr_v1_string_data($path[0]),
                $runtime->dr_v1_string_length($path[0]));
            __doria_throw_io_facts(__doria_native_io_operation($meta[0] & 255),
                __doria_native_io_target(($meta[0] >> 8) & 255, $program),
                __doria_native_io_reason(($meta[0] >> 16) & 255),
                ($meta[0] & (1 << 32)) === 0 ? null : $system[0],
                ($meta[0] & (1 << 34)) === 0 ? null : $transferred[0], 0, '');
        } finally {
            foreach ([$message[0], $path[0]] as $string) {
                if ($string !== null && !FFI::isNull($string)) { $runtime->dr_v1_string_release($string); }
            }
        }
    }

    public static function drop(FFI\CData $child): void
    {
        __DoriaNativeIo::get()->dr_v1_child_process_drop(null, $child);
    }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Output};
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn allocation_probe_covers_every_structured_loop_and_its_condition() {
        let script = format!(
            "{}\n{}",
            include_str!("../../tests/fixtures/php_constructor_probe.php")
                .strip_prefix("<?php")
                .unwrap(),
            r#"
foreach ([
    ['while (test(new Guard())) { new Marker(); }', 2],
    ['for ($index = new Guard(); test($index); step($index)) { new Marker(); }', 2],
    ['foreach ([new Guard()] as $item) { new Marker(); }', 2],
    ['do { new Marker(); } while (test(new Guard()));', 2],
    ['while (test(new Guard())) { new Marker(); } new Outside(); for ($index = new Guard(); test($index); step($index)) { new Marker(); }', 4],
    ['while (test(new Guard())) { foreach ([new Guard()] as $item) { new Marker(); } new Marker(); } do { new Marker(); } while (test(new Guard()));', 6],
    ['if (enabled()) { while (test(new Guard())) { new Marker(); } } try { foreach ([new Guard()] as $item) { new Marker(); } } finally { foreach ([new Guard()] as $item) { new Marker(); } }', 6],
] as [$loop, $expected]) {
    $source = '<?php function before() { while (test(new Elsewhere())) { new Elsewhere(); } } function probe() { ' . $loop . ' new Outside(); } function unrelated() { while (test(new Elsewhere())) { new Elsewhere(); } }';
    $tokens = __doria_test_loop_tokens($source, 'probe');
    if (($tokens[T_NEW] ?? 0) !== $expected) { throw new LogicException('loop scope or condition escaped the allocation probe'); }
}
foreach ([['<?php function probe() {}', 'absent', 'missing'],
    ['<?php function probe() { new Outside(); }', 'probe', 'no structured loop']] as [$source, $name, $message]) {
    try {
        __doria_test_loop_tokens($source, $name);
        throw new RuntimeException('allocation probe accepted a missing target');
    } catch (LogicException $error) {
        if (!str_contains($error->getMessage(), $message)) { throw $error; }
    }
}
echo 'loop forms';
"#
        );
        let result = php_execution::Script::new(&script)
            .command(&["-d", "ffi.enable=0"])
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"loop forms");
    }

    #[test]
    fn allocation_probe_counts_explicit_native_address_views_without_retaining_them() {
        let script = format!(
            "{}\n{}",
            include_str!("../../tests/fixtures/php_constructor_probe.php")
                .strip_prefix("<?php")
                .unwrap(),
            r#"
set_error_handler(static function(int $severity, string $message, string $file, int $line): never {
    throw new ErrorException($message, 0, $severity, $file, $line);
});
$ffi = FFI::cdef('');
$bytes = $ffi->new('uint8_t[4]');
foreach ([0, 1, 1000] as $iterations) {
    __DoriaConstructorProbe::reset();
    for ($index = 0; $index < $iterations; ++$index) {
        $view = FFI::addr($bytes[1]);
        __DoriaConstructorProbe::record($view);
        __DoriaConstructorProbe::record($view);
        $view[0] = 73;
        unset($view);
    }
    if (__DoriaConstructorProbe::count(FFI\CData::class) !== $iterations
        || __DoriaConstructorProbe::live() !== 0) {
        throw new LogicException('address views were miscounted or retained');
    }
    if ($iterations !== 0 && $bytes[1] !== 73) {
        throw new LogicException('address observation changed native storage');
    }
}
__DoriaConstructorProbe::reset();
$cached = FFI::addr($bytes[0]);
for ($index = 0; $index < 1000; ++$index) {
    __DoriaConstructorProbe::record($cached);
}
if (__DoriaConstructorProbe::count(FFI\CData::class) !== 1
    || __DoriaConstructorProbe::live() !== 1) {
    throw new LogicException('cached address view was counted repeatedly');
}
unset($cached);
if (__DoriaConstructorProbe::live() !== 0) {
    throw new LogicException('address observer retained a cached view');
}
echo 'native address views';
"#,
        );
        let result = php_execution::Script::new(&script)
            .command(&["-d", "ffi.enable=1", "-d", "memory_limit=64M"])
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"native address views");
    }

    #[test]
    fn borrowed_region_records_preserve_snapshots_and_construct_no_loop_descriptors() {
        let mut php = compile(
            "region-record-transport.doria",
            r#"
use Doria\Std\Io\{ByteRegion, WritableByteRegion};
function window(Bytes $source, int $offset, int $length): ByteRegion {
    return $source->getRegion($offset, $length);
}
function forward<T>(take T $value): T { return $value; }
function optional(?ByteRegion $value): ?ByteRegion { return $value?->getRegion(0, 1); }
function mutate(writable WritableByteRegion $value): void { $value[0] += 1; }
function byteValue(uint8 $value): uint8 { return $value; }
function throughCallback(function(ByteRegion): ByteRegion $call, ByteRegion $value): ByteRegion {
    return $call($value);
}
function regionLoop(int $iterations): int {
    writable Bytes $source = Bytes::allocate(5);
    $source[0] = 3; $source[1] = 5; $source[2] = 7;
    let $pass = fn(ByteRegion $value) => $value;
    let writable $sum = 0;
    foreach (0..<$iterations as int $index) {
        {
            writable ByteRegion $view = window($source, 0, 3);
            let $previous = forward($view);
            let $callbackView = throughCallback($pass, $previous);
            $view = window($source, 1, 2);
            let $part = $view->getRegion(1, 1);
            let $first = optional($previous);
            let $absent = optional(null);
            let $selected = $absent ?? $first;
            if ($previous->length != 3 || $previous[0] != 3 || $view->length != 2
                || $view[0] != 5 || $part[0] != 7 || $selected == null) { panic("region snapshot changed"); }
            if ($selected[0] != 3 || byteValue($previous[0]) != 3 || $callbackView[0] != 3) { panic("selected region changed"); }
        }
        {
            let writable $view = $source->getWritableRegion(4, 1);
            $view[0] = 1;
            mutate($view);
            if ($view[0] != 2) { panic("writable region update changed"); }
        }
        $sum += 20;
    }
    return $sum;
}
function main(): void {}
"#,
        );
        let script = format!(
            r#"
$GLOBALS['__doriaTestTraceRegions'] = true;
$loop = __doria_test_loop_tokens(file_get_contents(__FILE__), '__DORIA_REGION_LOOP__');
foreach ([T_NEW, T_FUNCTION, T_FN] as $token) {{
    if (($loop[$token] ?? 0) !== 0) {{ throw new LogicException('region loop contains an allocating construct'); }}
}}
foreach ([0, 1, 1000] as $iterations) {{
    __DoriaConstructorProbe::reset();
    if ({}($iterations) !== 20 * $iterations) {{ throw new LogicException('region record value changed'); }}
    if (__DoriaConstructorProbe::count('__DoriaByteRegion') !== 0) {{ throw new LogicException('region descriptor boxed in loop'); }}
    foreach ([__DORIA_REGION_CELLS__] as $cell) {{
        if (__DoriaConstructorProbe::count($cell) !== 0) {{ throw new LogicException('region descriptor replaced by a loop cell'); }}
    }}
    if (__DoriaConstructorProbe::live() !== 0) {{ throw new LogicException('region transport retained its byte owner'); }}
}}
$GLOBALS['__doriaTestTraceRegions'] = false;
__DoriaConstructorProbe::reset();
$owner = __DoriaBytes::allocate(1, 0, 0, '');
$boxed = $owner->getRegion(0, 1, 0, 0, '');
if (__DoriaConstructorProbe::count('__DoriaByteRegion') !== 1) {{ throw new LogicException('boxed region control was not observed'); }}
unset($boxed, $owner);
echo "region records passed\n";
"#,
            php_function_name("regionLoop")
        ).replace("__DORIA_REGION_LOOP__", &php_function_name("regionLoop"))
        .replace("__DORIA_REGION_CELLS__", &[
            crate::types::ByteRegionKind::Readonly, crate::types::ByteRegionKind::Writable,
        ].into_iter().map(|kind| emit_php_string_literal(&value_result::storage::cell_name(kind.source_name())))
            .collect::<Vec<_>>().join(", "));
        match &mut php {
            BackendOutput::Text { contents, .. }
            | BackendOutput::TextWithRuntime { contents, .. } => {
                let constructor = "private readonly bool $writable) {}";
                assert_eq!(contents.matches(constructor).count(), 1);
                *contents = contents.replace(
                    constructor,
                    r#"private readonly bool $writable) {
        static $traced = false;
        if (!$traced && ($GLOBALS['__doriaTestTraceRegions'] ?? false)) {
            $traced = true;
            debug_print_backtrace(DEBUG_BACKTRACE_IGNORE_ARGS);
        }
    }"#,
                );
                contents.push_str(&script);
            }
            _ => unreachable!("PHP produces text"),
        }
        let fixture = Fixture::new();
        let result = run_with_constructor_probe(&php, &fixture);
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"region records passed\n");
    }

    #[test]
    fn seek_origin_uses_the_shared_nominal_php_symbol() {
        let mut output = String::new();
        emit_runtime(&mut output, false);
        assert!(!output.contains("__DoriaSeekOrigin"));
        // The child-process runtime is emitted only for programs that reach it.
        assert!(!output.contains("__DoriaChildProcess"));
        let mut with_child = String::new();
        emit_runtime(&mut with_child, true);
        assert!(with_child.contains("final class __DoriaChildProcess"));
        assert!(!with_child.contains("__DoriaChildCleanup"));
        let symbol = php_type_symbol("Doria\\Std\\Io\\SeekOrigin");
        assert!(output.contains(&format!("seek(int $offset, {symbol} $origin)")));
        for case in ["Start", "Current", "End"] {
            assert!(output.contains(&format!("{symbol}::{case}")));
        }
    }

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct Fixture(std::path::PathBuf);

    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "doria-php-files-{}-{}",
                std::process::id(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn run(php: &BackendOutput, fixture: &Fixture) -> Option<Output> {
        let path = fixture.0.join("main.php");
        crate::backend::publish_output(&path, php.clone()).unwrap();
        match Command::new("php")
            .current_dir(&fixture.0)
            .arg(path)
            .output()
        {
            Ok(output) => Some(output),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("PHP unavailable; file compatibility execution skipped");
                None
            }
            Err(error) => panic!("PHP could not start: {error}"),
        }
    }

    fn run_with_constructor_probe(php: &BackendOutput, fixture: &Fixture) -> Output {
        crate::backend::publish_output(&fixture.0.join("main.php"), php.clone()).unwrap();
        std::fs::write(
            fixture.0.join("probe.php"),
            include_str!("../../tests/fixtures/php_constructor_probe.php"),
        )
        .unwrap();
        let runner = fixture.0.join("run-probe.php");
        std::fs::write(
            &runner,
            r#"<?php
require __DIR__ . DIRECTORY_SEPARATOR . 'probe.php';
$source = file_get_contents(__DIR__ . DIRECTORY_SEPARATOR . 'main.php');
$path = __DIR__ . DIRECTORY_SEPARATOR . 'instrumented.php';
file_put_contents($path, __doria_test_instrument_constructors($source));
require $path;
"#,
        )
        .unwrap();
        Command::new("php")
            .current_dir(&fixture.0)
            .arg(runner)
            .output()
            .expect("PHP/FFI is required for measured stream results")
    }

    fn compile(path: &str, source: &str) -> BackendOutput {
        crate::compile_source(path, source, crate::backend::BackendTarget::Php).unwrap()
    }

    fn contents(output: &BackendOutput) -> &str {
        match output {
            BackendOutput::Text { contents, .. }
            | BackendOutput::TextWithRuntime { contents, .. } => contents,
            _ => panic!("PHP must produce text"),
        }
    }

    #[test]
    fn trivial_inline_words_preserve_width_and_wait_decoding_uses_no_array() {
        let mut php = compile(
            "inline-word-transport.doria",
            r#"
use Doria\Std\Time\{Duration, Deadline};
use Doria\Std\Io\{RegistrationKey, ReadableFile, ReadTransferKey, WriteTransferKey,
    WatchKey, WatchSource, StandardInput, StandardOutput, StandardError, TransferGroup};
use Doria\Std\Concurrency\CancellationToken;
function carriers(Duration $duration, Deadline $deadline, RegistrationKey<ReadableFile> $registration,
    ReadTransferKey $read, WriteTransferKey $write, WatchKey $watch, WatchSource $watched,
    StandardInput $input, StandardOutput $output, StandardError $error, CancellationToken $token,
    TransferGroup $group): void {}
function main(): void {}
"#,
        );
        assert!(!contents(&php).contains("__doriaWaitWords"));
        assert!(!contents(&php).contains("__doria_transfer_wait_budget"));
        let script = r#"
function scalarWordBody(ReflectionMethod $method): void {
    $lines = file($method->getFileName());
    $text = implode('', array_slice($lines, $method->getStartLine() - 1,
        $method->getEndLine() - $method->getStartLine() + 1));
    $body = substr($text, strpos($text, '{') + 1);
    foreach (PhpToken::tokenize('<?php ' . $body) as $token) {
        if ($token->is([T_ARRAY, T_NEW, T_CLONE, T_FN, T_FUNCTION, T_ELLIPSIS, ord('[')])
            || $token->text === '&') {
            throw new LogicException('scalar word transport contains an allocating construct');
        }
    }
}
$parameters = (new ReflectionFunction('__DORIA_CARRIERS__'))->getParameters();
$widths = [1, 2, 2, 2, 2, 2, 1, 1, 1, 1];
foreach ($widths as $index => $width) {
    $class = new ReflectionClass($parameters[$index]->getType()->getName());
    foreach (['__doriaLowWord', '__doriaHighWord'] as $name) {
        $method = $class->getMethod($name);
        if ($method->returnsReference() || $method->getNumberOfParameters() !== 0
            || (string) $method->getReturnType() !== 'int') {
            throw new LogicException('inline words lost their scalar contract');
        }
        scalarWordBody($method);
    }
    foreach ([[0, 0], [PHP_INT_MIN, PHP_INT_MAX], [PHP_INT_MAX, PHP_INT_MIN], [-1, -1]] as [$low, $high]) {
        $value = $class->newInstanceWithoutConstructor();
        $class->getConstructor()->invokeArgs($value, $width === 1 ? [$low] : [$low, $high]);
        for ($round = 0; $round < 1000; ++$round) {
            if ($value->__doriaLowWord() !== $low || $value->__doriaHighWord() !== ($width === 1 ? 0 : $high)) {
                throw new LogicException('inline word bits changed');
            }
        }
    }
}
$managed = $parameters[10]->getType()->getName();
if (method_exists($managed, '__doriaLowWord') || method_exists($managed, '__doriaHighWord')) {
    throw new LogicException('managed Copy acquired unretained word transport');
}
scalarWordBody(new ReflectionMethod('__DoriaHostedTransferGroup', 'observeWait'));
echo 'scalar words';
"#
        .replace("__DORIA_CARRIERS__", &php_function_name("carriers"));
        match &mut php {
            BackendOutput::Text { contents, .. }
            | BackendOutput::TextWithRuntime { contents, .. } => contents.push_str(&script),
            _ => unreachable!("PHP produces text"),
        }
        let fixture = Fixture::new();
        let output = run(&php, &fixture).expect("PHP is required for scalar word transport");
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, b"scalar words");
        assert!(output.stderr.is_empty(), "{output:?}");
    }

    #[test]
    fn repeated_native_file_reads_construct_no_outcome_objects_with_a_boxed_control() {
        let mut php = compile(
            "measured-stream-outcomes.doria",
            r#"
use Doria\Std\Io\{File, SeekOrigin, ReadOutcome, ReadableStream};
function erasedReads(writable ReadableStream $file, int $iterations): int {
    let writable $bytes = Bytes::allocate(3);
    let writable $region = $bytes->getWritableRegion(0, 3);
    let writable $index = 0;
    let writable $sum = 0;
    while ($index < $iterations) {
        let $outcome = $file->read($region);
        $sum += match ($outcome) {
            ReadOutcome::Data($count) => $count,
            ReadOutcome::EndOfStream => 0,
            ReadOutcome::WouldBlock => -10000,
            ReadOutcome::TimedOut => -30000,
        };
        $index++;
    }
    return $sum;
}
function hotErasedRead(int $iterations): int {
    let writable $file = File::openRead("input");
    return erasedReads($file, $iterations);
}
function hotRead(int $iterations): int {
    let writable $file = File::openRead("input");
    let writable $bytes = Bytes::allocate(3);
    let writable $region = $bytes->getWritableRegion(0, 3);
    let writable $index = 0;
    let writable $sum = 0;
    while ($index < $iterations) {
        $file->seek(0, SeekOrigin::Start);
        let $outcome = $file->read($region);
        $sum += match ($outcome) {
            ReadOutcome::Data($count) => $count,
            ReadOutcome::WouldBlock => -10000,
            ReadOutcome::EndOfStream => -20000,
            ReadOutcome::TimedOut => -30000,
        };
        $index++;
    }
    return $sum;
}
function main(): void {}
"#,
        );
        let BackendOutput::TextWithRuntime { contents, .. } = &mut php else {
            panic!("measured reads require the verified native runtime");
        };
        let outcome = php_type_symbol("Doria\\Std\\Io\\ReadOutcome");
        let data = php_payload_case_method("Data", true);
        let body = contents
            .split_once(&format!("function {}(", php_function_name("hotRead")))
            .unwrap()
            .1
            .split_once("\n}\n")
            .unwrap()
            .0
            .to_string();
        assert!(
            !body.contains("(function"),
            "native read loop constructs a closure: {body}"
        );
        contents.push_str(&format!(
            r#"
foreach (['{}', '{}'] as $function) {{
    $tokens = __doria_test_loop_tokens(file_get_contents(__FILE__), $function);
    foreach ([T_NEW, T_FUNCTION, T_FN] as $token) {{
        if (($tokens[$token] ?? 0) !== 0) {{ throw new LogicException('read loop contains an allocating construct'); }}
    }}
}}
foreach ([false, true] as $erased) {{
    $baseline = null;
    foreach ([0, 1, 1000] as $iterations) {{
        __DoriaConstructorProbe::reset();
        $actual = $erased ? {}($iterations) : {}($iterations);
        $expected = $erased ? ($iterations === 0 ? 0 : 3) : $iterations * 3;
        if ($actual !== $expected || __DoriaConstructorProbe::count('{outcome}') !== 0) {{
            throw new LogicException('native read loop constructed outcome objects');
        }}
        $counts = (new ReflectionProperty(__DoriaConstructorProbe::class, 'counts'))->getValue();
        ksort($counts);
        if ($baseline !== null && $counts !== $baseline) {{
            throw new LogicException('native read loop constructs per-operation transport objects: '
                . json_encode(['erased' => $erased, 'iterations' => $iterations, 'baseline' => $baseline, 'actual' => $counts], JSON_THROW_ON_ERROR));
        }}
        $baseline = $counts;
    }}
}}
__DoriaConstructorProbe::reset();
for ($index = 0; $index < 1000; ++$index) {{
    $value = {outcome}::{data}(3);
    if ($value->__doriaPayloadAt(0) !== 3) {{
        throw new LogicException('boxed read control changed its count');
    }}
    unset($value);
}}
if (__DoriaConstructorProbe::count('{outcome}') !== 1000
    || __DoriaConstructorProbe::live() !== 0) {{
    throw new LogicException('boxed read control was not counted or released');
}}
echo "measured native reads passed\n";
"#,
            php_function_name("hotRead"),
            php_function_name("erasedReads"),
            php_function_name("hotErasedRead"),
            php_function_name("hotRead"),
        ));
        let fixture = Fixture::new();
        std::fs::write(fixture.0.join("input"), b"ABC").unwrap();
        let result = run_with_constructor_probe(&php, &fixture);
        assert_eq!(result.status.code(), Some(0), "{result:?}\n{body}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"measured native reads passed\n");
    }

    #[test]
    fn repeated_group_completions_reuse_keys_and_flatten_nested_results() {
        let mut php = compile(
            "measured-group-completions.doria",
            r#"
use Doria\Std\Io\{File, FileCreation, FileWriteMode, ReadinessSet, TransferPlan,
    TransferGroup, TransferWaitResult, TransferCompletion, ReadOutcome, WriteOutcome,
    ReadTransferKey, WriteTransferKey, IoError};
use Doria\Std\Io\Wait as PollWait;
function fetch(writable TransferGroup $group, PollWait $wait): TransferWaitResult throws IoError {
    return $group->getNextCompletion($wait);
}
function score(TransferWaitResult $result, ReadTransferKey $read, WriteTransferKey $write): int {
    if ($result == TransferWaitResult::Completed(TransferCompletion::Read($read, ReadOutcome::Data(0)))) { return 1; }
    if ($result == TransferWaitResult::Completed(TransferCompletion::Write($write, WriteOutcome::Accepted(0)))) { return 2; }
    panic("completion changed");
}
function hotCompletions(int $iterations): int {
    writable ReadinessSet $set = new ReadinessSet();
    let $reader = $set->register(File::openRead("input"));
    let $writer = $set->register(File::openReadWrite("output", FileCreation::OpenExisting, FileWriteMode::Positioned));
    writable Bytes $destination = Bytes::allocate(1);
    writable Bytes $source = Bytes::allocate(1);
    let $plan = TransferPlan::read($reader, $destination->getWritableRegion(0, 0))
        ->andWriteTo($writer, $source->getRegion(0, 0));
    let $read = $plan->getReadKey(0);
    let $write = $plan->getWriteKey(1);
    let writable $group = $set->begin($plan);
    let $first = score(fetch($group, PollWait::Immediate), $read, $write);
    let $second = score(fetch($group, PollWait::Immediate), $read, $write);
    if ($first + $second != 3) { panic("initial completion changed"); }
    let writable $index = 0;
    let writable $sum = 0;
    while ($index < $iterations) {
        $group->resubmitRead($read, 0, 0);
        $group->resubmitWrite($write, 0, 0);
        $sum += score(fetch($group, PollWait::Immediate), $read, $write);
        $sum += score(fetch($group, PollWait::Immediate), $read, $write);
        if ($group->observedReadKey != $read && $group->observedWriteKey != $write) { panic("key identity changed"); }
        {
            let $prefix = $group->getReadRegion($read);
            if ($prefix == null || $prefix->length != 0) { panic("empty completed region changed"); }
        }
        $index++;
    }
    if (fetch($group, PollWait::Immediate) != TransferWaitResult::Exhausted
        || $group->observedReadKey != null || $group->observedWriteKey != null) { panic("completion repeated"); }
    return $sum;
}
function scoreWindow(TransferWaitResult $result, ReadTransferKey $read, WriteTransferKey $write,
    bool $initial, int $length): int {
    if ($initial && $result == TransferWaitResult::Completed(TransferCompletion::Read($read, ReadOutcome::Data($length)))) { return 1; }
    if (!$initial && $result == TransferWaitResult::Completed(TransferCompletion::Read($read, ReadOutcome::EndOfStream))) { return 1; }
    if ($result == TransferWaitResult::Completed(TransferCompletion::Write($write, WriteOutcome::Accepted($length)))) { return 2; }
    panic("window completion changed");
}
function hotWindows(int $iterations): int {
    writable ReadinessSet $set = new ReadinessSet();
    let $reader = $set->register(File::openRead("input"));
    let $writer = $set->register(File::openReadWrite("output", FileCreation::OpenExisting, FileWriteMode::Positioned));
    writable Bytes $destination = Bytes::allocate(4);
    writable Bytes $source = Bytes::allocate(4);
    $source[1] = 19; $source[2] = 23; $source[3] = 29;
    let writable $sum = 0;
    {
        let $plan = TransferPlan::read($reader, $destination->getWritableRegion(1, 3))
            ->andWriteTo($writer, $source->getRegion(1, 3));
        let $read = $plan->getReadKey(0);
        let $write = $plan->getWriteKey(1);
        let writable $group = $set->begin($plan);
        let $first = scoreWindow(fetch($group, PollWait::Immediate), $read, $write, true, 3);
        let $second = scoreWindow(fetch($group, PollWait::Immediate), $read, $write, true, 3);
        if ($first + $second != 3) { panic("initial window completion changed"); }
        {
            let $prefix = $group->getReadRegion($read);
            if ($prefix == null || $prefix->length != 3) { panic("initial window prefix changed"); }
            if ($prefix[0] != 97 || $prefix[1] != 98 || $prefix[2] != 99) { panic("initial window bytes changed"); }
        }
        let writable $index = 0;
        let writable $offset = 0;
        while ($index < $iterations) {
            let $length = 3 - $offset;
            $group->resubmitRead($read, $offset, $length);
            $group->resubmitWrite($write, $offset, $length);
            $sum += scoreWindow(fetch($group, PollWait::Immediate), $read, $write, false, $length);
            $sum += scoreWindow(fetch($group, PollWait::Immediate), $read, $write, false, $length);
            {
                let $prefix = $group->getReadRegion($read);
                if ($prefix == null || $prefix->length != 0) { panic("EOF window prefix changed"); }
            }
            if ($offset == 2) { $offset = 0; } else { $offset++; }
            $index++;
        }
        if (fetch($group, PollWait::Immediate) != TransferWaitResult::Exhausted) { panic("window completion repeated"); }
    }
    if ($destination[0] != 0 || $destination[1] != 97 || $destination[2] != 98
        || $destination[3] != 99) { panic("window destination changed"); }
    return $sum;
}
function main(): void {}
"#,
        );
        let mut script = String::from(
            r#"
foreach (['__DORIA_HOT__', '__DORIA_WINDOWS__'] as $function) {
$baseline = null;
$loop = __doria_test_loop_tokens(file_get_contents(__FILE__), $function);
foreach ([T_FUNCTION, T_FN, T_NEW] as $token) {
    if (($loop[$token] ?? 0) !== 0) { throw new LogicException('completion loop contains an allocating construct'); }
}
foreach ([0, 1, 1000] as $iterations) {
    file_put_contents('output', '');
    __DoriaConstructorProbe::reset();
    if ($function($iterations) !== $iterations * 3) { throw new LogicException('completion score changed'); }
    foreach ([__DORIA_RESULTS__] as $type) {
        if (__DoriaConstructorProbe::count($type) !== 0) { throw new LogicException('completion result boxed: ' . $type); }
    }
    $counts = (new ReflectionProperty(__DoriaConstructorProbe::class, 'counts'))->getValue();
    ksort($counts);
    if ($baseline !== null && $counts !== $baseline) {
        throw new LogicException('completion loop constructed transport objects: ' . json_encode($counts));
    }
    $baseline ??= $counts;
    if (__DoriaConstructorProbe::live() !== 0) { throw new LogicException('completion objects retained after group teardown'); }
    if ($function === '__DORIA_WINDOWS__') {
        $expected = "\x13\x17\x1d";
        for ($index = 0; $index < $iterations; ++$index) { $expected .= substr("\x13\x17\x1d", $index % 3); }
        if (file_get_contents('output') !== $expected) { throw new LogicException('shifted write bytes changed'); }
    }
}
}
__DoriaConstructorProbe::reset();
for ($index = 0; $index < 1000; ++$index) {
    $read = __DORIA_READ_KEY__::__doriaFromTransferKey(7, 0);
    $outcome = __DORIA_READ_OUTCOME__::__DORIA_DATA__(0);
    $completion = __DORIA_COMPLETION__::__DORIA_READ__($read, $outcome);
    $result = __DORIA_WAIT_RESULT__::__DORIA_COMPLETED__($completion);
    unset($read, $outcome, $completion, $result);
}
foreach ([__DORIA_BOXED_TYPES__] as $type) {
    if (__DoriaConstructorProbe::count($type) !== 1000) { throw new LogicException('boxed completion control not counted: ' . $type); }
}
if (__DoriaConstructorProbe::live() !== 0) { throw new LogicException('boxed completion control retained'); }
echo "measured group completions passed\n";
"#,
        );
        let results = [
            "Wait",
            "ReadOutcome",
            "WriteOutcome",
            "TransferCompletion",
            "TransferWaitResult",
        ]
        .map(|name| php_type_symbol(&format!("Doria\\Std\\Io\\{name}")));
        let read_key = php_type_symbol("Doria\\Std\\Io\\ReadTransferKey");
        let boxed = [&read_key, &results[1], &results[3], &results[4]];
        for (token, value) in [
            ("__DORIA_HOT__", php_function_name("hotCompletions")),
            ("__DORIA_WINDOWS__", php_function_name("hotWindows")),
            (
                "__DORIA_RESULTS__",
                results
                    .iter()
                    .map(|name| emit_php_string_literal(name))
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            (
                "__DORIA_BOXED_TYPES__",
                boxed
                    .iter()
                    .map(|name| emit_php_string_literal(name))
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            ("__DORIA_READ_KEY__", read_key.clone()),
            ("__DORIA_READ_OUTCOME__", results[1].clone()),
            ("__DORIA_COMPLETION__", results[3].clone()),
            ("__DORIA_WAIT_RESULT__", results[4].clone()),
            ("__DORIA_DATA__", php_payload_case_method("Data", true)),
            ("__DORIA_READ__", php_payload_case_method("Read", true)),
            (
                "__DORIA_COMPLETED__",
                php_payload_case_method("Completed", true),
            ),
        ] {
            script = script.replace(token, &value);
        }
        match &mut php {
            BackendOutput::Text { contents, .. }
            | BackendOutput::TextWithRuntime { contents, .. } => contents.push_str(&script),
            _ => unreachable!("PHP produces text"),
        }
        let fixture = Fixture::new();
        std::fs::write(fixture.0.join("input"), b"abc").unwrap();
        std::fs::write(fixture.0.join("output"), b"").unwrap();
        let result = run_with_constructor_probe(&php, &fixture);
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"measured group completions passed\n");
    }

    #[test]
    fn cached_plan_keys_preserve_foreign_set_setup_errors_and_release_loans() {
        let php = compile(
            "foreign-plan-key-cache.doria",
            r#"
use Doria\Std\Io\{File, ReadinessSet, TransferPlan, ReadOutcome,
    IoError, IoOperation, IoTarget, IoErrorReason};
function main(): void {
    writable ReadinessSet $set = new ReadinessSet();
    writable ReadinessSet $other = new ReadinessSet();
    let $key = $other->register(File::openRead("input"));
    writable Bytes $buffer = Bytes::allocate(1);
    try {
        let $plan = TransferPlan::read($key, $buffer->getWritableRegion(0, 1));
        let $group = $set->begin($plan);
        panic("foreign plan admitted");
    } catch (IoError $error) {
        if ($error->operation != IoOperation::Configure || $error->target != IoTarget::TransferGroup
            || $error->reason != IoErrorReason::InvalidInput || $error->systemCode != null
            || $error->transferredByteCount != null) { panic("setup error changed"); }
    }
    $buffer[0] = 9;
    let writable $file = $other->unregister($key);
    if ($file->read($buffer->getWritableRegion(0, 1)) != ReadOutcome::Data(1)
        || $buffer[0] != 104) { panic("failed setup submitted work or retained storage"); }
    echo "foreign plan preserved\n";
}
"#,
        );
        let fixture = Fixture::new();
        std::fs::write(fixture.0.join("input"), b"hello").unwrap();
        let result = run(&php, &fixture).expect("PHP/FFI is required for plan key validation");
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"foreign plan preserved\n");
    }

    #[test]
    fn trivial_inline_operand_snapshots_preserve_order_and_lazy_nullable_paths_without_closures() {
        let families = [
            ("Duration", "Doria\\Std\\Time\\Duration", 1),
            ("Deadline", "Doria\\Std\\Time\\Deadline", 2),
            (
                "Registration",
                "Doria\\Std\\Io\\RegistrationKey<Doria\\Std\\Io\\ReadableFile>",
                2,
            ),
            ("ReadKey", "Doria\\Std\\Io\\ReadTransferKey", 2),
            ("WriteKey", "Doria\\Std\\Io\\WriteTransferKey", 2),
            ("WatchKey", "Doria\\Std\\Io\\WatchKey", 2),
            ("WatchSource", "Doria\\Std\\Io\\WatchSource", 1),
            ("Input", "Doria\\Std\\Io\\StandardInput", 1),
            ("Output", "Doria\\Std\\Io\\StandardOutput", 1),
            ("Error", "Doria\\Std\\Io\\StandardError", 1),
        ];
        let mut source = String::from("class Trace { static writable int $ticks = 0; }\n");
        for (name, ty, _) in families {
            source.push_str(&format!(r#"
function mark{name}(?{ty} $value, int $digit): ?{ty} {{
    Trace::ticks = Trace::ticks * 10 + $digit;
    return $value;
}}
function check{name}({ty} $value): void {{
    let writable $index = 0;
    while ($index < 1000) {{
        Trace::ticks = 0;
        if (mark{name}($value, 1) != mark{name}($value, 2) || Trace::ticks != 12) {{ panic("inline operand order changed"); }}
        if (mark{name}(null, 3) != null || Trace::ticks != 123) {{ panic("inline nullable comparison changed"); }}
        if (false && mark{name}($value, 9) == null) {{ panic("inline lazy and changed"); }}
        if (!(true || mark{name}($value, 9) == null) || Trace::ticks != 123) {{ panic("inline lazy or changed"); }}
        let $next = mark{name}(null, 4) ?? mark{name}($value, 5);
        if ($next != $value || Trace::ticks != 12345) {{ panic("inline absent coalesce changed"); }}
        let $same = mark{name}($value, 6) ?? mark{name}($value, 9);
        if ($same != $value || Trace::ticks != 123456) {{ panic("inline present coalesce changed"); }}
        $index++;
    }}
}}
"#));
        }
        source.push_str("function main(): void {}\n");
        let mut php = compile("inline-operand-snapshots.doria", &source);
        let mut script = String::from("\n$source = file_get_contents(__FILE__);\n");
        for (name, _, width) in families {
            let function = emit_php_string_literal(&php_function_name(&format!("check{name}")));
            script.push_str(&format!(r#"
$function = {function};
$loop = __doria_test_loop_tokens($source, $function);
foreach ([T_FUNCTION, T_FN, T_NEW] as $token) {{
    if (($loop[$token] ?? 0) !== 0) {{ throw new LogicException('inline operand loop allocates: ' . $function); }}
}}
$class = new ReflectionClass((new ReflectionFunction($function))->getParameters()[0]->getType()->getName());
$value = $class->newInstanceWithoutConstructor();
$class->getConstructor()->invokeArgs($value, {width} === 1 ? [17] : [17, 29]);
__DoriaConstructorProbe::reset();
$function($value);
$counts = (new ReflectionProperty(__DoriaConstructorProbe::class, 'counts'))->getValue();
if ($counts !== [] || __DoriaConstructorProbe::live() !== 0) {{
    throw new LogicException('inline operand transport constructs objects: ' . $function);
}}
unset($value);
"#));
        }
        script.push_str("echo \"inline operand snapshots passed\\n\";\n");
        match &mut php {
            BackendOutput::Text { contents, .. }
            | BackendOutput::TextWithRuntime { contents, .. } => contents.push_str(&script),
            _ => unreachable!("PHP produces text"),
        }
        let fixture = Fixture::new();
        let result = run_with_constructor_probe(&php, &fixture);
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(result.stdout, b"inline operand snapshots passed\n");
    }

    fn run_input(php: BackendOutput, input: &[u8]) -> Output {
        use std::io::Write;
        use std::process::Stdio;
        let fixture = Fixture::new();
        let path = fixture.0.join("main.php");
        crate::backend::publish_output(&path, php).unwrap();
        let mut child = Command::new("php")
            .arg(path)
            .current_dir(&fixture.0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("PHP/FFI is required for standard-device tests");
        child.stdin.take().unwrap().write_all(input).unwrap();
        child.wait_with_output().unwrap()
    }

    #[test]
    fn public_standard_views_share_the_first_line_intrinsic_and_binary_queue() {
        let php = compile(
            "public-standard-io.doria",
            r#"
use Doria\Std\Io\{StandardStreams, ReadableStream, WritableStream, WritableByteRegion,
    ByteRegion, ReadOutcome, WriteOutcome, IoError, InvalidUtf8Error};
function receive<T implements ReadableStream>(writable T $input, writable WritableByteRegion $region): ReadOutcome throws IoError {
    return $input->read($region);
}
function send(writable WritableStream $output, ByteRegion $region): WriteOutcome throws IoError {
    return $output->write($region);
}
function main(): void throws IoError, InvalidUtf8Error {
    echo read_line(">") ?? "missing";
    echo ":";
    let writable $input = StandardStreams::input;
    writable WritableStream $output = StandardStreams::output;
    let writable $error = StandardStreams::error;
    writable Bytes $bytes = Bytes::fromArray([77, 77, 77, 77, 77]);
    if (receive($input, $bytes->getWritableRegion(1, 3)) != ReadOutcome::Data(3)) { panic("read count"); }
    write_stdout_bytes($bytes);
    if (send($output, $bytes->getRegion(1, 3)) != WriteOutcome::Accepted(3)) { panic("write count"); }
    if ($error->write($bytes->getRegion(1, 3)) != WriteOutcome::Accepted(3)) { panic("stderr count"); }
    echo read_line() ?? "missing tail";
    if ($input->read($bytes->getWritableRegion(5, 0)) != ReadOutcome::Data(0)) { panic("empty read"); }
    if ($input->read($bytes->getWritableRegion(0, 1)) != ReadOutcome::EndOfStream) { panic("EOF"); }
    if (send($output, $bytes->getRegion(5, 0)) != WriteOutcome::Accepted(0)) { panic("empty write"); }
    let $source = $input->readinessSource;
    let $sameSource = $input->readinessSource;
}
"#,
        );
        assert!(matches!(&php, BackendOutput::TextWithRuntime { .. }));
        let output = run_input(php, b"line\r\n\xff\0Qtail\n");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b">line:M\xff\0QM\xff\0Qtail");
        assert_eq!(output.stderr, b"\xff\0Q");
    }

    #[test]
    fn byte_intrinsics_adopt_native_storage_and_preserve_checked_errors() {
        let php = compile(
            "native-byte-intrinsics.doria",
            &r#"
use Doria\Std\Io\{IoError, InvalidUtf8Error, IoOperation, IoErrorReason, IoTarget};
function main(): void throws IoError, InvalidUtf8Error {
    echo read_line() ?? "missing";
    writable Bytes $bytes = read_stdin_bytes();
    $bytes[2] = 88;
    write_stdout_bytes($bytes);
    write_file_bytes("bytes.bin", $bytes);
    append_file_bytes("bytes.bin", Bytes::fromArray([89]));
    writable Bytes $file = read_file_bytes("bytes.bin");
    $file[3] = 90;
    write_stderr_bytes($file);
    try { read_file_bytes("bad\0path"); panic("missing path error"); }
    catch (IoError $error) {
        if ($error->operation != IoOperation::Open || $error->reason != IoErrorReason::InvalidInput
            || $error->target != IoTarget::File("bad\0path") || $error->transferredByteCount != null) { panic("file error facts"); }
    }
}
"#.replace("\\0", "\0"),
        );
        assert!(matches!(&php, BackendOutput::TextWithRuntime { .. }));
        let output = run_input(php, b"one\n\xfe\0Z");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"one\xfe\0X");
        assert_eq!(output.stderr, b"\xfe\0XZ");
    }

    #[test]
    fn direct_and_erased_standard_operations_select_native_io_without_byte_intrinsics() {
        for declarations in [
            "let writable $input = StandardStreams::input; let writable $output = StandardStreams::output;",
            "writable ReadableStream $input = StandardStreams::input; writable WritableStream $output = StandardStreams::output;",
        ] {
            let source = format!(r#"
use Doria\Std\Io\{{StandardStreams, ReadableStream, WritableStream, ReadOutcome, WriteOutcome, IoError, InvalidUtf8Error}};
function main(): void throws IoError, InvalidUtf8Error {{
    echo read_line() ?? "missing";
    {declarations}
    writable Bytes $bytes = Bytes::allocate(1);
    if ($input->read($bytes->getWritableRegion(0, 1)) != ReadOutcome::Data(1)) {{ panic("read count"); }}
    if ($output->write($bytes->getRegion(0, 1)) != WriteOutcome::Accepted(1)) {{ panic("write count"); }}
    echo read_line() ?? "missing tail";
}}
"#);
            let php = compile("standard-dependency-selection.doria", &source);
            assert!(matches!(&php, BackendOutput::TextWithRuntime { .. }));
            let output = run_input(php, b"line\nQtail\n");
            assert_eq!(output.status.code(), Some(0), "{declarations}: {output:?}");
            assert_eq!(output.stdout, b"lineQtail");
            assert!(output.stderr.is_empty(), "{declarations}: {output:?}");
        }
    }

    #[test]
    fn standard_primitive_failures_preserve_device_facts_after_native_error_cleanup() {
        let php = compile(
            "standard-closed-input.doria",
            r#"
use Doria\Std\Io\{StandardStreams, ReadOutcome, IoError, IoOperation, IoTarget, IoErrorReason};
function main(): void throws IoError {
    let writable $input = StandardStreams::input;
    writable Bytes $bytes = Bytes::allocate(1);
    try { $input->read($bytes->getWritableRegion(0, 1)); panic("missing closed error"); }
    catch (IoError $error) {
        if ($error->operation != IoOperation::Read || $error->target != IoTarget::StandardInput
            || $error->reason != IoErrorReason::Closed || $error->systemCode == null
            || $error->transferredByteCount != null) { panic("closed facts"); }
    }
    echo "closed";
}
"#,
        );
        let fixture = Fixture::new();
        let path = fixture.0.join("main.php");
        crate::backend::publish_output(&path, php).unwrap();
        let output = Command::new("php")
            .args([
                "-r",
                "require $argv[1]; fclose(STDIN); __DoriaFunction_6d61696e();",
                "--",
            ])
            .arg(path)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"closed");
        assert!(output.stderr.is_empty(), "{output:?}");
    }

    #[test]
    fn native_standard_line_errors_preserve_utf8_facts_and_remaining_input() {
        let php = compile(
            "native-line-error.doria",
            r#"
use Doria\Std\Io\{IoError, InvalidUtf8Error, Utf8InputSource};
function main(): void throws IoError, InvalidUtf8Error {
    try { read_line(); panic("missing UTF-8 error"); }
    catch (InvalidUtf8Error $error) {
        if ($error->source != Utf8InputSource::StandardInput || $error->validByteCount != 0
            || ($error->invalidByteCount ?? -1) != 1) { panic("UTF-8 facts"); }
    }
    write_stdout_bytes(read_stdin_bytes());
}
"#,
        );
        let output = run_input(php, b"\xff\nZ");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"Z");
        assert_eq!(output.stderr, b"");
    }

    #[test]
    fn standard_copy_getters_and_userland_streams_keep_the_ffi_free_transport() {
        let php = compile(
            "pure-php-standard-copy.doria",
            r#"
use Doria\Std\Io\{StandardStreams, ReadableStream, ReadOutcome, WritableByteRegion, IoError};
class Empty implements ReadableStream {
    writable function read(writable WritableByteRegion $destination): ReadOutcome { return ReadOutcome::EndOfStream; }
}
function consume(writable ReadableStream $input, writable WritableByteRegion $region): ReadOutcome throws IoError {
    return $input->read($region);
}
function main(): void throws IoError {
    let $standard = StandardStreams::input;
    writable ReadableStream $input = new Empty();
    writable Bytes $bytes = Bytes::allocate(0);
    if (consume($input, $bytes->getWritableRegion(0, 0)) != ReadOutcome::EndOfStream) { panic("custom stream"); }
}
"#,
        );
        assert!(matches!(&php, BackendOutput::Text { .. }));
        let fixture = Fixture::new();
        let path = fixture.0.join("main.php");
        crate::backend::publish_output(&path, php).unwrap();
        let output = Command::new("php")
            .args(["-d", "ffi.enable=0"])
            .arg(path)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(
            output.stdout.is_empty() && output.stderr.is_empty(),
            "{output:?}"
        );
    }

    #[test]
    fn hosted_standard_pending_requests_share_the_native_controller_and_original_getter_errors() {
        use std::io::Write;
        use std::process::Stdio;

        let mut php = compile(
            "hosted-standard-pending.doria",
            r#"
use Doria\Std\Io\{ReadinessSet, IoError, ReadRequest, WriteRequest, ReadOutcome, WriteOutcome};
function readResult(ReadRequest $request): ?ReadOutcome throws IoError { return $request->outcome; }
function writeResult(WriteRequest $request): ?WriteOutcome throws IoError { return $request->outcome; }
function main(): void { let $set = new ReadinessSet(); }
"#,
        );
        let BackendOutput::TextWithRuntime { contents, .. } = &mut php else {
            panic!("hosted standard requests require the verified shared runtime");
        };
        contents.push_str(
            include_str!("../../tests/fixtures/hosted_standard_pending.php")
                .strip_prefix("<?php\n")
                .unwrap(),
        );
        let fixture = Fixture::new();
        let path = fixture.0.join("main.php");
        crate::backend::publish_output(&path, php).unwrap();
        let mut child = Command::new("php")
            .arg(path)
            .current_dir(&fixture.0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("PHP/FFI is required for hosted standard requests");
        child.stdin.take().unwrap().write_all(b"\xff\0Q").unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"\xff\0Q");
        assert_eq!(output.stderr, b"\xff\0Q");
    }

    #[test]
    fn hosted_error_claims_preserve_real_payload_and_defer_exact_once_cleanup() {
        let mut php = compile(
            "hosted-errors.doria",
            r#"
use Doria\Std\Io\{ReadinessSet, IoError};
class AdapterFailure implements Error {
    static writable int $drops = 0;
    function __construct(string $message) {}
    function __destruct() { self::drops++; }
}
function failAdapter(): void throws AdapterFailure { throw new AdapterFailure("original adapter error"); }
function getDropCount(): int { return AdapterFailure::drops; }
function observeIoError(IoError $error): void {}
function main(): void { let $set = new ReadinessSet(); }
"#,
        );
        let BackendOutput::TextWithRuntime { contents, .. } = &mut php else {
            panic!("hosted claims require the verified shared runtime");
        };
        contents.push_str(&format!(
            "\n$failAdapter = {};\n$getDropCount = {};\n",
            emit_php_string_literal(&php_function_name("failAdapter")),
            emit_php_string_literal(&php_function_name("getDropCount")),
        ));
        contents.push_str(r#"
$runtime = __DoriaNativeIo::get();
$layout = $runtime->new('size_t[4]');
$runtime->dr_v1_host_adapter_error_layout(FFI::addr($layout[0]), FFI::addr($layout[1]),
    FFI::addr($layout[2]), FFI::addr($layout[3]));
$word = $runtime->new('uint64_t');
if ($layout[3] > FFI::alignof($word)) { throw new LogicException('unsupported terminal alignment'); }
$terminal = $runtime->new('uint64_t[' . intdiv($layout[2] + FFI::sizeof($word) - 1, FFI::sizeof($word)) . ']');
$home = FFI::addr($terminal[0]);
$errors = new __DoriaHostErrors(2);
for ($round = 0; $round < 4; ++$round) {
    try { $failAdapter(); }
    catch (__DoriaCheckedError $caught) {
        $original = $caught->error();
        $descriptor = $caught->descriptor;
        $origin = $caught->origin;
        $callable = $original->__doriaErrorCallable();
        $index = $round % 2;
        $errors->retain($index, $caught, 4, $home);
        if ($getDropCount() !== $round) { throw new LogicException('Error dropped during claim'); }
        if ($index === 0) {
            $identity = $runtime->dr_v1_host_adapter_error_return($home);
            if ($identity !== $index) { throw new LogicException('host Error identity changed'); }
            $returned = $errors->take($identity);
            if ($returned->error() !== $original || $returned->descriptor !== $descriptor ||
                $returned->origin !== $origin || $returned->error()->message !== 'original adapter error' ||
                $returned->error()->__doriaErrorCallable() !== $callable) {
                throw new LogicException('original Error was replaced');
            }
            $returned->dropError();
            unset($returned);
        } else {
            $runtime->dr_v1_host_adapter_error_discard($home);
            if ($getDropCount() !== $round) { throw new LogicException('Error cleanup ran inside C'); }
        }
        $errors->sweep();
        $errors->sweep();
        unset($original, $caught);
        if ($getDropCount() !== $round + 1) { throw new LogicException('Error did not drop exactly once'); }
    }
}
foreach ([null, -1, 0, 4, 9] as $progress) {
    try {
        __doria_throw_io_facts(__DoriaStdIoIoOperation::Read,
            __DoriaStdIoIoTarget::File('original path'), __DoriaStdIoIoErrorReason::Other,
            91, $progress, 0, 'adapter-progress');
    } catch (__DoriaCheckedError $caught) {
        $original = $caught->error();
        $errors->retain(1, $caught, 4, $home);
        $identity = $runtime->dr_v1_host_adapter_error_return($home);
        $returned = $errors->take($identity);
        if ($returned->error() !== $original || !$returned->descriptor->hasTransferredByteCount ||
            $returned->error()->transferredByteCount !== $progress ||
            $returned->error()->systemCode !== 91) {
            throw new LogicException('host transport changed original signed progress');
        }
        $returned->dropError();
        unset($returned, $original, $caught);
    }
}
unset($errors);
if ($getDropCount() !== 4) { throw new LogicException('slot teardown dropped a consumed Error'); }
echo "original hosted errors; exact cleanup\n";
"#);
        let fixture = Fixture::new();
        let output = run(&php, &fixture).expect("PHP is required for hosted Error ownership");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"original hosted errors; exact cleanup\n");
        assert!(output.stderr.is_empty(), "{output:?}");
    }

    #[test]
    fn readiness_membership_reuses_owner_scratch_and_original_member_homes() {
        let mut php = compile(
            "membership-storage.doria",
            r#"
use Doria\Std\Io\{ReadinessSet, ReadinessStream, ReadinessSource, RegistrationKey,
    ReadableFile, File};
class Drops { static writable int $count = 0; }
class Ready implements ReadinessStream {
    internal ReadableFile $file;
    writable int $count = 0;
    function __construct(parameter take ReadableFile $file) { $this->file = $file; }
    ReadinessSource $readinessSource { borrowed get => $this->file->readinessSource; }
    function __destruct() { Drops::count++; }
}
function makeSet(): ReadinessSet { return new ReadinessSet(); }
function makeStream(): Ready { return new Ready(File::openRead("input")); }
function admit(writable ReadinessSet $set, take Ready $stream): RegistrationKey<Ready> {
    return $set->register($stream);
}
function readMember(ReadinessSet $set, RegistrationKey<Ready> $key): Ready {
    return $set->getStream($key);
}
function writeMember(writable ReadinessSet $set, RegistrationKey<Ready> $key): Ready {
    return $set->getWritableStream($key);
}
function retire(writable ReadinessSet $set, RegistrationKey<Ready> $key): Ready {
    return $set->unregister($key);
}
function bump(writable Ready $stream): void { $stream->count++; }
function dropCount(): int { return Drops::count; }
function main(): void {}
"#,
        );
        let BackendOutput::TextWithRuntime { contents, .. } = &mut php else {
            panic!("membership storage requires the verified shared runtime");
        };
        contents.push_str("\n$calls = [];\n");
        for name in [
            "makeSet",
            "makeStream",
            "admit",
            "readMember",
            "writeMember",
            "retire",
            "bump",
            "dropCount",
        ] {
            contents.push_str(&format!(
                "$calls['{name}'] = {};\n",
                emit_php_string_literal(&php_function_name(name))
            ));
        }
        contents.push_str(r#"
function membershipCheck(bool $condition, string $message): void {
    if (!$condition) { throw new LogicException($message); }
}
// Invoke the emitted typed boundary without assuming a parameter's cell plan.
function membershipCall(string $function, __DoriaCell ...$homes): mixed {
    $parameters = (new ReflectionFunction($function))->getParameters();
    $arguments = [];
    foreach ($homes as $index => $home) {
        $type = $parameters[$index]->getType();
        membershipCheck($type instanceof ReflectionNamedType, 'missing parameter transport');
        $arguments[] = $type->getName() === '__DoriaCell' ? $home : $home->value;
    }
    return $function(...$arguments);
}
$set = new __DoriaCell($calls['makeSet']());
$other = new __DoriaCell($calls['makeSet']());
$stream = new __DoriaCell($calls['makeStream']());
$foreignStream = new __DoriaCell($calls['makeStream']());
$key = new __DoriaCell(membershipCall($calls['admit'], $set, $stream));
$foreign = new __DoriaCell(membershipCall($calls['admit'], $other, $foreignStream));
$reflection = new ReflectionObject($set->value);
$statusProperty = $reflection->getProperty('__doriaReadinessStatus');
$addressProperty = $reflection->getProperty('__doriaReadinessStatusAddress');
$keyProperty = $reflection->getProperty('__doriaReadinessKey');
$membersProperty = $reflection->getProperty('__doriaReadinessPendingMembers');
$status = $statusProperty->getValue($set->value);
$statusAddress = $addressProperty->getValue($set->value);
$nativeKey = $keyProperty->getValue($set->value);
membershipCheck($status !== $statusProperty->getValue($other->value)
    && $statusAddress !== $addressProperty->getValue($other->value)
    && $nativeKey !== $keyProperty->getValue($other->value), 'sets share operation scratch');
$statusAddress[0]->value = PHP_INT_MAX;
membershipCheck($status->value === PHP_INT_MAX
    && $statusProperty->getValue($other->value)->value !== PHP_INT_MAX, 'status address lost its owner');
$member = $membersProperty->getValue($set->value)[$key->value->__doriaRegistrationIdentity()]->receiver;
$sourceLines = file(__FILE__);
$lookupBodies = 0;
foreach ($reflection->getMethods() as $method) {
    $body = implode('', array_slice($sourceLines, $method->getStartLine() - 1,
        $method->getEndLine() - $method->getStartLine() + 1));
    if (!str_contains($body, '->dr_v1_readiness_set_get_source(')
        && !str_contains($body, '->dr_v1_readiness_set_remove_source(')) { continue; }
    $lookupBodies++;
    foreach (['$runtime->new(', 'FFI::addr(', 'new __DoriaCell(', '->__doriaNativeRegistration()'] as $allocation) {
        membershipCheck(!str_contains($body, $allocation), 'lookup constructs fresh transport');
    }
}
membershipCheck($lookupBodies === 3, 'membership entry points were not checked');
for ($round = 0; $round < 1000; ++$round) {
    $readonly = membershipCall($calls['readMember'], $set, $key);
    $writable = membershipCall($calls['writeMember'], $set, $key);
    membershipCheck($readonly === $member && $writable === $member, 'member home was rebuilt');
    membershipCall($calls['bump'], $writable);
    membershipCheck($statusProperty->getValue($set->value) === $status
        && $addressProperty->getValue($set->value) === $statusAddress
        && $keyProperty->getValue($set->value) === $nativeKey, 'operation scratch was replaced');
}
membershipCheck($member->value->count === 1000, 'writable access did not update the original owner');
foreach (['readMember', 'writeMember', 'retire'] as $operation) {
    try {
        membershipCall($calls[$operation], $set, $foreign);
        throw new LogicException('foreign registration was accepted');
    } catch (__DoriaCheckedError $caught) {
        $error = $caught->error();
        membershipCheck($error->operation->name === ($operation === 'retire' ? 'Unregister' : 'Access')
            && $error->systemCode === null, 'membership failure facts changed');
        $caught->dropError();
        unset($error, $caught);
    }
    membershipCheck(membershipCall($calls['readMember'], $set, $key) === $member,
        'rejected operation changed the member');
}
$returned = membershipCall($calls['retire'], $set, $key);
membershipCheck($returned === $member->value && $returned->count === 1000, 'removal replaced the owner');
__doria_drop_value($returned);
$replacement = new __DoriaCell($calls['makeStream']());
$next = new __DoriaCell(membershipCall($calls['admit'], $set, $replacement));
$nextMember = membershipCall($calls['readMember'], $set, $next);
membershipCheck($nextMember !== $member && $nextMember->value->count === 0, 'registration reused a stale home');
foreach (['readMember', 'writeMember', 'retire'] as $operation) {
    try {
        membershipCall($calls[$operation], $set, $key);
        throw new LogicException('stale registration was accepted');
    } catch (__DoriaCheckedError $caught) { $caught->dropError(); unset($caught); }
}
membershipCheck($statusProperty->getValue($set->value) === $status
    && $addressProperty->getValue($set->value) === $statusAddress
    && $keyProperty->getValue($set->value) === $nativeKey, 'registration replaced owner scratch');
membershipCheck(membershipCall($calls['readMember'], $set, $next) === $nextMember, 'stale rejection corrupted reuse');
$retiredSet = $set->value;
__doria_drop_value($set->value);
membershipCheck($statusProperty->getValue($retiredSet) === null
    && $addressProperty->getValue($retiredSet) === null
    && $keyProperty->getValue($retiredSet) === null, 'set teardown retained operation scratch');
__doria_drop_value($retiredSet);
__doria_drop_value($other->value);
membershipCheck($calls['dropCount']() === 3, 'registered owners did not drop exactly once');
echo "stable membership scratch; original borrowed homes; exact cleanup\n";
"#);
        let fixture = Fixture::new();
        std::fs::write(fixture.0.join("input"), b"ABC").unwrap();
        let output =
            run(&php, &fixture).expect("PHP is required for membership storage verification");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            output.stdout,
            b"stable membership scratch; original borrowed homes; exact cleanup\n"
        );
        assert!(output.stderr.is_empty(), "{output:?}");
    }

    #[test]
    fn readiness_membership_preserves_the_durable_borrowed_cursor_and_cleanup() {
        let php = compile(
            "membership.doria",
            include_str!("../../../../examples/native/main_readiness_membership.doria"),
        );
        let fixture = Fixture::new();
        assert!(!contents(&php)
            .contains("$code = $runtime->dr_v1_readiness_set_add_host_pending_source("));
        std::fs::write(fixture.0.join("input"), b"ABC").unwrap();
        let output = run(&php, &fixture).expect("PHP is required for readiness membership parity");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            output.stdout,
            include_bytes!(
                "../../tests/fixtures/native_io/main_readiness_membership/expected_stdout"
            )
        );
        assert!(output.stderr.is_empty(), "{output:?}");
    }

    #[test]
    fn pending_membership_uses_nominal_direction_without_native_callbacks() {
        let php = compile(
            "pending-membership.doria",
            r#"
use Doria\Std\Io\{ReadinessSet, File, FileCreation, FileWriteMode, IoError};
function main(): void throws IoError {
    writable ReadinessSet $set = new ReadinessSet();
    let $read = $set->register(File::openRead("input"));
    let $write = $set->register(File::openWrite("output", FileCreation::CreateNew, FileWriteMode::Positioned));
    let $duplex = $set->register(File::openReadWrite("input", FileCreation::OpenExisting, FileWriteMode::Positioned));
    let $reader = $set->unregister($read);
    let $writer = $set->unregister($write);
    let $both = $set->unregister($duplex);
    echo "pending members returned\n";
}
"#,
        );
        for direction in ["1, 0", "0, 1", "1, 1"] {
            assert!(contents(&php).contains(&format!(
                "$code = $runtime->dr_v1_readiness_set_add_host_pending_source($this->__doriaReadinessSet, $source, {direction}, $nativeKey"
            )));
        }
        let fixture = Fixture::new();
        std::fs::write(fixture.0.join("input"), b"ABC").unwrap();
        let output = run(&php, &fixture).expect("PHP is required for pending admission parity");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"pending members returned\n");
        assert!(output.stderr.is_empty(), "{output:?}");
    }

    #[test]
    fn pending_completion_dispatch_uses_checked_field_result_entries() {
        let mut php = compile(
            "pending-field-results.doria",
            r#"
use Doria\Std\Io\{ReadinessSet, File, FileCreation, FileWriteMode};
function members(): ReadinessSet {
    let writable $set = new ReadinessSet();
    $set->register(File::openRead("input"));
    $set->register(File::openWrite("output", FileCreation::CreateNew, FileWriteMode::Positioned));
    $set->register(File::openReadWrite("input", FileCreation::OpenExisting, FileWriteMode::Positioned));
    return $set;
}
function main(): void {}
"#,
        );
        let BackendOutput::TextWithRuntime { contents, .. } = &mut php else {
            panic!("pending completion requires the verified native runtime");
        };
        contents.push_str(&format!("\n$set = {}();\n", php_function_name("members")));
        contents.push_str(
            r#"
try {
    $members = (new ReflectionProperty($set, '__doriaReadinessPendingMembers'))->getValue($set);
    $completions = 0;
    $preparations = 0;
    foreach ($members as $member) {
        foreach (['prepareRead', 'prepareWrite', 'completeRead', 'completeWrite'] as $operation) {
            $method = (new ReflectionProperty($member, $operation))->getValue($member);
            if ($method === null) { continue; }
            [$name, $flags] = $method;
            $completion = str_starts_with($operation, 'complete');
            if ((($flags & 8) !== 0) !== $completion) {
                throw new LogicException('checked completion mode was not preserved');
            }
            if (($flags & 16) === 0) { throw new LogicException('pending operation lost scalar region arguments'); }
            $entry = new ReflectionMethod($member->receiver->value, $name);
            $prefix = $completion ? '__doriaFieldArgumentsResult' : '__doriaFieldArguments';
            $region = 1 + (($flags & 1) !== 0 ? 1 : 0);
            $parameters = $entry->getParameters();
            if (!str_starts_with($name, $prefix)
                || $entry->getReturnType()?->getName() !== 'void'
                || count($parameters) !== $region + 5
                || $parameters[$region]->getType()?->getName() !== 'int'
                || $parameters[$region + 1]->getType()?->getName() !== 'bool') {
                throw new LogicException('pending entry disagrees with its checked argument/result transport');
            }
            if ($completion) {
                ++$completions;
            } else {
                ++$preparations;
            }
        }
    }
    if ($completions !== 4 || $preparations !== 4) { throw new LogicException('directional pending entry missing'); }
} finally {
    __doria_drop_value($set);
}
echo "checked field completion entries\n";
"#,
        );
        let fixture = Fixture::new();
        std::fs::write(fixture.0.join("input"), b"ABC").unwrap();
        let output = run(&php, &fixture).expect("PHP/FFI is required for completion dispatch");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"checked field completion entries\n");
        assert!(output.stderr.is_empty(), "{output:?}");
    }

    #[test]
    fn readiness_registration_keys_preserve_both_words_and_generic_identity() {
        let mut php = compile(
            "key-transport.doria",
            r#"
use Doria\Std\Io\{ReadinessSet, RegistrationKey, ReadableFile, WritableFile};
function observe(?RegistrationKey<ReadableFile> $read, ?RegistrationKey<WritableFile> $write): void {}
function main(): void { let $set = new ReadinessSet(); observe(null, null); }
"#,
        );
        let BackendOutput::TextWithRuntime { contents, .. } = &mut php else {
            panic!("readiness keys use the verified native runtime");
        };
        contents.push_str(r#"
$types = array_values(array_filter(get_declared_classes(),
    static fn(string $type): bool => method_exists($type, '__doriaFromRegistration')));
if (count($types) !== 2) { throw new LogicException('key specializations were erased'); }
$values = [[0, 0], [PHP_INT_MIN, PHP_INT_MAX], [-1, -1], [PHP_INT_MAX, PHP_INT_MIN], [1, 2], [1, 3]];
$scratch = __DoriaNativeIo::get()->new('int64_t[2]');
$sourceLines = file(__FILE__);
foreach ($types as $type) {
    $identityProperty = (new ReflectionClass($type))->getProperty('__doriaRegistrationIdentityBytes');
    if (!$identityProperty->isReadOnly()) { throw new LogicException('registration identity is mutable'); }
    $method = new ReflectionMethod($type, '__doriaRegistrationIdentity');
    $body = implode('', array_slice($sourceLines, $method->getStartLine() - 1,
        $method->getEndLine() - $method->getStartLine() + 1));
    if (str_contains($body, 'pack(') || !str_contains($body, 'return $this->__doriaRegistrationIdentityBytes;')) {
        throw new LogicException('registration lookup rebuilds identity bytes');
    }
    $identities = [];
    foreach ($values as [$low, $high]) {
        $key = $type::__doriaFromRegistration($low, $high);
        $native = $key->__doriaNativeRegistration();
        $key->__doriaWriteNativeRegistration($scratch);
        if ($native[0] !== $low || $native[1] !== $high || $scratch[0] !== $low || $scratch[1] !== $high ||
            !$key->__doriaEquals($type::__doriaFromRegistration($low, $high))) {
            throw new LogicException('registration key words were changed');
        }
        $identity = $key->__doriaRegistrationIdentity();
        if ($identity !== 'k' . pack('q2', $low, $high) || $identity !== $identityProperty->getValue($key)) {
            throw new LogicException('prepared registration identity changed');
        }
        $identities[] = $identity;
    }
    if (count(array_unique($identities)) !== count($values)) {
        throw new LogicException('registration identities collided');
    }
}
if ($types[0]::__doriaFromRegistration(-1, PHP_INT_MIN)->__doriaEquals(
    $types[1]::__doriaFromRegistration(-1, PHP_INT_MIN))) {
    throw new LogicException('different registration types compare equal');
}
echo "exact keys\n";
"#);
        let fixture = Fixture::new();
        let output = run(&php, &fixture).expect("PHP is required for readiness key transport");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"exact keys\n");
        assert!(output.stderr.is_empty(), "{output:?}");
    }

    #[test]
    fn file_transfers_reuse_owner_address_views_and_retire_them_on_drop() {
        let mut php = compile(
            "file-address-views.doria",
            r#"
use Doria\Std\Io\{File, FileCreation, FileWriteMode, SeekOrigin};
function main(): void {
    let writable $file = File::openReadWrite("file.bin", FileCreation::OpenExisting, FileWriteMode::Positioned);
    writable Bytes $bytes = Bytes::allocate(2);
    $file->read($bytes->getWritableRegion(0, 2));
    $file->seek(0, SeekOrigin::Start);
    $file->write($bytes->getRegion(0, 2));
    $file->close();
}
"#,
        );
        let BackendOutput::TextWithRuntime { contents, .. } = &mut php else {
            panic!("file address views require native storage");
        };
        assert!(
            !contents.contains("FFI::addr($this->values[$")
                && !contents.contains("FFI::addr($source->values["),
            "a byte operation formed an interior address view"
        );
        contents.push_str(&format!(
            "\n$start = {}::Start;\n",
            php_type_symbol("Doria\\Std\\Io\\SeekOrigin"),
        ));
        contents.push_str(
            r#"
function addressCheck(bool $condition, string $message): void {
    if (!$condition) { throw new LogicException($message); }
}
$source = file(__FILE__);
foreach (['native', 'readResult', 'readFields', 'readRange', 'writeResult', 'writeFields', 'writeRange',
    'close', 'control', 'prepare', 'prepareFields', 'drop'] as $method) {
    $reflection = new ReflectionMethod(__DoriaFileOwner::class, $method);
    $body = implode('', array_slice($source, $reflection->getStartLine() - 1,
        $reflection->getEndLine() - $reflection->getStartLine() + 1));
    addressCheck(!str_contains($body, 'FFI::addr(') && !str_contains($body, '->new('),
        'file operation rebuilt an address or buffer: ' . $method);
}
$storageProperty = new ReflectionProperty(__DoriaFileOwner::class, 'storage');
$addressProperty = new ReflectionProperty(__DoriaFileOwner::class, 'storageAddress');
$statusProperty = new ReflectionProperty(__DoriaFileOwner::class, 'statusAddress');
addressCheck($statusProperty->isReadOnly(), 'status address is replaceable');
$first = __DoriaFileOwner::open(2, 'file.bin');
$second = __DoriaFileOwner::open(2, 'file.bin');
$address = $addressProperty->getValue($first);
$status = $statusProperty->getValue($first);
$backing = WeakReference::create($storageProperty->getValue($first));
addressCheck($address !== $addressProperty->getValue($second)
    && $status !== $statusProperty->getValue($second), 'file owners share address views');
$bytes = __DoriaBytes::fromArray([65, 66], 0, 0, '');
$write = $bytes->getRegion(0, 2, 0, 0, '');
$read = $bytes->getWritableRegion(0, 2, 0, 0, '');
for ($iteration = 0; $iteration < 1000; ++$iteration) {
    addressCheck($first->seek(0, $start) === 0, 'seek failed');
    $first->writeResult($write);
    addressCheck(__DoriaValueResult::$tag === 0 && __DoriaValueResult::$field0 === 2, 'write lost progress');
    $first->seek(0, $start);
    $bytes->set(0, 0, 0, 0, '');
    $bytes->set(1, 0, 0, 0, '');
    $first->readResult($read);
    addressCheck(__DoriaValueResult::$tag === 0 && __DoriaValueResult::$field0 === 2, 'read lost progress');
    addressCheck($bytes->get(0, 0, 0, '') === 65 && $bytes->get(1, 0, 0, '') === 66, 'read selected different storage');
    addressCheck($second->inspect(5) === 4 && $status[0]->value === 2, 'file owners share status storage');
    addressCheck($addressProperty->getValue($first) === $address
        && $statusProperty->getValue($first) === $status, 'file address views were rebuilt');
}
$wide = __DoriaBytes::fromArray([7, 65, 66, 9], 0, 0, '');
$storage = $wide->getRegion(0, 4, 0, 0, '')->__doriaRegionStorage();
for ($iteration = 0; $iteration < 1000; ++$iteration) {
    $first->seek(0, $start);
    $first->writeFields($storage, 1, 2);
    addressCheck(__DoriaValueResult::$tag === 0 && __DoriaValueResult::$field0 === 2, 'interior write lost progress');
    $first->seek(0, $start);
    $wide->set(1, 0, 0, 0, '');
    $wide->set(2, 0, 0, 0, '');
    $first->readFields($storage, 1, 2);
    addressCheck(__DoriaValueResult::$tag === 0 && __DoriaValueResult::$field0 === 2, 'interior read lost progress');
    addressCheck($wide->toArray() === [7, 65, 66, 9], 'interior transfer selected different bytes');
}
$copy = __DoriaBytes::fromArray([1, 2, 3, 4, 5, 6], 0, 0, '');
$all = $copy->getWritableRegion(0, 6, 0, 0, '');
$all->copyWithin(0, 5, 1, 0, 0, '');
addressCheck($copy->toArray() === [1, 1, 2, 3, 4, 5], 'overlapping copy to a later offset');
$all->copyWithin(1, 5, 0, 0, 0, '');
addressCheck($copy->toArray() === [1, 2, 3, 4, 5, 5], 'overlapping copy to an earlier offset');
$other = __DoriaBytes::fromArray([9, 8, 7], 0, 0, '');
$copied = $copy->getWritableRegion(4, 2, 0, 0, '')->copyFrom($other->getRegion(1, 2, 0, 0, ''), 0, 0, '');
addressCheck($copied === 2 && $copy->toArray() === [1, 2, 3, 4, 8, 7], 'copy between owners');
try {
    $first->seek(-1, $start);
    throw new LogicException('invalid absolute seek succeeded');
} catch (__DoriaCheckedError) {}
addressCheck($addressProperty->getValue($first) === $address
    && $statusProperty->getValue($first) === $status, 'checked failure replaced address views');
addressCheck($first->seek(0, $start) === 0 && $first->inspect(5) === 4, 'checked failure retired the file');
$first->close();
unset($address, $status);
$first->drop();
$first->drop();
addressCheck($storageProperty->getValue($first) === null
    && $addressProperty->getValue($first) === null && $backing->get() === null, 'drop retained file storage');
try {
    $first->inspect(5);
    throw new RuntimeException('consumed file remained usable');
} catch (LogicException $error) {
    addressCheck($error->getMessage() === 'compiler invariant violated: consumed file was used', 'consumed-file check changed');
}
addressCheck($second->seek(0, $start) === 0 && $second->inspect(5) === 4, 'dropping one file retired another');
$second->drop();
echo "file address views\n";
"#,
        );
        let fixture = Fixture::new();
        std::fs::write(fixture.0.join("file.bin"), b"ABCD").unwrap();
        let output = run(&php, &fixture).expect("PHP/FFI is required for file address reuse");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"file address views\n");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert_eq!(std::fs::read(fixture.0.join("file.bin")).unwrap(), b"ABCD");
    }

    #[test]
    fn readable_file_uses_the_durable_nominal_stream_fixture() {
        let source = include_str!("../../../../examples/native/main_stream_readable_file.doria");
        let php = compile("files.doria", source);
        let fixture = Fixture::new();
        let Some(output) = run(&php, &fixture) else {
            return;
        };
        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert_eq!(
            output.stdout,
            include_bytes!(
                "../../tests/fixtures/native_io/main_stream_readable_file/expected_stdout"
            ),
        );
        assert_eq!(
            std::fs::read(fixture.0.join("stream-read.txt")).unwrap(),
            b"ABC\n",
        );
    }

    #[test]
    fn binary_reads_reuse_regions_preserve_edges_and_distinguish_empty_from_eof() {
        let php = compile(
            "binary-files.doria",
            r#"
use Doria\Std\Io\{File, ReadOutcome, IoError};
function main(): void throws IoError {
    let writable $file = File::openRead("binary.dat");
    writable Bytes $bytes = Bytes::allocate(5);
    {
        let writable $region = $bytes->getWritableRegion(1, 3);
        echo match ($file->read($region)) {
            ReadOutcome::Data($count) => "data {$count}\n", default => "wrong\n",
        };
    }
    echo "{$bytes[0]} {$bytes[1]} {$bytes[2]} {$bytes[3]} {$bytes[4]}\n";
    {
        let writable $empty = $bytes->getWritableRegion(5, 0);
        echo match ($file->read($empty)) {
            ReadOutcome::Data($count) => "empty {$count}\n", default => "wrong\n",
        };
    }
    {
        let writable $region = $bytes->getWritableRegion(1, 3);
        echo match ($file->read($region)) {
            ReadOutcome::Data($count) => "partial {$count}\n", default => "wrong\n",
        };
        echo match ($file->read($region)) {
            ReadOutcome::EndOfStream => "end\n", default => "wrong\n",
        };
    }
    echo "{$bytes[0]} {$bytes[1]} {$bytes[2]} {$bytes[3]} {$bytes[4]}\n";
    $file->close();
}
"#,
        );
        let fixture = Fixture::new();
        std::fs::write(fixture.0.join("binary.dat"), [0, 255, 10, 77]).unwrap();
        let Some(output) = run(&php, &fixture) else {
            return;
        };
        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert_eq!(
            output.stdout,
            b"data 3\n0 0 255 10 0\nempty 0\npartial 1\nend\n0 77 255 10 0\n",
        );
    }

    #[test]
    fn owned_factory_errors_keep_the_first_authored_origin_when_forwarded() {
        for source in [
            "use Doria\\Std\\Io\\{File, IoError};\nfunction main(): void throws IoError {\n    let $file = File::openRead(\"missing.dat\");\n}\n",
            "use Doria\\Std\\Io\\{File, ReadableFile, IoError};\nfunction openFile(): ReadableFile throws IoError {\n    return File::openRead(\"missing.dat\");\n}\nfunction main(): void throws IoError { let $file = openFile(); }\n",
        ] {
            let php = compile("origin-files.doria", source);
            let fixture = Fixture::new();
            let Some(output) = run(&php, &fixture) else {
                return;
            };
            assert_eq!(output.status.code(), Some(70), "{output:?}");
            assert!(output.stdout.is_empty(), "{output:?}");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(stderr.contains("origin-files.doria · line 3"), "{stderr}");
            assert!(stderr.contains("File::openRead"), "{stderr}");
            assert!(!stderr.contains("<compiler-known:streams> · line"), "{stderr}");
        }
    }

    #[test]
    fn opaque_file_cleanup_propagates_through_ordinary_owning_classes_and_lists() {
        let php = compile(
            "owned-files.doria",
            r#"
use Doria\Std\Io\{File, ReadableFile, IoError};
class FileHolder { function __construct(take ReadableFile $file) {} }
class FileListHolder { function __construct(take List<ReadableFile> $files) {} }
function main(): void throws IoError {
    let $holder = new FileHolder(File::openRead("binary.dat"));
    let $listHolder = new FileListHolder([File::openRead("binary.dat")]);
}
"#,
        );
        for name in ["FileHolder", "FileListHolder"] {
            assert!(
                contents(&php).contains(&format!(
                    "class {} implements __DoriaOwnedObject",
                    php_type_symbol(name),
                )),
                "{name} must use ordinary explicit owning cleanup",
            );
        }
        let fixture = Fixture::new();
        std::fs::write(fixture.0.join("binary.dat"), [42]).unwrap();
        let Some(output) = run(&php, &fixture) else {
            return;
        };
        assert!(output.status.success(), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
    }

    #[test]
    fn native_bytes_preserve_copy_parameters_and_project_real_borrow_cells() {
        let mut php = compile(
            "native-byte-borrow.doria",
            r#"
use Doria\Std\Io\{File, FileCreation, FileWriteMode, IoError};
function setByte(writable uint8 $byte, uint8 $value): void { $byte = $value; }
function main(): void throws IoError {
    writable Bytes $bytes = Bytes::fromArray([65, 66, 67]);
    setByte($bytes[1], 88);
    $bytes[1]++;
    $bytes[1] += 1;
    let writable $file = File::openWrite("projection.bin", FileCreation::CreateNew, FileWriteMode::Positioned);
    {
        let $region = $bytes->getRegion(0, 3);
        $file->write($region);
    }
    $file->close();
    let writable $reader = File::openRead("projection.bin");
    {
        let writable $region = $bytes->getWritableRegion(0, 3);
        $reader->read($region);
    }
    $reader->close();
    echo "{$bytes[0]} {$bytes[1]} {$bytes[2]}\n";
}
"#,
        );
        let BackendOutput::TextWithRuntime { contents, .. } = &mut php else {
            panic!("file transfer fixture must use native bytes");
        };
        // A writable Copy parameter changes its local copy, not its caller.
        // Exercise the private borrowed-slot carrier separately from that contract.
        contents.push_str(
            r#"
$owner = __DoriaBytes::fromArray([65, 66, 67], 0, 0, "");
$cell = $owner->__doriaBorrowCell(1, 0, 0, "");
$cell->value = 88;
$cell->value += 1;
$cell->value += 1;
$pointer = $owner->__doriaNativeBase();
echo $pointer[1] . "\n";
$pointer[1] = 91;
echo $cell->value . "\n";
"#,
        );
        let fixture = Fixture::new();
        let Some(output) = run(&php, &fixture) else {
            return;
        };
        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert_eq!(output.stdout, b"65 68 67\n90\n91\n");
        assert_eq!(
            std::fs::read(fixture.0.join("projection.bin")).unwrap(),
            b"ADC"
        );
    }

    #[test]
    fn native_byte_bases_reuse_one_view_without_retaining_buffer_owners() {
        let mut php = compile(
            "native-byte-pointer-homes.doria",
            r#"
use Doria\Std\Io\IoError;
function main(): void throws IoError {
    let $bytes = read_file_bytes("native.bin");
}
"#,
        );
        let BackendOutput::TextWithRuntime { contents, .. } = &mut php else {
            panic!("byte-pointer fixture must use native storage");
        };
        contents.push_str(
            r#"
foreach ([false, true] as $adopted) {
    $owner = $adopted
        ? __DoriaNativeStandardIo::get()->intrinsic(2, 'native.bin', null, 0, 0, '')
        : __DoriaBytes::fromArray([65, 66, 67, 68], 0, 0, '');
    $weak = WeakReference::create($owner);
    $values = (new ReflectionProperty(__DoriaBytes::class, 'values'))->getValue($owner);
    $weakValues = WeakReference::create($values);
    unset($values);
    $region = $owner->getWritableRegion(1, 2, 0, 0, '');
    $nested = $region->getRegion(1, 1, 0, 0, '');
    $empty = $owner->getWritableRegion(4, 0, 0, 0, '');
    $base = $owner->__doriaNativeBase();
    for ($iteration = 0; $iteration < 100; ++$iteration) {
        if ($base !== $owner->__doriaNativeBase()
            || $owner !== $region->__doriaNativeStorage(true)
            || $owner !== $nested->__doriaNativeStorage(false)
            || $owner !== $empty->__doriaNativeStorage(true)) {
            throw new LogicException('native base was rebuilt');
        }
    }
    $owner->set(1, 88, 0, 0, '');
    $base[2] = 90;
    if ($base[1] !== 88 || $owner->get(2, 0, 0, '') !== 90 || count($region) !== 2
        || $region->__doriaRegionOffset() !== 1 || $nested->__doriaRegionOffset() !== 2) {
        throw new LogicException('native base selected the wrong storage');
    }
    try {
        $nested->__doriaNativeStorage(true);
        throw new RuntimeException('cached readonly pointer was upgraded');
    } catch (LogicException $error) {
        if ($error->getMessage() !== 'MIR native transfer upgraded readonly access') { throw $error; }
    }
    unset($owner);
    if ($weak->get() !== null || $weakValues->get() !== null) {
        throw new LogicException('native base retained its buffer');
    }
    foreach ([$region, $nested, $empty] as $expired) {
        try {
            $expired->__doriaNativeStorage(false);
            throw new RuntimeException('cached native pointer outlived its buffer');
        } catch (LogicException $error) {
            if ($error->getMessage() !== 'MIR byte region outlived its source') { throw $error; }
        }
    }
}
$emptyOwner = __DoriaBytes::allocate(0, 0, 0, '');
$emptyRegion = $emptyOwner->getWritableRegion(0, 0, 0, 0, '');
if ($emptyRegion->__doriaNativeStorage(true)->__doriaNativeBase() !== null) {
    throw new LogicException('empty storage exposed a native base');
}
echo "pointer homes\n";
"#,
        );
        let fixture = Fixture::new();
        std::fs::write(fixture.0.join("native.bin"), b"ABCD").unwrap();
        let Some(output) = run(&php, &fixture) else {
            return;
        };
        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert_eq!(output.stdout, b"pointer homes\n");
    }

    #[test]
    fn native_php_bundle_remains_executable_after_relocation() {
        let php = compile(
            "relocated-files.doria",
            include_str!("../../../../examples/native/main_stream_readable_file.doria"),
        );
        let fixture = Fixture::new();
        let original = fixture.0.join("original");
        let relocated = fixture.0.join("relocated");
        crate::backend::publish_output(&original.join("main.php"), php).unwrap();
        std::fs::rename(&original, &relocated).unwrap();
        let output = Command::new("php")
            .arg(relocated.join("main.php"))
            .current_dir(&fixture.0)
            .output()
            .expect("PHP is required for native bundle parity");
        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert_eq!(
            output.stdout,
            include_bytes!(
                "../../tests/fixtures/native_io/main_stream_readable_file/expected_stdout"
            )
        );
    }

    #[test]
    fn disabled_ffi_is_not_required_for_pure_php_streams_and_is_not_silently_bypassed_for_files() {
        for (source, native) in [
            (
                include_str!("../../../../examples/native/main_stream_capabilities.doria"),
                false,
            ),
            (
                include_str!("../../../../examples/native/main_stream_readable_file.doria"),
                true,
            ),
        ] {
            let php = compile("ffi-configuration.doria", source);
            assert_eq!(matches!(php, BackendOutput::TextWithRuntime { .. }), native);
            let fixture = Fixture::new();
            let path = fixture.0.join("main.php");
            crate::backend::publish_output(&path, php).unwrap();
            let output = Command::new("php")
                .args(["-d", "ffi.enable=0"])
                .arg(path)
                .current_dir(&fixture.0)
                .output()
                .expect("PHP is required for deployment tests");
            if native {
                assert_eq!(output.status.code(), Some(1), "{output:?}");
                let error = String::from_utf8_lossy(&output.stderr);
                assert!(error.contains("Error[B3602]"), "{error}");
                assert!(error.contains("does not permit FFI"), "{error}");
                assert!(
                    !error.contains("Fatal error") && !error.contains("Stack trace"),
                    "{error}"
                );
            } else {
                assert!(output.status.success(), "{output:?}");
                assert!(output.stderr.is_empty(), "{output:?}");
                assert_eq!(
                    output.stdout,
                    include_bytes!(
                        "../../tests/fixtures/native_io/main_stream_capabilities/expected_stdout"
                    )
                );
            }
        }
    }

    #[test]
    fn missing_or_mismatched_native_dependencies_fail_before_native_execution() {
        let php = compile(
            "dependency-files.doria",
            include_str!("../../../../examples/native/main_stream_readable_file.doria"),
        );
        let BackendOutput::TextWithRuntime { runtime, .. } = &php else {
            panic!("native file operations must carry their verified dependency");
        };
        for fault in ["missing", "metadata", "bytes"] {
            let fixture = Fixture::new();
            let path = fixture.0.join("main.php");
            crate::backend::publish_output(&path, php.clone()).unwrap();
            let library = fixture.0.join(runtime.relative_library_path());
            let metadata = fixture.0.join(runtime.relative_metadata_path());
            match fault {
                "missing" => std::fs::remove_file(library).unwrap(),
                "metadata" => {
                    let mut document: serde_json::Value =
                        serde_json::from_str(runtime.metadata_json()).unwrap();
                    document["runtimeRevision"] = "wrong-runtime".into();
                    std::fs::write(metadata, serde_json::to_vec(&document).unwrap()).unwrap();
                }
                "bytes" => std::fs::write(library, b"not a native library").unwrap(),
                _ => unreachable!(),
            }
            let output = Command::new("php")
                .arg(path)
                .current_dir(&fixture.0)
                .output()
                .expect("PHP is required for deployment tests");
            assert_eq!(output.status.code(), Some(1), "{fault}: {output:?}");
            let error = String::from_utf8_lossy(&output.stderr);
            assert!(error.contains("Error[B3602]"), "{fault}: {error}");
            assert!(
                !error.contains("Fatal error") && !error.contains("Stack trace"),
                "{fault}: {error}"
            );
        }
    }
}
