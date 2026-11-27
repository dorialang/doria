//! A fixture's `link` file: the `--link-*` flags each host applies to link
//! a native library, read into the same `NativeLinking` that `doriac
//! compile` builds from its flags.

use std::path::Path;

use doriac::native_link::NativeLinking;

/// The link facts of a fixture's `link` file for this host: lines of
/// `<linux|macos|windows> <flag> <library>=<value>`, applied as
/// `doriac compile` applies its link flags, with relative paths resolved
/// against `directory`. A fixture without the file links no library.
pub fn read_linking(path: &Path, directory: &Path) -> NativeLinking {
    let mut linking = NativeLinking::default();
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return linking,
        Err(error) => panic!("failed to read {}: {error}", path.display()),
    };
    for line in text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        let [system, flag, value] = fields.as_slice() else {
            panic!(
                "{}: `{line}` should be `<linux|macos|windows> <flag> <library>=<value>`",
                path.display()
            );
        };
        assert!(
            matches!(*system, "linux" | "macos" | "windows"),
            "{}: `{system}` should be `linux`, `macos` or `windows`",
            path.display()
        );
        if *system == std::env::consts::OS {
            linking
                .apply_flag(flag, value, directory)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        }
    }
    linking
        .finish_flags()
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    linking
}
