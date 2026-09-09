//! Compiler-owned declarations; no runtime interface representation is introduced here.

use crate::ast::{InterfaceDecl, Item, Program};
use crate::source::{SourceFile, SourceId};

pub const SOURCE_ID: SourceId = SourceId(0x7fff_fffd);
pub const SOURCE_NAME: &str = "<compiler-known:contracts>";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoreValueOperation {
    Equal,
    Hash,
    Compare,
    Clone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IterationOperation {
    Acquire,
    HasCurrent,
    GetCurrent,
    Advance,
}

impl IterationOperation {
    pub fn contract(self) -> &'static str {
        if self == Self::Acquire {
            "Iterable"
        } else {
            "Iterator"
        }
    }

    pub fn method(self) -> &'static str {
        match self {
            Self::Acquire => "iterator",
            Self::HasCurrent => "hasCurrent",
            Self::GetCurrent => "getCurrent",
            Self::Advance => "advance",
        }
    }

    pub fn from_requirement(span: crate::source::Span) -> Option<Self> {
        [
            Self::Acquire,
            Self::HasCurrent,
            Self::GetCurrent,
            Self::Advance,
        ]
        .into_iter()
        .find(|operation| {
            interfaces()
                .find(|interface| interface.name == operation.contract())
                .is_some_and(|interface| {
                    interface.requirements.iter().any(|requirement| {
                        requirement.name == operation.method() && requirement.span == span
                    })
                })
        })
    }
}

impl CoreValueOperation {
    pub fn from_requirement(span: crate::source::Span) -> Option<Self> {
        [Self::Equal, Self::Hash, Self::Compare, Self::Clone]
            .into_iter()
            .find(|operation| {
                interfaces()
                    .find(|interface| interface.name == operation.contract())
                    .is_some_and(|interface| {
                        interface.requirements.iter().any(|requirement| {
                            requirement.name == operation.method() && requirement.span == span
                        })
                    })
            })
    }

    pub fn contract(self) -> &'static str {
        match self {
            Self::Equal => "Equatable",
            Self::Hash => "Hashable",
            Self::Compare => "Comparable",
            Self::Clone => "Cloneable",
        }
    }

    pub fn method(self) -> &'static str {
        match self {
            Self::Equal => "equals",
            Self::Hash => "hash",
            Self::Compare => "compare",
            Self::Clone => "clone",
        }
    }
}

pub const SOURCE_TEXT: &str = r#"
interface Displayable { function toString(): string; }
interface Error {}
interface Comparable<T> { function compare(T $other): Ordering; }
interface Equatable<T> { function equals(T $other): bool; }
interface Hashable { function hash(): uint64; }
interface Cloneable { function clone(): self; }
interface Iterable<T> { function iterator(): Iterator<T>; }
interface Iterator<T> {
    function hasCurrent(): bool;
    function getCurrent(): T;
    writable function advance(): void;
}
enum Ordering { case Less; case Equal; case Greater; }
"#;

pub fn declarations() -> &'static Program {
    static DECLARATIONS: std::sync::LazyLock<Program> = std::sync::LazyLock::new(|| {
        crate::parse_source_file(&SourceFile::with_id(SOURCE_ID, SOURCE_NAME, SOURCE_TEXT))
            .expect("compiler-known contract declarations parse")
    });
    &DECLARATIONS
}

pub fn interfaces() -> impl Iterator<Item = &'static InterfaceDecl> {
    declarations().items.iter().filter_map(|item| match item {
        Item::Interface(declaration) => Some(declaration),
        _ => None,
    })
}
