//! Targets without a process host: every launch is unsupported.
use super::Request;
use crate::native_handle::{NativeHandle, OwnedHandle};
use crate::platform_io::Failure;
use crate::DrStackFrameV2;

pub(super) const SHELL: &[u8] = b"sh";

pub(crate) struct Process(core::convert::Infallible);

pub(super) fn id(process: &Process) -> i64 {
    match process.0 {}
}

pub(super) unsafe fn spawn(
    _: *const DrStackFrameV2,
    _: &Request<'_>,
) -> Result<(Process, [OwnedHandle; 3]), Failure> {
    Err(Failure::unsupported())
}

pub(super) unsafe fn wait(process: &mut Process) -> Result<i64, Failure> {
    match process.0 {}
}

pub(super) unsafe fn wait_until(
    process: &mut Process,
    _: &crate::wait_budget::DrWaitBudgetV1,
) -> Result<Option<i64>, Failure> {
    match process.0 {}
}

pub(super) unsafe fn kill(process: &mut Process) -> Result<(), Failure> {
    match process.0 {}
}

pub(super) unsafe fn drain(
    _: [Option<NativeHandle>; 2],
    _: &mut dyn FnMut(&[u8]),
    _: &mut dyn FnMut(&[u8]),
) -> Result<(), (usize, Failure)> {
    Ok(())
}
