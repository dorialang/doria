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

/// A PHP free-function spelling and its Doria naming-charter replacement.
///
/// This table is compiler-owned data so diagnostics and the future PHP
/// migration command can teach the same spellings without duplicating policy.
pub const PHP_FUNCTION_SUGGESTIONS: &[(&str, &str)] = &[("readline", "read_line")];

pub fn php_function_suggestion(name: &str) -> Option<&'static str> {
    PHP_FUNCTION_SUGGESTIONS
        .iter()
        .find_map(|(php, doria)| (*php == name).then_some(*doria))
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
}
