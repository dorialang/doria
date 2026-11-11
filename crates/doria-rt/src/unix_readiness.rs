//! Private fd-backed half of the shared readiness core. Windows uses completion
//! registrations, not simulated pipe readiness. Registered owners keep their
//! handles alive; this backend owns only its watcher and reusable event storage.

use core::{alloc::Layout, ffi::c_void, mem, ptr, slice};

use crate::cancellation::{self, DrCancellationV1};
use crate::io_status::DrIoStatusV1;
use crate::native_handle::OwnedHandle;
use crate::platform_io::{self, Failure, Reason};
use crate::wait_budget::DrWaitBudgetV1;

pub(crate) type BorrowedProjection = crate::native_handle::NativeHandle;

const READ: u8 = 1;
const WRITE: u8 = 2;
const CLOSED: u8 = 4;
const FAULT: u8 = 8;
const CANCEL: u64 = u64::MAX;
/// A descriptor that only ends a wait: a watched source's own readiness. It
/// has no registration and reports no event.
const WAKE: u64 = u64::MAX - 1;

/// Private namespaces within this watch owner: registrations use even values,
/// helper notices use odd operation values. Public registration keys never
/// double as a request or helper identity. Native tokens still carry the watch
/// slot generation, protecting removed/reused metadata from queued events.
fn watch_key(index: usize, helper: bool) -> Result<u64, Failure> {
    let key = u64::try_from(index)
        .ok()
        .and_then(|index| index.checked_mul(2))
        .and_then(|key| key.checked_add(u64::from(helper)))
        .filter(|key| *key != CANCEL && *key != WAKE)
        .ok_or_else(exhausted)?;
    Ok(key)
}

/// One physical registration's shared pending interests. Operations subscribe
/// independently without duplicating native watches or registered ownership.
struct RegistrationWatch {
    handle: i32,
    key: u64,
    readers: usize,
    writers: usize,
}

impl RegistrationWatch {
    fn new(handle: i32, registration: usize) -> Result<Self, Failure> {
        Ok(Self {
            handle,
            key: watch_key(registration, false)?,
            readers: 0,
            writers: 0,
        })
    }

    fn source(handle: i32, route: usize) -> Result<Self, Failure> {
        // Physical routes coexist with logical registrations and operation
        // helper notices. The high-bit namespace never aliases either, and
        // native watch-slot generations still protect queued event reuse.
        if route > (i32::MAX as usize - 1) / 2 {
            return Err(exhausted());
        }
        let mut watch = Self::new(handle, route)?;
        watch.key |= 1 << 63;
        Ok(watch)
    }

    fn interests(&self) -> u8 {
        (u8::from(self.readers != 0) * READ) | (u8::from(self.writers != 0) * WRITE)
    }

    fn subscribe(&mut self, watcher: &mut Set, reading: bool) -> Result<(), Failure> {
        let old = self.interests();
        let count = if reading { self.readers } else { self.writers };
        let next = count.checked_add(1).ok_or_else(exhausted)?;
        let interests = old | if reading { READ } else { WRITE };
        if old == 0 {
            watcher.add(self.handle, self.key, interests)?;
        } else if old != interests {
            watcher.update(self.key, interests)?;
        }
        if reading {
            self.readers = next;
        } else {
            self.writers = next;
        }
        Ok(())
    }

    fn unsubscribe(&mut self, watcher: &mut Set, reading: bool) -> Result<(), Failure> {
        let old = self.interests();
        let count = if reading {
            &mut self.readers
        } else {
            &mut self.writers
        };
        *count = count.checked_sub(1).ok_or_else(Failure::invalid_input)?;
        let interests = self.interests();
        if interests == old {
            Ok(())
        } else if interests == 0 {
            watcher.remove(self.key)
        } else {
            watcher.update(self.key, interests)
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DrReadyEventV1 {
    key: u64,
    facts: u8,
}

#[derive(Clone, Copy)]
struct Registration {
    handle: i32,
    key: u64,
    generation: u32,
    interests: u8,
    immediate: bool,
}

#[derive(Debug)]
struct RegistrationFailure {
    failure: Failure,
    watcher_valid: bool,
}

impl From<Failure> for RegistrationFailure {
    fn from(failure: Failure) -> Self {
        Self {
            failure,
            watcher_valid: true,
        }
    }
}

#[cfg(any(target_os = "macos", test))]
fn change_pair(
    interests: u8,
    adding: bool,
    mut change: impl FnMut(u8, bool) -> Result<(), Failure>,
) -> Result<(), RegistrationFailure> {
    if interests & READ != 0 {
        change(READ, adding)?;
    }
    if interests & WRITE != 0 {
        if let Err(failure) = change(WRITE, adding) {
            let watcher_valid = interests & READ == 0 || change(READ, !adding).is_ok();
            return Err(RegistrationFailure {
                failure,
                watcher_valid,
            });
        }
    }
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
fn change_interests(
    old: u8,
    new: u8,
    mut change: impl FnMut(u8, bool) -> Result<(), Failure>,
) -> Result<(), RegistrationFailure> {
    let mut applied: Option<(u8, bool)> = None;
    for interest in [READ, WRITE] {
        if (old ^ new) & interest == 0 {
            continue;
        }
        let adding = new & interest != 0;
        if let Err(failure) = change(interest, adding) {
            let watcher_valid = applied
                .map(|(interest, adding)| change(interest, !adding).is_ok())
                .unwrap_or(true);
            return Err(RegistrationFailure {
                failure,
                watcher_valid,
            });
        }
        applied = Some((interest, adding));
    }
    Ok(())
}

struct Set {
    watcher: Option<OwnedHandle>,
    memory: *mut u8,
    slots: *mut Registration,
    events: *mut mem::MaybeUninit<platform::Event>,
    capacity: usize,
    immediate_count: usize,
    cancellation: *mut DrCancellationV1,
}

impl Set {
    fn layout(capacity: usize) -> Result<(Layout, usize), Failure> {
        if capacity == 0 || capacity > (i32::MAX as usize - 1) / 2 {
            return Err(Failure::invalid_input());
        }
        let registrations =
            Layout::array::<Registration>(capacity).map_err(|_| Failure::invalid_input())?;
        // One event for each direction of each watch, cancellation and a wake.
        let events = Layout::array::<platform::Event>(capacity * 2 + 2)
            .map_err(|_| Failure::invalid_input())?;
        registrations
            .extend(events)
            .map_err(|_| Failure::invalid_input())
    }

    unsafe fn new(capacity: usize, cancellation: *mut DrCancellationV1) -> Result<Self, Failure> {
        let (layout, offset) = Self::layout(capacity)?;
        let watcher = platform::create()?;
        let memory = crate::allocate(layout.size());
        if memory.is_null() {
            return Err(exhausted());
        }
        let mut set = Self {
            watcher: Some(watcher),
            memory,
            slots: memory.cast(),
            events: memory.add(offset).cast(),
            capacity,
            immediate_count: 0,
            cancellation: ptr::null_mut(),
        };
        for index in 0..capacity {
            set.slots.add(index).write(Registration {
                handle: -1,
                key: 0,
                generation: 0,
                interests: 0,
                immediate: false,
            });
        }
        if !cancellation.is_null() {
            set.cancellation = cancellation::dr_v1_cancellation_retain(cancellation);
            if set.cancellation.is_null() {
                return Err(exhausted());
            }
            let signal = (*cancellation).signal()?;
            platform::add(
                set.handle()?,
                crate::cancellation_signal::native_handle(signal),
                READ,
                CANCEL,
            )
            .map_err(|failure| failure.failure)?;
        }
        Ok(set)
    }

    /// Only configuration may grow this allocation. Tokens identify numeric
    /// indices/generations, so copying every row at its old index preserves all
    /// native watches, including queued notifications and dormant generations.
    fn reserve(&mut self, capacity: usize) -> Result<(), Failure> {
        self.handle()?;
        if capacity <= self.capacity {
            return Ok(());
        }
        let (layout, offset) = Self::layout(capacity)?;
        let memory = unsafe { crate::allocate(layout.size()) };
        if memory.is_null() {
            return Err(exhausted());
        }
        let slots = memory.cast::<Registration>();
        unsafe {
            ptr::copy_nonoverlapping(self.slots, slots, self.capacity);
            for index in self.capacity..capacity {
                slots.add(index).write(Registration {
                    handle: -1,
                    key: 0,
                    generation: 0,
                    interests: 0,
                    immediate: false,
                });
            }
            let old_memory = self.memory;
            self.memory = memory;
            self.slots = slots;
            self.events = memory.add(offset).cast();
            self.capacity = capacity;
            crate::deallocate(old_memory);
        }
        Ok(())
    }

    fn handle(&self) -> Result<i32, Failure> {
        self.watcher.as_ref().map(OwnedHandle::get).ok_or(Failure {
            reason: Reason::Closed,
            system_code: None,
            transferred_byte_count: None,
        })
    }

    fn registration_result(
        &mut self,
        result: Result<(), RegistrationFailure>,
    ) -> Result<(), Failure> {
        result.map_err(|failure| {
            if !failure.watcher_valid {
                drop(self.watcher.take());
            }
            failure.failure
        })
    }

    fn slots(&mut self) -> &mut [Registration] {
        unsafe { slice::from_raw_parts_mut(self.slots, self.capacity) }
    }

    fn add(&mut self, handle: i32, key: u64, interests: u8) -> Result<(), Failure> {
        let watcher = self.handle()?;
        if interests == 0 || interests & !(READ | WRITE) != 0 {
            return Err(Failure::invalid_input());
        }
        if self
            .slots()
            .iter()
            .any(|slot| slot.interests != 0 && (slot.handle == handle || slot.key == key))
        {
            return Err(Failure::invalid_input());
        }
        let index = self
            .slots()
            .iter()
            .position(|slot| slot.interests == 0)
            .ok_or_else(exhausted)?;
        let generation = self.slots()[index]
            .generation
            .checked_add(1)
            .ok_or_else(exhausted)?;
        let token = (u64::from(generation) << 32) | index as u64;
        let immediate = platform::is_regular_file(handle)?;
        if !immediate {
            let result = platform::add(watcher, handle, interests, token);
            self.registration_result(result)?;
        }
        self.slots()[index] = Registration {
            handle,
            key,
            generation,
            interests,
            immediate,
        };
        self.immediate_count += usize::from(immediate);
        Ok(())
    }

    fn remove(&mut self, key: u64) -> Result<(), Failure> {
        let watcher = self.handle()?;
        let index = self
            .slots()
            .iter()
            .position(|slot| slot.interests != 0 && slot.key == key)
            .ok_or_else(Failure::invalid_input)?;
        let slot = self.slots()[index];
        if !slot.immediate {
            let token = (u64::from(slot.generation) << 32) | index as u64;
            let result = platform::remove(watcher, slot.handle, slot.interests, token);
            self.registration_result(result)?;
        }
        self.slots()[index].interests = 0;
        self.immediate_count -= usize::from(slot.immediate);
        Ok(())
    }

    /// Change only this registration's interests, preserving its native token
    /// and generation. In particular retiring a read cannot remove a live write.
    fn update(&mut self, key: u64, interests: u8) -> Result<(), Failure> {
        if interests == 0 || interests & !(READ | WRITE) != 0 {
            return Err(Failure::invalid_input());
        }
        let watcher = self.handle()?;
        let index = self
            .slots()
            .iter()
            .position(|slot| slot.interests != 0 && slot.key == key)
            .ok_or_else(Failure::invalid_input)?;
        let slot = self.slots()[index];
        if interests == slot.interests {
            return Ok(());
        }
        if !slot.immediate {
            let token = (u64::from(slot.generation) << 32) | index as u64;
            let result = platform::modify(watcher, slot.handle, slot.interests, interests, token);
            self.registration_result(result)?;
        }
        self.slots()[index].interests = interests;
        Ok(())
    }

    /// Makes every wait also end, with tag 4, while `handle` is readable.
    /// The caller owns the descriptor and removes it before closing it.
    fn add_wake(&mut self, handle: i32) -> Result<(), Failure> {
        let watcher = self.handle()?;
        let result = platform::add(watcher, handle, READ, WAKE);
        self.registration_result(result)
    }

    fn remove_wake(&mut self, handle: i32) {
        let removed = self.handle().and_then(|watcher| {
            platform::remove(watcher, handle, READ, WAKE).map_err(|failure| failure.failure)
        });
        if removed.is_err() {
            // A wake left behind would end later waits for a source nobody
            // watches. Close only this runtime-owned poller.
            drop(self.watcher.take());
        }
    }

    /// Tags: 0 with ready events, 1 the budget ended, 2 cancellation, 4 only
    /// an added wake was ready.
    fn wait(
        &mut self,
        budget: &DrWaitBudgetV1,
        output: &mut [DrReadyEventV1],
    ) -> Result<(u8, usize), Failure> {
        self.wait_with(budget, output, platform::wait, crate::monotonic_clock::now)
    }

    fn wait_with(
        &mut self,
        budget: &DrWaitBudgetV1,
        output: &mut [DrReadyEventV1],
        mut native_wait: impl FnMut(
            i32,
            &mut [mem::MaybeUninit<platform::Event>],
            u8,
            u64,
        ) -> Result<usize, Failure>,
        mut now: impl FnMut() -> Result<u64, Failure>,
    ) -> Result<(u8, usize), Failure> {
        let watcher = self.handle()?;
        // kqueue can return separate read and write facts for the same watch.
        if output.len() < self.capacity * 2 {
            return Err(Failure::invalid_input());
        }
        loop {
            if !self.cancellation.is_null()
                && unsafe { cancellation::dr_v1_cancellation_is_requested(self.cancellation) } != 0
            {
                return Ok((2, 0));
            }
            let mut count = 0;
            if self.immediate_count != 0 {
                for slot in self
                    .slots()
                    .iter()
                    .filter(|slot| slot.interests != 0 && slot.immediate)
                {
                    output[count] = DrReadyEventV1 {
                        key: slot.key,
                        facts: slot.interests,
                    };
                    count += 1;
                }
            }
            let (kind, units) = budget.remaining(platform::UNIT, platform::MAXIMUM, &mut now)?;
            if kind == 3 {
                return Ok((1, 0));
            }
            let poll_only = count != 0 || kind == 0;
            let events = unsafe { slice::from_raw_parts_mut(self.events, self.capacity * 2 + 2) };
            let ready = match native_wait(watcher, events, if poll_only { 0 } else { kind }, units)
            {
                Ok(ready) => ready,
                Err(failure) if failure.system_code == Some(i64::from(libc::EINTR)) => continue,
                Err(failure) => return Err(failure),
            };
            let mut woken = false;
            for index in 0..ready {
                let event = unsafe { (*self.events.add(index)).assume_init_ref() };
                let (token, facts) = platform::decode(event)?;
                if token == CANCEL {
                    return Ok((2, 0));
                }
                if token == WAKE {
                    woken = true;
                    continue;
                }
                let slot_index = (token & u64::from(u32::MAX)) as usize;
                if slot_index >= self.capacity {
                    return Err(Failure::other());
                }
                let slot = self.slots()[slot_index];
                if slot.interests == 0 || u64::from(slot.generation) != token >> 32 {
                    continue;
                }
                // An interest delta preserves registration identity. Discard
                // queued facts for a direction retired since that event was
                // produced, while preserving endpoint closure/fault advisory.
                let facts = facts & (slot.interests | CLOSED | FAULT);
                if facts == 0 {
                    continue;
                }
                output[count] = DrReadyEventV1 {
                    key: slot.key,
                    facts,
                };
                count += 1;
            }
            if count != 0 {
                return Ok((0, count));
            }
            if woken {
                return Ok((4, 0));
            }
            if kind == 0 {
                return Ok((1, 0));
            }
            // Interrupted, stale, and clamped waits all spend the same budget.
        }
    }
}

/// A transfer group's claim keeps its borrowed wake descriptor alive until
/// native membership is removed, including failed setup and teardown paths.
struct GroupCancellationWatch {
    claim: Option<cancellation::CancellationClaim>,
    registered_signal: Option<i32>,
}

impl GroupCancellationWatch {
    unsafe fn install(set: &mut Set, state: *mut DrCancellationV1) -> Result<Self, Failure> {
        // The standalone private waiter may already have a set-scoped policy.
        // Public owning readiness sets have no such policy; group controls must
        // never silently inherit or replace one.
        if !set.cancellation.is_null() && set.cancellation != state {
            return Err(Failure::invalid_input());
        }
        let claim = cancellation::CancellationClaim::acquire(state)?;
        let mut watch = Self {
            claim,
            registered_signal: None,
        };
        if let Some(claim) = &watch.claim {
            if set.cancellation.is_null() {
                let signal = crate::cancellation_signal::native_handle((*claim.0).signal()?);
                let watcher = set.handle()?;
                let result = platform::add(watcher, signal, READ, CANCEL);
                set.registration_result(result)?;
                watch.registered_signal = Some(signal);
            }
        }
        Ok(watch)
    }

    fn state(&self) -> *mut DrCancellationV1 {
        self.claim.as_ref().map_or(ptr::null_mut(), |claim| claim.0)
    }

    fn detach(&mut self, set: &mut Set) {
        self.detach_with(set, |watcher, signal| {
            platform::remove(watcher, signal, READ, CANCEL).map_err(|error| error.failure)
        });
    }

    fn detach_with(&mut self, set: &mut Set, remove: impl FnOnce(i32, i32) -> Result<(), Failure>) {
        if let Some(signal) = self.registered_signal.take() {
            let result = set.handle().and_then(|watcher| remove(watcher, signal));
            if result.is_err() {
                // A stale native wake must not affect a later group after the
                // claim is released. Close only this runtime-owned poller.
                drop(set.watcher.take());
            }
        }
    }
}

impl Drop for Set {
    fn drop(&mut self) {
        drop(self.watcher.take());
        unsafe {
            cancellation::dr_v1_cancellation_release(self.cancellation);
            crate::deallocate(self.memory);
        }
    }
}

fn exhausted() -> Failure {
    Failure {
        reason: Reason::ResourceExhausted,
        system_code: None,
        transferred_byte_count: None,
    }
}

pub(crate) mod adapter;
pub(crate) mod pending;
pub(crate) mod terminal_watch;
#[cfg(test)]
mod tests;
mod transfer;
pub(crate) use transfer::{NativeGroup as NativeTransferGroup, TransferSet as NativeTransferSet};

/// Private platform transport, not a public stream type. Allocation and watcher
/// setup happen once; ordinary wait calls reuse both. The matching Windows
/// transport is completion-based and retains request/storage loans until drained.
/// # Safety
/// A non-null cancellation pointer must carry a live claim for this call.
/// Status must be live writable storage disjoint from cancellation.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_unix_readiness_new(
    capacity: usize,
    cancellation: *mut DrCancellationV1,
    status: *mut DrIoStatusV1,
) -> *mut c_void {
    *status = DrIoStatusV1::default();
    match Set::new(capacity, cancellation) {
        Ok(set) => {
            let owner = crate::allocate(mem::size_of::<Set>()).cast::<Set>();
            if !owner.is_null() {
                owner.write(set);
                return owner.cast();
            }
            (*status).fail(exhausted());
        }
        Err(failure) => {
            (*status).fail(failure);
        }
    }
    ptr::null_mut()
}

/// # Safety
/// `owner` is a live exclusively borrowed set. The caller owns `handle` and
/// keeps it live until removal or set destruction. Status is disjoint storage.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_unix_readiness_add(
    owner: *mut c_void,
    handle: i32,
    key: u64,
    interests: u8,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    let result = (*owner.cast::<Set>()).add(handle, key, interests);
    (*status).complete(result.map(|()| 0))
}

/// # Safety
/// `owner` is a live exclusively borrowed set; status is disjoint storage.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_unix_readiness_remove(
    owner: *mut c_void,
    key: u64,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    let result = (*owner.cast::<Set>()).remove(key);
    (*status).complete(result.map(|()| 0))
}

/// Returns private wait tags: 0 events, 1 no ready events/budget exhausted,
/// 2 cancellation, 3 checked backend failure. A clamped native timeout does not
/// expire the budget; a zero duration is expired, not an immediate poll request.
/// # Safety
/// The set is exclusively borrowed. Budget, outputs, and the event array are
/// live, disjoint storage. Event capacity is at least twice the set capacity.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_unix_readiness_wait(
    owner: *mut c_void,
    budget: *const DrWaitBudgetV1,
    output: *mut DrReadyEventV1,
    capacity: usize,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    match (*owner.cast::<Set>()).wait(&*budget, slice::from_raw_parts_mut(output, capacity)) {
        Ok((tag, n)) => {
            (*status).value = n as u64;
            tag
        }
        Err(failure) => (*status).fail(failure),
    }
}

/// # Safety
/// `owner` is null or one owned set, dropped once after all waits finish. Stream
/// handles remain owned by the caller; this never closes a registered stream.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_unix_readiness_drop(owner: *mut c_void) {
    if !owner.is_null() {
        ptr::drop_in_place(owner.cast::<Set>());
        crate::deallocate(owner.cast());
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;

    pub(super) type Event = libc::epoll_event;
    pub(super) const UNIT: u64 = 1_000_000;
    pub(super) const MAXIMUM: u64 = i32::MAX as u64;

    pub(super) fn create() -> Result<OwnedHandle, Failure> {
        #[cfg(test)]
        crate::io_cost_probe::record(crate::io_cost_probe::Operation::ReadinessCreate);
        let fd = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
        if fd < 0 {
            Err(platform_io::last_failure())
        } else {
            Ok(unsafe { OwnedHandle::from_raw(fd) })
        }
    }

    pub(super) fn is_regular_file(handle: i32) -> Result<bool, Failure> {
        crate::native_handle::is_regular_file(handle)
    }

    pub(super) fn add(
        watcher: i32,
        handle: i32,
        interests: u8,
        token: u64,
    ) -> Result<(), RegistrationFailure> {
        let mut event = Event {
            events: libc::EPOLLRDHUP as u32,
            u64: token,
        };
        if interests & READ != 0 {
            event.events |= libc::EPOLLIN as u32;
        }
        if interests & WRITE != 0 {
            event.events |= libc::EPOLLOUT as u32;
        }
        if unsafe { libc::epoll_ctl(watcher, libc::EPOLL_CTL_ADD, handle, &mut event) } == 0 {
            return Ok(());
        }
        let mut failure = platform_io::last_failure();
        // EPERM here means this descriptor kind has no epoll capability.
        if failure.system_code == Some(i64::from(libc::EPERM)) {
            failure.reason = Reason::Unsupported;
        }
        Err(failure.into())
    }

    pub(super) fn modify(
        watcher: i32,
        handle: i32,
        _: u8,
        interests: u8,
        token: u64,
    ) -> Result<(), RegistrationFailure> {
        let mut event = Event {
            events: libc::EPOLLRDHUP as u32,
            u64: token,
        };
        if interests & READ != 0 {
            event.events |= libc::EPOLLIN as u32;
        }
        if interests & WRITE != 0 {
            event.events |= libc::EPOLLOUT as u32;
        }
        if unsafe { libc::epoll_ctl(watcher, libc::EPOLL_CTL_MOD, handle, &mut event) } == 0 {
            Ok(())
        } else {
            Err(platform_io::last_failure().into())
        }
    }

    pub(super) fn remove(
        watcher: i32,
        handle: i32,
        _: u8,
        _: u64,
    ) -> Result<(), RegistrationFailure> {
        if unsafe { libc::epoll_ctl(watcher, libc::EPOLL_CTL_DEL, handle, ptr::null_mut()) } == 0 {
            Ok(())
        } else {
            Err(platform_io::last_failure().into())
        }
    }

    pub(super) fn wait(
        watcher: i32,
        events: &mut [mem::MaybeUninit<Event>],
        kind: u8,
        units: u64,
    ) -> Result<usize, Failure> {
        let timeout = if kind == 1 {
            -1
        } else if kind == 0 {
            0
        } else {
            units as i32
        };
        let count = unsafe {
            libc::epoll_wait(
                watcher,
                events.as_mut_ptr().cast(),
                events.len() as i32,
                timeout,
            )
        };
        if count < 0 {
            Err(platform_io::last_failure())
        } else {
            Ok(count as usize)
        }
    }

    pub(super) fn decode(event: &Event) -> Result<(u64, u8), Failure> {
        let mut facts = 0;
        if event.events & libc::EPOLLIN as u32 != 0 {
            facts |= READ;
        }
        if event.events & libc::EPOLLOUT as u32 != 0 {
            facts |= WRITE;
        }
        if event.events & (libc::EPOLLHUP | libc::EPOLLRDHUP) as u32 != 0 {
            facts |= CLOSED;
        }
        if event.events & libc::EPOLLERR as u32 != 0 {
            facts |= FAULT;
        }
        Ok((event.u64, facts))
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;

    pub(super) type Event = libc::kevent;
    pub(super) const UNIT: u64 = 1;
    pub(super) const MAXIMUM: u64 = u64::MAX;

    pub(super) fn create() -> Result<OwnedHandle, Failure> {
        #[cfg(test)]
        crate::io_cost_probe::record(crate::io_cost_probe::Operation::ReadinessCreate);
        let fd = unsafe { libc::kqueue() };
        if fd < 0 {
            return Err(platform_io::last_failure());
        }
        let owner = unsafe { OwnedHandle::from_raw(fd) };
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(platform_io::last_failure());
        }
        Ok(owner)
    }

    pub(super) fn is_regular_file(handle: i32) -> Result<bool, Failure> {
        crate::native_handle::is_regular_file(handle)
    }

    fn change(
        watcher: i32,
        handle: i32,
        filter: i16,
        flags: u16,
        token: u64,
    ) -> Result<(), Failure> {
        let event = Event {
            ident: handle as usize,
            filter,
            flags,
            fflags: 0,
            data: 0,
            udata: token as usize as *mut c_void,
        };
        loop {
            if unsafe { libc::kevent(watcher, &event, 1, ptr::null_mut(), 0, ptr::null()) } >= 0 {
                return Ok(());
            }
            let failure = platform_io::last_failure();
            if flags == libc::EV_DELETE && failure.system_code == Some(i64::from(libc::ENOENT)) {
                return Ok(());
            }
            if failure.system_code != Some(i64::from(libc::EINTR)) {
                return Err(failure);
            }
        }
    }

    pub(super) fn add(
        watcher: i32,
        handle: i32,
        interests: u8,
        token: u64,
    ) -> Result<(), RegistrationFailure> {
        update(watcher, handle, interests, token, true)
    }

    pub(super) fn remove(
        watcher: i32,
        handle: i32,
        interests: u8,
        token: u64,
    ) -> Result<(), RegistrationFailure> {
        update(watcher, handle, interests, token, false)
    }

    fn update(
        watcher: i32,
        handle: i32,
        interests: u8,
        token: u64,
        adding: bool,
    ) -> Result<(), RegistrationFailure> {
        change_pair(interests, adding, |interest, adding| {
            change(
                watcher,
                handle,
                if interest == READ {
                    libc::EVFILT_READ
                } else {
                    libc::EVFILT_WRITE
                },
                if adding {
                    libc::EV_ADD | libc::EV_ENABLE
                } else {
                    libc::EV_DELETE
                },
                if adding { token } else { 0 },
            )
        })
    }

    pub(super) fn modify(
        watcher: i32,
        handle: i32,
        old: u8,
        new: u8,
        token: u64,
    ) -> Result<(), RegistrationFailure> {
        change_interests(old, new, |interest, adding| {
            change(
                watcher,
                handle,
                if interest == READ {
                    libc::EVFILT_READ
                } else {
                    libc::EVFILT_WRITE
                },
                if adding {
                    libc::EV_ADD | libc::EV_ENABLE
                } else {
                    libc::EV_DELETE
                },
                if adding { token } else { 0 },
            )
        })
    }

    pub(super) fn wait(
        watcher: i32,
        events: &mut [mem::MaybeUninit<Event>],
        kind: u8,
        units: u64,
    ) -> Result<usize, Failure> {
        let timeout = libc::timespec {
            tv_sec: if kind == 0 {
                0
            } else {
                (units / 1_000_000_000) as _
            },
            tv_nsec: if kind == 0 {
                0
            } else {
                (units % 1_000_000_000) as _
            },
        };
        let count = unsafe {
            libc::kevent(
                watcher,
                ptr::null(),
                0,
                events.as_mut_ptr().cast(),
                events.len() as i32,
                if kind == 1 { ptr::null() } else { &timeout },
            )
        };
        if count < 0 {
            Err(platform_io::last_failure())
        } else {
            Ok(count as usize)
        }
    }

    pub(super) fn decode(event: &Event) -> Result<(u64, u8), Failure> {
        if event.flags & libc::EV_ERROR != 0 && event.data != 0 {
            return Err(Failure::from_system_code(event.data as i64));
        }
        let facts = match event.filter {
            libc::EVFILT_READ => READ,
            libc::EVFILT_WRITE => WRITE,
            _ => return Err(Failure::other()),
        };
        Ok((
            event.udata as usize as u64,
            facts
                | if event.flags & libc::EV_EOF != 0 {
                    CLOSED
                } else {
                    0
                },
        ))
    }
}
