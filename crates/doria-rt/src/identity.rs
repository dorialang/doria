// Hosted DLLs use the platform CRT, never the standalone executable's shims.
#[cfg(windows)]
#[link(name = "msvcrt")]
extern "C" {}

/// A versioned, allocation-free bootstrap independent of operational ABI layouts.
///
/// # Safety
/// A non-null `length` must point to writable storage for one `size_t`.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_runtime_identity(length: *mut usize) -> *const u8 {
    let bytes = include_bytes!(concat!(env!("OUT_DIR"), "/runtime-identity.json"));
    if !length.is_null() {
        length.write(bytes.len());
    }
    bytes.as_ptr()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_identity_has_static_storage_and_exact_utf8_length() {
        let mut length = 0;
        let bytes = unsafe { dr_v1_runtime_identity(&mut length) };
        assert_eq!(bytes, unsafe {
            dr_v1_runtime_identity(core::ptr::null_mut())
        });
        let text =
            core::str::from_utf8(unsafe { core::slice::from_raw_parts(bytes, length) }).unwrap();
        assert!(text.starts_with("{\"abiVersion\":"));
        assert!(text.ends_with("\"artifactKind\":\"shared\"}"));
        assert!(!text.contains('\0'));
    }
}
