<?php
// Appended to compiler output to exercise private host-controller storage and
// dispatch alongside the public standard-device contract regressions.
final class __TestStandardReadiness
{
    public function __construct(private readonly int $stream) {}
    public function __doriaNativeReadinessSource(): FFI\CData
    {
        return __DoriaNativeIo::get()->dr_v1_standard_stream_readiness_source($this->stream);
    }
}

final class __TestPendingDispatch
{
    public WeakReference $receiver;
    public WeakReference $request;
    public WeakReference $region;
    public bool $present = true;
    public int $tag = 0;
    public int $count = 0;
    public bool $fail = false;

    public function __construct(private readonly int $flags) {}

    private function accept(string $method, mixed $first, mixed $second, mixed $third): string
    {
        if (($this->flags & 1) !== 0) {
            if ($first !== $this->receiver->get()) { throw new LogicException('receiver home changed'); }
            $request = $second;
            $region = $third;
        } else {
            if ($third !== null) { throw new LogicException('unexpected receiver argument'); }
            $request = $first;
            $region = $second;
        }
        $requestHome = $this->request->get();
        $regionHome = $this->region->get();
        if ($request !== (($this->flags & 2) !== 0 ? $requestHome : $requestHome->value)
            || $region !== (($this->flags & 4) !== 0 ? $regionHome : $regionHome->value)) {
            throw new LogicException('callback argument transport changed');
        }
        return $method;
    }

    public function prepareRead(mixed $first, mixed $second, mixed $third = null): string
    { return $this->accept(__FUNCTION__, $first, $second, $third); }
    public function completeRead(mixed $first, mixed $second, mixed $third = null): string
    { return $this->accept(__FUNCTION__, $first, $second, $third); }
    public function prepareWrite(mixed $first, mixed $second, mixed $third = null): string
    { return $this->accept(__FUNCTION__, $first, $second, $third); }
    public function completeWrite(mixed $first, mixed $second, mixed $third = null): string
    { return $this->accept(__FUNCTION__, $first, $second, $third); }

    private function publish(string $method, mixed $first, mixed $second, mixed $third): void
    {
        $this->accept($method, $first, $second, $third);
        if ($this->present) {
            __DoriaValueResult::$type = 1;
            __DoriaValueResult::$tag = $this->tag;
            __DoriaValueResult::$present = true;
            __DoriaValueResult::$fields = $this->tag === 0 ? 1 : 0;
            __DoriaValueResult::$field0 = $this->tag === 0 ? $this->count : null;
        }
        if ($this->fail) { throw new LogicException('field dispatch failed'); }
    }

    public function completeReadFields(mixed $first, mixed $second, mixed $third = null): void
    { $this->publish('completeRead', $first, $second, $third); }
    public function completeWriteFields(mixed $first, mixed $second, mixed $third = null): void
    { $this->publish('completeWrite', $first, $second, $third); }
}

for ($flags = 0; $flags < 8; ++$flags) {
    $probe = new __TestPendingDispatch($flags);
    $requestValue = $regionValue = null;
    $receiver = __DoriaCell::borrowed($probe);
    $requestHome = __DoriaCell::borrowed($requestValue);
    $regionHome = __DoriaCell::borrowed($regionValue);
    $probe->receiver = WeakReference::create($receiver);
    $probe->request = WeakReference::create($requestHome);
    $probe->region = WeakReference::create($regionHome);
    $member = new __DoriaPendingMember($receiver, 'unused', false,
        ['prepareRead', $flags], ['completeRead', $flags],
        ['prepareWrite', $flags], ['completeWrite', $flags]);
    for ($round = 0; $round < 2; ++$round) {
        $requestValue = new stdClass();
        $regionValue = new stdClass();
        foreach ([true, false] as $reading) {
            foreach ([true, false] as $preparing) {
                $expected = ($preparing ? 'prepare' : 'complete') . ($reading ? 'Read' : 'Write');
                if ($member->dispatch($preparing, $reading, $requestHome, $regionHome) !== $expected) {
                    throw new LogicException('callback phase or direction changed');
                }
            }
        }
    }
    $fields = new __DoriaPendingMember($receiver, 'unused', false,
        null, ['completeReadFields', $flags | 8], null, ['completeWriteFields', $flags | 8]);
    foreach ([true, false] as $reading) {
        foreach ([[true, 0, 0], [true, 0, 7], [true, 1, 0], [true, 2, 0],
            [true, 3, 0], [false, 0, 99]] as [$present, $tag, $count]) {
            if (!$reading && $tag > 1) { continue; }
            $probe->present = $present;
            $probe->tag = $tag;
            $probe->count = $count;
            __DoriaValueResult::$present = true;
            __DoriaValueResult::$fields = 1;
            __DoriaValueResult::$tag = 0;
            __DoriaValueResult::$field0 = 999;
            if ($fields->dispatch(false, $reading, $requestHome, $regionHome) !== null
                || !$fields->fieldResult || $fields->resultTag !== ($present ? $tag : 4)
                || $fields->resultCount !== ($present && $tag === 0 ? $count : 0)
                || __DoriaValueResult::$present || __DoriaValueResult::$fields !== 0
                || __DoriaValueResult::$field0 !== null) {
                throw new LogicException('field completion changed presence, tag, count or cleanup');
            }
        }
        $probe->present = true;
        $probe->tag = 0;
        $probe->count = 7;
        $probe->fail = true;
        try {
            $fields->dispatch(false, $reading, $requestHome, $regionHome);
            throw new LogicException('field completion failure was lost');
        } catch (LogicException $error) {
            if ($error->getMessage() !== 'field dispatch failed') { throw $error; }
        }
        unset($error);
        $probe->fail = false;
        if ($fields->resultTag !== 4 || $fields->resultCount !== 0
            || __DoriaValueResult::$present || __DoriaValueResult::$fields !== 0
            || __DoriaValueResult::$field0 !== null) {
            throw new LogicException('failed field completion retained a stale result');
        }
    }
    unset($probe, $receiver, $requestHome, $regionHome, $member, $fields);
}

final class __TestStandardPending
{
    public int $getters = 0;
    public int $preparations = 0;
    public int $completions = 0;
    public mixed $original = null;
    public mixed $descriptor = null;
    public mixed $origin = null;
    public ?WeakReference $requestHome = null;
    public ?WeakReference $regionHome = null;
    private __TestStandardReadiness $readiness;

    public function __construct(public readonly int $stream, private readonly bool $usesHome,
        private readonly string $failure = '', public readonly int $dispatchFlags = 0)
    {
        $this->readiness = new __TestStandardReadiness($stream);
    }

    public function source(?__DoriaCell $home = null): mixed
    {
        if (($home !== null) !== $this->usesHome || ($home !== null && $home->value !== $this)) {
            throw new LogicException('fresh getter lost its original receiver home');
        }
        ++$this->getters;
        if ($this->failure === 'getter' && $this->getters === 3) {
            try {
                __doria_throw_io_facts(__DoriaStdIoIoOperation::Read,
                    __DoriaStdIoIoTarget::__doriaCaseStandardInput(), __DoriaStdIoIoErrorReason::Other,
                    0, 0, 0, 'original getter');
            } catch (__DoriaCheckedError $caught) {
                $this->original = $caught->error();
                $this->descriptor = $caught->descriptor;
                $this->origin = $caught->origin;
                throw $caught;
            }
        }
        return $this->usesHome ? __DoriaCell::borrowed($this->readiness) : $this->readiness;
    }

    private function argument(mixed $value, int $flag, ?WeakReference &$home): mixed
    {
        if (($value instanceof __DoriaCell) !== (($this->dispatchFlags & $flag) !== 0)) {
            throw new LogicException('callback lost its checked argument mode');
        }
        if (!$value instanceof __DoriaCell) { return $value; }
        if (!$value->borrowed || !$value->live) { throw new LogicException('callback home is not live'); }
        if ($home === null) { $home = WeakReference::create($value); }
        if ($home->get() !== $value) { throw new LogicException('callback home was rebuilt'); }
        return $value->value;
    }

    public function prepare(mixed $request, mixed $region): void
    {
        $request = $this->argument($request, 2, $this->requestHome);
        $region = $this->argument($region, 4, $this->regionHome);
        ++$this->preparations;
        $stream = $this->failure === 'device' ? 1 : $this->stream;
        $request->__doriaNativePendingRequest()->configureStandard($stream,
            $region->__doriaNativeStorage($this->stream === 0), $region->__doriaRegionOffset(), count($region));
    }

    private function completedRequest(mixed $request, mixed $region): __DoriaPendingRequest
    {
        $request = $this->argument($request, 2, $this->requestHome);
        $region = $this->argument($region, 4, $this->regionHome);
        ++$this->completions;
        return $request->__doriaNativePendingRequest();
    }

    public function complete(mixed $request, mixed $region): mixed
    {
        $request = $this->completedRequest($request, $region);
        return $this->stream === 0 ? $request->readOutcome() : $request->writeOutcome();
    }

    public function completeFields(mixed $request, mixed $region): void
    {
        $request = $this->completedRequest($request, $region);
        if ($this->stream === 0) { $request->readOutcomeResult(); }
        else { $request->writeOutcomeResult(); }
    }
}

$runtime = __DoriaNativeIo::get();
$addressProperties = [];
$homeProperties = [];
foreach (['requestHomes', 'regionHomes'] as $name) {
    $homeProperties[$name] = new ReflectionProperty(__DoriaHostedTransferGroup::class, $name);
}
foreach (['statusAddress', 'terminalAddress', 'viewAddress', 'outcomeAddress',
    'controlAddress', 'callerRead', 'key'] as $name) {
    $property = new ReflectionProperty(__DoriaHostedTransferGroup::class, $name);
    if (!$property->isReadOnly()) { throw new LogicException('group scratch is replaceable'); }
    $addressProperties[$name] = $property;
}
$source = file(__FILE__);
foreach (['drive', 'observe', 'observed', 'readRegion', 'readRegionFields', 'resubmit', 'throwResult'] as $name) {
    $method = new ReflectionMethod(__DoriaHostedTransferGroup::class, $name);
    $body = implode('', array_slice($source, $method->getStartLine() - 1,
        $method->getEndLine() - $method->getStartLine() + 1));
    foreach (['FFI::addr(', '->new(', '->cast(', '->__doriaNativeTransferKey(',
        '__DoriaCell::borrowed('] as $allocation) {
        if (str_contains($body, $allocation)) {
            throw new LogicException('group operation rebuilds native scratch: ' . $name);
        }
    }
}
$method = new ReflectionMethod(__DoriaPendingMember::class, 'dispatch');
$body = implode('', array_slice($source, $method->getStartLine() - 1,
    $method->getEndLine() - $method->getStartLine() + 1));
foreach (['$arguments', '...$'] as $allocation) {
    if (str_contains($body, $allocation)) { throw new LogicException('callback rebuilds argument storage'); }
}
$begin = static function(__TestStandardPending $device, bool $usesHome,
    __DoriaByteRegion $region, FFI\CData &$set): __DoriaHostedTransferGroup {
    $runtime = __DoriaNativeIo::get();
    $status = $runtime->new('DrIoStatusV1');
    $set = $runtime->dr_v1_readiness_set_new(null, FFI::addr($status));
    if ($set === null || FFI::isNull($set)) { throw new LogicException('set creation failed'); }
    $key = $runtime->new('uint64_t[2]');
    $reading = $device->stream === 0;
    if ($runtime->dr_v1_readiness_set_add_host_pending_source($set,
        (new __TestStandardReadiness($device->stream))->__doriaNativeReadinessSource(),
        (int) $reading, (int) !$reading, $key, FFI::addr($status)) !== 0) {
        throw new LogicException('standard registration failed');
    }
    $plan = $runtime->new('void *[1]');
    $message = $runtime->new('DrStringV1 *[1]');
    $path = $runtime->new('DrStringV1 *[1]');
    $system = $runtime->new('int64_t[1]');
    $transferred = $runtime->new('size_t[1]');
    $meta = $runtime->new('uint64_t[1]');
    $storage = $region->__doriaNativeStorage($reading);
    if ($runtime->dr_v1_host_transfer_plan_new(null, $key, $storage->__doriaNativeBase(), count($storage),
        $region->__doriaRegionOffset(), count($region), (int) $reading,
        $plan, $message, $path, $system, $transferred, $meta) !== 0) {
        throw new LogicException('standard plan creation failed');
    }
    $flags = $device->dispatchFlags;
    $complete = ($flags & 8) !== 0 ? 'completeFields' : 'complete';
    $member = new __DoriaPendingMember(__DoriaCell::borrowed($device), 'source', $usesHome,
        $reading ? ['prepare', $flags & 7] : null, $reading ? [$complete, $flags] : null,
        $reading ? null : ['prepare', $flags & 7], $reading ? null : [$complete, $flags]);
    // The third element is the operation's kind: 1 a read, 0 a write.
    return __DoriaHostedTransferGroup::begin($set, $plan[0], [['device', $region, (int) $reading]],
        ['device' => $member], null, null);
};

foreach ([0, 1, 2] as $stream) {
    $bytes = __DoriaBytes::fromArray([255, 0, 81], 0, 0, 'standard pending');
    $region = $stream === 0 ? $bytes->getWritableRegion(0, 3, 0, 0, '')
        : $bytes->getRegion(0, 3, 0, 0, '');
    $usesHome = $stream !== 1;
    $device = new __TestStandardPending($stream, $usesHome, '', ($stream === 0 ? 6 : 0) | 8);
    $set = $runtime->new('void *');
    $group = null;
    $homeReferences = [];
    try {
        $group = $begin($device, $usesHome, $region, $set);
        $addresses = [];
        foreach ($addressProperties as $name => $property) {
            $addresses[$name] = $property->getValue($group);
        }
        foreach ($homeProperties as $name => $property) {
            foreach ($property->getValue($group) as $index => $home) {
                $homeReferences[$name][$index] = WeakReference::create($home);
            }
        }
        unset($home);
        if ($group->observe(1, 0, 0) !== 0 || $group->outcome->tag !== 0 || $group->outcome->count !== 3 ||
            $device->getters !== 3 || $device->preparations !== 1 || $device->completions !== 1) {
            throw new LogicException('standard group changed bytes, outcome or callback count');
        }
        if ($bytes->toArray() !== [255, 0, 81]) { throw new LogicException('binary input changed'); }
        $key = $group->observed($stream === 0 ? 1 : 0);
        if ($key === null) { throw new LogicException('observed key missing'); }
        $keyWords = $runtime->new('int64_t[2]');
        $key->__doriaWriteNativeTransferKey($keyWords);
        $low = $keyWords[0];
        $high = $keyWords[1];
        $keyClass = get_class($key);
        $foreign = $keyClass::__doriaFromTransferKey(-1, PHP_INT_MIN);
        $foreign->__doriaWriteNativeTransferKey($keyWords);
        if ($keyWords[0] !== -1 || $keyWords[1] !== PHP_INT_MIN) {
            throw new LogicException('key writer changed signed identity words');
        }
        try {
            $group->resubmit($foreign, $stream === 0, 0, 0);
            throw new LogicException('foreign transfer key was accepted');
        } catch (__DoriaCheckedError) {}
        $key->__doriaWriteNativeTransferKey($keyWords);
        if ($keyWords[0] !== $low || $keyWords[1] !== $high) {
            throw new LogicException('scratch reuse retargeted a stored key');
        }
        if ($stream === 0) {
            $completed = $group->readRegion($key);
            if ($completed === null || count($completed) !== 3
                || $completed->__doriaNativeStorage(false)->get($completed->__doriaRegionOffset(), 0, 0, '') !== 255) {
                throw new LogicException('completed region changed its original storage');
            }
            unset($completed);
            $group->resubmit($key, true, 0, 3);
            if ($group->observe(1, 0, 0) !== 0 || $group->outcome->tag !== 2 || $group->outcome->count !== 0) {
                throw new LogicException('standard EOF was lost');
            }
        }
        $rounds = 1000;
        for ($round = 0; $round < $rounds; ++$round) {
            $group->resubmit($key, $stream === 0, 0, 0);
            if ($group->observe(1, 0, 0) !== 0 || $group->outcome->tag !== 0 || $group->outcome->count !== 0) {
                throw new LogicException('empty standard transfer changed its outcome');
            }
            $seen = $group->observed($stream === 0 ? 1 : 0);
            if ($seen === null || get_class($seen) !== $keyClass) {
                throw new LogicException('repeated observation lost its directional key');
            }
            $seen->__doriaWriteNativeTransferKey($keyWords);
            if ($keyWords[0] !== $low || $keyWords[1] !== $high) {
                throw new LogicException('observation changed operation identity');
            }
            if ($stream === 0) {
                $completed = $group->readRegion($seen);
                if ($completed === null || count($completed) !== 0
                    || $completed->__doriaNativeStorage(false) === null) {
                    throw new LogicException('empty completed region changed its storage');
                }
                unset($completed);
            }
            foreach ($addressProperties as $name => $property) {
                if ($property->getValue($group) !== $addresses[$name]) {
                    throw new LogicException('group rebuilt native scratch: ' . $name);
                }
            }
            foreach ($homeReferences as $name => $references) {
                foreach ($references as $index => $reference) {
                    if ($reference->get() !== $homeProperties[$name]->getValue($group)[$index]
                        || !$reference->get()->live || !$reference->get()->borrowed) {
                        throw new LogicException('group lost its stable borrowed callback home');
                    }
                }
            }
        }
        $attempts = ($stream === 0 ? 2 : 1) + $rounds;
        if ($device->getters !== 1 + 2 * $attempts || $device->preparations !== $attempts ||
            $device->completions !== $attempts) { throw new LogicException('request reuse changed dispatch'); }
    } finally {
        $group?->close();
        $runtime->dr_v1_readiness_set_drop(null, $set);
        $group?->close();
        foreach ($homeReferences as $references) {
            foreach ($references as $reference) {
                if ($reference->get() !== null) { throw new LogicException('closed group retained a callback home'); }
            }
        }
    }
}

foreach (['device', 'getter'] as $failure) {
    $bytes = __DoriaBytes::allocate(0, 0, 0, '');
    $region = $bytes->getWritableRegion(0, 0, 0, 0, '');
    $device = new __TestStandardPending(0, true, $failure, 6);
    $set = $runtime->new('void *');
    try {
        $group = $begin($device, true, $region, $set);
        $group->close();
        throw new LogicException('invalid configuration unexpectedly succeeded');
    } catch (__DoriaCheckedError $caught) {
        $error = $caught->error();
        if ($device->getters !== 3 || $device->preparations !== 1 || $device->completions !== 0) {
            throw new LogicException('failed preparation acquired a completion obligation');
        }
        if ($failure === 'getter') {
            if ($error !== $device->original ||
                $caught->descriptor !== $device->descriptor || $caught->origin !== $device->origin ||
                $error->systemCode !== 0 || $error->transferredByteCount !== 0) {
                throw new LogicException('fresh getter Error identity or metadata changed');
            }
        } elseif ($error->operation !== __DoriaStdIoIoOperation::Configure ||
            !$error->target->__doriaMatchesCase(2) || $error->reason !== __DoriaStdIoIoErrorReason::InvalidInput ||
            $error->systemCode !== null || $error->transferredByteCount !== null) {
            throw new LogicException('standard configuration Error changed its target or fields');
        }
        $caught->dropError();
        unset($caught);
        if ($device->requestHome === null || $device->regionHome === null
            || $device->requestHome->get() !== null || $device->regionHome->get() !== null) {
            throw new LogicException('failed startup retained its callback homes');
        }
    } finally {
        $runtime->dr_v1_readiness_set_drop(null, $set);
    }
}
