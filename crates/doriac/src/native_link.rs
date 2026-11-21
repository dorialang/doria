//! Foreign library link facts (Decision 0148 sections 18 to 21, with the
//! amendment of 2026-11-20).
//!
//! Source names a logical library with `#[Library("name")]`. Its resolved
//! link inputs reach `doriac` either per package in build-plan schema 2 or
//! through the direct `--link-*` flags, and become one [`NativeLinking`]. At
//! the native link, [`NativeLinking::resolve`] checks that every logical
//! library emitted code references has inputs, and gives the typed
//! [`LinkPlan`] the linker arguments are made from. `doriac` never receives
//! raw linker arguments.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::c_layout::{CTarget, TargetOperatingSystem};
use crate::diagnostics::Diagnostic;
use crate::source::{SourceId, Span};

/// The diagnostic for a logical library emitted code references with no
/// resolved link input.
pub const MISSING_INPUT_CODE: &str = "E0804";
/// The diagnostic for a resolved link input this target cannot use.
pub const UNUSABLE_INPUT_CODE: &str = "E0805";

/// How a logical library links: `"dynamic"` or `"static"`.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum LinkMode {
    #[default]
    Dynamic,
    Static,
}

/// The Windows subsystem a binary target links for (section 21):
/// `"console"` or `"gui"`. Other targets ignore it.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum WindowsSubsystem {
    #[default]
    Console,
    Gui,
}

impl WindowsSubsystem {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "console" => Some(Self::Console),
            "gui" => Some(Self::Gui),
            _ => None,
        }
    }
}

/// One resolved link input of a logical library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum LinkInput {
    /// A library linked by name through the search paths: `z` is `-lz`,
    /// and `z.lib` on Windows.
    Library { name: String },
    /// A directory searched for libraries.
    SearchPath { path: PathBuf },
    /// A macOS framework.
    Framework { name: String },
    /// A library or object file linked by path.
    File { path: PathBuf },
}

/// The resolved link facts of one logical library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeLibrary {
    pub name: String,
    pub link: LinkMode,
    pub inputs: Vec<LinkInput>,
    /// Files the linked artifact loads at run time, placed beside it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime_files: Vec<PathBuf>,
}

impl NativeLibrary {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            link: LinkMode::Dynamic,
            inputs: Vec::new(),
            runtime_files: Vec::new(),
        }
    }

    /// Every problem with this library's facts, as plain messages. Paths
    /// are checked by their owner: package-relative in a build plan,
    /// absolute from the flags.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.name.is_empty() {
            problems.push("a native library needs a nonempty logical name".to_string());
        }
        // A search path alone links nothing.
        if !self.inputs.iter().any(|input| {
            matches!(
                input,
                LinkInput::Library { .. } | LinkInput::Framework { .. } | LinkInput::File { .. }
            )
        }) {
            problems.push(format!(
                "native library `{}` needs at least one library, framework or file input",
                self.name
            ));
        }
        for input in &self.inputs {
            let (kind, name) = match input {
                LinkInput::Library { name } => ("library", name),
                LinkInput::Framework { name } => ("framework", name),
                LinkInput::SearchPath { .. } | LinkInput::File { .. } => continue,
            };
            if !is_link_name(name) {
                problems.push(format!(
                    "native library `{}` has the {kind} name `{name}`, which is not a library name: use letters, digits, `_`, `-`, `+` and `.`, starting with a letter, digit or `_`",
                    self.name
                ));
            }
        }
        problems
    }
}

/// Whether `name` names a library or framework and cannot be read as a
/// linker option.
fn is_link_name(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphanumeric() || first == '_')
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '+' | '.')
        })
}

/// The link facts of one compilation: each package's logical libraries,
/// or the direct flags' libraries for every source, and the Windows
/// subsystem.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeLinking {
    /// By owning package (`None` for the direct flags) and logical name.
    libraries: BTreeMap<(Option<String>, String), NativeLibrary>,
    /// The package of each build-plan source. Empty for direct use, where
    /// every source shares the flags' libraries.
    source_packages: BTreeMap<SourceId, String>,
    /// The link mode each `--link-library` flag stated, to reject two
    /// flags that disagree.
    stated_modes: BTreeMap<String, LinkMode>,
    pub windows_subsystem: WindowsSubsystem,
}

/// The direct `doriac compile` and `doriac run` flags that carry link facts
/// (section 18). Each takes one value.
pub const LINK_FLAGS: [&str; 6] = [
    "--link-library",
    "--link-search",
    "--link-framework",
    "--link-file",
    "--runtime-file",
    "--windows-subsystem",
];

impl NativeLinking {
    /// The link facts of a build plan's packages, with package-relative
    /// paths resolved against each package's `root`.
    pub fn for_packages<'a>(
        packages: impl IntoIterator<Item = (&'a str, &'a Path, &'a [NativeLibrary])>,
        source_packages: BTreeMap<SourceId, String>,
        windows_subsystem: WindowsSubsystem,
    ) -> Self {
        let mut libraries = BTreeMap::new();
        for (package, root, declared) in packages {
            for library in declared {
                let mut library = library.clone();
                for input in &mut library.inputs {
                    if let LinkInput::SearchPath { path } | LinkInput::File { path } = input {
                        *path = root.join(&*path);
                    }
                }
                for file in &mut library.runtime_files {
                    *file = root.join(&*file);
                }
                libraries.insert((Some(package.to_string()), library.name.clone()), library);
            }
        }
        Self {
            libraries,
            source_packages,
            stated_modes: BTreeMap::new(),
            windows_subsystem,
        }
    }

    /// Applies one direct flag of [`LINK_FLAGS`]; a relative path resolves
    /// against `directory`, the invocation's working directory.
    pub fn apply_flag(&mut self, flag: &str, value: &str, directory: &Path) -> Result<(), String> {
        if flag == "--windows-subsystem" {
            self.windows_subsystem = WindowsSubsystem::parse(value).ok_or_else(|| {
                format!("unknown Windows subsystem `{value}`; expected `console` or `gui`")
            })?;
            return Ok(());
        }
        let Some((name, fact)) = value.split_once('=') else {
            return Err(format!(
                "`{flag} {value}` names no logical library; write `{flag} <library>=<value>`, such as `--link-library zlib=z`"
            ));
        };
        if name.is_empty() || fact.is_empty() {
            return Err(format!(
                "`{flag} {value}` needs a logical library name and a value, such as `--link-library zlib=z`"
            ));
        }
        let path = || directory.join(fact);
        let input = match flag {
            "--link-library" => {
                let (mode, library) = match fact.strip_prefix("static:") {
                    Some(library) => (LinkMode::Static, library),
                    None => (LinkMode::Dynamic, fact),
                };
                if let Some(previous) = self.stated_modes.insert(name.to_string(), mode) {
                    if previous != mode {
                        return Err(format!(
                            "`--link-library` links `{name}` both statically and dynamically; give every `--link-library {name}=...` the same `static:` prefix or none"
                        ));
                    }
                }
                self.library(name).link = mode;
                LinkInput::Library {
                    name: library.to_string(),
                }
            }
            "--link-search" => LinkInput::SearchPath { path: path() },
            "--link-framework" => LinkInput::Framework {
                name: fact.to_string(),
            },
            "--link-file" => LinkInput::File { path: path() },
            "--runtime-file" => {
                self.library(name).runtime_files.push(path());
                return Ok(());
            }
            _ => return Err(format!("`{flag}` is not a link flag")),
        };
        self.library(name).inputs.push(input);
        Ok(())
    }

    /// Checks the facts the flags gave; run once after the last flag.
    pub fn finish_flags(&self) -> Result<(), String> {
        let problems = self
            .libraries
            .values()
            .flat_map(NativeLibrary::problems)
            .collect::<Vec<_>>();
        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems.join("\n"))
        }
    }

    fn library(&mut self, name: &str) -> &mut NativeLibrary {
        self.libraries
            .entry((None, name.to_string()))
            .or_insert_with(|| NativeLibrary::new(name))
    }

    /// The facts of the logical library `name` as source in `source` names
    /// it: its package's under a build plan, the flags' otherwise.
    fn library_for(&self, source: SourceId, name: &str) -> (Option<&str>, Option<&NativeLibrary>) {
        let package = self.source_packages.get(&source).map(String::as_str);
        let library = self
            .libraries
            .get(&(package.map(str::to_string), name.to_string()));
        (package, library)
    }

    /// The link plan of `program` for `target`: the inputs of every logical
    /// library its code references, in order of first reference, or one
    /// diagnostic per library with no inputs or with an input `target`
    /// cannot use.
    pub fn resolve(
        &self,
        program: &crate::mir::Program,
        target: CTarget,
    ) -> Result<LinkPlan, Vec<Diagnostic>> {
        let mut diagnostics = Vec::new();
        let mut plan = LinkPlan {
            windows_subsystem: self.windows_subsystem,
            ..LinkPlan::default()
        };
        let mut referenced = BTreeSet::new();
        let mut runtime_names = BTreeMap::<OsString, PathBuf>::new();
        for (library, span) in referenced_libraries(program) {
            let (package, facts) = self.library_for(span.source, &library);
            if !referenced.insert((package.map(str::to_string), library.clone())) {
                continue;
            }
            let Some(facts) = facts else {
                diagnostics.push(missing_input(&library, package, span, target));
                continue;
            };
            let search_paths = facts
                .inputs
                .iter()
                .filter_map(|input| match input {
                    LinkInput::SearchPath { path } => Some(path.as_path()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            for input in &facts.inputs {
                let item = match input {
                    LinkInput::SearchPath { path } => LinkItem::SearchPath(path.clone()),
                    LinkInput::File { path } => LinkItem::File(path.clone()),
                    LinkInput::Framework { name } => {
                        if target.operating_system != TargetOperatingSystem::MacOs {
                            diagnostics.push(unusable_input(
                                span,
                                "Frameworks Link Only On macOS",
                                format!("native library `{library}` links the framework `{name}`, and frameworks exist only on macOS"),
                                format!("give `{library}` a framework input only in its macOS entry of `[native-libraries.{library}]`"),
                            ));
                            continue;
                        }
                        LinkItem::Framework(name.clone())
                    }
                    LinkInput::Library { name } => match (facts.link, target.operating_system) {
                        // ld64 has no switch that prefers an archive, so a
                        // static library links by its file.
                        (LinkMode::Static, TargetOperatingSystem::MacOs) => {
                            let archive = format!("lib{name}.a");
                            match search_paths
                                .iter()
                                .map(|directory| directory.join(&archive))
                                .find(|candidate| candidate.is_file())
                            {
                                Some(path) => LinkItem::File(path),
                                None => {
                                    diagnostics.push(unusable_input(
                                        span,
                                        "Static Library Was Not Found",
                                        format!("native library `{library}` links `{name}` statically, and no search path of `{library}` holds `{archive}`"),
                                        format!("add the directory that holds `{archive}` with `search-paths` in `[native-libraries.{library}]` or `--link-search {library}=<directory>`, or link the archive by path with `--link-file {library}=<path>`"),
                                    ));
                                    continue;
                                }
                            }
                        }
                        (mode, _) => LinkItem::Library {
                            name: name.clone(),
                            mode,
                        },
                    },
                };
                plan.items.push(item);
            }
            for file in &facts.runtime_files {
                let Some(file_name) = file.file_name() else {
                    continue;
                };
                if !file.is_file() {
                    diagnostics.push(unusable_input(
                        span,
                        "Runtime File Was Not Found",
                        format!("native library `{library}` names the runtime file `{}`, which does not exist", file.display()),
                        format!("correct the path in `runtime-files` of `[native-libraries.{library}]` or in `--runtime-file {library}=<path>`"),
                    ));
                    continue;
                }
                match runtime_names.get(file_name) {
                    Some(previous) if previous == file => continue,
                    Some(previous) => {
                        diagnostics.push(unusable_input(
                            span,
                            "Runtime Files Share A Name",
                            format!("the runtime files `{}` and `{}` would both be placed beside the program as `{}`", previous.display(), file.display(), file_name.to_string_lossy()),
                            "give each runtime file a distinct file name".to_string(),
                        ));
                        continue;
                    }
                    None => {
                        runtime_names.insert(file_name.to_os_string(), file.clone());
                    }
                }
                plan.runtime_files.push(file.clone());
            }
        }
        if diagnostics.is_empty() {
            Ok(plan)
        } else {
            Err(diagnostics)
        }
    }
}

/// Every logical library a foreign call of `program` names, with the
/// call's span, in program order.
fn referenced_libraries(program: &crate::mir::Program) -> Vec<(String, Span)> {
    program
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.statements)
        .filter_map(|statement| match statement {
            crate::mir::Statement::ForeignCall { function, span, .. } => match &function.callee {
                crate::mir::ForeignCallee::Symbol {
                    library: Some(library),
                    ..
                } => Some((library.clone(), *span)),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn platform_key(target: CTarget) -> &'static str {
    match target.operating_system {
        TargetOperatingSystem::Linux => "linux",
        TargetOperatingSystem::MacOs => "macos",
        TargetOperatingSystem::Windows => "windows",
    }
}

fn missing_input(library: &str, package: Option<&str>, span: Span, target: CTarget) -> Diagnostic {
    let platform = platform_key(target);
    let owner = package.map_or_else(String::new, |package| format!(" in package `{package}`"));
    Diagnostic::new(
        MISSING_INPUT_CODE,
        format!("the native library `{library}`{owner} has no link input for {platform}"),
        span,
    )
    .with_title("Native Library Has No Link Input")
    .with_primary_label("Library Used Here")
    .with_explanation(
        "Code that calls into a native library links that library. Its link inputs come from the package's `Baton.toml`, or from `doriac` flags when the compiler runs directly.",
    )
    .with_help(format!(
        "add a `{platform}` entry to `[native-libraries.{library}]` in `Baton.toml`, or pass `--link-library {library}=<library>` to `doriac`"
    ))
}

fn unusable_input(span: Span, title: &str, message: String, help: String) -> Diagnostic {
    Diagnostic::new(UNUSABLE_INPUT_CODE, message, span)
        .with_title(title)
        .with_primary_label("Library Used Here")
        .with_help(help)
}

/// The resolved link inputs of one native link, in link order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkPlan {
    pub items: Vec<LinkItem>,
    /// Files placed beside the linked artifact.
    pub runtime_files: Vec<PathBuf>,
    pub windows_subsystem: WindowsSubsystem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkItem {
    SearchPath(PathBuf),
    Library { name: String, mode: LinkMode },
    Framework(String),
    File(PathBuf),
}

impl LinkPlan {
    /// The arguments of a Unix-style compiler driver (`cc`, `clang`) for
    /// `target`, placed after the object and runtime archive.
    pub fn driver_arguments(&self, target: CTarget) -> Vec<OsString> {
        let mut arguments = Vec::new();
        for item in &self.items {
            match item {
                LinkItem::SearchPath(path) => {
                    let mut argument = OsString::from("-L");
                    argument.push(path);
                    arguments.push(argument);
                }
                LinkItem::Library {
                    name,
                    mode: LinkMode::Static,
                } if target.operating_system != TargetOperatingSystem::MacOs => {
                    arguments.push(OsString::from("-Wl,-Bstatic"));
                    arguments.push(OsString::from(format!("-l{name}")));
                    arguments.push(OsString::from("-Wl,-Bdynamic"));
                }
                LinkItem::Library { name, .. } => {
                    arguments.push(OsString::from(format!("-l{name}")))
                }
                LinkItem::Framework(name) => {
                    arguments.push(OsString::from("-framework"));
                    arguments.push(OsString::from(name));
                }
                LinkItem::File(path) => arguments.push(path.as_os_str().to_os_string()),
            }
        }
        // Runtime files sit beside the artifact, which finds them through
        // an rpath relative to itself (section 19).
        if !self.runtime_files.is_empty() {
            match target.operating_system {
                TargetOperatingSystem::Linux => {
                    arguments.push(OsString::from("-Wl,-rpath,$ORIGIN"));
                }
                TargetOperatingSystem::MacOs => {
                    arguments.push(OsString::from("-Wl,-rpath,@loader_path"));
                }
                TargetOperatingSystem::Windows => {}
            }
        }
        if target.operating_system == TargetOperatingSystem::Windows
            && self.windows_subsystem == WindowsSubsystem::Gui
        {
            arguments.push(OsString::from("-Wl,--subsystem,windows"));
        }
        arguments
    }

    /// The linker arguments of an MSVC-style driver, placed after `/link`.
    /// The search loads a runtime file from the executable's directory, so
    /// no rpath exists.
    pub fn msvc_arguments(&self) -> Vec<OsString> {
        self.items
            .iter()
            .map(|item| match item {
                LinkItem::SearchPath(path) => {
                    let mut argument = OsString::from("/LIBPATH:");
                    argument.push(path);
                    argument
                }
                LinkItem::Library { name, .. } => {
                    if name.to_ascii_lowercase().ends_with(".lib") {
                        OsString::from(name)
                    } else {
                        OsString::from(format!("{name}.lib"))
                    }
                }
                // Validation keeps frameworks to macOS targets.
                LinkItem::Framework(name) => OsString::from(name),
                LinkItem::File(path) => path.as_os_str().to_os_string(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linking(flags: &[(&str, &str)]) -> Result<NativeLinking, String> {
        let mut linking = NativeLinking::default();
        for (flag, value) in flags {
            linking.apply_flag(flag, value, Path::new("work"))?;
        }
        linking.finish_flags()?;
        Ok(linking)
    }

    #[test]
    fn flags_build_typed_inputs_keyed_by_logical_library() {
        let linking = linking(&[
            ("--link-library", "zlib=static:z"),
            ("--link-search", "zlib=native/lib"),
            ("--runtime-file", "zlib=bin/zlib1.dll"),
            ("--link-framework", "appkit=AppKit"),
            ("--windows-subsystem", "gui"),
        ])
        .unwrap();
        let zlib = &linking.libraries[&(None, "zlib".to_string())];
        assert_eq!(zlib.link, LinkMode::Static);
        assert_eq!(
            zlib.inputs,
            vec![
                LinkInput::Library {
                    name: "z".to_string()
                },
                LinkInput::SearchPath {
                    path: Path::new("work").join("native/lib")
                },
            ]
        );
        assert_eq!(
            zlib.runtime_files,
            vec![Path::new("work").join("bin/zlib1.dll")]
        );
        assert_eq!(
            linking.libraries[&(None, "appkit".to_string())].inputs,
            vec![LinkInput::Framework {
                name: "AppKit".to_string()
            }]
        );
        assert_eq!(linking.windows_subsystem, WindowsSubsystem::Gui);
    }

    #[test]
    fn flags_reject_malformed_conflicting_and_option_like_values() {
        for (flags, fragment) in [
            (vec![("--link-library", "z")], "names no logical library"),
            (
                vec![("--link-library", "zlib=")],
                "needs a logical library name",
            ),
            (
                vec![
                    ("--link-library", "zlib=z"),
                    ("--link-library", "zlib=static:z"),
                ],
                "both statically and dynamically",
            ),
            (
                vec![("--link-library", "zlib=-Wl,-evil")],
                "not a library name",
            ),
            (
                vec![("--link-search", "zlib=lib")],
                "at least one library, framework or file input",
            ),
            (
                vec![("--windows-subsystem", "posix")],
                "unknown Windows subsystem",
            ),
        ] {
            let error = linking(&flags).expect_err(fragment);
            assert!(error.contains(fragment), "{fragment}: {error}");
        }
    }

    fn plan(items: Vec<LinkItem>, runtime_files: Vec<PathBuf>) -> LinkPlan {
        LinkPlan {
            items,
            runtime_files,
            windows_subsystem: WindowsSubsystem::Console,
        }
    }

    fn target(operating_system: TargetOperatingSystem) -> CTarget {
        CTarget {
            operating_system,
            architecture: crate::c_layout::TargetArchitecture::X64,
        }
    }

    #[test]
    fn driver_arguments_follow_each_target() {
        let items = vec![
            LinkItem::SearchPath(PathBuf::from("/deps/lib")),
            LinkItem::Library {
                name: "z".to_string(),
                mode: LinkMode::Static,
            },
            LinkItem::Library {
                name: "m".to_string(),
                mode: LinkMode::Dynamic,
            },
            LinkItem::File(PathBuf::from("/deps/lib/libextra.a")),
        ];
        let linux = plan(items.clone(), vec![PathBuf::from("/deps/libz.so.1")]);
        assert_eq!(
            linux.driver_arguments(target(TargetOperatingSystem::Linux)),
            [
                "-L/deps/lib",
                "-Wl,-Bstatic",
                "-lz",
                "-Wl,-Bdynamic",
                "-lm",
                "/deps/lib/libextra.a",
                "-Wl,-rpath,$ORIGIN",
            ]
            .map(OsString::from)
        );
        let mut macos = plan(items, Vec::new());
        macos.items.push(LinkItem::Framework("AppKit".to_string()));
        assert_eq!(
            macos.driver_arguments(target(TargetOperatingSystem::MacOs)),
            [
                "-L/deps/lib",
                "-lz",
                "-lm",
                "/deps/lib/libextra.a",
                "-framework",
                "AppKit",
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn msvc_arguments_name_import_libraries_and_search_paths() {
        let windows = plan(
            vec![
                LinkItem::SearchPath(PathBuf::from("C:/deps/lib")),
                LinkItem::Library {
                    name: "zlib".to_string(),
                    mode: LinkMode::Dynamic,
                },
                LinkItem::Library {
                    name: "user32.lib".to_string(),
                    mode: LinkMode::Dynamic,
                },
            ],
            vec![PathBuf::from("C:/deps/bin/zlib1.dll")],
        );
        assert_eq!(
            windows.msvc_arguments(),
            ["/LIBPATH:C:/deps/lib", "zlib.lib", "user32.lib"].map(OsString::from)
        );
    }
}
