//! Loading a native shared library and finding its exported symbols, the
//! one loader the interpreter's companion host and the tests share
//! (Decision 0148 section 22). On Unix a library's dependencies resolve
//! through its rpath; on Windows through its own directory first.

use std::ffi::c_void;
use std::path::Path;

/// Loads the library at `path` for the life of the process.
pub fn open(path: &Path) -> Result<*mut c_void, String> {
    platform::open(path)
}

/// Loads the companion `library` and the shared `runtime` it binds, and
/// returns their handles in that order. The runtime handle is the module
/// every library naming the runtime binds, so the host reaches the same
/// runtime data the companion does. On Unix both handles are the
/// companion's, whose lookups search its dependencies.
pub fn open_companion(
    library: &Path,
    runtime: &Path,
) -> Result<(*mut c_void, *mut c_void), String> {
    platform::open_companion(library, runtime)
}

/// The address of the symbol `name` that `library` exports.
pub fn symbol(library: *mut c_void, name: &str) -> Result<*mut c_void, String> {
    platform::symbol(library, name)
}

#[cfg(unix)]
mod platform {
    use std::ffi::{c_char, c_int, c_void, CStr, CString};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    #[cfg_attr(all(target_os = "linux", target_env = "gnu"), link(name = "dl"))]
    extern "C" {
        fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn dlerror() -> *mut c_char;
    }

    const RTLD_NOW: c_int = 2;

    fn last_error() -> String {
        // SAFETY: dlerror returns null or a NUL-terminated message.
        let message = unsafe { dlerror() };
        if message.is_null() {
            "the platform loader reported no reason".to_string()
        } else {
            unsafe { CStr::from_ptr(message) }
                .to_string_lossy()
                .into_owned()
        }
    }

    /// Loads the library at `path` for the life of the process. Its
    /// dependencies resolve through its own search path (an rpath).
    pub fn open(path: &Path) -> Result<*mut c_void, String> {
        let name = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| format!("`{}` holds a NUL byte", path.display()))?;
        // SAFETY: `name` is a NUL-terminated path.
        let handle = unsafe { dlopen(name.as_ptr(), RTLD_NOW) };
        if handle.is_null() {
            Err(last_error())
        } else {
            Ok(handle)
        }
    }

    /// Loads the companion `library`, which brings the shared runtime it
    /// was linked against through its rpath, and returns it twice: a
    /// lookup through the library's handle searches its dependencies, so
    /// the host reaches the very runtime the library binds, which the
    /// dynamic linker shares among every library that names it.
    pub fn open_companion(
        library: &Path,
        _runtime: &Path,
    ) -> Result<(*mut c_void, *mut c_void), String> {
        let handle = open(library)?;
        Ok((handle, handle))
    }

    pub fn symbol(library: *mut c_void, name: &str) -> Result<*mut c_void, String> {
        let symbol = CString::new(name).map_err(|_| format!("`{name}` holds a NUL byte"))?;
        // SAFETY: `library` came from `open`.
        let address = unsafe { dlsym(library, symbol.as_ptr()) };
        if address.is_null() {
            Err(format!("`{name}`: {}", last_error()))
        } else {
            Ok(address)
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::{c_char, c_void, CString};
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
        fn GetLastError() -> u32;
    }

    /// The library's own directory first, then the default directories.
    const LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR: u32 = 0x100;
    const LOAD_LIBRARY_SEARCH_DEFAULT_DIRS: u32 = 0x1000;

    /// Loads the library at `path` for the life of the process, finding its
    /// dependencies in its own directory first. A library the process
    /// loaded already (the shared runtime) satisfies a dependency by name.
    pub fn open(path: &Path) -> Result<*mut c_void, String> {
        let absolute = std::path::absolute(path).map_err(|error| error.to_string())?;
        let name = absolute
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>();
        // SAFETY: `name` is a NUL-terminated absolute path.
        let handle = unsafe {
            LoadLibraryExW(
                name.as_ptr(),
                std::ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
            )
        };
        if handle.is_null() {
            Err(format!("Windows error {}", unsafe { GetLastError() }))
        } else {
            Ok(handle)
        }
    }

    /// Loads the shared runtime and then the companion `library`, whose
    /// import of the runtime resolves to the loaded module of that name. A
    /// runtime module this process loaded already is the one every library
    /// that names it binds, so the host uses that one.
    pub fn open_companion(
        library: &Path,
        runtime: &Path,
    ) -> Result<(*mut c_void, *mut c_void), String> {
        let name = runtime
            .file_name()
            .ok_or_else(|| format!("`{}` names a file", runtime.display()))?
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>();
        // SAFETY: `name` is a NUL-terminated module name.
        let loaded = unsafe { GetModuleHandleW(name.as_ptr()) };
        let runtime = if loaded.is_null() {
            open(runtime)?
        } else {
            loaded
        };
        Ok((open(library)?, runtime))
    }

    pub fn symbol(library: *mut c_void, name: &str) -> Result<*mut c_void, String> {
        let symbol = CString::new(name).map_err(|_| format!("`{name}` holds a NUL byte"))?;
        // SAFETY: `library` came from `open`.
        let address = unsafe { GetProcAddress(library, symbol.as_ptr()) };
        if address.is_null() {
            Err(format!("`{name}`: Windows error {}", unsafe {
                GetLastError()
            }))
        } else {
            Ok(address)
        }
    }
}
