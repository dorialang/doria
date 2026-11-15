//! The `string` surface (Decision 0103): every `$text->` property and every
//! `String::` companion member, with its signature, availability and
//! one-line documentation.
//!
//! This table is the one copy. The type checker, MIR lowering, the PHP
//! backend, the foreign-spelling table and compiler-adjacent tooling read it;
//! none of them restates a member, a parameter, a default or a pending reason.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StringMemberKind {
    /// `$text->name`, a fact about or a view of one string.
    Property,
    /// `String::name(...)`, a string-specific operation.
    Companion,
}

/// The types that appear in the String surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StringValueType {
    String,
    Int,
    Bool,
    Bytes,
    /// `Doria\Std\Io\ByteRegion`, readonly.
    ByteRegion,
    NullableInt,
    NullableString,
    StringList,
    /// `Iterable<string>`.
    StringIterable,
    /// `Ordering`.
    Ordering,
    /// A pending member whose result type is not decided yet.
    Undecided,
}

impl StringValueType {
    /// The type as Doria source spells it; empty for [`Self::Undecided`].
    pub const fn source(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Int => "int",
            Self::Bool => "bool",
            Self::Bytes => "Bytes",
            Self::ByteRegion => "ByteRegion",
            Self::NullableInt => "?int",
            Self::NullableString => "?string",
            Self::StringList => "List<string>",
            Self::StringIterable => "Iterable<string>",
            Self::Ordering => "Ordering",
            Self::Undecided => "",
        }
    }
}

/// A default a caller may omit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StringDefault {
    Null,
    Text(&'static str),
}

impl StringDefault {
    /// The default as Doria source spells it.
    pub fn source(self) -> String {
        match self {
            Self::Null => "null".to_string(),
            Self::Text(text) => format!("{text:?}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StringParameter {
    pub name: &'static str,
    pub ty: StringValueType,
    pub default: Option<StringDefault>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StringAvailability {
    /// Runs in the interpreter and both native profiles.
    Executable,
    /// Accepted Doria that this compiler does not run yet. `reason` completes
    /// the sentence "`<member>` ..."; `help` says what to use meanwhile, and
    /// `None` means the executable properties.
    Pending {
        reason: &'static str,
        help: Option<&'static str>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StringMember {
    pub name: &'static str,
    pub kind: StringMemberKind,
    /// Companion parameters in declaration order; empty for a property.
    pub parameters: &'static [StringParameter],
    pub result: StringValueType,
    pub availability: StringAvailability,
    pub documentation: &'static str,
}

impl StringMember {
    pub const fn is_executable(&self) -> bool {
        matches!(self.availability, StringAvailability::Executable)
    }

    /// What to use while a pending member is unavailable.
    pub fn pending_help(&self) -> Option<String> {
        match self.availability {
            StringAvailability::Executable => None,
            StringAvailability::Pending {
                help: Some(help), ..
            } => Some(help.to_string()),
            StringAvailability::Pending { help: None, .. } => Some(format!(
                "use the executable {} property",
                executable_property_list("or")
            )),
        }
    }

    /// How source code names the member: `$text->length` or `String::trim`.
    pub fn spelling(&self) -> String {
        match self.kind {
            StringMemberKind::Property => format!("$text->{}", self.name),
            StringMemberKind::Companion => format!("String::{}", self.name),
        }
    }

    /// Canonical source-facing signature used by compiler-adjacent tooling,
    /// such as `int $length` or
    /// `String::slice(string $text, int $start, ?int $length = null): string`.
    pub fn signature(&self) -> String {
        match self.kind {
            StringMemberKind::Property if self.result == StringValueType::Undecided => {
                format!("${}", self.name)
            }
            StringMemberKind::Property => format!("{} ${}", self.result.source(), self.name),
            StringMemberKind::Companion => {
                let parameters = self
                    .parameters
                    .iter()
                    .map(|parameter| match parameter.default {
                        Some(default) => format!(
                            "{} ${} = {}",
                            parameter.ty.source(),
                            parameter.name,
                            default.source()
                        ),
                        None => format!("{} ${}", parameter.ty.source(), parameter.name),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "String::{}({parameters}): {}",
                    self.name,
                    self.result.source()
                )
            }
        }
    }
}

const fn required(name: &'static str, ty: StringValueType) -> StringParameter {
    StringParameter {
        name,
        ty,
        default: None,
    }
}

const fn optional(
    name: &'static str,
    ty: StringValueType,
    default: StringDefault,
) -> StringParameter {
    StringParameter {
        name,
        ty,
        default: Some(default),
    }
}

const fn property_entry(
    name: &'static str,
    result: StringValueType,
    documentation: &'static str,
) -> StringMember {
    StringMember {
        name,
        kind: StringMemberKind::Property,
        parameters: &[],
        result,
        availability: StringAvailability::Executable,
        documentation,
    }
}

const fn companion_entry(
    name: &'static str,
    parameters: &'static [StringParameter],
    result: StringValueType,
    documentation: &'static str,
) -> StringMember {
    StringMember {
        name,
        kind: StringMemberKind::Companion,
        parameters,
        result,
        availability: StringAvailability::Executable,
        documentation,
    }
}

const fn pending(
    member: StringMember,
    reason: &'static str,
    help: Option<&'static str>,
) -> StringMember {
    StringMember {
        availability: StringAvailability::Pending { reason, help },
        ..member
    }
}

use StringValueType as T;

const TEXT: &[StringParameter] = &[required("text", T::String)];
const NEEDLE: &[StringParameter] = &[required("text", T::String), required("needle", T::String)];
const PREFIX: &[StringParameter] = &[required("text", T::String), required("prefix", T::String)];
const SUFFIX: &[StringParameter] = &[required("text", T::String), required("suffix", T::String)];
const PAIR: &[StringParameter] = &[required("left", T::String), required("right", T::String)];
const PADDING: &[StringParameter] = &[
    required("text", T::String),
    required("length", T::Int),
    optional("padding", T::String, StringDefault::Text(" ")),
];

const PENDING_ITERATION: &str = "requires the future public iteration protocol";
const PENDING_ORDERING: &str = "requires the executable `Ordering` type";
const ORDERING_HELP: Option<&str> =
    Some("use typed equality or `String::equalsIgnoreCase` when only equality is needed");

/// Every String member, properties first, each group in reference order.
pub const STRING_MEMBERS: &[StringMember] = &[
    property_entry(
        "length",
        T::Int,
        "Returns the number of Unicode extended grapheme clusters.",
    ),
    property_entry(
        "displayWidth",
        T::Int,
        "Returns the number of terminal cells the text occupies (Decision 0146).",
    ),
    property_entry(
        "byteLength",
        T::Int,
        "Returns the exact UTF-8 byte count.",
    ),
    property_entry(
        "isEmpty",
        T::Bool,
        "Reports whether the string contains zero UTF-8 bytes.",
    ),
    property_entry(
        "bytes",
        T::Bytes,
        "Returns an owned mutable copy of the string's UTF-8 bytes.",
    ),
    property_entry(
        "byteRegion",
        T::ByteRegion,
        "Borrows this string's UTF-8 bytes as a readonly region without allocation or copying. Use the region's `getRegion` for a byte range. The region keeps the string borrowed while it is in use.",
    ),
    pending(
        property_entry(
            "graphemes",
            T::StringIterable,
            "Walks the string's extended grapheme clusters in order.",
        ),
        PENDING_ITERATION,
        None,
    ),
    pending(
        property_entry(
            "codePoints",
            T::Undecided,
            "Walks the string's Unicode scalar values in order.",
        ),
        PENDING_ITERATION,
        None,
    ),
    companion_entry(
        "trim",
        TEXT,
        T::String,
        "Removes Unicode whitespace from both ends.",
    ),
    companion_entry(
        "trimStart",
        TEXT,
        T::String,
        "Removes Unicode whitespace from the beginning.",
    ),
    companion_entry(
        "trimEnd",
        TEXT,
        T::String,
        "Removes Unicode whitespace from the end.",
    ),
    companion_entry(
        "lower",
        TEXT,
        T::String,
        "Applies locale-independent default Unicode lowercase mapping.",
    ),
    companion_entry(
        "upper",
        TEXT,
        T::String,
        "Applies locale-independent default Unicode uppercase mapping.",
    ),
    companion_entry(
        "lowerFirst",
        TEXT,
        T::String,
        "Lowercases the first extended grapheme cluster without normalization.",
    ),
    companion_entry(
        "upperFirst",
        TEXT,
        T::String,
        "Uppercases the first extended grapheme cluster without normalization.",
    ),
    companion_entry(
        "contains",
        NEEDLE,
        T::Bool,
        "Tests for a case-sensitive, grapheme-boundary-aligned literal match.",
    ),
    companion_entry(
        "startsWith",
        PREFIX,
        T::Bool,
        "Tests for a case-sensitive literal prefix on grapheme boundaries.",
    ),
    companion_entry(
        "endsWith",
        SUFFIX,
        T::Bool,
        "Tests for a case-sensitive literal suffix on grapheme boundaries.",
    ),
    companion_entry(
        "equalsIgnoreCase",
        PAIR,
        T::Bool,
        "Compares with full default Unicode case folding and no normalization.",
    ),
    companion_entry(
        "containsIgnoreCase",
        NEEDLE,
        T::Bool,
        "Tests for a grapheme-boundary-aligned match using full default Unicode case folding.",
    ),
    companion_entry(
        "startsWithIgnoreCase",
        PREFIX,
        T::Bool,
        "Tests for a folded prefix while preserving original grapheme boundaries.",
    ),
    companion_entry(
        "endsWithIgnoreCase",
        SUFFIX,
        T::Bool,
        "Tests for a folded suffix while preserving original grapheme boundaries.",
    ),
    companion_entry(
        "indexOf",
        NEEDLE,
        T::NullableInt,
        "Returns the first matching grapheme index, or null.",
    ),
    companion_entry(
        "lastIndexOf",
        NEEDLE,
        T::NullableInt,
        "Returns the final matching grapheme index, or null.",
    ),
    companion_entry(
        "indexOfIgnoreCase",
        NEEDLE,
        T::NullableInt,
        "Returns the first folded match as an index in the original grapheme sequence, or null.",
    ),
    companion_entry(
        "lastIndexOfIgnoreCase",
        NEEDLE,
        T::NullableInt,
        "Returns the final folded match as an index in the original grapheme sequence, or null.",
    ),
    companion_entry(
        "countOccurrences",
        NEEDLE,
        T::Int,
        "Counts non-overlapping grapheme-boundary matches from left to right.",
    ),
    companion_entry(
        "replace",
        &[
            required("text", T::String),
            required("search", T::String),
            required("replacement", T::String),
        ],
        T::String,
        "Replaces all non-overlapping literal matches on grapheme boundaries.",
    ),
    companion_entry(
        "split",
        &[required("text", T::String), required("separator", T::String)],
        T::StringList,
        "Splits on literal grapheme-boundary matches and preserves empty fields.",
    ),
    companion_entry(
        "join",
        &[
            required("separator", T::String),
            required("values", T::StringList),
        ],
        T::String,
        "Joins a readonly list without consuming it.",
    ),
    companion_entry(
        "slice",
        &[
            required("text", T::String),
            required("start", T::Int),
            optional("length", T::NullableInt, StringDefault::Null),
        ],
        T::String,
        "Returns a clamped slice measured in extended grapheme clusters.",
    ),
    companion_entry(
        "repeat",
        &[required("text", T::String), required("count", T::Int)],
        T::String,
        "Repeats the string; a negative count panics.",
    ),
    companion_entry(
        "padStart",
        PADDING,
        T::String,
        "Pads the beginning to an exact grapheme length; the padding defaults to one space.",
    ),
    companion_entry(
        "padEnd",
        PADDING,
        T::String,
        "Pads the end to an exact grapheme length; the padding defaults to one space.",
    ),
    companion_entry(
        "fromBytes",
        &[required("bytes", T::Bytes)],
        T::NullableString,
        "Copies valid UTF-8 bytes into a string and returns null for invalid UTF-8.",
    ),
    pending(
        companion_entry(
            "compare",
            PAIR,
            T::Ordering,
            "Orders two strings by their Unicode scalar values.",
        ),
        PENDING_ORDERING,
        ORDERING_HELP,
    ),
    pending(
        companion_entry(
            "compareIgnoreCase",
            PAIR,
            T::Ordering,
            "Orders two strings after full default Unicode case folding.",
        ),
        PENDING_ORDERING,
        ORDERING_HELP,
    ),
];

/// The member of `kind` named `name`, executable or pending.
pub fn member(kind: StringMemberKind, name: &str) -> Option<&'static StringMember> {
    STRING_MEMBERS
        .iter()
        .find(|member| member.kind == kind && member.name == name)
}

pub fn property(name: &str) -> Option<&'static StringMember> {
    member(StringMemberKind::Property, name)
}

pub fn companion(name: &str) -> Option<&'static StringMember> {
    member(StringMemberKind::Companion, name)
}

/// The members of `kind`, executable or pending, in table order.
pub fn members(kind: StringMemberKind) -> impl Iterator<Item = &'static StringMember> {
    STRING_MEMBERS
        .iter()
        .filter(move |member| member.kind == kind)
}

/// The executable property names as prose, such as "`length`, `bytes`, or
/// `byteRegion`", for diagnostics that list them.
pub fn executable_property_list(conjunction: &str) -> String {
    let names = members(StringMemberKind::Property)
        .filter(|member| member.is_executable())
        .map(|member| format!("`{}`", member.name))
        .collect::<Vec<_>>();
    match names.as_slice() {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{}, {conjunction} {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtins::{StringSpellingTarget, FOREIGN_STRING_SPELLINGS};

    #[test]
    fn every_member_is_listed_once() {
        for (index, member) in STRING_MEMBERS.iter().enumerate() {
            assert!(
                !STRING_MEMBERS[..index]
                    .iter()
                    .any(|earlier| earlier.name == member.name),
                "`{}` is listed twice",
                member.name
            );
            assert!(
                !member.documentation.is_empty(),
                "`{}` has no documentation",
                member.name
            );
            if member.kind == StringMemberKind::Property {
                assert!(
                    member.parameters.is_empty(),
                    "property `{}` has parameters",
                    member.name
                );
            }
        }
    }

    #[test]
    fn signatures_read_as_doria_source() {
        assert_eq!(property("length").unwrap().signature(), "int $length");
        assert_eq!(property("codePoints").unwrap().signature(), "$codePoints");
        assert_eq!(
            companion("slice").unwrap().signature(),
            "String::slice(string $text, int $start, ?int $length = null): string"
        );
        assert_eq!(
            companion("padEnd").unwrap().signature(),
            "String::padEnd(string $text, int $length, string $padding = \" \"): string"
        );
        assert_eq!(
            property("byteRegion").unwrap().spelling(),
            "$text->byteRegion"
        );
        assert_eq!(
            executable_property_list("and"),
            "`length`, `displayWidth`, `byteLength`, `isEmpty`, `bytes`, and `byteRegion`"
        );
    }

    #[test]
    fn foreign_spellings_point_at_members_of_the_table() {
        let check = |target: StringSpellingTarget, foreign: &str| {
            match target {
            StringSpellingTarget::Companion(name) => assert!(
                companion(name).is_some_and(StringMember::is_executable),
                "`{foreign}` teaches `String::{name}`, which is not an executable companion member"
            ),
            StringSpellingTarget::PendingComparison(name) => assert!(
                companion(name).is_some_and(|member| !member.is_executable()),
                "`{foreign}` teaches pending `String::{name}`, which the table does not list as pending"
            ),
            StringSpellingTarget::Property(name) => assert!(
                property(name).is_some_and(StringMember::is_executable),
                "`{foreign}` teaches `$text->{name}`, which is not an executable property"
            ),
        }
        };
        for spelling in FOREIGN_STRING_SPELLINGS {
            check(spelling.target, spelling.foreign);
            if let Some((alternative, _)) = spelling.alternative {
                check(alternative, spelling.foreign);
            }
        }
    }
}
