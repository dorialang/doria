#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum AssertionMatcher {
    Equal,
    Null,
    True,
    False,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
    StringContains,
    StringStartsWith,
    StringEndsWith,
    StringEmpty,
    CollectionContains,
    CollectionEmpty,
    CollectionCount,
    DictionaryHasKey,
    DictionaryHasValue,
    Throws,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatcherDomain {
    General,
    Nullable,
    Bool,
    Ordered,
    String,
    CollectionContains,
    CollectionEmpty,
    CollectionCount,
    DictionaryKey,
    DictionaryValue,
    Throws,
    ExplicitFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpectedOperandRule {
    None,
    SameType,
    OrderedSameType,
    String,
    ExactInt,
    CollectionElement,
    DictionaryKey,
    DictionaryValue,
    OptionalErrorInspector,
    FailureMessage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DifferenceStrategy {
    None,
    StringGrapheme,
    StringFragment,
    CollectionCount,
    CollectionMembership,
    DictionaryKey,
    DictionaryValue,
    CheckedError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatcherSpec {
    pub matcher: AssertionMatcher,
    pub source_name: &'static str,
    pub fact_name: &'static str,
    pub minimum_arity: usize,
    pub maximum_arity: usize,
    pub domain: MatcherDomain,
    pub expected_operand: ExpectedOperandRule,
    pub negation_supported: bool,
    pub positive_message: &'static str,
    pub negative_message: &'static str,
    pub difference: DifferenceStrategy,
    pub positive_difference: Option<&'static str>,
    pub negative_difference: Option<&'static str>,
    pub stable_complexity: Option<&'static str>,
}

macro_rules! matcher {
    ($matcher:ident, $source:literal, $fact:literal, $arity:literal, $domain:ident,
        $operand:ident, $positive:literal, $negative:literal, $difference:ident,
        $positive_difference:expr, $negative_difference:expr, $complexity:expr) => {
        MatcherSpec {
            matcher: AssertionMatcher::$matcher,
            source_name: $source,
            fact_name: $fact,
            minimum_arity: $arity,
            maximum_arity: $arity,
            domain: MatcherDomain::$domain,
            expected_operand: ExpectedOperandRule::$operand,
            negation_supported: true,
            positive_message: $positive,
            negative_message: $negative,
            difference: DifferenceStrategy::$difference,
            positive_difference: $positive_difference,
            negative_difference: $negative_difference,
            stable_complexity: $complexity,
        }
    };
    ($matcher:ident, $source:literal, $fact:literal, $minimum:literal ..= $maximum:literal, $domain:ident,
        $operand:ident, $positive:literal, $negative:literal, $difference:ident,
        $positive_difference:expr, $negative_difference:expr, $complexity:expr) => {
        MatcherSpec {
            matcher: AssertionMatcher::$matcher,
            source_name: $source,
            fact_name: $fact,
            minimum_arity: $minimum,
            maximum_arity: $maximum,
            domain: MatcherDomain::$domain,
            expected_operand: ExpectedOperandRule::$operand,
            negation_supported: true,
            positive_message: $positive,
            negative_message: $negative,
            difference: DifferenceStrategy::$difference,
            positive_difference: $positive_difference,
            negative_difference: $negative_difference,
            stable_complexity: $complexity,
        }
    };
}

pub const MATCHER_SPECS: [MatcherSpec; 19] = [
    matcher!(
        Equal,
        "toEqual",
        "Equal",
        1,
        General,
        SameType,
        "expected values to be equal",
        "expected values not to be equal",
        StringGrapheme,
        None,
        None,
        None
    ),
    matcher!(
        Null,
        "toBeNull",
        "Null",
        0,
        Nullable,
        None,
        "expected value to be null",
        "expected value not to be null",
        None,
        None,
        None,
        None
    ),
    matcher!(
        True,
        "toBeTrue",
        "True",
        0,
        Bool,
        None,
        "expected value to be true",
        "expected value not to be true",
        None,
        None,
        None,
        None
    ),
    matcher!(
        False,
        "toBeFalse",
        "False",
        0,
        Bool,
        None,
        "expected value to be false",
        "expected value not to be false",
        None,
        None,
        None,
        None
    ),
    matcher!(
        GreaterThan,
        "toBeGreaterThan",
        "GreaterThan",
        1,
        Ordered,
        OrderedSameType,
        "expected value to be greater than the comparison value",
        "expected value not to be greater than the comparison value",
        None,
        None,
        None,
        None
    ),
    matcher!(
        GreaterThanOrEqual,
        "toBeGreaterThanOrEqual",
        "GreaterThanOrEqual",
        1,
        Ordered,
        OrderedSameType,
        "expected value to be greater than or equal to the comparison value",
        "expected value not to be greater than or equal to the comparison value",
        None,
        None,
        None,
        None
    ),
    matcher!(
        LessThan,
        "toBeLessThan",
        "LessThan",
        1,
        Ordered,
        OrderedSameType,
        "expected value to be less than the comparison value",
        "expected value not to be less than the comparison value",
        None,
        None,
        None,
        None
    ),
    matcher!(
        LessThanOrEqual,
        "toBeLessThanOrEqual",
        "LessThanOrEqual",
        1,
        Ordered,
        OrderedSameType,
        "expected value to be less than or equal to the comparison value",
        "expected value not to be less than or equal to the comparison value",
        None,
        None,
        None,
        None
    ),
    matcher!(
        StringContains,
        "toContain",
        "StringContains",
        1,
        String,
        String,
        "expected string to contain the fragment",
        "expected string not to contain the fragment",
        StringFragment,
        Some("The Expected Fragment Was Not Found"),
        Some("The Unexpected Fragment Was Found"),
        Some("O(n)")
    ),
    matcher!(
        StringStartsWith,
        "toStartWith",
        "StringStartsWith",
        1,
        String,
        String,
        "expected string to start with the prefix",
        "expected string not to start with the prefix",
        StringGrapheme,
        None,
        None,
        Some("O(n)")
    ),
    matcher!(
        StringEndsWith,
        "toEndWith",
        "StringEndsWith",
        1,
        String,
        String,
        "expected string to end with the suffix",
        "expected string not to end with the suffix",
        StringGrapheme,
        None,
        None,
        Some("O(n)")
    ),
    matcher!(
        StringEmpty,
        "toBeEmpty",
        "StringEmpty",
        0,
        String,
        None,
        "expected string to be empty",
        "expected string not to be empty",
        None,
        None,
        None,
        Some("O(1)")
    ),
    matcher!(
        CollectionContains,
        "toContain",
        "CollectionContains",
        1,
        CollectionContains,
        CollectionElement,
        "expected collection to contain the value",
        "expected collection not to contain the value",
        CollectionMembership,
        Some("No Matching Element Was Found"),
        Some("A Matching Element Was Present"),
        None
    ),
    matcher!(
        CollectionEmpty,
        "toBeEmpty",
        "CollectionEmpty",
        0,
        CollectionEmpty,
        None,
        "expected collection to be empty",
        "expected collection not to be empty",
        None,
        None,
        None,
        Some("O(1)")
    ),
    matcher!(
        CollectionCount,
        "toHaveCount",
        "CollectionCount",
        1,
        CollectionCount,
        ExactInt,
        "expected collection to have the requested count",
        "expected collection not to have the requested count",
        CollectionCount,
        None,
        None,
        Some("O(1)")
    ),
    matcher!(
        DictionaryHasKey,
        "toHaveKey",
        "DictionaryHasKey",
        1,
        DictionaryKey,
        DictionaryKey,
        "expected dictionary to have the key",
        "expected dictionary not to have the key",
        DictionaryKey,
        Some("The Expected Key Was Not Found"),
        Some("The Unexpected Key Was Present"),
        None
    ),
    matcher!(
        DictionaryHasValue,
        "toHaveValue",
        "DictionaryHasValue",
        1,
        DictionaryValue,
        DictionaryValue,
        "expected dictionary to have the value",
        "expected dictionary not to have the value",
        DictionaryValue,
        Some("The Expected Value Was Not Found"),
        Some("The Unexpected Value Was Present"),
        Some("O(n)")
    ),
    matcher!(
        Throws,
        "toThrow",
        "Throws",
        0..=1,
        Throws,
        OptionalErrorInspector,
        "expected callable to throw the checked error",
        "expected callable not to throw a checked error",
        CheckedError,
        Some("No Checked Error Was Produced"),
        Some("A Checked Error Was Produced"),
        None
    ),
    matcher!(
        Fail,
        "fail",
        "Fail",
        1,
        ExplicitFailure,
        FailureMessage,
        "explicit test failure",
        "explicit test failure",
        None,
        None,
        None,
        None
    ),
];

impl AssertionMatcher {
    pub const fn spec(self) -> &'static MatcherSpec {
        &MATCHER_SPECS[self as usize]
    }

    pub const fn fact_name(self) -> &'static str {
        self.spec().fact_name
    }

    pub const fn source_name(self) -> &'static str {
        self.spec().source_name
    }

    pub const fn minimum_arity(self) -> usize {
        self.spec().minimum_arity
    }

    pub const fn maximum_arity(self) -> usize {
        self.spec().maximum_arity
    }

    pub const fn accepts_arity(self, arity: usize) -> bool {
        arity >= self.minimum_arity() && arity <= self.maximum_arity()
    }

    pub const fn domain(self) -> MatcherDomain {
        self.spec().domain
    }

    pub const fn expected_operand(self) -> ExpectedOperandRule {
        self.spec().expected_operand
    }

    pub const fn negation_supported(self) -> bool {
        self.spec().negation_supported
    }

    pub const fn difference_strategy(self) -> DifferenceStrategy {
        self.spec().difference
    }

    pub const fn stable_complexity(self) -> Option<&'static str> {
        self.spec().stable_complexity
    }

    pub const fn is_ordered(self) -> bool {
        matches!(self.domain(), MatcherDomain::Ordered)
    }

    pub const fn is_string(self) -> bool {
        matches!(self.domain(), MatcherDomain::String)
    }
}

pub fn matcher_candidates(name: &str) -> impl Iterator<Item = AssertionMatcher> + '_ {
    MATCHER_SPECS
        .iter()
        .filter(move |spec| spec.matcher != AssertionMatcher::Fail && spec.source_name == name)
        .map(|spec| spec.matcher)
}

pub fn matcher_from_fact_name(name: &str) -> Option<AssertionMatcher> {
    MATCHER_SPECS
        .iter()
        .find(|spec| spec.fact_name == name)
        .map(|spec| spec.matcher)
}

pub const fn stable_message(matcher: AssertionMatcher, negated: bool) -> &'static str {
    if negated {
        matcher.spec().negative_message
    } else {
        matcher.spec().positive_message
    }
}

pub const fn stable_difference(matcher: AssertionMatcher, negated: bool) -> Option<&'static str> {
    if negated {
        matcher.spec().negative_difference
    } else {
        matcher.spec().positive_difference
    }
}

pub const PRESENTATION_LIMIT: usize = 4096;
pub const COLLECTION_PRESENTATION_ITEMS: usize = 8;
pub const TRUNCATION_MARKER: &str = "...<truncated>";

pub fn quote_string(value: &str) -> String {
    let mut result = String::with_capacity(value.len().min(PRESENTATION_LIMIT));
    result.push('"');
    let content_limit = PRESENTATION_LIMIT - 2 - TRUNCATION_MARKER.len();
    let mut truncated = false;
    for character in value.chars() {
        let escaped = match character {
            '"' => "\\\"".to_string(),
            '\\' => "\\\\".to_string(),
            '\n' => "\\n".to_string(),
            '\r' => "\\r".to_string(),
            '\t' => "\\t".to_string(),
            character if character.is_control() => format!("\\u{:04x}", u32::from(character)),
            character => character.to_string(),
        };
        if result.len() + escaped.len() + 1 > PRESENTATION_LIMIT {
            truncated = true;
            break;
        }
        if result.len() - 1 + escaped.len() > content_limit && value.len() > content_limit {
            truncated = true;
            break;
        }
        result.push_str(&escaped);
    }
    if truncated {
        result.push_str(TRUNCATION_MARKER);
    }
    result.push('"');
    result
}

pub fn string_difference(actual: &str, expected: &str, mode: u8) -> String {
    let actual_boundaries = doria_unicode::grapheme_boundaries(actual).collect::<Vec<_>>();
    let expected_boundaries = doria_unicode::grapheme_boundaries(expected).collect::<Vec<_>>();
    let actual_count = actual_boundaries.len().saturating_sub(1);
    let expected_count = expected_boundaries.len().saturating_sub(1);
    let common = actual_count.min(expected_count);
    let first_difference = (0..common)
        .find(|index| {
            actual[actual_boundaries[*index]..actual_boundaries[*index + 1]]
                != expected[expected_boundaries[*index]..expected_boundaries[*index + 1]]
        })
        .unwrap_or(common);
    let relation = match mode {
        1 => "Prefix",
        2 => "Suffix",
        _ => "Value",
    };
    bound_text(format!(
        "First Differing Grapheme: {first_difference}\nExpected {relation}: {}\nActual {relation}: {}\nExpected Grapheme Length: {expected_count}\nActual Grapheme Length: {actual_count}",
        quote_string(expected),
        quote_string(actual),
    ))
}

pub fn bytes_difference(actual: &[u8], expected: &[u8]) -> String {
    let common = actual.len().min(expected.len());
    if let Some(index) = (0..common).find(|index| actual[*index] != expected[*index]) {
        return format!(
            "First Differing Byte: {index}\nExpected Byte: {:02x}\nActual Byte: {:02x}",
            expected[index], actual[index]
        );
    }
    format!(
        "Expected Byte Length: {}\nActual Byte Length: {}\nDelta: {}",
        expected.len(),
        actual.len(),
        (actual.len() as i128) - (expected.len() as i128),
    )
}

pub fn count_difference(actual: i64, expected: i64) -> String {
    format!(
        "Expected Count: {expected}\nActual Count: {actual}\nDelta: {}",
        i128::from(actual) - i128::from(expected),
    )
}

pub fn error_presentation(error_type: &str, message: &str) -> String {
    let mut result = format!("{error_type}: ");
    for character in message.chars() {
        match character {
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            '\\' => result.push_str("\\\\"),
            character if character.is_control() => {
                use std::fmt::Write as _;
                let _ = write!(result, "\\u{:04x}", u32::from(character));
            }
            character => result.push(character),
        }
    }
    bound_text(result)
}

pub fn bound_text(mut value: String) -> String {
    if value.len() <= PRESENTATION_LIMIT {
        return value;
    }
    let mut limit = PRESENTATION_LIMIT - TRUNCATION_MARKER.len();
    while !value.is_char_boundary(limit) {
        limit -= 1;
    }
    value.truncate(limit);
    value.push_str(TRUNCATION_MARKER);
    value
}
