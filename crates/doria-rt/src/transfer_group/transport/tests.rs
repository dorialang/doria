use super::*;

pub(super) unsafe fn assert_group_configuration_failure(status: &DrIoStatusV1) {
    let operation = doria_diagnostic_catalogue::IoMessageOperation::Configure as u8;
    let mut message = ptr::null_mut();
    let mut path = ptr::null_mut();
    let (mut system_code, mut progress, mut meta) = (-1, usize::MAX, u64::MAX);
    let before = status.failure();
    let (tag, allocations) = crate::probe_runtime_allocations(|| {
        crate::checked_io::dr_v1_checked_transfer_group_failure(
            ptr::null(),
            operation,
            status,
            &mut message,
            &mut path,
            &mut system_code,
            &mut progress,
            &mut meta,
        )
    });
    assert_eq!((tag, allocations), (1, 1));
    assert!(path.is_null());
    assert_eq!((system_code, progress), (0, 0));
    assert_eq!(
        meta,
        operation as u64 | (6 << 8) | ((Reason::InvalidInput as u64) << 16)
    );
    assert_eq!(
        slice::from_raw_parts(crate::string_bytes(message), (*message).byte_length),
        b"failed to configure transfer group: invalid input"
    );
    crate::dr_v1_string_release(message);
    assert_eq!(status.failure(), before);
}

#[test]
fn completed_region_projection_distinguishes_unavailable_empty_prefix_and_invalid_key() {
    let bytes = [11, 22, 33, 44];
    for (region, expected_tag, expected_data, expected_length) in [
        (Ok(None), 1, ptr::null(), 0),
        (Ok(Some(&bytes[..0])), 0, bytes.as_ptr(), 0),
        (Ok(Some(&bytes[1..3])), 0, bytes[1..].as_ptr(), 2),
        (Err(Failure::invalid_input()), 3, ptr::null(), 0),
    ] {
        let mut data = bytes.as_ptr();
        let mut length = usize::MAX;
        let mut status = DrIoStatusV1 {
            value: u64::MAX,
            reason: u8::MAX,
            system_code: -1,
            transferred_byte_count: usize::MAX,
            has_system_code: 1,
            has_transferred_byte_count: 1,
        };
        let (tag, allocations) = crate::probe_runtime_allocations(|| unsafe {
            project_completed_region(region, &mut data, &mut length, &mut status)
        });
        assert_eq!((tag, allocations), (expected_tag, 0));
        assert_eq!((data, length), (expected_data, expected_length));
        assert_eq!(
            (
                status.value,
                status.system_code,
                status.transferred_byte_count,
                status.has_system_code,
                status.has_transferred_byte_count
            ),
            (0, 0, 0, 0, 0)
        );
        if tag == 3 {
            assert_eq!(status.failure(), Some(Failure::invalid_input()));
        } else {
            assert_eq!(status.reason, 0);
        }
    }
}

#[test]
fn runtime_owner_allocation_uses_one_zeroed_headerless_class_payload() {
    #[repr(C)]
    struct Owner {
        first: usize,
        second: usize,
    }
    unsafe {
        let (owner, allocations) =
            crate::probe_runtime_allocations(|| allocate_owner::<Owner>(ptr::null()));
        assert_eq!(allocations, 1);
        assert_eq!(owner.addr() % mem::align_of::<Owner>(), 0);
        assert_eq!(((*owner).first, (*owner).second), (0, 0));
        owner.write(Owner {
            first: 7,
            second: 11,
        });
        assert_eq!(((*owner).first, (*owner).second), (7, 11));
        crate::dr_v1_class_free(owner.cast());
    }
}

#[test]
fn runtime_owner_allocator_preserves_source_identified_fatal_class_allocation() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "transfer_group::transport::tests::runtime_owner_allocation_panic_child",
            "--nocapture",
        ])
        .env("DORIA_RT_TEST_OWNER_CLASS_ALLOCATION", "1")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(101));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("P1601"), "{stderr}");
    assert!(stderr.contains("Class Allocation Failed"), "{stderr}");
    assert!(stderr.contains("runtime-owner.doria"), "{stderr}");
    assert!(stderr.contains("new ReadinessSet()"), "{stderr}");
}

#[test]
fn runtime_owner_allocation_panic_child() {
    if std::env::var("DORIA_RT_TEST_OWNER_CLASS_ALLOCATION").as_deref() != Ok("1") {
        return;
    }
    // Exercise the real allocator's deterministic layout-failure branch rather
    // than replacing it or requesting an unbounded amount of physical memory.
    #[repr(align(64))]
    struct OverAlignedOwner;
    let source = b"function main(): void { new ReadinessSet(); }\n";
    let path = b"runtime-owner.doria";
    let name = b"main";
    let expression = b"new ReadinessSet()";
    let start = source
        .windows(expression.len())
        .position(|bytes| bytes == expression)
        .unwrap();
    let frame = DrStackFrameV2 {
        parent: ptr::null(),
        function_name: name.as_ptr(),
        function_name_length: name.len(),
        source_path: path.as_ptr(),
        source_path_length: path.len(),
        source_text: source.as_ptr(),
        source_text_length: source.len(),
        function_span_start: 0,
        function_span_end: source.len() - 1,
        active_span_start: start,
        active_span_end: start + expression.len(),
    };
    unsafe { allocate_owner::<OverAlignedOwner>(&frame) };
    unreachable!("class allocation failure must not return a catchable result");
}

#[test]
fn key_layout_and_rejected_issuance_preserve_the_private_record_contract() {
    let (mut size, mut align) = (0, 0);
    let mut key = PlanKey {
        owner: usize::MAX,
        value: u64::MAX,
    };
    let mut status = DrIoStatusV1 {
        value: 99,
        reason: 255,
        ..DrIoStatusV1::default()
    };
    unsafe {
        plan::dr_v1_transfer_key_layout(&mut size, &mut align);
        assert_eq!(size, mem::size_of::<PlanKey>());
        assert_eq!(align, mem::align_of::<PlanKey>());
        assert_eq!(
            plan::dr_v1_transfer_key(
                ptr::null_mut(),
                usize::MAX,
                (&mut key as *mut PlanKey).cast(),
                &mut status,
            ),
            3
        );
    }
    assert_eq!(key, PlanKey { owner: 0, value: 0 });
    assert_eq!(status.value, 0);
    assert_eq!(status.reason, Reason::InvalidInput as u8);
}

#[test]
fn registration_epoch_exhaustion_never_reuses_an_identity() {
    let counter = AtomicUsize::new(usize::MAX - 1);
    assert_eq!(next_identity(&counter), Ok(usize::MAX));
    assert_eq!(next_identity(&counter), Err(exhausted()));
    assert_eq!(next_identity(&counter), Err(exhausted()));
    assert_eq!(counter.load(Ordering::Relaxed), usize::MAX);
}

#[test]
fn region_preflight_rejects_overflow_and_writable_overlap_without_forming_slices() {
    let descriptor = |address, length, reading| DrTransferBufferV1 {
        data: address as *mut u8,
        length,
        reading,
    };
    assert!(!descriptor(usize::MAX - 1, 2, 1).valid());
    assert!(!valid_descriptors(&[
        descriptor(100, 10, 1),
        descriptor(105, 10, 0)
    ]));
    assert!(!valid_descriptors(&[
        descriptor(100, 10, 0),
        descriptor(105, 10, 1)
    ]));
    assert!(!valid_descriptors(&[
        descriptor(100, 10, 1),
        descriptor(105, 10, 1)
    ]));
    assert!(valid_descriptors(&[
        descriptor(100, 10, 0),
        descriptor(105, 10, 0)
    ]));
    assert!(valid_descriptors(&[
        descriptor(100, 10, 1),
        descriptor(110, 10, 1)
    ]));
    assert!(valid_descriptors(&[
        descriptor(0, 0, 1),
        descriptor(0, 0, 1)
    ]));
}
