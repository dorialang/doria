#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    Panic,
    ReadLine,
    Sprintf,
    Printf,
    ReadFile,
    WriteFile,
    AppendFile,
    WriteStderr,
    ReadFileBytes,
    WriteFileBytes,
    AppendFileBytes,
    ReadStdinBytes,
    WriteStdoutBytes,
    WriteStderrBytes,
}

const RESERVED_FUTURE_INTRINSIC_NAMES: &[&str] = &[];

pub const ECHO_CHECKED_ERROR_TYPES: &[&str] = &[crate::compiler_known_io::IO_ERROR];

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
    pub const ALL: [Self; 14] = [
        Self::Panic,
        Self::ReadLine,
        Self::Sprintf,
        Self::Printf,
        Self::ReadFile,
        Self::WriteFile,
        Self::AppendFile,
        Self::WriteStderr,
        Self::ReadFileBytes,
        Self::WriteFileBytes,
        Self::AppendFileBytes,
        Self::ReadStdinBytes,
        Self::WriteStdoutBytes,
        Self::WriteStderrBytes,
    ];

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "panic" => Some(Self::Panic),
            "read_line" => Some(Self::ReadLine),
            "sprintf" => Some(Self::Sprintf),
            "printf" => Some(Self::Printf),
            "read_file" => Some(Self::ReadFile),
            "write_file" => Some(Self::WriteFile),
            "append_file" => Some(Self::AppendFile),
            "write_stderr" => Some(Self::WriteStderr),
            "read_file_bytes" => Some(Self::ReadFileBytes),
            "write_file_bytes" => Some(Self::WriteFileBytes),
            "append_file_bytes" => Some(Self::AppendFileBytes),
            "read_stdin_bytes" => Some(Self::ReadStdinBytes),
            "write_stdout_bytes" => Some(Self::WriteStdoutBytes),
            "write_stderr_bytes" => Some(Self::WriteStderrBytes),
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
            Self::AppendFile => "append_file",
            Self::WriteStderr => "write_stderr",
            Self::ReadFileBytes => "read_file_bytes",
            Self::WriteFileBytes => "write_file_bytes",
            Self::AppendFileBytes => "append_file_bytes",
            Self::ReadStdinBytes => "read_stdin_bytes",
            Self::WriteStdoutBytes => "write_stdout_bytes",
            Self::WriteStderrBytes => "write_stderr_bytes",
        }
    }

    /// Canonical source-facing signature used by compiler-adjacent tooling.
    pub const fn signature(self) -> &'static str {
        match self {
            Self::Panic => "panic(string $message)",
            Self::ReadLine => concat!(
                "read_line(string $prompt = \"\"): ?string throws ",
                "Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error"
            ),
            Self::Sprintf => "sprintf(string $format, ...): string",
            Self::Printf => "printf(string $format, ...): void throws Doria\\Std\\Io\\IoError",
            Self::ReadFile => concat!(
                "read_file(string $path): string throws ",
                "Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error"
            ),
            Self::WriteFile => concat!(
                "write_file(string $path, string $contents): void throws ",
                "Doria\\Std\\Io\\IoError"
            ),
            Self::AppendFile => concat!(
                "append_file(string $path, string $contents): void throws ",
                "Doria\\Std\\Io\\IoError"
            ),
            Self::WriteStderr => concat!(
                "write_stderr(string $value): void throws ",
                "Doria\\Std\\Io\\IoError"
            ),
            Self::ReadFileBytes => concat!(
                "read_file_bytes(string $path): Bytes throws ",
                "Doria\\Std\\Io\\IoError"
            ),
            Self::WriteFileBytes => concat!(
                "write_file_bytes(string $path, Bytes $contents): void throws ",
                "Doria\\Std\\Io\\IoError"
            ),
            Self::AppendFileBytes => concat!(
                "append_file_bytes(string $path, Bytes $contents): void throws ",
                "Doria\\Std\\Io\\IoError"
            ),
            Self::ReadStdinBytes => "read_stdin_bytes(): Bytes throws Doria\\Std\\Io\\IoError",
            Self::WriteStdoutBytes => concat!(
                "write_stdout_bytes(Bytes $contents): void throws ",
                "Doria\\Std\\Io\\IoError"
            ),
            Self::WriteStderrBytes => concat!(
                "write_stderr_bytes(Bytes $contents): void throws ",
                "Doria\\Std\\Io\\IoError"
            ),
        }
    }

    /// The accepted argument count as an inclusive `(minimum, maximum)` range, or
    /// `None` when the builtin is variadic.
    ///
    /// This is the one compiler-owned arity definition. A builtin with an optional
    /// parameter — `read_line(string $prompt = ""): ?string` — is one function with
    /// a range, never an overload pair, so the omitted argument is filled with the
    /// canonical default rather than selecting a second operation.
    pub const fn arity(self) -> Option<(usize, usize)> {
        match self {
            Self::ReadLine => Some((0, 1)),
            Self::ReadStdinBytes => Some((0, 0)),
            Self::ReadFile
            | Self::ReadFileBytes
            | Self::WriteStderr
            | Self::WriteStdoutBytes
            | Self::WriteStderrBytes => Some((1, 1)),
            Self::WriteFile | Self::AppendFile | Self::WriteFileBytes | Self::AppendFileBytes => {
                Some((2, 2))
            }
            Self::Sprintf | Self::Printf | Self::Panic => None,
        }
    }

    pub const fn return_is_non_null(self) -> Option<bool> {
        match self {
            Self::Sprintf | Self::ReadFile | Self::ReadFileBytes | Self::ReadStdinBytes => {
                Some(true)
            }
            Self::ReadLine => Some(false),
            Self::Panic
            | Self::Printf
            | Self::WriteFile
            | Self::AppendFile
            | Self::WriteStderr
            | Self::WriteFileBytes
            | Self::AppendFileBytes
            | Self::WriteStdoutBytes
            | Self::WriteStderrBytes => None,
        }
    }

    pub const fn returns_owned_bytes(self) -> bool {
        matches!(self, Self::ReadFileBytes | Self::ReadStdinBytes)
    }

    pub const fn uses_bytes(self) -> bool {
        matches!(
            self,
            Self::ReadFileBytes
                | Self::WriteFileBytes
                | Self::AppendFileBytes
                | Self::ReadStdinBytes
                | Self::WriteStdoutBytes
                | Self::WriteStderrBytes
        )
    }

    /// Canonical checked Error identities contributed by this builtin.
    ///
    /// Semantic checking and compiler-known type activation both consume this
    /// table so a builtin cannot acquire an I/O effect in only one phase.
    pub const fn checked_error_types(self) -> &'static [&'static str] {
        match self {
            Self::Panic | Self::Sprintf => &[],
            Self::ReadLine | Self::ReadFile => &[
                crate::compiler_known_io::IO_ERROR,
                crate::compiler_known_io::INVALID_UTF8_ERROR,
            ],
            Self::Printf
            | Self::WriteFile
            | Self::AppendFile
            | Self::WriteStderr
            | Self::ReadFileBytes
            | Self::WriteFileBytes
            | Self::AppendFileBytes
            | Self::ReadStdinBytes
            | Self::WriteStdoutBytes
            | Self::WriteStderrBytes => &[crate::compiler_known_io::IO_ERROR],
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
    fn reserves_intrinsics() {
        for name in ["append_file", "read_file_bytes", "write_stdout_bytes"] {
            assert!(is_reserved_intrinsic_name(name));
            assert!(Builtin::from_name(name).is_some());
        }
        assert!(is_reserved_intrinsic_name("read_file"));
        assert!(!is_reserved_intrinsic_name("user_function"));
    }

    #[test]
    fn checked_io_effects_are_owned_by_the_builtin_table() {
        assert!(Builtin::Sprintf.checked_error_types().is_empty());
        assert_eq!(
            Builtin::ReadLine.checked_error_types(),
            &[
                crate::compiler_known_io::IO_ERROR,
                crate::compiler_known_io::INVALID_UTF8_ERROR
            ]
        );
        assert_eq!(
            Builtin::WriteStdoutBytes.checked_error_types(),
            &[crate::compiler_known_io::IO_ERROR]
        );
        assert_eq!(
            ECHO_CHECKED_ERROR_TYPES,
            &[crate::compiler_known_io::IO_ERROR]
        );
        assert_eq!(
            Builtin::ReadLine.signature(),
            concat!(
                "read_line(string $prompt = \"\"): ?string throws ",
                "Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error"
            )
        );
    }
}
