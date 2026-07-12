#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    Panic,
    Readline,
    Sprintf,
    Printf,
    ReadFile,
    WriteFile,
    WriteStderr,
}

impl Builtin {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "panic" => Some(Self::Panic),
            "readline" => Some(Self::Readline),
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
            Self::Readline => "readline",
            Self::Sprintf => "sprintf",
            Self::Printf => "printf",
            Self::ReadFile => "read_file",
            Self::WriteFile => "write_file",
            Self::WriteStderr => "write_stderr",
        }
    }
}
