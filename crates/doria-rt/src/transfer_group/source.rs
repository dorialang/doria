//! Shared current-source binding and lifetime model. Platform routes live in
//! reusable pinned homes; a registration's latest route never replaces an
//! operation's already-selected route. This module does not submit or retire
//! native work and does not own language borrows or a second group engine.
use core::{ffi::c_void, ptr};

use super::{
    adapter::{AdapterTerminal, DrAdapterContextV1, OwnedIoError},
    StableSlots, TransferResource,
};
use crate::file_io::stream::transport::object::ResourcePin;
use crate::native_handle::{NativeHandle, PhysicalOrigin, ReadinessProjection};
use crate::platform_io::{Failure, Reason};
use crate::{DrInterfaceCarrierV1, DrStackFrameV2};

/// One compiler-emitted ordinary getter dispatch thunk. The exact retained
/// receiver home, caller frame, outputs and context are borrowed for this call.
/// Zero returns a source/current dispatch context; one moves the original Error.
/// No backend may substitute an admission-time source for this resolution.
pub type ResolveSourceCallback = unsafe extern "C" fn(
    *const DrStackFrameV2,
    *const DrInterfaceCarrierV1,
    *mut *const c_void,
    *mut DrAdapterContextV1,
    *mut DrInterfaceCarrierV1,
) -> u8;

#[derive(Clone, Copy)]
pub(crate) enum SourceProvider {
    Native {
        receiver_home: *const DrInterfaceCarrierV1,
        resolve: ResolveSourceCallback,
    },
    /// The hosted controller executes the ordinary getter itself and supplies
    /// its fresh result at every source-dependent boundary. No PHP C callbacks.
    External,
    /// Private raw/fixture admissions retain their fixed-owner unsafe contract.
    /// This mode is not a fallback for native or externally owned membership.
    #[cfg(test)]
    Fixed(PhysicalOrigin),
}

pub(crate) struct SourceBinding {
    provider: SourceProvider,
    cached_route: Option<usize>,
}

impl SourceBinding {
    pub(crate) fn new(provider: SourceProvider) -> Self {
        Self {
            provider,
            cached_route: None,
        }
    }

    /// Ordinary member lookup never calls this. Only source-dependent setup,
    /// preparation/configuration, wait and resubmission boundaries resolve.
    /// # Safety
    /// Native receiver home is the live pinned retained carrier. External input
    /// is a fresh live typed projection, never raw bits or cached stale metadata.
    /// Fixed sources obey their explicit complete-owner loan. Frame is call-only.
    pub(crate) unsafe fn resolve(
        &self,
        frame: *const DrStackFrameV2,
        external: Option<*const c_void>,
    ) -> Result<ResolvedSource, AdapterTerminal> {
        match self.provider {
            SourceProvider::Native {
                receiver_home,
                resolve,
            } => {
                if receiver_home.is_null() || external.is_some() {
                    return Err(AdapterTerminal::PlatformFailure(Failure::invalid_input()));
                }
                let mut source = ptr::null();
                let mut context = DrAdapterContextV1 {
                    object: ptr::null_mut(),
                    descriptor: ptr::null(),
                };
                let mut error = crate::empty_error_carrier();
                match resolve(frame, receiver_home, &mut source, &mut context, &mut error) {
                    0 => {
                        if !error.object.is_null() || !error.vtable.is_null() {
                            crate::runtime_invariant_panic();
                        }
                        if !context.valid() {
                            return Err(AdapterTerminal::PlatformFailure(Failure::invalid_input()));
                        }
                        ResolvedSource::borrow(source, Some(context))
                            .map_err(AdapterTerminal::PlatformFailure)
                    }
                    1 => Err(AdapterTerminal::CheckedError {
                        error: OwnedIoError::from_carrier(error),
                        progress: None,
                    }),
                    _ => crate::runtime_invariant_panic(),
                }
            }
            SourceProvider::External => {
                let source = external
                    .ok_or_else(|| AdapterTerminal::PlatformFailure(Failure::invalid_input()))?;
                ResolvedSource::borrow(source, None).map_err(AdapterTerminal::PlatformFailure)
            }
            #[cfg(test)]
            SourceProvider::Fixed(origin) => {
                if external.is_some() {
                    return Err(AdapterTerminal::PlatformFailure(Failure::invalid_input()));
                }
                ResolvedSource::from_origin(origin, None).map_err(AdapterTerminal::PlatformFailure)
            }
        }
    }
}

/// A just-resolved source retains its resource metadata across fallible route
/// staging. It is not yet a native-operation use pin and cannot revive an old
/// owner's resource after replacement/destruction.
pub(crate) struct ResolvedSource {
    origin: PhysicalOrigin,
    context: Option<DrAdapterContextV1>,
    metadata: Option<ResourcePin>,
}

impl ResolvedSource {
    pub(crate) unsafe fn borrow(
        source: *const c_void,
        context: Option<DrAdapterContextV1>,
    ) -> Result<Self, Failure> {
        Self::from_origin(ReadinessProjection::borrow(source)?.origin, context)
    }

    unsafe fn from_origin(
        origin: PhysicalOrigin,
        context: Option<DrAdapterContextV1>,
    ) -> Result<Self, Failure> {
        // Check liveness without interpreting a freed wrapper/old inline file.
        // Public resource metadata stays alive through the explicit pin below.
        origin.handle()?;
        Ok(Self {
            origin,
            context,
            metadata: origin.metadata_pin(),
        })
    }

    pub(crate) fn origin(&self) -> PhysicalOrigin {
        self.origin
    }
    #[cfg(test)]
    pub(crate) fn context(&self) -> Option<DrAdapterContextV1> {
        self.context
    }
}

struct SourceRoute<T> {
    origin: PhysicalOrigin,
    handle: NativeHandle,
    resource: TransferResource,
    platform: T,
    bindings: usize,
    selected: usize,
    // Platform retirement/drop precedes release of the origin metadata.
    _metadata: Option<ResourcePin>,
}

/// The just-resolved route and ordinary dispatch context travel together into
/// selection. A cached physical route never supplies an old callback context.
pub(crate) struct ResolvedRoute {
    index: usize,
    origin: PhysicalOrigin,
    handle: NativeHandle,
    context: Option<DrAdapterContextV1>,
}

impl ResolvedRoute {
    #[cfg(test)]
    pub(crate) fn index(&self) -> usize {
        self.index
    }
    #[cfg(test)]
    pub(crate) fn context(&self) -> Option<DrAdapterContextV1> {
        self.context
    }

    pub(crate) fn snapshot(&self) -> SourceSnapshot {
        SourceSnapshot {
            origin: self.origin,
            context: self.context,
        }
    }
}

/// Borrowed facts for the adapter boundary. The native driver, not this Copy
/// projection, retains the metadata or active-use pin that makes them live.
#[derive(Clone, Copy)]
pub(crate) struct SourceSnapshot {
    pub(crate) origin: PhysicalOrigin,
    pub(crate) context: Option<DrAdapterContextV1>,
}

pub(crate) struct SourceRoutes<T> {
    homes: StableSlots<Option<SourceRoute<T>>>,
}

impl<T> SourceRoutes<T> {
    pub(crate) fn new() -> Result<Self, Failure> {
        Ok(Self {
            homes: StableSlots::new()?,
        })
    }

    /// Reserve configuration capacity before any user callback/native submit.
    /// Vacant route homes remain allocated when routes retire; activating one
    /// during a current-source replacement performs no runtime allocation.
    pub(crate) fn reserve(&mut self, capacity: usize) -> Result<(), Failure> {
        for index in self.homes.len()..capacity {
            self.homes.insert_at(index, None)?;
        }
        Ok(())
    }

    /// # Safety
    /// Binding belongs to this pool. Serialized owner/group access; no concurrent route mutation or selected
    /// retirement. All homes were reserved for current bindings, setup snapshots,
    /// selected operations, plus one replacement being staged. Factory creates
    /// only platform route metadata and cannot submit bytes or close the source.
    /// T contains no kernel-/helper-written request or buffer storage; those
    /// remain in the existing, separately pinned native request slots.
    pub(crate) unsafe fn rebind(
        &mut self,
        binding: &mut SourceBinding,
        resolved: ResolvedSource,
        make: impl FnOnce(usize, PhysicalOrigin, NativeHandle, TransferResource) -> Result<T, Failure>,
    ) -> Result<ResolvedRoute, Failure> {
        let handle = resolved.origin.handle()?;
        if let Some(index) = binding.cached_route {
            let route = self.route(index)?;
            if route.origin.same_source(resolved.origin) && route.handle == handle {
                return Ok(ResolvedRoute {
                    index,
                    origin: resolved.origin,
                    handle,
                    context: resolved.context,
                });
            }
        }
        // Reuse a still-live physical source route, including an older selected
        // route to which the member returns. No registration-wide mutable route.
        let existing = (0..self.homes.len()).find(|index| {
            (*self.homes.raw(*index).expect("reserved route home"))
                .as_ref()
                .is_some_and(|route| {
                    route.origin.same_source(resolved.origin) && route.handle == handle
                })
        });
        let index = if let Some(index) = existing {
            let route = self.route_mut(index)?;
            route.bindings = route.bindings.checked_add(1).ok_or_else(exhausted)?;
            index
        } else {
            let index = (0..self.homes.len())
                .find(|index| (*self.homes.raw(*index).expect("reserved route home")).is_none())
                .ok_or_else(exhausted)?;
            let resource = TransferResource {
                identity: handle as usize,
                shared_cursor: resolved.origin.is_regular()?,
            };
            let platform = make(index, resolved.origin, handle, resource)?;
            self.homes
                .raw(index)
                .expect("reserved route home")
                .write(Some(SourceRoute {
                    origin: resolved.origin,
                    handle,
                    resource,
                    platform,
                    bindings: 1,
                    selected: 0,
                    _metadata: resolved.metadata,
                }));
            index
        };
        // Commit the replacement only AFTER every fallible setup succeeded.
        let old = binding.cached_route.replace(index);
        if let Some(old) = old {
            self.release_binding(old);
        }
        Ok(ResolvedRoute {
            index,
            origin: resolved.origin,
            handle,
            context: resolved.context,
        })
    }

    /// Unregister/drop never reruns the getter. Retire route metadata before
    /// releasing its source pin and, outside this module, its complete member.
    pub(crate) unsafe fn unbind(&mut self, binding: &mut SourceBinding) {
        if let Some(index) = binding.cached_route.take() {
            self.release_binding(index);
        }
    }

    unsafe fn release_binding(&mut self, index: usize) {
        let home = self.homes.raw(index).expect("live binding route");
        let route = (*home).as_mut().expect("live binding route");
        route.bindings = route.bindings.checked_sub(1).expect("route binding count");
        if route.bindings == 0 && route.selected == 0 {
            drop((*home).take());
        }
    }

    unsafe fn route(&self, index: usize) -> Result<&SourceRoute<T>, Failure> {
        (*self.homes.raw(index).ok_or_else(Failure::invalid_input)?)
            .as_ref()
            .ok_or_else(Failure::invalid_input)
    }

    unsafe fn route_mut(&mut self, index: usize) -> Result<&mut SourceRoute<T>, Failure> {
        (*self.homes.raw(index).ok_or_else(Failure::invalid_input)?)
            .as_mut()
            .ok_or_else(Failure::invalid_input)
    }

    /// # Safety
    /// Resolved route belongs to this pool/binding and selection follows its
    /// resolution without another mutation of that binding. Pool homes outlive
    /// the selected operation. Before dropping the selection,
    /// the existing native driver proves actual retirement (not just cancel),
    /// unsubscribes its route interests and completes the adapter exactly once.
    /// No helper thread touches the pool, binding or resource counters.
    pub(crate) unsafe fn select(
        &mut self,
        current: ResolvedRoute,
    ) -> Result<SelectedSource<T>, Failure> {
        let index = current.index;
        let home = self.homes.raw(index).ok_or_else(Failure::invalid_input)?;
        let route = (*home).as_mut().ok_or_else(Failure::invalid_input)?;
        if !route.origin.same_source(current.origin) || route.handle != current.handle {
            return Err(Failure::invalid_input());
        }
        let selected = route.selected.checked_add(1).ok_or_else(exhausted)?;
        let active = route.origin.selected_pin()?;
        if route.origin.handle()? != route.handle {
            return Err(Failure::invalid_input());
        }
        route.selected = selected;
        Ok(SelectedSource {
            home,
            #[cfg(test)]
            index,
            origin: route.origin,
            handle: route.handle,
            resource: route.resource,
            context: current.context,
            _active: active,
        })
    }
}

/// Captured per-operation facts. Never reread a registration's latest route to
/// submit, classify cursor conflicts, construct errors or retire this operation.
pub(crate) struct SelectedSource<T> {
    home: *mut Option<SourceRoute<T>>,
    #[cfg(test)]
    index: usize,
    origin: PhysicalOrigin,
    handle: NativeHandle,
    resource: TransferResource,
    context: Option<DrAdapterContextV1>,
    _active: Option<ResourcePin>,
}

impl<T> SelectedSource<T> {
    #[cfg(test)]
    pub(crate) fn index(&self) -> usize {
        self.index
    }
    pub(crate) fn origin(&self) -> PhysicalOrigin {
        self.origin
    }
    pub(crate) fn handle(&self) -> NativeHandle {
        self.handle
    }
    pub(crate) fn resource(&self) -> TransferResource {
        self.resource
    }
    pub(crate) fn context(&self) -> Option<DrAdapterContextV1> {
        self.context
    }

    pub(crate) fn snapshot(&self) -> SourceSnapshot {
        SourceSnapshot {
            origin: self.origin(),
            context: self.context(),
        }
    }

    /// # Safety
    /// Native route access is serialized by the existing group. Project only
    /// platform metadata; do not create aliases to kernel-owned request fields.
    pub(crate) unsafe fn platform(&self) -> *mut T {
        ptr::addr_of_mut!(
            (*self.home)
                .as_mut()
                .expect("selected source route")
                .platform
        )
    }
}

impl<T> Drop for SelectedSource<T> {
    fn drop(&mut self) {
        unsafe {
            let route = (*self.home).as_mut().expect("selected source route");
            route.selected = route
                .selected
                .checked_sub(1)
                .expect("selected source count");
            if route.selected == 0 && route.bindings == 0 {
                drop((*self.home).take());
            }
        }
        // The active pin field releases AFTER route teardown. Last active use
        // may now best-effort close an implicitly replaced File owner once.
    }
}

fn exhausted() -> Failure {
    Failure {
        reason: Reason::ResourceExhausted,
        system_code: None,
        transferred_byte_count: None,
    }
}

#[cfg(test)]
mod tests;
