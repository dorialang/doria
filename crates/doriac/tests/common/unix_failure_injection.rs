//! Real native status failures, confined to generated test child processes.

use super::native_execution;

use doriac::backend::{BackendTarget, NativeProfile};

pub fn assert_execution(source: &str, mode: &str, control: &str, injected: &str) {
    let program = doriac::lower_source_to_mir("runtime-failure.doria", source).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    let directory = native_execution::ExecutionDirectory::new();
    #[cfg(target_os = "macos")]
    let (library_name, loader, compiler_flags): (&str, &str, &[&str]) = (
        "runtime-failure.dylib",
        "DYLD_INSERT_LIBRARIES",
        &["-dynamiclib"],
    );
    #[cfg(target_os = "linux")]
    let (library_name, loader, compiler_flags): (&str, &str, &[&str]) = (
        "runtime-failure.so",
        "LD_PRELOAD",
        &["-shared", "-fPIC", "-ldl"],
    );
    let library = directory.path().join(library_name);
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/runtime_io_failure_unix.c");
    let build = std::process::Command::new("cc")
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror"])
        .arg(fixture)
        .args(compiler_flags)
        .arg("-o")
        .arg(&library)
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(build.status.success(), "{build:?}");
    assert!(build.stdout.is_empty(), "{build:?}");
    assert!(build.stderr.is_empty(), "{build:?}");
    let profiles = [
        NativeProfile::Fast,
        #[cfg(feature = "llvm-backend")]
        NativeProfile::Release,
    ];
    for profile in profiles {
        let output =
            native_execution::execute_native_in_directory(&program, profile, &[], &directory);
        assert_eq!(output.status.code(), Some(0), "{profile:?}: {output:?}");
        assert!(output.stderr.is_empty(), "{profile:?}: {output:?}");
        assert_eq!(output.stdout, control.as_bytes());
        let output = native_execution::execute_native_in_directory_with_env(
            &program,
            profile,
            &[],
            &directory,
            &[
                (loader, library.as_os_str()),
                ("DORIA_TEST_IO_FAILURE", std::ffi::OsStr::new(mode)),
            ],
        );
        assert_eq!(output.status.code(), Some(0), "{profile:?}: {output:?}");
        assert!(output.stderr.is_empty(), "{profile:?}: {output:?}");
        assert_eq!(output.stdout, injected.as_bytes());
    }
    let artifact =
        doriac::compile_source("runtime-failure.doria", source, BackendTarget::Php).unwrap();
    let path = directory.path().join("runtime-failure.php");
    doriac::backend::publish_output(&path, artifact).unwrap();
    for injecting in [false, true] {
        let mut command = std::process::Command::new("php");
        command
            .args(["-d", "ffi.enable=1"])
            .arg(&path)
            .current_dir(directory.path());
        if injecting {
            command
                .env(loader, &library)
                .env("DORIA_TEST_IO_FAILURE", mode);
        }
        let output = command.output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(0),
            "injecting={injecting}: {output:?}"
        );
        assert!(
            output.stderr.is_empty(),
            "injecting={injecting}: {output:?}"
        );
        assert_eq!(
            output.stdout,
            if injecting { injected } else { control }.as_bytes()
        );
    }
}
