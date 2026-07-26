//! Program entry arguments (decision 0099).
//!
//! The entry glue turns the platform argument vector into an owned
//! `List<string>` and lends it to `main` for the duration of the call.
//!
//! Two contract points are load-bearing and identical on every platform:
//!
//! * **The executable path is stripped.** `$args[0]` is the first real
//!   argument, so `$args->count` is "how many arguments the user passed". A
//!   no-argument invocation yields an empty list, never a one-element list.
//! * **Only valid UTF-8 becomes a Doria `string`.** `string` is defined as
//!   immutable UTF-8, and that invariant is load-bearing for the whole
//!   language, so an argument that is not valid UTF-8 panics rather than
//!   entering the program as a malformed value.
//!
//! On Unix the bytes come from the `argv` the process was started with. On
//! Windows they come from `GetCommandLineW`/`CommandLineToArgvW` instead: the
//! `char**` handed to C `main` there is encoded in the system ANSI code page
//! and cannot represent every path or argument a user can type, whereas the
//! wide command line round-trips to UTF-8 exactly.

#[cfg(windows)]
use core::ffi::c_void;
use core::ptr;

use crate::{collection, DrCollectionV1};

/// Builds the owned argument list handed to a `main` that declares one.
///
/// # Safety
///
/// On Unix, `argv` must be a valid array of `argc` NUL-terminated pointers, as
/// supplied to C `main`. On Windows both parameters are ignored.
pub unsafe fn build(argc: i32, argv: *const *const u8) -> *mut DrCollectionV1 {
    let list = collection::new(
        0,
        false,
        false,
        core::mem::size_of::<*mut crate::DrStringV1>() as u8,
    );
    append_platform_arguments(list, argc, argv);
    list
}

/// Validates the platform argument vector without materializing a Doria list.
///
/// Parameterless entrypoints still cross the same process boundary, so invalid
/// platform text must panic even when the program does not request `$args`.
///
/// # Safety
///
/// On Unix, `argv` must be a valid array of `argc` NUL-terminated pointers, as
/// supplied to C `main`. On Windows both parameters are ignored.
pub unsafe fn validate(argc: i32, argv: *const *const u8) {
    append_platform_arguments(ptr::null_mut(), argc, argv);
}

/// Releases an argument list built by [`build`], including its element strings.
///
/// # Safety
///
/// `list` must be a pointer returned by [`build`] that has not yet been freed.
pub unsafe fn release(list: *mut DrCollectionV1) {
    if list.is_null() {
        return;
    }
    for index in 0..collection::length(list) {
        let handle = collection::value_at(ptr::null(), list, index);
        crate::dr_v1_string_release(handle as usize as *mut crate::DrStringV1);
    }
    collection::free(list);
}

#[cfg(unix)]
unsafe fn append_platform_arguments(list: *mut DrCollectionV1, argc: i32, argv: *const *const u8) {
    if argv.is_null() || argc <= 1 {
        // Element 0 is the executable path; a program invoked with no
        // arguments therefore contributes nothing.
        return;
    }
    for index in 1..argc as usize {
        let argument = *argv.add(index);
        if argument.is_null() {
            continue;
        }
        push_utf8(list, argument, c_string_length(argument));
    }
}

#[cfg(windows)]
unsafe fn append_platform_arguments(
    list: *mut DrCollectionV1,
    _argc: i32,
    _argv: *const *const u8,
) {
    let command_line = GetCommandLineW();
    if command_line.is_null() {
        argument_panic(b"failed to decode program arguments");
    }
    let mut wide_count: i32 = 0;
    let wide_argv = CommandLineToArgvW(command_line, &mut wide_count);
    if wide_argv.is_null() {
        argument_panic(b"failed to decode program arguments");
    }

    // Element 0 is the executable path, exactly as on Unix.
    for index in 1..wide_count.max(0) as usize {
        let argument = *wide_argv.add(index);
        if argument.is_null() {
            continue;
        }
        push_wide(list, argument);
    }

    LocalFree(wide_argv.cast::<c_void>());
}

#[cfg(not(any(unix, windows)))]
unsafe fn append_platform_arguments(
    _list: *mut DrCollectionV1,
    _argc: i32,
    _argv: *const *const u8,
) {
}

/// Whether an argument's bytes may become a Doria `string`.
///
/// Decision 0099: only valid UTF-8 may, because `string` is defined as
/// immutable UTF-8. This is the decision point the panic below acts on, split
/// out so it can be tested without aborting the process.
///
/// Unix only: the Windows path decodes UTF-16 instead, where the equivalent
/// rejection is an unpaired surrogate rather than a malformed byte sequence.
#[cfg(unix)]
fn argument_is_representable(bytes: &[u8]) -> bool {
    core::str::from_utf8(bytes).is_ok()
}

/// Interns `byte_length` bytes as a Doria string and appends it to the list.
#[cfg(unix)]
unsafe fn push_utf8(list: *mut DrCollectionV1, bytes: *const u8, byte_length: usize) {
    if !argument_is_representable(core::slice::from_raw_parts(bytes, byte_length)) {
        argument_panic(b"program argument is not valid UTF-8");
    }
    if list.is_null() {
        return;
    }
    let string = crate::dr_v1_string_from_utf8(bytes, byte_length);
    collection::push(list, string as usize as u64);
}

#[cfg(windows)]
unsafe fn push_wide(list: *mut DrCollectionV1, argument: *const u16) {
    let length = wide_string_length(argument);
    let units = core::slice::from_raw_parts(argument, length);

    let mut byte_length = 0_usize;
    for decoded in core::char::decode_utf16(units.iter().copied()) {
        match decoded {
            Ok(character) => byte_length += character.len_utf8(),
            Err(_) => argument_panic(b"program argument is not valid UTF-8"),
        }
    }
    if list.is_null() {
        return;
    }

    // Allocate the exact byte length and encode straight into it: passing a
    // null source to `dr_v1_string_from_utf8` would copy from a null pointer.
    let string = crate::allocate_string(byte_length);
    let destination = crate::string_bytes_mut(string);
    let mut written = 0_usize;
    for decoded in core::char::decode_utf16(units.iter().copied()) {
        let Ok(character) = decoded else {
            argument_panic(b"program argument is not valid UTF-8");
        };
        let mut encoded = [0_u8; 4];
        let bytes = character.encode_utf8(&mut encoded).as_bytes();
        ptr::copy_nonoverlapping(bytes.as_ptr(), destination.add(written), bytes.len());
        written += bytes.len();
    }
    collection::push(list, string as usize as u64);
}

#[cfg(unix)]
unsafe fn c_string_length(bytes: *const u8) -> usize {
    let mut length = 0_usize;
    while *bytes.add(length) != 0 {
        length += 1;
    }
    length
}

#[cfg(windows)]
unsafe fn wide_string_length(units: *const u16) -> usize {
    let mut length = 0_usize;
    while *units.add(length) != 0 {
        length += 1;
    }
    length
}

unsafe fn argument_panic(message: &[u8]) -> ! {
    crate::dr_v1_panic(ptr::null(), message.as_ptr(), message.len())
}

#[cfg(windows)]
extern "system" {
    fn GetCommandLineW() -> *const u16;
    fn LocalFree(memory: *mut c_void) -> *mut c_void;
}

// `CommandLineToArgvW` ships in shell32, not kernel32, so it needs an explicit
// link directive: the default MSVC library set resolves the kernel32 entries
// above but not this one. Cross-target `cargo check` and `clippy` do not link,
// so only a real Windows link surfaces a missing directive here.
#[cfg(windows)]
#[link(name = "shell32")]
extern "system" {
    fn CommandLineToArgvW(command_line: *const u16, argument_count: *mut i32) -> *mut *const u16;
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// Reads the built list back as owned Rust strings, then releases it.
    unsafe fn collect(argv: &[&[u8]]) -> std::vec::Vec<std::string::String> {
        let pointers: std::vec::Vec<*const u8> = argv.iter().map(|entry| entry.as_ptr()).collect();
        let list = build(pointers.len() as i32, pointers.as_ptr());
        let mut collected = std::vec::Vec::new();
        for index in 0..collection::length(list) {
            let handle =
                collection::value_at(ptr::null(), list, index) as usize as *const crate::DrStringV1;
            let bytes = core::slice::from_raw_parts(
                crate::dr_v1_string_data(handle),
                crate::dr_v1_string_length(handle),
            );
            collected.push(std::string::String::from(
                core::str::from_utf8(bytes).expect("argument is UTF-8"),
            ));
        }
        release(list);
        collected
    }

    #[test]
    fn the_executable_path_is_stripped() {
        // Decision 0099: `$args[0]` is the first real argument, not the program.
        let collected = unsafe { collect(&[b"/bin/prog\0", b"first\0", b"second\0"]) };
        assert_eq!(collected, ["first", "second"]);
    }

    #[test]
    fn a_no_argument_invocation_yields_an_empty_list() {
        // Never a one-element list holding the executable, and never null.
        let collected = unsafe { collect(&[b"/bin/prog\0"]) };
        assert!(collected.is_empty());
    }

    #[test]
    fn arguments_preserve_exact_bytes_including_spaces_and_unicode() {
        let collected =
            unsafe { collect(&[b"/bin/prog\0", b"two words\0", b"\xc3\xa9\xe6\x97\xa5\0"]) };
        assert_eq!(collected, ["two words", "é日"]);
    }

    #[test]
    fn only_valid_utf8_arguments_may_become_doria_strings() {
        // Decision 0099: `string` is immutable UTF-8, so a malformed argument is
        // rejected at the process boundary rather than entering the program.
        assert!(argument_is_representable(b""));
        assert!(argument_is_representable(b"plain"));
        assert!(argument_is_representable("é日".as_bytes()));

        // A lone continuation byte, a truncated multi-byte sequence, and an
        // unpaired surrogate encoding are all rejected.
        assert!(!argument_is_representable(b"\x80"));
        assert!(!argument_is_representable(b"\xc3"));
        assert!(!argument_is_representable(b"\xed\xa0\x80"));
    }

    #[test]
    fn an_empty_argument_is_preserved_as_an_empty_string() {
        let collected = unsafe { collect(&[b"/bin/prog\0", b"\0", b"after\0"]) };
        assert_eq!(collected, ["", "after"]);
    }
}
