use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let package_version = env::var("CARGO_PKG_VERSION").expect("Cargo package version");
    let toolchain_version = canonical_toolchain_version(&package_version);
    println!("cargo:rustc-env=DORIA_TOOLCHAIN_VERSION={toolchain_version}");

    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let runtime_dir = manifest_dir.join("../doria-rt");
    let runtime_source_dir = runtime_dir.join("src");
    let runtime_source = runtime_source_dir.join("lib.rs");
    let runtime_manifest = runtime_dir.join("Cargo.toml");
    println!("cargo:rerun-if-changed={}", runtime_source_dir.display());
    println!("cargo:rerun-if-changed={}", runtime_manifest.display());

    if env::var_os("CARGO_FEATURE_BUNDLED_RUNTIME").is_none() {
        return;
    }

    let target = env::var("TARGET").expect("Cargo target triple");
    let filename = if target.ends_with("windows-msvc") {
        "doria_rt.lib"
    } else {
        "libdoria_rt.a"
    };
    let output =
        PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory")).join(filename);
    let dependency_dir = output
        .parent()
        .and_then(|path| path.parent())
        .and_then(|path| path.parent())
        .and_then(|path| path.parent())
        .expect("Cargo build directory")
        .join("deps");
    let ryu = std::fs::read_dir(&dependency_dir)
        .expect("Cargo dependency directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("libryu-") && name.ends_with(".rlib"))
        })
        .expect("compiled ryu dependency");
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc"));
    let mut command = Command::new(rustc);
    command
        .arg("--crate-name")
        .arg("doria_rt")
        .arg("--edition=2021")
        .arg("--crate-type=staticlib")
        .arg("--target")
        .arg(&target)
        .arg("-L")
        .arg(format!("dependency={}", dependency_dir.display()))
        .arg("--extern")
        .arg(format!("ryu={}", ryu.display()))
        .arg("-C")
        .arg(format!(
            "opt-level={}",
            env::var("OPT_LEVEL").unwrap_or_else(|_| "0".to_string())
        ));
    if let Some(encoded_flags) = env::var_os("CARGO_ENCODED_RUSTFLAGS") {
        for flag in encoded_flags.to_string_lossy().split('\u{1f}') {
            if !flag.is_empty() {
                command.arg(flag);
            }
        }
    }
    let status = command
        .arg("-C")
        .arg("panic=abort")
        .arg("-o")
        .arg(&output)
        .arg(&runtime_source)
        .status()
        .expect("failed to invoke rustc for doria-rt");
    assert!(status.success(), "failed to build doria-rt static library");

    println!("cargo:rustc-env=DORIA_RT_BUILT_PATH={}", output.display());
}

fn canonical_toolchain_version(package_version: &str) -> String {
    let mut components = package_version.splitn(3, '.');
    let year = components.next().expect("toolchain version year");
    let month = components
        .next()
        .expect("toolchain version month")
        .parse::<u8>()
        .expect("numeric toolchain version month");
    let release = components.next().expect("toolchain version release");
    assert!(
        year.len() == 4 && year.bytes().all(|byte| byte.is_ascii_digit()),
        "toolchain version year must use four digits"
    );
    assert!(
        (1..=12).contains(&month),
        "toolchain version month must be between 1 and 12"
    );
    let release_number = release
        .split_once('-')
        .map_or(release, |(number, _)| number);
    assert!(
        !release_number.is_empty() && release_number.bytes().all(|byte| byte.is_ascii_digit()),
        "toolchain release number must be numeric"
    );
    format!("{year}.{month:02}.{release}")
}
