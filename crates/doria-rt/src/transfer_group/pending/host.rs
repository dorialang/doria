//! Ordinary host calls pause the existing pending driver. Only stable record
//! facts and move-only replies survive a handoff, never native/PHP callbacks.

use super::*;

pub(super) enum Exchange {
    Source {
        setup: bool,
        reply: Option<Result<*const c_void, AdapterTerminal>>,
    },
    Prepare(Option<Prepared>),
    Complete {
        validation: Option<Failure>,
        reply: Option<AdapterTerminal>,
    },
}

/// # Safety
/// View is current initialized opaque request storage; record is a live group
/// record. This only checks identity before lending the driver for configuration.
pub(crate) unsafe fn matches_view(record: *mut c_void, view: *const c_void) -> bool {
    !view.is_null()
        && (*view.cast::<DrPendingRequestViewV1>())
            .record
            .cast::<c_void>()
            == record
}

impl<D: PendingNativeDriver, R: PendingRecords> PendingDriver<'_, D, R> {
    pub(crate) fn awaiting_host_call(&self, index: usize) -> Option<*mut c_void> {
        let record = self.record(index);
        unsafe {
            matches!(
                (*record).host,
                Some(Exchange::Source { reply: None, .. })
                    | Some(Exchange::Prepare(None))
                    | Some(Exchange::Complete { reply: None, .. })
            )
            .then_some(record.cast())
        }
    }

    /// # Safety
    /// Record/request belong to this outstanding preparation. The supplied
    /// operation cannot reenter, move the driver or retain its configurator.
    pub(crate) unsafe fn with_host_configurator<T>(
        &mut self,
        index: usize,
        request: *mut D::Request,
        operation: impl FnOnce() -> T,
    ) -> T {
        let record = self.record(index);
        if !matches!((*record).host, Some(Exchange::Prepare(None)))
            || (*record).configurator.is_some()
        {
            crate::runtime_invariant_panic();
        }
        (*record).configurator = Some(SourceConfigurator {
            driver: ptr::from_mut(&mut self.native).cast(),
            index,
            request: request.cast(),
            resolve: configure_source::<D>,
        });
        let call = ConfigurationCall(record);
        let result = operation();
        drop(call);
        result
    }

    unsafe fn host_source(
        &mut self,
        index: usize,
        setup: bool,
    ) -> Poll<Result<*const c_void, AdapterTerminal>> {
        let record = &mut *self.record(index);
        match &mut record.host {
            None => {
                record.host = Some(Exchange::Source { setup, reply: None });
                Poll::Pending
            }
            Some(Exchange::Source {
                setup: previous,
                reply,
            }) if *previous == setup => match reply.take() {
                Some(reply) => {
                    record.host = None;
                    Poll::Ready(reply)
                }
                None => Poll::Pending,
            },
            _ => crate::runtime_invariant_panic(),
        }
    }

    pub(super) unsafe fn host_setup(
        &mut self,
        index: usize,
        request: *mut D::Request,
    ) -> Poll<Result<(), AdapterTerminal>> {
        let source = match self.host_source(index, true) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
            Poll::Ready(Ok(source)) => source,
        };
        Poll::Ready(
            self.native
                .setup_source(index, request, self.frame, Some(source))
                .map(|source| {
                    if let Some(source) = source {
                        if source.context.is_some() {
                            crate::runtime_invariant_panic();
                        }
                        (*self.record(index)).origin = Some(source.origin);
                    }
                }),
        )
    }

    pub(super) unsafe fn host_prepare(
        &mut self,
        index: usize,
        request: *mut D::Request,
        buffer: &mut Buffer<'_>,
    ) -> Poll<Result<(), AdapterTerminal>> {
        let record = self.record(index);
        if matches!(
            (*record).host,
            None | Some(Exchange::Source { setup: false, .. })
        ) {
            let source = match self.host_source(index, false) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => {
                    (*record).phase = Phase::Taken;
                    return Poll::Ready(Err(error));
                }
                Poll::Ready(Ok(source)) => source,
            };
            if let Err(error) = self.begin_preparation_source(index, request, buffer, Some(source))
            {
                return Poll::Ready(Err(error));
            }
            (*record).host = Some(Exchange::Prepare(None));
            return Poll::Pending;
        }
        if let Some(Exchange::Prepare(reply)) = &mut (*record).host {
            let Some(prepared) = reply.take() else {
                return Poll::Pending;
            };
            (*record).host = None;
            match self.accept_preparation(index, request, prepared) {
                Preparation::Ready => return Poll::Ready(Ok(())),
                Preparation::Failed(error) => return Poll::Ready(Err(error)),
                Preparation::Completion(failure) => {
                    return self
                        .host_complete(index, request, Some(failure))
                        .map(|()| Err(self.preparation_failure(index, failure)));
                }
            }
        }
        let Some(Exchange::Complete {
            validation: Some(failure),
            ..
        }) = &(*record).host
        else {
            crate::runtime_invariant_panic();
        };
        let failure = *failure;
        self.host_complete(index, request, Some(failure))
            .map(|()| Err(self.preparation_failure(index, failure)))
    }

    pub(super) unsafe fn host_complete(
        &mut self,
        index: usize,
        request: *mut D::Request,
        validation: Option<Failure>,
    ) -> Poll<()> {
        let record = &mut *self.record(index);
        if matches!(record.phase, Phase::Ready(_)) {
            return Poll::Ready(());
        }
        if !matches!(record.phase, Phase::Finalizing(_)) {
            crate::runtime_invariant_panic();
        }
        match &mut record.host {
            None => {
                record.host = Some(Exchange::Complete {
                    validation,
                    reply: None,
                });
                Poll::Pending
            }
            Some(Exchange::Complete {
                validation: previous,
                reply,
            }) if *previous == validation => {
                let Some(terminal) = reply.take() else {
                    return Poll::Pending;
                };
                record.host = None;
                self.accept_completion(index, request, terminal);
                Poll::Ready(())
            }
            _ => crate::runtime_invariant_panic(),
        }
    }

    pub(super) unsafe fn host_abort(&mut self, index: usize, request: *mut D::Request) {
        let record = &mut *self.record(index);
        match record.host.take() {
            Some(Exchange::Source {
                setup: false,
                reply,
            }) => {
                // A returned getter Error may be disposed during abandoned
                // setup, but no successful adapter preparation is discarded.
                drop(reply);
            }
            Some(Exchange::Prepare(None)) => {}
            _ => crate::runtime_invariant_panic(),
        }
        record.caller_data = ptr::null_mut();
        record.configuration = None;
        record.phase = Phase::Taken;
        self.native.release_source(index, request);
    }
}

/// # Safety
/// Outputs are disjoint writable usize homes. A view is opaque to the host and
/// is borrowed only while its matching ordinary adapter call is outstanding.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_pending_view_layout(size: *mut usize, alignment: *mut usize) {
    *size = mem::size_of::<DrPendingRequestViewV1>();
    *alignment = mem::align_of::<DrPendingRequestViewV1>();
}

/// Returns 0 for no host call, 1 setup source, 2 preparation source,
/// 3 preparation, or 4 completion. An already-staged reply is not another call.
/// # Safety
/// Record is this live group's pinned pending record. View is aligned EMPTY
/// opaque storage; all outputs are disjoint. No host call may reenter the group.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_pending_action(
    record: *mut c_void,
    view: *mut c_void,
    caller: *mut *mut u8,
    length: *mut usize,
    reading: *mut u8,
) -> u8 {
    let record = record.cast::<PendingState>();
    *caller = ptr::null_mut();
    *length = 0;
    *reading = 0;
    match &(*record).host {
        Some(Exchange::Source { setup, reply: None }) => {
            if *setup {
                1
            } else {
                2
            }
        }
        Some(Exchange::Prepare(None)) | Some(Exchange::Complete { reply: None, .. }) => {
            *caller = (*record).caller_data;
            *length = (*record).bound;
            *reading = u8::from((*record).reading);
            if matches!((*record).host, Some(Exchange::Prepare(_))) {
                view.cast::<DrPendingRequestViewV1>()
                    .write(DrPendingRequestViewV1::preparation(record));
                3
            } else {
                view.cast::<DrPendingRequestViewV1>()
                    .write(DrPendingRequestViewV1::completion(record));
                4
            }
        }
        _ => 0,
    }
}

unsafe fn host_error(terminal: *mut c_void) -> AdapterTerminal {
    let terminal = terminal.cast::<AdapterTerminal>().read();
    if !matches!(terminal, AdapterTerminal::HostError { .. }) {
        crate::runtime_invariant_panic();
    }
    terminal
}

/// Stages one ordinary getter reply; rejection leaves the Error claim unmoved.
/// # Safety
/// Record/source belong to the outstanding host call. A returned source and
/// its owner stay live through the resumed selection. Error is null or one
/// initialized, uniquely owned hosted Error terminal, disjoint from the record.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_pending_source_reply(
    record: *mut c_void,
    source: *const c_void,
    error: *mut c_void,
) -> u8 {
    let record = &mut *record.cast::<PendingState>();
    let Some(Exchange::Source { reply, .. }) = &mut record.host else {
        return 1;
    };
    if reply.is_some() {
        return 1;
    }
    *reply = Some(if error.is_null() {
        Ok(source)
    } else {
        Err(host_error(error))
    });
    0
}

/// Stages one preparation return. Normal return uses the same configured
/// request decoder as native dispatch; throwing preparation owes no completion.
/// # Safety
/// View is the outstanding preparation view, no longer used after this call.
/// Error is null or a uniquely owned hosted terminal, as above.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_pending_prepare_reply(
    view: *mut c_void,
    error: *mut c_void,
) -> u8 {
    let view = &*view.cast::<DrPendingRequestViewV1>();
    if !matches!((*view.record).host, Some(Exchange::Prepare(None))) {
        return 1;
    }
    let prepared = if error.is_null() {
        view.prepared_reply(
            0,
            DrAdapterOutcomeV1::default(),
            crate::empty_error_carrier(),
        )
    } else {
        Prepared::Failed(host_error(error).bounded((*view.record).bound))
    };
    (*view.record).host = Some(Exchange::Prepare(Some(prepared)));
    0
}

/// Stages the ordinary completion reply after actual native retirement.
/// # Safety
/// View is the outstanding completion view, consumed by this reply. Error is
/// null or one uniquely owned hosted terminal. Outcome is the projected actual
/// host result, not an invented native Error carrier or callback frame.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_pending_complete_reply(
    view: *mut c_void,
    tag: u8,
    count: usize,
    error: *mut c_void,
) -> u8 {
    let view = &*view.cast::<DrPendingRequestViewV1>();
    let Some(Exchange::Complete {
        validation,
        reply: None,
    }) = (*view.record).host
    else {
        return 1;
    };
    let terminal = if error.is_null() {
        view.completed_reply(
            if tag == 4 { 4 } else { 0 },
            DrAdapterOutcomeV1 {
                count,
                tag,
                has_count: u8::from(tag == 0),
            },
            crate::empty_error_carrier(),
        )
    } else {
        host_error(error).bounded((*view.record).bound)
    };
    (*view.record).host = Some(Exchange::Complete {
        validation,
        reply: Some(terminal),
    });
    0
}
