#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    Fail,
}

impl AssertionMatcher {
    pub const fn fact_name(self) -> &'static str {
        match self {
            Self::Equal => "Equal",
            Self::Null => "Null",
            Self::True => "True",
            Self::False => "False",
            Self::GreaterThan => "GreaterThan",
            Self::GreaterThanOrEqual => "GreaterThanOrEqual",
            Self::LessThan => "LessThan",
            Self::LessThanOrEqual => "LessThanOrEqual",
            Self::StringContains => "StringContains",
            Self::StringStartsWith => "StringStartsWith",
            Self::StringEndsWith => "StringEndsWith",
            Self::StringEmpty => "StringEmpty",
            Self::Fail => "Fail",
        }
    }

    pub const fn source_name(self) -> &'static str {
        match self {
            Self::Equal => "toEqual",
            Self::Null => "toBeNull",
            Self::True => "toBeTrue",
            Self::False => "toBeFalse",
            Self::GreaterThan => "toBeGreaterThan",
            Self::GreaterThanOrEqual => "toBeGreaterThanOrEqual",
            Self::LessThan => "toBeLessThan",
            Self::LessThanOrEqual => "toBeLessThanOrEqual",
            Self::StringContains => "toContain",
            Self::StringStartsWith => "toStartWith",
            Self::StringEndsWith => "toEndWith",
            Self::StringEmpty => "toBeEmpty",
            Self::Fail => "fail",
        }
    }

    pub const fn expected_arity(self) -> usize {
        match self {
            Self::Equal
            | Self::GreaterThan
            | Self::GreaterThanOrEqual
            | Self::LessThan
            | Self::LessThanOrEqual
            | Self::StringContains
            | Self::StringStartsWith
            | Self::StringEndsWith
            | Self::Fail => 1,
            Self::Null | Self::True | Self::False | Self::StringEmpty => 0,
        }
    }

    pub const fn is_ordered(self) -> bool {
        matches!(
            self,
            Self::GreaterThan | Self::GreaterThanOrEqual | Self::LessThan | Self::LessThanOrEqual
        )
    }

    pub const fn is_string(self) -> bool {
        matches!(
            self,
            Self::StringContains
                | Self::StringStartsWith
                | Self::StringEndsWith
                | Self::StringEmpty
        )
    }
}

pub const MATCHERS: [AssertionMatcher; 12] = [
    AssertionMatcher::Equal,
    AssertionMatcher::Null,
    AssertionMatcher::True,
    AssertionMatcher::False,
    AssertionMatcher::GreaterThan,
    AssertionMatcher::GreaterThanOrEqual,
    AssertionMatcher::LessThan,
    AssertionMatcher::LessThanOrEqual,
    AssertionMatcher::StringContains,
    AssertionMatcher::StringStartsWith,
    AssertionMatcher::StringEndsWith,
    AssertionMatcher::StringEmpty,
];

pub const FUTURE_MATCHERS: [&str; 4] = ["toHaveCount", "toHaveKey", "toHaveValue", "toThrow"];

pub fn matcher_from_source_name(name: &str) -> Option<AssertionMatcher> {
    MATCHERS
        .iter()
        .copied()
        .find(|matcher| matcher.source_name() == name)
}

pub fn matcher_from_fact_name(name: &str) -> Option<AssertionMatcher> {
    MATCHERS
        .iter()
        .copied()
        .chain(core::iter::once(AssertionMatcher::Fail))
        .find(|matcher| matcher.fact_name() == name)
}

pub fn is_future_matcher(name: &str) -> bool {
    FUTURE_MATCHERS.contains(&name)
}

pub const fn stable_message(matcher: AssertionMatcher, negated: bool) -> &'static str {
    match (matcher, negated) {
        (AssertionMatcher::Equal, false) => "expected values to be equal",
        (AssertionMatcher::Equal, true) => "expected values not to be equal",
        (AssertionMatcher::Null, false) => "expected value to be null",
        (AssertionMatcher::Null, true) => "expected value not to be null",
        (AssertionMatcher::True, false) => "expected value to be true",
        (AssertionMatcher::True, true) => "expected value not to be true",
        (AssertionMatcher::False, false) => "expected value to be false",
        (AssertionMatcher::False, true) => "expected value not to be false",
        (AssertionMatcher::GreaterThan, false) => {
            "expected value to be greater than the comparison value"
        }
        (AssertionMatcher::GreaterThan, true) => {
            "expected value not to be greater than the comparison value"
        }
        (AssertionMatcher::GreaterThanOrEqual, false) => {
            "expected value to be greater than or equal to the comparison value"
        }
        (AssertionMatcher::GreaterThanOrEqual, true) => {
            "expected value not to be greater than or equal to the comparison value"
        }
        (AssertionMatcher::LessThan, false) => {
            "expected value to be less than the comparison value"
        }
        (AssertionMatcher::LessThan, true) => {
            "expected value not to be less than the comparison value"
        }
        (AssertionMatcher::LessThanOrEqual, false) => {
            "expected value to be less than or equal to the comparison value"
        }
        (AssertionMatcher::LessThanOrEqual, true) => {
            "expected value not to be less than or equal to the comparison value"
        }
        (AssertionMatcher::StringContains, false) => "expected string to contain the fragment",
        (AssertionMatcher::StringContains, true) => "expected string not to contain the fragment",
        (AssertionMatcher::StringStartsWith, false) => "expected string to start with the prefix",
        (AssertionMatcher::StringStartsWith, true) => {
            "expected string not to start with the prefix"
        }
        (AssertionMatcher::StringEndsWith, false) => "expected string to end with the suffix",
        (AssertionMatcher::StringEndsWith, true) => "expected string not to end with the suffix",
        (AssertionMatcher::StringEmpty, false) => "expected string to be empty",
        (AssertionMatcher::StringEmpty, true) => "expected string not to be empty",
        (AssertionMatcher::Fail, _) => "explicit test failure",
    }
}

pub const PRESENTATION_LIMIT: usize = 4096;
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
