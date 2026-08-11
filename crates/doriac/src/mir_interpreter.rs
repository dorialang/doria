use std::cell::{Cell, Ref, RefCell, RefMut};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;
use std::rc::Rc;

use doria_unicode::{CaseMapping, PadSide, StringError, TrimMode};

use crate::diagnostics::{
    ColorChoice, Diagnostic, DiagnosticFormat, DiagnosticSource, RenderOptions, RuntimeFact,
    RuntimeFactValue, RuntimeOutcomeDetails, RuntimeOutcomeFrame, RuntimeOutcomeOrigin,
    TerminationBehavior,
};
use crate::mir;
use crate::numeric::{FloatType, FloatValue, IntegerPanic, IntegerType, IntegerValue};
use crate::source::Span;

type SharedString = Rc<str>;
type SharedControl = Rc<RefCell<SharedControlValue>>;
type WritableSharedControl = Rc<RefCell<WritableSharedControlValue>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SharedAccessConflict {
    ReadonlyThenWritable,
    WritableThenReadonly,
    WritableThenWritable,
}

impl SharedAccessConflict {
    const fn reason(self) -> &'static str {
        match self {
            Self::ReadonlyThenWritable => {
                doria_diagnostic_catalogue::READONLY_THEN_WRITABLE_CONFLICT
            }
            Self::WritableThenReadonly => {
                doria_diagnostic_catalogue::WRITABLE_THEN_READONLY_CONFLICT
            }
            Self::WritableThenWritable => {
                doria_diagnostic_catalogue::WRITABLE_THEN_WRITABLE_CONFLICT
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct SharedControlValue {
    strong: usize,
    weak: usize,
    payload: Option<(usize, crate::class_layout::ClassId)>,
}

#[derive(Debug, PartialEq, Eq)]
struct WritableSharedControlValue {
    strong: usize,
    weak: usize,
    payload: Option<LocalValue>,
    readonly_accesses: usize,
    writable_access_active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterpreterOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_status: i32,
    pub runtime_diagnostic: Option<Diagnostic>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InterpreterLimits {
    pub max_executed_blocks: Option<usize>,
    pub max_call_frames: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterpreterError {
    pub message: String,
}

impl InterpreterError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

fn register_shared_access(
    control: &WritableSharedControl,
    writable: bool,
) -> Result<Option<SharedAccessConflict>, InterpreterError> {
    let mut state = control.borrow_mut();
    if writable {
        if state.readonly_accesses != 0 {
            return Ok(Some(SharedAccessConflict::ReadonlyThenWritable));
        }
        if state.writable_access_active {
            return Ok(Some(SharedAccessConflict::WritableThenWritable));
        }
        state.strong = state
            .strong
            .checked_add(1)
            .ok_or_else(|| InterpreterError::new("writable shared-reference count overflow"))?;
        state.writable_access_active = true;
    } else {
        if state.writable_access_active {
            return Ok(Some(SharedAccessConflict::WritableThenReadonly));
        }
        let next_readonly = state
            .readonly_accesses
            .checked_add(1)
            .ok_or_else(|| InterpreterError::new("readonly shared-access count overflow"))?;
        let next_strong = state
            .strong
            .checked_add(1)
            .ok_or_else(|| InterpreterError::new("writable shared-reference count overflow"))?;
        state.readonly_accesses = next_readonly;
        state.strong = next_strong;
    }
    Ok(None)
}

impl fmt::Display for InterpreterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for InterpreterError {}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FunctionOutcome {
    Value(LocalValue),
    Void,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LocalValue {
    Scalar(mir::ScalarValue),
    String(SharedString),
    Mixed(MixedValue),
    NullableScalar {
        ty: mir::ScalarType,
        value: Option<mir::ScalarValue>,
    },
    NullableString(Option<SharedString>),
    NullableMixed(Option<MixedValue>),
    Class {
        object: usize,
        class: crate::class_layout::ClassId,
    },
    NullableClass {
        object: Option<usize>,
        class: crate::class_layout::ClassId,
    },
    SharedReference {
        control: SharedControl,
        class: crate::class_layout::ClassId,
    },
    WeakReference {
        control: SharedControl,
        class: crate::class_layout::ClassId,
    },
    NullableSharedReference {
        control: Option<SharedControl>,
        class: crate::class_layout::ClassId,
    },
    NullableWeakReference {
        control: Option<SharedControl>,
        class: crate::class_layout::ClassId,
    },
    WritableSharedReference {
        control: WritableSharedControl,
        payload: mir::WritableSharedPayload,
    },
    WritableWeakReference {
        control: WritableSharedControl,
        payload: mir::WritableSharedPayload,
    },
    NullableWritableSharedReference {
        control: Option<WritableSharedControl>,
        payload: mir::WritableSharedPayload,
    },
    NullableWritableWeakReference {
        control: Option<WritableSharedControl>,
        payload: mir::WritableSharedPayload,
    },
    SharedReferenceAccess {
        control: WritableSharedControl,
        payload: mir::WritableSharedPayload,
        writable: bool,
    },
    NullableSharedReferenceAccess {
        control: Option<WritableSharedControl>,
        payload: mir::WritableSharedPayload,
        writable: bool,
    },
    Collection(CollectionValue),
}

#[derive(Debug, Clone)]
enum OwnedDrop {
    Class {
        object: usize,
        class: crate::class_layout::ClassId,
    },
    Shared(SharedControl),
    Weak(SharedControl),
    WritableShared(WritableSharedControl),
    WritableWeak(WritableSharedControl),
    SharedAccess {
        control: WritableSharedControl,
        writable: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WritableDropKind {
    Strong,
    Weak,
    Access,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum EvaluationValue {
    Scalar(mir::ScalarValue),
    String(SharedString),
    Mixed(MixedValue),
    NullableScalar {
        ty: mir::ScalarType,
        value: Option<mir::ScalarValue>,
    },
    NullableString(Option<SharedString>),
    NullableMixed(Option<MixedValue>),
    Class {
        object: usize,
        class: crate::class_layout::ClassId,
    },
    NullableClass {
        object: Option<usize>,
        class: crate::class_layout::ClassId,
    },
    SharedReference {
        control: SharedControl,
        class: crate::class_layout::ClassId,
    },
    WeakReference {
        control: SharedControl,
        class: crate::class_layout::ClassId,
    },
    NullableSharedReference {
        control: Option<SharedControl>,
        class: crate::class_layout::ClassId,
    },
    NullableWeakReference {
        control: Option<SharedControl>,
        class: crate::class_layout::ClassId,
    },
    WritableSharedReference {
        control: WritableSharedControl,
        payload: mir::WritableSharedPayload,
    },
    WritableWeakReference {
        control: WritableSharedControl,
        payload: mir::WritableSharedPayload,
    },
    NullableWritableSharedReference {
        control: Option<WritableSharedControl>,
        payload: mir::WritableSharedPayload,
    },
    NullableWritableWeakReference {
        control: Option<WritableSharedControl>,
        payload: mir::WritableSharedPayload,
    },
    SharedReferenceAccess {
        control: WritableSharedControl,
        payload: mir::WritableSharedPayload,
        writable: bool,
    },
    NullableSharedReferenceAccess {
        control: Option<WritableSharedControl>,
        payload: mir::WritableSharedPayload,
        writable: bool,
    },
    Collection(CollectionValue),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MixedValue {
    Scalar(mir::ScalarValue),
    String(SharedString),
    Class {
        object: usize,
        class: crate::class_layout::ClassId,
        owner: Rc<Cell<usize>>,
        payload_owned: bool,
    },
}

impl MixedValue {
    fn tag(&self) -> mir::MixedTag {
        match self {
            Self::Scalar(mir::ScalarValue::Bool(_)) => mir::MixedTag::Bool,
            Self::Scalar(mir::ScalarValue::Integer(value)) => mir::MixedTag::Integer(value.ty),
            Self::Scalar(mir::ScalarValue::Float(value)) => mir::MixedTag::Float(value.ty),
            Self::Scalar(mir::ScalarValue::Enum(value)) => mir::MixedTag::Enum(value.enum_id),
            Self::String(_) => mir::MixedTag::String,
            Self::Class { class, .. } => mir::MixedTag::Class(*class),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CollectionValue {
    ty: mir::CollectionTypeId,
    entries: SharedCollectionEntries,
    nullable: bool,
    present: bool,
}

type CollectionEntries = Vec<(Option<LocalValue>, LocalValue)>;
type SharedCollectionEntries = Rc<RefCell<CollectionEntries>>;

impl CollectionValue {
    fn new(ty: mir::CollectionTypeId, entries: CollectionEntries) -> Self {
        Self {
            ty,
            entries: Rc::new(RefCell::new(entries)),
            nullable: false,
            present: true,
        }
    }

    fn nullable(ty: mir::CollectionTypeId, value: Option<Self>) -> Self {
        value.map_or_else(
            || Self {
                ty,
                entries: Rc::new(RefCell::new(Vec::new())),
                nullable: true,
                present: false,
            },
            |mut value| {
                value.nullable = true;
                value.present = true;
                value
            },
        )
    }

    fn assume_non_null(mut self) -> Result<Self, InterpreterError> {
        if !self.present {
            return Err(InterpreterError::new(
                "MIR assumed a null collection was present",
            ));
        }
        self.nullable = false;
        Ok(self)
    }

    fn entries(&self) -> Ref<'_, CollectionEntries> {
        self.entries.borrow()
    }

    fn entries_mut(&self) -> RefMut<'_, CollectionEntries> {
        self.entries.borrow_mut()
    }
}

#[derive(Debug, Clone)]
struct ObjectValue {
    class: crate::class_layout::ClassId,
    properties: Vec<Option<LocalValue>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReturnExpectation {
    Value(mir::Type),
    Discard(mir::Type),
    Void,
}

#[derive(Debug, Clone)]
enum EvaluationTask {
    Rvalue(mir::Rvalue),
    Value(mir::ValueExpression),
    Enum(mir::EnumExpression),
    String(mir::StringExpression),
    ParseNullableScalar(mir::ScalarType),
    Mixed(mir::MixedExpression),
    NullableScalar(mir::NullableScalarExpression),
    NullableString(mir::NullableStringExpression),
    NullableMixed(mir::NullableMixedExpression),
    Class(mir::ClassExpression),
    NullableClass(mir::NullableClassExpression),
    SharedReference(mir::SharedReferenceExpression),
    WeakReference(mir::WeakReferenceExpression),
    NullableSharedReference(mir::NullableSharedReferenceExpression),
    NullableWeakReference(mir::NullableWeakReferenceExpression),
    WritableSharedReference(mir::WritableSharedReferenceExpression),
    WritableWeakReference(mir::WritableWeakReferenceExpression),
    NullableWritableSharedReference(mir::NullableWritableSharedReferenceExpression),
    NullableWritableWeakReference(mir::NullableWritableWeakReferenceExpression),
    SharedReferenceAccess(mir::SharedReferenceAccessExpression),
    NullableSharedReferenceAccess(mir::NullableSharedReferenceAccessExpression),
    AssignGroup(Vec<mir::LocalId>),
    BuildSharedReference(crate::class_layout::ClassId),
    BuildNullableSharedSome(crate::class_layout::ClassId),
    BuildNullableWeakSome(crate::class_layout::ClassId),
    FinishSharedShare(crate::class_layout::ClassId, bool),
    FinishWeakCreation(crate::class_layout::ClassId, bool),
    FinishWeakAcquire(crate::class_layout::ClassId, bool),
    FinishNullSafeShare(crate::class_layout::ClassId, bool),
    FinishNullSafeWeakCreation(crate::class_layout::ClassId, bool),
    FinishNullSafeWeakAcquire(crate::class_layout::ClassId, bool),
    FinishSharedPayload(crate::class_layout::ClassId, bool),
    FinishNullableSharedPayload(crate::class_layout::ClassId, bool),
    BuildWritableSharedReference(mir::WritableSharedPayload),
    BuildNullableWritableSharedSome(mir::WritableSharedPayload),
    BuildNullableWritableWeakSome(mir::WritableSharedPayload),
    FinishWritableSharedShare(mir::WritableSharedPayload, bool),
    FinishWritableWeakCreation(mir::WritableSharedPayload, bool),
    FinishWritableWeakAcquire(mir::WritableSharedPayload, bool),
    FinishWritableNullSafeShare(mir::WritableSharedPayload, bool),
    FinishWritableNullSafeWeakCreation(mir::WritableSharedPayload, bool),
    FinishWritableNullSafeWeakAcquire(mir::WritableSharedPayload, bool),
    FinishSharedAccessAcquire {
        payload: mir::WritableSharedPayload,
        writable: bool,
        drop_receiver: bool,
        span: Span,
    },
    FinishNullableSharedAccessAcquire {
        payload: mir::WritableSharedPayload,
        writable: bool,
        drop_receiver: bool,
        span: Span,
    },
    Collection(mir::CollectionExpression),
    NullableCollection(mir::NullableCollectionExpression),
    BuildNullableCollectionSome(mir::CollectionTypeId),
    FinishNullableCollectionCoalesce {
        right: mir::NullableCollectionExpression,
        transfer: bool,
    },
    BuildCollection {
        collection: mir::CollectionTypeId,
        keyed: Vec<bool>,
    },
    BuildCollectionFill {
        collection: mir::CollectionTypeId,
        count_span: Span,
    },
    LoadCollectionValue {
        collection: mir::LocalId,
        index_span: Span,
        transfer: bool,
        positional: bool,
    },
    CollectionAdd {
        collection: mir::LocalId,
        op: mir::CollectionMutationOp,
        has_index: bool,
    },
    CollectionSet(mir::LocalId),
    AssignCollectionIndex(mir::LocalId, bool),
    CollectionClear(mir::LocalId),
    CollectionHas {
        collection: mir::LocalId,
        op: mir::CollectionMembershipOp,
        ownership: mir::MixedOwnership,
    },
    CollectionIndexOf {
        collection: mir::LocalId,
        ownership: mir::MixedOwnership,
    },
    CollectionIsEmpty(mir::LocalId),
    CollectionLength(mir::LocalId),
    CollectionIndexScalar(mir::LocalId, bool),
    CollectionKeyScalar(mir::LocalId),
    CollectionKeyString(mir::LocalId),
    DictionaryGet {
        collection: mir::LocalId,
        expected: mir::Type,
        access: mir::NullableCollectionAccess,
    },
    CollectionIndexClass {
        collection: mir::LocalId,
        class: crate::class_layout::ClassId,
        transfer: bool,
        positional: bool,
    },
    CollectionIndexShared {
        collection: mir::LocalId,
        class: crate::class_layout::ClassId,
        weak: bool,
        nullable: bool,
        transfer: bool,
        positional: bool,
    },
    CollectionIndexWritableShared {
        collection: mir::LocalId,
        payload: mir::WritableSharedPayload,
        weak: bool,
        nullable: bool,
        transfer: bool,
        positional: bool,
    },
    CollectionIndexSharedAccess {
        collection: mir::LocalId,
        payload: mir::WritableSharedPayload,
        writable: bool,
        nullable: bool,
        remove: bool,
        positional: bool,
    },
    BuildClassNew {
        class: crate::class_layout::ClassId,
        properties: Vec<mir::PropertyValue>,
        constructor: Option<mir::FunctionId>,
        argument_count: usize,
        property_expression_count: usize,
        temporary_arg_drops: Vec<usize>,
    },
    FinishClassNew {
        object: usize,
        class: crate::class_layout::ClassId,
    },
    BuildNullableSome,
    BuildNullableScalarSome(mir::ScalarType),
    BuildNullableClassSome(crate::class_layout::ClassId),
    BuildMixedValue,
    BuildMixedString,
    BuildMixedClass(bool),
    OwnMixed,
    BuildNullableMixedSome,
    WrapNullable(mir::Type),
    AfterNullableMixedCoalesce {
        right: mir::NullableMixedExpression,
        left_ownership: mir::MixedOwnership,
    },
    OwnNullableMixed(mir::MixedOwnership),
    NullableScalarIsPresent,
    NullableClassIsPresent(Option<crate::class_layout::ClassId>),
    NullableCollectionIsPresent(Option<mir::CollectionTypeId>),
    NullableSharedReferenceIsPresent(bool),
    NullableWeakReferenceIsPresent(bool),
    NullableWritableSharedReferenceIsPresent(bool),
    NullableWritableWeakReferenceIsPresent(bool),
    NullableSharedReferenceAccessIsPresent {
        owned: bool,
        writable: bool,
    },
    NullableMixedIsPresent(mir::MixedOwnership),
    MixedIs {
        tag: mir::MixedTag,
        ownership: mir::MixedOwnership,
    },
    AfterIntegerCoalesce(mir::IntegerExpression),
    AfterFloatCoalesce(mir::FloatExpression),
    AfterBoolCoalesce(mir::BoolExpression),
    AfterEnumCoalesce(mir::EnumExpression),
    AfterStringCoalesce(mir::StringExpression),
    EnumBackingInt(crate::enums::EnumId),
    EnumBackingString(crate::enums::EnumId),
    AfterNullableScalarCoalesce(mir::NullableScalarExpression),
    AfterNullableStringCoalesce(mir::NullableStringExpression),
    AfterClassCoalesce {
        right: mir::ClassExpression,
        left_owned: bool,
        transfer: bool,
    },
    FinishClassCoalesceRight(Option<crate::class_layout::ClassId>),
    AfterNullableClassCoalesce {
        right: mir::NullableClassExpression,
        left_owned: bool,
        transfer: bool,
    },
    FinishNullableClassCoalesceRight(Option<crate::class_layout::ClassId>),
    AfterSharedCoalesce {
        right: mir::SharedReferenceExpression,
        left_owned: bool,
        transfer: bool,
    },
    FinishSharedCoalesceRight(bool),
    AfterNullableSharedCoalesce {
        right: mir::NullableSharedReferenceExpression,
        left_owned: bool,
        transfer: bool,
    },
    FinishNullableSharedCoalesceRight(bool),
    AfterWeakCoalesce {
        right: mir::WeakReferenceExpression,
        left_owned: bool,
        transfer: bool,
    },
    FinishWeakCoalesceRight(bool),
    AfterNullableWeakCoalesce {
        right: mir::NullableWeakReferenceExpression,
        left_owned: bool,
        transfer: bool,
    },
    FinishNullableWeakCoalesceRight(bool),
    AfterWritableSharedCoalesce {
        right: mir::WritableSharedReferenceExpression,
        left_owned: bool,
        transfer: bool,
    },
    FinishWritableSharedCoalesceRight(bool),
    AfterNullableWritableSharedCoalesce {
        right: mir::NullableWritableSharedReferenceExpression,
        left_owned: bool,
        transfer: bool,
    },
    FinishNullableWritableSharedCoalesceRight(bool),
    AfterWritableWeakCoalesce {
        right: mir::WritableWeakReferenceExpression,
        left_owned: bool,
        transfer: bool,
    },
    FinishWritableWeakCoalesceRight(bool),
    AfterNullableWritableWeakCoalesce {
        right: mir::NullableWritableWeakReferenceExpression,
        left_owned: bool,
        transfer: bool,
    },
    FinishNullableWritableWeakCoalesceRight(bool),
    AfterNullSafeProperty {
        property: crate::class_layout::PropertyId,
        result: mir::Type,
        owned_receiver: Option<crate::class_layout::ClassId>,
    },
    AfterNullSafeCall {
        function: mir::FunctionId,
        args: Vec<mir::Rvalue>,
        result: mir::Type,
        owned_receiver: Option<crate::class_layout::ClassId>,
    },
    AfterNullSafeStatementCall {
        function: mir::FunctionId,
        args: Vec<mir::Rvalue>,
        owned_receiver: Option<crate::class_layout::ClassId>,
        call_site: Option<Span>,
    },
    NullableStringCompare(mir::CompareOp),
    Format(mir::FormatExpression),
    BuildFormat(mir::FormatExpression),
    ReadFile(Span),
    ReadLine(Span),
    WriteFile,
    AppendFile,
    ReadFileBytes(mir::CollectionTypeId, Span),
    WriteFileBytes {
        contents: mir::LocalId,
        append: bool,
    },
    WriteStreamBytes {
        contents: mir::LocalId,
        stderr: bool,
    },
    WriteStderr,
    StringConcat(usize),
    StringIntrinsic {
        kind: mir::StringIntrinsicKind,
        result: mir::Type,
        argument_count: usize,
        span: Span,
        argument_spans: Vec<Span>,
    },
    StringDisplay,
    StringCompare(mir::CompareOp),
    Echo,
    PanicString(Span),
    Integer(mir::IntegerExpression),
    IntegerUnary {
        op: mir::IntegerUnaryOp,
        span: Span,
    },
    IntegerBinary {
        op: mir::IntegerBinaryOp,
        span: Span,
        right_span: Span,
    },
    IntegerConvert {
        target: IntegerType,
        operation_span: Span,
        primary_span: Span,
    },
    FloatToInt {
        operation_span: Span,
        primary_span: Span,
    },
    Float(mir::FloatExpression),
    FloatNegate,
    FloatBinary(mir::FloatBinaryOp),
    IntToFloat,
    Bool(mir::BoolExpression),
    Compare(mir::CompareOp),
    Not,
    AfterAnd(mir::BoolExpression),
    AfterOr(mir::BoolExpression),
    Xor,
    Invoke {
        function: mir::FunctionId,
        argument_count: usize,
        expectation: ReturnExpectation,
        temporary_arg_drops: Vec<usize>,
        call_site: Option<Span>,
    },
    FinishStatement,
    DropTemporaryValues(Vec<OwnedDrop>),
    Assign(mir::LocalId),
    AssignStatic(mir::StaticId),
    AssignProperty {
        object: mir::LocalId,
        property: crate::class_layout::PropertyId,
    },
    DropClass(mir::LocalId),
    DropShared(mir::LocalId),
    DropWeak(mir::LocalId),
    DropWritableShared(mir::LocalId),
    DropWritableWeak(mir::LocalId),
    DropSharedAccess(mir::LocalId),
    ReleaseShared(SharedControl),
    ReleaseWeak(SharedControl),
    ReleaseWritableShared(WritableSharedControl),
    ReleaseWritableWeak(WritableSharedControl),
    ReleaseSharedAccess {
        control: WritableSharedControl,
        writable: bool,
    },
    DropCollection(mir::LocalId),
    DropObject {
        object: usize,
        class: crate::class_layout::ClassId,
    },
    DropObjectProperties {
        object: usize,
        class: crate::class_layout::ClassId,
    },
    FreeObject {
        object: usize,
        class: crate::class_layout::ClassId,
    },
    CleanupFrame,
    ReturnValue(mir::Type),
    ReturnVoid,
    Branch {
        then_block: mir::BlockId,
        else_block: mir::BlockId,
    },
}

struct CallFrame {
    function: mir::FunctionId,
    block: mir::BlockId,
    statement_index: usize,
    entered_block: bool,
    locals: Vec<Option<LocalValue>>,
    tasks: Vec<EvaluationTask>,
    values: Vec<EvaluationValue>,
    statement_temporary_drops: Vec<OwnedDrop>,
    caller_expectation: Option<ReturnExpectation>,
    entered_from: Option<Span>,
}

struct Interpreter<'program> {
    program: &'program mir::Program,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    stdin: Vec<u8>,
    stdin_cursor: usize,
    io_faults: MirIoFaults,
    io_trace: MirIoTrace,
    files: BTreeMap<String, Vec<u8>>,
    heap: BTreeMap<usize, ObjectValue>,
    statics: Vec<LocalValue>,
    next_object: usize,
    frames: Vec<CallFrame>,
    limits: InterpreterLimits,
    executed_blocks: usize,
    pending_panic: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirIoWriteFailure {
    BrokenPipe,
    Other,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MirIoFaults {
    pub prompt_write: Option<MirIoWriteFailure>,
    pub stdout_flush: Option<MirIoWriteFailure>,
    pub stdin_line_read: bool,
    pub line_allocation: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MirIoTrace {
    pub prompt_writes: usize,
    pub stdout_flushes: usize,
    pub stdin_line_reads: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MirIo {
    pub stdin: Vec<u8>,
    pub files: BTreeMap<String, Vec<u8>>,
    /// Program arguments handed to a `main(List<string> $args)` (decision
    /// 0099). These are the arguments only — the executable path is stripped by
    /// the entry glue, so element 0 is the first real argument.
    pub args: Vec<String>,
    /// Deterministic host failures used to verify the shared standard-I/O
    /// contract without depending on platform-specific devices.
    pub faults: MirIoFaults,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterpreterIoOutput {
    pub output: InterpreterOutput,
    pub files: BTreeMap<String, Vec<u8>>,
    pub trace: MirIoTrace,
}

enum StepOutcome {
    Continue,
    CleanExit,
    EntryReturned(FunctionOutcome),
    RuntimePanic(RuntimePanicEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimePanicEvent {
    code: &'static str,
    operation_span: Span,
    primary_span: Span,
    facts: Vec<RuntimeFact>,
    explanation: Option<String>,
}

enum CollectionAccessError {
    Catalogued(&'static str),
    Bounds {
        code: &'static str,
        index: i64,
        length: usize,
    },
}

pub fn interpret(program: &mir::Program) -> Result<InterpreterOutput, InterpreterError> {
    Ok(interpret_with_io(program, MirIo::default())?.output)
}

pub fn interpret_with_io(
    program: &mir::Program,
    io: MirIo,
) -> Result<InterpreterIoOutput, InterpreterError> {
    interpret_internal(program, InterpreterLimits::default(), io)
}

pub fn interpret_with_limits(
    program: &mir::Program,
    limits: InterpreterLimits,
) -> Result<InterpreterOutput, InterpreterError> {
    Ok(interpret_internal(program, limits, MirIo::default())?.output)
}

fn interpret_internal(
    program: &mir::Program,
    limits: InterpreterLimits,
    io: MirIo,
) -> Result<InterpreterIoOutput, InterpreterError> {
    crate::mir_validation::validate_program(program)
        .map_err(|error| InterpreterError::new(error.message))?;
    let entry = function_in(program, program.entry)?;
    // Decision 0099: the entry takes either no parameters or one `List<string>`
    // of program arguments, which the glue owns and lends to `main`.
    let entry_argument_collection = match entry.params.as_slice() {
        [] => None,
        [parameter] => match entry.locals[parameter.0].ty {
            mir::Type::Collection(collection) => Some(collection),
            _ => {
                return Err(InterpreterError::new(
                    "MIR entry parameter must be the `List<string>` argument list",
                ));
            }
        },
        _ => {
            return Err(InterpreterError::new(
                "MIR entry function declares more than one parameter",
            ));
        }
    };
    let entry_arguments = entry_argument_collection.map(|collection| {
        LocalValue::Collection(CollectionValue::new(
            collection,
            io.args
                .iter()
                .map(|argument| {
                    (
                        None,
                        LocalValue::String(SharedString::from(argument.as_str())),
                    )
                })
                .collect(),
        ))
    });

    let mut interpreter = Interpreter {
        program,
        stdout: Vec::new(),
        stderr: Vec::new(),
        stdin: io.stdin,
        stdin_cursor: 0,
        io_faults: io.faults,
        io_trace: MirIoTrace::default(),
        files: io.files,
        heap: BTreeMap::new(),
        statics: program
            .statics
            .iter()
            .map(|property| match &property.initializer {
                mir::StaticValue::Scalar(value)
                    if property.ty == mir::Type::NullableScalar(value.ty()) =>
                {
                    LocalValue::NullableScalar {
                        ty: value.ty(),
                        value: Some(*value),
                    }
                }
                mir::StaticValue::Scalar(value) => LocalValue::Scalar(*value),
                mir::StaticValue::String(value) if property.ty == mir::Type::String => {
                    LocalValue::String(SharedString::from(value.as_str()))
                }
                mir::StaticValue::String(value) => {
                    LocalValue::NullableString(Some(SharedString::from(value.as_str())))
                }
                mir::StaticValue::Null => match property.ty {
                    mir::Type::NullableScalar(ty) => LocalValue::NullableScalar { ty, value: None },
                    mir::Type::NullableString => LocalValue::NullableString(None),
                    mir::Type::NullableClass(class) => LocalValue::NullableClass {
                        object: None,
                        class,
                    },
                    mir::Type::NullableSharedReference(class) => {
                        LocalValue::NullableSharedReference {
                            control: None,
                            class,
                        }
                    }
                    mir::Type::NullableWeakReference(class) => LocalValue::NullableWeakReference {
                        control: None,
                        class,
                    },
                    _ => LocalValue::NullableString(None),
                },
            })
            .collect(),
        next_object: 1,
        frames: Vec::new(),
        limits,
        executed_blocks: 0,
        pending_panic: None,
    };
    let entry_frame_arguments: Vec<LocalValue> = entry_arguments.into_iter().collect();
    interpreter.push_frame(program.entry, &entry_frame_arguments, None, None)?;

    loop {
        match interpreter.step()? {
            StepOutcome::Continue => {}
            StepOutcome::CleanExit => {
                return Ok(InterpreterIoOutput {
                    output: InterpreterOutput {
                        stdout: interpreter.stdout,
                        stderr: interpreter.stderr,
                        exit_status: 0,
                        runtime_diagnostic: None,
                    },
                    files: interpreter.files,
                    trace: interpreter.io_trace,
                });
            }
            StepOutcome::RuntimePanic(event) => {
                let output = interpreter.runtime_panic_output(event);
                return Ok(InterpreterIoOutput {
                    output,
                    files: interpreter.files,
                    trace: interpreter.io_trace,
                });
            }
            StepOutcome::EntryReturned(outcome) => {
                let output = interpreter.finish_entry(entry, outcome)?;
                return Ok(InterpreterIoOutput {
                    output,
                    files: interpreter.files,
                    trace: interpreter.io_trace,
                });
            }
        }
    }
}

impl Interpreter<'_> {
    fn step(&mut self) -> Result<StepOutcome, InterpreterError> {
        if let Some(code) = self.pending_panic.take() {
            return self.runtime_panic_step(code);
        }
        let task = self.frames.last_mut().and_then(|frame| frame.tasks.pop());
        if let Some(task) = task {
            return self.execute_task(task);
        }

        self.enter_current_block()?;
        let (function_id, block_id, statement_index) = {
            let frame = self.current_frame()?;
            (frame.function, frame.block, frame.statement_index)
        };
        let function = function_in(self.program, function_id)?;
        let block = block_in(function, block_id)?;

        if let Some(statement) = block.statements.get(statement_index).cloned() {
            self.current_frame_mut()?.statement_index += 1;
            self.current_frame_mut()?
                .tasks
                .push(EvaluationTask::FinishStatement);
            return self.execute_statement(function, statement);
        }

        self.execute_terminator(function, block.terminator.clone())
    }

    fn enter_current_block(&mut self) -> Result<(), InterpreterError> {
        if self.current_frame()?.entered_block {
            return Ok(());
        }
        if let Some(limit) = self.limits.max_executed_blocks {
            if self.executed_blocks >= limit {
                return Err(InterpreterError::new(format!(
                    "MIR interpreter reached the explicit test execution limit of {limit} basic blocks"
                )));
            }
        }
        self.executed_blocks += 1;
        self.current_frame_mut()?.entered_block = true;
        Ok(())
    }

    fn execute_statement(
        &mut self,
        function: &mir::Function,
        statement: mir::Statement,
    ) -> Result<StepOutcome, InterpreterError> {
        match statement {
            mir::Statement::AssignLocalGroup { targets, value } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AssignGroup(targets));
                frame.tasks.push(EvaluationTask::Rvalue(value));
            }
            mir::Statement::AssignLocal { target, value } => {
                let definition = local_in(function, target)?;
                match (definition.ty, value) {
                    (mir::Type::String, mir::Rvalue::String(expression)) => {
                        let frame = self.current_frame_mut()?;
                        frame.tasks.push(EvaluationTask::Assign(target));
                        frame.tasks.push(EvaluationTask::String(expression));
                    }
                    (mir::Type::String, _) => {
                        return Err(InterpreterError::new(format!(
                            "MIR string local local{} received a non-string value",
                            target.0
                        )));
                    }
                    (mir::Type::NullableString, mir::Rvalue::NullableString(expression)) => {
                        let frame = self.current_frame_mut()?;
                        frame.tasks.push(EvaluationTask::Assign(target));
                        frame.tasks.push(EvaluationTask::NullableString(expression));
                    }
                    (mir::Type::NullableString, _) => {
                        return Err(InterpreterError::new(format!(
                            "MIR nullable-string local local{} received another value type",
                            target.0
                        )));
                    }
                    (mir::Type::Mixed, mir::Rvalue::Mixed(expression)) => {
                        let frame = self.current_frame_mut()?;
                        frame.tasks.push(EvaluationTask::Assign(target));
                        frame.tasks.push(EvaluationTask::Mixed(expression));
                    }
                    (mir::Type::Mixed, _) => {
                        return Err(InterpreterError::new(format!(
                            "MIR mixed local local{} received another value type",
                            target.0
                        )));
                    }
                    (mir::Type::NullableMixed, mir::Rvalue::NullableMixed(expression)) => {
                        let frame = self.current_frame_mut()?;
                        frame.tasks.push(EvaluationTask::Assign(target));
                        frame.tasks.push(EvaluationTask::NullableMixed(expression));
                    }
                    (mir::Type::NullableMixed, _) => {
                        return Err(InterpreterError::new(format!(
                            "MIR nullable-mixed local local{} received another value type",
                            target.0
                        )));
                    }
                    (
                        mir::Type::NullableScalar(expected),
                        mir::Rvalue::NullableScalar(expression),
                    ) if expression.ty() == expected => {
                        let frame = self.current_frame_mut()?;
                        frame.tasks.push(EvaluationTask::Assign(target));
                        frame.tasks.push(EvaluationTask::NullableScalar(expression));
                    }
                    (mir::Type::NullableScalar(_), _) => {
                        return Err(InterpreterError::new(format!(
                            "MIR nullable scalar local local{} received another value type",
                            target.0
                        )));
                    }
                    (mir::Type::Scalar(expected), mir::Rvalue::Value(expression)) => {
                        if expression.ty() != expected {
                            return Err(InterpreterError::new(format!(
                                "MIR scalar local local{} has type {expected}, but its rvalue has type {}",
                                target.0,
                                expression.ty()
                            )));
                        }
                        self.queue_value_assignment(target, expression)?;
                    }
                    (mir::Type::Scalar(_), _) => {
                        return Err(InterpreterError::new(format!(
                            "MIR scalar local local{} received a string value",
                            target.0
                        )));
                    }
                    (mir::Type::Class(expected), mir::Rvalue::Class(expression))
                        if expression.class() == expected =>
                    {
                        let frame = self.current_frame_mut()?;
                        frame.tasks.push(EvaluationTask::Assign(target));
                        frame.tasks.push(EvaluationTask::Class(expression));
                    }
                    (
                        mir::Type::NullableClass(expected),
                        mir::Rvalue::NullableClass(expression),
                    ) if expression.class() == expected => {
                        let frame = self.current_frame_mut()?;
                        frame.tasks.push(EvaluationTask::Assign(target));
                        frame.tasks.push(EvaluationTask::NullableClass(expression));
                    }
                    (mir::Type::NullableClass(_), _) => {
                        return Err(InterpreterError::new(format!(
                            "MIR nullable class local local{} received another value type",
                            target.0
                        )));
                    }
                    (mir::Type::Class(_), _) => {
                        return Err(InterpreterError::new(
                            "MIR class local received a non-class value",
                        ));
                    }
                    (
                        mir::Type::SharedReference(expected),
                        mir::Rvalue::SharedReference(expression),
                    ) if expression.class() == expected => {
                        let frame = self.current_frame_mut()?;
                        frame.tasks.push(EvaluationTask::Assign(target));
                        frame
                            .tasks
                            .push(EvaluationTask::SharedReference(expression));
                    }
                    (
                        mir::Type::WeakReference(expected),
                        mir::Rvalue::WeakReference(expression),
                    ) if expression.class() == expected => {
                        let frame = self.current_frame_mut()?;
                        frame.tasks.push(EvaluationTask::Assign(target));
                        frame.tasks.push(EvaluationTask::WeakReference(expression));
                    }
                    (
                        mir::Type::NullableSharedReference(expected),
                        mir::Rvalue::NullableSharedReference(expression),
                    ) if expression.class() == expected => {
                        let frame = self.current_frame_mut()?;
                        frame.tasks.push(EvaluationTask::Assign(target));
                        frame
                            .tasks
                            .push(EvaluationTask::NullableSharedReference(expression));
                    }
                    (
                        mir::Type::NullableWeakReference(expected),
                        mir::Rvalue::NullableWeakReference(expression),
                    ) if expression.class() == expected => {
                        let frame = self.current_frame_mut()?;
                        frame.tasks.push(EvaluationTask::Assign(target));
                        frame
                            .tasks
                            .push(EvaluationTask::NullableWeakReference(expression));
                    }
                    (
                        mir::Type::WritableSharedReference(expected),
                        mir::Rvalue::WritableSharedReference(expression),
                    ) if expression.payload() == expected => {
                        let frame = self.current_frame_mut()?;
                        frame.tasks.push(EvaluationTask::Assign(target));
                        frame
                            .tasks
                            .push(EvaluationTask::WritableSharedReference(expression));
                    }
                    (
                        mir::Type::WritableWeakReference(expected),
                        mir::Rvalue::WritableWeakReference(expression),
                    ) if expression.payload() == expected => {
                        let frame = self.current_frame_mut()?;
                        frame.tasks.push(EvaluationTask::Assign(target));
                        frame
                            .tasks
                            .push(EvaluationTask::WritableWeakReference(expression));
                    }
                    (
                        mir::Type::NullableWritableSharedReference(expected),
                        mir::Rvalue::NullableWritableSharedReference(expression),
                    ) if expression.payload() == expected => {
                        let frame = self.current_frame_mut()?;
                        frame.tasks.push(EvaluationTask::Assign(target));
                        frame
                            .tasks
                            .push(EvaluationTask::NullableWritableSharedReference(expression));
                    }
                    (
                        mir::Type::NullableWritableWeakReference(expected),
                        mir::Rvalue::NullableWritableWeakReference(expression),
                    ) if expression.payload() == expected => {
                        let frame = self.current_frame_mut()?;
                        frame.tasks.push(EvaluationTask::Assign(target));
                        frame
                            .tasks
                            .push(EvaluationTask::NullableWritableWeakReference(expression));
                    }
                    (
                        mir::Type::ReadonlySharedReferenceAccess(expected),
                        mir::Rvalue::SharedReferenceAccess(expression),
                    ) if expression.payload() == expected && !expression.writable() => {
                        let frame = self.current_frame_mut()?;
                        frame.tasks.push(EvaluationTask::Assign(target));
                        frame
                            .tasks
                            .push(EvaluationTask::SharedReferenceAccess(expression));
                    }
                    (
                        mir::Type::WritableSharedReferenceAccess(expected),
                        mir::Rvalue::SharedReferenceAccess(expression),
                    ) if expression.payload() == expected && expression.writable() => {
                        let frame = self.current_frame_mut()?;
                        frame.tasks.push(EvaluationTask::Assign(target));
                        frame
                            .tasks
                            .push(EvaluationTask::SharedReferenceAccess(expression));
                    }
                    (
                        mir::Type::NullableReadonlySharedReferenceAccess(expected),
                        mir::Rvalue::NullableSharedReferenceAccess(expression),
                    ) if expression.payload() == expected && !expression.writable() => {
                        let frame = self.current_frame_mut()?;
                        frame.tasks.push(EvaluationTask::Assign(target));
                        frame
                            .tasks
                            .push(EvaluationTask::NullableSharedReferenceAccess(expression));
                    }
                    (
                        mir::Type::NullableWritableSharedReferenceAccess(expected),
                        mir::Rvalue::NullableSharedReferenceAccess(expression),
                    ) if expression.payload() == expected && expression.writable() => {
                        let frame = self.current_frame_mut()?;
                        frame.tasks.push(EvaluationTask::Assign(target));
                        frame
                            .tasks
                            .push(EvaluationTask::NullableSharedReferenceAccess(expression));
                    }
                    (
                        mir::Type::SharedReference(_)
                        | mir::Type::WeakReference(_)
                        | mir::Type::NullableSharedReference(_)
                        | mir::Type::NullableWeakReference(_)
                        | mir::Type::WritableSharedReference(_)
                        | mir::Type::WritableWeakReference(_)
                        | mir::Type::NullableWritableSharedReference(_)
                        | mir::Type::NullableWritableWeakReference(_)
                        | mir::Type::ReadonlySharedReferenceAccess(_)
                        | mir::Type::WritableSharedReferenceAccess(_)
                        | mir::Type::NullableReadonlySharedReferenceAccess(_)
                        | mir::Type::NullableWritableSharedReferenceAccess(_),
                        _,
                    ) => {
                        return Err(InterpreterError::new(
                            "MIR shared-handle local received another value type",
                        ));
                    }
                    (mir::Type::Collection(expected), mir::Rvalue::Collection(expression))
                        if expression.collection() == expected =>
                    {
                        let frame = self.current_frame_mut()?;
                        frame.tasks.push(EvaluationTask::Assign(target));
                        frame.tasks.push(EvaluationTask::Collection(expression));
                    }
                    (mir::Type::Collection(_), _) => {
                        return Err(InterpreterError::new(
                            "MIR collection local received a non-collection value",
                        ));
                    }
                    (
                        mir::Type::NullableCollection(expected),
                        mir::Rvalue::NullableCollection(expression),
                    ) if expression.collection() == expected => {
                        let frame = self.current_frame_mut()?;
                        frame.tasks.push(EvaluationTask::Assign(target));
                        frame
                            .tasks
                            .push(EvaluationTask::NullableCollection(expression));
                    }
                    (mir::Type::NullableCollection(_), _) => {
                        return Err(InterpreterError::new(
                            "MIR nullable-collection local received another value type",
                        ));
                    }
                }
            }
            mir::Statement::EchoStringLiteral(value) => {
                self.stdout.extend_from_slice(value.as_bytes());
            }
            mir::Statement::EchoString(expression) => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::Echo);
                frame.tasks.push(EvaluationTask::String(expression));
            }
            mir::Statement::CallVoid {
                function,
                args,
                span,
            } => {
                self.queue_call_at(function, args, ReturnExpectation::Void, Some(span))?;
            }
            mir::Statement::CallBorrowed {
                function,
                args,
                span,
            } => {
                let callee = function_in(self.program, function)?;
                let mir::ReturnType::Value(return_type) = callee.return_type else {
                    return Err(InterpreterError::new(
                        "MIR borrowed call targeted a void function",
                    ));
                };
                self.queue_call_at(
                    function,
                    args,
                    ReturnExpectation::Discard(return_type),
                    Some(span),
                )?;
            }
            mir::Statement::CallNullSafe {
                object,
                function,
                args,
                span,
            } => {
                let owned_receiver = object.owned_temporary_class();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::AfterNullSafeStatementCall {
                        function,
                        args,
                        owned_receiver,
                        call_site: Some(span),
                    });
                frame.tasks.push(EvaluationTask::NullableClass(object));
            }
            mir::Statement::Printf(format) => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::Echo);
                frame.tasks.push(EvaluationTask::Format(format));
            }
            mir::Statement::WriteFile { path, contents } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::WriteFile);
                frame.tasks.push(EvaluationTask::String(contents));
                frame.tasks.push(EvaluationTask::String(path));
            }
            mir::Statement::AppendFile { path, contents } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AppendFile);
                frame.tasks.push(EvaluationTask::String(contents));
                frame.tasks.push(EvaluationTask::String(path));
            }
            mir::Statement::WriteFileBytes {
                path,
                contents,
                append,
            } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::WriteFileBytes { contents, append });
                frame.tasks.push(EvaluationTask::String(path));
            }
            mir::Statement::WriteStreamBytes { contents, stderr } => {
                self.current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::WriteStreamBytes { contents, stderr });
            }
            mir::Statement::WriteStderr(value) => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::WriteStderr);
                frame.tasks.push(EvaluationTask::String(value));
            }
            mir::Statement::AssignProperty {
                object,
                property,
                value,
            } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::AssignProperty { object, property });
                frame.tasks.push(EvaluationTask::Rvalue(value));
            }
            mir::Statement::AssignStatic { target, value } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AssignStatic(target));
                frame.tasks.push(EvaluationTask::Rvalue(value));
            }
            mir::Statement::DropClass { local, .. } => {
                self.current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::DropClass(local));
            }
            mir::Statement::DropSharedReference { local, .. } => {
                self.current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::DropShared(local));
            }
            mir::Statement::DropWeakReference { local, .. } => {
                self.current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::DropWeak(local));
            }
            mir::Statement::DropWritableSharedReference { local, .. } => {
                self.current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::DropWritableShared(local));
            }
            mir::Statement::DropWritableWeakReference { local, .. } => {
                self.current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::DropWritableWeak(local));
            }
            mir::Statement::DropSharedReferenceAccess { local, .. } => {
                self.current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::DropSharedAccess(local));
            }
            mir::Statement::DropString { local } => {
                let value = self
                    .current_frame_mut()?
                    .locals
                    .get_mut(local.0)
                    .and_then(Option::take)
                    .ok_or_else(|| {
                        InterpreterError::new("string temporary was dropped before initialization")
                    })?;
                if !matches!(value, LocalValue::String(_)) {
                    return Err(InterpreterError::new(
                        "string drop references a non-string local",
                    ));
                }
            }
            mir::Statement::DropMixed { local } => {
                self.drop_mixed_local(local)?;
            }
            mir::Statement::CollectionAdd {
                collection,
                value,
                index,
                op,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::CollectionAdd {
                    collection,
                    op,
                    has_index: index.is_some(),
                });
                frame.tasks.push(EvaluationTask::Rvalue(value));
                if let Some(index) = index {
                    frame.tasks.push(EvaluationTask::Rvalue(index));
                }
            }
            mir::Statement::CollectionSet {
                collection,
                key,
                value,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::CollectionSet(collection));
                frame.tasks.push(EvaluationTask::Rvalue(value));
                frame.tasks.push(EvaluationTask::Rvalue(key));
            }
            mir::Statement::AssignCollectionIndex {
                positional,
                collection,
                index,
                value,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AssignCollectionIndex(
                    collection, positional,
                ));
                frame.tasks.push(EvaluationTask::Rvalue(value));
                frame.tasks.push(EvaluationTask::Rvalue(index));
            }
            mir::Statement::CollectionClear { collection, .. } => {
                self.current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::CollectionClear(collection));
            }
            mir::Statement::DropCollection { local, .. } => {
                self.current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::DropCollection(local));
            }
        }
        Ok(StepOutcome::Continue)
    }

    fn execute_terminator(
        &mut self,
        function: &mir::Function,
        terminator: mir::Terminator,
    ) -> Result<StepOutcome, InterpreterError> {
        match terminator {
            mir::Terminator::Return(operand) => {
                let mir::ReturnType::Value(expected) = function.return_type else {
                    return Err(InterpreterError::new(format!(
                        "MIR void function {} returned a scalar value",
                        function.name
                    )));
                };
                if operand.ty() != expected {
                    return Err(InterpreterError::new(format!(
                        "MIR function {} returns {expected}, but its return expression has type {}",
                        function.name,
                        operand.ty()
                    )));
                }
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::ReturnValue(expected));
                frame.tasks.push(EvaluationTask::CleanupFrame);
                frame.tasks.push(EvaluationTask::FinishStatement);
                frame.tasks.push(EvaluationTask::Rvalue(operand));
                Ok(StepOutcome::Continue)
            }
            mir::Terminator::ReturnVoid => {
                if function.return_type != mir::ReturnType::Void {
                    return Err(InterpreterError::new(format!(
                        "MIR int function {} returned void",
                        function.name
                    )));
                }
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::ReturnVoid);
                frame.tasks.push(EvaluationTask::CleanupFrame);
                Ok(StepOutcome::Continue)
            }
            mir::Terminator::Panic { message, span } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::PanicString(span));
                frame.tasks.push(EvaluationTask::String(message));
                Ok(StepOutcome::Continue)
            }
            mir::Terminator::Unreachable => Err(InterpreterError::new(format!(
                "MIR reached an unreachable block in function {}",
                function.name
            ))),
            mir::Terminator::Jump(target) => {
                self.move_to_block(function, target)?;
                Ok(StepOutcome::Continue)
            }
            mir::Terminator::Branch {
                condition,
                then_block,
                else_block,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::Branch {
                    then_block,
                    else_block,
                });
                frame.tasks.push(EvaluationTask::FinishStatement);
                frame.tasks.push(EvaluationTask::Bool(condition));
                Ok(StepOutcome::Continue)
            }
        }
    }

    fn execute_task(&mut self, task: EvaluationTask) -> Result<StepOutcome, InterpreterError> {
        match task {
            EvaluationTask::Rvalue(expression) => match expression {
                mir::Rvalue::Value(value) => self
                    .current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::Value(value)),
                mir::Rvalue::String(value) => self
                    .current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::String(value)),
                mir::Rvalue::Mixed(value) => self
                    .current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::Mixed(value)),
                mir::Rvalue::NullableScalar(value) => self
                    .current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::NullableScalar(value)),
                mir::Rvalue::NullableString(value) => self
                    .current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::NullableString(value)),
                mir::Rvalue::NullableMixed(value) => self
                    .current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::NullableMixed(value)),
                mir::Rvalue::Class(value) => self
                    .current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::Class(value)),
                mir::Rvalue::NullableClass(value) => self
                    .current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::NullableClass(value)),
                mir::Rvalue::SharedReference(value) => self
                    .current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::SharedReference(value)),
                mir::Rvalue::WeakReference(value) => self
                    .current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::WeakReference(value)),
                mir::Rvalue::NullableSharedReference(value) => self
                    .current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::NullableSharedReference(value)),
                mir::Rvalue::NullableWeakReference(value) => self
                    .current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::NullableWeakReference(value)),
                mir::Rvalue::WritableSharedReference(value) => self
                    .current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::WritableSharedReference(value)),
                mir::Rvalue::WritableWeakReference(value) => self
                    .current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::WritableWeakReference(value)),
                mir::Rvalue::NullableWritableSharedReference(value) => self
                    .current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::NullableWritableSharedReference(value)),
                mir::Rvalue::NullableWritableWeakReference(value) => self
                    .current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::NullableWritableWeakReference(value)),
                mir::Rvalue::SharedReferenceAccess(value) => self
                    .current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::SharedReferenceAccess(value)),
                mir::Rvalue::NullableSharedReferenceAccess(value) => self
                    .current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::NullableSharedReferenceAccess(value)),
                mir::Rvalue::Collection(value) => self
                    .current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::Collection(value)),
                mir::Rvalue::NullableCollection(value) => self
                    .current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::NullableCollection(value)),
            },
            EvaluationTask::Value(expression) => match expression {
                mir::ValueExpression::Integer(value) => {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::Integer(value));
                }
                mir::ValueExpression::Float(value) => {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::Float(value));
                }
                mir::ValueExpression::Bool(value) => {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::Bool(value));
                }
                mir::ValueExpression::Enum(value) => {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::Enum(value));
                }
            },
            EvaluationTask::Enum(expression) => self.expand_enum_expression(expression)?,
            EvaluationTask::String(expression) => self.expand_string_expression(expression)?,
            EvaluationTask::Mixed(expression) => self.expand_mixed_expression(expression)?,
            EvaluationTask::NullableScalar(expression) => {
                self.expand_nullable_scalar_expression(expression)?
            }
            EvaluationTask::NullableString(expression) => {
                self.expand_nullable_string_expression(expression)?
            }
            EvaluationTask::NullableMixed(expression) => {
                self.expand_nullable_mixed_expression(expression)?
            }
            EvaluationTask::Class(expression) => self.expand_class_expression(expression)?,
            EvaluationTask::NullableClass(expression) => {
                self.expand_nullable_class_expression(expression)?
            }
            EvaluationTask::SharedReference(expression) => {
                self.expand_shared_reference_expression(expression)?
            }
            EvaluationTask::WeakReference(expression) => {
                self.expand_weak_reference_expression(expression)?
            }
            EvaluationTask::NullableSharedReference(expression) => {
                self.expand_nullable_shared_reference_expression(expression)?
            }
            EvaluationTask::NullableWeakReference(expression) => {
                self.expand_nullable_weak_reference_expression(expression)?
            }
            EvaluationTask::WritableSharedReference(expression) => {
                self.expand_writable_shared_reference_expression(expression)?
            }
            EvaluationTask::WritableWeakReference(expression) => {
                self.expand_writable_weak_reference_expression(expression)?
            }
            EvaluationTask::NullableWritableSharedReference(expression) => {
                self.expand_nullable_writable_shared_reference_expression(expression)?
            }
            EvaluationTask::NullableWritableWeakReference(expression) => {
                self.expand_nullable_writable_weak_reference_expression(expression)?
            }
            EvaluationTask::SharedReferenceAccess(expression) => {
                self.expand_shared_reference_access_expression(expression)?
            }
            EvaluationTask::NullableSharedReferenceAccess(expression) => {
                self.expand_nullable_shared_reference_access_expression(expression)?
            }
            EvaluationTask::BuildSharedReference(class) => {
                let value = self.pop_local_value()?;
                let LocalValue::Class {
                    object,
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "shared construction did not produce a class payload",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "shared construction produced another payload class",
                    ));
                }
                let control = Rc::new(RefCell::new(SharedControlValue {
                    strong: 1,
                    weak: 0,
                    payload: Some((object, class)),
                }));
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::SharedReference { control, class });
            }
            EvaluationTask::BuildNullableSharedSome(class) => {
                let value = self.pop_local_value()?;
                let LocalValue::SharedReference {
                    control,
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "nullable shared construction received another value type",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "nullable shared construction changed payload class",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::NullableSharedReference {
                        control: Some(control),
                        class,
                    });
            }
            EvaluationTask::BuildNullableWeakSome(class) => {
                let value = self.pop_local_value()?;
                let LocalValue::WeakReference {
                    control,
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "nullable weak construction received another value type",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "nullable weak construction changed payload class",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::NullableWeakReference {
                        control: Some(control),
                        class,
                    });
            }
            EvaluationTask::FinishSharedShare(class, drop_receiver) => {
                let value = self.pop_local_value()?;
                let LocalValue::SharedReference {
                    control,
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new("share received a non-strong handle"));
                };
                if actual != class {
                    return Err(InterpreterError::new("share changed payload class"));
                }
                let mut state = control.borrow_mut();
                state.strong = state
                    .strong
                    .checked_add(1)
                    .ok_or_else(|| InterpreterError::new("shared-reference count overflow"))?;
                drop(state);
                if drop_receiver {
                    self.current_frame_mut()?
                        .statement_temporary_drops
                        .push(OwnedDrop::Shared(control.clone()));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::SharedReference { control, class });
            }
            EvaluationTask::FinishWeakCreation(class, drop_receiver) => {
                let value = self.pop_local_value()?;
                let LocalValue::SharedReference {
                    control,
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "weak creation received a non-strong handle",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new("weak creation changed payload class"));
                }
                let mut state = control.borrow_mut();
                state.weak = state
                    .weak
                    .checked_add(1)
                    .ok_or_else(|| InterpreterError::new("weak-reference count overflow"))?;
                drop(state);
                if drop_receiver {
                    self.current_frame_mut()?
                        .statement_temporary_drops
                        .push(OwnedDrop::Shared(control.clone()));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::WeakReference { control, class });
            }
            EvaluationTask::FinishWeakAcquire(class, drop_receiver) => {
                let value = self.pop_local_value()?;
                let LocalValue::WeakReference {
                    control,
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new("acquire received a non-weak handle"));
                };
                if actual != class {
                    return Err(InterpreterError::new("acquire changed payload class"));
                }
                let acquired = {
                    let mut state = control.borrow_mut();
                    if state.strong == 0 {
                        false
                    } else {
                        state.strong = state.strong.checked_add(1).ok_or_else(|| {
                            InterpreterError::new("shared-reference count overflow")
                        })?;
                        true
                    }
                };
                if drop_receiver {
                    self.current_frame_mut()?
                        .statement_temporary_drops
                        .push(OwnedDrop::Weak(control.clone()));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::NullableSharedReference {
                        control: acquired.then_some(control),
                        class,
                    });
            }
            EvaluationTask::FinishNullSafeShare(class, drop_receiver) => {
                let value = self.pop_local_value()?;
                let LocalValue::NullableSharedReference {
                    control,
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "null-safe share received a non-nullable strong handle",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "null-safe share changed payload class",
                    ));
                }
                if let Some(control) = &control {
                    let mut state = control.borrow_mut();
                    state.strong = state
                        .strong
                        .checked_add(1)
                        .ok_or_else(|| InterpreterError::new("shared-reference count overflow"))?;
                }
                if drop_receiver {
                    if let Some(control) = &control {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push(OwnedDrop::Shared(control.clone()));
                    }
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::NullableSharedReference { control, class });
            }
            EvaluationTask::FinishNullSafeWeakCreation(class, drop_receiver) => {
                let value = self.pop_local_value()?;
                let LocalValue::NullableSharedReference {
                    control,
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "null-safe weak creation received a non-nullable strong handle",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "null-safe weak creation changed payload class",
                    ));
                }
                if let Some(control) = &control {
                    let mut state = control.borrow_mut();
                    state.weak = state
                        .weak
                        .checked_add(1)
                        .ok_or_else(|| InterpreterError::new("weak-reference count overflow"))?;
                }
                if drop_receiver {
                    if let Some(control) = &control {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push(OwnedDrop::Shared(control.clone()));
                    }
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::NullableWeakReference { control, class });
            }
            EvaluationTask::FinishNullSafeWeakAcquire(class, drop_receiver) => {
                let value = self.pop_local_value()?;
                let LocalValue::NullableWeakReference {
                    control,
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "null-safe acquire received a non-nullable weak handle",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "null-safe acquire changed payload class",
                    ));
                }
                let acquired = if let Some(control) = &control {
                    let mut state = control.borrow_mut();
                    if state.strong == 0 {
                        None
                    } else {
                        state.strong = state.strong.checked_add(1).ok_or_else(|| {
                            InterpreterError::new("shared-reference count overflow")
                        })?;
                        Some(control.clone())
                    }
                } else {
                    None
                };
                if drop_receiver {
                    if let Some(control) = &control {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push(OwnedDrop::Weak(control.clone()));
                    }
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::NullableSharedReference {
                        control: acquired,
                        class,
                    });
            }
            EvaluationTask::FinishSharedPayload(class, drop_receiver) => {
                let value = self.pop_local_value()?;
                let LocalValue::SharedReference {
                    control,
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "payload projection received a non-strong handle",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "payload projection changed payload class",
                    ));
                }
                let (object, actual) = control
                    .borrow()
                    .payload
                    .ok_or_else(|| InterpreterError::new("strong handle has no live payload"))?;
                if drop_receiver {
                    self.current_frame_mut()?
                        .statement_temporary_drops
                        .push(OwnedDrop::Shared(control));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Class {
                        object,
                        class: actual,
                    });
            }
            EvaluationTask::FinishNullableSharedPayload(class, drop_receiver) => {
                let value = self.pop_local_value()?;
                let LocalValue::NullableSharedReference {
                    control,
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "nullable payload projection received another type",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "nullable payload projection changed payload class",
                    ));
                }
                let object = control
                    .as_ref()
                    .map(|control| {
                        control.borrow().payload.ok_or_else(|| {
                            InterpreterError::new("strong handle has no live payload")
                        })
                    })
                    .transpose()?;
                if drop_receiver {
                    if let Some(control) = control {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push(OwnedDrop::Shared(control));
                    }
                }
                self.push_nullable_class(class, object.map(|(object, _)| object))?;
            }
            EvaluationTask::BuildWritableSharedReference(payload) => {
                let value = self.pop_local_value()?;
                if local_value_type(&value) != writable_payload_type(payload) {
                    return Err(InterpreterError::new(
                        "writable shared construction produced another payload type",
                    ));
                }
                let control = Rc::new(RefCell::new(WritableSharedControlValue {
                    strong: 1,
                    weak: 0,
                    payload: Some(value),
                    readonly_accesses: 0,
                    writable_access_active: false,
                }));
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::WritableSharedReference { control, payload });
            }
            EvaluationTask::BuildNullableWritableSharedSome(payload) => {
                let (control, actual) = self.pop_writable_shared_reference()?;
                if actual != payload {
                    return Err(InterpreterError::new(
                        "nullable writable shared construction changed payload type",
                    ));
                }
                self.current_frame_mut()?.values.push(
                    EvaluationValue::NullableWritableSharedReference {
                        control: Some(control),
                        payload,
                    },
                );
            }
            EvaluationTask::BuildNullableWritableWeakSome(payload) => {
                let (control, actual) = self.pop_writable_weak_reference()?;
                if actual != payload {
                    return Err(InterpreterError::new(
                        "nullable writable weak construction changed payload type",
                    ));
                }
                self.current_frame_mut()?.values.push(
                    EvaluationValue::NullableWritableWeakReference {
                        control: Some(control),
                        payload,
                    },
                );
            }
            EvaluationTask::FinishWritableSharedShare(payload, drop_receiver) => {
                let (control, actual) = self.pop_writable_shared_reference()?;
                if actual != payload {
                    return Err(InterpreterError::new("writable share changed payload type"));
                }
                {
                    let mut state = control.borrow_mut();
                    state.strong = state.strong.checked_add(1).ok_or_else(|| {
                        InterpreterError::new("writable shared-reference count overflow")
                    })?;
                }
                if drop_receiver {
                    self.current_frame_mut()?
                        .statement_temporary_drops
                        .push(OwnedDrop::WritableShared(control.clone()));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::WritableSharedReference { control, payload });
            }
            EvaluationTask::FinishWritableWeakCreation(payload, drop_receiver) => {
                let (control, actual) = self.pop_writable_shared_reference()?;
                if actual != payload {
                    return Err(InterpreterError::new(
                        "writable weak creation changed payload type",
                    ));
                }
                {
                    let mut state = control.borrow_mut();
                    state.weak = state.weak.checked_add(1).ok_or_else(|| {
                        InterpreterError::new("writable weak-reference count overflow")
                    })?;
                }
                if drop_receiver {
                    self.current_frame_mut()?
                        .statement_temporary_drops
                        .push(OwnedDrop::WritableShared(control.clone()));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::WritableWeakReference { control, payload });
            }
            EvaluationTask::FinishWritableWeakAcquire(payload, drop_receiver) => {
                let (control, actual) = self.pop_writable_weak_reference()?;
                if actual != payload {
                    return Err(InterpreterError::new(
                        "writable weak acquisition changed payload type",
                    ));
                }
                let acquired = {
                    let mut state = control.borrow_mut();
                    if state.strong == 0 {
                        false
                    } else {
                        state.strong = state.strong.checked_add(1).ok_or_else(|| {
                            InterpreterError::new("writable shared-reference count overflow")
                        })?;
                        true
                    }
                };
                if drop_receiver {
                    self.current_frame_mut()?
                        .statement_temporary_drops
                        .push(OwnedDrop::WritableWeak(control.clone()));
                }
                self.current_frame_mut()?.values.push(
                    EvaluationValue::NullableWritableSharedReference {
                        control: acquired.then_some(control),
                        payload,
                    },
                );
            }
            EvaluationTask::FinishWritableNullSafeShare(payload, drop_receiver) => {
                let (control, actual) = self.pop_nullable_writable_shared_reference()?;
                if actual != payload {
                    return Err(InterpreterError::new(
                        "null-safe writable share changed payload type",
                    ));
                }
                if let Some(control) = &control {
                    let mut state = control.borrow_mut();
                    state.strong = state.strong.checked_add(1).ok_or_else(|| {
                        InterpreterError::new("writable shared-reference count overflow")
                    })?;
                }
                if drop_receiver {
                    if let Some(control) = &control {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push(OwnedDrop::WritableShared(control.clone()));
                    }
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::NullableWritableSharedReference { control, payload });
            }
            EvaluationTask::FinishWritableNullSafeWeakCreation(payload, drop_receiver) => {
                let (control, actual) = self.pop_nullable_writable_shared_reference()?;
                if actual != payload {
                    return Err(InterpreterError::new(
                        "null-safe writable weak creation changed payload type",
                    ));
                }
                if let Some(control) = &control {
                    let mut state = control.borrow_mut();
                    state.weak = state.weak.checked_add(1).ok_or_else(|| {
                        InterpreterError::new("writable weak-reference count overflow")
                    })?;
                }
                if drop_receiver {
                    if let Some(control) = &control {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push(OwnedDrop::WritableShared(control.clone()));
                    }
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::NullableWritableWeakReference { control, payload });
            }
            EvaluationTask::FinishWritableNullSafeWeakAcquire(payload, drop_receiver) => {
                let (control, actual) = self.pop_nullable_writable_weak_reference()?;
                if actual != payload {
                    return Err(InterpreterError::new(
                        "null-safe writable weak acquisition changed payload type",
                    ));
                }
                let acquired = if let Some(control) = &control {
                    let mut state = control.borrow_mut();
                    if state.strong == 0 {
                        None
                    } else {
                        state.strong = state.strong.checked_add(1).ok_or_else(|| {
                            InterpreterError::new("writable shared-reference count overflow")
                        })?;
                        Some(control.clone())
                    }
                } else {
                    None
                };
                if drop_receiver {
                    if let Some(control) = &control {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push(OwnedDrop::WritableWeak(control.clone()));
                    }
                }
                self.current_frame_mut()?.values.push(
                    EvaluationValue::NullableWritableSharedReference {
                        control: acquired,
                        payload,
                    },
                );
            }
            EvaluationTask::FinishSharedAccessAcquire {
                payload,
                writable,
                drop_receiver,
                span,
            } => {
                let (control, actual) = self.pop_writable_shared_reference()?;
                if actual != payload {
                    return Err(InterpreterError::new(
                        "shared access acquisition changed payload type",
                    ));
                }
                if let Some(conflict) = register_shared_access(&control, writable)? {
                    return self.shared_access_conflict_panic_step_at(conflict, span);
                }
                if drop_receiver {
                    self.current_frame_mut()?
                        .statement_temporary_drops
                        .push(OwnedDrop::WritableShared(control.clone()));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::SharedReferenceAccess {
                        control,
                        payload,
                        writable,
                    });
            }
            EvaluationTask::FinishNullableSharedAccessAcquire {
                payload,
                writable,
                drop_receiver,
                span,
            } => {
                let (control, actual) = self.pop_nullable_writable_shared_reference()?;
                if actual != payload {
                    return Err(InterpreterError::new(
                        "nullable shared access acquisition changed payload type",
                    ));
                }
                let Some(control) = control else {
                    self.current_frame_mut()?.values.push(
                        EvaluationValue::NullableSharedReferenceAccess {
                            control: None,
                            payload,
                            writable,
                        },
                    );
                    return Ok(StepOutcome::Continue);
                };
                if let Some(conflict) = register_shared_access(&control, writable)? {
                    return self.shared_access_conflict_panic_step_at(conflict, span);
                }
                if drop_receiver {
                    self.current_frame_mut()?
                        .statement_temporary_drops
                        .push(OwnedDrop::WritableShared(control.clone()));
                }
                self.current_frame_mut()?.values.push(
                    EvaluationValue::NullableSharedReferenceAccess {
                        control: Some(control),
                        payload,
                        writable,
                    },
                );
            }
            EvaluationTask::Collection(expression) => {
                self.expand_collection_expression(expression)?
            }
            EvaluationTask::NullableCollection(expression) => {
                self.expand_nullable_collection_expression(expression)?
            }
            EvaluationTask::BuildNullableCollectionSome(collection) => {
                let value = self.pop_collection_value()?.assume_non_null()?;
                if value.ty != collection {
                    return Err(InterpreterError::new(
                        "MIR nullable collection wrapper has another collection type",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Collection(CollectionValue::nullable(
                        collection,
                        Some(value),
                    )));
            }
            EvaluationTask::FinishNullableCollectionCoalesce { right, transfer } => {
                let value = self.pop_collection_value()?;
                if value.present {
                    let value = if transfer {
                        value
                    } else {
                        CollectionValue::nullable(value.ty, Some(value.assume_non_null()?))
                    };
                    self.current_frame_mut()?
                        .values
                        .push(EvaluationValue::Collection(value));
                } else {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::NullableCollection(right));
                }
            }
            EvaluationTask::BuildCollection { collection, keyed } => {
                let definition = self
                    .program
                    .collection_types
                    .get(collection.0)
                    .ok_or_else(|| InterpreterError::new("collection type does not exist"))?;
                let key_type = definition.key;
                let value_type = definition.value;
                let value_count = keyed.len() + keyed.iter().filter(|keyed| **keyed).count();
                let values = self.take_call_arguments(value_count)?;
                let mut values = values.into_iter();
                let mut entries: Vec<(Option<LocalValue>, LocalValue)> =
                    Vec::with_capacity(keyed.len());
                let kind = self.program.collection_types[collection.0].kind;
                let unique = matches!(
                    kind,
                    mir::CollectionKind::Set | mir::CollectionKind::SortedSet
                );
                let mut drops = Vec::new();
                for keyed in keyed {
                    let key = keyed.then(|| values.next()).flatten();
                    let value = values.next().ok_or_else(|| {
                        InterpreterError::new("MIR collection literal produced too few values")
                    })?;
                    if unique
                        && entries.iter().any(|(_, current)| {
                            collection_values_equal(value_type, current, &value)
                        })
                    {
                        if let Some(key) = key {
                            collect_owned_objects_from_value(key, &mut drops);
                        }
                        collect_owned_objects_from_value(value, &mut drops);
                    } else if let Some(position) = key.as_ref().and_then(|key| {
                        entries.iter().position(|(current, _)| {
                            current.as_ref().is_some_and(|current| {
                                key_type.is_some_and(|ty| collection_values_equal(ty, current, key))
                            })
                        })
                    }) {
                        let (old_key, old_value) =
                            std::mem::replace(&mut entries[position], (key, value));
                        if let Some(old_key) = old_key {
                            collect_owned_objects_from_value(old_key, &mut drops);
                        }
                        collect_owned_objects_from_value(old_value, &mut drops);
                    } else {
                        entries.push((key, value));
                    }
                }
                for drop in drops {
                    self.push_owned_drop_task(drop)?;
                }
                order_collection_entries(definition, &mut entries)?;
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Collection(CollectionValue::new(
                        collection, entries,
                    )));
            }
            EvaluationTask::BuildCollectionFill {
                collection,
                count_span,
            } => {
                let count = self.pop_local_value()?;
                let LocalValue::Scalar(mir::ScalarValue::Integer(count)) = count else {
                    return Err(InterpreterError::new(
                        "MIR collection fill count produced another value type",
                    ));
                };
                if count.ty != IntegerType::Int64 {
                    return Err(InterpreterError::new(
                        "MIR collection fill count is not canonical int",
                    ));
                }
                let count = count.signed_value();
                if count < 0 {
                    return self.runtime_panic_step_with_facts_at(
                        "P1311",
                        count_span,
                        vec![RuntimeFact {
                            name: doria_diagnostic_catalogue::COLLECTION_FILL_COUNT_FACT
                                .to_string(),
                            value: RuntimeFactValue::Signed(count as i64),
                        }],
                    );
                }
                let count = usize::try_from(count).map_err(|_| {
                    InterpreterError::new("MIR collection fill count exceeds host capacity")
                })?;
                let value = self.pop_local_value()?;
                if !matches!(value, LocalValue::Scalar(_) | LocalValue::String(_)) {
                    return Err(InterpreterError::new(
                        "MIR collection fill value is not a Copy scalar or string",
                    ));
                }
                let entries = match repeat_collection_entries(value, count) {
                    Ok(entries) => entries,
                    Err(code) => return self.runtime_panic_step_at(code, count_span),
                };
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Collection(CollectionValue::new(
                        collection, entries,
                    )));
            }
            EvaluationTask::LoadCollectionValue {
                collection,
                index_span,
                transfer,
                positional,
            } => {
                let index = self.pop_local_value()?;
                let value = match self.collection_value_at(collection, &index, transfer, positional)
                {
                    Ok(value) => value,
                    Err(error) => return self.collection_access_panic_step_at(error, index_span),
                };
                self.push_local_value(value)?;
            }
            EvaluationTask::CollectionAdd {
                collection,
                op,
                has_index,
            } => {
                let value = self.pop_local_value()?;
                let index = has_index
                    .then(|| self.pop_collection_offset())
                    .transpose()?;
                let collection_kind = {
                    let collection = self.collection_local(collection)?;
                    self.program.collection_types[collection.ty.0].kind
                };
                match op {
                    mir::CollectionMutationOp::Add => {
                        let collection = self.collection_local(collection)?;
                        if collection
                            .entries()
                            .iter()
                            .any(|(_, current)| current == &value)
                            && matches!(
                                collection_kind,
                                mir::CollectionKind::Set | mir::CollectionKind::SortedSet
                            )
                        {
                            self.queue_value_drops(value)?;
                            return Ok(StepOutcome::Continue);
                        }
                        collection.entries_mut().push((None, value));
                        if collection_kind == mir::CollectionKind::SortedSet {
                            let definition = &self.program.collection_types[collection.ty.0];
                            order_collection_entries(definition, &mut collection.entries_mut())?;
                        }
                    }
                    mir::CollectionMutationOp::InsertAt => {
                        let index = index.expect("insertAt task carries an index");
                        let collection = self.collection_local(collection)?;
                        let length = collection.entries().len();
                        if index > length {
                            return self.collection_access_panic_step(
                                CollectionAccessError::Bounds {
                                    code: "P1310",
                                    index: index as i64,
                                    length,
                                },
                            );
                        }
                        collection.entries_mut().insert(index, (None, value));
                    }
                    mir::CollectionMutationOp::Remove => {
                        let position = self
                            .collection_local(collection)?
                            .entries()
                            .iter()
                            .position(|(_, current)| current == &value);
                        if let Some(position) = position {
                            let (_, removed) = self
                                .collection_local(collection)?
                                .entries_mut()
                                .remove(position);
                            self.queue_value_drops(removed)?;
                        }
                        self.queue_value_drops(value)?;
                    }
                    mir::CollectionMutationOp::Push => {
                        let collection = self.collection_local(collection)?;
                        collection.entries_mut().push((None, value));
                        let definition = &self.program.collection_types[collection.ty.0];
                        order_collection_entries(definition, &mut collection.entries_mut())?;
                    }
                    mir::CollectionMutationOp::PushFront => {
                        self.collection_local(collection)?
                            .entries_mut()
                            .insert(0, (None, value));
                    }
                    mir::CollectionMutationOp::PushBack => {
                        self.collection_local(collection)?
                            .entries_mut()
                            .push((None, value));
                    }
                }
            }
            EvaluationTask::CollectionSet(collection) => {
                let value = self.pop_local_value()?;
                let key = self.pop_local_value()?;
                let position = self
                    .collection_local(collection)?
                    .entries()
                    .iter()
                    .position(|(current, _)| current.as_ref() == Some(&key));
                if let Some(position) = position {
                    let current = self.collection_local(collection)?;
                    let old = std::mem::replace(&mut current.entries_mut()[position].1, value);
                    self.queue_value_drops(old)?;
                    self.queue_value_drops(key)?;
                } else {
                    self.collection_local(collection)?
                        .entries_mut()
                        .push((Some(key), value));
                }
                let collection = self.collection_local(collection)?;
                if self.program.collection_types[collection.ty.0].kind
                    == mir::CollectionKind::SortedDictionary
                {
                    let definition = &self.program.collection_types[collection.ty.0];
                    order_collection_entries(definition, &mut collection.entries_mut())?;
                }
            }
            EvaluationTask::AssignCollectionIndex(collection, positional) => {
                let value = self.pop_local_value()?;
                let index = self.pop_local_value()?;
                let keyed = {
                    let collection = self.collection_local(collection)?;
                    self.program.collection_types[collection.ty.0].key.is_some()
                };
                match self.collection_position(collection, &index, positional) {
                    Ok(position) => {
                        let current = self.collection_local(collection)?;
                        let old = std::mem::replace(&mut current.entries_mut()[position].1, value);
                        self.queue_value_drops(old)?;
                    }
                    Err(_) if keyed => {
                        self.collection_local(collection)?
                            .entries_mut()
                            .push((Some(index), value));
                    }
                    Err(error) => return self.collection_access_panic_step(error),
                }
                let collection = self.collection_local(collection)?;
                if self.program.collection_types[collection.ty.0].kind
                    == mir::CollectionKind::SortedDictionary
                {
                    let definition = &self.program.collection_types[collection.ty.0];
                    order_collection_entries(definition, &mut collection.entries_mut())?;
                }
            }
            EvaluationTask::CollectionHas {
                collection,
                op,
                ownership,
            } => {
                let needle = self.pop_local_value()?;
                let (found, needle_type) = {
                    let collection = self.collection_local(collection)?;
                    let definition = &self.program.collection_types[collection.ty.0];
                    let needle_type = if op == mir::CollectionMembershipOp::Contains {
                        definition.key.unwrap_or(definition.value)
                    } else {
                        definition.value
                    };
                    if op == mir::CollectionMembershipOp::Contains && definition.key.is_some() {
                        (
                            collection.entries().iter().any(|(key, _)| {
                                key.as_ref().is_some_and(|key| {
                                    collection_values_equal(needle_type, key, &needle)
                                })
                            }),
                            needle_type,
                        )
                    } else {
                        (
                            collection.entries().iter().any(|(_, value)| {
                                collection_values_equal(needle_type, value, &needle)
                            }),
                            needle_type,
                        )
                    }
                };
                if op == mir::CollectionMembershipOp::Add {
                    if found {
                        if ownership == mir::MixedOwnership::Owned {
                            self.queue_value_drops(needle)?;
                        }
                    } else {
                        let collection = self.collection_local(collection)?;
                        collection.entries_mut().push((None, needle));
                        if self.program.collection_types[collection.ty.0].kind
                            == mir::CollectionKind::SortedSet
                        {
                            let definition = &self.program.collection_types[collection.ty.0];
                            order_collection_entries(definition, &mut collection.entries_mut())?;
                        }
                    }
                    self.push_scalar(mir::ScalarValue::Bool(!found))?;
                    return Ok(StepOutcome::Continue);
                }
                let result = match op {
                    mir::CollectionMembershipOp::Contains
                    | mir::CollectionMembershipOp::ContainsValue => found,
                    mir::CollectionMembershipOp::Remove => {
                        let position = {
                            let collection = self.collection_local(collection)?;
                            collection.entries().iter().position(|(_, value)| {
                                collection_values_equal(needle_type, value, &needle)
                            })
                        };
                        if let Some(position) = position {
                            let (_, removed) = self
                                .collection_local(collection)?
                                .entries_mut()
                                .remove(position);
                            self.queue_value_drops(removed)?;
                            true
                        } else {
                            false
                        }
                    }
                    mir::CollectionMembershipOp::Add => unreachable!("handled above"),
                };
                if ownership == mir::MixedOwnership::Owned {
                    self.queue_value_drops(needle)?;
                }
                self.push_scalar(mir::ScalarValue::Bool(result))?;
            }
            EvaluationTask::CollectionIndexOf {
                collection,
                ownership,
            } => {
                let needle = self.pop_local_value()?;
                let position = {
                    let collection = self.collection_local(collection)?;
                    let definition = &self.program.collection_types[collection.ty.0];
                    collection.entries().iter().position(|(_, value)| {
                        collection_values_equal(definition.value, value, &needle)
                    })
                };
                if ownership == mir::MixedOwnership::Owned {
                    self.queue_value_drops(needle)?;
                }
                let value = position.map(|position| {
                    mir::ScalarValue::Integer(
                        crate::numeric::IntegerValue::from_i128(
                            crate::numeric::IntegerType::Int64,
                            position as i128,
                        )
                        .expect("collection position fits Doria int"),
                    )
                });
                self.push_nullable_scalar(
                    mir::ScalarType::Integer(crate::numeric::IntegerType::Int64),
                    value,
                )?;
            }
            EvaluationTask::CollectionIsEmpty(collection) => {
                let empty = self.collection_local(collection)?.entries().is_empty();
                self.push_scalar(mir::ScalarValue::Bool(empty))?;
            }
            EvaluationTask::CollectionLength(collection) => {
                let length = self.collection_local(collection)?.entries().len();
                self.push_scalar(mir::ScalarValue::Integer(
                    IntegerValue::from_i128(IntegerType::Int64, length as i128)
                        .expect("collection length fits interpreter address space"),
                ))?;
            }
            EvaluationTask::CollectionIndexScalar(collection, positional) => {
                let index = self.pop_local_value()?;
                let value = match self.collection_value_at(collection, &index, false, positional) {
                    Ok(value) => value,
                    Err(error) => return self.collection_access_panic_step(error),
                };
                let LocalValue::Scalar(value) = value else {
                    return Err(InterpreterError::new(
                        "MIR indexed scalar produced another value type",
                    ));
                };
                self.push_scalar(value)?;
            }
            EvaluationTask::CollectionKeyScalar(collection) => {
                let offset = self.pop_collection_offset()?;
                let value = self.collection_key_at(collection, offset)?;
                let LocalValue::Scalar(value) = value else {
                    return Err(InterpreterError::new(
                        "MIR collection key produced another value type",
                    ));
                };
                self.push_scalar(value)?;
            }
            EvaluationTask::CollectionKeyString(collection) => {
                let offset = self.pop_collection_offset()?;
                let value = self.collection_key_at(collection, offset)?;
                let LocalValue::String(value) = value else {
                    return Err(InterpreterError::new(
                        "MIR collection key produced another value type",
                    ));
                };
                self.push_string(value)?;
            }
            EvaluationTask::DictionaryGet {
                collection,
                expected,
                access,
            } => {
                let key = self.pop_local_value()?;
                let value = match access {
                    mir::NullableCollectionAccess::Get => self
                        .collection_local(collection)?
                        .entries()
                        .iter()
                        .find(|(current, _)| current.as_ref() == Some(&key))
                        .map(|(_, value)| value.clone()),
                    mir::NullableCollectionAccess::Index => {
                        let value = self
                            .collection_local(collection)?
                            .entries()
                            .iter()
                            .find(|(current, _)| current.as_ref() == Some(&key))
                            .map(|(_, value)| value.clone());
                        let Some(value) = value else {
                            return self.collection_access_panic_step(
                                CollectionAccessError::Catalogued("P1312"),
                            );
                        };
                        Some(value)
                    }
                    mir::NullableCollectionAccess::Remove => {
                        let position = self
                            .collection_local(collection)?
                            .entries()
                            .iter()
                            .position(|(current, _)| current.as_ref() == Some(&key));
                        if let Some(position) = position {
                            let (removed_key, value) = self
                                .collection_local(collection)?
                                .entries_mut()
                                .remove(position);
                            if let Some(removed_key) = removed_key {
                                self.queue_value_drops(removed_key)?;
                            }
                            Some(value)
                        } else {
                            None
                        }
                    }
                    mir::NullableCollectionAccess::First => self
                        .collection_local(collection)?
                        .entries()
                        .first()
                        .map(|(_, value)| value.clone()),
                    mir::NullableCollectionAccess::Last => self
                        .collection_local(collection)?
                        .entries()
                        .last()
                        .map(|(_, value)| value.clone()),
                    mir::NullableCollectionAccess::Pop => {
                        let collection = self.collection_local(collection)?;
                        if self.program.collection_types[collection.ty.0].kind
                            == mir::CollectionKind::PriorityQueue
                        {
                            let empty = collection.entries().is_empty();
                            (!empty).then(|| collection.entries_mut().remove(0).1)
                        } else {
                            collection.entries_mut().pop().map(|(_, value)| value)
                        }
                    }
                    mir::NullableCollectionAccess::PopFront => {
                        let collection = self.collection_local(collection)?;
                        let empty = collection.entries().is_empty();
                        (!empty).then(|| collection.entries_mut().remove(0).1)
                    }
                    mir::NullableCollectionAccess::PopBack => self
                        .collection_local(collection)?
                        .entries_mut()
                        .pop()
                        .map(|(_, value)| value),
                    mir::NullableCollectionAccess::At => {
                        let LocalValue::Scalar(mir::ScalarValue::Integer(offset)) = key else {
                            return Err(InterpreterError::new(
                                "MIR collection offset is not an integer",
                            ));
                        };
                        let offset = usize::try_from(offset.signed_value()).map_err(|_| {
                            InterpreterError::new("MIR collection offset is negative")
                        })?;
                        self.collection_local(collection)?
                            .entries()
                            .get(offset)
                            .map(|(_, value)| value.clone())
                    }
                };
                match (expected, value) {
                    (mir::Type::Scalar(ty), Some(LocalValue::Scalar(value)))
                        if value.ty() == ty =>
                    {
                        self.push_nullable_scalar(ty, Some(value))?;
                    }
                    (mir::Type::Scalar(ty), None) => {
                        self.push_nullable_scalar(ty, None)?;
                    }
                    (
                        mir::Type::Scalar(ty),
                        Some(LocalValue::NullableScalar { ty: actual, value }),
                    ) if ty == actual => self.push_nullable_scalar(ty, value)?,
                    (mir::Type::String, Some(LocalValue::String(value))) => {
                        self.push_nullable_string(Some(value))?;
                    }
                    (mir::Type::String, None) => {
                        self.push_nullable_string(None)?;
                    }
                    (mir::Type::String, Some(LocalValue::NullableString(value))) => {
                        self.push_nullable_string(value)?;
                    }
                    (
                        mir::Type::Class(class),
                        Some(LocalValue::Class {
                            object,
                            class: actual,
                        }),
                    ) if class == actual => {
                        self.push_nullable_class(class, Some(object))?;
                    }
                    (mir::Type::Class(class), None) => {
                        self.push_nullable_class(class, None)?;
                    }
                    (
                        mir::Type::Class(class),
                        Some(LocalValue::NullableClass {
                            object,
                            class: actual,
                        }),
                    ) if class == actual => self.push_nullable_class(class, object)?,
                    (
                        mir::Type::SharedReference(class),
                        Some(LocalValue::SharedReference {
                            control,
                            class: actual,
                        }),
                    ) if class == actual => self.current_frame_mut()?.values.push(
                        EvaluationValue::NullableSharedReference {
                            control: Some(control),
                            class,
                        },
                    ),
                    (mir::Type::SharedReference(class), None) => self
                        .current_frame_mut()?
                        .values
                        .push(EvaluationValue::NullableSharedReference {
                            control: None,
                            class,
                        }),
                    (
                        mir::Type::NullableSharedReference(class),
                        Some(LocalValue::NullableSharedReference {
                            control,
                            class: actual,
                        }),
                    ) if class == actual => self
                        .current_frame_mut()?
                        .values
                        .push(EvaluationValue::NullableSharedReference { control, class }),
                    (mir::Type::NullableSharedReference(class), None) => self
                        .current_frame_mut()?
                        .values
                        .push(EvaluationValue::NullableSharedReference {
                            control: None,
                            class,
                        }),
                    (
                        mir::Type::WeakReference(class),
                        Some(LocalValue::WeakReference {
                            control,
                            class: actual,
                        }),
                    ) if class == actual => self.current_frame_mut()?.values.push(
                        EvaluationValue::NullableWeakReference {
                            control: Some(control),
                            class,
                        },
                    ),
                    (mir::Type::WeakReference(class), None) => self
                        .current_frame_mut()?
                        .values
                        .push(EvaluationValue::NullableWeakReference {
                            control: None,
                            class,
                        }),
                    (
                        mir::Type::NullableWeakReference(class),
                        Some(LocalValue::NullableWeakReference {
                            control,
                            class: actual,
                        }),
                    ) if class == actual => self
                        .current_frame_mut()?
                        .values
                        .push(EvaluationValue::NullableWeakReference { control, class }),
                    (mir::Type::NullableWeakReference(class), None) => self
                        .current_frame_mut()?
                        .values
                        .push(EvaluationValue::NullableWeakReference {
                            control: None,
                            class,
                        }),
                    (
                        mir::Type::WritableSharedReference(payload),
                        Some(LocalValue::WritableSharedReference {
                            control,
                            payload: actual,
                        }),
                    ) if payload == actual => self.current_frame_mut()?.values.push(
                        EvaluationValue::NullableWritableSharedReference {
                            control: Some(control),
                            payload,
                        },
                    ),
                    (mir::Type::WritableSharedReference(payload), None) => self
                        .current_frame_mut()?
                        .values
                        .push(EvaluationValue::NullableWritableSharedReference {
                            control: None,
                            payload,
                        }),
                    (
                        mir::Type::NullableWritableSharedReference(payload),
                        Some(LocalValue::NullableWritableSharedReference {
                            control,
                            payload: actual,
                        }),
                    ) if payload == actual => self.current_frame_mut()?.values.push(
                        EvaluationValue::NullableWritableSharedReference { control, payload },
                    ),
                    (mir::Type::NullableWritableSharedReference(payload), None) => self
                        .current_frame_mut()?
                        .values
                        .push(EvaluationValue::NullableWritableSharedReference {
                            control: None,
                            payload,
                        }),
                    (
                        mir::Type::WritableWeakReference(payload),
                        Some(LocalValue::WritableWeakReference {
                            control,
                            payload: actual,
                        }),
                    ) if payload == actual => self.current_frame_mut()?.values.push(
                        EvaluationValue::NullableWritableWeakReference {
                            control: Some(control),
                            payload,
                        },
                    ),
                    (mir::Type::WritableWeakReference(payload), None) => self
                        .current_frame_mut()?
                        .values
                        .push(EvaluationValue::NullableWritableWeakReference {
                            control: None,
                            payload,
                        }),
                    (
                        mir::Type::NullableWritableWeakReference(payload),
                        Some(LocalValue::NullableWritableWeakReference {
                            control,
                            payload: actual,
                        }),
                    ) if payload == actual => self
                        .current_frame_mut()?
                        .values
                        .push(EvaluationValue::NullableWritableWeakReference { control, payload }),
                    (mir::Type::NullableWritableWeakReference(payload), None) => self
                        .current_frame_mut()?
                        .values
                        .push(EvaluationValue::NullableWritableWeakReference {
                            control: None,
                            payload,
                        }),
                    (expected, value) if expected.shared_access().is_some() => {
                        let access = expected
                            .shared_access()
                            .expect("guarded shared access collection result");
                        let control = match (access.nullable, value) {
                            (
                                false,
                                Some(LocalValue::SharedReferenceAccess {
                                    control,
                                    payload,
                                    writable,
                                }),
                            ) if payload == access.payload && writable == access.writable => {
                                Some(control)
                            }
                            (
                                true,
                                Some(LocalValue::NullableSharedReferenceAccess {
                                    control,
                                    payload,
                                    writable,
                                }),
                            ) if payload == access.payload && writable == access.writable => {
                                control
                            }
                            (_, None) => None,
                            _ => {
                                return Err(InterpreterError::new(
                                    "Dictionary::get produced another shared access type",
                                ))
                            }
                        };
                        self.current_frame_mut()?.values.push(
                            EvaluationValue::NullableSharedReferenceAccess {
                                control,
                                payload: access.payload,
                                writable: access.writable,
                            },
                        );
                    }
                    _ => {
                        return Err(InterpreterError::new(
                            "Dictionary::get produced another value type",
                        ))
                    }
                }
            }
            EvaluationTask::CollectionIndexClass {
                collection,
                class,
                transfer,
                positional,
            } => {
                let index = self.pop_local_value()?;
                let value = match self.collection_value_at(collection, &index, transfer, positional)
                {
                    Ok(value) => value,
                    Err(error) => return self.collection_access_panic_step(error),
                };
                let LocalValue::Class {
                    object,
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "MIR indexed class produced another value type",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "MIR indexed class has another class type",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Class { object, class });
            }
            EvaluationTask::CollectionIndexShared {
                collection,
                class,
                weak,
                nullable,
                transfer,
                positional,
            } => {
                let index = self.pop_local_value()?;
                let value = match self.collection_value_at(collection, &index, transfer, positional)
                {
                    Ok(value) => value,
                    Err(error) => return self.collection_access_panic_step(error),
                };
                let value = match (weak, nullable, value) {
                    (
                        false,
                        false,
                        LocalValue::SharedReference {
                            control,
                            class: actual,
                        },
                    ) if actual == class => EvaluationValue::SharedReference { control, class },
                    (
                        true,
                        false,
                        LocalValue::WeakReference {
                            control,
                            class: actual,
                        },
                    ) if actual == class => EvaluationValue::WeakReference { control, class },
                    (
                        false,
                        true,
                        LocalValue::NullableSharedReference {
                            control,
                            class: actual,
                        },
                    ) if actual == class => {
                        EvaluationValue::NullableSharedReference { control, class }
                    }
                    (
                        true,
                        true,
                        LocalValue::NullableWeakReference {
                            control,
                            class: actual,
                        },
                    ) if actual == class => {
                        EvaluationValue::NullableWeakReference { control, class }
                    }
                    _ => {
                        return Err(InterpreterError::new(
                            "MIR indexed shared handle has another type",
                        ))
                    }
                };
                self.current_frame_mut()?.values.push(value);
            }
            EvaluationTask::CollectionIndexWritableShared {
                collection,
                payload,
                weak,
                nullable,
                transfer,
                positional,
            } => {
                let index = self.pop_local_value()?;
                let value = match self.collection_value_at(collection, &index, transfer, positional)
                {
                    Ok(value) => value,
                    Err(error) => return self.collection_access_panic_step(error),
                };
                let value = match (weak, nullable, value) {
                    (
                        false,
                        false,
                        LocalValue::WritableSharedReference {
                            control,
                            payload: actual,
                        },
                    ) if actual == payload => {
                        EvaluationValue::WritableSharedReference { control, payload }
                    }
                    (
                        true,
                        false,
                        LocalValue::WritableWeakReference {
                            control,
                            payload: actual,
                        },
                    ) if actual == payload => {
                        EvaluationValue::WritableWeakReference { control, payload }
                    }
                    (
                        false,
                        true,
                        LocalValue::NullableWritableSharedReference {
                            control,
                            payload: actual,
                        },
                    ) if actual == payload => {
                        EvaluationValue::NullableWritableSharedReference { control, payload }
                    }
                    (
                        true,
                        true,
                        LocalValue::NullableWritableWeakReference {
                            control,
                            payload: actual,
                        },
                    ) if actual == payload => {
                        EvaluationValue::NullableWritableWeakReference { control, payload }
                    }
                    _ => {
                        return Err(InterpreterError::new(
                            "MIR indexed writable shared handle has another type",
                        ))
                    }
                };
                self.current_frame_mut()?.values.push(value);
            }
            EvaluationTask::CollectionIndexSharedAccess {
                collection,
                payload,
                writable,
                nullable,
                remove,
                positional,
            } => {
                let index = self.pop_local_value()?;
                let value = match self.collection_value_at(collection, &index, remove, positional) {
                    Ok(value) => value,
                    Err(error) => return self.collection_access_panic_step(error),
                };
                let result = match (nullable, value) {
                    (
                        false,
                        LocalValue::SharedReferenceAccess {
                            control,
                            payload: actual,
                            writable: actual_writable,
                        },
                    ) if actual == payload && actual_writable == writable => {
                        EvaluationValue::SharedReferenceAccess {
                            control,
                            payload,
                            writable,
                        }
                    }
                    (
                        true,
                        LocalValue::NullableSharedReferenceAccess {
                            control,
                            payload: actual,
                            writable: actual_writable,
                        },
                    ) if actual == payload && actual_writable == writable => {
                        EvaluationValue::NullableSharedReferenceAccess {
                            control,
                            payload,
                            writable,
                        }
                    }
                    _ => {
                        return Err(InterpreterError::new(
                            "MIR indexed shared access produced another value type",
                        ))
                    }
                };
                self.current_frame_mut()?.values.push(result);
            }
            EvaluationTask::BuildClassNew {
                class,
                properties,
                constructor,
                argument_count,
                property_expression_count,
                temporary_arg_drops,
            } => {
                let arguments = self.take_call_arguments(argument_count)?;
                let property_expressions = self.take_call_arguments(property_expression_count)?;
                let object_id = self.next_object;
                self.next_object += 1;
                let class_definition = self.program.classes.get(class.0).ok_or_else(|| {
                    InterpreterError::new(format!("MIR class#{} does not exist", class.0))
                })?;
                let mut slots = vec![None; class_definition.properties.len()];
                let mut expression_values = property_expressions.into_iter();
                for property in &properties {
                    let value = match &property.source {
                        mir::PropertyValueSource::Expression(_) => {
                            expression_values.next().ok_or_else(|| {
                                InterpreterError::new(
                                    "MIR class construction produced too few property values",
                                )
                            })?
                        }
                        mir::PropertyValueSource::ConstructorArgument(index) => {
                            arguments.get(*index).cloned().ok_or_else(|| {
                                InterpreterError::new(format!(
                                    "MIR constructor argument {index} does not exist"
                                ))
                            })?
                        }
                        mir::PropertyValueSource::ConstructorBody => continue,
                    };
                    let slot = slots.get_mut(property.property.index).ok_or_else(|| {
                        InterpreterError::new(format!(
                            "MIR property{} does not exist",
                            property.property.index
                        ))
                    })?;
                    *slot = Some(value);
                }
                self.heap.insert(
                    object_id,
                    ObjectValue {
                        class,
                        properties: slots,
                    },
                );
                if let Some(constructor) = constructor {
                    let mut temporary_drops = Vec::new();
                    for index in temporary_arg_drops {
                        collect_owned_objects_from_value(
                            arguments[index].clone(),
                            &mut temporary_drops,
                        );
                    }
                    let mut constructor_arguments = Vec::with_capacity(arguments.len() + 1);
                    constructor_arguments.push(LocalValue::Class {
                        object: object_id,
                        class,
                    });
                    constructor_arguments.extend(arguments);
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::FinishClassNew {
                            object: object_id,
                            class,
                        });
                    if !temporary_drops.is_empty() {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .extend(temporary_drops);
                    }
                    self.push_frame(
                        constructor,
                        &constructor_arguments,
                        Some(ReturnExpectation::Void),
                        None,
                    )?;
                } else {
                    self.current_frame_mut()?
                        .values
                        .push(EvaluationValue::Class {
                            object: object_id,
                            class,
                        });
                }
            }
            EvaluationTask::FinishClassNew { object, class } => {
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Class { object, class });
            }
            EvaluationTask::BuildNullableSome => {
                let value = self.pop_string()?;
                self.push_nullable_string(Some(value))?;
            }
            EvaluationTask::BuildNullableScalarSome(ty) => {
                let value = self.pop_scalar()?;
                if value.ty() != ty {
                    return Err(InterpreterError::new(
                        "nullable scalar payload type mismatch",
                    ));
                }
                self.push_nullable_scalar(ty, Some(value))?;
            }
            EvaluationTask::ParseNullableScalar(ty) => {
                let text = self.pop_string()?;
                let trimmed = text.trim();
                let value = match ty {
                    mir::ScalarType::Integer(IntegerType::Int64) => trimmed
                        .parse::<i64>()
                        .ok()
                        .and_then(|value| {
                            IntegerValue::from_i128(IntegerType::Int64, i128::from(value))
                        })
                        .map(mir::ScalarValue::Integer),
                    mir::ScalarType::Float(FloatType::Float64) => trimmed
                        .parse::<f64>()
                        .ok()
                        .map(|value| mir::ScalarValue::Float(FloatValue::from_f64(value))),
                    _ => {
                        return Err(InterpreterError::new(
                            "parse only supports `int` and `float` in Stage 23",
                        ));
                    }
                };
                self.push_nullable_scalar(ty, value)?;
            }
            EvaluationTask::BuildNullableClassSome(class) => {
                let LocalValue::Class {
                    object,
                    class: actual,
                } = self.pop_local_value()?
                else {
                    return Err(InterpreterError::new(
                        "nullable class payload is not a class",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "nullable class payload type mismatch",
                    ));
                }
                self.push_nullable_class(class, Some(object))?;
            }
            EvaluationTask::BuildMixedValue => {
                let value = self.pop_scalar()?;
                self.push_mixed(MixedValue::Scalar(value))?;
            }
            EvaluationTask::BuildMixedString => {
                let value = self.pop_string()?;
                self.push_mixed(MixedValue::String(value))?;
            }
            EvaluationTask::BuildMixedClass(payload_owned) => {
                let LocalValue::Class { object, class } = self.pop_local_value()? else {
                    return Err(InterpreterError::new("mixed class payload is not a class"));
                };
                self.push_mixed(MixedValue::Class {
                    object,
                    class,
                    owner: Rc::new(Cell::new(1)),
                    payload_owned,
                })?;
            }
            EvaluationTask::OwnMixed => {
                let LocalValue::Mixed(mut value) = self.pop_local_value()? else {
                    return Err(InterpreterError::new("owned mixed value is not mixed"));
                };
                retain_mixed_claim(&mut value, mir::MixedOwnership::None);
                self.push_mixed(value)?;
            }
            EvaluationTask::BuildNullableMixedSome => {
                let LocalValue::Mixed(value) = self.pop_local_value()? else {
                    return Err(InterpreterError::new("nullable mixed payload is not mixed"));
                };
                self.push_nullable_mixed(Some(value))?;
            }
            EvaluationTask::WrapNullable(ty) => {
                let value = self.pop_local_value()?;
                self.push_nullable_from_value(ty, value)?;
            }
            EvaluationTask::NullableScalarIsPresent => {
                let (_, value) = self.pop_nullable_scalar()?;
                self.push_scalar(mir::ScalarValue::Bool(value.is_some()))?;
            }
            EvaluationTask::NullableClassIsPresent(owned) => {
                let (class, object) = self.pop_nullable_class()?;
                if let (Some(object), Some(_)) = (object, owned) {
                    self.current_frame_mut()?
                        .statement_temporary_drops
                        .push(OwnedDrop::Class { object, class });
                }
                self.push_scalar(mir::ScalarValue::Bool(object.is_some()))?;
            }
            EvaluationTask::NullableCollectionIsPresent(owned) => {
                let value = self.pop_collection_value()?;
                let present = value.present;
                if owned.is_some() && present {
                    let mut drops = Vec::new();
                    collect_owned_objects_from_value(
                        LocalValue::Collection(value.assume_non_null()?),
                        &mut drops,
                    );
                    for drop in drops {
                        self.push_owned_drop_task(drop)?;
                    }
                }
                self.push_scalar(mir::ScalarValue::Bool(present))?;
            }
            EvaluationTask::NullableSharedReferenceIsPresent(owned) => {
                let LocalValue::NullableSharedReference { control, .. } = self.pop_local_value()?
                else {
                    return Err(InterpreterError::new(
                        "nullable shared presence test produced another value type",
                    ));
                };
                let present = control.is_some();
                if owned {
                    if let Some(control) = control {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push(OwnedDrop::Shared(control));
                    }
                }
                self.push_scalar(mir::ScalarValue::Bool(present))?;
            }
            EvaluationTask::NullableWeakReferenceIsPresent(owned) => {
                let LocalValue::NullableWeakReference { control, .. } = self.pop_local_value()?
                else {
                    return Err(InterpreterError::new(
                        "nullable weak presence test produced another value type",
                    ));
                };
                let present = control.is_some();
                if owned {
                    if let Some(control) = control {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push(OwnedDrop::Weak(control));
                    }
                }
                self.push_scalar(mir::ScalarValue::Bool(present))?;
            }
            EvaluationTask::NullableWritableSharedReferenceIsPresent(owned) => {
                let (control, _) = self.pop_nullable_writable_shared_reference()?;
                let present = control.is_some();
                if owned {
                    if let Some(control) = control {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push(OwnedDrop::WritableShared(control));
                    }
                }
                self.push_scalar(mir::ScalarValue::Bool(present))?;
            }
            EvaluationTask::NullableWritableWeakReferenceIsPresent(owned) => {
                let (control, _) = self.pop_nullable_writable_weak_reference()?;
                let present = control.is_some();
                if owned {
                    if let Some(control) = control {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push(OwnedDrop::WritableWeak(control));
                    }
                }
                self.push_scalar(mir::ScalarValue::Bool(present))?;
            }
            EvaluationTask::NullableSharedReferenceAccessIsPresent { owned, writable } => {
                let LocalValue::NullableSharedReferenceAccess {
                    control,
                    writable: actual_writable,
                    ..
                } = self.pop_local_value()?
                else {
                    return Err(InterpreterError::new(
                        "nullable shared access presence test produced another value type",
                    ));
                };
                if actual_writable != writable {
                    return Err(InterpreterError::new(
                        "nullable shared access presence test changed access type",
                    ));
                }
                let present = control.is_some();
                if owned {
                    if let Some(control) = control {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push(OwnedDrop::SharedAccess { control, writable });
                    }
                }
                self.push_scalar(mir::ScalarValue::Bool(present))?;
            }
            EvaluationTask::NullableMixedIsPresent(ownership) => {
                let value = self.pop_nullable_mixed()?;
                let present = value.is_some();
                if ownership == mir::MixedOwnership::Owned {
                    self.queue_value_drops(LocalValue::NullableMixed(value))?;
                }
                self.push_scalar(mir::ScalarValue::Bool(present))?;
            }
            EvaluationTask::MixedIs { tag, ownership } => {
                let LocalValue::Mixed(value) = self.pop_local_value()? else {
                    return Err(InterpreterError::new(
                        "MIR mixed is expression produced another value type",
                    ));
                };
                let matches = value.tag() == tag;
                if ownership == mir::MixedOwnership::Owned {
                    self.queue_value_drops(LocalValue::Mixed(value))?;
                }
                self.push_scalar(mir::ScalarValue::Bool(matches))?;
            }
            EvaluationTask::AfterIntegerCoalesce(right) => {
                let (_, value) = self.pop_nullable_scalar()?;
                if let Some(mir::ScalarValue::Integer(value)) = value {
                    self.push_scalar(mir::ScalarValue::Integer(value))?;
                } else {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::Integer(right));
                }
            }
            EvaluationTask::AfterFloatCoalesce(right) => {
                let (_, value) = self.pop_nullable_scalar()?;
                if let Some(mir::ScalarValue::Float(value)) = value {
                    self.push_scalar(mir::ScalarValue::Float(value))?;
                } else {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::Float(right));
                }
            }
            EvaluationTask::AfterBoolCoalesce(right) => {
                let (_, value) = self.pop_nullable_scalar()?;
                if let Some(mir::ScalarValue::Bool(value)) = value {
                    self.push_scalar(mir::ScalarValue::Bool(value))?;
                } else {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::Bool(right));
                }
            }
            EvaluationTask::AfterEnumCoalesce(right) => {
                let (_, value) = self.pop_nullable_scalar()?;
                if let Some(mir::ScalarValue::Enum(value)) = value {
                    self.push_scalar(mir::ScalarValue::Enum(value))?;
                } else {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::Enum(right));
                }
            }
            EvaluationTask::EnumBackingInt(enum_id) => {
                let mir::ScalarValue::Enum(value) = self.pop_scalar()? else {
                    return Err(InterpreterError::new(
                        "MIR enum backing projection produced another scalar type",
                    ));
                };
                if value.enum_id != enum_id {
                    return Err(InterpreterError::new(
                        "MIR enum backing projection changed enum identity",
                    ));
                }
                let case = enum_case_in(self.program, value)?;
                let Some(crate::enums::EnumBackingValue::Int(backing)) = case.backing_value else {
                    return Err(InterpreterError::new(
                        "MIR integer backing projection targets another backing type",
                    ));
                };
                self.push_scalar(mir::ScalarValue::Integer(backing))?;
            }
            EvaluationTask::EnumBackingString(enum_id) => {
                let mir::ScalarValue::Enum(value) = self.pop_scalar()? else {
                    return Err(InterpreterError::new(
                        "MIR enum backing projection produced another scalar type",
                    ));
                };
                if value.enum_id != enum_id {
                    return Err(InterpreterError::new(
                        "MIR enum backing projection changed enum identity",
                    ));
                }
                let case = enum_case_in(self.program, value)?;
                let Some(crate::enums::EnumBackingValue::String(backing)) = &case.backing_value
                else {
                    return Err(InterpreterError::new(
                        "MIR string backing projection targets another backing type",
                    ));
                };
                let backing = backing.clone();
                self.push_string(backing)?;
            }
            EvaluationTask::AfterStringCoalesce(right) => {
                if let Some(value) = self.pop_nullable_string()? {
                    self.push_string(value)?;
                } else {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::String(right));
                }
            }
            EvaluationTask::AfterNullableScalarCoalesce(right) => {
                let (ty, value) = self.pop_nullable_scalar()?;
                if value.is_some() {
                    self.push_nullable_scalar(ty, value)?;
                } else {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::NullableScalar(right));
                }
            }
            EvaluationTask::AfterNullableStringCoalesce(right) => {
                if let Some(value) = self.pop_nullable_string()? {
                    self.push_nullable_string(Some(value))?;
                } else {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::NullableString(right));
                }
            }
            EvaluationTask::AfterClassCoalesce {
                right,
                left_owned,
                transfer,
            } => {
                let (class, object) = self.pop_nullable_class()?;
                if let Some(object) = object {
                    if left_owned && !transfer {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push(OwnedDrop::Class { object, class });
                    }
                    self.current_frame_mut()?
                        .values
                        .push(EvaluationValue::Class { object, class });
                } else {
                    let owned = (!transfer).then(|| right.owned_temporary_class()).flatten();
                    let frame = self.current_frame_mut()?;
                    frame
                        .tasks
                        .push(EvaluationTask::FinishClassCoalesceRight(owned));
                    frame.tasks.push(EvaluationTask::Class(right));
                }
            }
            EvaluationTask::FinishClassCoalesceRight(owned) => {
                let LocalValue::Class { object, class } = self.pop_local_value()? else {
                    return Err(InterpreterError::new(
                        "class coalesce fallback produced another value type",
                    ));
                };
                if owned.is_some() {
                    self.current_frame_mut()?
                        .statement_temporary_drops
                        .push(OwnedDrop::Class { object, class });
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Class { object, class });
            }
            EvaluationTask::AfterNullableClassCoalesce {
                right,
                left_owned,
                transfer,
            } => {
                let (class, object) = self.pop_nullable_class()?;
                if let Some(object) = object {
                    if left_owned && !transfer {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push(OwnedDrop::Class { object, class });
                    }
                    self.push_nullable_class(class, Some(object))?;
                } else {
                    let owned = (!transfer).then(|| right.owned_temporary_class()).flatten();
                    let frame = self.current_frame_mut()?;
                    frame
                        .tasks
                        .push(EvaluationTask::FinishNullableClassCoalesceRight(owned));
                    frame.tasks.push(EvaluationTask::NullableClass(right));
                }
            }
            EvaluationTask::AfterNullableMixedCoalesce {
                right,
                left_ownership,
            } => {
                let mut value = self.pop_nullable_mixed()?;
                if let Some(present) = value.as_mut() {
                    retain_mixed_claim(present, left_ownership);
                    self.push_nullable_mixed(value)?;
                } else {
                    let frame = self.current_frame_mut()?;
                    frame
                        .tasks
                        .push(EvaluationTask::OwnNullableMixed(right.ownership()));
                    frame.tasks.push(EvaluationTask::NullableMixed(right));
                }
            }
            EvaluationTask::OwnNullableMixed(ownership) => {
                let mut value = self.pop_nullable_mixed()?;
                if let Some(value) = value.as_mut() {
                    retain_mixed_claim(value, ownership);
                }
                self.push_nullable_mixed(value)?;
            }
            EvaluationTask::FinishNullableClassCoalesceRight(owned) => {
                let (class, object) = self.pop_nullable_class()?;
                if let (Some(object), Some(_)) = (object, owned) {
                    self.current_frame_mut()?
                        .statement_temporary_drops
                        .push(OwnedDrop::Class { object, class });
                }
                self.push_nullable_class(class, object)?;
            }
            EvaluationTask::AfterSharedCoalesce {
                right,
                left_owned,
                transfer,
            } => {
                let value = self.pop_local_value()?;
                let LocalValue::NullableSharedReference { control, class } = value else {
                    return Err(InterpreterError::new(
                        "shared coalesce left operand was not a nullable shared reference",
                    ));
                };
                if let Some(control) = control {
                    if left_owned && !transfer {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push(OwnedDrop::Shared(control.clone()));
                    }
                    self.current_frame_mut()?
                        .values
                        .push(EvaluationValue::SharedReference { control, class });
                } else {
                    let right_owned = !transfer && right.owned_temporary().is_some();
                    let frame = self.current_frame_mut()?;
                    frame
                        .tasks
                        .push(EvaluationTask::FinishSharedCoalesceRight(right_owned));
                    frame.tasks.push(EvaluationTask::SharedReference(right));
                }
            }
            EvaluationTask::FinishSharedCoalesceRight(owned) => {
                let LocalValue::SharedReference { control, class } = self.pop_local_value()? else {
                    return Err(InterpreterError::new(
                        "shared coalesce fallback produced another value type",
                    ));
                };
                if owned {
                    self.current_frame_mut()?
                        .statement_temporary_drops
                        .push(OwnedDrop::Shared(control.clone()));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::SharedReference { control, class });
            }
            EvaluationTask::AfterNullableSharedCoalesce {
                right,
                left_owned,
                transfer,
            } => {
                let (class, control) = self.pop_nullable_shared_reference()?;
                if let Some(control) = control {
                    if left_owned && !transfer {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push(OwnedDrop::Shared(control.clone()));
                    }
                    self.current_frame_mut()?.values.push(
                        EvaluationValue::NullableSharedReference {
                            control: Some(control),
                            class,
                        },
                    );
                } else {
                    let right_owned = !transfer && right.owned_temporary().is_some();
                    let frame = self.current_frame_mut()?;
                    frame
                        .tasks
                        .push(EvaluationTask::FinishNullableSharedCoalesceRight(
                            right_owned,
                        ));
                    frame
                        .tasks
                        .push(EvaluationTask::NullableSharedReference(right));
                }
            }
            EvaluationTask::FinishNullableSharedCoalesceRight(owned) => {
                let (class, control) = self.pop_nullable_shared_reference()?;
                if let (Some(control), true) = (&control, owned) {
                    self.current_frame_mut()?
                        .statement_temporary_drops
                        .push(OwnedDrop::Shared(control.clone()));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::NullableSharedReference { control, class });
            }
            EvaluationTask::AfterWeakCoalesce {
                right,
                left_owned,
                transfer,
            } => {
                let value = self.pop_local_value()?;
                let LocalValue::NullableWeakReference { control, class } = value else {
                    return Err(InterpreterError::new(
                        "weak coalesce left operand was not a nullable weak reference",
                    ));
                };
                if let Some(control) = control {
                    if left_owned && !transfer {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push(OwnedDrop::Weak(control.clone()));
                    }
                    self.current_frame_mut()?
                        .values
                        .push(EvaluationValue::WeakReference { control, class });
                } else {
                    let right_owned = !transfer && right.owned_temporary().is_some();
                    let frame = self.current_frame_mut()?;
                    frame
                        .tasks
                        .push(EvaluationTask::FinishWeakCoalesceRight(right_owned));
                    frame.tasks.push(EvaluationTask::WeakReference(right));
                }
            }
            EvaluationTask::FinishWeakCoalesceRight(owned) => {
                let LocalValue::WeakReference { control, class } = self.pop_local_value()? else {
                    return Err(InterpreterError::new(
                        "weak coalesce fallback produced another value type",
                    ));
                };
                if owned {
                    self.current_frame_mut()?
                        .statement_temporary_drops
                        .push(OwnedDrop::Weak(control.clone()));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::WeakReference { control, class });
            }
            EvaluationTask::AfterNullableWeakCoalesce {
                right,
                left_owned,
                transfer,
            } => {
                let (class, control) = self.pop_nullable_weak_reference()?;
                if let Some(control) = control {
                    if left_owned && !transfer {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push(OwnedDrop::Weak(control.clone()));
                    }
                    self.current_frame_mut()?
                        .values
                        .push(EvaluationValue::NullableWeakReference {
                            control: Some(control),
                            class,
                        });
                } else {
                    let right_owned = !transfer && right.owned_temporary().is_some();
                    let frame = self.current_frame_mut()?;
                    frame
                        .tasks
                        .push(EvaluationTask::FinishNullableWeakCoalesceRight(right_owned));
                    frame
                        .tasks
                        .push(EvaluationTask::NullableWeakReference(right));
                }
            }
            EvaluationTask::FinishNullableWeakCoalesceRight(owned) => {
                let (class, control) = self.pop_nullable_weak_reference()?;
                if let (Some(control), true) = (&control, owned) {
                    self.current_frame_mut()?
                        .statement_temporary_drops
                        .push(OwnedDrop::Weak(control.clone()));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::NullableWeakReference { control, class });
            }
            EvaluationTask::AfterWritableSharedCoalesce {
                right,
                left_owned,
                transfer,
            } => {
                let (control, payload) = self.pop_nullable_writable_shared_reference()?;
                if let Some(control) = control {
                    if left_owned && !transfer {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push(OwnedDrop::WritableShared(control.clone()));
                    }
                    self.current_frame_mut()?
                        .values
                        .push(EvaluationValue::WritableSharedReference { control, payload });
                } else {
                    let right_owned = !transfer && right.owned_temporary();
                    let frame = self.current_frame_mut()?;
                    frame
                        .tasks
                        .push(EvaluationTask::FinishWritableSharedCoalesceRight(
                            right_owned,
                        ));
                    frame
                        .tasks
                        .push(EvaluationTask::WritableSharedReference(right));
                }
            }
            EvaluationTask::FinishWritableSharedCoalesceRight(owned) => {
                let (control, payload) = self.pop_writable_shared_reference()?;
                if owned {
                    self.current_frame_mut()?
                        .statement_temporary_drops
                        .push(OwnedDrop::WritableShared(control.clone()));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::WritableSharedReference { control, payload });
            }
            EvaluationTask::AfterNullableWritableSharedCoalesce {
                right,
                left_owned,
                transfer,
            } => {
                let (control, payload) = self.pop_nullable_writable_shared_reference()?;
                if let Some(control) = control {
                    if left_owned && !transfer {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push(OwnedDrop::WritableShared(control.clone()));
                    }
                    self.current_frame_mut()?.values.push(
                        EvaluationValue::NullableWritableSharedReference {
                            control: Some(control),
                            payload,
                        },
                    );
                } else {
                    let right_owned = !transfer && right.owned_temporary();
                    let frame = self.current_frame_mut()?;
                    frame
                        .tasks
                        .push(EvaluationTask::FinishNullableWritableSharedCoalesceRight(
                            right_owned,
                        ));
                    frame
                        .tasks
                        .push(EvaluationTask::NullableWritableSharedReference(right));
                }
            }
            EvaluationTask::FinishNullableWritableSharedCoalesceRight(owned) => {
                let (control, payload) = self.pop_nullable_writable_shared_reference()?;
                if let (Some(control), true) = (&control, owned) {
                    self.current_frame_mut()?
                        .statement_temporary_drops
                        .push(OwnedDrop::WritableShared(control.clone()));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::NullableWritableSharedReference { control, payload });
            }
            EvaluationTask::AfterWritableWeakCoalesce {
                right,
                left_owned,
                transfer,
            } => {
                let (control, payload) = self.pop_nullable_writable_weak_reference()?;
                if let Some(control) = control {
                    if left_owned && !transfer {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push(OwnedDrop::WritableWeak(control.clone()));
                    }
                    self.current_frame_mut()?
                        .values
                        .push(EvaluationValue::WritableWeakReference { control, payload });
                } else {
                    let right_owned = !transfer && right.owned_temporary();
                    let frame = self.current_frame_mut()?;
                    frame
                        .tasks
                        .push(EvaluationTask::FinishWritableWeakCoalesceRight(right_owned));
                    frame
                        .tasks
                        .push(EvaluationTask::WritableWeakReference(right));
                }
            }
            EvaluationTask::FinishWritableWeakCoalesceRight(owned) => {
                let (control, payload) = self.pop_writable_weak_reference()?;
                if owned {
                    self.current_frame_mut()?
                        .statement_temporary_drops
                        .push(OwnedDrop::WritableWeak(control.clone()));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::WritableWeakReference { control, payload });
            }
            EvaluationTask::AfterNullableWritableWeakCoalesce {
                right,
                left_owned,
                transfer,
            } => {
                let (control, payload) = self.pop_nullable_writable_weak_reference()?;
                if let Some(control) = control {
                    if left_owned && !transfer {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push(OwnedDrop::WritableWeak(control.clone()));
                    }
                    self.current_frame_mut()?.values.push(
                        EvaluationValue::NullableWritableWeakReference {
                            control: Some(control),
                            payload,
                        },
                    );
                } else {
                    let right_owned = !transfer && right.owned_temporary();
                    let frame = self.current_frame_mut()?;
                    frame
                        .tasks
                        .push(EvaluationTask::FinishNullableWritableWeakCoalesceRight(
                            right_owned,
                        ));
                    frame
                        .tasks
                        .push(EvaluationTask::NullableWritableWeakReference(right));
                }
            }
            EvaluationTask::FinishNullableWritableWeakCoalesceRight(owned) => {
                let (control, payload) = self.pop_nullable_writable_weak_reference()?;
                if let (Some(control), true) = (&control, owned) {
                    self.current_frame_mut()?
                        .statement_temporary_drops
                        .push(OwnedDrop::WritableWeak(control.clone()));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::NullableWritableWeakReference { control, payload });
            }
            EvaluationTask::AfterNullSafeProperty {
                property,
                result,
                owned_receiver,
            } => {
                let (class, object) = self.pop_nullable_class()?;
                if let Some(object) = object {
                    if owned_receiver.is_some() {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push(OwnedDrop::Class { object, class });
                    }
                    let value = self.read_object_property(object, property)?;
                    self.push_nullable_from_value(result, value)?;
                } else {
                    self.push_null(result)?;
                }
            }
            EvaluationTask::AfterNullSafeCall {
                function,
                args,
                result,
                owned_receiver,
            } => {
                let (class, object) = self.pop_nullable_class()?;
                if let Some(object) = object {
                    if owned_receiver.is_some() {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push(OwnedDrop::Class { object, class });
                    }
                    self.queue_null_safe_call(object, class, function, args, result)?;
                } else {
                    self.push_null(result)?;
                }
            }
            EvaluationTask::AfterNullSafeStatementCall {
                function,
                args,
                owned_receiver,
                call_site,
            } => {
                let (class, object) = self.pop_nullable_class()?;
                if let Some(object) = object {
                    if owned_receiver.is_some() {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push(OwnedDrop::Class { object, class });
                    }
                    self.queue_null_safe_statement_call(object, class, function, args, call_site)?;
                }
            }
            EvaluationTask::NullableStringCompare(op) => {
                let right = self.pop_nullable_string()?;
                let left = self.pop_nullable_string()?;
                let result = match op {
                    mir::CompareOp::Equal => left == right,
                    mir::CompareOp::NotEqual => left != right,
                    _ => {
                        return Err(InterpreterError::new(
                            "MIR ordered nullable-string comparison is invalid",
                        ))
                    }
                };
                self.push_scalar(mir::ScalarValue::Bool(result))?;
            }
            EvaluationTask::Format(format) => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::BuildFormat(format.clone()));
                for argument in format.arguments.into_iter().rev() {
                    match argument {
                        mir::FormatArgument::Value(value) => {
                            frame.tasks.push(EvaluationTask::Value(value));
                        }
                        mir::FormatArgument::String(value)
                        | mir::FormatArgument::ClassDisplay(value) => {
                            frame.tasks.push(EvaluationTask::String(value));
                        }
                    }
                }
            }
            EvaluationTask::BuildFormat(format) => {
                let values = self.take_evaluation_values(format.arguments.len())?;
                self.push_string(render_format(&format, &values)?)?;
            }
            EvaluationTask::ReadLine(prompt_span) => {
                let prompt = self.pop_string()?;
                if !prompt.is_empty() {
                    self.io_trace.prompt_writes += 1;
                    match self.io_faults.prompt_write {
                        Some(MirIoWriteFailure::BrokenPipe) => {
                            return Ok(StepOutcome::CleanExit);
                        }
                        Some(MirIoWriteFailure::Other) => {
                            return self.runtime_panic_step_at("P1407", prompt_span);
                        }
                        None => {}
                    }
                    self.stdout.extend_from_slice(prompt.as_bytes());
                }
                self.io_trace.stdout_flushes += 1;
                match self.io_faults.stdout_flush {
                    Some(MirIoWriteFailure::BrokenPipe) => {
                        return Ok(StepOutcome::CleanExit);
                    }
                    Some(MirIoWriteFailure::Other) => {
                        return self.runtime_panic_step_at("P1407", prompt_span);
                    }
                    None => {}
                }
                self.io_trace.stdin_line_reads += 1;
                if self.io_faults.stdin_line_read {
                    return self.runtime_panic_step_at("P1403", prompt_span);
                }
                if self.stdin_cursor == self.stdin.len() {
                    self.push_nullable_string(None)?;
                } else {
                    let remaining = &self.stdin[self.stdin_cursor..];
                    let newline = remaining.iter().position(|byte| *byte == b'\n');
                    let consumed = newline.map_or(remaining.len(), |index| index + 1);
                    let mut line_length = newline.unwrap_or(remaining.len());
                    if line_length != 0 && remaining[line_length - 1] == b'\r' {
                        line_length -= 1;
                    }
                    let Ok(line) = core::str::from_utf8(&remaining[..line_length]) else {
                        return self.runtime_panic_step_at("P1404", prompt_span);
                    };
                    if self.io_faults.line_allocation {
                        return self.runtime_panic_step_at("P1206", prompt_span);
                    }
                    let line = line.to_string();
                    self.stdin_cursor += consumed;
                    self.push_nullable_string(Some(line.into()))?;
                }
            }
            EvaluationTask::ReadFile(path_span) => {
                let path = self.pop_string()?;
                if path.as_bytes().contains(&0) {
                    return self.runtime_panic_step_at("P1405", path_span);
                }
                let Some(bytes) = self.files.get(path.as_ref()) else {
                    return self.runtime_panic_step_at("P1401", path_span);
                };
                let Ok(value) = String::from_utf8(bytes.clone()) else {
                    return self.runtime_panic_step_at("P1406", path_span);
                };
                self.push_string(value)?;
            }
            EvaluationTask::WriteFile => {
                let contents = self.pop_string()?;
                let path = self.pop_string()?;
                if path.as_bytes().contains(&0) {
                    return self.runtime_panic_step("P1405");
                }
                self.files
                    .insert(path.to_string(), contents.as_bytes().to_vec());
            }
            EvaluationTask::AppendFile => {
                let contents = self.pop_string()?;
                let path = self.pop_string()?;
                if path.as_bytes().contains(&0) {
                    return self.runtime_panic_step("P1405");
                }
                self.files
                    .entry(path.to_string())
                    .or_default()
                    .extend_from_slice(contents.as_bytes());
            }
            EvaluationTask::ReadFileBytes(collection, path_span) => {
                let path = self.pop_string()?;
                if path.as_bytes().contains(&0) {
                    return self.runtime_panic_step_at("P1405", path_span);
                }
                let Some(contents) = self.files.get(path.as_ref()).cloned() else {
                    return self.runtime_panic_step_at("P1401", path_span);
                };
                self.push_byte_collection(collection, &contents)?;
            }
            EvaluationTask::WriteFileBytes { contents, append } => {
                let path = self.pop_string()?;
                if path.as_bytes().contains(&0) {
                    return self.runtime_panic_step("P1405");
                }
                let bytes = self.byte_collection(contents)?;
                if append {
                    self.files
                        .entry(path.to_string())
                        .or_default()
                        .extend_from_slice(&bytes);
                } else {
                    self.files.insert(path.to_string(), bytes);
                }
            }
            EvaluationTask::WriteStreamBytes { contents, stderr } => {
                let bytes = self.byte_collection(contents)?;
                if stderr {
                    self.stderr.extend_from_slice(&bytes);
                } else {
                    self.stdout.extend_from_slice(&bytes);
                }
            }
            EvaluationTask::WriteStderr => {
                let value = self.pop_string()?;
                self.stderr.extend_from_slice(value.as_bytes());
            }
            EvaluationTask::StringConcat(count) => {
                let mut parts = Vec::with_capacity(count);
                for _ in 0..count {
                    parts.push(self.pop_string()?);
                }
                parts.reverse();
                self.push_string(parts.concat())?;
            }
            EvaluationTask::StringIntrinsic {
                kind,
                result,
                argument_count,
                span,
                argument_spans,
            } => {
                let mut arguments = Vec::with_capacity(argument_count);
                for _ in 0..argument_count {
                    arguments.push(self.pop_local_value()?);
                }
                arguments.reverse();
                if let Some(event) =
                    self.execute_string_intrinsic(kind, result, arguments, span, &argument_spans)?
                {
                    return Ok(StepOutcome::RuntimePanic(event));
                }
            }
            EvaluationTask::StringDisplay => {
                let value = self.pop_scalar()?;
                self.push_string(display_scalar(value))?;
            }
            EvaluationTask::StringCompare(op) => {
                let right = self.pop_string()?;
                let left = self.pop_string()?;
                let ordering = left.as_bytes().cmp(right.as_bytes());
                let result = match op {
                    mir::CompareOp::Equal => ordering.is_eq(),
                    mir::CompareOp::NotEqual => !ordering.is_eq(),
                    mir::CompareOp::Less => ordering.is_lt(),
                    mir::CompareOp::LessEqual => !ordering.is_gt(),
                    mir::CompareOp::Greater => ordering.is_gt(),
                    mir::CompareOp::GreaterEqual => !ordering.is_lt(),
                };
                self.push_scalar(mir::ScalarValue::Bool(result))?;
            }
            EvaluationTask::Echo => {
                let value = self.pop_string()?;
                self.stdout.extend_from_slice(value.as_bytes());
            }
            EvaluationTask::PanicString(span) => {
                let message = self.pop_string()?.to_string();
                return Ok(StepOutcome::RuntimePanic(RuntimePanicEvent {
                    code: "P1000",
                    operation_span: span,
                    primary_span: span,
                    facts: vec![RuntimeFact {
                        name: "message".to_string(),
                        value: RuntimeFactValue::StaticString(message.clone()),
                    }],
                    explanation: None,
                }));
            }
            EvaluationTask::Integer(expression) => self.expand_integer_expression(expression)?,
            EvaluationTask::IntegerUnary { op, span } => {
                let operand = self.pop_integer()?;
                let value = match eval_unary(op, operand) {
                    Ok(value) => value,
                    Err(panic) => {
                        return Ok(StepOutcome::RuntimePanic(Self::integer_panic_event(
                            panic, span,
                        )))
                    }
                };
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Scalar(mir::ScalarValue::Integer(value)));
            }
            EvaluationTask::IntegerBinary {
                op,
                span,
                right_span,
            } => {
                let right = self.pop_integer()?;
                let left = self.pop_integer()?;
                let value = match eval_binary(op, left, right) {
                    Ok(value) => value,
                    Err(panic) => {
                        let primary_span = match panic {
                            IntegerPanic::DivisionByZero
                            | IntegerPanic::RemainderByZero
                            | IntegerPanic::ShiftCountOutOfRange => right_span,
                            _ => span,
                        };
                        return Ok(StepOutcome::RuntimePanic(Self::integer_panic_event(
                            panic,
                            primary_span,
                        )));
                    }
                };
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Scalar(mir::ScalarValue::Integer(value)));
            }
            EvaluationTask::IntegerConvert {
                target,
                operation_span,
                primary_span,
            } => {
                let value = match self.pop_integer()?.convert(target) {
                    Ok(value) => value,
                    Err(panic) => {
                        let mut event = Self::integer_panic_event(panic, operation_span);
                        event.primary_span = primary_span;
                        return Ok(StepOutcome::RuntimePanic(event));
                    }
                };
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Scalar(mir::ScalarValue::Integer(value)));
            }
            EvaluationTask::FloatToInt {
                operation_span,
                primary_span,
            } => {
                let value = self.pop_float()?;
                let Some(value) = value.to_i64_checked() else {
                    return Ok(StepOutcome::RuntimePanic(RuntimePanicEvent {
                        code: "P1110",
                        operation_span,
                        primary_span,
                        facts: Vec::new(),
                        explanation: None,
                    }));
                };
                self.push_scalar(mir::ScalarValue::Integer(
                    IntegerValue::from_i128(IntegerType::Int64, value as i128)
                        .expect("i64 always fits canonical int"),
                ))?;
            }
            EvaluationTask::Float(expression) => self.expand_float_expression(expression)?,
            EvaluationTask::FloatNegate => {
                let value = self.pop_float()?.negate();
                self.push_scalar(mir::ScalarValue::Float(value))?;
            }
            EvaluationTask::FloatBinary(op) => {
                let right = self.pop_float()?;
                let left = self.pop_float()?;
                let value = match op {
                    mir::FloatBinaryOp::Add => left.add(right),
                    mir::FloatBinaryOp::Subtract => left.subtract(right),
                    mir::FloatBinaryOp::Multiply => left.multiply(right),
                    mir::FloatBinaryOp::Divide => left.divide(right),
                };
                self.push_scalar(mir::ScalarValue::Float(value))?;
            }
            EvaluationTask::IntToFloat => {
                let value = self.pop_integer()?;
                if value.ty != IntegerType::Int64 {
                    return Err(InterpreterError::new(
                        "MIR Int::toFloat operand is not canonical int",
                    ));
                }
                self.push_scalar(mir::ScalarValue::Float(FloatValue::from_f64(
                    value.signed_value() as f64,
                )))?;
            }
            EvaluationTask::Bool(condition) => self.expand_bool_expression(condition)?,
            EvaluationTask::Compare(op) => {
                let right = self.pop_scalar()?;
                let left = self.pop_scalar()?;
                let value = eval_compare(op, left, right)?;
                self.push_scalar(mir::ScalarValue::Bool(value))?;
            }
            EvaluationTask::Not => {
                let value = !self.pop_bool()?;
                self.push_scalar(mir::ScalarValue::Bool(value))?;
            }
            EvaluationTask::AfterAnd(right) => {
                if self.pop_bool()? {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::Bool(right));
                } else {
                    self.push_scalar(mir::ScalarValue::Bool(false))?;
                }
            }
            EvaluationTask::AfterOr(right) => {
                if self.pop_bool()? {
                    self.push_scalar(mir::ScalarValue::Bool(true))?;
                } else {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::Bool(right));
                }
            }
            EvaluationTask::Xor => {
                let right = self.pop_bool()?;
                let left = self.pop_bool()?;
                self.push_scalar(mir::ScalarValue::Bool(left ^ right))?;
            }
            EvaluationTask::Invoke {
                function,
                argument_count,
                expectation,
                temporary_arg_drops,
                call_site,
            } => {
                let args = self.take_call_arguments(argument_count)?;
                let mut drops = Vec::new();
                for index in temporary_arg_drops {
                    collect_owned_objects_from_value(args[index].clone(), &mut drops);
                }
                if !drops.is_empty() {
                    self.current_frame_mut()?
                        .statement_temporary_drops
                        .extend(drops);
                }
                self.push_frame(function, &args, Some(expectation), call_site)?;
            }
            EvaluationTask::FinishStatement => {
                let drops =
                    std::mem::take(&mut self.current_frame_mut()?.statement_temporary_drops);
                if !drops.is_empty() {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::DropTemporaryValues(drops));
                }
            }
            EvaluationTask::DropTemporaryValues(drops) => {
                for drop in drops {
                    self.push_owned_drop_task(drop)?;
                }
            }
            EvaluationTask::Assign(target) => {
                let value = self.pop_local_value()?;
                let function = function_in(self.program, self.current_frame()?.function)?;
                let owned = local_in(function, target)?.owned;
                let old = assign_local(
                    &function.locals,
                    &mut self.current_frame_mut()?.locals,
                    target,
                    value,
                )?;
                if owned {
                    if let Some(value) = old {
                        self.queue_value_drops(value)?;
                    }
                }
            }
            EvaluationTask::AssignGroup(targets) => {
                let value = self.pop_local_value()?;
                let function = function_in(self.program, self.current_frame()?.function)?;
                for target in targets {
                    let old = assign_local(
                        &function.locals,
                        &mut self.current_frame_mut()?.locals,
                        target,
                        value.clone(),
                    )?;
                    if old.is_some() {
                        return Err(InterpreterError::new(
                            "grouped local initializer targets an initialized local",
                        ));
                    }
                }
            }
            EvaluationTask::AssignStatic(target) => {
                let value = self.pop_local_value()?;
                let slot = self.statics.get_mut(target.0).ok_or_else(|| {
                    InterpreterError::new(format!("MIR static{} does not exist", target.0))
                })?;
                *slot = value;
            }
            EvaluationTask::AssignProperty { object, property } => {
                let value = self.pop_local_value()?;
                if let Some(old) = self.assign_property(object, property, value)? {
                    self.queue_value_drops(old)?;
                }
            }
            EvaluationTask::DropClass(local) => {
                self.drop_class_local(local)?;
            }
            EvaluationTask::DropShared(local) => {
                self.drop_shared_local(local, false)?;
            }
            EvaluationTask::DropWeak(local) => {
                self.drop_shared_local(local, true)?;
            }
            EvaluationTask::DropWritableShared(local) => {
                self.drop_writable_shared_local(local, WritableDropKind::Strong)?;
            }
            EvaluationTask::DropWritableWeak(local) => {
                self.drop_writable_shared_local(local, WritableDropKind::Weak)?;
            }
            EvaluationTask::DropSharedAccess(local) => {
                self.drop_writable_shared_local(local, WritableDropKind::Access)?;
            }
            EvaluationTask::ReleaseShared(control) => {
                let payload = {
                    let mut state = control.borrow_mut();
                    if state.strong == 0 {
                        return Err(InterpreterError::new(
                            "MIR released an exhausted strong handle",
                        ));
                    }
                    state.strong -= 1;
                    (state.strong == 0).then(|| state.payload.take()).flatten()
                };
                if let Some((object, class)) = payload {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::DropObject { object, class });
                }
            }
            EvaluationTask::ReleaseWeak(control) => {
                let mut state = control.borrow_mut();
                if state.weak == 0 {
                    return Err(InterpreterError::new(
                        "MIR released an exhausted weak handle",
                    ));
                }
                state.weak -= 1;
            }
            EvaluationTask::ReleaseWritableShared(control) => {
                let payload = {
                    let mut state = control.borrow_mut();
                    if state.strong == 0 {
                        return Err(InterpreterError::new(
                            "MIR released an exhausted writable strong handle",
                        ));
                    }
                    state.strong -= 1;
                    (state.strong == 0).then(|| state.payload.take()).flatten()
                };
                if let Some(payload) = payload {
                    self.queue_value_drops(payload)?;
                }
            }
            EvaluationTask::ReleaseWritableWeak(control) => {
                let mut state = control.borrow_mut();
                if state.weak == 0 {
                    return Err(InterpreterError::new(
                        "MIR released an exhausted writable weak handle",
                    ));
                }
                state.weak -= 1;
            }
            EvaluationTask::ReleaseSharedAccess { control, writable } => {
                {
                    let mut state = control.borrow_mut();
                    if writable {
                        if !state.writable_access_active {
                            return Err(InterpreterError::new(
                                "MIR released an inactive writable shared access",
                            ));
                        }
                        state.writable_access_active = false;
                    } else {
                        if state.readonly_accesses == 0 {
                            return Err(InterpreterError::new(
                                "MIR released an exhausted readonly shared access",
                            ));
                        }
                        state.readonly_accesses -= 1;
                    }
                }
                self.current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::ReleaseWritableShared(control));
            }
            EvaluationTask::DropCollection(local) => {
                self.drop_collection_local(local)?;
            }
            EvaluationTask::CollectionClear(local) => {
                self.clear_collection_local(local)?;
            }
            EvaluationTask::DropObject { object, class } => {
                self.queue_object_drop(object, class)?;
            }
            EvaluationTask::DropObjectProperties { object, class } => {
                self.queue_object_property_drops(object, class)?;
            }
            EvaluationTask::FreeObject { object, class } => {
                self.free_object(object, class)?;
            }
            EvaluationTask::CleanupFrame => {
                self.cleanup_current_frame()?;
            }
            EvaluationTask::ReturnValue(expected) => {
                let value = self.pop_local_value()?;
                if local_value_type(&value) != expected {
                    return Err(InterpreterError::new(format!(
                        "MIR return evaluation produced {}, expected {expected}",
                        local_value_type(&value)
                    )));
                }
                return self.complete_frame(FunctionOutcome::Value(value));
            }
            EvaluationTask::ReturnVoid => {
                return self.complete_frame(FunctionOutcome::Void);
            }
            EvaluationTask::Branch {
                then_block,
                else_block,
            } => {
                let target = if self.pop_bool()? {
                    then_block
                } else {
                    else_block
                };
                let function = function_in(self.program, self.current_frame()?.function)?;
                self.move_to_block(function, target)?;
            }
        }
        Ok(StepOutcome::Continue)
    }

    fn expand_integer_expression(
        &mut self,
        expression: mir::IntegerExpression,
    ) -> Result<(), InterpreterError> {
        match expression {
            mir::IntegerExpression::Use { ty, operand } => {
                if self.queue_collection_scalar_operand(&operand)? {
                    return Ok(());
                }
                let value = self.eval_operand(&operand)?;
                let mir::ScalarValue::Integer(value) = value else {
                    return Err(InterpreterError::new(
                        "MIR integer operand produced another scalar type",
                    ));
                };
                if value.ty != ty {
                    return Err(InterpreterError::new(format!(
                        "MIR operand evaluation produced {}, expression declares {ty}",
                        value.ty
                    )));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Scalar(mir::ScalarValue::Integer(value)));
            }
            mir::IntegerExpression::Unary {
                ty,
                op,
                operand,
                span,
            } => {
                if operand.ty() != ty {
                    return Err(InterpreterError::new(format!(
                        "MIR {ty} unary expression has operand type {}",
                        operand.ty()
                    )));
                }
                if op == mir::IntegerUnaryOp::Negate && !ty.is_signed() {
                    return Err(InterpreterError::new(format!(
                        "MIR unary negation requires a signed integer, got {ty}"
                    )));
                }
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::IntegerUnary { op, span });
                frame.tasks.push(EvaluationTask::Integer(*operand));
            }
            mir::IntegerExpression::Binary {
                ty,
                op,
                left,
                right,
                span,
                right_span,
            } => {
                if left.ty() != ty || right.ty() != ty {
                    return Err(InterpreterError::new(format!(
                        "MIR {ty} binary expression has operand types {} and {}",
                        left.ty(),
                        right.ty()
                    )));
                }
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::IntegerBinary {
                    op,
                    span,
                    right_span,
                });
                frame.tasks.push(EvaluationTask::Integer(*right));
                frame.tasks.push(EvaluationTask::Integer(*left));
            }
            mir::IntegerExpression::Convert {
                ty,
                value,
                span,
                value_span,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::IntegerConvert {
                    target: ty,
                    operation_span: span,
                    primary_span: value_span,
                });
                frame.tasks.push(EvaluationTask::Integer(*value));
            }
            mir::IntegerExpression::FloatToInt {
                value,
                span,
                value_span,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::FloatToInt {
                    operation_span: span,
                    primary_span: value_span,
                });
                frame.tasks.push(EvaluationTask::Float(*value));
            }
            mir::IntegerExpression::Call { ty, function, args } => {
                self.queue_call(
                    function,
                    args,
                    ReturnExpectation::Value(mir::Type::Scalar(mir::ScalarType::Integer(ty))),
                )?;
            }
            mir::IntegerExpression::Coalesce { left, right, .. } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::AfterIntegerCoalesce(*right));
                frame.tasks.push(EvaluationTask::NullableScalar(*left));
            }
            mir::IntegerExpression::EnumBacking { enum_id, value } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::EnumBackingInt(enum_id));
                frame.tasks.push(EvaluationTask::Enum(*value));
            }
        }
        Ok(())
    }

    fn expand_enum_expression(
        &mut self,
        expression: mir::EnumExpression,
    ) -> Result<(), InterpreterError> {
        match expression {
            mir::EnumExpression::Case(value) => {
                self.push_scalar(mir::ScalarValue::Enum(value))?;
            }
            mir::EnumExpression::Use { enum_id, operand } => {
                if self.queue_collection_scalar_operand(&operand)? {
                    return Ok(());
                }
                let value = self.eval_operand(&operand)?;
                let mir::ScalarValue::Enum(value) = value else {
                    return Err(InterpreterError::new(
                        "MIR enum operand produced another scalar type",
                    ));
                };
                if value.enum_id != enum_id {
                    return Err(InterpreterError::new(
                        "MIR enum operand changed enum identity",
                    ));
                }
                self.push_scalar(mir::ScalarValue::Enum(value))?;
            }
            mir::EnumExpression::Call {
                enum_id,
                function,
                args,
            } => self.queue_call(
                function,
                args,
                ReturnExpectation::Value(mir::Type::Scalar(mir::ScalarType::Enum(enum_id))),
            )?,
            mir::EnumExpression::Coalesce { left, right, .. } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AfterEnumCoalesce(*right));
                frame.tasks.push(EvaluationTask::NullableScalar(*left));
            }
        }
        Ok(())
    }

    fn expand_float_expression(
        &mut self,
        expression: mir::FloatExpression,
    ) -> Result<(), InterpreterError> {
        match expression {
            mir::FloatExpression::Use { ty, operand } => {
                if self.queue_collection_scalar_operand(&operand)? {
                    return Ok(());
                }
                let value = self.eval_operand(&operand)?;
                let mir::ScalarValue::Float(value) = value else {
                    return Err(InterpreterError::new(
                        "MIR float operand produced another scalar type",
                    ));
                };
                if value.ty != ty {
                    return Err(InterpreterError::new(format!(
                        "MIR float operand produced {}, expected {ty}",
                        value.ty
                    )));
                }
                self.push_scalar(mir::ScalarValue::Float(value))?;
            }
            mir::FloatExpression::Negate { operand, .. } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::FloatNegate);
                frame.tasks.push(EvaluationTask::Float(*operand));
            }
            mir::FloatExpression::Binary {
                op, left, right, ..
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::FloatBinary(op));
                frame.tasks.push(EvaluationTask::Float(*right));
                frame.tasks.push(EvaluationTask::Float(*left));
            }
            mir::FloatExpression::IntToFloat { value } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::IntToFloat);
                frame.tasks.push(EvaluationTask::Integer(*value));
            }
            mir::FloatExpression::Call { ty, function, args } => {
                self.queue_call(
                    function,
                    args,
                    ReturnExpectation::Value(mir::Type::Scalar(mir::ScalarType::Float(ty))),
                )?;
            }
            mir::FloatExpression::Coalesce { left, right, .. } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AfterFloatCoalesce(*right));
                frame.tasks.push(EvaluationTask::NullableScalar(*left));
            }
        }
        Ok(())
    }

    fn expand_bool_expression(
        &mut self,
        condition: mir::BoolExpression,
    ) -> Result<(), InterpreterError> {
        match condition {
            mir::BoolExpression::Use { operand } => {
                if self.queue_collection_scalar_operand(&operand)? {
                    return Ok(());
                }
                let value = self.eval_operand(&operand)?;
                if !matches!(value, mir::ScalarValue::Bool(_)) {
                    return Err(InterpreterError::new(
                        "MIR bool operand produced another scalar type",
                    ));
                }
                self.push_scalar(value)?;
            }
            mir::BoolExpression::Compare { op, left, right } => {
                if left.ty() != right.ty() {
                    return Err(InterpreterError::new(format!(
                        "MIR comparison has operand types {} and {}",
                        left.ty(),
                        right.ty()
                    )));
                }
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::Compare(op));
                frame.tasks.push(EvaluationTask::Value(*right));
                frame.tasks.push(EvaluationTask::Value(*left));
            }
            mir::BoolExpression::StringCompare { op, left, right } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::StringCompare(op));
                frame.tasks.push(EvaluationTask::String(*right));
                frame.tasks.push(EvaluationTask::String(*left));
            }
            mir::BoolExpression::NullableStringCompare { op, left, right } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::NullableStringCompare(op));
                frame.tasks.push(EvaluationTask::NullableString(*right));
                frame.tasks.push(EvaluationTask::NullableString(*left));
            }
            mir::BoolExpression::CollectionEqual { left, right } => {
                let equal = self.byte_collection(left)? == self.byte_collection(right)?;
                self.push_scalar(mir::ScalarValue::Bool(equal))?;
            }
            mir::BoolExpression::NullableScalarIsPresent(value) => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::NullableScalarIsPresent);
                frame.tasks.push(EvaluationTask::NullableScalar(*value));
            }
            mir::BoolExpression::NullableClassIsPresent(value) => {
                let owned = value.owned_temporary_class();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::NullableClassIsPresent(owned));
                frame.tasks.push(EvaluationTask::NullableClass(*value));
            }
            mir::BoolExpression::NullableCollectionIsPresent(value) => {
                let owned = value.owned_temporary_collection();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::NullableCollectionIsPresent(owned));
                frame.tasks.push(EvaluationTask::NullableCollection(*value));
            }
            mir::BoolExpression::NullableSharedReferenceIsPresent(value) => {
                let owned = value.owned_temporary().is_some();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::NullableSharedReferenceIsPresent(owned));
                frame
                    .tasks
                    .push(EvaluationTask::NullableSharedReference(*value));
            }
            mir::BoolExpression::NullableWeakReferenceIsPresent(value) => {
                let owned = value.owned_temporary().is_some();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::NullableWeakReferenceIsPresent(owned));
                frame
                    .tasks
                    .push(EvaluationTask::NullableWeakReference(*value));
            }
            mir::BoolExpression::NullableWritableSharedReferenceIsPresent(value) => {
                let owned = value.owned_temporary();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::NullableWritableSharedReferenceIsPresent(
                        owned,
                    ));
                frame
                    .tasks
                    .push(EvaluationTask::NullableWritableSharedReference(*value));
            }
            mir::BoolExpression::NullableWritableWeakReferenceIsPresent(value) => {
                let owned = value.owned_temporary();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::NullableWritableWeakReferenceIsPresent(
                        owned,
                    ));
                frame
                    .tasks
                    .push(EvaluationTask::NullableWritableWeakReference(*value));
            }
            mir::BoolExpression::NullableSharedReferenceAccessIsPresent(value) => {
                let owned = value.owned_temporary();
                let writable = value.writable();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::NullableSharedReferenceAccessIsPresent {
                        owned,
                        writable,
                    });
                frame
                    .tasks
                    .push(EvaluationTask::NullableSharedReferenceAccess(*value));
            }
            mir::BoolExpression::NullableMixedIsPresent(value) => {
                let ownership = value.ownership();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::NullableMixedIsPresent(ownership));
                frame.tasks.push(EvaluationTask::NullableMixed(*value));
            }
            mir::BoolExpression::MixedIs { mixed, tag } => {
                let ownership = mixed.ownership();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::MixedIs { tag, ownership });
                frame.tasks.push(EvaluationTask::Mixed(*mixed));
            }
            mir::BoolExpression::Not(condition) => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::Not);
                frame.tasks.push(EvaluationTask::Bool(*condition));
            }
            mir::BoolExpression::Binary {
                op: mir::BoolBinaryOp::And,
                left,
                right,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AfterAnd(*right));
                frame.tasks.push(EvaluationTask::Bool(*left));
            }
            mir::BoolExpression::Binary {
                op: mir::BoolBinaryOp::Or,
                left,
                right,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AfterOr(*right));
                frame.tasks.push(EvaluationTask::Bool(*left));
            }
            mir::BoolExpression::Binary {
                op: mir::BoolBinaryOp::Xor,
                left,
                right,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::Xor);
                frame.tasks.push(EvaluationTask::Bool(*right));
                frame.tasks.push(EvaluationTask::Bool(*left));
            }
            mir::BoolExpression::Call { function, args } => {
                self.queue_call(
                    function,
                    args,
                    ReturnExpectation::Value(mir::Type::Scalar(mir::ScalarType::Bool)),
                )?;
            }
            mir::BoolExpression::Coalesce { left, right } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AfterBoolCoalesce(*right));
                frame.tasks.push(EvaluationTask::NullableScalar(*left));
            }
            mir::BoolExpression::CollectionHas {
                collection,
                value,
                op,
            } => {
                let ownership = value.mixed_ownership();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::CollectionHas {
                    collection,
                    op,
                    ownership,
                });
                frame.tasks.push(EvaluationTask::Rvalue(*value));
            }
            mir::BoolExpression::CollectionIsEmpty { collection } => {
                self.current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::CollectionIsEmpty(collection));
            }
        }
        Ok(())
    }

    fn expand_string_expression(
        &mut self,
        expression: mir::StringExpression,
    ) -> Result<(), InterpreterError> {
        match expression {
            mir::StringExpression::Literal(value) => self.push_string(value)?,
            mir::StringExpression::Local(id) => {
                match read_local(&self.current_frame()?.locals, id)? {
                    LocalValue::String(value) => self.push_string(value.clone())?,
                    LocalValue::Scalar(_) => {
                        return Err(InterpreterError::new(format!(
                            "MIR scalar local local{} was used as a string value",
                            id.0
                        )))
                    }
                    LocalValue::NullableString(_) => {
                        return Err(InterpreterError::new(format!(
                            "MIR nullable-string local local{} was used as a string value",
                            id.0
                        )))
                    }
                    LocalValue::Mixed(_) | LocalValue::NullableMixed(_) => {
                        return Err(InterpreterError::new(format!(
                            "MIR mixed local local{} was used as a string value",
                            id.0
                        )))
                    }
                    LocalValue::Class { .. } => {
                        return Err(InterpreterError::new(format!(
                            "MIR class local local{} was used as a string value",
                            id.0
                        )))
                    }
                    LocalValue::NullableScalar { .. } | LocalValue::NullableClass { .. } => {
                        return Err(InterpreterError::new(format!(
                            "MIR nullable local local{} was used as a string value",
                            id.0
                        )))
                    }
                    LocalValue::SharedReference { .. }
                    | LocalValue::WeakReference { .. }
                    | LocalValue::NullableSharedReference { .. }
                    | LocalValue::NullableWeakReference { .. }
                    | LocalValue::WritableSharedReference { .. }
                    | LocalValue::WritableWeakReference { .. }
                    | LocalValue::NullableWritableSharedReference { .. }
                    | LocalValue::NullableWritableWeakReference { .. }
                    | LocalValue::SharedReferenceAccess { .. }
                    | LocalValue::NullableSharedReferenceAccess { .. } => {
                        return Err(InterpreterError::new(format!(
                            "MIR shared handle local local{} was used as a string value",
                            id.0
                        )))
                    }
                    LocalValue::Collection(_) => {
                        return Err(InterpreterError::new(format!(
                            "MIR collection local local{} was used as a string value",
                            id.0
                        )))
                    }
                }
            }
            mir::StringExpression::Static(id) => match self.statics.get(id.0) {
                Some(LocalValue::String(value)) => self.push_string(value.clone())?,
                _ => {
                    return Err(InterpreterError::new(format!(
                        "MIR static{} was used as string",
                        id.0
                    )))
                }
            },
            mir::StringExpression::MixedPayload(local) => {
                let value =
                    mixed_value_from_local(read_local(&self.current_frame()?.locals, local)?)
                        .ok_or_else(|| {
                            InterpreterError::new(
                                "MIR mixed string payload references another local type",
                            )
                        })?;
                let MixedValue::String(value) = value else {
                    return Err(InterpreterError::new(
                        "MIR mixed string payload observed another tag",
                    ));
                };
                self.push_string(value.clone())?;
            }
            mir::StringExpression::NullableLocalAssumeNonNull(id) => {
                match read_local(&self.current_frame()?.locals, id)? {
                    LocalValue::NullableString(Some(value)) => self.push_string(value.clone())?,
                    LocalValue::NullableString(None) => {
                        return Err(InterpreterError::new(
                            "MIR nonnull string expression observed null",
                        ))
                    }
                    _ => {
                        return Err(InterpreterError::new(
                            "MIR nonnull string expression references another local type",
                        ))
                    }
                }
            }
            mir::StringExpression::Property { object, property } => {
                match self.read_property(object, property)? {
                    LocalValue::String(value) => self.push_string(value)?,
                    _ => {
                        return Err(InterpreterError::new(format!(
                            "MIR property{} was used as a string value",
                            property.index
                        )))
                    }
                }
            }
            mir::StringExpression::Concat(parts) => {
                let count = parts.len();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::StringConcat(count));
                for part in parts.into_iter().rev() {
                    frame.tasks.push(EvaluationTask::String(part));
                }
            }
            mir::StringExpression::Display(value) => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::StringDisplay);
                frame.tasks.push(EvaluationTask::Value(value));
            }
            mir::StringExpression::Call { function, args } => {
                self.queue_call(function, args, ReturnExpectation::Value(mir::Type::String))?;
            }
            mir::StringExpression::ReadFile { path, path_span } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::ReadFile(path_span));
                frame.tasks.push(EvaluationTask::String(*path));
            }
            mir::StringExpression::Format(format) => {
                self.current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::Format(*format));
            }
            mir::StringExpression::Coalesce { left, right } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::AfterStringCoalesce(*right));
                frame.tasks.push(EvaluationTask::NullableString(*left));
            }
            mir::StringExpression::CollectionIndex {
                positional,
                collection,
                index,
                remove,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::LoadCollectionValue {
                    positional,
                    collection,
                    index_span: Span::default(),
                    transfer: remove,
                });
                frame.tasks.push(EvaluationTask::Rvalue(*index));
            }
            mir::StringExpression::CollectionKeyAt { collection, offset } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::CollectionKeyString(collection));
                frame.tasks.push(EvaluationTask::Rvalue(*offset));
            }
            mir::StringExpression::Intrinsic(call) => {
                self.queue_string_intrinsic(*call)?;
            }
            mir::StringExpression::EnumBacking { enum_id, value } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::EnumBackingString(enum_id));
                frame.tasks.push(EvaluationTask::Enum(*value));
            }
        }
        Ok(())
    }

    fn expand_nullable_scalar_expression(
        &mut self,
        expression: mir::NullableScalarExpression,
    ) -> Result<(), InterpreterError> {
        let ty = expression.ty();
        match expression {
            mir::NullableScalarExpression::Null(_) => self.push_nullable_scalar(ty, None)?,
            mir::NullableScalarExpression::Value(value) => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::BuildNullableScalarSome(ty));
                frame.tasks.push(EvaluationTask::Value(value));
            }
            mir::NullableScalarExpression::Local { local, .. } => {
                let LocalValue::NullableScalar { ty, value } =
                    read_local(&self.current_frame()?.locals, local)?.clone()
                else {
                    return Err(InterpreterError::new(
                        "nullable scalar references another local type",
                    ));
                };
                self.push_nullable_scalar(ty, value)?;
            }
            mir::NullableScalarExpression::Property {
                object, property, ..
            } => {
                let LocalValue::NullableScalar { ty, value } =
                    self.read_property(object, property)?
                else {
                    return Err(InterpreterError::new(
                        "nullable scalar property has another type",
                    ));
                };
                self.push_nullable_scalar(ty, value)?;
            }
            mir::NullableScalarExpression::Static { id, .. } => {
                let Some(LocalValue::NullableScalar { ty, value }) =
                    self.statics.get(id.0).cloned()
                else {
                    return Err(InterpreterError::new(
                        "nullable scalar static has another type",
                    ));
                };
                self.push_nullable_scalar(ty, value)?;
            }
            mir::NullableScalarExpression::Call { function, args, .. } => {
                self.queue_call(
                    function,
                    args,
                    ReturnExpectation::Value(mir::Type::NullableScalar(ty)),
                )?;
            }
            mir::NullableScalarExpression::StringIntrinsic(call) => {
                self.queue_string_intrinsic(*call)?;
            }
            mir::NullableScalarExpression::NullSafeProperty {
                object, property, ..
            } => {
                let owned_receiver = object.owned_temporary_class();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AfterNullSafeProperty {
                    property,
                    result: mir::Type::NullableScalar(ty),
                    owned_receiver,
                });
                frame.tasks.push(EvaluationTask::NullableClass(*object));
            }
            mir::NullableScalarExpression::NullSafeCall {
                object,
                function,
                args,
                ..
            } => {
                let owned_receiver = object.owned_temporary_class();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AfterNullSafeCall {
                    function,
                    args,
                    result: mir::Type::NullableScalar(ty),
                    owned_receiver,
                });
                frame.tasks.push(EvaluationTask::NullableClass(*object));
            }
            mir::NullableScalarExpression::Coalesce { left, right, .. } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::AfterNullableScalarCoalesce(*right));
                frame.tasks.push(EvaluationTask::NullableScalar(*left));
            }
            mir::NullableScalarExpression::DictionaryGet {
                ty,
                collection,
                key,
                access,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::DictionaryGet {
                    collection,
                    expected: mir::Type::Scalar(ty),
                    access,
                });
                frame.tasks.push(EvaluationTask::Rvalue(*key));
            }
            mir::NullableScalarExpression::CollectionIndexOf { collection, value } => {
                let ownership = value.mixed_ownership();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::CollectionIndexOf {
                    collection,
                    ownership,
                });
                frame.tasks.push(EvaluationTask::Rvalue(*value));
            }
            mir::NullableScalarExpression::Parse { ty, value } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::ParseNullableScalar(ty));
                frame.tasks.push(EvaluationTask::String(*value));
            }
        }
        Ok(())
    }

    fn expand_mixed_expression(
        &mut self,
        expression: mir::MixedExpression,
    ) -> Result<(), InterpreterError> {
        match expression {
            mir::MixedExpression::Local { local, transfer } => {
                let value = if transfer {
                    self.current_frame_mut()?
                        .locals
                        .get_mut(local.0)
                        .and_then(Option::take)
                        .ok_or_else(|| {
                            InterpreterError::new(format!(
                                "MIR mixed local local{} was moved before use",
                                local.0
                            ))
                        })?
                } else {
                    read_local(&self.current_frame()?.locals, local)?.clone()
                };
                let value = mixed_value_from_local(&value)
                    .ok_or_else(|| {
                        InterpreterError::new("MIR mixed expression used another local type")
                    })?
                    .clone();
                self.push_mixed(value)?;
            }
            mir::MixedExpression::Property { object, property } => {
                let LocalValue::Mixed(value) = self.read_property(object, property)? else {
                    return Err(InterpreterError::new(
                        "MIR mixed property contains another value type",
                    ));
                };
                self.push_mixed(value)?;
            }
            mir::MixedExpression::Call { function, args, .. } => {
                self.queue_call(function, args, ReturnExpectation::Value(mir::Type::Mixed))?;
            }
            mir::MixedExpression::BoxValue(value) => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::BuildMixedValue);
                frame.tasks.push(EvaluationTask::Value(value));
            }
            mir::MixedExpression::BoxString { value, .. } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::BuildMixedString);
                frame.tasks.push(EvaluationTask::String(value));
            }
            mir::MixedExpression::BoxClass {
                value,
                payload_owned,
            } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::BuildMixedClass(payload_owned));
                frame.tasks.push(EvaluationTask::Class(value));
            }
            mir::MixedExpression::CollectionIndex {
                positional,
                collection,
                index,
                transfer,
                remove,
            } => {
                let frame = self.current_frame_mut()?;
                // An owning index read clones the box into an owned handle that shares the
                // collection element's payload owner; `removeAt` instead moves the element
                // out, so the popped box is already owned and must not be cloned.
                if transfer && !remove {
                    frame.tasks.push(EvaluationTask::OwnMixed);
                }
                frame.tasks.push(EvaluationTask::LoadCollectionValue {
                    positional,
                    collection,
                    index_span: Span::default(),
                    transfer: remove,
                });
                frame.tasks.push(EvaluationTask::Rvalue(*index));
            }
        }
        Ok(())
    }

    fn expand_nullable_mixed_expression(
        &mut self,
        expression: mir::NullableMixedExpression,
    ) -> Result<(), InterpreterError> {
        match expression {
            mir::NullableMixedExpression::Null => self.push_nullable_mixed(None)?,
            mir::NullableMixedExpression::Mixed(value) => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::BuildNullableMixedSome);
                frame.tasks.push(EvaluationTask::Mixed(value));
            }
            mir::NullableMixedExpression::Local { local, transfer } => {
                let value = if transfer {
                    self.current_frame_mut()?
                        .locals
                        .get_mut(local.0)
                        .and_then(Option::take)
                        .ok_or_else(|| {
                            InterpreterError::new(format!(
                                "MIR nullable mixed local local{} was moved before use",
                                local.0
                            ))
                        })?
                } else {
                    read_local(&self.current_frame()?.locals, local)?.clone()
                };
                let LocalValue::NullableMixed(value) = value else {
                    return Err(InterpreterError::new(
                        "MIR nullable mixed expression used another local type",
                    ));
                };
                self.push_nullable_mixed(value)?;
            }
            mir::NullableMixedExpression::Property { object, property } => {
                let LocalValue::NullableMixed(value) = self.read_property(object, property)? else {
                    return Err(InterpreterError::new(
                        "MIR nullable mixed property contains another value type",
                    ));
                };
                self.push_nullable_mixed(value)?;
            }
            mir::NullableMixedExpression::Call { function, args, .. } => {
                self.queue_call(
                    function,
                    args,
                    ReturnExpectation::Value(mir::Type::NullableMixed),
                )?;
            }
            mir::NullableMixedExpression::Coalesce {
                left,
                right,
                transfer: _,
            } => {
                let left_ownership = left.ownership();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::AfterNullableMixedCoalesce {
                        right: *right,
                        left_ownership,
                    });
                frame.tasks.push(EvaluationTask::NullableMixed(*left));
            }
        }
        Ok(())
    }

    fn expand_nullable_string_expression(
        &mut self,
        expression: mir::NullableStringExpression,
    ) -> Result<(), InterpreterError> {
        match expression {
            mir::NullableStringExpression::Null => self.push_nullable_string(None)?,
            mir::NullableStringExpression::String(value) => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::BuildNullableSome);
                frame.tasks.push(EvaluationTask::String(value));
            }
            mir::NullableStringExpression::Local(local) => {
                match read_local(&self.current_frame()?.locals, local)? {
                    LocalValue::NullableString(value) => {
                        self.push_nullable_string(value.clone())?;
                    }
                    _ => {
                        return Err(InterpreterError::new(format!(
                            "MIR non-nullable local local{} used as ?string",
                            local.0
                        )))
                    }
                }
            }
            mir::NullableStringExpression::Static(id) => match self.statics.get(id.0) {
                Some(LocalValue::NullableString(value)) => {
                    self.push_nullable_string(value.clone())?;
                }
                _ => {
                    return Err(InterpreterError::new(format!(
                        "MIR static{} was used as ?string",
                        id.0
                    )))
                }
            },
            mir::NullableStringExpression::Property { object, property } => {
                match self.read_property(object, property)? {
                    LocalValue::NullableString(value) => self.push_nullable_string(value)?,
                    _ => {
                        return Err(InterpreterError::new(format!(
                            "MIR property{} was used as a nullable string value",
                            property.index
                        )))
                    }
                }
            }
            mir::NullableStringExpression::ReadLine {
                prompt,
                prompt_span,
            } => {
                // The prompt evaluates first and exactly once; the continuation then
                // writes it, flushes, and reads one line.
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::ReadLine(prompt_span));
                frame.tasks.push(EvaluationTask::String(*prompt));
            }
            mir::NullableStringExpression::Call { function, args } => {
                self.queue_call(
                    function,
                    args,
                    ReturnExpectation::Value(mir::Type::NullableString),
                )?;
            }
            mir::NullableStringExpression::Intrinsic(call) => {
                self.queue_string_intrinsic(*call)?;
            }
            mir::NullableStringExpression::NullSafeProperty { object, property } => {
                let owned_receiver = object.owned_temporary_class();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AfterNullSafeProperty {
                    property,
                    result: mir::Type::NullableString,
                    owned_receiver,
                });
                frame.tasks.push(EvaluationTask::NullableClass(*object));
            }
            mir::NullableStringExpression::NullSafeCall {
                object,
                function,
                args,
            } => {
                let owned_receiver = object.owned_temporary_class();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AfterNullSafeCall {
                    function,
                    args,
                    result: mir::Type::NullableString,
                    owned_receiver,
                });
                frame.tasks.push(EvaluationTask::NullableClass(*object));
            }
            mir::NullableStringExpression::Coalesce { left, right } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::AfterNullableStringCoalesce(*right));
                frame.tasks.push(EvaluationTask::NullableString(*left));
            }
            mir::NullableStringExpression::DictionaryGet {
                collection,
                key,
                access,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::DictionaryGet {
                    collection,
                    expected: mir::Type::String,
                    access,
                });
                frame.tasks.push(EvaluationTask::Rvalue(*key));
            }
        }
        Ok(())
    }

    fn expand_class_expression(
        &mut self,
        expression: mir::ClassExpression,
    ) -> Result<(), InterpreterError> {
        match expression {
            mir::ClassExpression::Local {
                class,
                local,
                transfer,
            } => {
                let value = if transfer {
                    self.current_frame_mut()?
                        .locals
                        .get_mut(local.0)
                        .ok_or_else(|| {
                            InterpreterError::new(format!(
                                "MIR local local{} does not exist",
                                local.0
                            ))
                        })?
                        .take()
                        .ok_or_else(|| {
                            InterpreterError::new(format!(
                                "MIR local local{} was moved before use",
                                local.0
                            ))
                        })?
                } else {
                    read_local(&self.current_frame()?.locals, local)?.clone()
                };
                if local_value_type(&value) != mir::Type::Class(class) {
                    return Err(InterpreterError::new(format!(
                        "MIR class expression expected class#{}, got {}",
                        class.0,
                        local_value_type(&value)
                    )));
                }
                let LocalValue::Class { object, class } = value else {
                    unreachable!("checked class local value")
                };
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Class { object, class });
            }
            mir::ClassExpression::Property {
                class,
                object,
                property,
            } => {
                let value = self.read_property(object, property)?;
                let LocalValue::Class {
                    object,
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new(format!(
                        "MIR property{} was used as a class value",
                        property.index
                    )));
                };
                if actual != class {
                    return Err(InterpreterError::new(format!(
                        "MIR class property produced class#{}, expected class#{}",
                        actual.0, class.0
                    )));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Class {
                        object,
                        class: actual,
                    });
            }
            mir::ClassExpression::Call {
                class,
                function,
                args,
                ..
            } => {
                self.queue_call(
                    function,
                    args,
                    ReturnExpectation::Value(mir::Type::Class(class)),
                )?;
            }
            mir::ClassExpression::New {
                class,
                properties,
                constructor,
                args,
            } => {
                let temporary_arg_drops = if let Some(constructor) = constructor {
                    let definition = function_in(self.program, constructor)?;
                    temporary_argument_drop_order(&args, definition, 1, |index| {
                        properties.iter().any(|property| {
                            matches!(
                                property.source,
                                mir::PropertyValueSource::ConstructorArgument(argument)
                                    if argument == index
                            )
                        })
                    })?
                } else {
                    Vec::new()
                };
                let property_expression_count = properties
                    .iter()
                    .filter(|property| {
                        matches!(property.source, mir::PropertyValueSource::Expression(_))
                    })
                    .count();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::BuildClassNew {
                    class,
                    properties: properties.clone(),
                    constructor,
                    argument_count: args.len(),
                    property_expression_count,
                    temporary_arg_drops,
                });
                for argument in args.into_iter().rev() {
                    frame.tasks.push(EvaluationTask::Rvalue(argument));
                }
                for property in properties.into_iter().rev() {
                    if let mir::PropertyValueSource::Expression(value) = property.source {
                        frame.tasks.push(EvaluationTask::Rvalue(value));
                    }
                }
            }
            mir::ClassExpression::NullableLocalAssumeNonNull {
                class,
                local,
                transfer,
            } => {
                let value = if transfer {
                    self.current_frame_mut()?
                        .locals
                        .get_mut(local.0)
                        .and_then(Option::take)
                        .ok_or_else(|| {
                            InterpreterError::new("nullable class was moved before use")
                        })?
                } else {
                    read_local(&self.current_frame()?.locals, local)?.clone()
                };
                let LocalValue::NullableClass {
                    object: Some(object),
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "nonnull class expression observed null",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "nonnull class expression has another class",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Class { object, class });
            }
            mir::ClassExpression::Coalesce {
                left,
                right,
                transfer,
                ..
            } => {
                let left_owned = left.owned_temporary_class().is_some();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AfterClassCoalesce {
                    right: *right,
                    left_owned,
                    transfer,
                });
                frame.tasks.push(EvaluationTask::NullableClass(*left));
            }
            mir::ClassExpression::CollectionIndex {
                positional,
                class,
                collection,
                index,
                transfer,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::CollectionIndexClass {
                    positional,
                    collection,
                    class,
                    transfer,
                });
                frame.tasks.push(EvaluationTask::Rvalue(*index));
            }
            mir::ClassExpression::MixedPayload {
                class,
                mixed,
                transfer,
            } => {
                let value = if transfer {
                    match mixed_value_from_local(read_local(&self.current_frame()?.locals, mixed)?)
                    {
                        Some(MixedValue::Class { class: actual, .. }) if *actual == class => {}
                        Some(MixedValue::Class { .. }) => {
                            return Err(InterpreterError::new(
                                "MIR mixed class payload observed another class",
                            ));
                        }
                        Some(_) => {
                            return Err(InterpreterError::new(
                                "MIR mixed class payload observed another tag",
                            ));
                        }
                        None => {
                            return Err(InterpreterError::new(
                                "MIR mixed class payload references another local type",
                            ));
                        }
                    }
                    let slot = self
                        .current_frame_mut()?
                        .locals
                        .get_mut(mixed.0)
                        .ok_or_else(|| InterpreterError::new("MIR mixed local does not exist"))?;
                    match slot.take() {
                        Some(LocalValue::Mixed(value)) => value,
                        Some(LocalValue::NullableMixed(Some(value))) => value,
                        Some(value) => {
                            *slot = Some(value);
                            return Err(InterpreterError::new(
                                "MIR mixed class payload references another local type",
                            ));
                        }
                        None => {
                            return Err(InterpreterError::new(
                                "MIR mixed class payload was read before assignment",
                            ));
                        }
                    }
                } else {
                    mixed_value_from_local(read_local(&self.current_frame()?.locals, mixed)?)
                        .ok_or_else(|| {
                            InterpreterError::new(
                                "MIR mixed class payload references another local type",
                            )
                        })?
                        .clone()
                };
                let MixedValue::Class {
                    object,
                    class: actual,
                    owner,
                    payload_owned,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "MIR mixed class payload observed another tag",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "MIR mixed class payload observed another class",
                    ));
                }
                if transfer {
                    // Moving the class payload out is only sound when this box holds the
                    // final owning claim. If another box still shares the owner (e.g. read
                    // from a collection with `mixed $x = $items[0]`) or the box only borrows
                    // its payload, transferring the object would double-drop it once the
                    // other holder releases the final claim. Refuse it, matching the native
                    // backends' runtime panic.
                    let claims = owner.get();
                    let owns_final = if claims == 0 {
                        false
                    } else {
                        owner.set(claims - 1);
                        claims == 1 && payload_owned
                    };
                    if !owns_final {
                        self.pending_panic = Some("P1321");
                        return Ok(());
                    }
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Class { object, class });
            }
            mir::ClassExpression::SharedPayload { class, reference } => {
                let drop_receiver = reference.owned_temporary().is_some();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::FinishSharedPayload(class, drop_receiver));
                frame
                    .tasks
                    .push(EvaluationTask::SharedReference(*reference));
            }
            mir::ClassExpression::SharedAccessPayload { class, access, .. } => {
                let value = self.shared_access_payload(access)?;
                let LocalValue::Class {
                    object,
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "MIR shared access payload is not a class",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "MIR shared access payload has another class",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Class { object, class });
            }
        }
        Ok(())
    }

    fn expand_shared_reference_expression(
        &mut self,
        expression: mir::SharedReferenceExpression,
    ) -> Result<(), InterpreterError> {
        match expression {
            mir::SharedReferenceExpression::New { class, value } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::BuildSharedReference(class));
                frame.tasks.push(EvaluationTask::Class(*value));
            }
            mir::SharedReferenceExpression::Local {
                class,
                local,
                transfer,
            } => {
                let value = self.read_or_take_local(local, transfer)?;
                let LocalValue::SharedReference {
                    control,
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "shared expression read another local type",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "shared expression changed payload class",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::SharedReference { control, class });
            }
            mir::SharedReferenceExpression::NullableLocalAssumeNonNull {
                class,
                local,
                transfer,
            } => {
                let value = self.read_or_take_local(local, transfer)?;
                let LocalValue::NullableSharedReference {
                    control,
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "nonnull shared expression read another local type",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "nonnull shared expression changed payload class",
                    ));
                }
                let control = control.ok_or_else(|| {
                    InterpreterError::new("nonnull shared expression received null")
                })?;
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::SharedReference { control, class });
            }
            mir::SharedReferenceExpression::Property {
                class,
                object,
                property,
            } => {
                let value = self.read_property(object, property)?;
                let LocalValue::SharedReference {
                    control,
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "shared property read another value type",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "shared property changed payload class",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::SharedReference { control, class });
            }
            mir::SharedReferenceExpression::Call {
                class,
                function,
                args,
                ..
            } => self.queue_call(
                function,
                args,
                ReturnExpectation::Value(mir::Type::SharedReference(class)),
            )?,
            mir::SharedReferenceExpression::Share { class, value } => {
                let drop_receiver = value.owned_temporary().is_some();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::FinishSharedShare(class, drop_receiver));
                frame.tasks.push(EvaluationTask::SharedReference(*value));
            }
            mir::SharedReferenceExpression::Coalesce {
                left,
                right,
                transfer,
                ..
            } => {
                let left_owned = left.owned_temporary().is_some();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AfterSharedCoalesce {
                    right: *right,
                    left_owned,
                    transfer,
                });
                frame
                    .tasks
                    .push(EvaluationTask::NullableSharedReference(*left));
            }
            mir::SharedReferenceExpression::CollectionIndex {
                positional,
                class,
                collection,
                index,
                remove,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::CollectionIndexShared {
                    positional,
                    collection,
                    class,
                    weak: false,
                    nullable: false,
                    transfer: remove,
                });
                frame.tasks.push(EvaluationTask::Rvalue(*index));
            }
        }
        Ok(())
    }

    fn expand_weak_reference_expression(
        &mut self,
        expression: mir::WeakReferenceExpression,
    ) -> Result<(), InterpreterError> {
        match expression {
            mir::WeakReferenceExpression::Local {
                class,
                local,
                transfer,
            } => {
                let value = self.read_or_take_local(local, transfer)?;
                let LocalValue::WeakReference {
                    control,
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "weak expression read another local type",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "weak expression changed payload class",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::WeakReference { control, class });
            }
            mir::WeakReferenceExpression::NullableLocalAssumeNonNull {
                class,
                local,
                transfer,
            } => {
                let value = self.read_or_take_local(local, transfer)?;
                let LocalValue::NullableWeakReference {
                    control,
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "nonnull weak expression read another local type",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "nonnull weak expression changed payload class",
                    ));
                }
                let control = control.ok_or_else(|| {
                    InterpreterError::new("nonnull weak expression received null")
                })?;
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::WeakReference { control, class });
            }
            mir::WeakReferenceExpression::Property {
                class,
                object,
                property,
            } => {
                let value = self.read_property(object, property)?;
                let LocalValue::WeakReference {
                    control,
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "weak property read another value type",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new("weak property changed payload class"));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::WeakReference { control, class });
            }
            mir::WeakReferenceExpression::Call {
                class,
                function,
                args,
                ..
            } => self.queue_call(
                function,
                args,
                ReturnExpectation::Value(mir::Type::WeakReference(class)),
            )?,
            mir::WeakReferenceExpression::Create { class, value } => {
                let drop_receiver = value.owned_temporary().is_some();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::FinishWeakCreation(class, drop_receiver));
                frame.tasks.push(EvaluationTask::SharedReference(*value));
            }
            mir::WeakReferenceExpression::Coalesce {
                left,
                right,
                transfer,
                ..
            } => {
                let left_owned = left.owned_temporary().is_some();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AfterWeakCoalesce {
                    right: *right,
                    left_owned,
                    transfer,
                });
                frame
                    .tasks
                    .push(EvaluationTask::NullableWeakReference(*left));
            }
            mir::WeakReferenceExpression::CollectionIndex {
                positional,
                class,
                collection,
                index,
                remove,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::CollectionIndexShared {
                    positional,
                    collection,
                    class,
                    weak: true,
                    nullable: false,
                    transfer: remove,
                });
                frame.tasks.push(EvaluationTask::Rvalue(*index));
            }
        }
        Ok(())
    }

    fn expand_nullable_shared_reference_expression(
        &mut self,
        expression: mir::NullableSharedReferenceExpression,
    ) -> Result<(), InterpreterError> {
        match expression {
            mir::NullableSharedReferenceExpression::Null(class) => self
                .current_frame_mut()?
                .values
                .push(EvaluationValue::NullableSharedReference {
                    control: None,
                    class,
                }),
            mir::NullableSharedReferenceExpression::Shared(value) => {
                let class = value.class();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::BuildNullableSharedSome(class));
                frame.tasks.push(EvaluationTask::SharedReference(value));
            }
            mir::NullableSharedReferenceExpression::Local {
                class,
                local,
                transfer,
            } => {
                let value = self.read_or_take_local(local, transfer)?;
                let LocalValue::NullableSharedReference {
                    control,
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "nullable shared expression read another local type",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "nullable shared expression changed payload class",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::NullableSharedReference { control, class });
            }
            mir::NullableSharedReferenceExpression::Property {
                class,
                object,
                property,
            } => {
                let value = self.read_property(object, property)?;
                let LocalValue::NullableSharedReference {
                    control,
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "nullable shared property read another value type",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "nullable shared property changed payload class",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::NullableSharedReference { control, class });
            }
            mir::NullableSharedReferenceExpression::Call {
                class,
                function,
                args,
                ..
            } => self.queue_call(
                function,
                args,
                ReturnExpectation::Value(mir::Type::NullableSharedReference(class)),
            )?,
            mir::NullableSharedReferenceExpression::Acquire { class, value } => {
                let drop_receiver = value.owned_temporary().is_some();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::FinishWeakAcquire(class, drop_receiver));
                frame.tasks.push(EvaluationTask::WeakReference(*value));
            }
            mir::NullableSharedReferenceExpression::NullSafeShare { class, value } => {
                let drop_receiver = value.owned_temporary().is_some();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::FinishNullSafeShare(class, drop_receiver));
                frame
                    .tasks
                    .push(EvaluationTask::NullableSharedReference(*value));
            }
            mir::NullableSharedReferenceExpression::NullSafeAcquire { class, value } => {
                let drop_receiver = value.owned_temporary().is_some();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::FinishNullSafeWeakAcquire(
                    class,
                    drop_receiver,
                ));
                frame
                    .tasks
                    .push(EvaluationTask::NullableWeakReference(*value));
            }
            mir::NullableSharedReferenceExpression::Coalesce {
                left,
                right,
                transfer,
                ..
            } => {
                let left_owned = left.owned_temporary().is_some();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::AfterNullableSharedCoalesce {
                        right: *right,
                        left_owned,
                        transfer,
                    });
                frame
                    .tasks
                    .push(EvaluationTask::NullableSharedReference(*left));
            }
            mir::NullableSharedReferenceExpression::DictionaryGet {
                class,
                collection,
                key,
                access,
                stored_nullable,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::DictionaryGet {
                    collection,
                    expected: if stored_nullable {
                        mir::Type::NullableSharedReference(class)
                    } else {
                        mir::Type::SharedReference(class)
                    },
                    access,
                });
                frame.tasks.push(EvaluationTask::Rvalue(*key));
            }
            mir::NullableSharedReferenceExpression::CollectionIndex {
                positional,
                class,
                collection,
                index,
                remove,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::CollectionIndexShared {
                    positional,
                    collection,
                    class,
                    weak: false,
                    nullable: true,
                    transfer: remove,
                });
                frame.tasks.push(EvaluationTask::Rvalue(*index));
            }
        }
        Ok(())
    }

    fn expand_nullable_weak_reference_expression(
        &mut self,
        expression: mir::NullableWeakReferenceExpression,
    ) -> Result<(), InterpreterError> {
        match expression {
            mir::NullableWeakReferenceExpression::Null(class) => self
                .current_frame_mut()?
                .values
                .push(EvaluationValue::NullableWeakReference {
                    control: None,
                    class,
                }),
            mir::NullableWeakReferenceExpression::Weak(value) => {
                let class = value.class();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::BuildNullableWeakSome(class));
                frame.tasks.push(EvaluationTask::WeakReference(value));
            }
            mir::NullableWeakReferenceExpression::Local {
                class,
                local,
                transfer,
            } => {
                let value = self.read_or_take_local(local, transfer)?;
                let LocalValue::NullableWeakReference {
                    control,
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "nullable weak expression read another local type",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "nullable weak expression changed payload class",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::NullableWeakReference { control, class });
            }
            mir::NullableWeakReferenceExpression::Property {
                class,
                object,
                property,
            } => {
                let value = self.read_property(object, property)?;
                let LocalValue::NullableWeakReference {
                    control,
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "nullable weak property read another value type",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "nullable weak property changed payload class",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::NullableWeakReference { control, class });
            }
            mir::NullableWeakReferenceExpression::Call {
                class,
                function,
                args,
                ..
            } => self.queue_call(
                function,
                args,
                ReturnExpectation::Value(mir::Type::NullableWeakReference(class)),
            )?,
            mir::NullableWeakReferenceExpression::NullSafeCreate { class, value } => {
                let drop_receiver = value.owned_temporary().is_some();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::FinishNullSafeWeakCreation(
                    class,
                    drop_receiver,
                ));
                frame
                    .tasks
                    .push(EvaluationTask::NullableSharedReference(*value));
            }
            mir::NullableWeakReferenceExpression::Coalesce {
                left,
                right,
                transfer,
                ..
            } => {
                let left_owned = left.owned_temporary().is_some();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AfterNullableWeakCoalesce {
                    right: *right,
                    left_owned,
                    transfer,
                });
                frame
                    .tasks
                    .push(EvaluationTask::NullableWeakReference(*left));
            }
            mir::NullableWeakReferenceExpression::DictionaryGet {
                class,
                collection,
                key,
                access,
                stored_nullable,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::DictionaryGet {
                    collection,
                    expected: if stored_nullable {
                        mir::Type::NullableWeakReference(class)
                    } else {
                        mir::Type::WeakReference(class)
                    },
                    access,
                });
                frame.tasks.push(EvaluationTask::Rvalue(*key));
            }
            mir::NullableWeakReferenceExpression::CollectionIndex {
                positional,
                class,
                collection,
                index,
                remove,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::CollectionIndexShared {
                    positional,
                    collection,
                    class,
                    weak: true,
                    nullable: true,
                    transfer: remove,
                });
                frame.tasks.push(EvaluationTask::Rvalue(*index));
            }
        }
        Ok(())
    }

    fn expand_writable_shared_reference_expression(
        &mut self,
        expression: mir::WritableSharedReferenceExpression,
    ) -> Result<(), InterpreterError> {
        match expression {
            mir::WritableSharedReferenceExpression::New { payload, value } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::BuildWritableSharedReference(payload));
                frame.tasks.push(EvaluationTask::Rvalue(*value));
            }
            mir::WritableSharedReferenceExpression::Local {
                payload,
                local,
                transfer,
            } => {
                let value = self.read_or_take_local(local, transfer)?;
                let LocalValue::WritableSharedReference {
                    control,
                    payload: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "writable shared expression read another local type",
                    ));
                };
                if actual != payload {
                    return Err(InterpreterError::new(
                        "writable shared expression changed payload type",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::WritableSharedReference { control, payload });
            }
            mir::WritableSharedReferenceExpression::NullableLocalAssumeNonNull {
                payload,
                local,
                transfer,
            } => {
                let value = self.read_or_take_local(local, transfer)?;
                let LocalValue::NullableWritableSharedReference {
                    control,
                    payload: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "nonnull writable shared expression read another local type",
                    ));
                };
                if actual != payload {
                    return Err(InterpreterError::new(
                        "nonnull writable shared expression changed payload type",
                    ));
                }
                let control = control.ok_or_else(|| {
                    InterpreterError::new("nonnull writable shared expression received null")
                })?;
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::WritableSharedReference { control, payload });
            }
            mir::WritableSharedReferenceExpression::Property {
                payload,
                object,
                property,
            } => {
                let value = self.read_property(object, property)?;
                let LocalValue::WritableSharedReference {
                    control,
                    payload: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "writable shared property read another value type",
                    ));
                };
                if actual != payload {
                    return Err(InterpreterError::new(
                        "writable shared property changed payload type",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::WritableSharedReference { control, payload });
            }
            mir::WritableSharedReferenceExpression::Call {
                payload,
                function,
                args,
                ..
            } => self.queue_call(
                function,
                args,
                ReturnExpectation::Value(mir::Type::WritableSharedReference(payload)),
            )?,
            mir::WritableSharedReferenceExpression::Share { payload, value } => {
                let drop_receiver = value.owned_temporary();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::FinishWritableSharedShare(
                    payload,
                    drop_receiver,
                ));
                frame
                    .tasks
                    .push(EvaluationTask::WritableSharedReference(*value));
            }
            mir::WritableSharedReferenceExpression::Coalesce {
                left,
                right,
                transfer,
                ..
            } => {
                let left_owned = left.owned_temporary();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::AfterWritableSharedCoalesce {
                        right: *right,
                        left_owned,
                        transfer,
                    });
                frame
                    .tasks
                    .push(EvaluationTask::NullableWritableSharedReference(*left));
            }
            mir::WritableSharedReferenceExpression::CollectionIndex {
                positional,
                payload,
                collection,
                index,
                remove,
            } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::CollectionIndexWritableShared {
                        positional,
                        collection,
                        payload,
                        weak: false,
                        nullable: false,
                        transfer: remove,
                    });
                frame.tasks.push(EvaluationTask::Rvalue(*index));
            }
        }
        Ok(())
    }

    fn expand_writable_weak_reference_expression(
        &mut self,
        expression: mir::WritableWeakReferenceExpression,
    ) -> Result<(), InterpreterError> {
        match expression {
            mir::WritableWeakReferenceExpression::Local {
                payload,
                local,
                transfer,
            } => {
                let value = self.read_or_take_local(local, transfer)?;
                let LocalValue::WritableWeakReference {
                    control,
                    payload: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "writable weak expression read another local type",
                    ));
                };
                if actual != payload {
                    return Err(InterpreterError::new(
                        "writable weak expression changed payload type",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::WritableWeakReference { control, payload });
            }
            mir::WritableWeakReferenceExpression::NullableLocalAssumeNonNull {
                payload,
                local,
                transfer,
            } => {
                let value = self.read_or_take_local(local, transfer)?;
                let LocalValue::NullableWritableWeakReference {
                    control,
                    payload: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "nonnull writable weak expression read another local type",
                    ));
                };
                if actual != payload {
                    return Err(InterpreterError::new(
                        "nonnull writable weak expression changed payload type",
                    ));
                }
                let control = control.ok_or_else(|| {
                    InterpreterError::new("nonnull writable weak expression received null")
                })?;
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::WritableWeakReference { control, payload });
            }
            mir::WritableWeakReferenceExpression::Property {
                payload,
                object,
                property,
            } => {
                let value = self.read_property(object, property)?;
                let LocalValue::WritableWeakReference {
                    control,
                    payload: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "writable weak property read another value type",
                    ));
                };
                if actual != payload {
                    return Err(InterpreterError::new(
                        "writable weak property changed payload type",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::WritableWeakReference { control, payload });
            }
            mir::WritableWeakReferenceExpression::Call {
                payload,
                function,
                args,
                ..
            } => self.queue_call(
                function,
                args,
                ReturnExpectation::Value(mir::Type::WritableWeakReference(payload)),
            )?,
            mir::WritableWeakReferenceExpression::Create { payload, value } => {
                let drop_receiver = value.owned_temporary();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::FinishWritableWeakCreation(
                    payload,
                    drop_receiver,
                ));
                frame
                    .tasks
                    .push(EvaluationTask::WritableSharedReference(*value));
            }
            mir::WritableWeakReferenceExpression::Coalesce {
                left,
                right,
                transfer,
                ..
            } => {
                let left_owned = left.owned_temporary();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AfterWritableWeakCoalesce {
                    right: *right,
                    left_owned,
                    transfer,
                });
                frame
                    .tasks
                    .push(EvaluationTask::NullableWritableWeakReference(*left));
            }
            mir::WritableWeakReferenceExpression::CollectionIndex {
                positional,
                payload,
                collection,
                index,
                remove,
            } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::CollectionIndexWritableShared {
                        positional,
                        collection,
                        payload,
                        weak: true,
                        nullable: false,
                        transfer: remove,
                    });
                frame.tasks.push(EvaluationTask::Rvalue(*index));
            }
        }
        Ok(())
    }

    fn expand_nullable_writable_shared_reference_expression(
        &mut self,
        expression: mir::NullableWritableSharedReferenceExpression,
    ) -> Result<(), InterpreterError> {
        match expression {
            mir::NullableWritableSharedReferenceExpression::Null(payload) => {
                self.current_frame_mut()?.values.push(
                    EvaluationValue::NullableWritableSharedReference {
                        control: None,
                        payload,
                    },
                );
            }
            mir::NullableWritableSharedReferenceExpression::Strong(value) => {
                let payload = value.payload();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::BuildNullableWritableSharedSome(payload));
                frame
                    .tasks
                    .push(EvaluationTask::WritableSharedReference(value));
            }
            mir::NullableWritableSharedReferenceExpression::Local {
                payload,
                local,
                transfer,
            } => {
                let value = self.read_or_take_local(local, transfer)?;
                let LocalValue::NullableWritableSharedReference {
                    control,
                    payload: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "nullable writable shared expression read another local type",
                    ));
                };
                if actual != payload {
                    return Err(InterpreterError::new(
                        "nullable writable shared expression changed payload type",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::NullableWritableSharedReference { control, payload });
            }
            mir::NullableWritableSharedReferenceExpression::Property {
                payload,
                object,
                property,
            } => {
                let value = self.read_property(object, property)?;
                let LocalValue::NullableWritableSharedReference {
                    control,
                    payload: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "nullable writable shared property read another value type",
                    ));
                };
                if actual != payload {
                    return Err(InterpreterError::new(
                        "nullable writable shared property changed payload type",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::NullableWritableSharedReference { control, payload });
            }
            mir::NullableWritableSharedReferenceExpression::Call {
                payload,
                function,
                args,
                ..
            } => self.queue_call(
                function,
                args,
                ReturnExpectation::Value(mir::Type::NullableWritableSharedReference(payload)),
            )?,
            mir::NullableWritableSharedReferenceExpression::Acquire { payload, value } => {
                let drop_receiver = value.owned_temporary();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::FinishWritableWeakAcquire(
                    payload,
                    drop_receiver,
                ));
                frame
                    .tasks
                    .push(EvaluationTask::WritableWeakReference(*value));
            }
            mir::NullableWritableSharedReferenceExpression::NullSafeShare { payload, value } => {
                let drop_receiver = value.owned_temporary();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::FinishWritableNullSafeShare(
                        payload,
                        drop_receiver,
                    ));
                frame
                    .tasks
                    .push(EvaluationTask::NullableWritableSharedReference(*value));
            }
            mir::NullableWritableSharedReferenceExpression::NullSafeAcquire { payload, value } => {
                let drop_receiver = value.owned_temporary();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::FinishWritableNullSafeWeakAcquire(
                        payload,
                        drop_receiver,
                    ));
                frame
                    .tasks
                    .push(EvaluationTask::NullableWritableWeakReference(*value));
            }
            mir::NullableWritableSharedReferenceExpression::Coalesce {
                left,
                right,
                transfer,
                ..
            } => {
                let left_owned = left.owned_temporary();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::AfterNullableWritableSharedCoalesce {
                        right: *right,
                        left_owned,
                        transfer,
                    });
                frame
                    .tasks
                    .push(EvaluationTask::NullableWritableSharedReference(*left));
            }
            mir::NullableWritableSharedReferenceExpression::DictionaryGet {
                payload,
                collection,
                key,
                access,
                stored_nullable,
            } => {
                let expected = if stored_nullable {
                    mir::Type::NullableWritableSharedReference(payload)
                } else {
                    mir::Type::WritableSharedReference(payload)
                };
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::DictionaryGet {
                    collection,
                    expected,
                    access,
                });
                frame.tasks.push(EvaluationTask::Rvalue(*key));
            }
        }
        Ok(())
    }

    fn expand_nullable_writable_weak_reference_expression(
        &mut self,
        expression: mir::NullableWritableWeakReferenceExpression,
    ) -> Result<(), InterpreterError> {
        match expression {
            mir::NullableWritableWeakReferenceExpression::Null(payload) => {
                self.current_frame_mut()?.values.push(
                    EvaluationValue::NullableWritableWeakReference {
                        control: None,
                        payload,
                    },
                );
            }
            mir::NullableWritableWeakReferenceExpression::Weak(value) => {
                let payload = value.payload();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::BuildNullableWritableWeakSome(payload));
                frame
                    .tasks
                    .push(EvaluationTask::WritableWeakReference(value));
            }
            mir::NullableWritableWeakReferenceExpression::Local {
                payload,
                local,
                transfer,
            } => {
                let value = self.read_or_take_local(local, transfer)?;
                let LocalValue::NullableWritableWeakReference {
                    control,
                    payload: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "nullable writable weak expression read another local type",
                    ));
                };
                if actual != payload {
                    return Err(InterpreterError::new(
                        "nullable writable weak expression changed payload type",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::NullableWritableWeakReference { control, payload });
            }
            mir::NullableWritableWeakReferenceExpression::Property {
                payload,
                object,
                property,
            } => {
                let value = self.read_property(object, property)?;
                let LocalValue::NullableWritableWeakReference {
                    control,
                    payload: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "nullable writable weak property read another value type",
                    ));
                };
                if actual != payload {
                    return Err(InterpreterError::new(
                        "nullable writable weak property changed payload type",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::NullableWritableWeakReference { control, payload });
            }
            mir::NullableWritableWeakReferenceExpression::Call {
                payload,
                function,
                args,
                ..
            } => self.queue_call(
                function,
                args,
                ReturnExpectation::Value(mir::Type::NullableWritableWeakReference(payload)),
            )?,
            mir::NullableWritableWeakReferenceExpression::NullSafeCreate { payload, value } => {
                let drop_receiver = value.owned_temporary();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::FinishWritableNullSafeWeakCreation(
                        payload,
                        drop_receiver,
                    ));
                frame
                    .tasks
                    .push(EvaluationTask::NullableWritableSharedReference(*value));
            }
            mir::NullableWritableWeakReferenceExpression::Coalesce {
                left,
                right,
                transfer,
                ..
            } => {
                let left_owned = left.owned_temporary();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::AfterNullableWritableWeakCoalesce {
                        right: *right,
                        left_owned,
                        transfer,
                    });
                frame
                    .tasks
                    .push(EvaluationTask::NullableWritableWeakReference(*left));
            }
            mir::NullableWritableWeakReferenceExpression::DictionaryGet {
                payload,
                collection,
                key,
                access,
                stored_nullable,
            } => {
                let expected = if stored_nullable {
                    mir::Type::NullableWritableWeakReference(payload)
                } else {
                    mir::Type::WritableWeakReference(payload)
                };
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::DictionaryGet {
                    collection,
                    expected,
                    access,
                });
                frame.tasks.push(EvaluationTask::Rvalue(*key));
            }
        }
        Ok(())
    }

    fn expand_shared_reference_access_expression(
        &mut self,
        expression: mir::SharedReferenceAccessExpression,
    ) -> Result<(), InterpreterError> {
        match expression {
            mir::SharedReferenceAccessExpression::Local {
                payload,
                local,
                writable,
                transfer,
            } => {
                let value = self.read_or_take_local(local, transfer)?;
                let LocalValue::SharedReferenceAccess {
                    control,
                    payload: actual,
                    writable: actual_writable,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "shared access expression read another local type",
                    ));
                };
                if actual != payload || actual_writable != writable {
                    return Err(InterpreterError::new(
                        "shared access expression changed access type",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::SharedReferenceAccess {
                        control,
                        payload,
                        writable,
                    });
            }
            mir::SharedReferenceAccessExpression::NullableLocalAssumeNonNull {
                payload,
                local,
                writable,
                transfer,
            } => {
                let value = self.read_or_take_local(local, transfer)?;
                let LocalValue::NullableSharedReferenceAccess {
                    control,
                    payload: actual,
                    writable: actual_writable,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "shared access narrowing read another local type",
                    ));
                };
                let control = control.ok_or_else(|| {
                    InterpreterError::new("shared access narrowing assumed an absent value")
                })?;
                if actual != payload || actual_writable != writable {
                    return Err(InterpreterError::new(
                        "shared access narrowing changed access type",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::SharedReferenceAccess {
                        control,
                        payload,
                        writable,
                    });
            }
            mir::SharedReferenceAccessExpression::Property {
                payload,
                object,
                property,
                writable,
            } => {
                let value = self.read_property(object, property)?;
                let LocalValue::SharedReferenceAccess {
                    control,
                    payload: actual,
                    writable: actual_writable,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "shared access property read another value type",
                    ));
                };
                if actual != payload || actual_writable != writable {
                    return Err(InterpreterError::new(
                        "shared access property changed access type",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::SharedReferenceAccess {
                        control,
                        payload,
                        writable,
                    });
            }
            mir::SharedReferenceAccessExpression::CollectionIndex {
                positional,
                payload,
                collection,
                index,
                writable,
                remove,
            } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::CollectionIndexSharedAccess {
                        positional,
                        collection,
                        payload,
                        writable,
                        nullable: false,
                        remove,
                    });
                frame.tasks.push(EvaluationTask::Rvalue(*index));
            }
            mir::SharedReferenceAccessExpression::Call {
                payload,
                function,
                args,
                writable,
                ..
            } => {
                let ty = if writable {
                    mir::Type::WritableSharedReferenceAccess(payload)
                } else {
                    mir::Type::ReadonlySharedReferenceAccess(payload)
                };
                self.queue_call(function, args, ReturnExpectation::Value(ty))?;
            }
            mir::SharedReferenceAccessExpression::Acquire {
                payload,
                value,
                writable,
                span,
            } => {
                let drop_receiver = value.owned_temporary();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::FinishSharedAccessAcquire {
                    payload,
                    writable,
                    drop_receiver,
                    span,
                });
                frame
                    .tasks
                    .push(EvaluationTask::WritableSharedReference(*value));
            }
        }
        Ok(())
    }

    fn expand_nullable_shared_reference_access_expression(
        &mut self,
        expression: mir::NullableSharedReferenceAccessExpression,
    ) -> Result<(), InterpreterError> {
        match expression {
            mir::NullableSharedReferenceAccessExpression::Null { payload, writable } => {
                self.current_frame_mut()?.values.push(
                    EvaluationValue::NullableSharedReferenceAccess {
                        control: None,
                        payload,
                        writable,
                    },
                );
            }
            mir::NullableSharedReferenceAccessExpression::Access(value) => {
                let ty = if value.writable() {
                    mir::Type::NullableWritableSharedReferenceAccess(value.payload())
                } else {
                    mir::Type::NullableReadonlySharedReferenceAccess(value.payload())
                };
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::WrapNullable(ty));
                frame
                    .tasks
                    .push(EvaluationTask::SharedReferenceAccess(*value));
            }
            mir::NullableSharedReferenceAccessExpression::Local {
                payload,
                local,
                writable,
                transfer,
            } => {
                let value = self.read_or_take_local(local, transfer)?;
                let LocalValue::NullableSharedReferenceAccess {
                    control,
                    payload: actual,
                    writable: actual_writable,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "nullable shared access expression read another local type",
                    ));
                };
                if actual != payload || actual_writable != writable {
                    return Err(InterpreterError::new(
                        "nullable shared access expression changed access type",
                    ));
                }
                self.current_frame_mut()?.values.push(
                    EvaluationValue::NullableSharedReferenceAccess {
                        control,
                        payload,
                        writable,
                    },
                );
            }
            mir::NullableSharedReferenceAccessExpression::Property {
                payload,
                object,
                property,
                writable,
            } => {
                let value = self.read_property(object, property)?;
                let LocalValue::NullableSharedReferenceAccess {
                    control,
                    payload: actual,
                    writable: actual_writable,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "nullable shared access property read another value type",
                    ));
                };
                if actual != payload || actual_writable != writable {
                    return Err(InterpreterError::new(
                        "nullable shared access property changed access type",
                    ));
                }
                self.current_frame_mut()?.values.push(
                    EvaluationValue::NullableSharedReferenceAccess {
                        control,
                        payload,
                        writable,
                    },
                );
            }
            mir::NullableSharedReferenceAccessExpression::CollectionIndex {
                positional,
                payload,
                collection,
                index,
                writable,
                remove,
            } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::CollectionIndexSharedAccess {
                        positional,
                        collection,
                        payload,
                        writable,
                        nullable: true,
                        remove,
                    });
                frame.tasks.push(EvaluationTask::Rvalue(*index));
            }
            mir::NullableSharedReferenceAccessExpression::CollectionGet {
                collection,
                key,
                access,
                stored,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::DictionaryGet {
                    collection,
                    expected: stored.into_type(),
                    access,
                });
                frame.tasks.push(EvaluationTask::Rvalue(*key));
            }
            mir::NullableSharedReferenceAccessExpression::Call {
                payload,
                function,
                args,
                writable,
                ..
            } => {
                let ty = if writable {
                    mir::Type::NullableWritableSharedReferenceAccess(payload)
                } else {
                    mir::Type::NullableReadonlySharedReferenceAccess(payload)
                };
                self.queue_call(function, args, ReturnExpectation::Value(ty))?;
            }
            mir::NullableSharedReferenceAccessExpression::NullSafeAcquire {
                payload,
                value,
                writable,
                span,
            } => {
                let drop_receiver = value.owned_temporary();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::FinishNullableSharedAccessAcquire {
                        payload,
                        writable,
                        drop_receiver,
                        span,
                    });
                frame
                    .tasks
                    .push(EvaluationTask::NullableWritableSharedReference(*value));
            }
        }
        Ok(())
    }

    fn expand_collection_expression(
        &mut self,
        expression: mir::CollectionExpression,
    ) -> Result<(), InterpreterError> {
        match expression {
            mir::CollectionExpression::Local {
                collection,
                local,
                transfer,
            } => {
                let value = if transfer {
                    self.current_frame_mut()?
                        .locals
                        .get_mut(local.0)
                        .and_then(Option::take)
                        .ok_or_else(|| {
                            InterpreterError::new(format!(
                                "MIR collection local local{} was moved before use",
                                local.0
                            ))
                        })?
                } else {
                    read_local(&self.current_frame()?.locals, local)?.clone()
                };
                let LocalValue::Collection(value) = value else {
                    return Err(InterpreterError::new(
                        "MIR collection expression used another local type",
                    ));
                };
                if value.ty != collection {
                    return Err(InterpreterError::new(
                        "MIR collection expression has another collection type",
                    ));
                }
                let value = value.assume_non_null()?;
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Collection(value));
            }
            mir::CollectionExpression::Literal {
                collection,
                entries,
            } => {
                let keyed = entries
                    .iter()
                    .map(|entry| entry.key.is_some())
                    .collect::<Vec<_>>();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::BuildCollection { collection, keyed });
                for entry in entries.into_iter().rev() {
                    frame.tasks.push(EvaluationTask::Rvalue(entry.value));
                    if let Some(key) = entry.key {
                        frame.tasks.push(EvaluationTask::Rvalue(key));
                    }
                }
            }
            mir::CollectionExpression::Fill {
                collection,
                value,
                count,
                count_span,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::BuildCollectionFill {
                    collection,
                    count_span,
                });
                frame
                    .tasks
                    .push(EvaluationTask::Value(mir::ValueExpression::Integer(*count)));
                frame.tasks.push(EvaluationTask::Rvalue(*value));
            }
            mir::CollectionExpression::Index {
                source,
                index,
                index_span,
                transfer,
                positional,
                ..
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::LoadCollectionValue {
                    positional,
                    collection: source,
                    index_span,
                    transfer,
                });
                frame.tasks.push(EvaluationTask::Rvalue(*index));
            }
            mir::CollectionExpression::Property {
                collection,
                object,
                property,
            } => {
                let LocalValue::Collection(value) = self.read_property(object, property)? else {
                    return Err(InterpreterError::new(
                        "MIR collection property contains another value type",
                    ));
                };
                if value.ty != collection {
                    return Err(InterpreterError::new(
                        "MIR collection property has another collection type",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Collection(value));
            }
            mir::CollectionExpression::SharedAccessPayload {
                collection, access, ..
            } => {
                let value = self.shared_access_payload(access)?;
                let LocalValue::Collection(value) = value else {
                    return Err(InterpreterError::new(
                        "MIR shared access payload is not a collection",
                    ));
                };
                if value.ty != collection {
                    return Err(InterpreterError::new(
                        "MIR shared access payload has another collection type",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Collection(value));
            }
            mir::CollectionExpression::From {
                collection,
                source,
                transfer,
                algebra,
            } => {
                if let Some((op, right)) = algebra {
                    let left = self.collection_local(source)?.clone();
                    let right = self.collection_local(right)?.clone();
                    let value_type = self.program.collection_types[left.ty.0].value;
                    let left_entries = left.entries();
                    let right_entries = right.entries();
                    let mut entries = Vec::new();
                    for (_, value) in left_entries.iter() {
                        let include = match op {
                            mir::SetAlgebraOp::Union => true,
                            mir::SetAlgebraOp::Intersect => {
                                right_entries.iter().any(|(_, candidate)| {
                                    collection_values_equal(value_type, candidate, value)
                                })
                            }
                            mir::SetAlgebraOp::Difference => {
                                !right_entries.iter().any(|(_, candidate)| {
                                    collection_values_equal(value_type, candidate, value)
                                })
                            }
                        };
                        if include {
                            entries.push((None, value.clone()));
                        }
                    }
                    if op == mir::SetAlgebraOp::Union {
                        for (_, value) in right_entries.iter() {
                            if !entries.iter().any(
                                |(_, candidate): &(Option<LocalValue>, LocalValue)| {
                                    collection_values_equal(value_type, candidate, value)
                                },
                            ) {
                                entries.push((None, value.clone()));
                            }
                        }
                    }
                    let definition = &self.program.collection_types[collection.0];
                    order_collection_entries(definition, &mut entries)?;
                    self.current_frame_mut()?
                        .values
                        .push(EvaluationValue::Collection(CollectionValue::new(
                            collection, entries,
                        )));
                    return Ok(());
                }
                let source = if transfer {
                    self.current_frame_mut()?
                        .locals
                        .get_mut(source.0)
                        .and_then(Option::take)
                        .ok_or_else(|| {
                            InterpreterError::new(
                                "collection conversion source was moved before use",
                            )
                        })?
                } else {
                    read_local(&self.current_frame()?.locals, source)?.clone()
                };
                let LocalValue::Collection(source) = source else {
                    return Err(InterpreterError::new(
                        "collection conversion source is not a collection",
                    ));
                };
                let definition = self.program.collection_types[collection.0].clone();
                let mut entries = Vec::new();
                let mut drops = Vec::new();
                let source_entries = source.entries().clone();
                for (key, value) in source_entries {
                    if matches!(
                        definition.kind,
                        mir::CollectionKind::Set | mir::CollectionKind::SortedSet
                    ) && entries
                        .iter()
                        .any(|(_, current): &(Option<LocalValue>, LocalValue)| {
                            collection_values_equal(definition.value, current, &value)
                        })
                    {
                        if let Some(key) = key {
                            collect_owned_objects_from_value(key, &mut drops);
                        }
                        collect_owned_objects_from_value(value, &mut drops);
                    } else {
                        entries.push((key, value));
                    }
                }
                for drop in drops {
                    self.push_owned_drop_task(drop)?;
                }
                order_collection_entries(&definition, &mut entries)?;
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Collection(CollectionValue::new(
                        collection, entries,
                    )));
            }
            mir::CollectionExpression::FromBytes { collection, source }
            | mir::CollectionExpression::BytesFromArray { collection, source } => {
                let entries = self.collection_local(source)?.entries().clone();
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Collection(CollectionValue::new(
                        collection, entries,
                    )));
            }
            mir::CollectionExpression::ReadFileBytes {
                collection,
                path,
                path_span,
            } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::ReadFileBytes(collection, path_span));
                frame.tasks.push(EvaluationTask::String(*path));
            }
            mir::CollectionExpression::ReadStdinBytes { collection } => {
                let remaining = self.stdin[self.stdin_cursor..].to_vec();
                self.stdin_cursor = self.stdin.len();
                self.push_byte_collection(collection, &remaining)?;
            }
            mir::CollectionExpression::StringIntrinsic(call) => {
                self.queue_string_intrinsic(*call)?;
            }
            mir::CollectionExpression::Call {
                collection,
                function,
                args,
                ..
            } => {
                self.queue_call(
                    function,
                    args,
                    ReturnExpectation::Value(mir::Type::Collection(collection)),
                )?;
            }
        }
        Ok(())
    }

    fn expand_nullable_collection_expression(
        &mut self,
        expression: mir::NullableCollectionExpression,
    ) -> Result<(), InterpreterError> {
        let collection = expression.collection();
        match expression {
            mir::NullableCollectionExpression::Null(_) => {
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Collection(CollectionValue::nullable(
                        collection, None,
                    )));
            }
            mir::NullableCollectionExpression::Collection(value) => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::BuildNullableCollectionSome(collection));
                frame.tasks.push(EvaluationTask::Collection(value));
            }
            mir::NullableCollectionExpression::Local {
                local, transfer, ..
            } => {
                let value = if transfer {
                    self.current_frame_mut()?
                        .locals
                        .get_mut(local.0)
                        .and_then(Option::take)
                        .ok_or_else(|| {
                            InterpreterError::new(format!(
                                "MIR nullable collection local local{} was moved before use",
                                local.0
                            ))
                        })?
                } else {
                    read_local(&self.current_frame()?.locals, local)?.clone()
                };
                let LocalValue::Collection(value) = value else {
                    return Err(InterpreterError::new(
                        "MIR nullable collection expression used another local type",
                    ));
                };
                if value.ty != collection || !value.nullable {
                    return Err(InterpreterError::new(
                        "MIR nullable collection expression has another type",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Collection(value));
            }
            mir::NullableCollectionExpression::Property {
                object, property, ..
            } => {
                let LocalValue::Collection(value) = self.read_property(object, property)? else {
                    return Err(InterpreterError::new(
                        "MIR nullable collection property contains another value type",
                    ));
                };
                if value.ty != collection || !value.nullable {
                    return Err(InterpreterError::new(
                        "MIR nullable collection property has another type",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Collection(value));
            }
            mir::NullableCollectionExpression::Call { function, args, .. } => {
                self.queue_call(
                    function,
                    args,
                    ReturnExpectation::Value(mir::Type::NullableCollection(collection)),
                )?;
            }
            mir::NullableCollectionExpression::Coalesce {
                left,
                right,
                transfer,
                ..
            } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::FinishNullableCollectionCoalesce {
                        right: *right,
                        transfer,
                    });
                frame.tasks.push(EvaluationTask::NullableCollection(*left));
            }
        }
        Ok(())
    }

    fn expand_nullable_class_expression(
        &mut self,
        expression: mir::NullableClassExpression,
    ) -> Result<(), InterpreterError> {
        let class = expression.class();
        match expression {
            mir::NullableClassExpression::Null(_) => self.push_nullable_class(class, None)?,
            mir::NullableClassExpression::Class(value) => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::BuildNullableClassSome(class));
                frame.tasks.push(EvaluationTask::Class(value));
            }
            mir::NullableClassExpression::SharedPayload { reference, .. } => {
                let drop_receiver = reference.owned_temporary().is_some();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::FinishNullableSharedPayload(
                        class,
                        drop_receiver,
                    ));
                frame
                    .tasks
                    .push(EvaluationTask::NullableSharedReference(*reference));
            }
            mir::NullableClassExpression::Local {
                local, transfer, ..
            } => {
                let value = if transfer {
                    self.current_frame_mut()?
                        .locals
                        .get_mut(local.0)
                        .and_then(Option::take)
                        .ok_or_else(|| {
                            InterpreterError::new("nullable class was moved before use")
                        })?
                } else {
                    read_local(&self.current_frame()?.locals, local)?.clone()
                };
                let LocalValue::NullableClass {
                    object,
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "nullable class local has another type",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "nullable class local has another class",
                    ));
                }
                self.push_nullable_class(class, object)?;
            }
            mir::NullableClassExpression::Property {
                object, property, ..
            } => {
                let LocalValue::NullableClass {
                    object,
                    class: actual,
                } = self.read_property(object, property)?
                else {
                    return Err(InterpreterError::new(
                        "nullable class property has another type",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "nullable class property has another class",
                    ));
                }
                self.push_nullable_class(class, object)?;
            }
            mir::NullableClassExpression::Call { function, args, .. } => {
                self.queue_call(
                    function,
                    args,
                    ReturnExpectation::Value(mir::Type::NullableClass(class)),
                )?;
            }
            mir::NullableClassExpression::NullSafeProperty {
                object, property, ..
            } => {
                let owned_receiver = object.owned_temporary_class();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AfterNullSafeProperty {
                    property,
                    result: mir::Type::NullableClass(class),
                    owned_receiver,
                });
                frame.tasks.push(EvaluationTask::NullableClass(*object));
            }
            mir::NullableClassExpression::NullSafeCall {
                object,
                function,
                args,
                ..
            } => {
                let owned_receiver = object.owned_temporary_class();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AfterNullSafeCall {
                    function,
                    args,
                    result: mir::Type::NullableClass(class),
                    owned_receiver,
                });
                frame.tasks.push(EvaluationTask::NullableClass(*object));
            }
            mir::NullableClassExpression::Coalesce {
                left,
                right,
                transfer,
                ..
            } => {
                let left_owned = left.owned_temporary_class().is_some();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::AfterNullableClassCoalesce {
                        right: *right,
                        left_owned,
                        transfer,
                    });
                frame.tasks.push(EvaluationTask::NullableClass(*left));
            }
            mir::NullableClassExpression::DictionaryGet {
                class,
                collection,
                key,
                access,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::DictionaryGet {
                    collection,
                    expected: mir::Type::Class(class),
                    access,
                });
                frame.tasks.push(EvaluationTask::Rvalue(*key));
            }
        }
        Ok(())
    }

    fn queue_value_assignment(
        &mut self,
        target: mir::LocalId,
        value: mir::ValueExpression,
    ) -> Result<(), InterpreterError> {
        let frame = self.current_frame_mut()?;
        frame.tasks.push(EvaluationTask::Assign(target));
        frame.tasks.push(EvaluationTask::Value(value));
        Ok(())
    }

    fn queue_call(
        &mut self,
        function: mir::FunctionId,
        args: Vec<mir::Rvalue>,
        expectation: ReturnExpectation,
    ) -> Result<(), InterpreterError> {
        self.queue_call_at(function, args, expectation, None)
    }

    fn queue_call_at(
        &mut self,
        function: mir::FunctionId,
        args: Vec<mir::Rvalue>,
        expectation: ReturnExpectation,
        call_site: Option<Span>,
    ) -> Result<(), InterpreterError> {
        let callee = function_in(self.program, function)?;
        let temporary_arg_drops = temporary_argument_drop_order(&args, callee, 0, |_| false)?;
        let frame = self.current_frame_mut()?;
        frame.tasks.push(EvaluationTask::Invoke {
            function,
            argument_count: args.len(),
            expectation,
            temporary_arg_drops,
            call_site,
        });
        for argument in args.into_iter().rev() {
            frame.tasks.push(EvaluationTask::Rvalue(argument));
        }
        Ok(())
    }

    fn queue_null_safe_call(
        &mut self,
        object: usize,
        class: crate::class_layout::ClassId,
        function: mir::FunctionId,
        args: Vec<mir::Rvalue>,
        nullable_result: mir::Type,
    ) -> Result<(), InterpreterError> {
        let callee = function_in(self.program, function)?;
        let non_nullable_result = non_nullable_type(nullable_result)
            .ok_or_else(|| InterpreterError::new("null-safe call result is not nullable"))?;
        let mir::ReturnType::Value(result) = callee.return_type else {
            return Err(InterpreterError::new(
                "null-safe value call targeted a void method",
            ));
        };
        if result != non_nullable_result && result != nullable_result {
            return Err(InterpreterError::new(
                "null-safe call result does not match the requested nullable type",
            ));
        }
        let temporary_arg_drops = temporary_argument_drop_order(&args, callee, 1, |_| false)?
            .into_iter()
            .map(|index| index + 1)
            .collect();
        let frame = self.current_frame_mut()?;
        frame.values.push(EvaluationValue::Class { object, class });
        if result == non_nullable_result {
            frame
                .tasks
                .push(EvaluationTask::WrapNullable(nullable_result));
        }
        frame.tasks.push(EvaluationTask::Invoke {
            function,
            argument_count: args.len() + 1,
            expectation: ReturnExpectation::Value(result),
            temporary_arg_drops,
            call_site: None,
        });
        for argument in args.into_iter().rev() {
            frame.tasks.push(EvaluationTask::Rvalue(argument));
        }
        Ok(())
    }

    fn queue_null_safe_statement_call(
        &mut self,
        object: usize,
        class: crate::class_layout::ClassId,
        function: mir::FunctionId,
        args: Vec<mir::Rvalue>,
        call_site: Option<Span>,
    ) -> Result<(), InterpreterError> {
        let callee = function_in(self.program, function)?;
        let expectation = match callee.return_type {
            mir::ReturnType::Void => ReturnExpectation::Void,
            mir::ReturnType::Value(ty) => ReturnExpectation::Discard(ty),
        };
        let temporary_arg_drops = temporary_argument_drop_order(&args, callee, 1, |_| false)?
            .into_iter()
            .map(|index| index + 1)
            .collect();
        let frame = self.current_frame_mut()?;
        frame.values.push(EvaluationValue::Class { object, class });
        frame.tasks.push(EvaluationTask::Invoke {
            function,
            argument_count: args.len() + 1,
            expectation,
            temporary_arg_drops,
            call_site,
        });
        for argument in args.into_iter().rev() {
            frame.tasks.push(EvaluationTask::Rvalue(argument));
        }
        Ok(())
    }

    fn push_frame(
        &mut self,
        function_id: mir::FunctionId,
        args: &[LocalValue],
        caller_expectation: Option<ReturnExpectation>,
        entered_from: Option<Span>,
    ) -> Result<(), InterpreterError> {
        if let Some(limit) = self.limits.max_call_frames {
            if self.frames.len() >= limit {
                return Err(InterpreterError::new(format!(
                    "MIR interpreter reached the explicit test call-frame limit of {limit}"
                )));
            }
        }

        let function = function_in(self.program, function_id)?;
        if args.len() != function.params.len() {
            return Err(InterpreterError::new(format!(
                "MIR function {} expected {} argument(s), got {}",
                function.name,
                function.params.len(),
                args.len()
            )));
        }
        let mut locals = vec![None; function.locals.len()];
        for (index, local) in function.locals.iter().enumerate() {
            if local.id != mir::LocalId(index) {
                return Err(InterpreterError::new(format!(
                    "MIR function {} local slot {index} contains local{}",
                    function.name, local.id.0
                )));
            }
        }
        for (parameter, value) in function.params.iter().zip(args.iter().cloned()) {
            let definition = local_in(function, *parameter)?;
            if local_value_type(&value) != definition.ty {
                return Err(InterpreterError::new(format!(
                    "MIR function {} parameter local{} expects {}, got {}",
                    function.name,
                    parameter.0,
                    definition.ty,
                    local_value_type(&value)
                )));
            }
            let _ = assign_local(&function.locals, &mut locals, *parameter, value)?;
        }
        block_in(function, function.entry_block)?;
        self.frames.push(CallFrame {
            function: function_id,
            block: function.entry_block,
            statement_index: 0,
            entered_block: false,
            locals,
            tasks: Vec::new(),
            values: Vec::new(),
            statement_temporary_drops: Vec::new(),
            caller_expectation,
            entered_from,
        });
        Ok(())
    }

    fn complete_frame(
        &mut self,
        outcome: FunctionOutcome,
    ) -> Result<StepOutcome, InterpreterError> {
        let frame = self
            .frames
            .pop()
            .ok_or_else(|| InterpreterError::new("MIR interpreter has no call frame to return"))?;
        let Some(expectation) = frame.caller_expectation else {
            return Ok(StepOutcome::EntryReturned(outcome));
        };
        match (expectation, outcome) {
            (ReturnExpectation::Value(expected), FunctionOutcome::Value(value)) => {
                if local_value_type(&value) != expected {
                    return Err(InterpreterError::new(format!(
                        "MIR scalar call expected {expected}, returned {}",
                        local_value_type(&value)
                    )));
                }
                self.push_local_value(value)?;
            }
            (ReturnExpectation::Discard(expected), FunctionOutcome::Value(value)) => {
                if local_value_type(&value) != expected {
                    return Err(InterpreterError::new(format!(
                        "MIR discarded call expected {expected}, returned {}",
                        local_value_type(&value)
                    )));
                }
            }
            (ReturnExpectation::Void, FunctionOutcome::Void) => {}
            (
                ReturnExpectation::Value(_) | ReturnExpectation::Discard(_),
                FunctionOutcome::Void,
            ) => {
                return Err(InterpreterError::new(
                    "MIR scalar call returned a void value",
                ));
            }
            (ReturnExpectation::Void, FunctionOutcome::Value(_)) => {
                return Err(InterpreterError::new(
                    "MIR void call returned a scalar value",
                ));
            }
        }
        Ok(StepOutcome::Continue)
    }

    fn move_to_block(
        &mut self,
        function: &mir::Function,
        target: mir::BlockId,
    ) -> Result<(), InterpreterError> {
        block_in(function, target)?;
        let frame = self.current_frame_mut()?;
        frame.block = target;
        frame.statement_index = 0;
        frame.entered_block = false;
        Ok(())
    }

    fn take_call_arguments(&mut self, count: usize) -> Result<Vec<LocalValue>, InterpreterError> {
        let frame = self.current_frame_mut()?;
        if frame.values.len() < count {
            return Err(InterpreterError::new(
                "MIR call argument evaluation produced too few values",
            ));
        }
        let start = frame.values.len() - count;
        frame
            .values
            .drain(start..)
            .map(|value| match value {
                EvaluationValue::Scalar(value) => Ok(LocalValue::Scalar(value)),
                EvaluationValue::String(value) => Ok(LocalValue::String(value)),
                EvaluationValue::Mixed(value) => Ok(LocalValue::Mixed(value)),
                EvaluationValue::NullableScalar { ty, value } => {
                    Ok(LocalValue::NullableScalar { ty, value })
                }
                EvaluationValue::NullableString(value) => Ok(LocalValue::NullableString(value)),
                EvaluationValue::NullableMixed(value) => Ok(LocalValue::NullableMixed(value)),
                EvaluationValue::Class { object, class } => Ok(LocalValue::Class { object, class }),
                EvaluationValue::NullableClass { object, class } => {
                    Ok(LocalValue::NullableClass { object, class })
                }
                EvaluationValue::SharedReference { control, class } => {
                    Ok(LocalValue::SharedReference { control, class })
                }
                EvaluationValue::WeakReference { control, class } => {
                    Ok(LocalValue::WeakReference { control, class })
                }
                EvaluationValue::NullableSharedReference { control, class } => {
                    Ok(LocalValue::NullableSharedReference { control, class })
                }
                EvaluationValue::NullableWeakReference { control, class } => {
                    Ok(LocalValue::NullableWeakReference { control, class })
                }
                EvaluationValue::WritableSharedReference { control, payload } => {
                    Ok(LocalValue::WritableSharedReference { control, payload })
                }
                EvaluationValue::WritableWeakReference { control, payload } => {
                    Ok(LocalValue::WritableWeakReference { control, payload })
                }
                EvaluationValue::NullableWritableSharedReference { control, payload } => {
                    Ok(LocalValue::NullableWritableSharedReference { control, payload })
                }
                EvaluationValue::NullableWritableWeakReference { control, payload } => {
                    Ok(LocalValue::NullableWritableWeakReference { control, payload })
                }
                EvaluationValue::SharedReferenceAccess {
                    control,
                    payload,
                    writable,
                } => Ok(LocalValue::SharedReferenceAccess {
                    control,
                    payload,
                    writable,
                }),
                EvaluationValue::NullableSharedReferenceAccess {
                    control,
                    payload,
                    writable,
                } => Ok(LocalValue::NullableSharedReferenceAccess {
                    control,
                    payload,
                    writable,
                }),
                EvaluationValue::Collection(value) => Ok(LocalValue::Collection(value)),
            })
            .collect()
    }

    fn push_scalar(&mut self, value: mir::ScalarValue) -> Result<(), InterpreterError> {
        self.current_frame_mut()?
            .values
            .push(EvaluationValue::Scalar(value));
        Ok(())
    }

    fn pop_scalar(&mut self) -> Result<mir::ScalarValue, InterpreterError> {
        match self.current_frame_mut()?.values.pop() {
            Some(EvaluationValue::Scalar(value)) => Ok(value),
            Some(EvaluationValue::String(_)) => Err(InterpreterError::new(
                "MIR scalar evaluation produced a string",
            )),
            Some(EvaluationValue::Mixed(_)) | Some(EvaluationValue::NullableMixed(_)) => Err(
                InterpreterError::new("MIR scalar evaluation produced a mixed value"),
            ),
            Some(EvaluationValue::NullableString(_)) => Err(InterpreterError::new(
                "MIR scalar evaluation produced a nullable string",
            )),
            Some(EvaluationValue::NullableScalar { .. })
            | Some(EvaluationValue::NullableClass { .. }) => Err(InterpreterError::new(
                "MIR scalar evaluation produced a nullable value",
            )),
            Some(EvaluationValue::Class { .. }) => Err(InterpreterError::new(
                "MIR scalar evaluation produced a class",
            )),
            Some(EvaluationValue::SharedReference { .. })
            | Some(EvaluationValue::WeakReference { .. })
            | Some(EvaluationValue::NullableSharedReference { .. })
            | Some(EvaluationValue::NullableWeakReference { .. })
            | Some(EvaluationValue::WritableSharedReference { .. })
            | Some(EvaluationValue::WritableWeakReference { .. })
            | Some(EvaluationValue::NullableWritableSharedReference { .. })
            | Some(EvaluationValue::NullableWritableWeakReference { .. })
            | Some(EvaluationValue::SharedReferenceAccess { .. })
            | Some(EvaluationValue::NullableSharedReferenceAccess { .. }) => Err(
                InterpreterError::new("MIR scalar evaluation produced a shared handle"),
            ),
            Some(EvaluationValue::Collection(_)) => Err(InterpreterError::new(
                "MIR scalar evaluation produced a collection",
            )),
            None => Err(InterpreterError::new(
                "MIR scalar evaluation produced no value",
            )),
        }
    }

    fn push_string(&mut self, value: impl Into<SharedString>) -> Result<(), InterpreterError> {
        self.current_frame_mut()?
            .values
            .push(EvaluationValue::String(value.into()));
        Ok(())
    }

    fn pop_string(&mut self) -> Result<SharedString, InterpreterError> {
        match self.current_frame_mut()?.values.pop() {
            Some(EvaluationValue::String(value)) => Ok(value),
            Some(EvaluationValue::Scalar(_)) => Err(InterpreterError::new(
                "MIR string evaluation produced a scalar",
            )),
            Some(EvaluationValue::Mixed(_)) | Some(EvaluationValue::NullableMixed(_)) => Err(
                InterpreterError::new("MIR string evaluation produced a mixed value"),
            ),
            Some(EvaluationValue::NullableString(_)) => Err(InterpreterError::new(
                "MIR string evaluation produced a nullable string",
            )),
            Some(EvaluationValue::NullableScalar { .. })
            | Some(EvaluationValue::NullableClass { .. }) => Err(InterpreterError::new(
                "MIR string evaluation produced a nullable value",
            )),
            Some(EvaluationValue::Class { .. }) => Err(InterpreterError::new(
                "MIR string evaluation produced a class",
            )),
            Some(EvaluationValue::SharedReference { .. })
            | Some(EvaluationValue::WeakReference { .. })
            | Some(EvaluationValue::NullableSharedReference { .. })
            | Some(EvaluationValue::NullableWeakReference { .. })
            | Some(EvaluationValue::WritableSharedReference { .. })
            | Some(EvaluationValue::WritableWeakReference { .. })
            | Some(EvaluationValue::NullableWritableSharedReference { .. })
            | Some(EvaluationValue::NullableWritableWeakReference { .. })
            | Some(EvaluationValue::SharedReferenceAccess { .. })
            | Some(EvaluationValue::NullableSharedReferenceAccess { .. }) => Err(
                InterpreterError::new("MIR string evaluation produced a shared handle"),
            ),
            Some(EvaluationValue::Collection(_)) => Err(InterpreterError::new(
                "MIR string evaluation produced a collection",
            )),
            None => Err(InterpreterError::new(
                "MIR string evaluation produced no value",
            )),
        }
    }

    fn push_nullable_string(
        &mut self,
        value: Option<SharedString>,
    ) -> Result<(), InterpreterError> {
        self.current_frame_mut()?
            .values
            .push(EvaluationValue::NullableString(value));
        Ok(())
    }

    fn push_mixed(&mut self, value: MixedValue) -> Result<(), InterpreterError> {
        self.current_frame_mut()?
            .values
            .push(EvaluationValue::Mixed(value));
        Ok(())
    }

    fn push_nullable_mixed(&mut self, value: Option<MixedValue>) -> Result<(), InterpreterError> {
        self.current_frame_mut()?
            .values
            .push(EvaluationValue::NullableMixed(value));
        Ok(())
    }

    fn pop_nullable_mixed(&mut self) -> Result<Option<MixedValue>, InterpreterError> {
        match self.current_frame_mut()?.values.pop() {
            Some(EvaluationValue::NullableMixed(value)) => Ok(value),
            Some(_) => Err(InterpreterError::new(
                "MIR nullable-mixed evaluation produced another value type",
            )),
            None => Err(InterpreterError::new(
                "MIR nullable-mixed evaluation produced no value",
            )),
        }
    }

    fn pop_nullable_string(&mut self) -> Result<Option<SharedString>, InterpreterError> {
        match self.current_frame_mut()?.values.pop() {
            Some(EvaluationValue::NullableString(value)) => Ok(value),
            Some(_) => Err(InterpreterError::new(
                "MIR nullable-string evaluation produced another value type",
            )),
            None => Err(InterpreterError::new(
                "MIR nullable-string evaluation produced no value",
            )),
        }
    }

    fn push_nullable_scalar(
        &mut self,
        ty: mir::ScalarType,
        value: Option<mir::ScalarValue>,
    ) -> Result<(), InterpreterError> {
        if value.is_some_and(|value| value.ty() != ty) {
            return Err(InterpreterError::new(
                "nullable scalar payload type mismatch",
            ));
        }
        self.current_frame_mut()?
            .values
            .push(EvaluationValue::NullableScalar { ty, value });
        Ok(())
    }

    fn pop_nullable_scalar(
        &mut self,
    ) -> Result<(mir::ScalarType, Option<mir::ScalarValue>), InterpreterError> {
        match self.current_frame_mut()?.values.pop() {
            Some(EvaluationValue::NullableScalar { ty, value }) => Ok((ty, value)),
            Some(_) => Err(InterpreterError::new(
                "nullable scalar produced another value type",
            )),
            None => Err(InterpreterError::new("nullable scalar produced no value")),
        }
    }

    fn push_nullable_class(
        &mut self,
        class: crate::class_layout::ClassId,
        object: Option<usize>,
    ) -> Result<(), InterpreterError> {
        self.current_frame_mut()?
            .values
            .push(EvaluationValue::NullableClass { object, class });
        Ok(())
    }

    fn pop_nullable_class(
        &mut self,
    ) -> Result<(crate::class_layout::ClassId, Option<usize>), InterpreterError> {
        match self.current_frame_mut()?.values.pop() {
            Some(EvaluationValue::NullableClass { object, class }) => Ok((class, object)),
            Some(_) => Err(InterpreterError::new(
                "nullable class produced another value type",
            )),
            None => Err(InterpreterError::new("nullable class produced no value")),
        }
    }

    fn pop_nullable_shared_reference(
        &mut self,
    ) -> Result<(crate::class_layout::ClassId, Option<SharedControl>), InterpreterError> {
        match self.current_frame_mut()?.values.pop() {
            Some(EvaluationValue::NullableSharedReference { control, class }) => {
                Ok((class, control))
            }
            Some(_) => Err(InterpreterError::new(
                "nullable shared reference produced another value type",
            )),
            None => Err(InterpreterError::new(
                "nullable shared reference produced no value",
            )),
        }
    }

    fn pop_nullable_weak_reference(
        &mut self,
    ) -> Result<(crate::class_layout::ClassId, Option<SharedControl>), InterpreterError> {
        match self.current_frame_mut()?.values.pop() {
            Some(EvaluationValue::NullableWeakReference { control, class }) => Ok((class, control)),
            Some(_) => Err(InterpreterError::new(
                "nullable weak reference produced another value type",
            )),
            None => Err(InterpreterError::new(
                "nullable weak reference produced no value",
            )),
        }
    }

    fn pop_writable_shared_reference(
        &mut self,
    ) -> Result<(WritableSharedControl, mir::WritableSharedPayload), InterpreterError> {
        match self.current_frame_mut()?.values.pop() {
            Some(EvaluationValue::WritableSharedReference { control, payload }) => {
                Ok((control, payload))
            }
            Some(_) => Err(InterpreterError::new(
                "writable shared reference produced another value type",
            )),
            None => Err(InterpreterError::new(
                "writable shared reference produced no value",
            )),
        }
    }

    fn pop_writable_weak_reference(
        &mut self,
    ) -> Result<(WritableSharedControl, mir::WritableSharedPayload), InterpreterError> {
        match self.current_frame_mut()?.values.pop() {
            Some(EvaluationValue::WritableWeakReference { control, payload }) => {
                Ok((control, payload))
            }
            Some(_) => Err(InterpreterError::new(
                "writable weak reference produced another value type",
            )),
            None => Err(InterpreterError::new(
                "writable weak reference produced no value",
            )),
        }
    }

    fn pop_nullable_writable_shared_reference(
        &mut self,
    ) -> Result<(Option<WritableSharedControl>, mir::WritableSharedPayload), InterpreterError> {
        match self.current_frame_mut()?.values.pop() {
            Some(EvaluationValue::NullableWritableSharedReference { control, payload }) => {
                Ok((control, payload))
            }
            Some(_) => Err(InterpreterError::new(
                "nullable writable shared reference produced another value type",
            )),
            None => Err(InterpreterError::new(
                "nullable writable shared reference produced no value",
            )),
        }
    }

    fn pop_nullable_writable_weak_reference(
        &mut self,
    ) -> Result<(Option<WritableSharedControl>, mir::WritableSharedPayload), InterpreterError> {
        match self.current_frame_mut()?.values.pop() {
            Some(EvaluationValue::NullableWritableWeakReference { control, payload }) => {
                Ok((control, payload))
            }
            Some(_) => Err(InterpreterError::new(
                "nullable writable weak reference produced another value type",
            )),
            None => Err(InterpreterError::new(
                "nullable writable weak reference produced no value",
            )),
        }
    }

    fn push_null(&mut self, ty: mir::Type) -> Result<(), InterpreterError> {
        match ty {
            mir::Type::NullableScalar(ty) => self.push_nullable_scalar(ty, None),
            mir::Type::NullableString => self.push_nullable_string(None),
            mir::Type::NullableMixed => self.push_nullable_mixed(None),
            mir::Type::NullableClass(class) => self.push_nullable_class(class, None),
            mir::Type::NullableSharedReference(class) => {
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::NullableSharedReference {
                        control: None,
                        class,
                    });
                Ok(())
            }
            mir::Type::NullableWeakReference(class) => {
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::NullableWeakReference {
                        control: None,
                        class,
                    });
                Ok(())
            }
            mir::Type::NullableWritableSharedReference(payload) => {
                self.current_frame_mut()?.values.push(
                    EvaluationValue::NullableWritableSharedReference {
                        control: None,
                        payload,
                    },
                );
                Ok(())
            }
            mir::Type::NullableWritableWeakReference(payload) => {
                self.current_frame_mut()?.values.push(
                    EvaluationValue::NullableWritableWeakReference {
                        control: None,
                        payload,
                    },
                );
                Ok(())
            }
            mir::Type::NullableReadonlySharedReferenceAccess(payload)
            | mir::Type::NullableWritableSharedReferenceAccess(payload) => {
                self.current_frame_mut()?.values.push(
                    EvaluationValue::NullableSharedReferenceAccess {
                        control: None,
                        payload,
                        writable: matches!(ty, mir::Type::NullableWritableSharedReferenceAccess(_)),
                    },
                );
                Ok(())
            }
            _ => Err(InterpreterError::new(
                "null result does not have nullable type",
            )),
        }
    }

    fn push_nullable_from_value(
        &mut self,
        nullable: mir::Type,
        value: LocalValue,
    ) -> Result<(), InterpreterError> {
        match (nullable, value) {
            (mir::Type::NullableScalar(ty), LocalValue::Scalar(value)) if value.ty() == ty => {
                self.push_nullable_scalar(ty, Some(value))
            }
            (mir::Type::NullableScalar(expected), LocalValue::NullableScalar { ty, value })
                if expected == ty =>
            {
                self.push_nullable_scalar(ty, value)
            }
            (mir::Type::NullableString, LocalValue::String(value)) => {
                self.push_nullable_string(Some(value))
            }
            (mir::Type::NullableString, LocalValue::NullableString(value)) => {
                self.push_nullable_string(value)
            }
            (mir::Type::NullableMixed, LocalValue::Mixed(value)) => {
                self.push_nullable_mixed(Some(value))
            }
            (mir::Type::NullableMixed, LocalValue::NullableMixed(value)) => {
                self.push_nullable_mixed(value)
            }
            (mir::Type::NullableClass(expected), LocalValue::Class { object, class })
                if expected == class =>
            {
                self.push_nullable_class(class, Some(object))
            }
            (mir::Type::NullableClass(expected), LocalValue::NullableClass { object, class })
                if expected == class =>
            {
                self.push_nullable_class(class, object)
            }
            (
                mir::Type::NullableSharedReference(expected),
                LocalValue::SharedReference { control, class },
            ) if expected == class => {
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::NullableSharedReference {
                        control: Some(control),
                        class,
                    });
                Ok(())
            }
            (
                mir::Type::NullableWeakReference(expected),
                LocalValue::WeakReference { control, class },
            ) if expected == class => {
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::NullableWeakReference {
                        control: Some(control),
                        class,
                    });
                Ok(())
            }
            (
                mir::Type::NullableWritableSharedReference(expected),
                LocalValue::WritableSharedReference { control, payload },
            ) if expected == payload => {
                self.current_frame_mut()?.values.push(
                    EvaluationValue::NullableWritableSharedReference {
                        control: Some(control),
                        payload,
                    },
                );
                Ok(())
            }
            (
                mir::Type::NullableWritableWeakReference(expected),
                LocalValue::WritableWeakReference { control, payload },
            ) if expected == payload => {
                self.current_frame_mut()?.values.push(
                    EvaluationValue::NullableWritableWeakReference {
                        control: Some(control),
                        payload,
                    },
                );
                Ok(())
            }
            (
                mir::Type::NullableReadonlySharedReferenceAccess(expected),
                LocalValue::SharedReferenceAccess {
                    control,
                    payload,
                    writable: false,
                },
            )
            | (
                mir::Type::NullableWritableSharedReferenceAccess(expected),
                LocalValue::SharedReferenceAccess {
                    control,
                    payload,
                    writable: true,
                },
            ) if expected == payload => {
                self.current_frame_mut()?.values.push(
                    EvaluationValue::NullableSharedReferenceAccess {
                        control: Some(control),
                        payload,
                        writable: matches!(
                            nullable,
                            mir::Type::NullableWritableSharedReferenceAccess(_)
                        ),
                    },
                );
                Ok(())
            }
            _ => Err(InterpreterError::new(
                "cannot wrap value in requested nullable type",
            )),
        }
    }

    fn take_evaluation_values(
        &mut self,
        count: usize,
    ) -> Result<Vec<EvaluationValue>, InterpreterError> {
        let frame = self.current_frame_mut()?;
        if frame.values.len() < count {
            return Err(InterpreterError::new(
                "MIR format evaluation produced too few values",
            ));
        }
        Ok(frame.values.drain(frame.values.len() - count..).collect())
    }

    fn pop_local_value(&mut self) -> Result<LocalValue, InterpreterError> {
        match self.current_frame_mut()?.values.pop() {
            Some(EvaluationValue::Scalar(value)) => Ok(LocalValue::Scalar(value)),
            Some(EvaluationValue::String(value)) => Ok(LocalValue::String(value)),
            Some(EvaluationValue::Mixed(value)) => Ok(LocalValue::Mixed(value)),
            Some(EvaluationValue::NullableScalar { ty, value }) => {
                Ok(LocalValue::NullableScalar { ty, value })
            }
            Some(EvaluationValue::NullableString(value)) => Ok(LocalValue::NullableString(value)),
            Some(EvaluationValue::NullableMixed(value)) => Ok(LocalValue::NullableMixed(value)),
            Some(EvaluationValue::Class { object, class }) => {
                Ok(LocalValue::Class { object, class })
            }
            Some(EvaluationValue::NullableClass { object, class }) => {
                Ok(LocalValue::NullableClass { object, class })
            }
            Some(EvaluationValue::SharedReference { control, class }) => {
                Ok(LocalValue::SharedReference { control, class })
            }
            Some(EvaluationValue::WeakReference { control, class }) => {
                Ok(LocalValue::WeakReference { control, class })
            }
            Some(EvaluationValue::NullableSharedReference { control, class }) => {
                Ok(LocalValue::NullableSharedReference { control, class })
            }
            Some(EvaluationValue::NullableWeakReference { control, class }) => {
                Ok(LocalValue::NullableWeakReference { control, class })
            }
            Some(EvaluationValue::WritableSharedReference { control, payload }) => {
                Ok(LocalValue::WritableSharedReference { control, payload })
            }
            Some(EvaluationValue::WritableWeakReference { control, payload }) => {
                Ok(LocalValue::WritableWeakReference { control, payload })
            }
            Some(EvaluationValue::NullableWritableSharedReference { control, payload }) => {
                Ok(LocalValue::NullableWritableSharedReference { control, payload })
            }
            Some(EvaluationValue::NullableWritableWeakReference { control, payload }) => {
                Ok(LocalValue::NullableWritableWeakReference { control, payload })
            }
            Some(EvaluationValue::SharedReferenceAccess {
                control,
                payload,
                writable,
            }) => Ok(LocalValue::SharedReferenceAccess {
                control,
                payload,
                writable,
            }),
            Some(EvaluationValue::NullableSharedReferenceAccess {
                control,
                payload,
                writable,
            }) => Ok(LocalValue::NullableSharedReferenceAccess {
                control,
                payload,
                writable,
            }),
            Some(EvaluationValue::Collection(value)) => Ok(LocalValue::Collection(value)),
            None => Err(InterpreterError::new("MIR evaluation produced no value")),
        }
    }

    fn pop_collection_value(&mut self) -> Result<CollectionValue, InterpreterError> {
        match self.current_frame_mut()?.values.pop() {
            Some(EvaluationValue::Collection(value)) => Ok(value),
            Some(_) => Err(InterpreterError::new(
                "MIR collection evaluation produced another value type",
            )),
            None => Err(InterpreterError::new(
                "MIR collection evaluation produced no value",
            )),
        }
    }

    fn push_local_value(&mut self, value: LocalValue) -> Result<(), InterpreterError> {
        let value = match value {
            LocalValue::Scalar(value) => EvaluationValue::Scalar(value),
            LocalValue::String(value) => EvaluationValue::String(value),
            LocalValue::Mixed(value) => EvaluationValue::Mixed(value),
            LocalValue::NullableScalar { ty, value } => {
                EvaluationValue::NullableScalar { ty, value }
            }
            LocalValue::NullableString(value) => EvaluationValue::NullableString(value),
            LocalValue::NullableMixed(value) => EvaluationValue::NullableMixed(value),
            LocalValue::Class { object, class } => EvaluationValue::Class { object, class },
            LocalValue::NullableClass { object, class } => {
                EvaluationValue::NullableClass { object, class }
            }
            LocalValue::SharedReference { control, class } => {
                EvaluationValue::SharedReference { control, class }
            }
            LocalValue::WeakReference { control, class } => {
                EvaluationValue::WeakReference { control, class }
            }
            LocalValue::NullableSharedReference { control, class } => {
                EvaluationValue::NullableSharedReference { control, class }
            }
            LocalValue::NullableWeakReference { control, class } => {
                EvaluationValue::NullableWeakReference { control, class }
            }
            LocalValue::WritableSharedReference { control, payload } => {
                EvaluationValue::WritableSharedReference { control, payload }
            }
            LocalValue::WritableWeakReference { control, payload } => {
                EvaluationValue::WritableWeakReference { control, payload }
            }
            LocalValue::NullableWritableSharedReference { control, payload } => {
                EvaluationValue::NullableWritableSharedReference { control, payload }
            }
            LocalValue::NullableWritableWeakReference { control, payload } => {
                EvaluationValue::NullableWritableWeakReference { control, payload }
            }
            LocalValue::SharedReferenceAccess {
                control,
                payload,
                writable,
            } => EvaluationValue::SharedReferenceAccess {
                control,
                payload,
                writable,
            },
            LocalValue::NullableSharedReferenceAccess {
                control,
                payload,
                writable,
            } => EvaluationValue::NullableSharedReferenceAccess {
                control,
                payload,
                writable,
            },
            LocalValue::Collection(value) => EvaluationValue::Collection(value),
        };
        self.current_frame_mut()?.values.push(value);
        Ok(())
    }

    fn queue_collection_scalar_operand(
        &mut self,
        operand: &mir::Operand,
    ) -> Result<bool, InterpreterError> {
        match operand {
            mir::Operand::CollectionLength(collection) => {
                self.current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::CollectionLength(*collection));
                Ok(true)
            }
            mir::Operand::CollectionIndex {
                positional,
                collection,
                index,
                remove,
            } => {
                let frame = self.current_frame_mut()?;
                if *remove {
                    frame.tasks.push(EvaluationTask::LoadCollectionValue {
                        positional: *positional,
                        collection: *collection,
                        index_span: Span::default(),
                        transfer: true,
                    });
                } else {
                    frame.tasks.push(EvaluationTask::CollectionIndexScalar(
                        *collection,
                        *positional,
                    ));
                }
                frame.tasks.push(EvaluationTask::Rvalue((**index).clone()));
                Ok(true)
            }
            mir::Operand::CollectionKeyAt { collection, offset } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::CollectionKeyScalar(*collection));
                frame.tasks.push(EvaluationTask::Rvalue((**offset).clone()));
                Ok(true)
            }
            mir::Operand::StringIntrinsic(call) => {
                self.queue_string_intrinsic((**call).clone())?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn queue_string_intrinsic(
        &mut self,
        mut call: mir::StringIntrinsicCall,
    ) -> Result<(), InterpreterError> {
        let arguments = std::mem::take(&mut call.args);
        let frame = self.current_frame_mut()?;
        frame.tasks.push(EvaluationTask::StringIntrinsic {
            kind: call.kind,
            result: call.result,
            argument_count: arguments.len(),
            span: call.span,
            argument_spans: call.argument_spans,
        });
        for argument in arguments.into_iter().rev() {
            frame.tasks.push(EvaluationTask::Rvalue(argument));
        }
        Ok(())
    }

    fn execute_string_intrinsic(
        &mut self,
        kind: mir::StringIntrinsicKind,
        result: mir::Type,
        arguments: Vec<LocalValue>,
        span: Span,
        argument_spans: &[Span],
    ) -> Result<Option<RuntimePanicEvent>, InterpreterError> {
        use mir::StringIntrinsicKind as Kind;

        let panic = |error: StringError| {
            let facts = match error {
                StringError::SliceLengthNegative => vec![RuntimeFact {
                    name: doria_diagnostic_catalogue::STRING_SLICE_LENGTH_FACT.to_string(),
                    value: RuntimeFactValue::Signed(
                        local_nullable_int(&arguments, 2)?.ok_or_else(|| {
                            InterpreterError::new(
                                "negative string slice length is unexpectedly absent",
                            )
                        })?,
                    ),
                }],
                StringError::PaddingLengthNegative => vec![RuntimeFact {
                    name: doria_diagnostic_catalogue::STRING_PADDING_REQUESTED_LENGTH_FACT
                        .to_string(),
                    value: RuntimeFactValue::Signed(local_int(&arguments, 1)?),
                }],
                StringError::RepetitionCountNegative => vec![RuntimeFact {
                    name: doria_diagnostic_catalogue::STRING_REPETITION_COUNT_FACT.to_string(),
                    value: RuntimeFactValue::Signed(local_int(&arguments, 1)?),
                }],
                StringError::PaddingTextEmpty => {
                    return Err(InterpreterError::new(
                        "empty string padding must use the operation-aware panic path",
                    ))
                }
                StringError::ResultTooLarge => Vec::new(),
            };
            Ok(Some(string_panic_event(
                error,
                kind,
                span,
                argument_spans,
                facts,
                None,
            )))
        };
        match kind {
            Kind::GraphemeLength | Kind::ByteLength => {
                let text = local_string(&arguments, 0)?;
                let length = if kind == Kind::GraphemeLength {
                    doria_unicode::grapheme_count(text)
                } else {
                    doria_unicode::byte_length(text)
                };
                let Some(value) = i64::try_from(length).ok().and_then(|value| {
                    IntegerValue::from_i128(IntegerType::Int64, i128::from(value))
                }) else {
                    return panic(StringError::ResultTooLarge);
                };
                self.push_scalar(mir::ScalarValue::Integer(value))?;
            }
            Kind::IsEmpty => {
                self.push_scalar(mir::ScalarValue::Bool(doria_unicode::is_empty(
                    local_string(&arguments, 0)?,
                )))?;
            }
            Kind::ToBytes => {
                let collection = collection_result_id(result)?;
                self.push_byte_collection(collection, local_string(&arguments, 0)?.as_bytes())?;
            }
            Kind::Trim | Kind::TrimStart | Kind::TrimEnd => {
                let text = local_string(&arguments, 0)?;
                let mode = match kind {
                    Kind::Trim => TrimMode::Both,
                    Kind::TrimStart => TrimMode::Start,
                    Kind::TrimEnd => TrimMode::End,
                    _ => unreachable!(),
                };
                self.push_string(text[doria_unicode::trim_range(text, mode)].to_string())?;
            }
            Kind::Lower | Kind::Upper | Kind::LowerFirst | Kind::UpperFirst => {
                let text = local_string(&arguments, 0)?;
                let mapping = match kind {
                    Kind::Lower | Kind::LowerFirst => CaseMapping::Lower,
                    Kind::Upper | Kind::UpperFirst => CaseMapping::Upper,
                    _ => unreachable!(),
                };
                let first_only = matches!(kind, Kind::LowerFirst | Kind::UpperFirst);
                let length = match if first_only {
                    doria_unicode::first_case_output_length(text, mapping)
                } else {
                    doria_unicode::case_output_length(text, mapping)
                } {
                    Ok(length) => length,
                    Err(error) => return panic(error),
                };
                let mut output = vec![0; length];
                let written = if first_only {
                    doria_unicode::write_first_case(text, mapping, &mut output)
                } else {
                    doria_unicode::write_case(text, mapping, &mut output)
                };
                if let Err(error) = written {
                    return panic(error);
                }
                self.push_string(unsafe { String::from_utf8_unchecked(output) })?;
            }
            Kind::Contains | Kind::StartsWith | Kind::EndsWith => {
                let text = local_string(&arguments, 0)?;
                let needle = local_string(&arguments, 1)?;
                let value = match kind {
                    Kind::Contains => doria_unicode::contains(text, needle),
                    Kind::StartsWith => doria_unicode::starts_with(text, needle),
                    Kind::EndsWith => doria_unicode::ends_with(text, needle),
                    _ => unreachable!(),
                };
                self.push_scalar(mir::ScalarValue::Bool(value))?;
            }
            Kind::ContainsIgnoreCase | Kind::StartsWithIgnoreCase | Kind::EndsWithIgnoreCase => {
                let text = local_string(&arguments, 0)?;
                let needle = local_string(&arguments, 1)?;
                let value = match kind {
                    Kind::ContainsIgnoreCase => doria_unicode::contains_ignore_case(text, needle),
                    Kind::StartsWithIgnoreCase => {
                        doria_unicode::starts_with_ignore_case(text, needle)
                    }
                    Kind::EndsWithIgnoreCase => doria_unicode::ends_with_ignore_case(text, needle),
                    _ => unreachable!(),
                };
                match value {
                    Ok(value) => self.push_scalar(mir::ScalarValue::Bool(value))?,
                    Err(error) => return panic(error),
                }
            }
            Kind::EqualsIgnoreCase => {
                let left = local_string(&arguments, 0)?;
                let right = local_string(&arguments, 1)?;
                let length = match doria_unicode::case_output_length(left, CaseMapping::Fold) {
                    Ok(length) => length,
                    Err(error) => return panic(error),
                };
                let mut scratch = vec![0; length];
                let value = match doria_unicode::equals_ignore_case(left, right, &mut scratch) {
                    Ok(value) => value,
                    Err(error) => return panic(error),
                };
                self.push_scalar(mir::ScalarValue::Bool(value))?;
            }
            Kind::IndexOf
            | Kind::LastIndexOf
            | Kind::IndexOfIgnoreCase
            | Kind::LastIndexOfIgnoreCase => {
                let text = local_string(&arguments, 0)?;
                let needle = local_string(&arguments, 1)?;
                let index = match kind {
                    Kind::IndexOf => Ok(doria_unicode::first_index_of(text, needle)),
                    Kind::LastIndexOf => Ok(doria_unicode::last_index_of(text, needle)),
                    Kind::IndexOfIgnoreCase => {
                        doria_unicode::first_index_of_ignore_case(text, needle)
                    }
                    Kind::LastIndexOfIgnoreCase => {
                        doria_unicode::last_index_of_ignore_case(text, needle)
                    }
                    _ => unreachable!(),
                };
                let index = match index {
                    Ok(index) => index,
                    Err(error) => return panic(error),
                };
                let value = match index {
                    Some(index) => {
                        let Ok(index) = i64::try_from(index) else {
                            return panic(StringError::ResultTooLarge);
                        };
                        Some(
                            IntegerValue::from_i128(IntegerType::Int64, i128::from(index))
                                .expect("i64 fits Doria int"),
                        )
                    }
                    None => None,
                };
                self.push_nullable_scalar(
                    mir::ScalarType::Integer(IntegerType::Int64),
                    value.map(mir::ScalarValue::Integer),
                )?;
            }
            Kind::CountOccurrences => {
                let count = match doria_unicode::count_occurrences(
                    local_string(&arguments, 0)?,
                    local_string(&arguments, 1)?,
                ) {
                    Ok(count) => count,
                    Err(error) => return panic(error),
                };
                let Ok(count) = i64::try_from(count) else {
                    return panic(StringError::ResultTooLarge);
                };
                self.push_scalar(mir::ScalarValue::Integer(
                    IntegerValue::from_i128(IntegerType::Int64, i128::from(count))
                        .expect("i64 fits Doria int"),
                ))?;
            }
            Kind::Replace => {
                let text = local_string(&arguments, 0)?;
                let search = local_string(&arguments, 1)?;
                let replacement = local_string(&arguments, 2)?;
                let length =
                    match doria_unicode::replacement_output_length(text, search, replacement) {
                        Ok(length) => length,
                        Err(error) => return panic(error),
                    };
                let mut output = vec![0; length];
                if let Err(error) =
                    doria_unicode::write_replacement(text, search, replacement, &mut output)
                {
                    return panic(error);
                }
                self.push_string(unsafe { String::from_utf8_unchecked(output) })?;
            }
            Kind::Split => {
                let text = local_string(&arguments, 0)?;
                let separator = local_string(&arguments, 1)?;
                if let Err(error) = doria_unicode::split_field_count(text, separator) {
                    return panic(error);
                }
                let mut entries = Vec::new();
                doria_unicode::split_fields(text, separator, |field| {
                    entries.push((None, LocalValue::String(SharedString::from(field))));
                });
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Collection(CollectionValue::new(
                        collection_result_id(result)?,
                        entries,
                    )));
            }
            Kind::Join => {
                let separator = local_string(&arguments, 0)?;
                let values = local_collection(&arguments, 1)?;
                let entries = values.entries();
                let mut length = 0usize;
                for (index, (_, value)) in entries.iter().enumerate() {
                    let LocalValue::String(value) = value else {
                        return Err(InterpreterError::new(
                            "String join observed a non-string list element",
                        ));
                    };
                    length = match doria_unicode::checked_add(length, value.len()) {
                        Ok(length) => length,
                        Err(error) => return panic(error),
                    };
                    if index != 0 {
                        length = match doria_unicode::checked_add(length, separator.len()) {
                            Ok(length) => length,
                            Err(error) => return panic(error),
                        };
                    }
                }
                let mut output = String::with_capacity(length);
                for (index, (_, value)) in entries.iter().enumerate() {
                    if index != 0 {
                        output.push_str(separator);
                    }
                    let LocalValue::String(value) = value else {
                        unreachable!("validated List<string>");
                    };
                    output.push_str(value);
                }
                drop(entries);
                self.push_string(output)?;
            }
            Kind::Slice => {
                let text = local_string(&arguments, 0)?;
                let start = local_int(&arguments, 1)?;
                let length = local_nullable_int(&arguments, 2)?;
                let range = match doria_unicode::slice_range(text, start, length) {
                    Ok(range) => range,
                    Err(error) => return panic(error),
                };
                self.push_string(text[range].to_string())?;
            }
            Kind::Repeat => {
                let text = local_string(&arguments, 0)?;
                let count = local_int(&arguments, 1)?;
                let length = match doria_unicode::repetition_output_length(text, count) {
                    Ok(length) => length,
                    Err(error) => return panic(error),
                };
                let mut output = vec![0; length];
                if let Err(error) = doria_unicode::write_repetition(text, count, &mut output) {
                    return panic(error);
                }
                self.push_string(unsafe { String::from_utf8_unchecked(output) })?;
            }
            Kind::PadStart | Kind::PadEnd => {
                let text = local_string(&arguments, 0)?;
                let length = local_int(&arguments, 1)?;
                let padding = local_string(&arguments, 2)?;
                let output_length = match doria_unicode::padding_output_length(
                    text, length, padding,
                ) {
                    Ok(length) => length,
                    Err(StringError::PaddingTextEmpty) => {
                        let current = doria_unicode::grapheme_count(text) as u64;
                        let padding_length = doria_unicode::grapheme_count(padding) as u64;
                        let operation = if kind == Kind::PadStart {
                            "padStart"
                        } else {
                            "padEnd"
                        };
                        let facts = vec![
                            RuntimeFact {
                                name: doria_diagnostic_catalogue::STRING_PADDING_OPERATION_FACT
                                    .to_string(),
                                value: RuntimeFactValue::StaticString(operation.to_string()),
                            },
                            RuntimeFact {
                                name: doria_diagnostic_catalogue::STRING_PADDING_VALUE_FACT
                                    .to_string(),
                                value: RuntimeFactValue::StaticString(text.to_string()),
                            },
                            RuntimeFact {
                                name:
                                    doria_diagnostic_catalogue::STRING_PADDING_CURRENT_LENGTH_FACT
                                        .to_string(),
                                value: RuntimeFactValue::Unsigned(current),
                            },
                            RuntimeFact {
                                name: doria_diagnostic_catalogue::STRING_PADDING_REQUESTED_GRAPHEME_LENGTH_FACT
                                    .to_string(),
                                value: RuntimeFactValue::Signed(length),
                            },
                            RuntimeFact {
                                name: doria_diagnostic_catalogue::STRING_PADDING_PADDING_LENGTH_FACT
                                    .to_string(),
                                value: RuntimeFactValue::Unsigned(padding_length),
                            },
                        ];
                        return Ok(Some(string_panic_event(
                                StringError::PaddingTextEmpty,
                                kind,
                                span,
                                argument_spans,
                                facts,
                                Some(format!(
                                    "`{operation}` was asked to extend `\"{text}\"` from {current} to {length} graphemes,\nbut an empty padding string cannot add any graphemes."
                                )),
                            )));
                    }
                    Err(error) => return panic(error),
                };
                let mut output = vec![0; output_length];
                let side = if kind == Kind::PadStart {
                    PadSide::Start
                } else {
                    PadSide::End
                };
                if let Err(error) =
                    doria_unicode::write_padding(text, length, padding, side, &mut output)
                {
                    return panic(error);
                }
                self.push_string(unsafe { String::from_utf8_unchecked(output) })?;
            }
            Kind::FromBytes => {
                let bytes = collection_bytes(local_collection(&arguments, 0)?)?;
                match String::from_utf8(bytes) {
                    Ok(value) => self.push_nullable_string(Some(value.into()))?,
                    Err(_) => self.push_nullable_string(None)?,
                }
            }
        }
        Ok(None)
    }

    fn pop_integer(&mut self) -> Result<IntegerValue, InterpreterError> {
        match self.pop_scalar()? {
            mir::ScalarValue::Integer(value) => Ok(value),
            _ => Err(InterpreterError::new(
                "MIR integer evaluation produced another scalar type",
            )),
        }
    }

    fn pop_float(&mut self) -> Result<FloatValue, InterpreterError> {
        match self.pop_scalar()? {
            mir::ScalarValue::Float(value) => Ok(value),
            _ => Err(InterpreterError::new(
                "MIR float evaluation produced another scalar type",
            )),
        }
    }

    fn pop_bool(&mut self) -> Result<bool, InterpreterError> {
        match self.pop_scalar()? {
            mir::ScalarValue::Bool(value) => Ok(value),
            _ => Err(InterpreterError::new(
                "MIR bool evaluation produced another scalar type",
            )),
        }
    }

    fn eval_operand(&self, operand: &mir::Operand) -> Result<mir::ScalarValue, InterpreterError> {
        match operand {
            mir::Operand::Scalar(value) => Ok(*value),
            mir::Operand::Local(id) => match read_local(&self.current_frame()?.locals, *id)? {
                LocalValue::Scalar(value) => Ok(*value),
                LocalValue::String(_) => Err(InterpreterError::new(format!(
                    "MIR string local local{} was used as a scalar value",
                    id.0
                ))),
                LocalValue::Mixed(_) | LocalValue::NullableMixed(_) => Err(InterpreterError::new(
                    format!("MIR mixed local local{} was used as a scalar value", id.0),
                )),
                LocalValue::NullableString(_) => Err(InterpreterError::new(format!(
                    "MIR nullable-string local local{} was used as a scalar value",
                    id.0
                ))),
                LocalValue::NullableScalar { .. } | LocalValue::NullableClass { .. } => {
                    Err(InterpreterError::new(format!(
                        "MIR nullable local local{} was used as a scalar value",
                        id.0
                    )))
                }
                LocalValue::Class { .. } => Err(InterpreterError::new(format!(
                    "MIR class local local{} was used as a scalar value",
                    id.0
                ))),
                LocalValue::SharedReference { .. }
                | LocalValue::WeakReference { .. }
                | LocalValue::NullableSharedReference { .. }
                | LocalValue::NullableWeakReference { .. }
                | LocalValue::WritableSharedReference { .. }
                | LocalValue::WritableWeakReference { .. }
                | LocalValue::NullableWritableSharedReference { .. }
                | LocalValue::NullableWritableWeakReference { .. }
                | LocalValue::SharedReferenceAccess { .. }
                | LocalValue::NullableSharedReferenceAccess { .. } => {
                    Err(InterpreterError::new(format!(
                        "MIR shared handle local local{} was used as a scalar value",
                        id.0
                    )))
                }
                LocalValue::Collection(_) => Err(InterpreterError::new(format!(
                    "MIR collection local local{} was used as a scalar value",
                    id.0
                ))),
            },
            mir::Operand::NullablePayload(id) => {
                match read_local(&self.current_frame()?.locals, *id)? {
                    LocalValue::NullableScalar {
                        value: Some(value), ..
                    } => Ok(*value),
                    LocalValue::NullableScalar { value: None, .. } => Err(InterpreterError::new(
                        "MIR nullable payload was read while null",
                    )),
                    _ => Err(InterpreterError::new(
                        "MIR nullable payload has another type",
                    )),
                }
            }
            mir::Operand::Static(id) => match self.statics.get(id.0) {
                Some(LocalValue::Scalar(value)) => Ok(*value),
                _ => Err(InterpreterError::new(format!(
                    "MIR static{} was used as scalar",
                    id.0
                ))),
            },
            mir::Operand::Property { object, property } => {
                match self.read_property(*object, *property)? {
                    LocalValue::Scalar(value) => Ok(value),
                    _ => Err(InterpreterError::new(format!(
                        "MIR property{} was used as a scalar value",
                        property.index
                    ))),
                }
            }
            mir::Operand::CollectionLength(_)
            | mir::Operand::CollectionIndex { .. }
            | mir::Operand::CollectionKeyAt { .. } => Err(InterpreterError::new(
                "MIR collection operand requires queued evaluation",
            )),
            mir::Operand::StringIntrinsic(_) => Err(InterpreterError::new(
                "String intrinsic scalar operand was not queued",
            )),
            mir::Operand::MixedPayload { mixed, tag } => {
                let value =
                    mixed_value_from_local(read_local(&self.current_frame()?.locals, *mixed)?)
                        .ok_or_else(|| {
                            InterpreterError::new(
                                "MIR mixed scalar payload references another local type",
                            )
                        })?;
                if value.tag() != *tag {
                    return Err(InterpreterError::new(
                        "MIR mixed scalar payload observed another tag",
                    ));
                }
                let MixedValue::Scalar(value) = value else {
                    return Err(InterpreterError::new(
                        "MIR mixed scalar payload observed non-scalar payload",
                    ));
                };
                Ok(*value)
            }
        }
    }

    fn read_property(
        &self,
        object: mir::LocalId,
        property: crate::class_layout::PropertyId,
    ) -> Result<LocalValue, InterpreterError> {
        let object_id = match read_local(&self.current_frame()?.locals, object)? {
            LocalValue::Class { object, .. } => *object,
            LocalValue::NullableClass {
                object: Some(object),
                ..
            } => *object,
            _ => {
                return Err(InterpreterError::new(format!(
                    "MIR property access uses non-class local local{}",
                    object.0
                )))
            }
        };
        self.read_object_property(object_id, property)
    }

    fn read_object_property(
        &self,
        object_id: usize,
        property: crate::class_layout::PropertyId,
    ) -> Result<LocalValue, InterpreterError> {
        let object_value = self.heap.get(&object_id).ok_or_else(|| {
            InterpreterError::new(format!("MIR object {object_id} is not allocated"))
        })?;
        if object_value.class != property.class {
            return Err(InterpreterError::new(format!(
                "MIR property access expected class#{} but object has class#{}",
                property.class.0, object_value.class.0
            )));
        }
        object_value
            .properties
            .get(property.index)
            .and_then(|value| value.clone())
            .ok_or_else(|| {
                InterpreterError::new(format!(
                    "MIR property{} was read before assignment",
                    property.index
                ))
            })
    }

    fn assign_property(
        &mut self,
        object: mir::LocalId,
        property: crate::class_layout::PropertyId,
        value: LocalValue,
    ) -> Result<Option<LocalValue>, InterpreterError> {
        let object_id = match read_local(&self.current_frame()?.locals, object)? {
            LocalValue::Class { object, .. } => *object,
            LocalValue::NullableClass {
                object: Some(object),
                ..
            } => *object,
            _ => {
                return Err(InterpreterError::new(format!(
                    "MIR property assignment uses non-class local local{}",
                    object.0
                )))
            }
        };
        let object_value = self.heap.get_mut(&object_id).ok_or_else(|| {
            InterpreterError::new(format!("MIR object {object_id} is not allocated"))
        })?;
        if object_value.class != property.class {
            return Err(InterpreterError::new(format!(
                "MIR property assignment expected class#{} but object has class#{}",
                property.class.0, object_value.class.0
            )));
        }
        let slot = object_value
            .properties
            .get_mut(property.index)
            .ok_or_else(|| {
                InterpreterError::new(format!("MIR property{} does not exist", property.index))
            })?;
        Ok(slot.replace(value))
    }

    fn drop_class_local(&mut self, local: mir::LocalId) -> Result<(), InterpreterError> {
        let Some(value) = self
            .current_frame_mut()?
            .locals
            .get_mut(local.0)
            .ok_or_else(|| {
                InterpreterError::new(format!("MIR local local{} does not exist", local.0))
            })?
            .take()
        else {
            return Ok(());
        };
        match value {
            LocalValue::Class { object, class }
            | LocalValue::NullableClass {
                object: Some(object),
                class,
            } => self
                .current_frame_mut()?
                .tasks
                .push(EvaluationTask::DropObject { object, class }),
            LocalValue::NullableClass { object: None, .. } => {}
            _ => {
                return Err(InterpreterError::new(format!(
                    "MIR drop local{} did not contain a class value",
                    local.0
                )))
            }
        }
        Ok(())
    }

    fn read_or_take_local(
        &mut self,
        local: mir::LocalId,
        transfer: bool,
    ) -> Result<LocalValue, InterpreterError> {
        if transfer {
            return self
                .current_frame_mut()?
                .locals
                .get_mut(local.0)
                .ok_or_else(|| {
                    InterpreterError::new(format!("MIR local local{} does not exist", local.0))
                })?
                .take()
                .ok_or_else(|| {
                    InterpreterError::new(format!(
                        "MIR local local{} was moved before use",
                        local.0
                    ))
                });
        }
        Ok(read_local(&self.current_frame()?.locals, local)?.clone())
    }

    fn drop_shared_local(
        &mut self,
        local: mir::LocalId,
        weak: bool,
    ) -> Result<(), InterpreterError> {
        let Some(value) = self
            .current_frame_mut()?
            .locals
            .get_mut(local.0)
            .ok_or_else(|| {
                InterpreterError::new(format!("MIR local local{} does not exist", local.0))
            })?
            .take()
        else {
            return Ok(());
        };
        let control = match (weak, value) {
            (false, LocalValue::SharedReference { control, .. })
            | (
                false,
                LocalValue::NullableSharedReference {
                    control: Some(control),
                    ..
                },
            ) => Some(control),
            (false, LocalValue::NullableSharedReference { control: None, .. }) => None,
            (true, LocalValue::WeakReference { control, .. }) => Some(control),
            (
                true,
                LocalValue::NullableWeakReference {
                    control: Some(control),
                    ..
                },
            ) => Some(control),
            (true, LocalValue::NullableWeakReference { control: None, .. }) => None,
            _ => {
                return Err(InterpreterError::new(format!(
                    "MIR shared drop local{} contained another value type",
                    local.0
                )))
            }
        };
        if let Some(control) = control {
            self.current_frame_mut()?.tasks.push(if weak {
                EvaluationTask::ReleaseWeak(control)
            } else {
                EvaluationTask::ReleaseShared(control)
            });
        }
        Ok(())
    }

    fn drop_writable_shared_local(
        &mut self,
        local: mir::LocalId,
        kind: WritableDropKind,
    ) -> Result<(), InterpreterError> {
        let Some(value) = self
            .current_frame_mut()?
            .locals
            .get_mut(local.0)
            .ok_or_else(|| {
                InterpreterError::new(format!("MIR local local{} does not exist", local.0))
            })?
            .take()
        else {
            return Ok(());
        };
        let task = match (kind, value) {
            (WritableDropKind::Strong, LocalValue::WritableSharedReference { control, .. })
            | (
                WritableDropKind::Strong,
                LocalValue::NullableWritableSharedReference {
                    control: Some(control),
                    ..
                },
            ) => Some(EvaluationTask::ReleaseWritableShared(control)),
            (
                WritableDropKind::Strong,
                LocalValue::NullableWritableSharedReference { control: None, .. },
            ) => None,
            (WritableDropKind::Weak, LocalValue::WritableWeakReference { control, .. })
            | (
                WritableDropKind::Weak,
                LocalValue::NullableWritableWeakReference {
                    control: Some(control),
                    ..
                },
            ) => Some(EvaluationTask::ReleaseWritableWeak(control)),
            (
                WritableDropKind::Weak,
                LocalValue::NullableWritableWeakReference { control: None, .. },
            ) => None,
            (
                WritableDropKind::Access,
                LocalValue::SharedReferenceAccess {
                    control, writable, ..
                },
            ) => Some(EvaluationTask::ReleaseSharedAccess { control, writable }),
            (
                WritableDropKind::Access,
                LocalValue::NullableSharedReferenceAccess {
                    control: Some(control),
                    writable,
                    ..
                },
            ) => Some(EvaluationTask::ReleaseSharedAccess { control, writable }),
            (
                WritableDropKind::Access,
                LocalValue::NullableSharedReferenceAccess { control: None, .. },
            ) => None,
            _ => {
                return Err(InterpreterError::new(format!(
                    "MIR writable shared drop local{} contained another value type",
                    local.0
                )))
            }
        };
        if let Some(task) = task {
            self.current_frame_mut()?.tasks.push(task);
        }
        Ok(())
    }

    fn queue_object_drop(
        &mut self,
        object: usize,
        class: crate::class_layout::ClassId,
    ) -> Result<(), InterpreterError> {
        let value = self.heap.get(&object).ok_or_else(|| {
            InterpreterError::new(format!("MIR object {object} is not allocated"))
        })?;
        if value.class != class {
            return Err(InterpreterError::new(format!(
                "MIR drop expected class#{} but object has class#{}",
                class.0, value.class.0
            )));
        }
        let destructor = class_in(self.program, class)?.destructor;
        let frame = self.current_frame_mut()?;
        frame
            .tasks
            .push(EvaluationTask::FreeObject { object, class });
        frame
            .tasks
            .push(EvaluationTask::DropObjectProperties { object, class });
        if let Some(function) = destructor {
            self.push_frame(
                function,
                &[LocalValue::Class { object, class }],
                Some(ReturnExpectation::Void),
                None,
            )?;
        }
        Ok(())
    }

    fn queue_object_property_drops(
        &mut self,
        object: usize,
        class: crate::class_layout::ClassId,
    ) -> Result<(), InterpreterError> {
        let object_value = self.heap.get_mut(&object).ok_or_else(|| {
            InterpreterError::new(format!("MIR object {object} is not allocated"))
        })?;
        if object_value.class != class {
            return Err(InterpreterError::new(format!(
                "MIR property drop expected class#{} but object has class#{}",
                class.0, object_value.class.0
            )));
        }
        let mut drops = Vec::new();
        for property in object_value.properties.iter_mut().rev() {
            if let Some(value) = property.take() {
                collect_owned_objects_from_value(value, &mut drops);
            }
        }
        let frame = self.current_frame_mut()?;
        for drop in drops.into_iter().rev() {
            frame.tasks.push(match drop {
                OwnedDrop::Class { object, class } => EvaluationTask::DropObject { object, class },
                OwnedDrop::Shared(control) => EvaluationTask::ReleaseShared(control),
                OwnedDrop::Weak(control) => EvaluationTask::ReleaseWeak(control),
                OwnedDrop::WritableShared(control) => {
                    EvaluationTask::ReleaseWritableShared(control)
                }
                OwnedDrop::WritableWeak(control) => EvaluationTask::ReleaseWritableWeak(control),
                OwnedDrop::SharedAccess { control, writable } => {
                    EvaluationTask::ReleaseSharedAccess { control, writable }
                }
            });
        }
        Ok(())
    }

    fn free_object(
        &mut self,
        object: usize,
        class: crate::class_layout::ClassId,
    ) -> Result<(), InterpreterError> {
        let Some(value) = self.heap.remove(&object) else {
            return Ok(());
        };
        if value.class != class {
            return Err(InterpreterError::new(format!(
                "MIR free expected class#{} but object has class#{}",
                class.0, value.class.0
            )));
        }
        Ok(())
    }

    fn cleanup_current_frame(&mut self) -> Result<(), InterpreterError> {
        let function = function_in(self.program, self.current_frame()?.function)?;
        let owned_classes = function
            .locals
            .iter()
            .filter_map(|local| match (local.owned, local.ty) {
                (true, mir::Type::Class(_)) => Some(local.id),
                _ => None,
            })
            .collect::<Vec<_>>();
        let owned_collections = function
            .locals
            .iter()
            .filter_map(|local| match (local.owned, local.ty) {
                (true, mir::Type::Collection(_)) => Some(local.id),
                _ => None,
            })
            .collect::<Vec<_>>();
        let owned_mixed = function
            .locals
            .iter()
            .filter_map(|local| match (local.owned, local.ty) {
                (true, mir::Type::Mixed | mir::Type::NullableMixed) => Some(local.id),
                _ => None,
            })
            .collect::<Vec<_>>();
        for local in owned_mixed {
            self.drop_mixed_local(local)?;
        }
        for local in owned_classes {
            self.drop_class_local(local)?;
        }
        for local in owned_collections {
            self.drop_collection_local(local)?;
        }
        Ok(())
    }

    fn current_frame(&self) -> Result<&CallFrame, InterpreterError> {
        self.frames
            .last()
            .ok_or_else(|| InterpreterError::new("MIR interpreter has no active call frame"))
    }

    fn integer_panic_event(panic: IntegerPanic, span: Span) -> RuntimePanicEvent {
        RuntimePanicEvent {
            code: panic.code(),
            operation_span: span,
            primary_span: span,
            facts: Vec::new(),
            explanation: None,
        }
    }

    fn current_frame_mut(&mut self) -> Result<&mut CallFrame, InterpreterError> {
        self.frames
            .last_mut()
            .ok_or_else(|| InterpreterError::new("MIR interpreter has no active call frame"))
    }

    fn collection_local(&self, local: mir::LocalId) -> Result<&CollectionValue, InterpreterError> {
        match read_local(&self.current_frame()?.locals, local)? {
            LocalValue::Collection(value) => Ok(value),
            _ => Err(InterpreterError::new(format!(
                "MIR local local{} is not a collection",
                local.0
            ))),
        }
    }

    fn shared_access_payload(&self, local: mir::LocalId) -> Result<LocalValue, InterpreterError> {
        let LocalValue::SharedReferenceAccess { control, .. } =
            read_local(&self.current_frame()?.locals, local)?
        else {
            return Err(InterpreterError::new(format!(
                "MIR local local{} is not a shared access object",
                local.0
            )));
        };
        control
            .borrow()
            .payload
            .clone()
            .ok_or_else(|| InterpreterError::new("shared access payload is no longer alive"))
    }

    fn byte_collection(&self, local: mir::LocalId) -> Result<Vec<u8>, InterpreterError> {
        let collection = self.collection_local(local)?;
        let definition = self
            .program
            .collection_types
            .get(collection.ty.0)
            .ok_or_else(|| InterpreterError::new("Bytes type does not exist"))?;
        if definition.kind != mir::CollectionKind::Bytes {
            return Err(InterpreterError::new(
                "MIR Bytes operation used another collection",
            ));
        }
        collection
            .entries()
            .iter()
            .map(|(_, value)| match value {
                LocalValue::Scalar(mir::ScalarValue::Integer(value))
                    if value.ty == IntegerType::UInt8 =>
                {
                    Ok(value.unsigned_value() as u8)
                }
                _ => Err(InterpreterError::new(
                    "MIR Bytes contains a non-uint8 value",
                )),
            })
            .collect()
    }

    fn push_byte_collection(
        &mut self,
        collection: mir::CollectionTypeId,
        bytes: &[u8],
    ) -> Result<(), InterpreterError> {
        let entries = bytes
            .iter()
            .map(|byte| {
                (
                    None,
                    LocalValue::Scalar(mir::ScalarValue::Integer(
                        IntegerValue::from_u128(IntegerType::UInt8, u128::from(*byte))
                            .expect("u8 always fits uint8"),
                    )),
                )
            })
            .collect();
        self.current_frame_mut()?
            .values
            .push(EvaluationValue::Collection(CollectionValue::new(
                collection, entries,
            )));
        Ok(())
    }

    fn collection_position(
        &self,
        local: mir::LocalId,
        index: &LocalValue,
        positional: bool,
    ) -> Result<usize, CollectionAccessError> {
        let collection = self
            .collection_local(local)
            .map_err(|_| CollectionAccessError::Catalogued("P1001"))?;
        let definition = self
            .program
            .collection_types
            .get(collection.ty.0)
            .ok_or(CollectionAccessError::Catalogued("P1001"))?;
        if definition.key.is_some() && !positional {
            collection
                .entries()
                .iter()
                .position(|(key, _)| key.as_ref() == Some(index))
                .ok_or(CollectionAccessError::Catalogued("P1312"))
        } else {
            let LocalValue::Scalar(mir::ScalarValue::Integer(index)) = index else {
                return Err(CollectionAccessError::Catalogued("P1001"));
            };
            let signed_index = index.signed_value() as i64;
            let length = collection.entries().len();
            let Some(index) = usize::try_from(index.signed_value()).ok() else {
                return Err(CollectionAccessError::Bounds {
                    code: if definition.kind == mir::CollectionKind::Bytes {
                        "P1301"
                    } else {
                        "P1310"
                    },
                    index: signed_index,
                    length,
                });
            };
            (index < length)
                .then_some(index)
                .ok_or_else(|| CollectionAccessError::Bounds {
                    code: if definition.kind == mir::CollectionKind::Bytes {
                        "P1301"
                    } else {
                        "P1310"
                    },
                    index: signed_index,
                    length,
                })
        }
    }

    fn collection_value_at(
        &mut self,
        local: mir::LocalId,
        index: &LocalValue,
        transfer: bool,
        positional: bool,
    ) -> Result<LocalValue, CollectionAccessError> {
        let position = self.collection_position(local, index, positional)?;
        if transfer {
            self.collection_local(local)
                .map(|collection| collection.entries_mut().remove(position).1)
                .map_err(|_| CollectionAccessError::Catalogued("P1001"))
        } else {
            self.collection_local(local)
                .map(|collection| collection.entries()[position].1.clone())
                .map_err(|_| CollectionAccessError::Catalogued("P1001"))
        }
    }

    fn pop_collection_offset(&mut self) -> Result<usize, InterpreterError> {
        let value = self.pop_integer()?;
        usize::try_from(value.signed_value())
            .map_err(|_| InterpreterError::new("MIR collection offset is negative"))
    }

    fn collection_key_at(
        &self,
        local: mir::LocalId,
        offset: usize,
    ) -> Result<LocalValue, InterpreterError> {
        self.collection_local(local)?
            .entries()
            .get(offset)
            .and_then(|(key, _)| key.clone())
            .ok_or_else(|| InterpreterError::new("MIR dictionary key offset is out of bounds"))
    }

    fn drop_collection_local(&mut self, local: mir::LocalId) -> Result<(), InterpreterError> {
        let value = self
            .current_frame_mut()?
            .locals
            .get_mut(local.0)
            .ok_or_else(|| InterpreterError::new("collection local does not exist"))?
            .take();
        if let Some(LocalValue::Collection(collection)) = value {
            let mut drops = Vec::new();
            collect_owned_objects_from_collection(collection, &mut drops);
            for drop in drops {
                self.push_owned_drop_task(drop)?;
            }
        }
        Ok(())
    }

    fn clear_collection_local(&mut self, local: mir::LocalId) -> Result<(), InterpreterError> {
        let entries = {
            let collection = self.collection_local(local)?;
            std::mem::take(&mut *collection.entries_mut())
        };
        let mut drops = Vec::new();
        collect_owned_objects_from_entries(entries, &mut drops);
        for drop in drops {
            self.push_owned_drop_task(drop)?;
        }
        Ok(())
    }

    fn drop_mixed_local(&mut self, local: mir::LocalId) -> Result<(), InterpreterError> {
        let value = self
            .current_frame_mut()?
            .locals
            .get_mut(local.0)
            .ok_or_else(|| InterpreterError::new("mixed local does not exist"))?
            .take();
        if let Some(value @ (LocalValue::Mixed(_) | LocalValue::NullableMixed(_))) = value {
            self.queue_value_drops(value)?;
        }
        Ok(())
    }

    fn queue_value_drops(&mut self, value: LocalValue) -> Result<(), InterpreterError> {
        let mut drops = Vec::new();
        collect_owned_objects_from_value(value, &mut drops);
        for drop in drops {
            self.push_owned_drop_task(drop)?;
        }
        Ok(())
    }

    fn push_owned_drop_task(&mut self, drop: OwnedDrop) -> Result<(), InterpreterError> {
        self.current_frame_mut()?.tasks.push(match drop {
            OwnedDrop::Class { object, class } => EvaluationTask::DropObject { object, class },
            OwnedDrop::Shared(control) => EvaluationTask::ReleaseShared(control),
            OwnedDrop::Weak(control) => EvaluationTask::ReleaseWeak(control),
            OwnedDrop::WritableShared(control) => EvaluationTask::ReleaseWritableShared(control),
            OwnedDrop::WritableWeak(control) => EvaluationTask::ReleaseWritableWeak(control),
            OwnedDrop::SharedAccess { control, writable } => {
                EvaluationTask::ReleaseSharedAccess { control, writable }
            }
        });
        Ok(())
    }

    fn finish_entry(
        &self,
        entry: &mir::Function,
        outcome: FunctionOutcome,
    ) -> Result<InterpreterOutput, InterpreterError> {
        match (entry.return_type, outcome) {
            (
                mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(
                    IntegerType::Int64,
                ))),
                FunctionOutcome::Value(LocalValue::Scalar(mir::ScalarValue::Integer(value))),
            ) if value.ty == IntegerType::Int64 => {
                let value = value.signed_value();
                if (0..=125).contains(&value) {
                    Ok(InterpreterOutput {
                        stdout: self.stdout.clone(),
                        stderr: self.stderr.clone(),
                        exit_status: value as i32,
                        runtime_diagnostic: None,
                    })
                } else {
                    Ok(self.runtime_panic_output_for_entry(
                        RuntimePanicEvent {
                            code: "P1111",
                            operation_span: entry.source_span,
                            primary_span: entry.source_span,
                            facts: vec![RuntimeFact {
                                name: doria_diagnostic_catalogue::PROCESS_STATUS_FACT.to_string(),
                                value: RuntimeFactValue::Signed(value as i64),
                            }],
                            explanation: None,
                        },
                        entry,
                    ))
                }
            }
            (mir::ReturnType::Void, FunctionOutcome::Void) => Ok(InterpreterOutput {
                stdout: self.stdout.clone(),
                stderr: self.stderr.clone(),
                exit_status: 0,
                runtime_diagnostic: None,
            }),
            (mir::ReturnType::Value(_), FunctionOutcome::Void) => Err(InterpreterError::new(
                "MIR scalar entry function returned void",
            )),
            (mir::ReturnType::Void, FunctionOutcome::Value(_)) => Err(InterpreterError::new(
                "MIR void entry function returned a scalar value",
            )),
            (mir::ReturnType::Value(ty), FunctionOutcome::Value(value)) => {
                Err(InterpreterError::new(format!(
                    "MIR entry must return int, but signature/value were {ty}/{}",
                    local_value_type(&value)
                )))
            }
        }
    }

    fn runtime_panic_step(&self, code: &'static str) -> Result<StepOutcome, InterpreterError> {
        self.runtime_panic_step_with_facts(code, Vec::new())
    }

    fn runtime_panic_step_at(
        &self,
        code: &'static str,
        span: Span,
    ) -> Result<StepOutcome, InterpreterError> {
        self.runtime_panic_step_with_facts_at(code, span, Vec::new())
    }

    fn collection_access_panic_step(
        &self,
        error: CollectionAccessError,
    ) -> Result<StepOutcome, InterpreterError> {
        let span = self
            .frames
            .last()
            .and_then(|frame| self.program.functions.get(frame.function.0))
            .map_or(Span::default(), |function| function.source_span);
        self.collection_access_panic_step_at(error, span)
    }

    fn collection_access_panic_step_at(
        &self,
        error: CollectionAccessError,
        span: Span,
    ) -> Result<StepOutcome, InterpreterError> {
        match error {
            CollectionAccessError::Catalogued(code) => self.runtime_panic_step_at(code, span),
            CollectionAccessError::Bounds {
                code,
                index,
                length,
            } => self.runtime_panic_step_with_facts_at(
                code,
                span,
                vec![
                    RuntimeFact {
                        name: doria_diagnostic_catalogue::INDEX_FACT.to_string(),
                        value: RuntimeFactValue::Signed(index),
                    },
                    RuntimeFact {
                        name: doria_diagnostic_catalogue::INDEXED_LENGTH_FACT.to_string(),
                        value: RuntimeFactValue::Unsigned(length as u64),
                    },
                ],
            ),
        }
    }

    fn shared_access_conflict_panic_step_at(
        &self,
        conflict: SharedAccessConflict,
        span: Span,
    ) -> Result<StepOutcome, InterpreterError> {
        self.runtime_panic_step_with_facts_at(
            "P1501",
            span,
            vec![RuntimeFact {
                name: doria_diagnostic_catalogue::SHARED_ACCESS_CONFLICT_REASON_FACT.to_string(),
                value: RuntimeFactValue::StaticString(conflict.reason().to_string()),
            }],
        )
    }

    fn runtime_panic_step_with_facts(
        &self,
        code: &'static str,
        facts: Vec<RuntimeFact>,
    ) -> Result<StepOutcome, InterpreterError> {
        let span = self
            .frames
            .last()
            .and_then(|frame| self.program.functions.get(frame.function.0))
            .map_or(Span::default(), |function| function.source_span);
        self.runtime_panic_step_with_facts_at(code, span, facts)
    }

    fn runtime_panic_step_with_facts_at(
        &self,
        code: &'static str,
        span: Span,
        facts: Vec<RuntimeFact>,
    ) -> Result<StepOutcome, InterpreterError> {
        Ok(StepOutcome::RuntimePanic(RuntimePanicEvent {
            code,
            operation_span: span,
            primary_span: span,
            facts,
            explanation: None,
        }))
    }

    fn runtime_panic_output(&self, event: RuntimePanicEvent) -> InterpreterOutput {
        let code = event.code;
        let primary_span = event.primary_span;
        let explanation = event.explanation;
        let origin_function = self
            .frames
            .last()
            .and_then(|frame| self.program.functions.get(frame.function.0))
            .map(|function| function.name.clone());
        let mut path = Vec::with_capacity(self.frames.len());
        for reverse_index in 0..self.frames.len() {
            let frame_index = self.frames.len() - 1 - reverse_index;
            let frame = &self.frames[frame_index];
            let Some(function) = self.program.functions.get(frame.function.0) else {
                continue;
            };
            let span = if reverse_index == 0 {
                event.operation_span
            } else {
                self.frames[frame_index + 1]
                    .entered_from
                    .unwrap_or(function.source_span)
            };
            path.push(RuntimeOutcomeFrame {
                function: function.name.clone(),
                source: DiagnosticSource::Current,
                span,
            });
        }
        let outcome = RuntimeOutcomeDetails {
            process_status: 101,
            termination_behavior: TerminationBehavior::AbortWithoutCleanup,
            origin: RuntimeOutcomeOrigin {
                source: DiagnosticSource::Current,
                span: event.primary_span,
                function: origin_function,
            },
            path,
            facts: event.facts,
        };
        self.render_runtime_panic(code, primary_span, explanation, outcome)
    }

    fn runtime_panic_output_for_entry(
        &self,
        event: RuntimePanicEvent,
        entry: &mir::Function,
    ) -> InterpreterOutput {
        let code = event.code;
        let primary_span = event.primary_span;
        let explanation = event.explanation;
        let outcome = RuntimeOutcomeDetails {
            process_status: 101,
            termination_behavior: TerminationBehavior::AbortWithoutCleanup,
            origin: RuntimeOutcomeOrigin {
                source: DiagnosticSource::Current,
                span: event.primary_span,
                function: Some(entry.name.clone()),
            },
            path: vec![RuntimeOutcomeFrame {
                function: entry.name.clone(),
                source: DiagnosticSource::Current,
                span: event.operation_span,
            }],
            facts: event.facts,
        };
        self.render_runtime_panic(code, primary_span, explanation, outcome)
    }

    fn render_runtime_panic(
        &self,
        code: &'static str,
        primary_span: Span,
        explanation: Option<String>,
        outcome: RuntimeOutcomeDetails,
    ) -> InterpreterOutput {
        let mut diagnostic = Diagnostic::runtime_panic(code, primary_span, outcome);
        if let Some(explanation) = explanation {
            diagnostic.explanation = Some(explanation);
        }
        if code == "P1000" {
            let message = diagnostic
                .runtime_outcome
                .as_ref()
                .and_then(|outcome| outcome.facts.iter().find(|fact| fact.name == "message"))
                .and_then(|fact| match &fact.value {
                    RuntimeFactValue::StaticString(message) => Some(message.clone()),
                    _ => None,
                });
            if let Some(message) = message {
                diagnostic.notes.push(message);
            }
        }
        let rendered = crate::diagnostics::render_diagnostics(
            &self.program.source,
            std::slice::from_ref(&diagnostic),
            RenderOptions {
                format: DiagnosticFormat::Human,
                color: ColorChoice::Never,
                context_lines: 0,
                ..RenderOptions::default()
            },
        );
        let mut stderr = Vec::new();
        stderr.extend_from_slice(&self.stderr);
        stderr.extend_from_slice(rendered.as_bytes());
        stderr.push(b'\n');
        InterpreterOutput {
            stdout: self.stdout.clone(),
            stderr,
            exit_status: 101,
            runtime_diagnostic: Some(diagnostic),
        }
    }
}

fn string_panic_event(
    error: StringError,
    _kind: mir::StringIntrinsicKind,
    operation_span: Span,
    argument_spans: &[Span],
    facts: Vec<RuntimeFact>,
    explanation: Option<String>,
) -> RuntimePanicEvent {
    let (code, argument) = match error {
        StringError::ResultTooLarge => ("P1205", None),
        StringError::SliceLengthNegative => ("P1201", Some(2)),
        StringError::RepetitionCountNegative => ("P1204", Some(1)),
        StringError::PaddingLengthNegative => ("P1202", Some(1)),
        StringError::PaddingTextEmpty => ("P1203", Some(2)),
    };
    RuntimePanicEvent {
        code,
        operation_span,
        primary_span: argument
            .and_then(|index| argument_spans.get(index).copied())
            .unwrap_or(operation_span),
        facts,
        explanation,
    }
}

pub fn render_debug_output(output: &InterpreterOutput) -> String {
    let stdout = if output.stdout.is_empty() {
        "stdout:\n".to_string()
    } else {
        format!("stdout: {}\n", String::from_utf8_lossy(&output.stdout))
    };
    if output.stderr.is_empty() {
        format!("exit_status: {}\n{stdout}", output.exit_status)
    } else {
        format!(
            "exit_status: {}\n{stdout}stderr: {}",
            output.exit_status,
            String::from_utf8_lossy(&output.stderr)
        )
    }
}

fn eval_compare(
    op: mir::CompareOp,
    left: mir::ScalarValue,
    right: mir::ScalarValue,
) -> Result<bool, InterpreterError> {
    let result = match (left, right) {
        (mir::ScalarValue::Integer(left), mir::ScalarValue::Integer(right))
            if left.ty == right.ty =>
        {
            let ordering = left.compare(right);
            match op {
                mir::CompareOp::Equal => ordering.is_eq(),
                mir::CompareOp::NotEqual => !ordering.is_eq(),
                mir::CompareOp::Less => ordering.is_lt(),
                mir::CompareOp::LessEqual => !ordering.is_gt(),
                mir::CompareOp::Greater => ordering.is_gt(),
                mir::CompareOp::GreaterEqual => !ordering.is_lt(),
            }
        }
        (mir::ScalarValue::Float(left), mir::ScalarValue::Float(right)) if left.ty == right.ty => {
            match op {
                mir::CompareOp::Equal => left.compare_equal(right),
                mir::CompareOp::NotEqual => left.compare_not_equal(right),
                mir::CompareOp::Less => left.compare_less(right),
                mir::CompareOp::LessEqual => left.compare_less_equal(right),
                mir::CompareOp::Greater => left.compare_greater(right),
                mir::CompareOp::GreaterEqual => left.compare_greater_equal(right),
            }
        }
        (mir::ScalarValue::Bool(left), mir::ScalarValue::Bool(right)) => match op {
            mir::CompareOp::Equal => left == right,
            mir::CompareOp::NotEqual => left != right,
            mir::CompareOp::Less => !left && right,
            mir::CompareOp::LessEqual => !left || right,
            mir::CompareOp::Greater => left && !right,
            mir::CompareOp::GreaterEqual => left || !right,
        },
        (mir::ScalarValue::Enum(left), mir::ScalarValue::Enum(right))
            if left.enum_id == right.enum_id =>
        {
            match op {
                mir::CompareOp::Equal => left.case_id == right.case_id,
                mir::CompareOp::NotEqual => left.case_id != right.case_id,
                mir::CompareOp::Less
                | mir::CompareOp::LessEqual
                | mir::CompareOp::Greater
                | mir::CompareOp::GreaterEqual => {
                    return Err(InterpreterError::new(
                        "MIR enum values only support equality comparison",
                    ));
                }
            }
        }
        _ => {
            return Err(InterpreterError::new(
                "MIR comparison operands have different scalar types",
            ))
        }
    };
    Ok(result)
}

fn eval_unary(
    op: mir::IntegerUnaryOp,
    operand: IntegerValue,
) -> Result<IntegerValue, IntegerPanic> {
    match op {
        mir::IntegerUnaryOp::Negate => operand.checked_neg(),
        mir::IntegerUnaryOp::BitwiseNot => Ok(operand.bitwise_not()),
    }
}

fn eval_binary(
    op: mir::IntegerBinaryOp,
    left: IntegerValue,
    right: IntegerValue,
) -> Result<IntegerValue, IntegerPanic> {
    match op {
        mir::IntegerBinaryOp::Add => left.checked_add(right),
        mir::IntegerBinaryOp::Subtract => left.checked_sub(right),
        mir::IntegerBinaryOp::Multiply => left.checked_mul(right),
        mir::IntegerBinaryOp::Divide => left.divide(right),
        mir::IntegerBinaryOp::Remainder => left.remainder(right),
        mir::IntegerBinaryOp::ShiftLeft => left.shift_left(right),
        mir::IntegerBinaryOp::ShiftRight => left.shift_right(right),
        mir::IntegerBinaryOp::BitwiseAnd => Ok(left.bitwise_and(right)),
        mir::IntegerBinaryOp::BitwiseXor => Ok(left.bitwise_xor(right)),
        mir::IntegerBinaryOp::BitwiseOr => Ok(left.bitwise_or(right)),
    }
}

fn local_string(arguments: &[LocalValue], index: usize) -> Result<&str, InterpreterError> {
    match arguments.get(index) {
        Some(LocalValue::String(value)) => Ok(value.as_ref()),
        _ => Err(InterpreterError::new(format!(
            "String intrinsic argument {index} is not a string"
        ))),
    }
}

fn local_collection(
    arguments: &[LocalValue],
    index: usize,
) -> Result<&CollectionValue, InterpreterError> {
    match arguments.get(index) {
        Some(LocalValue::Collection(value)) => Ok(value),
        _ => Err(InterpreterError::new(format!(
            "String intrinsic argument {index} is not a collection"
        ))),
    }
}

fn local_int(arguments: &[LocalValue], index: usize) -> Result<i64, InterpreterError> {
    match arguments.get(index) {
        Some(LocalValue::Scalar(mir::ScalarValue::Integer(value)))
            if value.ty == IntegerType::Int64 =>
        {
            i64::try_from(value.signed_value()).map_err(|_| {
                InterpreterError::new(format!(
                    "String intrinsic argument {index} is outside Doria int range"
                ))
            })
        }
        _ => Err(InterpreterError::new(format!(
            "String intrinsic argument {index} is not an int"
        ))),
    }
}

fn local_nullable_int(
    arguments: &[LocalValue],
    index: usize,
) -> Result<Option<i64>, InterpreterError> {
    match arguments.get(index) {
        Some(LocalValue::NullableScalar {
            ty: mir::ScalarType::Integer(IntegerType::Int64),
            value: None,
        }) => Ok(None),
        Some(LocalValue::NullableScalar {
            ty: mir::ScalarType::Integer(IntegerType::Int64),
            value: Some(mir::ScalarValue::Integer(value)),
        }) => i64::try_from(value.signed_value()).map(Some).map_err(|_| {
            InterpreterError::new(format!(
                "String intrinsic argument {index} is outside Doria int range"
            ))
        }),
        _ => Err(InterpreterError::new(format!(
            "String intrinsic argument {index} is not a nullable int"
        ))),
    }
}

fn collection_result_id(ty: mir::Type) -> Result<mir::CollectionTypeId, InterpreterError> {
    match ty {
        mir::Type::Collection(collection) => Ok(collection),
        _ => Err(InterpreterError::new(
            "String intrinsic result is not a collection",
        )),
    }
}

fn collection_bytes(collection: &CollectionValue) -> Result<Vec<u8>, InterpreterError> {
    collection
        .entries()
        .iter()
        .map(|(_, value)| match value {
            LocalValue::Scalar(mir::ScalarValue::Integer(value))
                if value.ty == IntegerType::UInt8 =>
            {
                u8::try_from(value.unsigned_value())
                    .map_err(|_| InterpreterError::new("MIR Bytes value is outside uint8 range"))
            }
            _ => Err(InterpreterError::new(
                "String intrinsic Bytes argument contains a non-uint8 value",
            )),
        })
        .collect()
}

fn local_value_type(value: &LocalValue) -> mir::Type {
    match value {
        LocalValue::Scalar(value) => mir::Type::Scalar(value.ty()),
        LocalValue::String(_) => mir::Type::String,
        LocalValue::Mixed(_) => mir::Type::Mixed,
        LocalValue::NullableScalar { ty, .. } => mir::Type::NullableScalar(*ty),
        LocalValue::NullableString(_) => mir::Type::NullableString,
        LocalValue::NullableMixed(_) => mir::Type::NullableMixed,
        LocalValue::Class { class, .. } => mir::Type::Class(*class),
        LocalValue::NullableClass { class, .. } => mir::Type::NullableClass(*class),
        LocalValue::SharedReference { class, .. } => mir::Type::SharedReference(*class),
        LocalValue::WeakReference { class, .. } => mir::Type::WeakReference(*class),
        LocalValue::NullableSharedReference { class, .. } => {
            mir::Type::NullableSharedReference(*class)
        }
        LocalValue::NullableWeakReference { class, .. } => mir::Type::NullableWeakReference(*class),
        LocalValue::WritableSharedReference { payload, .. } => {
            mir::Type::WritableSharedReference(*payload)
        }
        LocalValue::WritableWeakReference { payload, .. } => {
            mir::Type::WritableWeakReference(*payload)
        }
        LocalValue::NullableWritableSharedReference { payload, .. } => {
            mir::Type::NullableWritableSharedReference(*payload)
        }
        LocalValue::NullableWritableWeakReference { payload, .. } => {
            mir::Type::NullableWritableWeakReference(*payload)
        }
        LocalValue::SharedReferenceAccess {
            payload, writable, ..
        } => {
            if *writable {
                mir::Type::WritableSharedReferenceAccess(*payload)
            } else {
                mir::Type::ReadonlySharedReferenceAccess(*payload)
            }
        }
        LocalValue::NullableSharedReferenceAccess {
            payload, writable, ..
        } => {
            if *writable {
                mir::Type::NullableWritableSharedReferenceAccess(*payload)
            } else {
                mir::Type::NullableReadonlySharedReferenceAccess(*payload)
            }
        }
        LocalValue::Collection(value) => {
            if value.nullable {
                mir::Type::NullableCollection(value.ty)
            } else {
                mir::Type::Collection(value.ty)
            }
        }
    }
}

fn writable_payload_type(payload: mir::WritableSharedPayload) -> mir::Type {
    match payload {
        mir::WritableSharedPayload::Class(class) => mir::Type::Class(class),
        mir::WritableSharedPayload::Collection(collection) => mir::Type::Collection(collection),
    }
}

fn non_nullable_type(ty: mir::Type) -> Option<mir::Type> {
    match ty {
        mir::Type::NullableScalar(ty) => Some(mir::Type::Scalar(ty)),
        mir::Type::NullableString => Some(mir::Type::String),
        mir::Type::NullableMixed => Some(mir::Type::Mixed),
        mir::Type::NullableClass(class) => Some(mir::Type::Class(class)),
        mir::Type::NullableSharedReference(class) => Some(mir::Type::SharedReference(class)),
        mir::Type::NullableWeakReference(class) => Some(mir::Type::WeakReference(class)),
        mir::Type::NullableWritableSharedReference(payload) => {
            Some(mir::Type::WritableSharedReference(payload))
        }
        mir::Type::NullableWritableWeakReference(payload) => {
            Some(mir::Type::WritableWeakReference(payload))
        }
        mir::Type::NullableCollection(collection) => Some(mir::Type::Collection(collection)),
        mir::Type::Collection(_) => None,
        _ => None,
    }
}

fn owned_object(value: &LocalValue) -> Option<(usize, crate::class_layout::ClassId)> {
    match value {
        LocalValue::Class { object, class }
        | LocalValue::NullableClass {
            object: Some(object),
            class,
        } => Some((*object, *class)),
        LocalValue::Mixed(MixedValue::Class {
            object,
            class,
            owner,
            payload_owned,
        })
        | LocalValue::NullableMixed(Some(MixedValue::Class {
            object,
            class,
            owner,
            payload_owned,
        })) => {
            let claims = owner.get();
            if claims == 0 {
                None
            } else {
                owner.set(claims - 1);
                (claims == 1 && *payload_owned).then_some((*object, *class))
            }
        }
        _ => None,
    }
}

fn retain_mixed_claim(value: &mut MixedValue, ownership: mir::MixedOwnership) {
    if ownership == mir::MixedOwnership::Owned {
        return;
    }
    if let MixedValue::Class { owner, .. } = value {
        owner.set(owner.get().saturating_add(1));
    }
}

fn collect_owned_objects_from_collection(collection: CollectionValue, drops: &mut Vec<OwnedDrop>) {
    collect_owned_objects_from_entries(collection.entries().iter().cloned(), drops);
}

fn collect_owned_objects_from_entries(
    entries: impl IntoIterator<Item = (Option<LocalValue>, LocalValue)>,
    drops: &mut Vec<OwnedDrop>,
) {
    for (key, value) in entries {
        if let Some(key) = key {
            collect_owned_objects_from_value(key, drops);
        }
        collect_owned_objects_from_value(value, drops);
    }
}

fn collect_owned_objects_from_value(value: LocalValue, drops: &mut Vec<OwnedDrop>) {
    match value {
        LocalValue::SharedReference { control, .. } => drops.push(OwnedDrop::Shared(control)),
        LocalValue::WeakReference { control, .. } => drops.push(OwnedDrop::Weak(control)),
        LocalValue::NullableSharedReference {
            control: Some(control),
            ..
        } => drops.push(OwnedDrop::Shared(control)),
        LocalValue::NullableWeakReference {
            control: Some(control),
            ..
        } => drops.push(OwnedDrop::Weak(control)),
        LocalValue::WritableSharedReference { control, .. } => {
            drops.push(OwnedDrop::WritableShared(control))
        }
        LocalValue::WritableWeakReference { control, .. } => {
            drops.push(OwnedDrop::WritableWeak(control))
        }
        LocalValue::NullableWritableSharedReference {
            control: Some(control),
            ..
        } => drops.push(OwnedDrop::WritableShared(control)),
        LocalValue::NullableWritableWeakReference {
            control: Some(control),
            ..
        } => drops.push(OwnedDrop::WritableWeak(control)),
        LocalValue::SharedReferenceAccess {
            control, writable, ..
        } => drops.push(OwnedDrop::SharedAccess { control, writable }),
        LocalValue::NullableSharedReferenceAccess {
            control: Some(control),
            writable,
            ..
        } => drops.push(OwnedDrop::SharedAccess { control, writable }),
        LocalValue::Collection(collection) => {
            collect_owned_objects_from_collection(collection, drops)
        }
        value => {
            if let Some((object, class)) = owned_object(&value) {
                drops.push(OwnedDrop::Class { object, class });
            }
        }
    }
}

fn collection_values_equal(ty: mir::Type, left: &LocalValue, right: &LocalValue) -> bool {
    match (ty, left, right) {
        (
            mir::Type::Scalar(mir::ScalarType::Float(FloatType::Float32)),
            LocalValue::Scalar(mir::ScalarValue::Float(left)),
            LocalValue::Scalar(mir::ScalarValue::Float(right)),
        ) => left.as_f32() == right.as_f32(),
        (
            mir::Type::Scalar(mir::ScalarType::Float(FloatType::Float64)),
            LocalValue::Scalar(mir::ScalarValue::Float(left)),
            LocalValue::Scalar(mir::ScalarValue::Float(right)),
        ) => left.as_f64() == right.as_f64(),
        _ => left == right,
    }
}

fn order_collection_entries(
    definition: &mir::CollectionType,
    entries: &mut CollectionEntries,
) -> Result<(), InterpreterError> {
    let Some(comparator) = definition.comparator else {
        return Ok(());
    };
    let keyed = definition.kind == mir::CollectionKind::SortedDictionary;
    if entries.iter().any(|(key, value)| {
        let candidate = if keyed { key.as_ref() } else { Some(value) };
        candidate.is_none_or(|candidate| {
            compare_collection_values(comparator, candidate, candidate).is_none()
        })
    }) {
        return Err(InterpreterError::new(
            "ordered collection entry does not match its comparator identity",
        ));
    }
    entries.sort_by(|(left_key, left_value), (right_key, right_value)| {
        let left = if keyed {
            left_key.as_ref().expect("validated sorted dictionary key")
        } else {
            left_value
        };
        let right = if keyed {
            right_key.as_ref().expect("validated sorted dictionary key")
        } else {
            right_value
        };
        compare_collection_values(comparator, left, right)
            .expect("validated ordered collection value")
    });
    Ok(())
}

fn compare_collection_values(
    comparator: mir::CollectionComparator,
    left: &LocalValue,
    right: &LocalValue,
) -> Option<Ordering> {
    match (comparator, left, right) {
        (
            mir::CollectionComparator::SignedInteger(_)
            | mir::CollectionComparator::UnsignedInteger(_),
            LocalValue::Scalar(mir::ScalarValue::Integer(left)),
            LocalValue::Scalar(mir::ScalarValue::Integer(right)),
        ) if left.ty == right.ty => Some(left.compare(*right)),
        (
            mir::CollectionComparator::Bool,
            LocalValue::Scalar(mir::ScalarValue::Bool(left)),
            LocalValue::Scalar(mir::ScalarValue::Bool(right)),
        ) => Some(left.cmp(right)),
        (
            mir::CollectionComparator::StringBytes,
            LocalValue::String(left),
            LocalValue::String(right),
        ) => Some(left.as_bytes().cmp(right.as_bytes())),
        _ => None,
    }
}

fn repeat_collection_entries(
    value: LocalValue,
    count: usize,
) -> Result<CollectionEntries, &'static str> {
    if count.checked_mul(core::mem::size_of::<u64>()).is_none() {
        return Err("P1313");
    }
    let mut entries = Vec::new();
    entries.try_reserve_exact(count).map_err(|_| "P1313")?;
    entries.extend(core::iter::repeat_n((None, value), count));
    Ok(entries)
}

fn display_scalar(value: mir::ScalarValue) -> String {
    match value {
        mir::ScalarValue::Integer(value) => value.display(),
        mir::ScalarValue::Bool(value) => value.to_string(),
        mir::ScalarValue::Float(value) => value.display(),
        mir::ScalarValue::Enum(_) => "<enum>".to_string(),
    }
}

fn render_format(
    format: &mir::FormatExpression,
    values: &[EvaluationValue],
) -> Result<String, InterpreterError> {
    use crate::format_string::{FormatConversion, FormatPiece};

    let mut output = String::new();
    for piece in &format.pieces {
        match piece {
            FormatPiece::Literal(value) => output.push_str(value),
            FormatPiece::Argument { index, spec } => {
                let value = values.get(*index as usize).ok_or_else(|| {
                    InterpreterError::new("MIR format argument index is out of bounds")
                })?;
                let rendered = match (spec.conversion, value) {
                    (FormatConversion::Display, EvaluationValue::Scalar(value)) => {
                        display_scalar(*value)
                    }
                    (FormatConversion::Display, EvaluationValue::String(value)) => {
                        value.to_string()
                    }
                    (
                        FormatConversion::Decimal,
                        EvaluationValue::Scalar(mir::ScalarValue::Integer(value)),
                    ) => {
                        if value.ty.is_signed() {
                            value.signed_value().to_string()
                        } else {
                            value.unsigned_value().to_string()
                        }
                    }
                    (
                        FormatConversion::HexLower
                        | FormatConversion::HexUpper
                        | FormatConversion::Octal
                        | FormatConversion::Binary,
                        EvaluationValue::Scalar(mir::ScalarValue::Integer(value)),
                    ) => format_integer_base(*value, spec.conversion),
                    (
                        FormatConversion::Float,
                        EvaluationValue::Scalar(mir::ScalarValue::Float(value)),
                    ) => format_fixed_float(*value, spec.precision.unwrap_or(6)),
                    _ => {
                        return Err(InterpreterError::new(
                            "MIR format conversion and argument type disagree",
                        ))
                    }
                };
                output.push_str(&apply_width(rendered, *spec));
            }
        }
    }
    Ok(output)
}

fn format_integer_base(
    value: IntegerValue,
    conversion: crate::format_string::FormatConversion,
) -> String {
    use crate::format_string::FormatConversion;
    let bits = value.unsigned_value();
    match conversion {
        FormatConversion::HexLower => format!("{bits:x}"),
        FormatConversion::HexUpper => format!("{bits:X}"),
        FormatConversion::Octal => format!("{bits:o}"),
        FormatConversion::Binary => format!("{bits:b}"),
        _ => unreachable!("only integer-base conversions reach this helper"),
    }
}

fn format_fixed_float(value: FloatValue, precision: u32) -> String {
    let precision = precision as usize;
    match value.ty {
        crate::numeric::FloatType::Float32 => {
            let value = value.as_f32();
            if value.is_nan() {
                "NaN".to_string()
            } else if value == f32::INFINITY {
                "Infinity".to_string()
            } else if value == f32::NEG_INFINITY {
                "-Infinity".to_string()
            } else {
                format!("{value:.precision$}")
            }
        }
        crate::numeric::FloatType::Float64 => {
            let value = value.as_f64();
            if value.is_nan() {
                "NaN".to_string()
            } else if value == f64::INFINITY {
                "Infinity".to_string()
            } else if value == f64::NEG_INFINITY {
                "-Infinity".to_string()
            } else {
                format!("{value:.precision$}")
            }
        }
    }
}

fn apply_width(mut value: String, spec: crate::format_string::FormatSpec) -> String {
    let width = spec.width.unwrap_or(0) as usize;
    if value.len() >= width {
        return value;
    }
    let padding = width - value.len();
    if spec.left_align {
        value.extend(core::iter::repeat_n(' ', padding));
        return value;
    }
    let fill = if spec.zero_pad { '0' } else { ' ' };
    if fill == '0' && value.starts_with('-') {
        let tail = value.split_off(1);
        let mut padded = String::with_capacity(width);
        padded.push('-');
        padded.extend(core::iter::repeat_n('0', padding));
        padded.push_str(&tail);
        padded
    } else {
        let mut padded = String::with_capacity(width);
        padded.extend(core::iter::repeat_n(fill, padding));
        padded.push_str(&value);
        padded
    }
}

fn read_local(
    locals: &[Option<LocalValue>],
    id: mir::LocalId,
) -> Result<&LocalValue, InterpreterError> {
    locals
        .get(id.0)
        .ok_or_else(|| InterpreterError::new(format!("MIR local local{} does not exist", id.0)))?
        .as_ref()
        .ok_or_else(|| {
            InterpreterError::new(format!(
                "MIR local local{} was read before assignment",
                id.0
            ))
        })
}

fn assign_local(
    definitions: &[mir::Local],
    locals: &mut [Option<LocalValue>],
    id: mir::LocalId,
    value: LocalValue,
) -> Result<Option<LocalValue>, InterpreterError> {
    let definition = definitions
        .get(id.0)
        .filter(|local| local.id == id)
        .ok_or_else(|| InterpreterError::new(format!("MIR local local{} does not exist", id.0)))?;
    let compatible = matches!(
        (definition.ty, &value),
        (mir::Type::Scalar(expected), LocalValue::Scalar(actual)) if expected == actual.ty()
    ) || matches!(
        (definition.ty, &value),
        (mir::Type::String, LocalValue::String(_))
            | (mir::Type::NullableString, LocalValue::NullableString(_))
            | (mir::Type::Mixed, LocalValue::Mixed(_))
            | (mir::Type::NullableMixed, LocalValue::NullableMixed(_))
    ) || matches!(
        (definition.ty, &value),
        (mir::Type::NullableScalar(expected), LocalValue::NullableScalar { ty, .. }) if expected == *ty
    ) || matches!(
        (definition.ty, &value),
        (mir::Type::Class(expected), LocalValue::Class { class, .. }) if expected == *class
    ) || matches!(
        (definition.ty, &value),
        (mir::Type::NullableClass(expected), LocalValue::NullableClass { class, .. }) if expected == *class
    ) || matches!(
        (definition.ty, &value),
        (mir::Type::SharedReference(expected), LocalValue::SharedReference { class, .. }) if expected == *class
    ) || matches!(
        (definition.ty, &value),
        (mir::Type::WeakReference(expected), LocalValue::WeakReference { class, .. }) if expected == *class
    ) || matches!(
        (definition.ty, &value),
        (mir::Type::NullableSharedReference(expected), LocalValue::NullableSharedReference { class, .. }) if expected == *class
    ) || matches!(
        (definition.ty, &value),
        (mir::Type::NullableWeakReference(expected), LocalValue::NullableWeakReference { class, .. }) if expected == *class
    ) || matches!(
        (definition.ty, &value),
        (mir::Type::WritableSharedReference(expected), LocalValue::WritableSharedReference { payload, .. }) if expected == *payload
    ) || matches!(
        (definition.ty, &value),
        (mir::Type::WritableWeakReference(expected), LocalValue::WritableWeakReference { payload, .. }) if expected == *payload
    ) || matches!(
        (definition.ty, &value),
        (mir::Type::NullableWritableSharedReference(expected), LocalValue::NullableWritableSharedReference { payload, .. }) if expected == *payload
    ) || matches!(
        (definition.ty, &value),
        (mir::Type::NullableWritableWeakReference(expected), LocalValue::NullableWritableWeakReference { payload, .. }) if expected == *payload
    ) || matches!(
        (definition.ty, &value),
        (
            mir::Type::ReadonlySharedReferenceAccess(expected),
            LocalValue::SharedReferenceAccess {
                payload,
                writable: false,
                ..
            }
        ) if expected == *payload
    ) || matches!(
        (definition.ty, &value),
        (
            mir::Type::NullableReadonlySharedReferenceAccess(expected),
            LocalValue::NullableSharedReferenceAccess {
                payload,
                writable: false,
                ..
            }
        ) if expected == *payload
    ) || matches!(
        (definition.ty, &value),
        (
            mir::Type::NullableWritableSharedReferenceAccess(expected),
            LocalValue::NullableSharedReferenceAccess {
                payload,
                writable: true,
                ..
            }
        ) if expected == *payload
    ) || matches!(
        (definition.ty, &value),
        (
            mir::Type::WritableSharedReferenceAccess(expected),
            LocalValue::SharedReferenceAccess {
                payload,
                writable: true,
                ..
            }
        ) if expected == *payload
    ) || matches!(
        (definition.ty, &value),
        (mir::Type::Collection(expected), LocalValue::Collection(collection)) if expected == collection.ty
    ) || matches!(
        (definition.ty, &value),
        (mir::Type::NullableCollection(expected), LocalValue::Collection(collection))
            if expected == collection.ty && collection.nullable
    );
    if !compatible {
        let actual = match &value {
            LocalValue::Scalar(value) => match value.ty() {
                mir::ScalarType::Integer(value) => value.source_name(),
                mir::ScalarType::Float(value) => value.source_name(),
                mir::ScalarType::Bool => "bool",
                mir::ScalarType::Enum(_) => "enum",
            },
            LocalValue::String(_) => "string",
            LocalValue::Mixed(_) => "mixed",
            LocalValue::NullableScalar { .. } => "nullable scalar",
            LocalValue::NullableString(_) => "?string",
            LocalValue::NullableMixed(_) => "?mixed",
            LocalValue::Class { .. } => "class",
            LocalValue::NullableClass { .. } => "nullable class",
            LocalValue::SharedReference { .. } => "shared reference",
            LocalValue::WeakReference { .. } => "weak reference",
            LocalValue::NullableSharedReference { .. } => "nullable shared reference",
            LocalValue::NullableWeakReference { .. } => "nullable weak reference",
            LocalValue::WritableSharedReference { .. } => "writable shared reference",
            LocalValue::WritableWeakReference { .. } => "writable weak reference",
            LocalValue::NullableWritableSharedReference { .. } => {
                "nullable writable shared reference"
            }
            LocalValue::NullableWritableWeakReference { .. } => "nullable writable weak reference",
            LocalValue::SharedReferenceAccess {
                writable: false, ..
            } => "readonly shared access",
            LocalValue::SharedReferenceAccess { writable: true, .. } => "writable shared access",
            LocalValue::NullableSharedReferenceAccess {
                writable: false, ..
            } => "nullable readonly shared access",
            LocalValue::NullableSharedReferenceAccess { writable: true, .. } => {
                "nullable writable shared access"
            }
            LocalValue::Collection(_) => "collection",
        };
        return Err(InterpreterError::new(format!(
            "MIR local local{} has type {}, but assignment produced {actual}",
            id.0, definition.ty
        )));
    }
    let slot = locals
        .get_mut(id.0)
        .ok_or_else(|| InterpreterError::new(format!("MIR local local{} does not exist", id.0)))?;
    Ok(slot.replace(value))
}

fn mixed_value_from_local(value: &LocalValue) -> Option<&MixedValue> {
    match value {
        LocalValue::Mixed(value) => Some(value),
        LocalValue::NullableMixed(Some(value)) => Some(value),
        LocalValue::NullableMixed(None) => None,
        _ => None,
    }
}

fn enum_case_in(
    program: &mir::Program,
    value: crate::enums::EnumValue,
) -> Result<&mir::EnumCaseDefinition, InterpreterError> {
    program
        .enums
        .get(value.enum_id.0)
        .filter(|definition| definition.id == value.enum_id)
        .and_then(|definition| definition.cases.get(value.case_id.index))
        .filter(|case| case.id == value.case_id)
        .ok_or_else(|| InterpreterError::new("MIR enum case does not exist"))
}

fn function_in(
    program: &mir::Program,
    id: mir::FunctionId,
) -> Result<&mir::Function, InterpreterError> {
    program
        .functions
        .get(id.0)
        .filter(|function| function.id == id)
        .ok_or_else(|| {
            InterpreterError::new(format!("MIR FunctionId function{} does not exist", id.0))
        })
}

fn class_in(
    program: &mir::Program,
    id: crate::class_layout::ClassId,
) -> Result<&mir::Class, InterpreterError> {
    program
        .classes
        .get(id.0)
        .filter(|class| class.id == id)
        .ok_or_else(|| InterpreterError::new(format!("MIR ClassId class{} does not exist", id.0)))
}

fn local_in(function: &mir::Function, id: mir::LocalId) -> Result<&mir::Local, InterpreterError> {
    function
        .locals
        .get(id.0)
        .filter(|local| local.id == id)
        .ok_or_else(|| InterpreterError::new(format!("MIR LocalId local{} does not exist", id.0)))
}

fn temporary_argument_drop_order(
    args: &[mir::Rvalue],
    callee: &mir::Function,
    parameter_offset: usize,
    promoted: impl Fn(usize) -> bool,
) -> Result<Vec<usize>, InterpreterError> {
    let mut drops = Vec::new();
    for (index, argument) in args.iter().enumerate() {
        let temporary = argument.owned_temporary_class().is_some()
            || argument.owned_temporary_collection().is_some()
            || argument.owned_temporary_shared().is_some()
            || argument.mixed_ownership() == mir::MixedOwnership::Owned;
        if !temporary || promoted(index) {
            continue;
        }
        let parameter = *callee.params.get(index + parameter_offset).ok_or_else(|| {
            InterpreterError::new(format!(
                "MIR function{} is missing parameter {}",
                callee.id.0,
                index + parameter_offset
            ))
        })?;
        if !local_in(callee, parameter)?.owned {
            drops.push(index);
        }
    }
    if drops
        .iter()
        .all(|index| args[*index].transferred_owned_local().is_some())
    {
        drops.sort_by_key(|index| {
            args[*index]
                .transferred_owned_local()
                .expect("all reordered owned temporaries have source-order locals")
                .0
        });
    }
    Ok(drops)
}

fn block_in(
    function: &mir::Function,
    id: mir::BlockId,
) -> Result<&mir::BasicBlock, InterpreterError> {
    function
        .blocks
        .get(id.0)
        .filter(|block| block.id == id)
        .ok_or_else(|| InterpreterError::new(format!("MIR BlockId block{} does not exist", id.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_strings_share_one_immutable_allocation() {
        let string = SharedString::from("shared");
        let entries =
            repeat_collection_entries(LocalValue::String(string.clone()), 3).expect("small fill");

        assert_eq!(Rc::strong_count(&string), 4);
        for (_, value) in entries {
            let LocalValue::String(value) = value else {
                panic!("string fill produced another value type");
            };
            assert!(Rc::ptr_eq(&string, &value));
        }
    }

    #[test]
    fn oversized_repeat_capacity_uses_the_catalogued_panic_identity() {
        let count = usize::MAX / core::mem::size_of::<u64>() + 1;
        let error =
            repeat_collection_entries(LocalValue::Scalar(mir::ScalarValue::Bool(false)), count)
                .expect_err("native-word capacity overflow should be rejected");

        assert_eq!(error, "P1313");
    }
}
