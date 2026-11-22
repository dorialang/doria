//! The native companion library (Decision 0148 sections 22 and 23, entry
//! ABI version 1): built for each native profile this compiler has, with
//! its exported entries and its sidecar, and one entry called from this
//! test through a stub host to prove its slot marshalling and exit status.

#[path = "common/execution_directory.rs"]
mod execution_directory;

use std::path::Path;

use doriac::backend::NativeProfile;
use doriac::mir::{Program, SlotRecord};
use doriac::native_link::NativeLinking;
use execution_directory::ExecutionDirectory;

fn program(source: &str) -> Program {
    doriac::lower_source_to_mir("companion.doria", source.to_string())
        .unwrap_or_else(|diagnostics| panic!("the program should build: {diagnostics:#?}"))
}

const C_LIBRARY: &str = r#"
extern "C"
{
    /** @safety Any value but the least `int32`. */
    function abs(int32 $value): int32;
}
"#;

/// A region with native and string inputs and outputs, a fallthrough and
/// a `break`.
const SHOUT: &str = r#"
function shout(string $name, int32 $limit): string
{
    writable string $text = $name;
    writable int32 $count = 0;
    while (true) {
        unsafe {
            $text = $text . "!";
            $count = abs($count) + 1;
            if ($count == $limit) {
                break;
            }
        }
    }
    return $text;
}

function main(): void
{
    echo shout("hey", 3) . "\n";
}
"#;

const ZLIB_PACKAGE: &str = include_str!("../../../examples/ffi/zlib/src/main.doria");

fn linking(flags: &[(&str, &str)]) -> NativeLinking {
    let mut linking = NativeLinking::default();
    for (flag, value) in flags {
        linking
            .apply_flag(flag, value, Path::new("."))
            .unwrap_or_else(|error| panic!("{flag} {value}: {error}"));
    }
    linking.finish_flags().unwrap();
    linking
}

/// Builds the companion of `mir` for `profile` and checks its sidecar
/// against the library and the plan.
fn publish(
    mir: &Program,
    profile: NativeProfile,
    linking: &NativeLinking,
    directory: &Path,
) -> doriac::native_companion::PublishedCompanion {
    let published = doriac::native_companion::publish(mir, profile, linking, directory, "program")
        .unwrap_or_else(|error| panic!("{profile:?}: the companion builds: {error:?}"));
    let sidecar: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&published.sidecar).unwrap()).unwrap();
    assert_eq!(sidecar["schemaVersion"], 1, "{sidecar:#}");
    assert_eq!(sidecar["entryAbiVersion"], 1, "{sidecar:#}");
    let digest = sidecar["sha256"].as_str().unwrap_or_default();
    assert!(
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "the sidecar records the library's SHA-256: {sidecar:#}"
    );
    assert_eq!(
        sidecar["library"].as_str(),
        published.library.file_name().and_then(|name| name.to_str())
    );
    let bindings = sidecar["bindings"].as_array().unwrap();
    assert_eq!(bindings.len(), mir.companion.bindings.len(), "{sidecar:#}");
    for (binding, recorded) in mir.companion.bindings.iter().zip(bindings) {
        assert_eq!(
            recorded["symbol"].as_str(),
            Some(binding.entry_symbol().as_str())
        );
    }
    let regions = sidecar["regions"].as_array().unwrap();
    assert_eq!(regions.len(), mir.companion.regions.len(), "{sidecar:#}");
    for (region, recorded) in mir.companion.regions.iter().zip(regions) {
        assert_eq!(recorded["id"].as_str(), Some(region.id.as_str()));
        assert_eq!(
            recorded["symbol"].as_str(),
            Some(region.entry_symbol().as_str())
        );
        assert_eq!(recorded["inputs"]["size"], region.inputs.size);
        assert_eq!(
            recorded["outputs"]["index"],
            serde_json::json!(region.outputs.index)
        );
    }
    assert!(
        directory.join(&published.runtime).is_file(),
        "the shared runtime sits beside the companion"
    );
    published
}

#[cfg(unix)]
mod loader {
    use std::ffi::{c_char, c_int, c_void, CStr, CString};
    use std::path::Path;

    extern "C" {
        fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
        fn dlerror() -> *const c_char;
    }

    const RTLD_NOW: c_int = 2;

    pub struct Library(*mut c_void);

    impl Library {
        pub fn open(path: &Path) -> Self {
            let path = CString::new(path.to_str().unwrap()).unwrap();
            let handle = unsafe { dlopen(path.as_ptr(), RTLD_NOW) };
            assert!(!handle.is_null(), "dlopen: {}", unsafe {
                CStr::from_ptr(dlerror()).to_string_lossy()
            });
            Self(handle)
        }

        /// The address of `name`, which this library or one it loads
        /// exports.
        pub fn symbol(&self, name: &str) -> Option<*mut c_void> {
            let name = CString::new(name).unwrap();
            let address = unsafe { dlsym(self.0, name.as_ptr()) };
            (!address.is_null()).then_some(address)
        }
    }
}

/// A host that accepts outputs (operation 0) and runs no other operation.
#[cfg(unix)]
#[repr(C)]
struct StubHost {
    call: unsafe extern "C" fn(*mut StubHost, u32, *mut u8, *mut u8) -> i32,
    panic: unsafe extern "C" fn(
        *mut StubHost,
        *const u8,
        *const u8,
        usize,
        *const u8,
        usize,
        *const u8,
        usize,
    ),
    accepted: Vec<Vec<u8>>,
}

#[cfg(unix)]
unsafe extern "C" fn stub_call(
    host: *mut StubHost,
    operation: u32,
    arguments: *mut u8,
    results: *mut u8,
) -> i32 {
    assert_eq!(operation, 0, "the stub host runs only operation 0");
    assert_eq!(
        arguments, results,
        "operation 0 takes and returns the output slot"
    );
    let data = arguments.cast::<*const u8>().read();
    let length = arguments.add(8).cast::<usize>().read();
    let host = &mut *host;
    host.accepted
        .push(std::slice::from_raw_parts(data, length).to_vec());
    // The host token: the accepted value's position, from 1.
    results.cast::<usize>().write(host.accepted.len());
    0
}

#[cfg(unix)]
unsafe extern "C" fn stub_panic(
    _: *mut StubHost,
    _: *const u8,
    code: *const u8,
    code_length: usize,
    _: *const u8,
    _: usize,
    _: *const u8,
    _: usize,
) {
    eprintln!(
        "companion panic {}",
        String::from_utf8_lossy(std::slice::from_raw_parts(code, code_length))
    );
    std::process::abort();
}

/// A zero-filled C-layout record of `size` bytes.
fn record(record: &SlotRecord) -> Vec<u64> {
    vec![0; (record.size as usize).div_ceil(8).max(1)]
}

/// The offset of the slot of the local named `name` in `record`.
fn offset(mir: &Program, function: &str, record: &SlotRecord, name: &str) -> usize {
    let function = mir
        .functions
        .iter()
        .find(|candidate| candidate.name == function)
        .unwrap();
    record
        .slots
        .iter()
        .find(|slot| {
            function.locals[slot.value.local.0]
                .name
                .trim_start_matches('$')
                == name
        })
        .unwrap_or_else(|| panic!("`{name}` has a slot in {record:?}"))
        .offset as usize
}

/// Each local a region exchanges with its surroundings names the source
/// binding it lowers, which a host that keeps its own place for each
/// binding (PHP output) reads and writes; a temporary names none.
#[test]
fn region_locals_name_the_bindings_they_lower() {
    let source = format!("{C_LIBRARY}{SHOUT}");
    let mir = program(&source);
    let function = mir
        .functions
        .iter()
        .find(|candidate| candidate.name == "shout")
        .unwrap();
    let region = &function.unsafe_regions[0];
    let binding = |name: &str| {
        let local = region
            .inputs
            .iter()
            .map(|value| &function.locals[value.local.0])
            .find(|local| local.name.trim_start_matches('$') == name)
            .unwrap_or_else(|| panic!("`{name}` is a region input"));
        local
            .binding
            .unwrap_or_else(|| panic!("`{name}` names its binding"))
    };
    assert_ne!(binding("text"), binding("count"));
    let parameters = function
        .params
        .iter()
        .map(|parameter| function.locals[parameter.0].binding)
        .collect::<Vec<_>>();
    assert!(parameters.iter().all(Option::is_some), "{parameters:?}");
    assert!(function
        .locals
        .iter()
        .filter(|local| local.synthetic)
        .all(|local| local.binding.is_none()));
}

/// Calls the region of `shout` from this process through a stub host on
/// every profile: a fallthrough edge and a `break` edge, each writing the
/// outputs, handing the string to the host and returning its status.
#[cfg(unix)]
#[test]
fn a_region_entry_marshals_its_slots_and_returns_its_exit() {
    let source = format!("{C_LIBRARY}{SHOUT}");
    let mir = program(&source);
    let region = &mir.companion.regions[0];
    let text = offset(&mir, "shout", &region.inputs, "text");
    let count = offset(&mir, "shout", &region.inputs, "count");
    let limit = offset(&mir, "shout", &region.inputs, "limit");
    let text_out = offset(&mir, "shout", &region.outputs, "text");
    let count_out = offset(&mir, "shout", &region.outputs, "count");
    let index = region
        .outputs
        .index
        .expect("a region with a break has an index") as usize;
    for profile in NativeProfile::AVAILABLE {
        let directory = ExecutionDirectory::new();
        let published = publish(&mir, *profile, &NativeLinking::default(), directory.path());
        let library = loader::Library::open(&published.library);
        let entry = library
            .symbol(&region.entry_symbol())
            .unwrap_or_else(|| panic!("{profile:?}: the entry is exported"));
        let attach = library
            .symbol("dr_v1_attach_region_host")
            .expect("the shared runtime is loaded with the companion");
        let entry: unsafe extern "C" fn(*mut StubHost, *const u8, *mut u8, *mut u8) -> i32 =
            unsafe { std::mem::transmute(entry) };
        let attach: unsafe extern "C" fn(*mut StubHost) = unsafe { std::mem::transmute(attach) };
        let mut host = StubHost {
            call: stub_call,
            panic: stub_panic,
            accepted: Vec::new(),
        };
        unsafe { attach(&mut host) };
        for (start, status, expected) in [(0_i32, 0, 1_i32), (2, 2, 3)] {
            let name = b"hey";
            let mut inputs = record(&region.inputs);
            let mut outputs = record(&region.outputs);
            let base = inputs.as_mut_ptr().cast::<u8>();
            unsafe {
                base.add(text).cast::<*const u8>().write(name.as_ptr());
                base.add(text + 8).cast::<usize>().write(name.len());
                base.add(count).cast::<i32>().write(start);
                base.add(limit).cast::<i32>().write(3);
            }
            host.accepted.clear();
            let returned = unsafe {
                entry(
                    &mut host,
                    std::ptr::null(),
                    base,
                    outputs.as_mut_ptr().cast(),
                )
            };
            assert_eq!(returned, status, "{profile:?} from {start}");
            let out = outputs.as_ptr().cast::<u8>();
            unsafe {
                assert_eq!(
                    out.add(count_out).cast::<i32>().read(),
                    expected,
                    "{profile:?}"
                );
                assert_eq!(
                    out.add(text_out).cast::<usize>().read(),
                    1,
                    "{profile:?}: the host's token replaces the lent string"
                );
                // Fallthrough writes no index; the break is the region's
                // first exit.
                assert_eq!(out.add(index).cast::<i32>().read(), 0, "{profile:?}");
            }
            assert_eq!(
                host.accepted,
                [b"hey!".to_vec()],
                "{profile:?} from {start}"
            );
        }
    }
}

/// `examples/ffi/zlib` builds its companion on every profile, linking
/// zlib, with one exported entry per region and one exported forwarding C
/// entry per C-callable function.
#[cfg(unix)]
#[test]
fn the_zlib_example_builds_its_companion() {
    let mir = program(ZLIB_PACKAGE);
    assert!(!mir.companion.regions.is_empty());
    assert_eq!(
        mir.companion
            .callbacks
            .iter()
            .map(|callback| mir.functions[callback.function.0].name.as_str())
            .collect::<Vec<_>>(),
        ["allocateForZlib", "releaseForZlib"],
        "{:#?}",
        mir.companion.callbacks
    );
    let linking = linking(&[("--link-library", "zlib=z")]);
    for profile in NativeProfile::AVAILABLE {
        let directory = ExecutionDirectory::new();
        let published = publish(&mir, *profile, &linking, directory.path());
        let sidecar: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&published.sidecar).unwrap()).unwrap();
        assert_eq!(sidecar["foreignLibraries"], serde_json::json!(["zlib"]));
        let library = loader::Library::open(&published.library);
        for region in &mir.companion.regions {
            assert!(
                library.symbol(&region.entry_symbol()).is_some(),
                "{profile:?}: {} is exported",
                region.entry_symbol()
            );
        }
        for callback in &mir.companion.callbacks {
            assert!(
                library.symbol(&callback.entry_symbol()).is_some(),
                "{profile:?}: {} is exported",
                callback.entry_symbol()
            );
        }
        assert!(
            !mir.companion.bindings.is_empty(),
            "the regions call extern functions"
        );
        for binding in &mir.companion.bindings {
            assert!(
                library.symbol(&binding.entry_symbol()).is_some(),
                "{profile:?}: {} is exported",
                binding.entry_symbol()
            );
        }
        assert!(library.symbol("deflateInit_").is_some(), "zlib is linked");
    }
}

/// PHP output runs `shout`'s region in the companion it publishes beside
/// itself: the inputs cross from the bindings' places, the string output
/// comes back through operation 0, and the `break` edge leaves the loop
/// (Decision 0148 section 23). A sidecar that does not match, and a
/// missing library, fail with B3605 before any region runs.
#[test]
fn php_output_runs_a_region_in_its_companion() {
    if std::process::Command::new("php")
        .args(["-r", "exit(class_exists('FFI') ? 0 : 1);"])
        .status()
        .map_or(true, |status| !status.success())
    {
        eprintln!("PHP with FFI unavailable; PHP companion execution skipped");
        return;
    }
    let source = format!("{C_LIBRARY}{SHOUT}");
    let output = doriac::compile_source(
        "companion.doria",
        source,
        doriac::backend::BackendTarget::Php,
    )
    .unwrap_or_else(|diagnostics| panic!("PHP output should build: {diagnostics:#?}"));
    assert!(
        matches!(
            output,
            doriac::backend::BackendOutput::TextWithCompanion { .. }
        ),
        "{output:?}"
    );
    let directory = ExecutionDirectory::new();
    let script = directory.path().join("shout.php");
    doriac::backend::publish_output(&script, output).unwrap();
    let library = directory
        .path()
        .join(doriac::native_companion::library_file_name("shout"));
    let sidecar = directory
        .path()
        .join(doriac::native_companion::sidecar_file_name("shout"));
    assert!(library.is_file() && sidecar.is_file());
    let run = || {
        std::process::Command::new("php")
            .arg(&script)
            .output()
            .unwrap()
    };
    let ran = run();
    assert_eq!(
        (
            ran.status.code(),
            String::from_utf8_lossy(&ran.stdout).into_owned()
        ),
        (Some(0), "hey!!!\n".to_string()),
        "{}",
        String::from_utf8_lossy(&ran.stderr)
    );

    let document = std::fs::read_to_string(&sidecar).unwrap();
    std::fs::write(
        &sidecar,
        document.replace("doria_region_v1_", "doria_region_v9_"),
    )
    .unwrap();
    let mismatched = run();
    assert_eq!(mismatched.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&mismatched.stderr).contains("B3605"));
    std::fs::write(&sidecar, document).unwrap();
    std::fs::remove_file(&library).unwrap();
    let missing = run();
    assert_eq!(missing.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("B3605"));
}

/// An expression region's value reaches the PHP expression around it.
#[test]
fn php_output_reads_the_value_of_an_expression_region() {
    if std::process::Command::new("php")
        .args(["-r", "exit(class_exists('FFI') ? 0 : 1);"])
        .status()
        .map_or(true, |status| !status.success())
    {
        eprintln!("PHP with FFI unavailable; PHP companion execution skipped");
        return;
    }
    let source = format!(
        "{C_LIBRARY}function main(): void {{ let $value = unsafe(abs(-3)); echo unsafe(abs(-4)) . \" \" . $value . \"\\n\"; }}"
    );
    let output = doriac::compile_source("value.doria", source, doriac::backend::BackendTarget::Php)
        .unwrap_or_else(|diagnostics| panic!("PHP output should build: {diagnostics:#?}"));
    let directory = ExecutionDirectory::new();
    let script = directory.path().join("value.php");
    doriac::backend::publish_output(&script, output).unwrap();
    let ran = std::process::Command::new("php")
        .arg(&script)
        .output()
        .unwrap();
    assert_eq!(
        (
            ran.status.code(),
            String::from_utf8_lossy(&ran.stdout).into_owned()
        ),
        (Some(0), "4 3\n".to_string()),
        "{}",
        String::from_utf8_lossy(&ran.stderr)
    );
}
