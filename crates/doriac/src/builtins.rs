#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    Panic,
    ReadLine,
    Sprintf,
    Printf,
    ReadFile,
    WriteFile,
    WriteStderr,
}

const RESERVED_FUTURE_INTRINSIC_NAMES: &[&str] = &[
    "append_file",
    "read_file_bytes",
    "write_file_bytes",
    "append_file_bytes",
];

/// A PHP free-function spelling and its Doria naming-charter replacement.
///
/// This table is compiler-owned data so diagnostics and the future PHP
/// migration command can teach the same spellings without duplicating policy.
pub const PHP_FUNCTION_SUGGESTIONS: &[(&str, &str)] = &[("readline", "read_line")];

/// Compiler-owned policy for migrating PHP double-quoted string segments.
///
/// A future migration command can combine this policy with
/// `PHP_FUNCTION_SUGGESTIONS` without inventing a second mapping source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhpDoubleQuotedStringMigration {
    pub literal_open_brace: char,
    pub doria_literal_open_brace: &'static str,
    pub rewrite_bare_close_brace: bool,
}

pub const PHP_DOUBLE_QUOTED_STRING_MIGRATION: PhpDoubleQuotedStringMigration =
    PhpDoubleQuotedStringMigration {
        literal_open_brace: '{',
        doria_literal_open_brace: "\\{",
        rewrite_bare_close_brace: false,
    };

pub fn php_function_suggestion(name: &str) -> Option<&'static str> {
    PHP_FUNCTION_SUGGESTIONS
        .iter()
        .find_map(|(php, doria)| (*php == name).then_some(*doria))
}

pub fn is_reserved_intrinsic_name(name: &str) -> bool {
    Builtin::from_name(name).is_some() || RESERVED_FUTURE_INTRINSIC_NAMES.contains(&name)
}

impl Builtin {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "panic" => Some(Self::Panic),
            "read_line" => Some(Self::ReadLine),
            "sprintf" => Some(Self::Sprintf),
            "printf" => Some(Self::Printf),
            "read_file" => Some(Self::ReadFile),
            "write_file" => Some(Self::WriteFile),
            "write_stderr" => Some(Self::WriteStderr),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Panic => "panic",
            Self::ReadLine => "read_line",
            Self::Sprintf => "sprintf",
            Self::Printf => "printf",
            Self::ReadFile => "read_file",
            Self::WriteFile => "write_file",
            Self::WriteStderr => "write_stderr",
        }
    }

    pub const fn return_is_non_null(self) -> Option<bool> {
        match self {
            Self::Sprintf | Self::ReadFile => Some(true),
            Self::ReadLine => Some(false),
            Self::Panic | Self::Printf | Self::WriteFile | Self::WriteStderr => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_php_spelling_and_string_rewrites_in_one_migration_source() {
        let string_policy = std::hint::black_box(PHP_DOUBLE_QUOTED_STRING_MIGRATION);
        assert_eq!(php_function_suggestion("readline"), Some("read_line"));
        assert_eq!(string_policy.literal_open_brace, '{');
        assert_eq!(string_policy.doria_literal_open_brace, "\\{");
        assert!(!string_policy.rewrite_bare_close_brace);
    }

    #[test]
    fn reserves_future_intrinsics_without_implementing_them() {
        for name in RESERVED_FUTURE_INTRINSIC_NAMES {
            assert!(is_reserved_intrinsic_name(name));
            assert_eq!(Builtin::from_name(name), None);
        }
        assert!(is_reserved_intrinsic_name("read_file"));
        assert!(!is_reserved_intrinsic_name("user_function"));
    }
}
