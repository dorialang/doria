//! Decision 0110 byte-stream contracts use ordinary nominal enums and interfaces.

use crate::ast::{Item, Program};
use crate::names::{CompilerSymbolIdentity, GlobalSymbolFacts, GlobalSymbolOwner};
use crate::source::{SourceFile, SourceId};

pub const SOURCE_ID: SourceId = SourceId(0x7fff_fffc);
pub const SOURCE_NAME: &str = "<compiler-known:streams>";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferPlanFunction {
    Read,
    AndReadFrom,
    WriteTo,
    AndWriteTo,
    GetReadKey,
    GetWriteKey,
}

impl TransferPlanFunction {
    pub fn is_read(self) -> bool {
        matches!(self, Self::Read | Self::AndReadFrom | Self::GetReadKey)
    }

    pub fn is_key_lookup(self) -> bool {
        matches!(self, Self::GetReadKey | Self::GetWriteKey)
    }

    pub fn is_appending(self) -> bool {
        matches!(self, Self::AndReadFrom | Self::AndWriteTo)
    }

    pub fn return_borrow(self) -> Option<crate::symbols::ReturnBorrow> {
        let sources = self.retained_sources();
        let [source] = sources.as_slice() else {
            return None;
        };
        Some(crate::symbols::ReturnBorrow {
            source: source.source,
            kind: crate::types::ReturnBorrowKind::Retained,
            writable: source.writable,
            lifetime: source.lifetime,
        })
    }

    pub(crate) fn retained_sources(self) -> Vec<crate::ownership::RetainedSource> {
        if self.is_key_lookup() {
            return Vec::new();
        }
        use crate::symbols::BorrowSource;
        let source = |source, writable| crate::ownership::RetainedSource {
            source,
            inherited: true,
            writable,
            lifetime: crate::types::RetainedLifetime::Cleanup,
            owned_path: Vec::new(),
        };
        let mut sources = Vec::new();
        if self.is_appending() {
            sources.push(source(BorrowSource::Receiver, false));
        }
        sources.push(source(BorrowSource::Parameter(1), self.is_read()));
        sources
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadinessFunction {
    New,
    Register,
    Unregister,
    GetStream,
    GetWritableStream,
    Begin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferGroupFunction {
    NextCompletion,
    ObservedReadKey,
    ObservedWriteKey,
    ReadRegion,
    ResubmitRead,
    ResubmitWrite,
}

impl TransferGroupFunction {
    pub fn return_borrow(self) -> Option<crate::symbols::ReturnBorrow> {
        (self == Self::ReadRegion).then_some(crate::symbols::ReturnBorrow {
            source: crate::symbols::BorrowSource::Receiver,
            writable: false,
            kind: crate::types::ReturnBorrowKind::Retained,
            lifetime: crate::types::RetainedLifetime::LastUse,
        })
    }

    pub(crate) fn retained_sources(self) -> Vec<crate::ownership::RetainedSource> {
        self.return_borrow()
            .into_iter()
            .map(|borrow| crate::ownership::RetainedSource {
                source: borrow.source,
                inherited: false,
                writable: borrow.writable,
                lifetime: borrow.lifetime,
                owned_path: Vec::new(),
            })
            .collect()
    }
}

/// The nested result types come from checked MIR, never a backend layout guess.
pub(crate) struct TransferObservationTypes {
    pub(crate) completion: crate::mir::PayloadEnumType,
    pub(crate) keys: [crate::inline_value::InlineValueType; 2],
    pub(crate) outcomes: [crate::mir::PayloadEnumType; 2],
}

impl TransferObservationTypes {
    pub(crate) fn for_result(
        program: &crate::mir::Program,
        result: crate::mir::PayloadEnumType,
    ) -> Option<Self> {
        use crate::mir::{ScalarType, Type};
        let definition = program
            .enums
            .iter()
            .find(|definition| definition.id == result.id)?;
        let Type::PayloadEnum(completion) = definition.cases.first()?.payload.first()?.ty else {
            return None;
        };
        let definition = program
            .enums
            .iter()
            .find(|definition| definition.id == completion.id)?;
        let mut keys = Vec::with_capacity(2);
        let mut outcomes = Vec::with_capacity(2);
        for case in definition.cases.iter().take(2) {
            let Type::Scalar(ScalarType::Inline(key)) = case.payload.first()?.ty else {
                return None;
            };
            let Type::NullablePayloadEnum(outcome) = case.payload.get(1)?.ty else {
                return None;
            };
            keys.push(key);
            outcomes.push(outcome);
        }
        Some(Self {
            completion,
            keys: keys.try_into().ok()?,
            outcomes: outcomes.try_into().ok()?,
        })
    }
}

impl ReadinessFunction {
    pub fn return_borrow(self) -> Option<crate::symbols::ReturnBorrow> {
        matches!(self, Self::GetStream | Self::GetWritableStream).then_some(
            crate::symbols::ReturnBorrow {
                source: crate::symbols::BorrowSource::Receiver,
                writable: self == Self::GetWritableStream,
                kind: crate::types::ReturnBorrowKind::Value,
                lifetime: crate::types::RetainedLifetime::LastUse,
            },
        )
    }

    pub fn may_block(self) -> bool {
        matches!(self, Self::Register | Self::Unregister | Self::Begin)
    }

    pub(crate) fn retained_sources(self) -> Vec<crate::ownership::RetainedSource> {
        if self != Self::Begin {
            return Vec::new();
        }
        use crate::symbols::BorrowSource;
        vec![
            crate::ownership::RetainedSource {
                source: BorrowSource::Receiver,
                inherited: false,
                writable: true,
                lifetime: crate::types::RetainedLifetime::Cleanup,
                owned_path: Vec::new(),
            },
            crate::ownership::RetainedSource {
                source: BorrowSource::Parameter(0),
                inherited: true,
                writable: false,
                lifetime: crate::types::RetainedLifetime::Cleanup,
                owned_path: Vec::new(),
            },
        ]
    }
}

/// Nominal callback capabilities follow compiler-owned declaration identity,
/// including an inherited requirement redeclared by an authored interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StreamOperation {
    Read,
    Write,
    ReadinessSource,
    PrepareRead,
    PrepareWrite,
    CompleteRead,
    CompleteWrite,
}

impl StreamOperation {
    pub fn from_requirement(span: crate::source::Span) -> Option<Self> {
        if span.source != SOURCE_ID {
            return None;
        }
        declarations().items.iter().find_map(|item| {
            let Item::Interface(interface) = item else {
                return None;
            };
            if interface.name == READINESS_STREAM
                && interface.properties.iter().any(|property| {
                    property.name == "readinessSource"
                        && property.hooks.iter().any(|hook| {
                            hook.kind == crate::ast::PropertyHookKind::Get && hook.span == span
                        })
                })
            {
                return Some(Self::ReadinessSource);
            }
            let method = interface
                .requirements
                .iter()
                .find(|method| method.span == span)?;
            match (interface.name.as_str(), method.name.as_str()) {
                ("Doria\\Std\\Io\\ReadableStream", "read") => Some(Self::Read),
                ("Doria\\Std\\Io\\WritableStream", "write") => Some(Self::Write),
                (PENDING_READABLE_STREAM, "prepareRead") => Some(Self::PrepareRead),
                (PENDING_READABLE_STREAM, "completeRead") => Some(Self::CompleteRead),
                (PENDING_WRITABLE_STREAM, "prepareWrite") => Some(Self::PrepareWrite),
                (PENDING_WRITABLE_STREAM, "completeWrite") => Some(Self::CompleteWrite),
                _ => None,
            }
        })
    }
}

/// Declaration identity, not the authored spelling, owns the execution boundary.
pub fn readiness_function(declaration: crate::source::Span) -> Option<ReadinessFunction> {
    if declaration.source != SOURCE_ID {
        return None;
    }
    declarations().items.iter().find_map(|item| {
        let Item::Class(class) = item else {
            return None;
        };
        class.members.iter().find_map(|member| {
            let crate::ast::ClassMember::Method(method) = member else {
                return None;
            };
            match method.body {
                crate::ast::FunctionBody::Intrinsic {
                    implementation: crate::ast::CompilerIntrinsic::Readiness(function),
                    ..
                } if method.span == declaration => Some(function),
                _ => None,
            }
        })
    })
}

pub fn transfer_group_function(declaration: crate::source::Span) -> Option<TransferGroupFunction> {
    if declaration.source != SOURCE_ID {
        return None;
    }
    declarations().items.iter().find_map(|item| {
        let Item::Class(class) = item else {
            return None;
        };
        class.members.iter().find_map(|member| {
            if let crate::ast::ClassMember::Method(method) = member {
                return match method.body {
                    crate::ast::FunctionBody::Intrinsic {
                        implementation: crate::ast::CompilerIntrinsic::TransferGroup(function),
                        ..
                    } if method.span == declaration => Some(function),
                    _ => None,
                };
            }
            let crate::ast::ClassMember::Property(property) = member else {
                return None;
            };
            property.hooks.iter().find_map(|hook| match hook.body {
                crate::ast::FunctionBody::Intrinsic {
                    implementation: crate::ast::CompilerIntrinsic::TransferGroup(function),
                    ..
                } if hook.span == declaration => Some(function),
                _ => None,
            })
        })
    })
}

/// Runtime operations of `Child`, a runtime-owned value (Decision 0110,
/// Child Process Declarations). It holds the child's handle and its three
/// pipe objects; the resolving methods are authored Doria over these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessFunction {
    Launch,
    /// Writable borrows of the owner's pipe objects.
    InputPipe,
    OutputPipe,
    ErrorsPipe,
    Id,
    /// Closes the input, drains both outputs concurrently keeping at most the
    /// limit of each, closes the pipes and waits. Returns the raw status.
    Finish,
    WaitUntil,
    Status,
    /// Forces termination, closes the pipes without draining, and waits.
    Terminate,
    /// Detaches: the cleanup policy no longer runs.
    Release,
    CapturedOutput,
    CapturedErrors,
    Truncated,
}

impl ProcessFunction {
    /// Waiting blocks; a launch searches the file system and forks.
    pub fn may_block(self) -> bool {
        matches!(
            self,
            Self::Launch | Self::Finish | Self::WaitUntil | Self::Terminate
        )
    }

    /// The runtime tag shared by every native backend.
    pub const fn tag(self) -> u8 {
        match self {
            Self::Launch => 0,
            Self::InputPipe => 1,
            Self::OutputPipe => 2,
            Self::ErrorsPipe => 3,
            Self::Id => 4,
            Self::Finish => 5,
            Self::WaitUntil => 6,
            Self::Status => 7,
            Self::Terminate => 8,
            Self::Release => 9,
            Self::CapturedOutput => 10,
            Self::CapturedErrors => 11,
            Self::Truncated => 12,
        }
    }

    /// The class a launch creates, or the pipe class a getter borrows.
    pub const fn result_class(self) -> Option<&'static str> {
        match self {
            Self::Launch => Some(CHILD),
            Self::InputPipe => Some(CHILD_INPUT),
            Self::OutputPipe | Self::ErrorsPipe => Some(CHILD_OUTPUT),
            _ => None,
        }
    }

    pub const fn is_pipe(self) -> bool {
        matches!(self, Self::InputPipe | Self::OutputPipe | Self::ErrorsPipe)
    }

    /// A pipe getter lends one of the owner's pipe objects for its last use.
    pub fn return_borrow(self) -> Option<crate::symbols::ReturnBorrow> {
        self.is_pipe().then_some(crate::symbols::ReturnBorrow {
            source: crate::symbols::BorrowSource::Receiver,
            writable: true,
            kind: crate::types::ReturnBorrowKind::Value,
            lifetime: crate::types::RetainedLifetime::LastUse,
        })
    }

    /// The success type, except for captured bytes, whose collection type
    /// belongs to the program: `returns_bytes` covers those.
    pub fn result_type(
        self,
        class: Option<crate::class_layout::ClassId>,
    ) -> Option<crate::mir::Type> {
        use crate::mir::{ScalarType, Type};
        match self {
            Self::Launch | Self::InputPipe | Self::OutputPipe | Self::ErrorsPipe => {
                class.map(Type::Class)
            }
            Self::Id | Self::Finish | Self::Status | Self::Terminate => Some(Type::Scalar(
                ScalarType::Integer(crate::types::IntegerType::Int64),
            )),
            Self::WaitUntil | Self::Truncated => Some(Type::Scalar(ScalarType::Bool)),
            Self::Release | Self::CapturedOutput | Self::CapturedErrors => None,
        }
    }

    pub const fn returns_bytes(self) -> bool {
        matches!(self, Self::CapturedOutput | Self::CapturedErrors)
    }

    /// Only the launch request, a collection limit and a deadline cross.
    pub const fn argument_count(self) -> usize {
        match self {
            Self::Launch => 8,
            Self::Finish | Self::WaitUntil => 1,
            _ => 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileFunction {
    StandardInput,
    StandardOutput,
    StandardError,
    OpenRead,
    OpenWrite,
    OpenReadWrite,
    Read,
    Write,
    Close,
    Seek,
    SynchronizeData,
    SynchronizeMetadata,
    SetLength,
    Position,
    Length,
    /// Waits for the whole-file lock.
    AcquireLock,
    /// Returns false on contention instead of waiting.
    TryAcquireLock,
    /// Explicit release by the guard that holds the lock.
    ReleaseLock,
    /// Best-effort release while a guard is destroyed; no-op after release.
    AbandonLock,
    /// Half-closes a child's input pipe; later writes fail as Closed.
    CloseInput,
    ReadinessSource,
    PrepareRead,
    PrepareWrite,
    CompleteRead,
    CompleteWrite,
    ReadRequestOutcome,
    WriteRequestOutcome,
    CompleteReadRequest,
    CompleteWriteRequest,
    UnavailableConstructor,
}

impl FileFunction {
    pub fn may_block(self) -> bool {
        !matches!(
            self,
            Self::UnavailableConstructor
                | Self::StandardInput
                | Self::StandardOutput
                | Self::StandardError
                | Self::ReadinessSource
                | Self::PrepareRead
                | Self::PrepareWrite
                | Self::CompleteRead
                | Self::CompleteWrite
                | Self::ReadRequestOutcome
                | Self::WriteRequestOutcome
                | Self::CompleteReadRequest
                | Self::CompleteWriteRequest
                | Self::TryAcquireLock
                | Self::ReleaseLock
                | Self::AbandonLock
        )
    }

    pub fn return_borrow(self) -> Option<crate::symbols::ReturnBorrow> {
        (self == Self::ReadinessSource).then_some(crate::symbols::ReturnBorrow {
            source: crate::symbols::BorrowSource::Receiver,
            writable: false,
            kind: crate::types::ReturnBorrowKind::Value,
            lifetime: crate::types::RetainedLifetime::LastUse,
        })
    }
}

pub const READABLE_FILE: &str = "Doria\\Std\\Io\\ReadableFile";
pub const COMMAND: &str = "Doria\\Std\\Process\\Command";
pub const CHILD: &str = "Doria\\Std\\Process\\Child";
pub const CHILD_INPUT: &str = "Doria\\Std\\Process\\ChildInput";
pub const CHILD_OUTPUT: &str = "Doria\\Std\\Process\\ChildOutput";
pub const STANDARD_STREAMS: &str = "Doria\\Std\\Io\\StandardStreams";
pub const STANDARD_INPUT: &str = "Doria\\Std\\Io\\StandardInput";
pub const STANDARD_OUTPUT: &str = "Doria\\Std\\Io\\StandardOutput";
pub const STANDARD_ERROR: &str = "Doria\\Std\\Io\\StandardError";

pub fn standard_device(name: &str) -> Option<crate::mir::StandardIoStream> {
    match name {
        STANDARD_INPUT => Some(crate::mir::StandardIoStream::Input),
        STANDARD_OUTPUT => Some(crate::mir::StandardIoStream::Output),
        STANDARD_ERROR => Some(crate::mir::StandardIoStream::Error),
        _ => None,
    }
}
pub const WRITABLE_FILE: &str = "Doria\\Std\\Io\\WritableFile";
pub const DUPLEX_FILE: &str = "Doria\\Std\\Io\\DuplexFile";
pub const READINESS_SOURCE: &str = "Doria\\Std\\Io\\ReadinessSource";
pub const REGISTRATION_KEY: &str = "Doria\\Std\\Io\\RegistrationKey";
pub const READ_TRANSFER_KEY: &str = "Doria\\Std\\Io\\ReadTransferKey";
pub const WRITE_TRANSFER_KEY: &str = "Doria\\Std\\Io\\WriteTransferKey";
pub const TRANSFER_COMPLETION: &str = "Doria\\Std\\Io\\TransferCompletion";
pub const TRANSFER_WAIT_RESULT: &str = "Doria\\Std\\Io\\TransferWaitResult";
pub const READINESS_SET: &str = "Doria\\Std\\Io\\ReadinessSet";
pub const TRANSFER_PLAN: &str = "Doria\\Std\\Io\\TransferPlan";
pub const TRANSFER_GROUP: &str = "Doria\\Std\\Io\\TransferGroup";
pub const READINESS_STREAM: &str = "Doria\\Std\\Io\\ReadinessStream";
pub const PENDING_READABLE_STREAM: &str = "Doria\\Std\\Io\\PendingReadableStream";
pub const PENDING_WRITABLE_STREAM: &str = "Doria\\Std\\Io\\PendingWritableStream";
pub const READ_REQUEST: &str = "Doria\\Std\\Io\\ReadRequest";
pub const WRITE_REQUEST: &str = "Doria\\Std\\Io\\WriteRequest";

pub fn is_pending_request(name: &str) -> bool {
    matches!(name, READ_REQUEST | WRITE_REQUEST)
}

/// A request is a loan of a group slot, never independent owned storage.
pub fn is_call_scoped_request(ty: &crate::types::ResolvedType) -> bool {
    match ty {
        crate::types::ResolvedType::Class(class) => is_pending_request(&class.name),
        crate::types::ResolvedType::Nullable(inner) => is_call_scoped_request(inner),
        _ => false,
    }
}

pub fn contains_call_scoped_request(ty: &crate::types::ResolvedType) -> bool {
    use crate::types::ResolvedType;
    match ty {
        ResolvedType::Class(class) | ResolvedType::Interface(class) => {
            is_pending_request(&class.name)
                || class.arguments.iter().any(contains_call_scoped_request)
        }
        ResolvedType::Nullable(inner)
        | ResolvedType::TypedArray(inner)
        | ResolvedType::List(inner)
        | ResolvedType::Set(inner)
        | ResolvedType::SortedSet(inner)
        | ResolvedType::PriorityQueue(inner)
        | ResolvedType::Deque(inner)
        | ResolvedType::SharedHandle(_, inner) => contains_call_scoped_request(inner),
        ResolvedType::Dictionary(key, value) | ResolvedType::SortedDictionary(key, value) => {
            contains_call_scoped_request(key) || contains_call_scoped_request(value)
        }
        // Parameter-rooted return loans preserve the request's call lifetime.
        ResolvedType::Function(function) => {
            (contains_call_scoped_request(&function.return_type)
                && !(is_call_scoped_request(&function.return_type)
                    && function.return_borrow.is_some()))
                || function.parameters.iter().any(|parameter| {
                    parameter.ownership_mode == crate::types::FunctionTypeParameterMode::Take
                        && contains_call_scoped_request(&parameter.ty)
                })
        }
        _ => false,
    }
}

pub fn is_pending_phase(interface: &str, method: &str) -> bool {
    matches!(
        (interface, method),
        (PENDING_READABLE_STREAM, "prepareRead" | "completeRead")
            | (PENDING_WRITABLE_STREAM, "prepareWrite" | "completeWrite")
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Readable,
    Writable,
    Duplex,
}

impl FileKind {
    pub fn for_name(name: &str) -> Option<Self> {
        match name {
            READABLE_FILE | CHILD_OUTPUT => Some(Self::Readable),
            WRITABLE_FILE | CHILD_INPUT => Some(Self::Writable),
            DUPLEX_FILE => Some(Self::Duplex),
            _ => None,
        }
    }

    pub fn access_tag(self) -> u8 {
        match self {
            Self::Readable => 0,
            Self::Writable => 1,
            Self::Duplex => 2,
        }
    }
}

pub fn owns_native_file(name: &str) -> bool {
    FileKind::for_name(name).is_some()
}

/// The parent ends of a child's pipes: file owners without a path.
pub fn is_child_pipe(name: &str) -> bool {
    matches!(name, CHILD_INPUT | CHILD_OUTPUT)
}

/// Closing a pipe end never blocks; a child process's cleanup policy may wait.
pub fn cleanup_may_block(name: &str) -> bool {
    (owns_native_file(name) && !is_child_pipe(name)) || matches!(name, TRANSFER_GROUP | CHILD)
}

pub const CANONICAL_TYPES: [&str; 56] = [
    STANDARD_STREAMS,
    STANDARD_INPUT,
    STANDARD_OUTPUT,
    STANDARD_ERROR,
    "Doria\\Std\\Io\\Wait",
    "Doria\\Std\\Io\\ReadOutcome",
    "Doria\\Std\\Io\\WriteOutcome",
    "Doria\\Std\\Io\\FlushOutcome",
    "Doria\\Std\\Io\\FinishOutcome",
    "Doria\\Std\\Io\\BorrowedFinishOutcome",
    "Doria\\Std\\Io\\BufferedReader",
    "Doria\\Std\\Io\\BorrowedBufferedReader",
    "Doria\\Std\\Io\\BufferedWriter",
    "Doria\\Std\\Io\\BorrowedBufferedWriter",
    "Doria\\Std\\Io\\LineOutcome",
    "Doria\\Std\\Io\\LineReader",
    "Doria\\Std\\Io\\BorrowedLineReader",
    "Doria\\Std\\Io\\FileLockMode",
    "Doria\\Std\\Io\\LockOutcome",
    "Doria\\Std\\Io\\BorrowedLockOutcome",
    "Doria\\Std\\Io\\FileLock",
    "Doria\\Std\\Io\\LockedFile",
    "Doria\\Std\\Io\\ReadableStream",
    "Doria\\Std\\Io\\WritableStream",
    "Doria\\Std\\Io\\DuplexStream",
    "Doria\\Std\\Io\\FlushableStream",
    "Doria\\Std\\Io\\SeekableStream",
    "Doria\\Std\\Io\\SeekOrigin",
    "Doria\\Std\\Io\\FileCreation",
    "Doria\\Std\\Io\\FileWriteMode",
    "Doria\\Std\\Io\\File",
    READABLE_FILE,
    WRITABLE_FILE,
    DUPLEX_FILE,
    READINESS_SOURCE,
    REGISTRATION_KEY,
    READ_TRANSFER_KEY,
    WRITE_TRANSFER_KEY,
    TRANSFER_COMPLETION,
    TRANSFER_WAIT_RESULT,
    READINESS_SET,
    TRANSFER_PLAN,
    TRANSFER_GROUP,
    READINESS_STREAM,
    PENDING_READABLE_STREAM,
    PENDING_WRITABLE_STREAM,
    READ_REQUEST,
    WRITE_REQUEST,
    "Doria\\Std\\Process\\ChildCleanup",
    "Doria\\Std\\Process\\ExitStatus",
    "Doria\\Std\\Process\\WaitOutcome",
    COMMAND,
    CHILD,
    CHILD_INPUT,
    CHILD_OUTPUT,
    "Doria\\Std\\Process\\CollectedOutput",
];

/// Standard classes written in Doria: their bodies compile like user code on
/// every backend instead of binding runtime intrinsics.
const AUTHORED_CLASSES: [&str; 10] = [
    "BufferedReader",
    "BorrowedBufferedReader",
    "BufferedWriter",
    "BorrowedBufferedWriter",
    "LineReader",
    "BorrowedLineReader",
    "FileLock",
    "LockedFile",
    "Command",
    "CollectedOutput",
];

/// Declarations of this source that live in `Doria\Std\Process`. The child
/// pipes are file owners, so the process family shares the stream source,
/// its intrinsics and its std-wide `internal` access.
const PROCESS_DECLARATIONS: [&str; 8] = [
    "ChildCleanup",
    "ExitStatus",
    "WaitOutcome",
    "Command",
    "ChildInput",
    "ChildOutput",
    "Child",
    "CollectedOutput",
];

/// Methods of intrinsic classes written in Doria, and intrinsic methods of
/// authored classes. Lock guards reach their file through these primitives;
/// the file's public lock methods construct the guards in Doria.
fn authored_member(class: &str, member: &crate::ast::ClassMember) -> bool {
    let crate::ast::ClassMember::Method(method) = member else {
        return AUTHORED_CLASSES.contains(&class);
    };
    match class {
        "FileLock" | "LockedFile" => !matches!(method.name.as_str(), "releaseFile" | "abandonFile"),
        "ReadableFile" | "WritableFile" | "DuplexFile" => matches!(
            method.name.as_str(),
            "lock" | "tryLock" | "lockOwned" | "tryLockOwned"
        ),
        "Child" => matches!(
            method.name.as_str(),
            "waitUntil" | "wait" | "terminate" | "detach" | "collect" | "status"
        ),
        _ => AUTHORED_CLASSES.contains(&class),
    }
}

pub const SOURCE_TEXT: &str = r#"
enum Wait {
    case Immediate;
    case For(Doria\Std\Time\Duration $duration);
    case Until(Doria\Std\Time\Deadline $deadline);
    case Indefinite;
}
enum FileCreation {
    case OpenExisting;
    case OpenOrCreate;
    case CreateNew;
    case TruncateExisting;
    case CreateOrTruncate;
}
enum FileWriteMode {
    case Positioned;
    case Append;
}
enum SeekOrigin {
    case Start;
    case Current;
    case End;
}
enum ReadOutcome {
    case Data(int $byteCount);
    case WouldBlock;
    case EndOfStream;
    case TimedOut;
}
enum WriteOutcome {
    case Accepted(int $byteCount);
    case WouldBlock;
}
enum FlushOutcome {
    case Complete;
    case Pending;
}
enum FinishOutcome<S, R> {
    case Complete(S $source);
    case Pending(R $writer);
}
enum BorrowedFinishOutcome<R> {
    case Complete;
    case Pending(R $writer);
}
enum LineOutcome {
    case Line(string $text);
    case WouldBlock;
    case EndOfStream;
    case TimedOut;
    case TooLong;
}
class LineReader<S implements Doria\Std\Io\ReadableStream> {
    internal writable S $source;

    function __construct(parameter take S $source, parameter int $capacity) {
        if ($capacity < 1) {
            panic("a line reader capacity must be at least 1 byte; pass the longest line length you accept");
        }
        $this->source = $source;
        $this->buffer = Bytes::allocate($capacity);
    }
    internal writable Bytes $buffer;
    internal writable int $start = 0;
    internal writable int $end = 0;
    internal writable int $scanned = 0;
    internal writable int $consumed = 0;
    internal writable bool $ended = false;

    writable function readLine(): Doria\Std\Io\LineOutcome throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error {
        while (true) {
            while ($this->scanned < $this->end) {
                if ($this->buffer[$this->scanned] == 10) {
                    let $lineEnd = $this->scanned;
                    return $this->takeLine($lineEnd, true);
                }
                $this->scanned++;
            }
            if ($this->ended) {
                if ($this->start == $this->end) {
                    return Doria\Std\Io\LineOutcome::EndOfStream;
                }
                let $lineEnd = $this->end;
                return $this->takeLine($lineEnd, false);
            }
            if ($this->start > 0) {
                $this->compact();
            }
            if ($this->end == $this->buffer->length) {
                return Doria\Std\Io\LineOutcome::TooLong;
            }
            let $offset = $this->end;
            let $length = $this->buffer->length - $offset;
            let writable $free = $this->buffer->getWritableRegion($offset, $length);
            let $filled = match ($this->source->read($free)) {
                Doria\Std\Io\ReadOutcome::Data($count) => $count,
                Doria\Std\Io\ReadOutcome::EndOfStream => -1,
                Doria\Std\Io\ReadOutcome::WouldBlock => -2,
                Doria\Std\Io\ReadOutcome::TimedOut => -3,
            };
            if ($filled == -2) {
                return Doria\Std\Io\LineOutcome::WouldBlock;
            }
            if ($filled == -3) {
                return Doria\Std\Io\LineOutcome::TimedOut;
            }
            if ($filled == -1) {
                $this->ended = true;
            } else {
                $this->end += $filled;
            }
        }
    }

    consuming function finish(): S {
        return take $this->source;
    }

    internal writable function takeLine(int $lineEnd, bool $terminated): Doria\Std\Io\LineOutcome throws Doria\Std\Io\InvalidUtf8Error {
        let writable $length = $lineEnd - $this->start;
        if ($terminated && $length > 0 && $this->buffer[$lineEnd - 1] == 13) {
            $length--;
        }
        let writable $line = Bytes::allocate($length);
        $line->getWritableRegion(0, $length)->copyFrom($this->buffer->getRegion($this->start, $length));
        // One validation pass for valid input; only invalid input pays for the
        // scan that locates the error, before any reader state changes.
        let $text = String::fromBytes($line);
        if ($text == null) {
            $this->checkUtf8($lineEnd, $terminated);
            panic("a line that failed UTF-8 conversion passed the line validator; report this compiler defect");
        }
        let $next = $terminated ? $lineEnd + 1 : $lineEnd;
        $this->consumed += $next - $this->start;
        $this->start = $next;
        $this->scanned = $next;
        return Doria\Std\Io\LineOutcome::Line($text);
    }

    internal function checkUtf8(int $lineEnd, bool $terminated): void throws Doria\Std\Io\InvalidUtf8Error {
        let writable $index = $this->start;
        while ($index < $lineEnd) {
            let $lead = Int::from($this->buffer[$index]);
            let writable $needed = 0;
            let writable $low = 128;
            let writable $high = 191;
            if ($lead < 128) {
                $index++;
                continue;
            } else if ($lead >= 194 && $lead <= 223) {
                $needed = 1;
            } else if ($lead >= 224 && $lead <= 239) {
                $needed = 2;
                if ($lead == 224) {
                    $low = 160;
                } else if ($lead == 237) {
                    $high = 159;
                }
            } else if ($lead >= 240 && $lead <= 244) {
                $needed = 3;
                if ($lead == 240) {
                    $low = 144;
                } else if ($lead == 244) {
                    $high = 143;
                }
            } else {
                throw $this->invalidUtf8($index, 1);
            }
            let writable $seen = 1;
            while ($seen <= $needed) {
                if ($index + $seen >= $lineEnd) {
                    // The line terminator ends the sequence; only the true end
                    // of the stream leaves it incomplete.
                    if ($terminated) {
                        throw $this->invalidUtf8($index, $seen);
                    }
                    throw new Doria\Std\Io\InvalidUtf8Error("invalid UTF-8 in stream", Doria\Std\Io\Utf8InputSource::Stream, $this->consumed + $index - $this->start, null);
                }
                let $next = Int::from($this->buffer[$index + $seen]);
                if ($next < $low || $next > $high) {
                    throw $this->invalidUtf8($index, $seen);
                }
                $low = 128;
                $high = 191;
                $seen++;
            }
            $index += $needed + 1;
        }
    }

    internal function invalidUtf8(int $index, int $invalidByteCount): Doria\Std\Io\InvalidUtf8Error {
        return new Doria\Std\Io\InvalidUtf8Error("invalid UTF-8 in stream", Doria\Std\Io\Utf8InputSource::Stream, $this->consumed + $index - $this->start, $invalidByteCount);
    }

    internal writable function compact(): void {
        let $count = $this->end - $this->start;
        let $from = $this->start;
        let $capacity = $this->buffer->length;
        $this->buffer->getWritableRegion(0, $capacity)->copyWithin($from, $count, 0);
        $this->scanned -= $this->start;
        $this->start = 0;
        $this->end = $count;
    }
}
class BorrowedLineReader<S implements Doria\Std\Io\ReadableStream> {
    function __construct(borrow writable S $source, parameter int $capacity) {
        if ($capacity < 1) {
            panic("a line reader capacity must be at least 1 byte; pass the longest line length you accept");
        }
        $this->buffer = Bytes::allocate($capacity);
    }
    internal writable Bytes $buffer;
    internal writable int $start = 0;
    internal writable int $end = 0;
    internal writable int $scanned = 0;
    internal writable int $consumed = 0;
    internal writable bool $ended = false;

    writable function readLine(): Doria\Std\Io\LineOutcome throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error {
        while (true) {
            while ($this->scanned < $this->end) {
                if ($this->buffer[$this->scanned] == 10) {
                    let $lineEnd = $this->scanned;
                    return $this->takeLine($lineEnd, true);
                }
                $this->scanned++;
            }
            if ($this->ended) {
                if ($this->start == $this->end) {
                    return Doria\Std\Io\LineOutcome::EndOfStream;
                }
                let $lineEnd = $this->end;
                return $this->takeLine($lineEnd, false);
            }
            if ($this->start > 0) {
                $this->compact();
            }
            if ($this->end == $this->buffer->length) {
                return Doria\Std\Io\LineOutcome::TooLong;
            }
            let $offset = $this->end;
            let $length = $this->buffer->length - $offset;
            let writable $free = $this->buffer->getWritableRegion($offset, $length);
            let $filled = match ($this->source->read($free)) {
                Doria\Std\Io\ReadOutcome::Data($count) => $count,
                Doria\Std\Io\ReadOutcome::EndOfStream => -1,
                Doria\Std\Io\ReadOutcome::WouldBlock => -2,
                Doria\Std\Io\ReadOutcome::TimedOut => -3,
            };
            if ($filled == -2) {
                return Doria\Std\Io\LineOutcome::WouldBlock;
            }
            if ($filled == -3) {
                return Doria\Std\Io\LineOutcome::TimedOut;
            }
            if ($filled == -1) {
                $this->ended = true;
            } else {
                $this->end += $filled;
            }
        }
    }

    internal writable function takeLine(int $lineEnd, bool $terminated): Doria\Std\Io\LineOutcome throws Doria\Std\Io\InvalidUtf8Error {
        let writable $length = $lineEnd - $this->start;
        if ($terminated && $length > 0 && $this->buffer[$lineEnd - 1] == 13) {
            $length--;
        }
        let writable $line = Bytes::allocate($length);
        $line->getWritableRegion(0, $length)->copyFrom($this->buffer->getRegion($this->start, $length));
        // One validation pass for valid input; only invalid input pays for the
        // scan that locates the error, before any reader state changes.
        let $text = String::fromBytes($line);
        if ($text == null) {
            $this->checkUtf8($lineEnd, $terminated);
            panic("a line that failed UTF-8 conversion passed the line validator; report this compiler defect");
        }
        let $next = $terminated ? $lineEnd + 1 : $lineEnd;
        $this->consumed += $next - $this->start;
        $this->start = $next;
        $this->scanned = $next;
        return Doria\Std\Io\LineOutcome::Line($text);
    }

    internal function checkUtf8(int $lineEnd, bool $terminated): void throws Doria\Std\Io\InvalidUtf8Error {
        let writable $index = $this->start;
        while ($index < $lineEnd) {
            let $lead = Int::from($this->buffer[$index]);
            let writable $needed = 0;
            let writable $low = 128;
            let writable $high = 191;
            if ($lead < 128) {
                $index++;
                continue;
            } else if ($lead >= 194 && $lead <= 223) {
                $needed = 1;
            } else if ($lead >= 224 && $lead <= 239) {
                $needed = 2;
                if ($lead == 224) {
                    $low = 160;
                } else if ($lead == 237) {
                    $high = 159;
                }
            } else if ($lead >= 240 && $lead <= 244) {
                $needed = 3;
                if ($lead == 240) {
                    $low = 144;
                } else if ($lead == 244) {
                    $high = 143;
                }
            } else {
                throw $this->invalidUtf8($index, 1);
            }
            let writable $seen = 1;
            while ($seen <= $needed) {
                if ($index + $seen >= $lineEnd) {
                    // The line terminator ends the sequence; only the true end
                    // of the stream leaves it incomplete.
                    if ($terminated) {
                        throw $this->invalidUtf8($index, $seen);
                    }
                    throw new Doria\Std\Io\InvalidUtf8Error("invalid UTF-8 in stream", Doria\Std\Io\Utf8InputSource::Stream, $this->consumed + $index - $this->start, null);
                }
                let $next = Int::from($this->buffer[$index + $seen]);
                if ($next < $low || $next > $high) {
                    throw $this->invalidUtf8($index, $seen);
                }
                $low = 128;
                $high = 191;
                $seen++;
            }
            $index += $needed + 1;
        }
    }

    internal function invalidUtf8(int $index, int $invalidByteCount): Doria\Std\Io\InvalidUtf8Error {
        return new Doria\Std\Io\InvalidUtf8Error("invalid UTF-8 in stream", Doria\Std\Io\Utf8InputSource::Stream, $this->consumed + $index - $this->start, $invalidByteCount);
    }

    internal writable function compact(): void {
        let $count = $this->end - $this->start;
        let $from = $this->start;
        let $capacity = $this->buffer->length;
        $this->buffer->getWritableRegion(0, $capacity)->copyWithin($from, $count, 0);
        $this->scanned -= $this->start;
        $this->start = 0;
        $this->end = $count;
    }
}
enum FileLockMode {
    case Shared;
    case Exclusive;
}
enum LockOutcome<G, F> {
    case Acquired(G $guard);
    case Contended(F $file);
}
enum BorrowedLockOutcome<G> {
    case Acquired(G $guard);
    case Contended;
}
class FileLock<F> {
    internal function __construct(borrow writable F $file) {}

    consuming function unlock(): void throws Doria\Std\Io\IoError {
        $this->releaseFile($this->file);
    }

    function __destruct(): void {
        try {
            $this->abandonFile($this->file);
        } catch (Doria\Std\Io\IoError) {}
    }

    internal function releaseFile(F $file): void throws Doria\Std\Io\IoError {}
    internal function abandonFile(F $file): void throws Doria\Std\Io\IoError {}
}
class LockedFile<F> {
    internal writable F $held;

    internal function __construct(parameter take F $file) {
        $this->held = $file;
    }

    F $file { writable borrowed get => $this->held; }

    consuming function unlock(): F throws Doria\Std\Io\IoError {
        $this->releaseFile($this->held);
        return take $this->held;
    }

    function __destruct(): void {
        try {
            $this->abandonFile($this->held);
        } catch (Doria\Std\Io\IoError) {}
    }

    internal function releaseFile(F $file): void throws Doria\Std\Io\IoError {}
    internal function abandonFile(F $file): void throws Doria\Std\Io\IoError {}
}
class BufferedReader<S implements Doria\Std\Io\ReadableStream> implements Doria\Std\Io\ReadableStream,
    Doria\Std\Io\ReadinessStream where (S implements Doria\Std\Io\ReadinessStream),
    Doria\Std\Io\PendingReadableStream where (S implements Doria\Std\Io\PendingReadableStream) {
    internal writable S $source;

    function __construct(parameter take S $source, parameter int $capacity) {
        if ($capacity < 1) {
            panic("a buffered adapter capacity must be at least 1 byte; pass a positive capacity");
        }
        $this->source = $source;
        $this->buffer = Bytes::allocate($capacity);
    }
    internal writable Bytes $buffer;
    internal writable int $start = 0;
    internal writable int $end = 0;

    writable function read(writable Doria\Std\Io\WritableByteRegion $destination): Doria\Std\Io\ReadOutcome throws Doria\Std\Io\IoError {
        if ($destination->length == 0) {
            return Doria\Std\Io\ReadOutcome::Data(0);
        }
        if ($this->start == $this->end) {
            if ($destination->length >= $this->buffer->length) {
                return $this->source->read($destination);
            }
            let $filled = $this->fill();
            if ($this->start == $this->end) {
                return $filled;
            }
        }
        let writable $count = $this->end - $this->start;
        if ($count > $destination->length) {
            $count = $destination->length;
        }
        $destination->copyFrom($this->buffer->getRegion($this->start, $count));
        $this->start += $count;
        return Doria\Std\Io\ReadOutcome::Data($count);
    }

    writable function fill(): Doria\Std\Io\ReadOutcome throws Doria\Std\Io\IoError {
        if ($this->start == $this->end) {
            $this->start = 0;
            $this->end = 0;
        }
        if ($this->end == $this->buffer->length) {
            return Doria\Std\Io\ReadOutcome::Data(0);
        }
        let $offset = $this->end;
        let $length = $this->buffer->length - $offset;
        let writable $free = $this->buffer->getWritableRegion($offset, $length);
        let $outcome = $this->source->read($free);
        $this->end += match ($outcome) { Doria\Std\Io\ReadOutcome::Data($count) => $count, default => 0 };
        return $outcome;
    }

    Doria\Std\Io\ByteRegion $buffered { borrowed get => $this->buffer->getRegion($this->start, $this->end - $this->start); }

    Doria\Std\Io\ReadinessSource $readinessSource where (S implements Doria\Std\Io\ReadinessStream) {
        borrowed get => $this->source->readinessSource;
    }

    // Pending transfers outstanding on this reader: a positive count is
    // immediate outcomes served from buffered bytes, a negative count is
    // transfers passed through to the source. Preparation never mixes the two,
    // so completion knows each request's path whatever order completions run
    // in. A pass-through transfer starts only once the buffer is empty, so
    // waiting on the source's readiness never hides buffered bytes.
    internal writable int $pendingReads = 0;

    writable function prepareRead(writable Doria\Std\Io\ReadRequest $request, writable Doria\Std\Io\WritableByteRegion $destination): void throws Doria\Std\Io\IoError
        where (S implements Doria\Std\Io\PendingReadableStream) {
        if ($this->pendingReads < 0 || ($this->pendingReads == 0 && $this->start == $this->end && $destination->length > 0)) {
            $this->source->prepareRead($request, $destination);
            $this->pendingReads--;
            return;
        }
        let writable $count = $this->end - $this->start;
        if ($count > $destination->length) {
            $count = $destination->length;
        }
        if ($count == 0 && $destination->length > 0) {
            $request->complete(Doria\Std\Io\ReadOutcome::WouldBlock);
        } else {
            $destination->copyFrom($this->buffer->getRegion($this->start, $count));
            $this->start += $count;
            $request->complete(Doria\Std\Io\ReadOutcome::Data($count));
        }
        $this->pendingReads++;
    }

    writable function completeRead(Doria\Std\Io\ReadRequest $request, writable Doria\Std\Io\WritableByteRegion $destination): ?Doria\Std\Io\ReadOutcome throws Doria\Std\Io\IoError
        where (S implements Doria\Std\Io\PendingReadableStream) {
        if ($this->pendingReads > 0) {
            $this->pendingReads--;
            return $request->outcome;
        }
        $this->pendingReads++;
        return $this->source->completeRead($request, $destination);
    }

    writable function consume(int $byteCount): void throws Doria\Std\Io\IoError {
        if ($byteCount < 0 || $byteCount > $this->end - $this->start) {
            panic("buffered reader consume count is outside the buffered bytes; consume at most `$reader->buffered->length` bytes");
        }
        $this->start += $byteCount;
    }

    consuming function finish(): S {
        return take $this->source;
    }
}
class BorrowedBufferedReader<S implements Doria\Std\Io\ReadableStream> implements Doria\Std\Io\ReadableStream,
    Doria\Std\Io\ReadinessStream where (S implements Doria\Std\Io\ReadinessStream),
    Doria\Std\Io\PendingReadableStream where (S implements Doria\Std\Io\PendingReadableStream) {
    function __construct(borrow writable S $source, parameter int $capacity) {
        if ($capacity < 1) {
            panic("a buffered adapter capacity must be at least 1 byte; pass a positive capacity");
        }
        $this->buffer = Bytes::allocate($capacity);
    }
    internal writable Bytes $buffer;
    internal writable int $start = 0;
    internal writable int $end = 0;

    writable function read(writable Doria\Std\Io\WritableByteRegion $destination): Doria\Std\Io\ReadOutcome throws Doria\Std\Io\IoError {
        if ($destination->length == 0) {
            return Doria\Std\Io\ReadOutcome::Data(0);
        }
        if ($this->start == $this->end) {
            if ($destination->length >= $this->buffer->length) {
                return $this->source->read($destination);
            }
            let $filled = $this->fill();
            if ($this->start == $this->end) {
                return $filled;
            }
        }
        let writable $count = $this->end - $this->start;
        if ($count > $destination->length) {
            $count = $destination->length;
        }
        $destination->copyFrom($this->buffer->getRegion($this->start, $count));
        $this->start += $count;
        return Doria\Std\Io\ReadOutcome::Data($count);
    }

    writable function fill(): Doria\Std\Io\ReadOutcome throws Doria\Std\Io\IoError {
        if ($this->start == $this->end) {
            $this->start = 0;
            $this->end = 0;
        }
        if ($this->end == $this->buffer->length) {
            return Doria\Std\Io\ReadOutcome::Data(0);
        }
        let $offset = $this->end;
        let $length = $this->buffer->length - $offset;
        let writable $free = $this->buffer->getWritableRegion($offset, $length);
        let $outcome = $this->source->read($free);
        $this->end += match ($outcome) { Doria\Std\Io\ReadOutcome::Data($count) => $count, default => 0 };
        return $outcome;
    }

    Doria\Std\Io\ByteRegion $buffered { borrowed get => $this->buffer->getRegion($this->start, $this->end - $this->start); }

    Doria\Std\Io\ReadinessSource $readinessSource where (S implements Doria\Std\Io\ReadinessStream) {
        borrowed get => $this->source->readinessSource;
    }

    // Pending transfers outstanding on this reader: a positive count is
    // immediate outcomes served from buffered bytes, a negative count is
    // transfers passed through to the source. Preparation never mixes the two,
    // so completion knows each request's path whatever order completions run
    // in. A pass-through transfer starts only once the buffer is empty, so
    // waiting on the source's readiness never hides buffered bytes.
    internal writable int $pendingReads = 0;

    writable function prepareRead(writable Doria\Std\Io\ReadRequest $request, writable Doria\Std\Io\WritableByteRegion $destination): void throws Doria\Std\Io\IoError
        where (S implements Doria\Std\Io\PendingReadableStream) {
        if ($this->pendingReads < 0 || ($this->pendingReads == 0 && $this->start == $this->end && $destination->length > 0)) {
            $this->source->prepareRead($request, $destination);
            $this->pendingReads--;
            return;
        }
        let writable $count = $this->end - $this->start;
        if ($count > $destination->length) {
            $count = $destination->length;
        }
        if ($count == 0 && $destination->length > 0) {
            $request->complete(Doria\Std\Io\ReadOutcome::WouldBlock);
        } else {
            $destination->copyFrom($this->buffer->getRegion($this->start, $count));
            $this->start += $count;
            $request->complete(Doria\Std\Io\ReadOutcome::Data($count));
        }
        $this->pendingReads++;
    }

    writable function completeRead(Doria\Std\Io\ReadRequest $request, writable Doria\Std\Io\WritableByteRegion $destination): ?Doria\Std\Io\ReadOutcome throws Doria\Std\Io\IoError
        where (S implements Doria\Std\Io\PendingReadableStream) {
        if ($this->pendingReads > 0) {
            $this->pendingReads--;
            return $request->outcome;
        }
        $this->pendingReads++;
        return $this->source->completeRead($request, $destination);
    }

    writable function consume(int $byteCount): void throws Doria\Std\Io\IoError {
        if ($byteCount < 0 || $byteCount > $this->end - $this->start) {
            panic("buffered reader consume count is outside the buffered bytes; consume at most `$reader->buffered->length` bytes");
        }
        $this->start += $byteCount;
    }

}
class BufferedWriter<S implements Doria\Std\Io\WritableStream> implements Doria\Std\Io\WritableStream, Doria\Std\Io\FlushableStream,
    Doria\Std\Io\ReadinessStream where (S implements Doria\Std\Io\ReadinessStream),
    Doria\Std\Io\PendingWritableStream where (S implements Doria\Std\Io\PendingWritableStream) {
    internal writable S $source;

    function __construct(parameter take S $source, parameter int $capacity) {
        if ($capacity < 1) {
            panic("a buffered adapter capacity must be at least 1 byte; pass a positive capacity");
        }
        $this->source = $source;
        $this->buffer = Bytes::allocate($capacity);
    }
    internal writable Bytes $buffer;
    internal writable int $start = 0;
    internal writable int $end = 0;

    writable function write(Doria\Std\Io\ByteRegion $source): Doria\Std\Io\WriteOutcome throws Doria\Std\Io\IoError {
        if ($source->length == 0) {
            return Doria\Std\Io\WriteOutcome::Accepted(0);
        }
        if ($this->end == $this->buffer->length) {
            let $drained = $this->drain();
            $this->compact();
            if ($this->end == $this->buffer->length) {
                return Doria\Std\Io\WriteOutcome::WouldBlock;
            }
        }
        if ($this->start == $this->end && $source->length >= $this->buffer->length) {
            return $this->source->write($source);
        }
        return $this->accept($source);
    }

    Doria\Std\Io\ReadinessSource $readinessSource where (S implements Doria\Std\Io\ReadinessStream) {
        borrowed get => $this->source->readinessSource;
    }

    // The one pending write outstanding on this writer: 0 none, 1 an immediate
    // outcome, 2 the caller's bytes passed through to an empty-buffered sink,
    // 3 a drain of the full buffer. A drain lends the whole buffer to one
    // transfer, so a second concurrent write on one writer is rejected.
    internal writable int $pendingWrite = 0;

    writable function prepareWrite(writable Doria\Std\Io\WriteRequest $request, Doria\Std\Io\ByteRegion $source): void throws Doria\Std\Io\IoError
        where (S implements Doria\Std\Io\PendingWritableStream) {
        if ($this->pendingWrite != 0) {
            throw new Doria\Std\Io\IoError("a buffered writer carries one pending write at a time; plan one write per buffered writer and resubmit it after its completion", Doria\Std\Io\IoOperation::Configure, Doria\Std\Io\IoTarget::TransferGroup, Doria\Std\Io\IoErrorReason::InvalidInput, null, null);
        }
        if ($this->start == $this->end && $source->length >= $this->buffer->length) {
            $this->source->prepareWrite($request, $source);
            $this->pendingWrite = 2;
            return;
        }
        if ($this->end == $this->buffer->length && $source->length > 0) {
            $this->compact();
            if ($this->end == $this->buffer->length) {
                let $capacity = $this->buffer->length;
                $this->source->prepareWrite($request, $this->buffer->getRegion(0, $capacity));
                $this->pendingWrite = 3;
                return;
            }
        }
        $request->complete($this->accept($source));
        $this->pendingWrite = 1;
    }

    writable function completeWrite(Doria\Std\Io\WriteRequest $request, Doria\Std\Io\ByteRegion $source): ?Doria\Std\Io\WriteOutcome throws Doria\Std\Io\IoError
        where (S implements Doria\Std\Io\PendingWritableStream) {
        let $mode = $this->pendingWrite;
        $this->pendingWrite = 0;
        if ($mode == 1) {
            return $request->outcome;
        }
        if ($mode == 2) {
            return $this->source->completeWrite($request, $source);
        }
        let $capacity = $this->buffer->length;
        let $drained = $this->source->completeWrite($request, $this->buffer->getRegion(0, $capacity));
        if ($drained == null) {
            return null;
        }
        $this->start += match ($drained) { Doria\Std\Io\WriteOutcome::Accepted($count) => $count, default => 0 };
        $this->compact();
        return $this->accept($source);
    }

    // Copies what fits of `$source` into the free tail of the buffer.
    internal writable function accept(Doria\Std\Io\ByteRegion $source): Doria\Std\Io\WriteOutcome {
        let writable $count = $this->buffer->length - $this->end;
        if ($count > $source->length) {
            $count = $source->length;
        }
        if ($count == 0 && $source->length > 0) {
            return Doria\Std\Io\WriteOutcome::WouldBlock;
        }
        $this->buffer->getWritableRegion($this->end, $count)->copyFrom($source);
        $this->end += $count;
        return Doria\Std\Io\WriteOutcome::Accepted($count);
    }

    writable function flush(): Doria\Std\Io\FlushOutcome throws Doria\Std\Io\IoError {
        return $this->drain();
    }

    consuming function finish(): Doria\Std\Io\FinishOutcome<S, Doria\Std\Io\BufferedWriter<S>> throws Doria\Std\Io\IoError {
        let writable $writer = $this;
        let $drained = $writer->drain();
        if ($drained == Doria\Std\Io\FlushOutcome::Pending) {
            return Doria\Std\Io\FinishOutcome::Pending($writer);
        }
        return $writer->complete();
    }

    internal consuming function complete(): Doria\Std\Io\FinishOutcome<S, Doria\Std\Io\BufferedWriter<S>> {
        return Doria\Std\Io\FinishOutcome::Complete(take $this->source);
    }

    // Dropping the writer flushes what it can; failures cannot escape a
    // destructor, so call flush or finish to observe them (Rust's BufWriter).
    // The wrapped stream may raise either ambient I/O Error.
    function __destruct(): void {
        try {
            $this->drain();
        } catch (Doria\Std\Io\IoError) {
        } catch (Doria\Std\Io\InvalidUtf8Error) {}
    }

    internal writable function drain(): Doria\Std\Io\FlushOutcome throws Doria\Std\Io\IoError {
        while ($this->start < $this->end) {
            let $offset = $this->start;
            let $length = $this->end - $offset;
            let $pending = $this->buffer->getRegion($offset, $length);
            let $accepted = match ($this->source->write($pending)) {
                Doria\Std\Io\WriteOutcome::Accepted($count) => $count,
                Doria\Std\Io\WriteOutcome::WouldBlock => 0,
            };
            if ($accepted == 0) {
                return Doria\Std\Io\FlushOutcome::Pending;
            }
            $this->start += $accepted;
        }
        $this->start = 0;
        $this->end = 0;
        return Doria\Std\Io\FlushOutcome::Complete;
    }

    internal writable function compact(): void {
        if ($this->start == 0) {
            return;
        }
        let $length = $this->end - $this->start;
        let $from = $this->start;
        let $capacity = $this->buffer->length;
        $this->buffer->getWritableRegion(0, $capacity)->copyWithin($from, $length, 0);
        $this->start = 0;
        $this->end = $length;
    }
}
class BorrowedBufferedWriter<S implements Doria\Std\Io\WritableStream> implements Doria\Std\Io\WritableStream, Doria\Std\Io\FlushableStream,
    Doria\Std\Io\ReadinessStream where (S implements Doria\Std\Io\ReadinessStream),
    Doria\Std\Io\PendingWritableStream where (S implements Doria\Std\Io\PendingWritableStream) {
    function __construct(borrow writable S $source, parameter int $capacity) {
        if ($capacity < 1) {
            panic("a buffered adapter capacity must be at least 1 byte; pass a positive capacity");
        }
        $this->buffer = Bytes::allocate($capacity);
    }
    internal writable Bytes $buffer;
    internal writable int $start = 0;
    internal writable int $end = 0;

    writable function write(Doria\Std\Io\ByteRegion $source): Doria\Std\Io\WriteOutcome throws Doria\Std\Io\IoError {
        if ($source->length == 0) {
            return Doria\Std\Io\WriteOutcome::Accepted(0);
        }
        if ($this->end == $this->buffer->length) {
            let $drained = $this->drain();
            $this->compact();
            if ($this->end == $this->buffer->length) {
                return Doria\Std\Io\WriteOutcome::WouldBlock;
            }
        }
        if ($this->start == $this->end && $source->length >= $this->buffer->length) {
            return $this->source->write($source);
        }
        return $this->accept($source);
    }

    Doria\Std\Io\ReadinessSource $readinessSource where (S implements Doria\Std\Io\ReadinessStream) {
        borrowed get => $this->source->readinessSource;
    }

    // The one pending write outstanding on this writer: 0 none, 1 an immediate
    // outcome, 2 the caller's bytes passed through to an empty-buffered sink,
    // 3 a drain of the full buffer. A drain lends the whole buffer to one
    // transfer, so a second concurrent write on one writer is rejected.
    internal writable int $pendingWrite = 0;

    writable function prepareWrite(writable Doria\Std\Io\WriteRequest $request, Doria\Std\Io\ByteRegion $source): void throws Doria\Std\Io\IoError
        where (S implements Doria\Std\Io\PendingWritableStream) {
        if ($this->pendingWrite != 0) {
            throw new Doria\Std\Io\IoError("a buffered writer carries one pending write at a time; plan one write per buffered writer and resubmit it after its completion", Doria\Std\Io\IoOperation::Configure, Doria\Std\Io\IoTarget::TransferGroup, Doria\Std\Io\IoErrorReason::InvalidInput, null, null);
        }
        if ($this->start == $this->end && $source->length >= $this->buffer->length) {
            $this->source->prepareWrite($request, $source);
            $this->pendingWrite = 2;
            return;
        }
        if ($this->end == $this->buffer->length && $source->length > 0) {
            $this->compact();
            if ($this->end == $this->buffer->length) {
                let $capacity = $this->buffer->length;
                $this->source->prepareWrite($request, $this->buffer->getRegion(0, $capacity));
                $this->pendingWrite = 3;
                return;
            }
        }
        $request->complete($this->accept($source));
        $this->pendingWrite = 1;
    }

    writable function completeWrite(Doria\Std\Io\WriteRequest $request, Doria\Std\Io\ByteRegion $source): ?Doria\Std\Io\WriteOutcome throws Doria\Std\Io\IoError
        where (S implements Doria\Std\Io\PendingWritableStream) {
        let $mode = $this->pendingWrite;
        $this->pendingWrite = 0;
        if ($mode == 1) {
            return $request->outcome;
        }
        if ($mode == 2) {
            return $this->source->completeWrite($request, $source);
        }
        let $capacity = $this->buffer->length;
        let $drained = $this->source->completeWrite($request, $this->buffer->getRegion(0, $capacity));
        if ($drained == null) {
            return null;
        }
        $this->start += match ($drained) { Doria\Std\Io\WriteOutcome::Accepted($count) => $count, default => 0 };
        $this->compact();
        return $this->accept($source);
    }

    // Copies what fits of `$source` into the free tail of the buffer.
    internal writable function accept(Doria\Std\Io\ByteRegion $source): Doria\Std\Io\WriteOutcome {
        let writable $count = $this->buffer->length - $this->end;
        if ($count > $source->length) {
            $count = $source->length;
        }
        if ($count == 0 && $source->length > 0) {
            return Doria\Std\Io\WriteOutcome::WouldBlock;
        }
        $this->buffer->getWritableRegion($this->end, $count)->copyFrom($source);
        $this->end += $count;
        return Doria\Std\Io\WriteOutcome::Accepted($count);
    }

    writable function flush(): Doria\Std\Io\FlushOutcome throws Doria\Std\Io\IoError {
        return $this->drain();
    }

    consuming function finish(): Doria\Std\Io\BorrowedFinishOutcome<Doria\Std\Io\BorrowedBufferedWriter<S>> throws Doria\Std\Io\IoError {
        let writable $writer = $this;
        let $drained = $writer->drain();
        if ($drained == Doria\Std\Io\FlushOutcome::Pending) {
            return Doria\Std\Io\BorrowedFinishOutcome::Pending($writer);
        }
        return $writer->complete();
    }

    internal consuming function complete(): Doria\Std\Io\BorrowedFinishOutcome<Doria\Std\Io\BorrowedBufferedWriter<S>> {
        return Doria\Std\Io\BorrowedFinishOutcome::Complete;
    }

    // Dropping the writer flushes what it can; failures cannot escape a
    // destructor, so call flush or finish to observe them (Rust's BufWriter).
    // The wrapped stream may raise either ambient I/O Error.
    function __destruct(): void {
        try {
            $this->drain();
        } catch (Doria\Std\Io\IoError) {
        } catch (Doria\Std\Io\InvalidUtf8Error) {}
    }

    internal writable function drain(): Doria\Std\Io\FlushOutcome throws Doria\Std\Io\IoError {
        while ($this->start < $this->end) {
            let $offset = $this->start;
            let $length = $this->end - $offset;
            let $pending = $this->buffer->getRegion($offset, $length);
            let $accepted = match ($this->source->write($pending)) {
                Doria\Std\Io\WriteOutcome::Accepted($count) => $count,
                Doria\Std\Io\WriteOutcome::WouldBlock => 0,
            };
            if ($accepted == 0) {
                return Doria\Std\Io\FlushOutcome::Pending;
            }
            $this->start += $accepted;
        }
        $this->start = 0;
        $this->end = 0;
        return Doria\Std\Io\FlushOutcome::Complete;
    }

    internal writable function compact(): void {
        if ($this->start == 0) {
            return;
        }
        let $length = $this->end - $this->start;
        let $from = $this->start;
        let $capacity = $this->buffer->length;
        $this->buffer->getWritableRegion(0, $capacity)->copyWithin($from, $length, 0);
        $this->start = 0;
        $this->end = $length;
    }
}
enum TransferCompletion {
    case Read(Doria\Std\Io\ReadTransferKey $key, ?Doria\Std\Io\ReadOutcome $outcome);
    case Write(Doria\Std\Io\WriteTransferKey $key, ?Doria\Std\Io\WriteOutcome $outcome);
}
enum TransferWaitResult {
    case Completed(Doria\Std\Io\TransferCompletion $completion);
    case TimedOut;
    case Exhausted;
}
interface ReadableStream {
    writable function read(writable Doria\Std\Io\WritableByteRegion $destination): Doria\Std\Io\ReadOutcome throws Doria\Std\Io\IoError;
}
interface WritableStream {
    writable function write(Doria\Std\Io\ByteRegion $source): Doria\Std\Io\WriteOutcome throws Doria\Std\Io\IoError;
}
interface DuplexStream extends Doria\Std\Io\ReadableStream, Doria\Std\Io\WritableStream {}
interface FlushableStream {
    writable function flush(): Doria\Std\Io\FlushOutcome throws Doria\Std\Io\IoError;
}
interface SeekableStream {
    writable function seek(int $offset, Doria\Std\Io\SeekOrigin $origin): int throws Doria\Std\Io\IoError;
}
class ReadinessSource {
    internal function __construct() {}
}
class RegistrationKey<T> {
    internal function __construct() {}
}
class ReadTransferKey {
    internal function __construct() {}
}
class WriteTransferKey {
    internal function __construct() {}
}
class ReadinessSet {
    function __construct() throws Doria\Std\Io\IoError {}
    writable function register<T implements Doria\Std\Io\ReadinessStream>(take T $stream): Doria\Std\Io\RegistrationKey<T> throws Doria\Std\Io\IoError {}
    writable function unregister<T>(Doria\Std\Io\RegistrationKey<T> $key): T throws Doria\Std\Io\IoError {}
    function getStream<T>(Doria\Std\Io\RegistrationKey<T> $key): T throws Doria\Std\Io\IoError {}
    writable function getWritableStream<T>(Doria\Std\Io\RegistrationKey<T> $key): T throws Doria\Std\Io\IoError {}
    writable function begin(take Doria\Std\Io\TransferPlan $plan,
        ?Doria\Std\Time\Deadline $deadline = null,
        ?Doria\Std\Concurrency\CancellationToken $cancellation = null): Doria\Std\Io\TransferGroup throws Doria\Std\Io\IoError {}
}
class TransferGroup {
    internal function __construct() {}
    writable function getNextCompletion(Doria\Std\Io\Wait $wait): Doria\Std\Io\TransferWaitResult throws Doria\Std\Io\IoError {}
    ?Doria\Std\Io\ReadTransferKey $observedReadKey { get {} }
    ?Doria\Std\Io\WriteTransferKey $observedWriteKey { get {} }
    function getReadRegion(Doria\Std\Io\ReadTransferKey $key): ?Doria\Std\Io\ByteRegion throws Doria\Std\Io\IoError {}
    writable function resubmitRead(Doria\Std\Io\ReadTransferKey $key, int $offset, int $length): void throws Doria\Std\Io\IoError {}
    writable function resubmitWrite(Doria\Std\Io\WriteTransferKey $key, int $offset, int $length): void throws Doria\Std\Io\IoError {}
}
class TransferPlan {
    internal function __construct() {}
    static function read<T implements Doria\Std\Io\PendingReadableStream>(Doria\Std\Io\RegistrationKey<T> $key, writable Doria\Std\Io\WritableByteRegion $destination): self throws Doria\Std\Io\IoError {}
    consuming function andReadFrom<T implements Doria\Std\Io\PendingReadableStream>(Doria\Std\Io\RegistrationKey<T> $key, writable Doria\Std\Io\WritableByteRegion $destination): self throws Doria\Std\Io\IoError {}
    static function writeTo<T implements Doria\Std\Io\PendingWritableStream>(Doria\Std\Io\RegistrationKey<T> $key, Doria\Std\Io\ByteRegion $source): self throws Doria\Std\Io\IoError {}
    consuming function andWriteTo<T implements Doria\Std\Io\PendingWritableStream>(Doria\Std\Io\RegistrationKey<T> $key, Doria\Std\Io\ByteRegion $source): self throws Doria\Std\Io\IoError {}
    function getReadKey(int $position): Doria\Std\Io\ReadTransferKey throws Doria\Std\Io\IoError {}
    function getWriteKey(int $position): Doria\Std\Io\WriteTransferKey throws Doria\Std\Io\IoError {}
}
interface ReadinessStream {
    Doria\Std\Io\ReadinessSource $readinessSource { borrowed get; }
}
class ReadRequest {
    internal function __construct() {}
    ?Doria\Std\Io\ReadOutcome $outcome { get throws Doria\Std\Io\IoError {} }
    writable function complete(Doria\Std\Io\ReadOutcome $outcome): void {}
}
class WriteRequest {
    internal function __construct() {}
    ?Doria\Std\Io\WriteOutcome $outcome { get throws Doria\Std\Io\IoError {} }
    writable function complete(Doria\Std\Io\WriteOutcome $outcome): void {}
}
interface PendingReadableStream extends Doria\Std\Io\ReadableStream, Doria\Std\Io\ReadinessStream {
    writable function prepareRead(writable Doria\Std\Io\ReadRequest $request, writable Doria\Std\Io\WritableByteRegion $destination): void throws Doria\Std\Io\IoError;
    writable function completeRead(Doria\Std\Io\ReadRequest $request, writable Doria\Std\Io\WritableByteRegion $destination): ?Doria\Std\Io\ReadOutcome throws Doria\Std\Io\IoError;
}
interface PendingWritableStream extends Doria\Std\Io\WritableStream, Doria\Std\Io\ReadinessStream {
    writable function prepareWrite(writable Doria\Std\Io\WriteRequest $request, Doria\Std\Io\ByteRegion $source): void throws Doria\Std\Io\IoError;
    writable function completeWrite(Doria\Std\Io\WriteRequest $request, Doria\Std\Io\ByteRegion $source): ?Doria\Std\Io\WriteOutcome throws Doria\Std\Io\IoError;
}
class StandardStreams {
    internal function __construct() {}
    static Doria\Std\Io\StandardInput $input { get {} }
    static Doria\Std\Io\StandardOutput $output { get {} }
    static Doria\Std\Io\StandardError $error { get {} }
}
class StandardInput implements Doria\Std\Io\PendingReadableStream {
    internal function __construct() {}
    writable function read(writable Doria\Std\Io\WritableByteRegion $destination): Doria\Std\Io\ReadOutcome throws Doria\Std\Io\IoError {}
    writable function prepareRead(writable Doria\Std\Io\ReadRequest $request, writable Doria\Std\Io\WritableByteRegion $destination): void throws Doria\Std\Io\IoError {}
    writable function completeRead(Doria\Std\Io\ReadRequest $request, writable Doria\Std\Io\WritableByteRegion $destination): ?Doria\Std\Io\ReadOutcome throws Doria\Std\Io\IoError {}
    Doria\Std\Io\ReadinessSource $readinessSource { borrowed get {} }
}
class StandardOutput implements Doria\Std\Io\PendingWritableStream {
    internal function __construct() {}
    writable function write(Doria\Std\Io\ByteRegion $source): Doria\Std\Io\WriteOutcome throws Doria\Std\Io\IoError {}
    writable function prepareWrite(writable Doria\Std\Io\WriteRequest $request, Doria\Std\Io\ByteRegion $source): void throws Doria\Std\Io\IoError {}
    writable function completeWrite(Doria\Std\Io\WriteRequest $request, Doria\Std\Io\ByteRegion $source): ?Doria\Std\Io\WriteOutcome throws Doria\Std\Io\IoError {}
    Doria\Std\Io\ReadinessSource $readinessSource { borrowed get {} }
}
class StandardError implements Doria\Std\Io\PendingWritableStream {
    internal function __construct() {}
    writable function write(Doria\Std\Io\ByteRegion $source): Doria\Std\Io\WriteOutcome throws Doria\Std\Io\IoError {}
    writable function prepareWrite(writable Doria\Std\Io\WriteRequest $request, Doria\Std\Io\ByteRegion $source): void throws Doria\Std\Io\IoError {}
    writable function completeWrite(Doria\Std\Io\WriteRequest $request, Doria\Std\Io\ByteRegion $source): ?Doria\Std\Io\WriteOutcome throws Doria\Std\Io\IoError {}
    Doria\Std\Io\ReadinessSource $readinessSource { borrowed get {} }
}
class File {
    internal function __construct() {}
    static function openRead(string $path): Doria\Std\Io\ReadableFile throws Doria\Std\Io\IoError {}
    static function openWrite(string $path, Doria\Std\Io\FileCreation $creation, Doria\Std\Io\FileWriteMode $mode): Doria\Std\Io\WritableFile throws Doria\Std\Io\IoError {}
    static function openReadWrite(string $path, Doria\Std\Io\FileCreation $creation, Doria\Std\Io\FileWriteMode $mode): Doria\Std\Io\DuplexFile throws Doria\Std\Io\IoError {}
}
class ReadableFile implements Doria\Std\Io\PendingReadableStream, Doria\Std\Io\SeekableStream {
    internal function __construct() {}
    writable function read(writable Doria\Std\Io\WritableByteRegion $destination): Doria\Std\Io\ReadOutcome throws Doria\Std\Io\IoError {}
    writable function prepareRead(writable Doria\Std\Io\ReadRequest $request, writable Doria\Std\Io\WritableByteRegion $destination): void throws Doria\Std\Io\IoError {}
    writable function completeRead(Doria\Std\Io\ReadRequest $request, writable Doria\Std\Io\WritableByteRegion $destination): ?Doria\Std\Io\ReadOutcome throws Doria\Std\Io\IoError {}
    consuming function close(): void throws Doria\Std\Io\IoError {}
    writable function seek(int $offset, Doria\Std\Io\SeekOrigin $origin): int throws Doria\Std\Io\IoError {}
    function getPosition(): int throws Doria\Std\Io\IoError {}
    function getLength(): int throws Doria\Std\Io\IoError {}
    writable function lock(Doria\Std\Io\FileLockMode $mode): Doria\Std\Io\FileLock<Doria\Std\Io\ReadableFile> throws Doria\Std\Io\IoError {
        $this->acquireLock($mode);
        return new Doria\Std\Io\FileLock<Doria\Std\Io\ReadableFile>($this);
    }
    writable function tryLock(Doria\Std\Io\FileLockMode $mode): Doria\Std\Io\BorrowedLockOutcome<Doria\Std\Io\FileLock<Doria\Std\Io\ReadableFile>> throws Doria\Std\Io\IoError {
        if ($this->tryAcquireLock($mode)) {
            return Doria\Std\Io\BorrowedLockOutcome::Acquired(new Doria\Std\Io\FileLock<Doria\Std\Io\ReadableFile>($this));
        }
        return Doria\Std\Io\BorrowedLockOutcome::Contended;
    }
    consuming function lockOwned(Doria\Std\Io\FileLockMode $mode): Doria\Std\Io\LockedFile<Doria\Std\Io\ReadableFile> throws Doria\Std\Io\IoError {
        let writable $file = $this;
        $file->acquireLock($mode);
        return new Doria\Std\Io\LockedFile<Doria\Std\Io\ReadableFile>($file);
    }
    consuming function tryLockOwned(Doria\Std\Io\FileLockMode $mode): Doria\Std\Io\LockOutcome<Doria\Std\Io\LockedFile<Doria\Std\Io\ReadableFile>, Doria\Std\Io\ReadableFile> throws Doria\Std\Io\IoError {
        let writable $file = $this;
        if ($file->tryAcquireLock($mode)) {
            return Doria\Std\Io\LockOutcome::Acquired(new Doria\Std\Io\LockedFile<Doria\Std\Io\ReadableFile>($file));
        }
        return Doria\Std\Io\LockOutcome::Contended($file);
    }
    internal writable function acquireLock(Doria\Std\Io\FileLockMode $mode): void throws Doria\Std\Io\IoError {}
    internal writable function tryAcquireLock(Doria\Std\Io\FileLockMode $mode): bool throws Doria\Std\Io\IoError {}
    Doria\Std\Io\ReadinessSource $readinessSource { borrowed get {} }
}
class WritableFile implements Doria\Std\Io\PendingWritableStream, Doria\Std\Io\SeekableStream {
    internal function __construct() {}
    writable function write(Doria\Std\Io\ByteRegion $source): Doria\Std\Io\WriteOutcome throws Doria\Std\Io\IoError {}
    writable function prepareWrite(writable Doria\Std\Io\WriteRequest $request, Doria\Std\Io\ByteRegion $source): void throws Doria\Std\Io\IoError {}
    writable function completeWrite(Doria\Std\Io\WriteRequest $request, Doria\Std\Io\ByteRegion $source): ?Doria\Std\Io\WriteOutcome throws Doria\Std\Io\IoError {}
    consuming function close(): void throws Doria\Std\Io\IoError {}
    writable function seek(int $offset, Doria\Std\Io\SeekOrigin $origin): int throws Doria\Std\Io\IoError {}
    writable function synchronizeData(): void throws Doria\Std\Io\IoError {}
    writable function synchronizeMetadata(): void throws Doria\Std\Io\IoError {}
    writable function setLength(int $length): void throws Doria\Std\Io\IoError {}
    function getPosition(): int throws Doria\Std\Io\IoError {}
    function getLength(): int throws Doria\Std\Io\IoError {}
    writable function lock(Doria\Std\Io\FileLockMode $mode): Doria\Std\Io\FileLock<Doria\Std\Io\WritableFile> throws Doria\Std\Io\IoError {
        $this->acquireLock($mode);
        return new Doria\Std\Io\FileLock<Doria\Std\Io\WritableFile>($this);
    }
    writable function tryLock(Doria\Std\Io\FileLockMode $mode): Doria\Std\Io\BorrowedLockOutcome<Doria\Std\Io\FileLock<Doria\Std\Io\WritableFile>> throws Doria\Std\Io\IoError {
        if ($this->tryAcquireLock($mode)) {
            return Doria\Std\Io\BorrowedLockOutcome::Acquired(new Doria\Std\Io\FileLock<Doria\Std\Io\WritableFile>($this));
        }
        return Doria\Std\Io\BorrowedLockOutcome::Contended;
    }
    consuming function lockOwned(Doria\Std\Io\FileLockMode $mode): Doria\Std\Io\LockedFile<Doria\Std\Io\WritableFile> throws Doria\Std\Io\IoError {
        let writable $file = $this;
        $file->acquireLock($mode);
        return new Doria\Std\Io\LockedFile<Doria\Std\Io\WritableFile>($file);
    }
    consuming function tryLockOwned(Doria\Std\Io\FileLockMode $mode): Doria\Std\Io\LockOutcome<Doria\Std\Io\LockedFile<Doria\Std\Io\WritableFile>, Doria\Std\Io\WritableFile> throws Doria\Std\Io\IoError {
        let writable $file = $this;
        if ($file->tryAcquireLock($mode)) {
            return Doria\Std\Io\LockOutcome::Acquired(new Doria\Std\Io\LockedFile<Doria\Std\Io\WritableFile>($file));
        }
        return Doria\Std\Io\LockOutcome::Contended($file);
    }
    internal writable function acquireLock(Doria\Std\Io\FileLockMode $mode): void throws Doria\Std\Io\IoError {}
    internal writable function tryAcquireLock(Doria\Std\Io\FileLockMode $mode): bool throws Doria\Std\Io\IoError {}
    Doria\Std\Io\ReadinessSource $readinessSource { borrowed get {} }
}
class DuplexFile implements Doria\Std\Io\DuplexStream, Doria\Std\Io\PendingReadableStream, Doria\Std\Io\PendingWritableStream, Doria\Std\Io\SeekableStream {
    internal function __construct() {}
    writable function read(writable Doria\Std\Io\WritableByteRegion $destination): Doria\Std\Io\ReadOutcome throws Doria\Std\Io\IoError {}
    writable function write(Doria\Std\Io\ByteRegion $source): Doria\Std\Io\WriteOutcome throws Doria\Std\Io\IoError {}
    writable function prepareRead(writable Doria\Std\Io\ReadRequest $request, writable Doria\Std\Io\WritableByteRegion $destination): void throws Doria\Std\Io\IoError {}
    writable function completeRead(Doria\Std\Io\ReadRequest $request, writable Doria\Std\Io\WritableByteRegion $destination): ?Doria\Std\Io\ReadOutcome throws Doria\Std\Io\IoError {}
    writable function prepareWrite(writable Doria\Std\Io\WriteRequest $request, Doria\Std\Io\ByteRegion $source): void throws Doria\Std\Io\IoError {}
    writable function completeWrite(Doria\Std\Io\WriteRequest $request, Doria\Std\Io\ByteRegion $source): ?Doria\Std\Io\WriteOutcome throws Doria\Std\Io\IoError {}
    consuming function close(): void throws Doria\Std\Io\IoError {}
    writable function seek(int $offset, Doria\Std\Io\SeekOrigin $origin): int throws Doria\Std\Io\IoError {}
    writable function synchronizeData(): void throws Doria\Std\Io\IoError {}
    writable function synchronizeMetadata(): void throws Doria\Std\Io\IoError {}
    writable function setLength(int $length): void throws Doria\Std\Io\IoError {}
    function getPosition(): int throws Doria\Std\Io\IoError {}
    function getLength(): int throws Doria\Std\Io\IoError {}
    writable function lock(Doria\Std\Io\FileLockMode $mode): Doria\Std\Io\FileLock<Doria\Std\Io\DuplexFile> throws Doria\Std\Io\IoError {
        $this->acquireLock($mode);
        return new Doria\Std\Io\FileLock<Doria\Std\Io\DuplexFile>($this);
    }
    writable function tryLock(Doria\Std\Io\FileLockMode $mode): Doria\Std\Io\BorrowedLockOutcome<Doria\Std\Io\FileLock<Doria\Std\Io\DuplexFile>> throws Doria\Std\Io\IoError {
        if ($this->tryAcquireLock($mode)) {
            return Doria\Std\Io\BorrowedLockOutcome::Acquired(new Doria\Std\Io\FileLock<Doria\Std\Io\DuplexFile>($this));
        }
        return Doria\Std\Io\BorrowedLockOutcome::Contended;
    }
    consuming function lockOwned(Doria\Std\Io\FileLockMode $mode): Doria\Std\Io\LockedFile<Doria\Std\Io\DuplexFile> throws Doria\Std\Io\IoError {
        let writable $file = $this;
        $file->acquireLock($mode);
        return new Doria\Std\Io\LockedFile<Doria\Std\Io\DuplexFile>($file);
    }
    consuming function tryLockOwned(Doria\Std\Io\FileLockMode $mode): Doria\Std\Io\LockOutcome<Doria\Std\Io\LockedFile<Doria\Std\Io\DuplexFile>, Doria\Std\Io\DuplexFile> throws Doria\Std\Io\IoError {
        let writable $file = $this;
        if ($file->tryAcquireLock($mode)) {
            return Doria\Std\Io\LockOutcome::Acquired(new Doria\Std\Io\LockedFile<Doria\Std\Io\DuplexFile>($file));
        }
        return Doria\Std\Io\LockOutcome::Contended($file);
    }
    internal writable function acquireLock(Doria\Std\Io\FileLockMode $mode): void throws Doria\Std\Io\IoError {}
    internal writable function tryAcquireLock(Doria\Std\Io\FileLockMode $mode): bool throws Doria\Std\Io\IoError {}
    Doria\Std\Io\ReadinessSource $readinessSource { borrowed get {} }
}
enum ChildCleanup {
    case Wait;
    case Terminate;
    case Detach;
}
enum ExitStatus {
    case Exited(int $code);
    case Signaled(int $signal);
}
enum WaitOutcome {
    case Exited(Doria\Std\Process\ExitStatus $status);
    case TimedOut;
}
class Command {
    internal string $program;
    internal bool $throughShell;
    internal writable List<string> $arguments = [];
    internal writable List<string> $variableNames = [];
    internal writable List<string> $variableValues = [];
    internal writable List<string> $removedVariables = [];
    internal writable ?string $workingDirectory = null;

    internal function __construct(parameter string $program, parameter bool $throughShell) {
        $this->program = $program;
        $this->throughShell = $throughShell;
    }

    static function executable(string $path): Doria\Std\Process\Command {
        return new Doria\Std\Process\Command($path, false);
    }

    static function shell(string $script): Doria\Std\Process\Command {
        return new Doria\Std\Process\Command($script, true);
    }

    writable function argument(string $value): void {
        $this->arguments->add($value);
    }

    writable function environment(string $name, string $value): void {
        $this->forget($name);
        $this->variableNames->add($name);
        $this->variableValues->add($value);
    }

    writable function removeEnvironment(string $name): void {
        $this->forget($name);
        $this->removedVariables->add($name);
    }

    writable function directory(string $path): void {
        $this->workingDirectory = $path;
    }

    function spawn(Doria\Std\Process\ChildCleanup $cleanup): Doria\Std\Process\Child throws Doria\Std\Io\IoError {
        return Doria\Std\Process\Child::launch($this->program, $this->throughShell,
            $this->arguments, $this->variableNames, $this->variableValues,
            $this->removedVariables, $this->workingDirectory, $cleanup);
    }

    // The last edit of a variable wins: drop its earlier set or removal.
    internal writable function forget(string $name): void {
        let writable $index = 0;
        while ($index < $this->variableNames->count) {
            if ($this->variableNames[$index] == $name) {
                $this->variableNames->removeAt($index);
                $this->variableValues->removeAt($index);
            } else {
                $index++;
            }
        }
        $this->removedVariables->remove($name);
    }
}
class ChildInput implements Doria\Std\Io\WritableStream, Doria\Std\Io\PendingWritableStream {
    internal function __construct() {}
    writable function write(Doria\Std\Io\ByteRegion $source): Doria\Std\Io\WriteOutcome throws Doria\Std\Io\IoError {}
    writable function prepareWrite(writable Doria\Std\Io\WriteRequest $request, Doria\Std\Io\ByteRegion $source): void throws Doria\Std\Io\IoError {}
    writable function completeWrite(Doria\Std\Io\WriteRequest $request, Doria\Std\Io\ByteRegion $source): ?Doria\Std\Io\WriteOutcome throws Doria\Std\Io\IoError {}
    Doria\Std\Io\ReadinessSource $readinessSource { borrowed get {} }
    writable function closeInput(): void throws Doria\Std\Io\IoError {}
}
class ChildOutput implements Doria\Std\Io\ReadableStream, Doria\Std\Io\PendingReadableStream {
    internal function __construct() {}
    writable function read(writable Doria\Std\Io\WritableByteRegion $destination): Doria\Std\Io\ReadOutcome throws Doria\Std\Io\IoError {}
    writable function prepareRead(writable Doria\Std\Io\ReadRequest $request, writable Doria\Std\Io\WritableByteRegion $destination): void throws Doria\Std\Io\IoError {}
    writable function completeRead(Doria\Std\Io\ReadRequest $request, writable Doria\Std\Io\WritableByteRegion $destination): ?Doria\Std\Io\ReadOutcome throws Doria\Std\Io\IoError {}
    Doria\Std\Io\ReadinessSource $readinessSource { borrowed get {} }
}
class Child {
    internal function __construct() {}
    // The runtime owner holds the child's handle and its three pipes. It
    // closes the pipes before applying the cleanup policy, so a Wait policy
    // cannot deadlock on a full pipe.
    internal static function launch(string $program, bool $shell, List<string> $arguments,
        List<string> $variableNames, List<string> $variableValues, List<string> $removedVariables,
        ?string $directory, Doria\Std\Process\ChildCleanup $cleanup): Doria\Std\Process\Child throws Doria\Std\Io\IoError {}
    writable function getInput(): Doria\Std\Process\ChildInput {}
    writable function getOutput(): Doria\Std\Process\ChildOutput {}
    writable function getErrors(): Doria\Std\Process\ChildOutput {}
    function getId(): int {}

    writable function waitUntil(Doria\Std\Time\Deadline $deadline): Doria\Std\Process\WaitOutcome throws Doria\Std\Io\IoError {
        if ($this->waitExitUntil($deadline)) {
            return Doria\Std\Process\WaitOutcome::Exited(Doria\Std\Process\Child::status($this->exitStatus()));
        }
        return Doria\Std\Process\WaitOutcome::TimedOut;
    }

    // Closes the input, drains and discards both outputs concurrently, then waits.
    consuming function wait(): Doria\Std\Process\ExitStatus throws Doria\Std\Io\IoError {
        let writable $child = $this;
        return Doria\Std\Process\Child::status($child->finish(0));
    }

    consuming function terminate(): Doria\Std\Process\ExitStatus throws Doria\Std\Io\IoError {
        let writable $child = $this;
        return Doria\Std\Process\Child::status($child->terminateExit());
    }

    consuming function detach(): void {
        let writable $child = $this;
        $child->release();
    }

    consuming function collect(int $limit): Doria\Std\Process\CollectedOutput throws Doria\Std\Io\IoError {
        if ($limit < 0) {
            panic("a collection limit must not be negative; pass the most bytes to keep from each stream");
        }
        let writable $child = $this;
        let $status = Doria\Std\Process\Child::status($child->finish($limit));
        let $output = $child->takeCapturedOutput();
        let $errors = $child->takeCapturedErrors();
        return new Doria\Std\Process\CollectedOutput($status, $output, $errors, $child->captureTruncated());
    }

    internal static function status(int $raw): Doria\Std\Process\ExitStatus {
        if ($raw >= 0) {
            return Doria\Std\Process\ExitStatus::Exited($raw);
        }
        return Doria\Std\Process\ExitStatus::Signaled(-$raw);
    }

    internal writable function finish(int $limit): int throws Doria\Std\Io\IoError {}
    internal writable function waitExitUntil(Doria\Std\Time\Deadline $deadline): bool throws Doria\Std\Io\IoError {}
    internal function exitStatus(): int {}
    internal writable function terminateExit(): int throws Doria\Std\Io\IoError {}
    internal writable function release(): void {}
    internal writable function takeCapturedOutput(): Bytes {}
    internal writable function takeCapturedErrors(): Bytes {}
    internal function captureTruncated(): bool {}
}
class CollectedOutput {
    internal Doria\Std\Process\ExitStatus $exitStatus;
    internal Bytes $outputBytes;
    internal Bytes $errorsBytes;
    internal bool $wasTruncated;

    internal function __construct(parameter Doria\Std\Process\ExitStatus $status,
        parameter take Bytes $output, parameter take Bytes $errors, parameter bool $truncated) {
        $this->exitStatus = $status;
        $this->outputBytes = $output;
        $this->errorsBytes = $errors;
        $this->wasTruncated = $truncated;
    }

    Doria\Std\Process\ExitStatus $status { get => $this->exitStatus; }
    Bytes $output { borrowed get => $this->outputBytes; }
    Bytes $errors { borrowed get => $this->errorsBytes; }
    bool $truncated { get => $this->wasTruncated; }
}
"#;

pub fn declarations() -> &'static Program {
    static DECLARATIONS: std::sync::LazyLock<Program> = std::sync::LazyLock::new(|| {
        let mut program =
            crate::parse_source_file(&SourceFile::with_id(SOURCE_ID, SOURCE_NAME, SOURCE_TEXT))
                .expect("compiler-known stream declarations parse");
        for item in &mut program.items {
            if let Item::Class(declaration) = item {
                let class_name = declaration.name.clone();
                for member in declaration
                    .members
                    .iter_mut()
                    .filter(|member| !authored_member(&class_name, member))
                {
                    if let crate::ast::ClassMember::Property(property) = member {
                        for hook in &mut property.hooks {
                            if declaration.name == "TransferGroup" {
                                hook.body = crate::ast::FunctionBody::Intrinsic {
                                    implementation: crate::ast::CompilerIntrinsic::TransferGroup(
                                        match property.name.as_str() {
                                            "observedReadKey" => {
                                                TransferGroupFunction::ObservedReadKey
                                            }
                                            "observedWriteKey" => {
                                                TransferGroupFunction::ObservedWriteKey
                                            }
                                            _ => unreachable!("compiler-known group property"),
                                        },
                                    ),
                                    span: hook.body.span(),
                                };
                                continue;
                            }
                            hook.body = crate::ast::FunctionBody::Intrinsic {
                                implementation: crate::ast::CompilerIntrinsic::File(
                                    match (declaration.name.as_str(), property.name.as_str()) {
                                        ("StandardStreams", "input") => FileFunction::StandardInput,
                                        ("StandardStreams", "output") => {
                                            FileFunction::StandardOutput
                                        }
                                        ("StandardStreams", "error") => FileFunction::StandardError,
                                        ("ReadRequest", _) => FileFunction::ReadRequestOutcome,
                                        ("WriteRequest", _) => FileFunction::WriteRequestOutcome,
                                        (_, "readinessSource") => FileFunction::ReadinessSource,
                                        _ => unreachable!("compiler-owned file property"),
                                    },
                                ),
                                span: hook.body.span(),
                            };
                        }
                        continue;
                    }
                    let crate::ast::ClassMember::Method(method) = member else {
                        continue;
                    };
                    if declaration.name == "TransferGroup" && method.name != "__construct" {
                        method.body = crate::ast::FunctionBody::Intrinsic {
                            implementation: crate::ast::CompilerIntrinsic::TransferGroup(
                                match method.name.as_str() {
                                    "getNextCompletion" => TransferGroupFunction::NextCompletion,
                                    "getReadRegion" => TransferGroupFunction::ReadRegion,
                                    "resubmitRead" => TransferGroupFunction::ResubmitRead,
                                    "resubmitWrite" => TransferGroupFunction::ResubmitWrite,
                                    _ => unreachable!("compiler-owned group method"),
                                },
                            ),
                            span: method.body.span(),
                        };
                        continue;
                    }
                    if declaration.name == "TransferPlan" && method.name != "__construct" {
                        method.body = crate::ast::FunctionBody::Intrinsic {
                            implementation: crate::ast::CompilerIntrinsic::TransferPlan(
                                match method.name.as_str() {
                                    "read" => TransferPlanFunction::Read,
                                    "andReadFrom" => TransferPlanFunction::AndReadFrom,
                                    "writeTo" => TransferPlanFunction::WriteTo,
                                    "andWriteTo" => TransferPlanFunction::AndWriteTo,
                                    "getReadKey" => TransferPlanFunction::GetReadKey,
                                    "getWriteKey" => TransferPlanFunction::GetWriteKey,
                                    _ => unreachable!("compiler-owned transfer-plan method"),
                                },
                            ),
                            span: method.body.span(),
                        };
                        continue;
                    }
                    if declaration.name == "Child" && method.name != "__construct" {
                        method.body = crate::ast::FunctionBody::Intrinsic {
                            implementation: crate::ast::CompilerIntrinsic::Process(
                                match method.name.as_str() {
                                    "launch" => ProcessFunction::Launch,
                                    "getInput" => ProcessFunction::InputPipe,
                                    "getOutput" => ProcessFunction::OutputPipe,
                                    "getErrors" => ProcessFunction::ErrorsPipe,
                                    "getId" => ProcessFunction::Id,
                                    "finish" => ProcessFunction::Finish,
                                    "waitExitUntil" => ProcessFunction::WaitUntil,
                                    "exitStatus" => ProcessFunction::Status,
                                    "terminateExit" => ProcessFunction::Terminate,
                                    "release" => ProcessFunction::Release,
                                    "takeCapturedOutput" => ProcessFunction::CapturedOutput,
                                    "takeCapturedErrors" => ProcessFunction::CapturedErrors,
                                    "captureTruncated" => ProcessFunction::Truncated,
                                    _ => unreachable!("compiler-owned process method"),
                                },
                            ),
                            span: method.body.span(),
                        };
                        continue;
                    }
                    if declaration.name == "ReadinessSet" {
                        method.body = crate::ast::FunctionBody::Intrinsic {
                            implementation: crate::ast::CompilerIntrinsic::Readiness(match method
                                .name
                                .as_str()
                            {
                                "__construct" => ReadinessFunction::New,
                                "register" => ReadinessFunction::Register,
                                "unregister" => ReadinessFunction::Unregister,
                                "getStream" => ReadinessFunction::GetStream,
                                "getWritableStream" => ReadinessFunction::GetWritableStream,
                                "begin" => ReadinessFunction::Begin,
                                _ => unreachable!("compiler-owned readiness method"),
                            }),
                            span: method.body.span(),
                        };
                        continue;
                    }
                    let implementation = match (declaration.name.as_str(), method.name.as_str()) {
                        (_, "__construct") => FileFunction::UnavailableConstructor,
                        ("ReadRequest", "complete") => FileFunction::CompleteReadRequest,
                        ("WriteRequest", "complete") => FileFunction::CompleteWriteRequest,
                        (_, "prepareRead") => FileFunction::PrepareRead,
                        (_, "prepareWrite") => FileFunction::PrepareWrite,
                        (_, "completeRead") => FileFunction::CompleteRead,
                        (_, "completeWrite") => FileFunction::CompleteWrite,
                        ("File", "openRead") => FileFunction::OpenRead,
                        ("File", "openWrite") => FileFunction::OpenWrite,
                        ("File", "openReadWrite") => FileFunction::OpenReadWrite,
                        (
                            "ReadableFile" | "DuplexFile" | "StandardInput" | "ChildOutput",
                            "read",
                        ) => FileFunction::Read,
                        (
                            "WritableFile" | "DuplexFile" | "StandardOutput" | "StandardError"
                            | "ChildInput",
                            "write",
                        ) => FileFunction::Write,
                        ("ChildInput", "closeInput") => FileFunction::CloseInput,
                        ("ReadableFile" | "WritableFile" | "DuplexFile", "seek") => {
                            FileFunction::Seek
                        }
                        ("ReadableFile" | "WritableFile" | "DuplexFile", "getPosition") => {
                            FileFunction::Position
                        }
                        ("ReadableFile" | "WritableFile" | "DuplexFile", "getLength") => {
                            FileFunction::Length
                        }
                        ("WritableFile" | "DuplexFile", "synchronizeData") => {
                            FileFunction::SynchronizeData
                        }
                        ("WritableFile" | "DuplexFile", "synchronizeMetadata") => {
                            FileFunction::SynchronizeMetadata
                        }
                        ("WritableFile" | "DuplexFile", "setLength") => FileFunction::SetLength,
                        ("ReadableFile" | "WritableFile" | "DuplexFile", "close") => {
                            FileFunction::Close
                        }
                        ("ReadableFile" | "WritableFile" | "DuplexFile", "acquireLock") => {
                            FileFunction::AcquireLock
                        }
                        ("ReadableFile" | "WritableFile" | "DuplexFile", "tryAcquireLock") => {
                            FileFunction::TryAcquireLock
                        }
                        ("FileLock" | "LockedFile", "releaseFile") => FileFunction::ReleaseLock,
                        ("FileLock" | "LockedFile", "abandonFile") => FileFunction::AbandonLock,
                        _ => unreachable!("compiler-owned file method has an implementation"),
                    };
                    method.body = crate::ast::FunctionBody::Intrinsic {
                        implementation: crate::ast::CompilerIntrinsic::File(implementation),
                        span: method.body.span(),
                    };
                }
            }
            let name = match item {
                Item::Enum(declaration) => &mut declaration.name,
                Item::Interface(declaration) => &mut declaration.name,
                Item::Class(declaration) => &mut declaration.name,
                _ => unreachable!("stream contracts contain only nominal declarations"),
            };
            let namespace = if PROCESS_DECLARATIONS.contains(&name.as_str()) {
                "Process"
            } else {
                "Io"
            };
            *name = format!("Doria\\Std\\{namespace}\\{name}");
        }
        program
    });
    &DECLARATIONS
}

pub fn resolved_facts_use_streams(facts: &GlobalSymbolFacts) -> bool {
    facts.references.iter().any(|reference| {
        matches!(&reference.symbol_id.owner,
            GlobalSymbolOwner::CompilerKnown(CompilerSymbolIdentity::StandardIo(name))
                if CANONICAL_TYPES.contains(&name.as_str()))
    })
}

pub fn augment_program(program: &Program) -> Program {
    let mut augmented = program.clone();
    augmented.items.extend(declarations().items.iter().cloned());
    augmented
}

fn item_name(item: &Item) -> &str {
    match item {
        Item::Enum(declaration) => &declaration.name,
        Item::Interface(declaration) => &declaration.name,
        Item::Class(declaration) => &declaration.name,
        _ => unreachable!("stream contracts contain only nominal declarations"),
    }
}

fn item_span(item: &Item) -> crate::source::Span {
    match item {
        Item::Enum(declaration) => declaration.span,
        Item::Interface(declaration) => declaration.span,
        Item::Class(declaration) => declaration.span,
        _ => unreachable!("stream contracts contain only nominal declarations"),
    }
}

/// For each declaration of this source, the declarations its text names.
/// Every cross-reference in SOURCE_TEXT is fully qualified (`Doria\Std\Io\X`,
/// `Doria\Std\Process\X`), so scanning each declaration's own text is exact.
fn declaration_dependencies() -> &'static [Vec<usize>] {
    static DEPENDENCIES: std::sync::LazyLock<Vec<Vec<usize>>> = std::sync::LazyLock::new(|| {
        let items = &declarations().items;
        let index = items
            .iter()
            .enumerate()
            .map(|(position, item)| (item_name(item), position))
            .collect::<std::collections::HashMap<_, _>>();
        items
            .iter()
            .map(|item| {
                let span = item_span(item);
                let text = &SOURCE_TEXT[span.start..span.end];
                let mut dependencies = Vec::new();
                for (start, _) in text.match_indices("Doria\\Std\\") {
                    let rest = &text[start..];
                    let end = rest
                        .char_indices()
                        .skip("Doria\\Std\\".len())
                        .find(|(_, character)| !(character.is_alphanumeric() || *character == '\\'))
                        .map_or(rest.len(), |(end, _)| end);
                    if let Some(position) = index.get(&rest[..end]) {
                        if !dependencies.contains(position) {
                            dependencies.push(*position);
                        }
                    }
                }
                dependencies
            })
            .collect()
    });
    &DEPENDENCIES
}

/// Adds only the declarations the program reaches: those it names, closed
/// over what those declarations name in turn. Unreached standard code is never
/// checked, lowered or compiled, so a program pays only for what it uses.
pub fn augment_program_for(program: &Program, facts: &GlobalSymbolFacts) -> Program {
    let items = &declarations().items;
    let dependencies = declaration_dependencies();
    let mut reached = vec![false; items.len()];
    let mut pending = items
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            facts.references.iter().any(|reference| {
                matches!(&reference.symbol_id.owner,
                    GlobalSymbolOwner::CompilerKnown(CompilerSymbolIdentity::StandardIo(name))
                        if name == item_name(item))
            })
        })
        .map(|(position, _)| position)
        .collect::<Vec<_>>();
    while let Some(position) = pending.pop() {
        if std::mem::replace(&mut reached[position], true) {
            continue;
        }
        pending.extend(dependencies[position].iter().copied());
    }
    let mut augmented = program.clone();
    augmented.items.extend(
        items
            .iter()
            .zip(&reached)
            .filter(|(_, reached)| **reached)
            .map(|(item, _)| item.clone()),
    );
    augmented
}

pub(crate) fn append_hir_source(program: &mut crate::hir::Program) {
    // Any reached declaration, not only an enum: a program may reach classes alone.
    if !program.items.iter().any(|item| match item {
        crate::hir::Item::Enum(declaration) => declaration.span.source == SOURCE_ID,
        crate::hir::Item::Class(declaration) => declaration.span.source == SOURCE_ID,
        _ => false,
    }) {
        return;
    }
    program.sources.push(crate::hir::SourceUnit {
        id: SOURCE_ID,
        identity: crate::names::SourceIdentity(SOURCE_NAME.to_string()),
        package: crate::names::PackageIdentity::CompilerKnown,
        display_path: SOURCE_NAME.to_string(),
        scope: crate::build_plan::SourceScope::Main,
        origin: crate::build_plan::SourceOrigin::Explicit,
        generated_for: None,
        active: true,
        source: SourceFile::with_id(SOURCE_ID, SOURCE_NAME, SOURCE_TEXT),
    });
    if !program
        .packages
        .iter()
        .any(|package| package.identity == crate::names::PackageIdentity::CompilerKnown)
    {
        program.packages.push(crate::hir::PackageUnit {
            identity: crate::names::PackageIdentity::CompilerKnown,
            normal_dependencies: Vec::new(),
            development_dependencies: Vec::new(),
        });
    }
}
