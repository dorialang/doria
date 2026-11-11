//! A transfer group that only watches, driven through the entry points
//! compiled Doria calls. The terminal layer's pseudo-terminal tests use it to
//! put a real raw mode behind the watch.

use super::checked_group::{
    dr_v1_checked_transfer_group_observe, dr_v1_checked_transfer_group_resubmit_watch,
    dr_v2_checked_transfer_group_begin,
};
use super::plan::{dr_v1_checked_transfer_plan_key, dr_v1_checked_transfer_plan_watch_new};
use super::*;

/// What one fetch reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fetched {
    /// The watch completed, carrying the key the plan gave for it.
    Ready,
    TimedOut,
    Exhausted,
}

pub(crate) struct WatchingGroup {
    set: *mut c_void,
    group: *mut c_void,
    key: [u64; 2],
}

struct Outputs {
    message: *mut DrStringV1,
    path: *mut DrStringV1,
    system_code: i64,
    transferred: usize,
    meta: u64,
}

impl Outputs {
    fn new() -> Self {
        Self {
            message: ptr::null_mut(),
            path: ptr::null_mut(),
            system_code: 0,
            transferred: 0,
            meta: 0,
        }
    }

    /// A refusal's message is owned by the caller, as it is in compiled code.
    unsafe fn release(&mut self) {
        if !self.message.is_null() {
            crate::dr_v1_string_release(self.message);
            self.message = ptr::null_mut();
        }
    }
}

impl WatchingGroup {
    /// A plan of one watch on a new readiness set with no member, begun with
    /// an optional group deadline.
    pub(crate) unsafe fn begin(
        source: u64,
        deadline: Option<crate::deadline::DrDeadlineV1>,
    ) -> Self {
        let mut outputs = Outputs::new();
        let mut set = ptr::null_mut();
        assert_eq!(
            dr_v1_checked_readiness_set_new(
                ptr::null(),
                &mut set,
                &mut outputs.message,
                &mut outputs.path,
                &mut outputs.system_code,
                &mut outputs.transferred,
                &mut outputs.meta,
            ),
            0
        );
        let (group, key) = Self::begin_on(set, source, deadline);
        Self { set, group, key }
    }

    /// Ends this group and begins another like it on the same readiness set,
    /// which must be as free of the first group's watch as a new one.
    pub(crate) unsafe fn begin_again(
        &mut self,
        source: u64,
        deadline: Option<crate::deadline::DrDeadlineV1>,
    ) {
        adapter::dr_v1_transfer_adapter_group_drop(ptr::null(), self.group);
        self.group = ptr::null_mut();
        let (group, key) = Self::begin_on(self.set, source, deadline);
        self.group = group;
        self.key = key;
    }

    unsafe fn begin_on(
        set: *mut c_void,
        source: u64,
        deadline: Option<crate::deadline::DrDeadlineV1>,
    ) -> (*mut c_void, [u64; 2]) {
        let mut outputs = Outputs::new();
        let mut plan = ptr::null_mut();
        assert_eq!(
            dr_v1_checked_transfer_plan_watch_new(
                ptr::null(),
                source,
                &mut plan,
                &mut outputs.message,
                &mut outputs.path,
                &mut outputs.system_code,
                &mut outputs.transferred,
                &mut outputs.meta,
            ),
            0
        );
        let mut key = [0_u64; 2];
        // The entry is a watch, so only a watch key can be had for it.
        for (kind, refused) in [(1, true), (0, true), (2, false)] {
            let tag = dr_v1_checked_transfer_plan_key(
                ptr::null(),
                plan,
                0,
                kind,
                ptr::from_mut(&mut key).cast(),
                &mut outputs.message,
                &mut outputs.path,
                &mut outputs.system_code,
                &mut outputs.transferred,
                &mut outputs.meta,
            );
            assert_eq!(tag != 0, refused, "plan key of kind {kind}");
            outputs.release();
        }
        let mut group = ptr::null_mut();
        let mut error = crate::empty_error_carrier();
        assert_eq!(
            dr_v2_checked_transfer_group_begin(
                ptr::null(),
                set,
                plan,
                deadline.as_ref().map_or(ptr::null(), ptr::from_ref),
                ptr::null_mut(),
                &mut group,
                &mut error,
                &mut outputs.message,
                &mut outputs.path,
                &mut outputs.system_code,
                &mut outputs.transferred,
                &mut outputs.meta,
            ),
            0
        );
        (group, key)
    }

    /// One fetch. `wait` is a `Doria\Std\Io\Wait` as compiled code passes it:
    /// its kind and nanoseconds.
    pub(crate) unsafe fn fetch(&mut self, wait: (u8, u64)) -> Fetched {
        let mut outputs = Outputs::new();
        let (mut result, mut kind, mut outcome, mut byte_count) = (0, 0, 0, 0);
        let mut key = [0_u64; 2];
        let mut error = crate::empty_error_carrier();
        let mut control = DrCompletionStatusV1::default();
        assert_eq!(
            dr_v1_checked_transfer_group_observe(
                ptr::null(),
                self.group,
                wait.0,
                wait.1,
                0,
                &mut result,
                &mut kind,
                ptr::from_mut(&mut key).cast(),
                &mut outcome,
                &mut byte_count,
                &mut error,
                ptr::from_mut(&mut control).cast(),
                &mut outputs.message,
                &mut outputs.path,
                &mut outputs.system_code,
                &mut outputs.transferred,
                &mut outputs.meta,
            ),
            0
        );
        let mut observed = [0_u64; 2];
        let watched = adapter::dr_v1_transfer_adapter_group_observed(
            self.group,
            2,
            ptr::from_mut(&mut observed).cast(),
        ) == 0;
        for transfer in [0, 1] {
            let mut none = [0_u64; 2];
            assert_eq!(
                adapter::dr_v1_transfer_adapter_group_observed(
                    self.group,
                    transfer,
                    ptr::from_mut(&mut none).cast()
                ),
                1
            );
        }
        match result {
            0 => {
                assert_eq!((kind, outcome, byte_count), (2, 0, 0));
                assert_eq!(key, self.key);
                assert!(watched);
                assert_eq!(observed, self.key);
                Fetched::Ready
            }
            1 => {
                assert!(!watched);
                Fetched::TimedOut
            }
            2 => {
                assert!(!watched);
                Fetched::Exhausted
            }
            _ => panic!("unknown fetch result {result}"),
        }
    }

    /// Arms the watch again. False when the group refuses, which it reports
    /// as a checked configuration failure.
    pub(crate) unsafe fn resubmit(&mut self) -> bool {
        self.resubmit_key(self.key)
    }

    /// The same with a key another plan made.
    pub(crate) unsafe fn resubmit_foreign(&mut self) -> bool {
        self.resubmit_key([self.key[0] + 1, self.key[1]])
    }

    unsafe fn resubmit_key(&mut self, key: [u64; 2]) -> bool {
        let mut outputs = Outputs::new();
        let tag = dr_v1_checked_transfer_group_resubmit_watch(
            ptr::null(),
            self.group,
            ptr::from_ref(&key).cast(),
            &mut outputs.message,
            &mut outputs.path,
            &mut outputs.system_code,
            &mut outputs.transferred,
            &mut outputs.meta,
        );
        outputs.release();
        tag == 0
    }
}

impl Drop for WatchingGroup {
    fn drop(&mut self) {
        unsafe {
            adapter::dr_v1_transfer_adapter_group_drop(ptr::null(), self.group);
            adapter::dr_v1_readiness_set_drop(ptr::null(), self.set);
        }
    }
}
