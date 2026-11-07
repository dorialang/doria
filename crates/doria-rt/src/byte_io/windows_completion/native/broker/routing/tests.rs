use core::pin::Pin;
use core::ptr::NonNull;
use std::boxed::Box;

use super::*;

const REQUESTS: [usize; 3] = [0x1000, 0x2000, 0x3000];

// These addresses model packet identity only, not allocated OVERLAPPEDs or fake
// slices. Every sequence is serialized like operations under the broker lock;
// the tests exercise interleavings, not OS execution or mutex implementation.
struct Model {
    registry: Registry,
    keys: [usize; 2],
    association_pointers: [*mut Association; 2],
    group_pointers: [*mut Group; 2],
    route_pointers: [*mut Route; 3],
    _associations: [Pin<Box<Association>>; 2],
    _groups: [Pin<Box<Group>>; 2],
    _routes: [Pin<Box<Route>>; 3],
}

fn raw<T>(node: Pin<&mut T>) -> *mut T {
    // The reference ends at this function's return. Its pinned allocation stays
    // alive in Model; protected state is accessed only by registry operations.
    unsafe { node.get_unchecked_mut() as *mut T }
}

impl Model {
    fn new() -> Self {
        let mut registry = Registry::new();
        let keys = [
            registry.reserve_key().unwrap(),
            registry.reserve_key().unwrap(),
        ];
        let mut associations = [
            Box::pin(Association::new(keys[0])),
            Box::pin(Association::new(keys[1])),
        ];
        let mut groups = [Box::pin(Group::new()), Box::pin(Group::new())];
        let mut routes = [
            Box::pin(Route::new()),
            Box::pin(Route::new()),
            Box::pin(Route::new()),
        ];
        // Establish raw identities once, before registration. No later helper
        // creates a new exclusive reference over linked, dispatcher-owned state.
        let association_pointers = [raw(associations[0].as_mut()), raw(associations[1].as_mut())];
        let group_pointers = [raw(groups[0].as_mut()), raw(groups[1].as_mut())];
        let route_pointers = [
            raw(routes[0].as_mut()),
            raw(routes[1].as_mut()),
            raw(routes[2].as_mut()),
        ];
        let mut model = Self {
            registry,
            keys,
            association_pointers,
            group_pointers,
            route_pointers,
            _associations: associations,
            _groups: groups,
            _routes: routes,
        };
        for index in 0..2 {
            let association = model.association(index);
            let group = model.group(index);
            unsafe {
                model.registry.insert_association(association).unwrap();
                model.registry.insert_group(group).unwrap();
                assert!(model.registry.group_registered(group));
            }
        }
        for (index, request) in REQUESTS.iter().copied().enumerate() {
            let owner = usize::from(index == 2);
            let association = model.association(owner);
            let group = model.group(owner);
            let route = model.route(index);
            unsafe {
                model
                    .registry
                    .insert_route(association, group, route, request)
                    .unwrap();
                assert!(model.registry.route_registered(route));
            }
        }
        model
    }

    fn association(&self, index: usize) -> *mut Association {
        self.association_pointers[index]
    }

    fn group(&self, index: usize) -> *mut Group {
        self.group_pointers[index]
    }

    fn route(&self, index: usize) -> *mut Route {
        self.route_pointers[index]
    }

    fn key(&self, index: usize) -> usize {
        self.keys[index]
    }

    fn start(&mut self, index: usize) {
        let route = self.route(index);
        unsafe {
            self.registry.arm(route).unwrap();
            self.registry.accepted(route).unwrap();
        }
    }

    fn publish(&mut self, index: usize, result: Result<usize, Failure>) {
        let owner = usize::from(index == 2);
        unsafe {
            self.registry
                .publish(self.key(owner), REQUESTS[index], result)
                .unwrap();
        }
    }

    fn query(&mut self, index: usize) -> Option<Result<usize, Failure>> {
        let route = self.route(index);
        unsafe { self.registry.query(route) }
    }

    fn ready(&mut self, index: usize) -> bool {
        let group = self.group(index);
        unsafe { self.registry.group_ready(group) }
    }

    fn finish(mut self) {
        for index in 0..3 {
            let route = self.route(index);
            unsafe {
                assert!(!self.registry.active(route));
                if self.registry.contains_route(route) {
                    self.registry.remove_route(route).unwrap();
                }
            }
        }
        for index in 0..2 {
            let group = self.group(index);
            let association = self.association(index);
            unsafe {
                if self.registry.contains_group(group) {
                    self.registry.remove_group(group).unwrap();
                }
                if self.registry.contains_association(association) {
                    self.registry.remove_association(association).unwrap();
                }
            }
        }
        assert!(self.registry.is_empty());
    }
}

#[test]
fn early_packet_is_hidden_until_acceptance_and_acknowledged_once() {
    let mut model = Model::new();
    let route = model.route(0);
    unsafe {
        model.registry.arm(route).unwrap();
        model.publish(0, Ok(7));
        assert!(model.registry.active(route));
        assert_eq!(model.registry.query(route), None);
        assert!(!model.ready(0));
        assert_eq!(model.registry.arm(route), Err(Failure::invalid_input()));
        assert_eq!(
            model.registry.remove_route(route),
            Err(Failure::invalid_input())
        );
        model.registry.accepted(route).unwrap();
        assert!(!model.registry.active(route));
        assert!(model.ready(0));
        assert_eq!(model.registry.query(route), Some(Ok(7)));
        assert!(!model.ready(0));
        assert_eq!(model.registry.query(route), Some(Ok(7)));
        assert!(!model.ready(0));
    }
    model.finish();
}

#[test]
fn foreign_group_pump_preserves_both_groups_durable_results() {
    let mut model = Model::new();
    model.start(0);
    model.start(1);
    model.start(2);
    // A caller waiting for group 0 routes group 1's packet first.
    model.publish(2, Ok(11));
    assert!(!model.ready(0));
    assert!(model.ready(1));
    assert_eq!(model.query(0), None);
    model.publish(1, Ok(3));
    model.publish(0, Ok(5));
    assert!(model.ready(0));
    assert!(model.ready(1));
    assert_eq!(model.query(1), Some(Ok(3)));
    assert!(model.ready(0));
    assert!(model.ready(1));
    assert_eq!(model.query(0), Some(Ok(5)));
    assert!(!model.ready(0));
    assert!(model.ready(1));
    assert_eq!(model.query(2), Some(Ok(11)));
    assert!(!model.ready(1));
    model.finish();
}

#[test]
fn same_request_requires_packet_retirement_and_consumed_outcome_before_rearm() {
    let mut model = Model::new();
    let route = model.route(0);
    model.start(0);
    unsafe {
        assert_eq!(model.registry.arm(route), Err(Failure::invalid_input()));
        assert_eq!(
            model.registry.remove_route(route),
            Err(Failure::invalid_input())
        );
    }
    model.publish(0, Ok(2));
    unsafe {
        assert_eq!(model.registry.arm(route), Err(Failure::invalid_input()));
    }
    assert!(model.ready(0));
    assert_eq!(model.query(0), Some(Ok(2)));
    let address = route as usize;
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        for count in 0..16 {
            assert_eq!(model.route(0) as usize, address);
            model.start(0);
            assert_eq!(model.query(0), None);
            model.publish(0, Ok(count));
            assert_eq!(model.query(0), Some(Ok(count)));
            assert!(!model.ready(0));
        }
    });
    // Counts Doria runtime allocations, not this fixture's earlier Rust Box
    // allocations or operating-system resources (none are constructed here).
    assert_eq!(allocations, 0);
    model.finish();
}

#[test]
fn rejected_submission_has_no_packet_and_early_packet_cannot_be_rejected() {
    let mut model = Model::new();
    let route = model.route(0);
    unsafe {
        assert_eq!(
            model.registry.accepted(route),
            Err(Failure::invalid_input())
        );
        assert_eq!(
            model.registry.rejected(route, Failure::other()),
            Err(Failure::invalid_input())
        );
        model.registry.arm(route).unwrap();
        // A synchronously failed submission (ReadFile FALSE with
        // ERROR_BROKEN_PIPE) queues no packet. Its terminal is the rejection
        // itself, so a drain that queries this route ends at once instead of
        // waiting for a packet that cannot come. Before this was recorded, the
        // route stayed terminal-less forever, the state that hung Windows CI.
        let broken = Failure::from_system_code(109);
        model.registry.rejected(route, broken).unwrap();
        assert!(!model.registry.active(route));
        assert_eq!(model.registry.query(route), Some(Err(broken)));
        assert_eq!(model.registry.query(route), Some(Err(broken)));
        assert_eq!(
            model.registry.publish(model.key(0), REQUESTS[0], Ok(0)),
            Err(Failure::invalid_input())
        );
        model.registry.arm(route).unwrap();
        model.publish(
            0,
            Err(Failure::other().with_transferred_byte_count(Some(2))),
        );
        assert_eq!(
            model.registry.rejected(route, Failure::other()),
            Err(Failure::invalid_input())
        );
        assert!(model.registry.active(route));
        assert_eq!(model.registry.query(route), None);
        model.registry.accepted(route).unwrap();
    }
    assert_eq!(
        model.query(0),
        Some(Err(Failure::other().with_transferred_byte_count(Some(2))))
    );
    model.finish();
}

#[test]
fn wrong_key_address_or_duplicate_packet_does_not_mutate_live_route() {
    let mut model = Model::new();
    let route = model.route(0);
    model.start(0);
    unsafe {
        for (key, request) in [
            (0, REQUESTS[0]),
            (usize::MAX, usize::MAX),
            (model.key(1), REQUESTS[0]),
            (model.key(0), usize::MAX),
            (model.key(0), 0),
        ] {
            assert_eq!(
                model.registry.publish(key, request, Ok(99)),
                Err(Failure::invalid_input())
            );
            assert!(model.registry.active(route));
            assert_eq!(model.registry.query(route), None);
        }
    }
    model.publish(0, Ok(4));
    unsafe {
        assert_eq!(
            model
                .registry
                .publish(model.key(0), REQUESTS[0], Err(Failure::other())),
            Err(Failure::invalid_input())
        );
        assert_eq!(
            model.registry.accepted(route),
            Err(Failure::invalid_input())
        );
        assert_eq!(
            model.registry.rejected(route, Failure::other()),
            Err(Failure::invalid_input())
        );
    }
    assert_eq!(model.query(0), Some(Ok(4)));
    model.finish();
}

#[test]
fn removal_discards_retired_values_but_never_live_obligations() {
    let mut model = Model::new();
    let group = model.group(0);
    let association = model.association(0);
    let first = model.route(0);
    let second = model.route(1);
    model.start(0);
    model.start(1);
    unsafe {
        assert_eq!(
            model.registry.remove_group(group),
            Err(Failure::invalid_input())
        );
        assert_eq!(
            model.registry.remove_association(association),
            Err(Failure::invalid_input())
        );
        assert_eq!(
            model.registry.remove_route(first),
            Err(Failure::invalid_input())
        );
    }
    model.publish(0, Ok(6));
    model.publish(1, Ok(8));
    assert!(model.ready(0));
    unsafe { model.registry.remove_route(first).unwrap() };
    assert!(model.ready(0));
    unsafe { model.registry.remove_route(second).unwrap() };
    assert!(!model.ready(0));
    unsafe {
        model.registry.remove_group(group).unwrap();
        model.registry.remove_association(association).unwrap();
        assert_eq!(
            model.registry.remove_route(first),
            Err(Failure::invalid_input())
        );
        assert_eq!(
            model.registry.remove_group(group),
            Err(Failure::invalid_input())
        );
        assert_eq!(
            model.registry.remove_association(association),
            Err(Failure::invalid_input())
        );
        assert_eq!(
            model.registry.publish(model.key(0), usize::MAX, Ok(0)),
            Err(Failure::invalid_input())
        );
    }
    model.finish();
}

#[test]
fn stream_identity_survives_group_membership_removal_and_reregistration() {
    let mut model = Model::new();
    let association = model.association(0);
    let old_group = model.group(0);
    let new_group = model.group(1);
    let route = model.route(0);
    let other = model.route(1);
    let key = model.key(0);
    model.start(0);
    model.publish(0, Ok(1));
    unsafe {
        model.registry.remove_route(route).unwrap();
        model.registry.remove_route(other).unwrap();
        model.registry.remove_group(old_group).unwrap();
        assert!(model.registry.contains_association(association));
        model
            .registry
            .insert_route(association, new_group, route, 0x4000)
            .unwrap();
        model.registry.arm(route).unwrap();
        model.registry.accepted(route).unwrap();
        assert_eq!(
            model.registry.publish(key, REQUESTS[0], Ok(2)),
            Err(Failure::invalid_input())
        );
        model.registry.publish(key, 0x4000, Ok(9)).unwrap();
        assert!(!model.registry.group_ready(old_group));
        assert!(model.registry.group_ready(new_group));
        assert_eq!(model.registry.query(route), Some(Ok(9)));
        model.registry.remove_route(route).unwrap();
        // A live stream's association may itself be unlinked/relinked only
        // without routes. A new stream must reserve a different numeric key.
        model.registry.remove_association(association).unwrap();
        assert_eq!(
            model.registry.publish(key, 0x4000, Ok(3)),
            Err(Failure::invalid_input())
        );
        model.registry.insert_association(association).unwrap();
    }
    assert_eq!(model.key(0), key);
    assert!(model.registry.reserve_key().unwrap() > key);
    model.finish();
}

#[test]
fn duplicate_registration_and_unregistered_pointers_are_rejected_without_dereference() {
    let mut model = Model::new();
    let association = model.association(0);
    let group = model.group(0);
    let route = model.route(0);
    let mut extra_route = Box::pin(Route::new());
    let extra = raw(extra_route.as_mut());
    let mut duplicate_key = Box::pin(Association::new(model.key(0)));
    let duplicate = raw(duplicate_key.as_mut());
    let mut zero_key = Box::pin(Association::new(0));
    let zero = raw(zero_key.as_mut());
    let mut unreserved_key = Box::pin(Association::new(usize::MAX));
    let unreserved = raw(unreserved_key.as_mut());
    unsafe {
        assert_eq!(
            model.registry.insert_association(association),
            Err(Failure::invalid_input())
        );
        assert_eq!(
            model.registry.insert_group(group),
            Err(Failure::invalid_input())
        );
        assert_eq!(
            model.registry.insert_association(duplicate),
            Err(Failure::invalid_input())
        );
        assert_eq!(
            model.registry.insert_association(zero),
            Err(Failure::invalid_input())
        );
        assert_eq!(
            model.registry.insert_association(unreserved),
            Err(Failure::invalid_input())
        );
        assert_eq!(
            model
                .registry
                .insert_route(association, group, route, 0x4000),
            Err(Failure::invalid_input())
        );
        assert_eq!(
            model
                .registry
                .insert_route(association, group, extra, REQUESTS[0]),
            Err(Failure::invalid_input())
        );
        assert_eq!(
            model.registry.insert_route(association, group, extra, 0),
            Err(Failure::invalid_input())
        );
        assert_eq!(
            model
                .registry
                .insert_route(ptr::null_mut(), group, extra, 0x4000),
            Err(Failure::invalid_input())
        );
        assert_eq!(
            model
                .registry
                .insert_route(association, ptr::null_mut(), extra, 0x4000),
            Err(Failure::invalid_input())
        );

        // These removal/query inputs have no live pointee. Numeric membership
        // checks must return without trying to read them.
        let unknown_route = NonNull::<Route>::dangling().as_ptr();
        let unknown_group = NonNull::<Group>::dangling().as_ptr();
        let unknown_association = NonNull::<Association>::dangling().as_ptr();
        assert_eq!(
            model.registry.remove_route(unknown_route),
            Err(Failure::invalid_input())
        );
        assert_eq!(
            model.registry.remove_group(unknown_group),
            Err(Failure::invalid_input())
        );
        assert_eq!(
            model.registry.remove_association(unknown_association),
            Err(Failure::invalid_input())
        );
        assert_eq!(
            model.registry.arm(unknown_route),
            Err(Failure::invalid_input())
        );
        assert_eq!(
            model.registry.accepted(unknown_route),
            Err(Failure::invalid_input())
        );
        assert_eq!(
            model.registry.rejected(unknown_route, Failure::other()),
            Err(Failure::invalid_input())
        );
        assert_eq!(model.registry.query(unknown_route), None);
        assert!(!model.registry.active(unknown_route));
        assert!(!model.registry.group_ready(unknown_group));
    }
    model.finish();
}

#[test]
fn terminal_progress_preserves_unknown_zero_positive_and_large_facts() {
    let mut model = Model::new();
    let failure = Failure {
        reason: Reason::Interrupted,
        system_code: Some(995),
        transferred_byte_count: None,
    };
    for result in [
        Ok(0),
        Ok(3),
        Ok(usize::MAX),
        Err(failure),
        Err(failure.with_transferred_byte_count(Some(0))),
        Err(failure.with_transferred_byte_count(Some(3))),
        Err(failure.with_transferred_byte_count(Some(usize::MAX))),
    ] {
        model.start(0);
        model.publish(0, result);
        assert_eq!(model.query(0), Some(result));
        assert_eq!(model.query(0), Some(result));
    }
    // Routing has no requested byte length and must not turn unknown failure
    // progress into zero or invent a bound. The group owner validates counts.
    model.finish();
}

#[test]
fn reserved_numeric_identity_never_wraps_or_reuses_discarded_keys() {
    let mut registry = Registry::new();
    assert!(registry.is_empty());
    assert_eq!(registry.reserve_key(), Ok(1));
    assert_eq!(registry.reserve_key(), Ok(2));
    registry.last_key = usize::MAX - 1;
    assert_eq!(registry.reserve_key(), Ok(usize::MAX));
    let exhausted = Failure {
        reason: Reason::ResourceExhausted,
        system_code: None,
        transferred_byte_count: None,
    };
    assert_eq!(registry.reserve_key(), Err(exhausted));
    assert_eq!(registry.reserve_key(), Err(exhausted));
    assert_eq!(registry.last_key, usize::MAX);
    assert!(registry.is_empty());
}

#[test]
fn registry_and_opaque_node_ownership_are_send() {
    fn assert_send<T: Send>() {}
    assert_send::<Registry>();
    assert_send::<Association>();
    assert_send::<Group>();
    assert_send::<Route>();
}
