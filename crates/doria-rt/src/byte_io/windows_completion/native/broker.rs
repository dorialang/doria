//! One lazy, stream-owned completion broker. Set membership never changes a
//! handle's native association. Calling threads elect one bounded queue pump;
//! other groups sleep against durable routing predicates, not reposted packets.

use core::alloc::Layout;
use core::pin::Pin;
use core::ptr::NonNull;

use super::*;
use crate::cancellation::WakeTarget;
use crate::transfer_group::FixedSlice;

mod driver;
mod notice;
mod routing;
mod sync;
#[cfg(test)]
mod tests;
pub(super) mod transport;
#[cfg(test)]
static TEST_BROKER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

use notice::NotificationLatch;
use routing::{Association, Group, Registry, Route};
use sync::SyncCell;

pub(super) use driver::CompletionSet;

const WAIT_TIMEOUT: u32 = 258;
const POKE: u32 = 1;

#[cfg(test)]
std::thread_local! {
    static NATIVE_DEQUEUES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn native_dequeues_for_test() -> usize {
    NATIVE_DEQUEUES.with(std::cell::Cell::get)
}

struct Lifecycle {
    weak: *mut Broker,
    claims: usize,
}

// The static lock owns all weak-pointer/strong-count transitions. No caller
// observes or increments a potentially freed allocation outside that lock.
unsafe impl Send for Lifecycle {}

static LIFECYCLE: SyncCell<Lifecycle> = SyncCell::new(Lifecycle {
    weak: ptr::null_mut(),
    claims: 0,
});

fn lifecycle() -> Pin<&'static SyncCell<Lifecycle>> {
    // Static storage never moves, including between broker lifetimes.
    unsafe { Pin::new_unchecked(&LIFECYCLE) }
}

struct Routing {
    registry: Registry,
    pumping: bool,
}

struct Broker {
    port: OwnedHandle,
    routing: SyncCell<Routing>,
    notice: NotificationLatch,
    allocation: *mut u8,
}

// Every mutable runtime fact is behind its isolated native lock. OS port use
// supports concurrent posting/dequeue. Borrowed claims exclude destruction.
unsafe impl Send for Broker {}
unsafe impl Sync for Broker {}

struct BrokerClaim(NonNull<Broker>);

unsafe impl Send for BrokerClaim {}
unsafe impl Sync for BrokerClaim {}

impl BrokerClaim {
    fn acquire() -> Result<Self, Failure> {
        #[cfg(test)]
        crate::io_cost_probe::record(crate::io_cost_probe::Operation::CompletionBrokerAcquire);
        let mut state = lifecycle().lock();
        if !state.weak.is_null() {
            state.claims = state.claims.checked_add(1).ok_or_else(exhausted)?;
            return Ok(Self(unsafe { NonNull::new_unchecked(state.weak) }));
        }
        let broker = Broker::allocate()?;
        state.weak = broker.as_ptr();
        state.claims = 1;
        Ok(Self(broker))
    }

    fn broker(&self) -> &Broker {
        unsafe { self.0.as_ref() }
    }
}

impl Drop for BrokerClaim {
    fn drop(&mut self) {
        let last = {
            let mut state = lifecycle().lock();
            if state.weak != self.0.as_ptr() || state.claims == 0 {
                unsafe { crate::runtime_invariant_panic() };
            }
            state.claims -= 1;
            if state.claims == 0 {
                state.weak = ptr::null_mut();
                true
            } else {
                false
            }
        };
        if last {
            // No association, driver, notification or waiter may now borrow
            // this allocation. Native close failures do not permit retry.
            unsafe {
                let allocation = self.0.as_ref().allocation;
                ptr::drop_in_place(self.0.as_ptr());
                crate::deallocate(allocation);
            }
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

impl Broker {
    fn allocate() -> Result<NonNull<Self>, Failure> {
        // The runtime elects one actual pump. Native concurrency must not cap
        // that election while former dequeuers are doing runnable user work.
        // Acceptance and handoff of this DWORD value need real Windows tests;
        // there is deliberately no lower-concurrency fallback.
        let handle = unsafe {
            CreateIoCompletionPort(usize::MAX as NativeHandle, ptr::null_mut(), 0, u32::MAX)
        };
        if handle.is_null() {
            return Err(last_failure());
        }
        let port = unsafe { OwnedHandle::from_raw(handle) };
        let layout = Layout::new::<Self>();
        let bytes = layout
            .size()
            .checked_add(layout.align() - 1)
            .filter(|bytes| *bytes <= isize::MAX as usize)
            .ok_or_else(Failure::invalid_input)?;
        let allocation = unsafe { crate::allocate(bytes) };
        if allocation.is_null() {
            return Err(exhausted());
        }
        let offset = allocation.align_offset(layout.align());
        if offset > layout.align() - 1 {
            unsafe { crate::deallocate(allocation) };
            return Err(Failure::invalid_input());
        }
        let pointer = unsafe { allocation.add(offset).cast::<Self>() };
        unsafe {
            pointer.write(Self {
                port,
                routing: SyncCell::new(Routing {
                    registry: Registry::new(),
                    pumping: false,
                }),
                notice: NotificationLatch::new(),
                allocation,
            });
            Ok(NonNull::new_unchecked(pointer))
        }
    }

    fn routing(&self) -> Pin<&SyncCell<Routing>> {
        // The broker has been pinned in its allocation before first use.
        unsafe { Pin::new_unchecked(&self.routing) }
    }

    fn notice(&self) -> Pin<&NotificationLatch> {
        unsafe { Pin::new_unchecked(&self.notice) }
    }

    /// The group remains registered and its cancellation state remains claimed
    /// for this whole call. A selected route ignores cancellation while retiring an
    /// already accepted request; shared TransferGroup owns cancel-all/drain-all.
    unsafe fn wait(
        &self,
        group: *mut Group,
        timeout: u32,
        cancellation: *const DrCancellationV1,
        draining: Option<*mut Route>,
    ) -> Result<Wake, Failure> {
        let mut routing = self.routing().lock();
        if ready(&mut routing.registry, group, draining) {
            return Ok(Wake::Completion);
        }
        if draining.is_none() && requested(cancellation) {
            return Ok(Wake::Cancellation);
        }
        if routing.pumping {
            // Sleep atomically releases this same predicate lock. Return after
            // one wake so the shared core refreshes its original budget, even
            // after spurious/foreign-group wakes. Never restart a finite wait.
            let awakened = routing.wait(timeout)?;
            return if ready(&mut routing.registry, group, draining) {
                Ok(Wake::Completion)
            } else if draining.is_none() && requested(cancellation) {
                Ok(Wake::Cancellation)
            } else if awakened {
                Ok(Wake::Completion)
            } else {
                Ok(Wake::Timeout)
            };
        }
        routing.pumping = true;
        let pump = Pump { broker: self };
        drop(routing);
        let result = self.receive(timeout);
        drop(pump);
        // A shared poke is not a cancellation fact for this particular group.
        if result.is_ok() && draining.is_none() && requested(cancellation) {
            Ok(Wake::Cancellation)
        } else {
            result
        }
    }

    fn receive(&self, timeout: u32) -> Result<Wake, Failure> {
        let (mut count, mut key, mut request) = (0, 0, ptr::null_mut());
        let success = unsafe {
            GetQueuedCompletionStatus(self.port.get(), &mut count, &mut key, &mut request, timeout)
        };
        // Capture GetLastError before acquiring a native lock or any other API.
        let code = if success == 0 {
            unsafe { GetLastError() }
        } else {
            0
        };
        if success == 0 && request.is_null() {
            return if code == WAIT_TIMEOUT {
                Ok(Wake::Timeout)
            } else {
                Err(Failure::from_system_code(code.into()))
            };
        }
        if key == 0 && request.is_null() && success != 0 && count == POKE {
            // Clear confirmed queued state before waking current predicates.
            // A new cancellation can then post the next pointer-free packet.
            self.notice().consumed();
            return Ok(Wake::Completion);
        }
        #[cfg(test)]
        if !request.is_null() {
            NATIVE_DEQUEUES.with(|count| count.set(count.get().checked_add(1).unwrap()));
        }
        let result = if success != 0 {
            Ok(count as usize)
        } else {
            Err(Failure::from_system_code(code.into())
                .with_transferred_byte_count(Some(count as usize)))
        };
        let mut routing = self.routing().lock();
        if unsafe { routing.registry.publish(key, request as usize, result) }.is_err() {
            // Provenance/one-packet invariants were broken. Releasing borrowed
            // storage or inventing terminal completion would be unsound.
            unsafe { crate::runtime_invariant_panic() };
        }
        Ok(Wake::Completion)
    }
}

impl Drop for Broker {
    fn drop(&mut self) {
        let routing = self.routing().lock();
        if routing.pumping || !routing.registry.is_empty() {
            unsafe { crate::runtime_invariant_panic() };
        }
    }
}

struct Pump<'a> {
    broker: &'a Broker,
}

impl Drop for Pump<'_> {
    fn drop(&mut self) {
        let mut routing = self.broker.routing().lock();
        if !routing.pumping {
            unsafe { crate::runtime_invariant_panic() };
        }
        routing.pumping = false;
        self.broker.routing().wake_all();
    }
}

// This uses only its isolated, short notification lock and native Post. It does
// not take the routing lock, await waiter processing, retain a borrowed pointer
// in the queue, reenter cancellation, allocate, or unwind.
unsafe impl WakeTarget for Broker {
    fn wake(&self) -> Result<(), Failure> {
        self.notice().notify(|| {
            if unsafe { PostQueuedCompletionStatus(self.port.get(), POKE, 0, ptr::null_mut()) } == 0
            {
                Err(last_failure())
            } else {
                Ok(())
            }
        })
    }
}

unsafe fn requested(state: *const DrCancellationV1) -> bool {
    !state.is_null() && cancellation::dr_v1_cancellation_is_requested(state) != 0
}

unsafe fn ready(registry: &mut Registry, group: *mut Group, draining: Option<*mut Route>) -> bool {
    // Other cached results must not prevent pumping during one route's drain.
    // Query keeps the retired value cached, so the selected drain still obtains
    // its exact terminal outcome after this protected readiness observation.
    match draining {
        Some(route) => registry.query(route).is_some(),
        None => registry.group_ready(group),
    }
}

/// Owns both the device and its immutable native association, independently of
/// readiness-set membership. No publicly usable raw handle escapes this owner.
pub(super) struct AttachedStream {
    owner: Option<OwnedHandle>,
    association: FixedSlice<Association>,
    broker: BrokerClaim,
}

// Moving this owner does not move its association. Configuration and I/O need
// exclusive set/group access; immutable metadata and the broker are synchronized.
unsafe impl Send for AttachedStream {}
unsafe impl Sync for AttachedStream {}

impl AttachedStream {
    #[cfg(test)]
    fn identity_for_test(&self) -> (usize, usize, usize) {
        (
            self.handle() as usize,
            self.association.as_slice()[0].key,
            self.broker.0.as_ptr() as usize,
        )
    }

    /// # Safety
    /// The owner is idle and created FILE_FLAG_OVERLAPPED, byte-mode PIPE_WAIT,
    /// with no native port/APC/external I/O or close consumer and no notification
    /// suppression. Every accepted ReadFile/WriteFile produces one packet.
    /// Adoption consumes the supplied owner, including on failure.
    pub(super) unsafe fn adopt(owner: OwnedHandle) -> Result<Self, Failure> {
        let broker = BrokerClaim::acquire()?;
        let mut association = FixedSlice::new(1)?;
        let key = broker.broker().routing().lock().registry.reserve_key()?;
        association.push(Association::new(key))?;
        let stream = Self {
            owner: Some(owner),
            association,
            broker,
        };
        {
            let mut routing = stream.broker.broker().routing().lock();
            if routing
                .registry
                .insert_association(stream.association.raw(0))
                .is_err()
            {
                crate::runtime_invariant_panic();
            }
        }
        // All allocations and routing capacity exist before irreversible native
        // attachment. Rollback removes only our metadata, never pretends detach.
        let port = stream.broker.broker().port.get();
        if CreateIoCompletionPort(stream.handle(), port, key, 0) != port {
            return Err(last_failure());
        }
        Ok(stream)
    }

    fn handle(&self) -> NativeHandle {
        self.owner.as_ref().expect("live attached stream").get()
    }

    pub(super) fn close(mut self) -> Result<(), Failure> {
        self.retire();
        // Routes have retired and been removed before this consuming attempt.
        // Failure remains unconfirmed host closure, not a retryable ownership
        // claim; no borrowed request/buffer pointers remain in the registry.
        self.owner.take().expect("live attached stream").close()
    }

    fn retire(&mut self) {
        if self.owner.is_none() {
            return;
        }
        let mut routing = self.broker.broker().routing().lock();
        if unsafe { routing.registry.remove_association(self.association.raw(0)) }.is_err() {
            unsafe { crate::runtime_invariant_panic() };
        }
    }
}

impl Drop for AttachedStream {
    fn drop(&mut self) {
        if self.owner.is_some() {
            self.retire();
            // One attempt even on host close failure. All packet obligations
            // are already retired; the metadata contains value identities only.
            drop(self.owner.take());
        }
    }
}

unsafe extern "system" {
    fn CreateIoCompletionPort(
        handle: NativeHandle,
        port: NativeHandle,
        key: usize,
        concurrency: u32,
    ) -> NativeHandle;
    fn GetQueuedCompletionStatus(
        port: NativeHandle,
        count: *mut u32,
        key: *mut usize,
        request: *mut *mut c_void,
        timeout: u32,
    ) -> i32;
    fn PostQueuedCompletionStatus(
        port: NativeHandle,
        count: u32,
        key: usize,
        request: *mut c_void,
    ) -> i32;
}
