//! Ordinary PHP dispatch over the shared native transfer controller. These
//! carriers borrow existing receivers and regions; they never own an I/O source.
use super::*;

pub(super) fn emit_runtime(output: &mut String) {
    let mut runtime = RUNTIME.to_string();
    for name in [
        "ReadRequest",
        "WriteRequest",
        "ReadOutcome",
        "WriteOutcome",
        "ReadTransferKey",
        "WriteTransferKey",
        "TransferCompletion",
        "TransferWaitResult",
    ] {
        runtime = runtime.replace(
            &format!("__Doria{name}"),
            &php_type_symbol(&format!("Doria\\Std\\Io\\{name}")),
        );
    }
    for (token, kind) in [
        (
            "__DoriaReadonlyRegionHome",
            crate::types::ByteRegionKind::Readonly,
        ),
        (
            "__DoriaWritableRegionHome",
            crate::types::ByteRegionKind::Writable,
        ),
    ] {
        runtime = runtime.replace(token, &value_result::storage::cell_name(kind.source_name()));
    }
    output.push_str(&runtime);
}

pub(in crate::codegen_php) fn emit_group_members(output: &mut String, indent: usize) {
    for line in [
        "private __DoriaHostedTransferGroup $__doriaTransferGroup;",
        "public static function __doriaFromHostedGroup(__DoriaHostedTransferGroup $group): self",
        "{",
        "    return new self($group);",
        "}",
    ] {
        writeln(output, indent, line);
    }
}

pub(in crate::codegen_php) fn emit_group_body(
    intrinsic: crate::compiler_known_stream::TransferGroupFunction,
    function: &FunctionDecl,
    parameters: &[Option<value_result::Place>],
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    use crate::compiler_known_stream::TransferGroupFunction;
    if intrinsic == TransferGroupFunction::NextCompletion {
        emit_completion_body(function, parameters, output, indent, scopes);
        return;
    }
    let body = match intrinsic {
        TransferGroupFunction::NextCompletion => unreachable!(),
        TransferGroupFunction::ObservedReadKey | TransferGroupFunction::ObservedWriteKey => {
            let reading = intrinsic == TransferGroupFunction::ObservedReadKey;
            format!(
                "return $this->__doriaTransferGroup->observed({});",
                if reading { "true" } else { "false" }
            )
        }
        TransferGroupFunction::ReadRegion => {
            assert!(
                scopes.value_return.is_some(),
                "completed regions publish shared fields"
            );
            format!(
                "$this->__doriaTransferGroup->readRegionFields({}); return;",
                intrinsic_parameter(function, 0, scopes)
            )
        }
        TransferGroupFunction::ResubmitRead | TransferGroupFunction::ResubmitWrite => {
            format!(
                "$this->__doriaTransferGroup->resubmit({}, {}, {}, {});",
                intrinsic_parameter(function, 0, scopes),
                if intrinsic == TransferGroupFunction::ResubmitRead {
                    "true"
                } else {
                    "false"
                },
                intrinsic_parameter(function, 1, scopes),
                intrinsic_parameter(function, 2, scopes)
            )
        }
    };
    writeln(output, indent, &body);
}

fn emit_completion_body(
    function: &FunctionDecl,
    parameters: &[Option<value_result::Place>],
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    use value_result::CaseArgument::{Result, Scalar};
    let wait = value_result::intrinsic_parameter(
        function,
        0,
        parameters[0].as_ref(),
        output,
        indent,
        scopes,
    );
    let tag = wait.tag();
    let duration = wait.scalar_payload("For", 0, &scopes.specialization);
    let deadline = wait.scalar_payload("Until", 0, &scopes.specialization);
    value_result::clear(output, indent);
    writeln(output, indent, "$group = $this->__doriaTransferGroup;");
    writeln(
        output,
        indent,
        &format!("$instant = {tag} === 1 ? {duration} : ({tag} === 2 ? {deadline} : null);"),
    );
    writeln(
        output,
        indent,
        &format!("$code = $group->observeWait({tag}, $instant);"),
    );
    let result = "Doria\\Std\\Io\\TransferWaitResult";
    for (code, case) in [(5, "Exhausted"), (6, "TimedOut")] {
        let call = value_result::case_call(result, case, &[], &scopes.specialization);
        writeln(
            output,
            indent,
            &format!("if ($code === {code}) {{ {call} return; }}"),
        );
    }
    writeln(
        output,
        indent,
        "if ($code !== 0 && $code !== 4) { __DoriaNativeIo::fail('protocol'); }",
    );
    writeln(output, indent, "$reading = $group->isReadCompletion();");
    writeln(output, indent, "$key = $group->observed($reading);");
    writeln(
        output,
        indent,
        "if ($key === null) { __DoriaNativeIo::fail('protocol'); }",
    );
    // Adapter dispatch can use the same channel. Publish only this observation.
    value_result::clear(output, indent);
    for (reading, name, case, variants) in [
        (
            true,
            "Doria\\Std\\Io\\ReadOutcome",
            "Read",
            &["Data", "WouldBlock", "EndOfStream", "TimedOut"][..],
        ),
        (
            false,
            "Doria\\Std\\Io\\WriteOutcome",
            "Write",
            &["Accepted", "WouldBlock"][..],
        ),
    ] {
        writeln(
            output,
            indent,
            if reading { "if ($reading) {" } else { "else {" },
        );
        writeln(output, indent + 1, "if ($code === 0) {");
        writeln(output, indent + 2, "switch ($group->outcome->tag) {");
        for (tag, variant) in variants.iter().enumerate() {
            let arguments = if tag == 0 {
                vec![Scalar("$group->outcome->count")]
            } else {
                Vec::new()
            };
            let call = value_result::case_call(name, variant, &arguments, &scopes.specialization);
            writeln(output, indent + 3, &format!("case {tag}: {call} break;"));
        }
        writeln(
            output,
            indent + 3,
            "default: __DoriaNativeIo::fail('protocol');",
        );
        writeln(output, indent + 2, "}");
        writeln(output, indent + 1, "}");
        let call = value_result::case_call(
            "Doria\\Std\\Io\\TransferCompletion",
            case,
            &[Scalar("$key"), Result(name)],
            &scopes.specialization,
        );
        writeln(output, indent + 1, &call);
        writeln(output, indent, "}");
    }
    let call = value_result::case_call(
        result,
        "Completed",
        &[Result("Doria\\Std\\Io\\TransferCompletion")],
        &scopes.specialization,
    );
    writeln(output, indent, &call);
    writeln(output, indent, "return;");
}

pub(super) fn member_dispatch(receiver: &ResolvedType, scopes: &PhpNameScopes) -> String {
    use crate::compiler_known_stream::StreamOperation;
    [
        StreamOperation::PrepareRead,
        StreamOperation::CompleteRead,
        StreamOperation::PrepareWrite,
        StreamOperation::CompleteWrite,
    ]
    .into_iter()
    .map(|operation| {
        let Some((name, plan, fields)) = scopes.specialization.stream_callable(receiver, operation)
        else {
            return "null".to_string();
        };
        let field_result = matches!(
            operation,
            StreamOperation::CompleteRead | StreamOperation::CompleteWrite
        );
        if field_result {
            assert!(
                !plan.returns_borrow,
                "completion outcomes are independent Copy values"
            );
        }
        let field_arguments = fields.get(1).is_some_and(Option::is_some);
        let name = if field_arguments {
            scopes.specialization.argument_symbol(name, field_result)
        } else if field_result {
            scopes.specialization.result_symbol(name)
        } else {
            name
        };
        let flags = u8::from(plan.caller_receiver_home())
            | (u8::from(plan.parameters[0].cell) << 1)
            | (u8::from(plan.parameters[1].cell) << 2)
            | (u8::from(field_result) << 3)
            | (u8::from(field_arguments) << 4);
        format!("[{}, {flags}]", emit_php_string_literal(name))
    })
    .collect::<Vec<_>>()
    .join(", ")
}

const RUNTIME: &str = r#"
final class __DoriaPendingMember
{
    public bool $fieldResult = false;
    public int $resultTag = 4;
    public int $resultCount = 0;

    public function __construct(public readonly __DoriaCell $receiver,
        public readonly string $getter, public readonly bool $getterUsesHome,
        private readonly ?array $prepareRead, private readonly ?array $completeRead,
        private readonly ?array $prepareWrite, private readonly ?array $completeWrite) {}

    public function source(): FFI\CData
    {
        $owner = $this->receiver->value;
        $getter = $this->getter;
        $source = $this->getterUsesHome ? $owner->$getter($this->receiver) : $owner->$getter();
        if ($source instanceof __DoriaCell) { $source = $source->value; }
        return $source->__doriaNativeReadinessSource();
    }

    public function dispatch(bool $preparing, bool $reading, __DoriaCell $request,
        __DoriaCell $region): mixed
    {
        $method = $reading
            ? ($preparing ? $this->prepareRead : $this->completeRead)
            : ($preparing ? $this->prepareWrite : $this->completeWrite);
        if ($method === null) { __DoriaNativeIo::fail('protocol'); }
        [$name, $flags] = $method;
        $request = ($flags & 2) !== 0 ? $request : $request->value;
        $this->fieldResult = ($flags & 8) !== 0;
        $this->resultTag = 4;
        $this->resultCount = 0;
        if ($this->fieldResult) { __DoriaValueResult::clear(); }
        try {
            if (($flags & 16) !== 0) {
                $region->__doriaLoadFields();
                if (!__DoriaValueResult::$present) { __DoriaNativeIo::fail('protocol'); }
                $storage = __DoriaValueResult::$field0;
                $offset = __DoriaValueResult::$field1;
                $length = __DoriaValueResult::$field2;
                __DoriaValueResult::clear();
                $result = ($flags & 1) !== 0
                    ? $this->receiver->value->$name($this->receiver, $request, 0, true, $storage, $offset, $length)
                    : $this->receiver->value->$name($request, 0, true, $storage, $offset, $length);
            } else {
                $region = ($flags & 4) !== 0 ? $region : $region->value;
                $result = ($flags & 1) !== 0
                    ? $this->receiver->value->$name($this->receiver, $request, $region)
                    : $this->receiver->value->$name($request, $region);
            }
            if ($this->fieldResult) {
                $this->resultTag = __DoriaValueResult::$present ? __DoriaValueResult::$tag : 4;
                $this->resultCount = $this->resultTag === 0 ? __DoriaValueResult::$field0 : 0;
            }
            return $result;
        } finally {
            if ($this->fieldResult) { __DoriaValueResult::clear(); }
        }
    }
}

function __doria_native_opaque_storage(int $size, int $alignment): FFI\CData
{
    $runtime = __DoriaNativeIo::get();
    $word = $runtime->new('uint64_t');
    if ($size < 1 || $alignment < 1 || $alignment > FFI::alignof($word)) {
        __DoriaNativeIo::fail('layout');
    }
    return $runtime->new('uint64_t[' . intdiv($size + FFI::sizeof($word) - 1, FFI::sizeof($word)) . ']');
}

final class __DoriaHostedOperation
{
    public function __construct(public readonly __DoriaPendingMember $member,
        public readonly __DoriaByteRegion $root, public readonly bool $reading,
        public readonly __DoriaReadTransferKey|__DoriaWriteTransferKey $key) {}
}

final class __DoriaHostedTransferGroup
{
    private ?FFI\CData $owner = null;
    private bool $closed = false;
    private readonly FFI $runtime;
    private readonly __DoriaHostErrors $errors;
    private readonly FFI\CData $terminal;
    private readonly FFI\CData $view;
    private readonly FFI\CData $record;
    private readonly FFI\CData $caller;
    private readonly FFI\CData $length;
    private readonly FFI\CData $reading;
    private readonly FFI\CData $status;
    private readonly FFI\CData $statusAddress;
    private readonly FFI\CData $terminalAddress;
    private readonly FFI\CData $viewAddress;
    private readonly FFI\CData $outcomeAddress;
    private readonly FFI\CData $controlAddress;
    private readonly FFI\CData $callerRead;
    private readonly FFI\CData $key;
    public readonly FFI\CData $kind;
    public readonly FFI\CData $index;
    public readonly FFI\CData $outcome;
    public readonly FFI\CData $control;
    private array $requests = [];
    private array $requestHomes = [];
    private array $regionHomes = [];
    private array $operations = [];

    // Operations contain compiler-proved member dispatch, original borrowed
    // region and direction. The readiness set remains the sole receiver owner.
    private function __construct(int $operationCount)
    {
        $this->runtime = $runtime = __DoriaNativeIo::get();
        $this->errors = new __DoriaHostErrors($operationCount);
        $layout = $runtime->new('size_t[4]');
        $runtime->dr_v1_host_adapter_error_layout(FFI::addr($layout[0]), FFI::addr($layout[1]),
            FFI::addr($layout[2]), FFI::addr($layout[3]));
        $this->terminal = __doria_native_opaque_storage($layout[2], $layout[3]);
        $runtime->dr_v1_host_pending_view_layout(FFI::addr($layout[0]), FFI::addr($layout[1]));
        $this->view = __doria_native_opaque_storage($layout[0], $layout[1]);
        $this->record = $runtime->new('void *[1]');
        $this->caller = $runtime->new('uint8_t *[1]');
        $this->length = $runtime->new('size_t[1]');
        $this->reading = $runtime->new('uint8_t[1]');
        $this->kind = $runtime->new('uint8_t[1]');
        $this->index = $runtime->new('size_t[1]');
        $this->status = $runtime->new('DrIoStatusV1');
        $this->outcome = $runtime->new('DrAdapterOutcomeV1');
        $this->control = $runtime->new('DrCompletionStatusV1');
        $this->statusAddress = FFI::addr($this->status);
        $this->terminalAddress = FFI::addr($this->terminal[0]);
        $this->viewAddress = FFI::addr($this->view[0]);
        $this->outcomeAddress = FFI::addr($this->outcome);
        $this->controlAddress = FFI::addr($this->control);
        $this->callerRead = $runtime->cast('const uint8_t **', $this->caller);
        $this->key = $runtime->new('int64_t[2]');
        foreach ([['dr_v1_transfer_adapter_outcome_layout', $this->outcome],
            ['dr_v1_transfer_outcome_layout', $this->control]] as [$query, $storage]) {
            $runtime->$query(FFI::addr($layout[0]), FFI::addr($layout[1]));
            if (FFI::sizeof($storage) !== $layout[0] || FFI::alignof($storage) !== $layout[1]) {
                __DoriaNativeIo::fail('layout');
            }
        }
    }

    public static function begin(FFI\CData $set, FFI\CData $plan, array $operations,
        array $members, mixed $deadline, mixed $cancellation): self
    {
        // Install the PHP cleanup guard before shared-core startup can dispatch.
        $consumed = false;
        try {
            $group = new self(count($operations));
            $keys = $group->planKeys($plan, $operations);
            $instant = null;
            if ($deadline !== null) {
                $instant = $group->runtime->new('DrDeadlineV1');
                $instant->low = $deadline->__doriaLowWord();
                $instant->high = $deadline->__doriaHighWord();
            }
            $state = $cancellation?->__doriaCancellationState();
            $consumed = true;
            $owner = $group->runtime->dr_v2_host_transfer_plan_configure(null, $set, $plan,
                $instant === null ? null : FFI::addr($instant), $state, $group->statusAddress);
            if ($owner === null || FFI::isNull($owner)) { $group->throwStatus(__DoriaStdIoIoOperation::Configure, $group->status); }
            $group->owner = $owner;
            foreach ($operations as $index => [$identity, $root, $reading]) {
                if (!isset($members[$identity])) { __DoriaNativeIo::fail('protocol'); }
                $member = $members[$identity];
                $group->operations[$index] = new __DoriaHostedOperation($member, $root, $reading, $keys[$index]);
                $request = new __DoriaPendingRequest($group->viewAddress, $group->runtime,
                    $member->receiver, $member->getter, $member->getterUsesHome, $owner);
                $group->requests[$index] = $reading
                    ? __DoriaReadRequest::__doriaFromPendingRequest($request)
                    : __DoriaWriteRequest::__doriaFromPendingRequest($request);
                // Callback borrows refer to group-lifetime slots, including replacements.
                $group->requestHomes[$index] = __DoriaCell::borrowed($group->requests[$index]);
                $home = $reading ? new __DoriaWritableRegionHome() : new __DoriaReadonlyRegionHome();
                $home->borrowed = true;
                $root->__doriaPublishFields();
                $home->__doriaStoreFields();
                $group->regionHomes[$index] = $home;
            }
            $code = $group->drive();
            if ($code !== 0) {
                $group->closed = true; // Failed startup has already drained the whole group.
                $group->throwResult($code, __DoriaStdIoIoOperation::Configure);
            }
            return $group;
        } catch (Throwable $error) {
            if (isset($group) && __doria_cleanup_enabled()) { $group->close(); }
            throw $error;
        } finally {
            if (!$consumed && __doria_cleanup_enabled()) {
                __DoriaNativeIo::get()->dr_v1_transfer_plan_drop($plan);
            }
        }
    }

    private function native(): FFI\CData
    {
        return $this->owner ?? throw new LogicException('consumed hosted transfer group');
    }

    private function planKeys(FFI\CData $plan, array $operations): array
    {
        $runtime = $this->runtime;
        $message = $runtime->new('DrStringV1 *[1]');
        $path = $runtime->new('DrStringV1 *[1]');
        $system = $runtime->new('int64_t[1]');
        $transferred = $runtime->new('size_t[1]');
        $meta = $runtime->new('uint64_t[1]');
        $keys = [];
        foreach ($operations as $index => [, , $reading]) {
            $code = $runtime->dr_v1_checked_transfer_plan_key(null, $plan, $index, (int) $reading,
                $this->key, $message, $path, $system, $transferred, $meta);
            try {
                if ($code === 0) {
                    $keys[] = $reading
                        ? __DoriaReadTransferKey::__doriaFromTransferKey($this->key[0], $this->key[1])
                        : __DoriaWriteTransferKey::__doriaFromTransferKey($this->key[0], $this->key[1]);
                    continue;
                }
                if ($code !== 1) { __DoriaNativeIo::fail('protocol'); }
                __doria_throw_io_facts(__DoriaStdIoIoOperation::Access,
                    __DoriaStdIoIoTarget::__doriaCaseTransferPlan(),
                    __doria_native_io_reason(($meta[0] >> 16) & 255),
                    ($meta[0] & (1 << 32)) === 0 ? null : $system[0], null, 0, '');
            } finally {
                foreach ([$message, $path] as $slot) {
                    if ($slot[0] !== null && !FFI::isNull($slot[0])) { $runtime->dr_v1_string_release($slot[0]); }
                    $slot[0] = null;
                }
            }
        }
        return $keys;
    }

    public function drive(): int
    {
        $runtime = $this->runtime;
        while (true) {
            $code = $runtime->dr_v1_host_transfer_group_poll(null, $this->native(),
                $this->kind, $this->index, $this->outcomeAddress,
                $this->terminalAddress, $this->controlAddress);
            $this->errors->sweep();
            if ($code !== 8) { return $code; }
            $action = $runtime->dr_v1_host_transfer_group_action($this->native(), $this->index,
                $this->record, $this->viewAddress, $this->caller, $this->length, $this->reading);
            if ($action === 0) { continue; } // A native poll may advance without host work.
            $index = $this->index[0];
            $operation = $this->operations[$index];
            $member = $operation->member;
            $reading = $operation->reading;
            // Source handoffs have no request/buffer direction yet.
            if ($action >= 3 && $reading !== ($this->reading[0] !== 0)) { __DoriaNativeIo::fail('protocol'); }
            $error = null;
            $result = null;
            try {
                if ($action === 1 || $action === 2) {
                    $source = $member->source();
                } else {
                    $result = $member->dispatch($action === 3, $reading,
                        $this->requestHomes[$index], $this->regionHomes[$index]);
                }
            } catch (__DoriaCheckedError $caught) {
                $this->errors->retain($index, $caught, $this->length[0], $this->terminalAddress);
                $error = $this->terminalAddress;
            }
            if ($action === 1 || $action === 2) {
                $reply = $runtime->dr_v1_host_pending_source_reply($this->record[0],
                    $error === null ? $source : null, $error);
            } elseif ($action === 3) {
                $reply = $runtime->dr_v1_host_pending_prepare_reply($this->viewAddress, $error);
            } elseif ($action === 4) {
                if ($member->fieldResult) {
                    $tag = $member->resultTag;
                    $count = $member->resultCount;
                } else {
                    $tag = $result === null ? 4 : ($result->__doriaMatchesCase(0) ? 0 :
                        ($result->__doriaMatchesCase(1) ? 1 : ($result->__doriaMatchesCase(2) ? 2 : 3)));
                    $count = $tag === 0 ? $result->__doriaPayloadAt(0) : 0;
                }
                $reply = $runtime->dr_v1_host_pending_complete_reply($this->viewAddress,
                    $tag, $count, $error);
            } else { __DoriaNativeIo::fail('protocol'); }
            if ($reply !== 0) { __DoriaNativeIo::fail('protocol'); }
        }
    }

    private function throwStatus(__DoriaStdIoIoOperation $operation, FFI\CData $status): never
    {
        __doria_throw_io_facts($operation,
            __DoriaStdIoIoTarget::__doriaCaseTransferGroup(), __doria_native_io_reason($status->reason),
            $status->has_system_code === 0 ? null : $status->system_code,
            $status->has_transferred_byte_count === 0 ? null : $status->transferred_byte_count, 0, '');
    }

    public function throwResult(int $code, __DoriaStdIoIoOperation $operation): never
    {
        if ($code === 1) {
            $identity = $this->runtime->dr_v1_host_adapter_error_return($this->terminalAddress);
            throw $this->errors->take($identity);
        }
        if ($code === 3) { $this->throwStatus($operation, $this->control->transfer); }
        if ($code === 7) { $this->throwStatus(__DoriaStdIoIoOperation::Wait, $this->control->wait_failure); }
        __DoriaNativeIo::fail('protocol');
    }

    public function observe(int $kind, int $low, int $high): int
    {
        if ($this->runtime->dr_v1_host_transfer_group_observe_wait($this->native(), $kind, $low, $high,
            $this->statusAddress) !== 0) {
            $this->throwStatus(__DoriaStdIoIoOperation::Wait, $this->status);
        }
        $code = $this->drive();
        if ($code === 1 || $code === 3 || $code === 7) {
            $this->throwResult($code, __DoriaStdIoIoOperation::Configure);
        }
        return $code;
    }

    public function observeWait(int $tag, mixed $instant): int
    {
        if ($tag === 0) { return $this->observe(0, 0, 0); }
        if ($tag === 3) { return $this->observe(1, 0, 0); }
        if ($tag === 1) { $kind = 2; }
        elseif ($tag === 2) { $kind = 3; }
        else { __DoriaNativeIo::fail('protocol'); }
        return $this->observe($kind, $instant->__doriaLowWord(), $instant->__doriaHighWord());
    }

    public function isReadCompletion(): bool
    {
        return $this->operations[$this->index[0]]->reading;
    }

    public function observed(bool $reading): __DoriaReadTransferKey|__DoriaWriteTransferKey|null
    {
        $code = $this->runtime->dr_v1_host_transfer_group_observed($this->native(), (int) $reading, $this->key);
        if ($code === 1) { return null; }
        if ($code !== 0) { __DoriaNativeIo::fail('protocol'); }
        $operation = $this->operations[$this->key[1]];
        $key = $operation->key;
        if ($operation->reading !== $reading || $key->__doriaLowWord() !== $this->key[0]
            || $key->__doriaHighWord() !== $this->key[1]) { __DoriaNativeIo::fail('protocol'); }
        return $key;
    }

    public function readRegion(__DoriaReadTransferKey $key): ?__DoriaByteRegion
    {
        $this->readRegionFields($key);
        try { return __DoriaValueResult::boxed(); }
        finally { __DoriaValueResult::clear(); }
    }

    public function readRegionFields(__DoriaReadTransferKey $key): void
    {
        __DoriaValueResult::clear();
        $key->__doriaWriteNativeTransferKey($this->key);
        $code = $this->runtime->dr_v1_host_transfer_group_read_region($this->native(), $this->key,
            $this->callerRead, $this->length, $this->statusAddress);
        if ($code === 1) { return; }
        if ($code === 3) { $this->throwStatus(__DoriaStdIoIoOperation::Access, $this->status); }
        if ($code !== 0) { __DoriaNativeIo::fail('protocol'); }
        // The core has proved this exact key's position, direction and availability.
        $home = $this->regionHomes[$this->key[1]];
        __DoriaByteRegion::__doriaSubregionFields($home->field0, $home->field1,
            $home->field2, true, 0, $this->length[0], false, 0, 0, '');
    }

    public function resubmit(__DoriaReadTransferKey|__DoriaWriteTransferKey $key,
        bool $reading, int $offset, int $length): void
    {
        $key->__doriaWriteNativeTransferKey($this->key);
        if ($this->runtime->dr_v1_host_transfer_group_resubmit($this->native(), $this->key,
            (int) $reading, $offset, $length, $this->statusAddress) !== 0) {
            $this->throwStatus(__DoriaStdIoIoOperation::Configure, $this->status);
        }
        // Successful core validation proves this position and its original root range.
        $index = $this->key[1];
        $root = $this->operations[$index]->root;
        __DoriaByteRegion::__doriaSubregionFields($root->__doriaRegionStorage(),
            $root->__doriaRegionOffset(), count($root), $reading,
            $offset, $length, $reading, 0, 0, '');
        $this->regionHomes[$index]->__doriaStoreFields();
        $code = $this->drive();
        if ($code !== 0) { $this->throwResult($code, __DoriaStdIoIoOperation::Configure); }
    }

    public function close(): void
    {
        if ($this->owner === null) { return; }
        try {
            if (!$this->closed) {
                $this->runtime->dr_v1_host_transfer_group_close($this->owner);
                if ($this->drive() !== 0 || $this->kind[0] !== 3) { __DoriaNativeIo::fail('protocol'); }
                $this->closed = true;
            }
            $this->runtime->dr_v1_host_transfer_group_drop(null, $this->owner);
        } finally {
            $this->owner = null;
            $this->errors->sweep();
            $this->requestHomes = $this->regionHomes = [];
            $this->requests = $this->operations = [];
        }
    }

    public function __destruct()
    {
        if (__doria_cleanup_enabled()) { $this->close(); }
    }
}
"#;
