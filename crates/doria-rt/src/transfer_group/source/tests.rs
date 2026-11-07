use super::*;
use crate::file_io::stream::transport::object::{
    dr_v1_file_object_drop, dr_v1_file_object_layout, dr_v1_file_object_open,
    dr_v1_file_object_readiness_source,
};
use crate::io_status::DrIoStatusV1;
use crate::native_handle::with_close_probe;
use crate::transfer_group::adapter::test_object::TestObject;
use crate::{DrInterfaceVtableV1, DrStringV1};
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Real public File storage: tests must not hide wrapper replacement behind an
/// independently retained raw OwnedFile fixture.
struct FileFixture {
    directory: PathBuf,
    filename: PathBuf,
    path: *mut DrStringV1,
    object: *mut c_void,
}

impl FileFixture {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "doria-current-source-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir(&directory).unwrap();
        let filename = directory.join(format!("{name}-Dória.bin"));
        std::fs::write(&filename, name.as_bytes()).unwrap();
        unsafe {
            let bytes = filename.to_str().unwrap().as_bytes();
            let path = crate::dr_v1_string_from_utf8(bytes.as_ptr(), bytes.len());
            let (mut size, mut alignment) = (0, 0);
            dr_v1_file_object_layout(&mut size, &mut alignment);
            let object = crate::dr_v2_class_allocate(ptr::null(), size, alignment).cast();
            assert_eq!(
                dr_v1_file_object_open(path, 2, 0, 0, object, &mut DrIoStatusV1::default()),
                0
            );
            Self {
                directory,
                filename,
                path,
                object,
            }
        }
    }

    fn source(&self) -> *const c_void {
        assert!(!self.object.is_null());
        unsafe { dr_v1_file_object_readiness_source(self.object) }
    }

    fn destroy(&mut self) {
        let object = core::mem::replace(&mut self.object, ptr::null_mut());
        if !object.is_null() {
            unsafe {
                dr_v1_file_object_drop(ptr::null(), object);
                crate::dr_v1_class_free(object.cast());
            }
        }
    }
}

impl Drop for FileFixture {
    fn drop(&mut self) {
        self.destroy();
        unsafe { crate::dr_v1_string_release(self.path) };
        std::fs::remove_file(&self.filename).unwrap();
        std::fs::remove_dir(&self.directory).unwrap();
    }
}

struct PlatformRoute {
    origin: PhysicalOrigin,
    drops: Rc<Cell<usize>>,
    live_at_drop: Rc<Cell<Option<bool>>>,
}

impl Drop for PlatformRoute {
    fn drop(&mut self) {
        // Resource metadata must outlive route teardown even when the wrapper
        // is gone. A last selected use must still keep its native handle open.
        unsafe {
            let path = self.origin.file().unwrap().1.path;
            assert!((*path).references >= 2);
            self.live_at_drop.set(Some(self.origin.handle().is_ok()));
        }
        self.drops.set(self.drops.get() + 1);
    }
}

fn make_route(
    origin: PhysicalOrigin,
    drops: &Rc<Cell<usize>>,
    live_at_drop: &Rc<Cell<Option<bool>>>,
) -> PlatformRoute {
    PlatformRoute {
        origin,
        drops: Rc::clone(drops),
        live_at_drop: Rc::clone(live_at_drop),
    }
}

unsafe fn bind(
    routes: &mut SourceRoutes<PlatformRoute>,
    binding: &mut SourceBinding,
    source: *const c_void,
    drops: &Rc<Cell<usize>>,
    live_at_drop: &Rc<Cell<Option<bool>>>,
) -> ResolvedRoute {
    let resolved = binding.resolve(ptr::null(), Some(source)).unwrap();
    routes
        .rebind(binding, resolved, |_, origin, _, _| {
            Ok(make_route(origin, drops, live_at_drop))
        })
        .unwrap()
}

#[test]
fn external_binding_requires_a_fresh_typed_source_even_after_successful_admission() {
    let file = FileFixture::new("fresh");
    let binding = SourceBinding::new(SourceProvider::External);
    unsafe {
        let first = binding.resolve(ptr::null(), Some(file.source())).unwrap();
        assert_eq!(first.origin().file().unwrap().1.payload, file.object);
        assert!(first.context().is_none());
        assert!(
            matches!(binding.resolve(ptr::null(), None), Err(AdapterTerminal::PlatformFailure(f)) if f.reason == Reason::InvalidInput)
        );
        assert!(
            matches!(binding.resolve(ptr::null(), Some(ptr::null())), Err(AdapterTerminal::PlatformFailure(f)) if f.reason == Reason::InvalidInput)
        );
    }
}

#[test]
fn replacement_keeps_old_selected_handle_and_path_until_actual_retirement() {
    let mut first = FileFixture::new("first");
    let second = FileFixture::new("second");
    let drops = Rc::new(Cell::new(0));
    let live = Rc::new(Cell::new(None));
    let mut routes = SourceRoutes::new().unwrap();
    routes.reserve(3).unwrap();
    let mut binding = SourceBinding::new(SourceProvider::External);
    unsafe {
        let route = bind(&mut routes, &mut binding, first.source(), &drops, &live);
        let selected = routes.select(route).unwrap();
        let old_index = selected.index();
        let old_handle = selected.handle();
        assert!(selected.resource().shared_cursor);
        assert_eq!(selected.resource().identity, old_handle as usize);
        assert_eq!(
            (*selected.platform()).origin.file().unwrap().1.path,
            first.path
        );
        let ((), closes) = with_close_probe(|| first.destroy());
        assert_eq!(
            closes, 0,
            "wrapper replacement must not close selected native work"
        );
        let ((current, replacement), allocations) = crate::probe_runtime_allocations(|| {
            let current = bind(&mut routes, &mut binding, second.source(), &drops, &live);
            let index = current.index();
            (index, routes.select(current).unwrap())
        });
        assert_eq!(allocations, 0);
        assert_ne!(current, old_index);
        assert_eq!(replacement.origin().file().unwrap().1.path, second.path);
        assert_ne!(replacement.handle(), old_handle);
        assert_eq!(selected.handle(), old_handle);
        assert_eq!(selected.origin().file().unwrap().1.path, first.path);
        assert_eq!(selected.origin().handle().unwrap(), old_handle);
        assert_eq!(drops.get(), 0);
        // This point models terminal native retirement plus adapter completion,
        // not the earlier cancellation request.
        let ((), closes) = with_close_probe(|| drop(selected));
        assert_eq!(closes, 1);
        assert_eq!(drops.get(), 1);
        assert_eq!(
            live.get(),
            Some(true),
            "platform route drops before last active pin"
        );
        assert_eq!((*first.path).references, 1);
        drop(replacement);
        routes.unbind(&mut binding);
        assert_eq!(drops.get(), 2);
    }
}

#[test]
fn idle_route_metadata_preserves_path_but_does_not_keep_unused_old_handle_open() {
    let mut first = FileFixture::new("idle-first");
    let second = FileFixture::new("idle-second");
    let drops = Rc::new(Cell::new(0));
    let live = Rc::new(Cell::new(None));
    let mut routes = SourceRoutes::new().unwrap();
    routes.reserve(2).unwrap();
    let mut binding = SourceBinding::new(SourceProvider::External);
    unsafe {
        let old = bind(&mut routes, &mut binding, first.source(), &drops, &live);
        let ((), closes) = with_close_probe(|| first.destroy());
        assert_eq!(closes, 1);
        assert_eq!((*first.path).references, 2);
        assert!(matches!(routes.select(old), Err(f) if f.reason == Reason::Closed));
        let (new, allocations) = crate::probe_runtime_allocations(|| {
            bind(&mut routes, &mut binding, second.source(), &drops, &live)
        });
        assert_eq!(allocations, 0);
        assert_eq!(drops.get(), 1);
        assert_eq!(live.get(), Some(false));
        assert_eq!((*first.path).references, 1);
        drop(routes.select(new).unwrap());
        routes.unbind(&mut binding);
    }
}

#[test]
fn unchanged_source_reuses_platform_route_but_selects_fresh_dispatch_context() {
    let file = FileFixture::new("same");
    let drops = Rc::new(Cell::new(0));
    let live = Rc::new(Cell::new(None));
    let mut routes = SourceRoutes::new().unwrap();
    routes.reserve(1).unwrap();
    let mut binding = SourceBinding::new(SourceProvider::External);
    let mut contexts = [0u8; 2];
    unsafe {
        let resolved = ResolvedSource::borrow(
            file.source(),
            Some(DrAdapterContextV1 {
                object: contexts.as_mut_ptr().cast(),
                descriptor: ptr::null(),
            }),
        )
        .unwrap();
        let first = routes
            .rebind(&mut binding, resolved, |_, origin, _, _| {
                Ok(make_route(origin, &drops, &live))
            })
            .unwrap();
        let selected = routes.select(first).unwrap();
        let old_home = selected.platform();
        let (replacement, allocations) = crate::probe_runtime_allocations(|| {
            let resolved = ResolvedSource::borrow(
                file.source(),
                Some(DrAdapterContextV1 {
                    object: contexts.as_mut_ptr().add(1).cast(),
                    descriptor: ptr::null(),
                }),
            )
            .unwrap();
            let current = routes
                .rebind(
                    &mut binding,
                    resolved,
                    |_, _, _, _| -> Result<PlatformRoute, Failure> {
                        panic!("unchanged source recreated route")
                    },
                )
                .unwrap();
            assert_eq!(
                current.context().unwrap().object,
                contexts.as_mut_ptr().add(1).cast()
            );
            routes.select(current).unwrap()
        });
        assert_eq!(allocations, 0);
        assert_eq!(replacement.platform(), old_home);
        assert_ne!(
            replacement.context().unwrap().object,
            selected.context().unwrap().object
        );
        assert_eq!(drops.get(), 0);
        routes.unbind(&mut binding);
        drop(selected);
        assert_eq!(drops.get(), 0);
        drop(replacement);
        assert_eq!(drops.get(), 1);
    }
}

#[test]
fn shared_source_route_and_reserved_vacant_home_are_reused_without_allocation() {
    let first = FileFixture::new("route-first");
    let second = FileFixture::new("route-second");
    let drops = Rc::new(Cell::new(0));
    let live = Rc::new(Cell::new(None));
    let mut routes = SourceRoutes::new().unwrap();
    routes.reserve(2).unwrap();
    let mut a = SourceBinding::new(SourceProvider::External);
    let mut b = SourceBinding::new(SourceProvider::External);
    unsafe {
        let route = bind(&mut routes, &mut a, first.source(), &drops, &live);
        let selected = routes.select(route).unwrap();
        let initial_home = selected.platform();
        let initial_index = selected.index();
        let ((other, next), allocations) = crate::probe_runtime_allocations(|| {
            let shared = bind(&mut routes, &mut b, first.source(), &drops, &live);
            assert_eq!(shared.index(), initial_index);
            let other = routes.select(shared).unwrap();
            let next = bind(&mut routes, &mut a, second.source(), &drops, &live);
            (other, routes.select(next).unwrap())
        });
        assert_eq!(allocations, 0);
        assert_eq!(other.platform(), initial_home);
        routes.unbind(&mut b);
        drop(other);
        drop(selected);
        assert_eq!(drops.get(), 1);
        let (returning, allocations) = crate::probe_runtime_allocations(|| {
            bind(&mut routes, &mut a, first.source(), &drops, &live)
        });
        assert_eq!(allocations, 0);
        assert_eq!(returning.index(), initial_index);
        let returning = routes.select(returning).unwrap();
        assert_eq!(returning.platform(), initial_home);
        drop(next);
        drop(returning);
        routes.unbind(&mut a);
        assert_eq!(drops.get(), 3);
    }
}

#[test]
fn failed_route_staging_and_capacity_leave_original_binding_and_source_intact() {
    let first = FileFixture::new("stage-first");
    let mut second = FileFixture::new("stage-second");
    let drops = Rc::new(Cell::new(0));
    let live = Rc::new(Cell::new(None));
    let mut routes = SourceRoutes::new().unwrap();
    routes.reserve(1).unwrap();
    let mut binding = SourceBinding::new(SourceProvider::External);
    unsafe {
        let old = bind(&mut routes, &mut binding, first.source(), &drops, &live);
        let index = old.index();
        let resolved = binding.resolve(ptr::null(), Some(second.source())).unwrap();
        assert!(
            matches!(routes.rebind(&mut binding, resolved, |_, _, _, _| -> Result<PlatformRoute, Failure> { panic!("capacity failure invoked factory") }), Err(f) if f.reason == Reason::ResourceExhausted)
        );
        assert_eq!(binding.cached_route, Some(index));
        assert_eq!(
            (*second.path).references,
            2,
            "failed resolution pin does not retain path separately"
        );
        routes.reserve(2).unwrap();
        let resolved = binding.resolve(ptr::null(), Some(second.source())).unwrap();
        assert!(
            matches!(routes.rebind(&mut binding, resolved, |_, _, _, _| Err(Failure::unsupported())), Err(f) if f.reason == Reason::Unsupported)
        );
        assert_eq!(binding.cached_route, Some(index));
        assert_eq!(drops.get(), 0);
        let selected = routes.select(old).unwrap();
        assert_eq!(selected.origin().file().unwrap().1.path, first.path);
        drop(selected);
        routes.unbind(&mut binding);
        let ((), closes) = with_close_probe(|| second.destroy());
        assert_eq!(closes, 1);
        assert_eq!(
            (*second.path).references,
            1,
            "failed staging leaked resource metadata"
        );
    }
}

struct Getter {
    source: *const c_void,
    expected_home: *const DrInterfaceCarrierV1,
    expected_frame: *const DrStackFrameV2,
    calls: usize,
    error: Option<OwnedIoError>,
}

unsafe extern "C" fn resolve(
    frame: *const DrStackFrameV2,
    receiver: *const DrInterfaceCarrierV1,
    source: *mut *const c_void,
    context: *mut DrAdapterContextV1,
    error: *mut DrInterfaceCarrierV1,
) -> u8 {
    let getter = TestObject::<Getter>::in_callback((*receiver).object);
    assert_eq!(receiver, getter.expected_home);
    assert_eq!(frame, getter.expected_frame);
    getter.calls += 1;
    if let Some(owned) = getter.error.take() {
        error.write(owned.into_carrier());
        return 1;
    }
    source.write(getter.source);
    context.write(DrAdapterContextV1 {
        object: (*receiver).object.cast(),
        descriptor: (*receiver).vtable.cast(),
    });
    0
}

unsafe extern "C" fn drop_error(_: *const DrStackFrameV2, object: *mut u8) {
    let counter = Box::from_raw(object.cast::<Rc<Cell<usize>>>());
    counter.set(counter.get() + 1);
}

fn descriptor() -> DrInterfaceVtableV1 {
    DrInterfaceVtableV1 {
        type_name: b"SourceTestError".as_ptr(),
        type_name_length: 15,
        message_offset: 0,
        drop: drop_error,
        class_size: core::mem::size_of::<Rc<Cell<usize>>>(),
        origin_offset: 0,
        assertion_offsets: 0,
        assertion_magic: 0,
        class_descriptor: ptr::null(),
        interface_id: 0,
        io_transferred_byte_count_offset: 0,
        io_progress_magic: 0,
    }
}

#[test]
fn native_resolution_uses_original_receiver_home_fresh_getter_and_original_error_carrier() {
    let first = FileFixture::new("getter-first");
    let second = FileFixture::new("getter-second");
    let descriptor = descriptor();
    let getter = TestObject::new(Getter {
        source: first.source(),
        expected_home: ptr::null(),
        expected_frame: ptr::null(),
        calls: 0,
        error: None,
    });
    let receiver = Box::new(DrInterfaceCarrierV1 {
        object: getter.object().cast(),
        vtable: &descriptor,
    });
    unsafe { (*getter.as_ptr()).expected_home = ptr::from_ref(&*receiver) };
    let binding = SourceBinding::new(SourceProvider::Native {
        receiver_home: unsafe { (*getter.as_ptr()).expected_home },
        resolve,
    });
    unsafe {
        assert!(
            matches!(binding.resolve(ptr::null(), Some(first.source())), Err(AdapterTerminal::PlatformFailure(f)) if f.reason == Reason::InvalidInput)
        );
        assert_eq!((*getter.as_ptr()).calls, 0);
        // All-zero frames are valid ABI records. Resolution must forward each
        // call's current home, not save the frame from the first getter.
        let frames: [DrStackFrameV2; 2] = core::mem::zeroed();
        (*getter.as_ptr()).expected_frame = ptr::from_ref(&frames[0]);
        let ((a, b), allocations) = crate::probe_runtime_allocations(|| {
            let a = binding.resolve(ptr::from_ref(&frames[0]), None).unwrap();
            (*getter.as_ptr()).source = second.source();
            (*getter.as_ptr()).expected_frame = ptr::from_ref(&frames[1]);
            let b = binding.resolve(ptr::from_ref(&frames[1]), None).unwrap();
            (a, b)
        });
        assert_eq!(allocations, 0);
        assert_eq!(a.origin().file().unwrap().1.path, first.path);
        assert_eq!(b.origin().file().unwrap().1.path, second.path);
        assert_eq!(b.context().unwrap().object, receiver.object.cast());
        assert_eq!(b.context().unwrap().descriptor, receiver.vtable.cast());
        let drops = Rc::new(Cell::new(0));
        let object = Box::into_raw(Box::new(Rc::clone(&drops))).cast();
        (*getter.as_ptr()).error = Some(OwnedIoError::from_carrier(DrInterfaceCarrierV1 {
            object,
            vtable: &descriptor,
        }));
        (*getter.as_ptr()).expected_frame = ptr::null();
        let terminal = match binding.resolve(ptr::null(), None) {
            Err(terminal) => terminal,
            Ok(_) => panic!("getter error was lost"),
        };
        let AdapterTerminal::CheckedError { error, progress } = terminal else {
            panic!("getter error was converted")
        };
        assert_eq!(error.carrier().object, object);
        assert!(ptr::eq(error.carrier().vtable, &descriptor));
        assert_eq!(progress, None);
        assert_eq!(drops.get(), 0);
        drop(error);
        assert_eq!(drops.get(), 1);
        assert_eq!((*getter.as_ptr()).calls, 3);
    }
}

#[test]
fn unbind_never_reruns_getter_and_fixed_mode_rejects_external_substitution() {
    let file = FileFixture::new("unbind");
    let descriptor = descriptor();
    let getter = TestObject::new(Getter {
        source: file.source(),
        expected_home: ptr::null(),
        expected_frame: ptr::null(),
        calls: 0,
        error: None,
    });
    let receiver = Box::new(DrInterfaceCarrierV1 {
        object: getter.object().cast(),
        vtable: &descriptor,
    });
    unsafe { (*getter.as_ptr()).expected_home = ptr::from_ref(&*receiver) };
    let mut binding = SourceBinding::new(SourceProvider::Native {
        receiver_home: unsafe { (*getter.as_ptr()).expected_home },
        resolve,
    });
    let mut routes = SourceRoutes::new().unwrap();
    routes.reserve(1).unwrap();
    unsafe {
        let resolved = binding.resolve(ptr::null(), None).unwrap();
        let origin = resolved.origin();
        let route = routes
            .rebind(&mut binding, resolved, |_, _, _, _| Ok(()))
            .unwrap();
        drop(routes.select(route).unwrap());
        assert_eq!((*getter.as_ptr()).calls, 1);
        (*getter.as_ptr()).source = ptr::null();
        routes.unbind(&mut binding);
        routes.unbind(&mut binding);
        assert_eq!((*getter.as_ptr()).calls, 1);
        let fixed = SourceBinding::new(SourceProvider::Fixed(origin));
        assert!(fixed.resolve(ptr::null(), None).is_ok());
        assert!(
            matches!(fixed.resolve(ptr::null(), Some(file.source())), Err(AdapterTerminal::PlatformFailure(f)) if f.reason == Reason::InvalidInput)
        );
    }
}
