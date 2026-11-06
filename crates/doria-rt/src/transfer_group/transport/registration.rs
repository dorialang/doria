//! Dynamic native membership borrows complete compiler-retained stream owners.
//! Registration keys carry identity, not a separate handle or access claim.

use super::*;
use crate::cancellation::DrCancellationV1;

#[cfg(any(target_os = "linux", target_os = "macos"))]
type BorrowedProjection = crate::native_handle::NativeHandle;
#[cfg(windows)]
type BorrowedProjection = *const c_void;

pub(super) fn next_registration(last: &mut u64) -> Result<u64, Failure> {
    // MAX is the Unix watcher cancellation sentinel. Never wrap or reuse an
    // attempted value, even if its later allocation/admission fails.
    let value = last
        .checked_add(1)
        .filter(|value| *value != u64::MAX)
        .ok_or_else(exhausted)?;
    *last = value;
    Ok(value)
}

/// Checks the shared admission boundary before any source projection or lazy
/// association is touched. Invalid calls clear key output but never acquire
/// native resources, inspect the borrowed source, or consume its owner.
/// # Safety
/// Set is null or a live exclusively borrowed set. Key is null or disjoint,
/// aligned writable queried-layout storage. Active/fixed sets may be rejected.
pub(crate) unsafe fn validate_admission(set: *mut c_void, key: *mut c_void) -> Result<(), Failure> {
    if !key.is_null() {
        key.cast::<PlanKey>().write(PlanKey { owner: 0, value: 0 });
    }
    let set = set.cast::<SetOwner>();
    if key.is_null() || set.is_null() || (*set).active || !(*set).native.is_borrowed() {
        Err(Failure::invalid_input())
    } else {
        Ok(())
    }
}

/// # Safety
/// Cancellation is null or live for this call. Status is disjoint writable
/// storage. The compiler exclusively owns the returned set, retains complete
/// stream values independently, and drops registration metadata before them.
/// Frame is null or live for owner-allocation failure reporting during this call.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_set_new(
    frame: *const DrStackFrameV2,
    cancellation: *mut DrCancellationV1,
    status: *mut DrIoStatusV1,
) -> *mut c_void {
    *status = DrIoStatusV1::default();
    match NativeTransferSet::borrowed_new(cancellation)
        .and_then(|native| allocate_set(frame, native))
    {
        Ok(owner) => owner,
        Err(failure) => {
            (*status).fail(failure);
            ptr::null_mut()
        }
    }
}

/// # Safety
/// Set is null or a live exclusively owned set. Key is aligned writable storage
/// of the queried key layout, disjoint from status/set/projection. Null, active
/// and fixed owning sets are permitted for rejection before projection access.
/// On Unix projection is an idle nonblocking stream or regular-file descriptor.
/// On Windows it is null for rejection or points to the original stream-owned
/// AttachedStream payload during this call only; its pinned association is
/// captured without retaining that movable payload address. On successful
/// admission, the compiler retains the complete stream owner and association
/// until removal or set destruction, after every active group has retired.
/// Neither success nor failure transfers a native close claim to this set.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_set_add(
    set: *mut c_void,
    projection: BorrowedProjection,
    key: *mut c_void,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    if let Err(failure) = validate_admission(set, key) {
        return (*status).fail(failure);
    }
    let set = set.cast::<SetOwner>();
    let value = match next_registration(&mut (*set).next_registration) {
        Ok(value) => value,
        Err(failure) => {
            (*status).fail(failure);
            return 3;
        }
    };
    (*status).complete((*set).native.add_borrowed(value, projection).map(|_| {
        key.cast::<PlanKey>().write(PlanKey {
            owner: (*set).identity,
            value,
        });
        0
    }))
}

/// # Safety
/// Set is null or a live exclusively owned set. Key is null or readable queried
/// key-layout storage, disjoint from writable status. Null, active and fixed
/// owning sets are rejected before reading the key or registration metadata.
/// The compiler retains the complete stream until this succeeds; it then
/// returns that same value with its static type and drop glue intact. Native
/// removal retires only the selected metadata, never the projected handle or
/// its persistent association. Unaffected keys remain valid.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_set_remove(
    set: *mut c_void,
    key: *const c_void,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    let set = set.cast::<SetOwner>();
    if set.is_null() || (*set).active || !(*set).native.is_borrowed() || key.is_null() {
        (*status).fail(Failure::invalid_input());
        return 3;
    }
    let key = *key.cast::<PlanKey>();
    if key.owner != (*set).identity {
        (*status).fail(Failure::invalid_input());
        return 3;
    }
    (*status).complete((*set).native.remove_borrowed(key.value).map(|()| 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_exhaustion_excludes_the_cancellation_sentinel_and_never_wraps() {
        let mut last = u64::MAX - 2;
        assert_eq!(next_registration(&mut last), Ok(u64::MAX - 1));
        assert_eq!(next_registration(&mut last), Err(exhausted()));
        assert_eq!(next_registration(&mut last), Err(exhausted()));
        assert_eq!(last, u64::MAX - 1);
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod unix_tests;
