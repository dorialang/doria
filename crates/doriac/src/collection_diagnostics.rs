use crate::diagnostics::FixApplicability;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CollectionReceiver {
    TypedArray,
    List,
    Dictionary,
    Set,
    SortedDictionary,
    SortedSet,
    PriorityQueue,
    Deque,
}

impl CollectionReceiver {
    pub const fn source_name(self) -> &'static str {
        match self {
            Self::TypedArray => "T[]",
            Self::List => "List",
            Self::Dictionary => "Dictionary",
            Self::Set => "Set",
            Self::SortedDictionary => "SortedDictionary",
            Self::SortedSet => "SortedSet",
            Self::PriorityQueue => "PriorityQueue",
            Self::Deque => "Deque",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionMemberKind {
    Method,
    Property,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgumentShape {
    Property,
    Exact(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImplementationStatus {
    Executable,
    PendingSlice4,
}

impl ImplementationStatus {
    pub const fn slice(self) -> Option<u8> {
        match self {
            Self::Executable => None,
            Self::PendingSlice4 => Some(4),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollectionMemberSuggestion {
    pub input: &'static str,
    pub canonical: &'static str,
    pub receivers: &'static [CollectionReceiver],
    pub member_kind: CollectionMemberKind,
    pub arguments: ArgumentShape,
    pub applicability: FixApplicability,
    pub decision_owner: &'static str,
    pub implementation: ImplementationStatus,
}

const MAPS: &[CollectionReceiver] = &[
    CollectionReceiver::Dictionary,
    CollectionReceiver::SortedDictionary,
];
const ELEMENT_COLLECTIONS: &[CollectionReceiver] = &[
    CollectionReceiver::TypedArray,
    CollectionReceiver::List,
    CollectionReceiver::Set,
    CollectionReceiver::SortedSet,
    CollectionReceiver::PriorityQueue,
    CollectionReceiver::Deque,
];
const NAMED_COLLECTIONS: &[CollectionReceiver] = &[
    CollectionReceiver::List,
    CollectionReceiver::Dictionary,
    CollectionReceiver::Set,
    CollectionReceiver::SortedDictionary,
    CollectionReceiver::SortedSet,
    CollectionReceiver::PriorityQueue,
    CollectionReceiver::Deque,
];
const ADD_COLLECTIONS: &[CollectionReceiver] = &[
    CollectionReceiver::List,
    CollectionReceiver::Set,
    CollectionReceiver::SortedSet,
];
const REMOVE_COLLECTIONS: &[CollectionReceiver] = &[
    CollectionReceiver::List,
    CollectionReceiver::Dictionary,
    CollectionReceiver::Set,
    CollectionReceiver::SortedDictionary,
    CollectionReceiver::SortedSet,
];
const SET_ENDPOINT_COLLECTIONS: &[CollectionReceiver] =
    &[CollectionReceiver::Set, CollectionReceiver::SortedSet];
const LIST: &[CollectionReceiver] = &[CollectionReceiver::List];
const DEQUE: &[CollectionReceiver] = &[CollectionReceiver::Deque];

macro_rules! suggestion {
    ($input:literal, $canonical:literal, $receivers:expr, $kind:ident, $arguments:expr, $applicability:ident, $status:ident) => {
        CollectionMemberSuggestion {
            input: $input,
            canonical: $canonical,
            receivers: $receivers,
            member_kind: CollectionMemberKind::$kind,
            arguments: $arguments,
            applicability: FixApplicability::$applicability,
            decision_owner: "Decision 0113",
            implementation: ImplementationStatus::$status,
        }
    };
}

/// The compiler-owned spelling authority for collection member discovery.
///
/// Future migration tooling consumes this data directly. Editors project the
/// structured fixes emitted by semantic analysis and do not duplicate it.
pub const COLLECTION_MEMBER_SUGGESTIONS: &[CollectionMemberSuggestion] = &[
    suggestion!(
        "has",
        "containsKey",
        MAPS,
        Method,
        ArgumentShape::Exact(1),
        MachineApplicable,
        Executable
    ),
    suggestion!(
        "hasKey",
        "containsKey",
        MAPS,
        Method,
        ArgumentShape::Exact(1),
        MachineApplicable,
        Executable
    ),
    suggestion!(
        "array_key_exists",
        "containsKey",
        MAPS,
        Method,
        ArgumentShape::Exact(1),
        MachineApplicable,
        Executable
    ),
    suggestion!(
        "contains_key",
        "containsKey",
        MAPS,
        Method,
        ArgumentShape::Exact(1),
        MachineApplicable,
        Executable
    ),
    suggestion!(
        "ContainsKey",
        "containsKey",
        MAPS,
        Method,
        ArgumentShape::Exact(1),
        MachineApplicable,
        Executable
    ),
    suggestion!(
        "in_array",
        "contains",
        ELEMENT_COLLECTIONS,
        Method,
        ArgumentShape::Exact(1),
        MachineApplicable,
        Executable
    ),
    suggestion!(
        "includes",
        "contains",
        ELEMENT_COLLECTIONS,
        Method,
        ArgumentShape::Exact(1),
        MachineApplicable,
        Executable
    ),
    suggestion!(
        "size",
        "count",
        NAMED_COLLECTIONS,
        Property,
        ArgumentShape::Property,
        MachineApplicable,
        Executable
    ),
    suggestion!(
        "Count",
        "count",
        NAMED_COLLECTIONS,
        Property,
        ArgumentShape::Property,
        MachineApplicable,
        Executable
    ),
    suggestion!(
        "len",
        "count",
        NAMED_COLLECTIONS,
        Property,
        ArgumentShape::Property,
        MachineApplicable,
        Executable
    ),
    suggestion!(
        "length",
        "count",
        NAMED_COLLECTIONS,
        Property,
        ArgumentShape::Property,
        MachineApplicable,
        Executable
    ),
    suggestion!(
        "append",
        "add",
        ADD_COLLECTIONS,
        Method,
        ArgumentShape::Exact(1),
        MachineApplicable,
        Executable
    ),
    suggestion!(
        "push",
        "add",
        ADD_COLLECTIONS,
        Method,
        ArgumentShape::Exact(1),
        MachineApplicable,
        Executable
    ),
    suggestion!(
        "array_search",
        "indexOf",
        LIST,
        Method,
        ArgumentShape::Exact(1),
        RequiresReview,
        Executable
    ),
    suggestion!(
        "position",
        "indexOf",
        LIST,
        Method,
        ArgumentShape::Exact(1),
        RequiresReview,
        Executable
    ),
    suggestion!(
        "find",
        "indexOf",
        LIST,
        Method,
        ArgumentShape::Exact(1),
        RequiresReview,
        Executable
    ),
    suggestion!(
        "unset",
        "remove",
        REMOVE_COLLECTIONS,
        Method,
        ArgumentShape::Exact(1),
        MachineApplicable,
        Executable
    ),
    suggestion!(
        "delete",
        "remove",
        REMOVE_COLLECTIONS,
        Method,
        ArgumentShape::Exact(1),
        MachineApplicable,
        Executable
    ),
    suggestion!(
        "Min",
        "first",
        LIST,
        Property,
        ArgumentShape::Property,
        MachineApplicable,
        Executable
    ),
    suggestion!(
        "Max",
        "last",
        LIST,
        Property,
        ArgumentShape::Property,
        MachineApplicable,
        Executable
    ),
    suggestion!(
        "Min",
        "first",
        SET_ENDPOINT_COLLECTIONS,
        Property,
        ArgumentShape::Property,
        MachineApplicable,
        Executable
    ),
    suggestion!(
        "Max",
        "last",
        SET_ENDPOINT_COLLECTIONS,
        Property,
        ArgumentShape::Property,
        MachineApplicable,
        Executable
    ),
    suggestion!(
        "Enqueue",
        "pushBack",
        DEQUE,
        Method,
        ArgumentShape::Exact(1),
        MachineApplicable,
        Executable
    ),
    suggestion!(
        "Dequeue",
        "popFront",
        DEQUE,
        Method,
        ArgumentShape::Exact(0),
        MachineApplicable,
        Executable
    ),
];

pub fn suggestion_for(
    receiver: CollectionReceiver,
    input: &str,
) -> Option<&'static CollectionMemberSuggestion> {
    COLLECTION_MEMBER_SUGGESTIONS
        .iter()
        .find(|entry| entry.input == input && entry.receivers.contains(&receiver))
}

pub fn canonical_property_status(
    receiver: CollectionReceiver,
    member: &str,
) -> Option<ImplementationStatus> {
    use CollectionReceiver::*;
    use ImplementationStatus::*;

    match (receiver, member) {
        (TypedArray, "length") => Some(Executable),
        (
            List | Dictionary | Set | SortedDictionary | SortedSet | PriorityQueue | Deque,
            "count" | "isEmpty",
        ) => Some(Executable),
        (List, "first" | "last") => Some(Executable),
        (Dictionary | SortedDictionary, "keys" | "values") => Some(Executable),
        (PriorityQueue, "peek") => Some(Executable),
        (Deque, "peekFront" | "peekBack") => Some(Executable),
        (Set | SortedSet, "first" | "last") => Some(Executable),
        _ => None,
    }
}

pub fn pending_method_status(
    receiver: CollectionReceiver,
    member: &str,
) -> Option<ImplementationStatus> {
    use CollectionReceiver::*;
    use ImplementationStatus::*;

    match (receiver, member) {
        (
            List | Dictionary | Set | SortedDictionary | SortedSet | PriorityQueue | Deque,
            "clear",
        ) => Some(PendingSlice4),
        _ => None,
    }
}
