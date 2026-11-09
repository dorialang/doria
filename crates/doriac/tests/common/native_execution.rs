use doriac::{backend::NativeProfile, mir::Program};

#[path = "native_execution_runner.rs"]
mod runner;
pub use runner::*;

pub fn assert_native_execution(mir: &Program, profile: NativeProfile, stdout: &str) {
    let result = execute_native(mir, profile);
    assert_eq!(result.status.code(), Some(0), "{profile:?}: {result:?}");
    assert_eq!(result.stderr, b"", "{profile:?}");
    assert_eq!(result.stdout, stdout.as_bytes(), "{profile:?}");
}
