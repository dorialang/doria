//! Value-only routing for stable, caller-owned completion records.
//!
//! The broker's lock is the sole authority for this registry and every node's
//! mutable state. No pointer in a completion packet is dereferenced: its numeric
//! association key is found first, then its numeric request address is matched
//! against a registered route. Borrowed bytes are never stored or accessed here.
//!
//! # Safety
//! Every unsafe operation requires the same exclusive external broker lock.
//! Inserted nodes must remain pinned, initialized, and alive until removal; their
//! state may not be accessed through another registry or outside that lock. An
//! association's key belongs to that one live stream and is immutable from first
//! insertion until the stream can no longer produce packets. New stream
//! identities must use fresh keys from this registry, never an old stream's key.
//! A request may be reused only after its one accepted packet has been retired
//! and the submitting FFI call has finalized. Native submission/rejection facts
//! and packet multiplicity must be truthful; this registry cannot prove them.

use core::cell::UnsafeCell;
use core::marker::PhantomPinned;
use core::ptr;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::platform_io::{Failure, Reason};

#[cfg(test)]
#[path = "routing/tests.rs"]
mod tests;

pub(super) struct Association {
    pub(super) key: usize,
    state: UnsafeCell<AssociationState>,
    _pinned: PhantomPinned,
}

struct AssociationState {
    linked: bool,
    next: *mut Association,
}

impl Association {
    pub(super) const fn new(key: usize) -> Self {
        Self {
            key,
            state: UnsafeCell::new(AssociationState {
                linked: false,
                next: ptr::null_mut(),
            }),
            _pinned: PhantomPinned,
        }
    }
}

pub(super) struct Group {
    state: UnsafeCell<GroupState>,
    pub(super) helper_ready: AtomicBool,
    _pinned: PhantomPinned,
}

struct GroupState {
    linked: bool,
    next: *mut Group,
    unreported: usize,
}

impl Group {
    pub(super) const fn new() -> Self {
        Self {
            state: UnsafeCell::new(GroupState {
                linked: false,
                next: ptr::null_mut(),
                unreported: 0,
            }),
            helper_ready: AtomicBool::new(false),
            _pinned: PhantomPinned,
        }
    }
}

pub(super) struct Route {
    state: UnsafeCell<RouteState>,
    _pinned: PhantomPinned,
}

struct RouteState {
    linked: bool,
    next: *mut Route,
    association: *mut Association,
    group: *mut Group,
    request: usize,
    submitting: bool,
    packet: bool,
    terminal: Option<Result<usize, Failure>>,
    unreported: bool,
}

impl RouteState {
    const fn new() -> Self {
        Self {
            linked: false,
            next: ptr::null_mut(),
            association: ptr::null_mut(),
            group: ptr::null_mut(),
            request: 0,
            submitting: false,
            packet: false,
            terminal: None,
            unreported: false,
        }
    }
}

impl Route {
    pub(super) const fn new() -> Self {
        Self {
            state: UnsafeCell::new(RouteState::new()),
            _pinned: PhantomPinned,
        }
    }
}

pub(super) struct Registry {
    last_key: usize,
    associations: *mut Association,
    groups: *mut Group,
    routes: *mut Route,
}

// Nodes contain only identities and copyable completion facts. Moving a node
// before insertion is harmless; moving its ownership between threads after
// insertion does not move its pinned allocation. All linked access still needs
// the one external lock, and Registry itself is not Sync.
unsafe impl Send for Association {}
unsafe impl Send for Group {}
unsafe impl Send for Route {}
unsafe impl Send for Registry {}

impl Registry {
    pub(super) const fn new() -> Self {
        Self {
            last_key: 0,
            associations: ptr::null_mut(),
            groups: ptr::null_mut(),
            routes: ptr::null_mut(),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.associations.is_null() && self.groups.is_null() && self.routes.is_null()
    }

    #[cfg(test)]
    pub(super) unsafe fn route_registered(&self, node: *mut Route) -> bool {
        self.contains_route(node)
    }

    #[cfg(test)]
    pub(super) unsafe fn group_registered(&self, node: *mut Group) -> bool {
        self.contains_group(node)
    }

    pub(super) fn reserve_key(&mut self) -> Result<usize, Failure> {
        let key = self.last_key.checked_add(1).ok_or(Failure {
            reason: Reason::ResourceExhausted,
            system_code: None,
            transferred_byte_count: None,
        })?;
        self.last_key = key;
        Ok(key)
    }

    /// # Safety
    /// Obey the module contract; `node` must name a live initialized node even
    /// when it is not already registered. Its nonzero key must have been reserved.
    pub(super) unsafe fn insert_association(
        &mut self,
        node: *mut Association,
    ) -> Result<(), Failure> {
        if node.is_null() {
            return Err(Failure::invalid_input());
        }
        let key = unsafe { (*node).key };
        let state = unsafe { &mut *(*node).state.get() };
        if state.linked || key == 0 || key > self.last_key {
            return Err(Failure::invalid_input());
        }
        let mut cursor = self.associations;
        while !cursor.is_null() {
            if unsafe { (*cursor).key } == key {
                return Err(Failure::invalid_input());
            }
            cursor = unsafe { (*(*cursor).state.get()).next };
        }
        state.linked = true;
        state.next = self.associations;
        self.associations = node;
        Ok(())
    }

    /// # Safety
    /// Obey the module contract. Removal never retires native packet obligations.
    pub(super) unsafe fn remove_association(
        &mut self,
        node: *mut Association,
    ) -> Result<(), Failure> {
        let mut cursor = self.associations;
        let mut previous: *mut Association = ptr::null_mut();
        while !cursor.is_null() && cursor != node {
            previous = cursor;
            cursor = unsafe { (*(*cursor).state.get()).next };
        }
        if cursor.is_null() {
            return Err(Failure::invalid_input());
        }
        let mut route = self.routes;
        while !route.is_null() {
            let state = unsafe { &*(*route).state.get() };
            if state.association == node {
                return Err(Failure::invalid_input());
            }
            route = state.next;
        }
        let state = unsafe { &mut *(*node).state.get() };
        if previous.is_null() {
            self.associations = state.next;
        } else {
            unsafe { (*(*previous).state.get()).next = state.next };
        }
        state.linked = false;
        state.next = ptr::null_mut();
        Ok(())
    }

    /// # Safety
    /// Obey the module contract; `node` must name a live initialized node.
    pub(super) unsafe fn insert_group(&mut self, node: *mut Group) -> Result<(), Failure> {
        if node.is_null() {
            return Err(Failure::invalid_input());
        }
        let state = unsafe { &mut *(*node).state.get() };
        if state.linked || state.unreported != 0 {
            return Err(Failure::invalid_input());
        }
        state.linked = true;
        state.next = self.groups;
        (*node).helper_ready.store(false, Ordering::Release);
        self.groups = node;
        Ok(())
    }

    /// # Safety
    /// Obey the module contract. All of this group's routes must be removed first.
    pub(super) unsafe fn remove_group(&mut self, node: *mut Group) -> Result<(), Failure> {
        let mut cursor = self.groups;
        let mut previous: *mut Group = ptr::null_mut();
        while !cursor.is_null() && cursor != node {
            previous = cursor;
            cursor = unsafe { (*(*cursor).state.get()).next };
        }
        if cursor.is_null() {
            return Err(Failure::invalid_input());
        }
        let mut route = self.routes;
        while !route.is_null() {
            let state = unsafe { &*(*route).state.get() };
            if state.group == node {
                return Err(Failure::invalid_input());
            }
            route = state.next;
        }
        let state = unsafe { &mut *(*node).state.get() };
        if state.unreported != 0 {
            return Err(Failure::invalid_input());
        }
        if previous.is_null() {
            self.groups = state.next;
        } else {
            unsafe { (*(*previous).state.get()).next = state.next };
        }
        state.linked = false;
        state.next = ptr::null_mut();
        (*node).helper_ready.store(false, Ordering::Release);
        Ok(())
    }

    /// # Safety
    /// Obey the module contract; `route` must name a live initialized node. The
    /// request address is an identity, not a pointer this registry may dereference.
    pub(super) unsafe fn insert_route(
        &mut self,
        association: *mut Association,
        group: *mut Group,
        route: *mut Route,
        request: usize,
    ) -> Result<(), Failure> {
        if route.is_null()
            || request == 0
            || !unsafe { self.contains_association(association) }
            || !unsafe { self.contains_group(group) }
        {
            return Err(Failure::invalid_input());
        }
        let state = unsafe { &mut *(*route).state.get() };
        if state.linked {
            return Err(Failure::invalid_input());
        }
        let mut cursor = self.routes;
        while !cursor.is_null() {
            let other = unsafe { &*(*cursor).state.get() };
            if other.request == request {
                return Err(Failure::invalid_input());
            }
            cursor = other.next;
        }
        *state = RouteState::new();
        state.linked = true;
        state.next = self.routes;
        state.association = association;
        state.group = group;
        state.request = request;
        self.routes = route;
        Ok(())
    }

    /// # Safety
    /// Obey the module contract. An unreported retired value may be discarded,
    /// but no submitting call or outstanding native packet may be forgotten.
    pub(super) unsafe fn remove_route(&mut self, node: *mut Route) -> Result<(), Failure> {
        let mut cursor = self.routes;
        let mut previous: *mut Route = ptr::null_mut();
        while !cursor.is_null() && cursor != node {
            previous = cursor;
            cursor = unsafe { (*(*cursor).state.get()).next };
        }
        if cursor.is_null() {
            return Err(Failure::invalid_input());
        }
        let state = unsafe { &mut *(*node).state.get() };
        if state.submitting || state.packet {
            return Err(Failure::invalid_input());
        }
        if state.unreported {
            unsafe { (*(*state.group).state.get()).unreported -= 1 };
        }
        if previous.is_null() {
            self.routes = state.next;
        } else {
            unsafe { (*(*previous).state.get()).next = state.next };
        }
        *state = RouteState::new();
        Ok(())
    }

    /// # Safety
    /// Obey the module contract. Call before entering the native submit FFI.
    pub(super) unsafe fn arm(&mut self, node: *mut Route) -> Result<(), Failure> {
        if !unsafe { self.contains_route(node) } {
            return Err(Failure::invalid_input());
        }
        let state = unsafe { &mut *(*node).state.get() };
        if state.submitting || state.packet || state.unreported {
            return Err(Failure::invalid_input());
        }
        state.terminal = None;
        state.submitting = true;
        state.packet = true;
        Ok(())
    }

    /// # Safety
    /// Obey the module contract. The FFI accepted exactly one packet-producing
    /// transfer, possibly already dequeued by a different group's caller.
    pub(super) unsafe fn accepted(&mut self, node: *mut Route) -> Result<(), Failure> {
        if !unsafe { self.contains_route(node) } {
            return Err(Failure::invalid_input());
        }
        let state = unsafe { &mut *(*node).state.get() };
        if !state.submitting || state.packet == state.terminal.is_some() {
            return Err(Failure::invalid_input());
        }
        if state.terminal.is_some() {
            let group = unsafe { &mut *(*state.group).state.get() };
            let count = group
                .unreported
                .checked_add(1)
                .ok_or_else(Failure::invalid_input)?;
            group.unreported = count;
            state.unreported = true;
        }
        state.submitting = false;
        Ok(())
    }

    /// # Safety
    /// Obey the module contract. A rejected transfer must produce no packet.
    ///
    /// Every armed submission ends with exactly one terminal: a published
    /// packet for an accepted transfer, or here, the synchronous failure of a
    /// rejected one. Recording it lets every terminal wait (`query`, the
    /// broker's drain predicate) observe the rejection instead of waiting for a
    /// packet that cannot come. The group already received this failure from
    /// submission, so it is not counted as unreported.
    pub(super) unsafe fn rejected(
        &mut self,
        node: *mut Route,
        failure: Failure,
    ) -> Result<(), Failure> {
        if !unsafe { self.contains_route(node) } {
            return Err(Failure::invalid_input());
        }
        let state = unsafe { &mut *(*node).state.get() };
        if !state.submitting || !state.packet || state.terminal.is_some() {
            return Err(Failure::invalid_input());
        }
        state.submitting = false;
        state.packet = false;
        state.terminal = Some(Err(failure));
        Ok(())
    }

    /// # Safety
    /// Obey the module contract. `result` is the packet's terminal fact; progress
    /// bounds are checked by the transfer owner that knows the requested length.
    pub(super) unsafe fn publish(
        &mut self,
        key: usize,
        request: usize,
        result: Result<usize, Failure>,
    ) -> Result<(), Failure> {
        let mut association = self.associations;
        while !association.is_null() && unsafe { (*association).key } != key {
            association = unsafe { (*(*association).state.get()).next };
        }
        if association.is_null() || request == 0 {
            return Err(Failure::invalid_input());
        }
        let mut route = self.routes;
        while !route.is_null() {
            let state = unsafe { &mut *(*route).state.get() };
            if state.association == association && state.request == request {
                if !state.packet || state.terminal.is_some() {
                    return Err(Failure::invalid_input());
                }
                if !state.submitting {
                    let group = unsafe { &mut *(*state.group).state.get() };
                    let count = group
                        .unreported
                        .checked_add(1)
                        .ok_or_else(Failure::invalid_input)?;
                    group.unreported = count;
                    state.unreported = true;
                }
                state.packet = false;
                state.terminal = Some(result);
                return Ok(());
            }
            route = state.next;
        }
        Err(Failure::invalid_input())
    }

    /// # Safety
    /// Obey the module contract. The returned fact is copied; no node or buffer
    /// reference escapes the lock. Repeated queries retain the same cached value.
    pub(super) unsafe fn query(&mut self, node: *mut Route) -> Option<Result<usize, Failure>> {
        if !unsafe { self.contains_route(node) } {
            return None;
        }
        let state = unsafe { &mut *(*node).state.get() };
        if state.submitting {
            return None;
        }
        let result = state.terminal?;
        if state.unreported {
            unsafe { (*(*state.group).state.get()).unreported -= 1 };
            state.unreported = false;
        }
        Some(result)
    }

    /// # Safety
    /// Obey the module contract. Readiness persists until each first query or
    /// removal; unrelated packet pumping never consumes another group's values.
    pub(super) unsafe fn group_ready(&self, node: *mut Group) -> bool {
        unsafe {
            self.contains_group(node)
                && ((*(*node).state.get()).unreported != 0
                    || (*node).helper_ready.swap(false, Ordering::AcqRel))
        }
    }

    /// # Safety
    /// Obey the module contract. A dequeued early packet is still active until
    /// the FFI caller finalizes acceptance, despite having no packet left queued.
    pub(super) unsafe fn active(&self, node: *mut Route) -> bool {
        if !unsafe { self.contains_route(node) } {
            return false;
        }
        let state = unsafe { &*(*node).state.get() };
        state.submitting || state.packet
    }

    unsafe fn contains_association(&self, node: *mut Association) -> bool {
        let mut cursor = self.associations;
        while !cursor.is_null() {
            if cursor == node {
                return true;
            }
            cursor = unsafe { (*(*cursor).state.get()).next };
        }
        false
    }

    unsafe fn contains_group(&self, node: *mut Group) -> bool {
        let mut cursor = self.groups;
        while !cursor.is_null() {
            if cursor == node {
                return true;
            }
            cursor = unsafe { (*(*cursor).state.get()).next };
        }
        false
    }

    unsafe fn contains_route(&self, node: *mut Route) -> bool {
        let mut cursor = self.routes;
        while !cursor.is_null() {
            if cursor == node {
                return true;
            }
            cursor = unsafe { (*(*cursor).state.get()).next };
        }
        false
    }
}
