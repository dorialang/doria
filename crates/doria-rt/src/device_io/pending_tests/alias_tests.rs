//! Real aliased inherited stdout/stderr, with independent live transfer groups.
use super::*;
use crate::transfer_group::adapter::test_object::TestObject;
use crate::transfer_group::adapter::DrAdapterContextV1;
use crate::transfer_group::TerminalOutcome;

struct Writer {
    stream: u8,
    driver: std::thread::ThreadId,
    completions: usize,
}

impl Writer {
    fn new(stream: u8) -> TestObject<Self> {
        TestObject::new(Self {
            stream,
            driver: std::thread::current().id(),
            completions: 0,
        })
    }
}

impl TestObject<Writer> {
    fn adapter(&self) -> DrPendingTransferAdapterV1 {
        DrPendingTransferAdapterV1 {
            context: self.context(),
            prepare_read: None,
            complete_read: None,
            prepare_write: Some(prepare_write),
            complete_write: Some(complete_write),
        }
    }
}

unsafe extern "C" fn prepare_write(
    _: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    request: *mut c_void,
    data: *const u8,
    length: usize,
    _: *mut DrAdapterOutcomeV1,
    _: *mut DrInterfaceCarrierV1,
) -> u8 {
    let writer = TestObject::<Writer>::in_callback((*context).object);
    assert_eq!(std::thread::current().id(), writer.driver);
    dr_v1_pending_request_configure_standard(request, writer.stream, data.cast_mut(), length);
    0
}

unsafe extern "C" fn complete_write(
    _: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    request: *mut c_void,
    data: *const u8,
    length: usize,
    outcome: *mut DrAdapterOutcomeV1,
    _: *mut DrInterfaceCarrierV1,
) -> u8 {
    let writer = TestObject::<Writer>::in_callback((*context).object);
    assert_eq!(std::thread::current().id(), writer.driver);
    writer.completions += 1;
    let (tag, submitted, native, region, _) = pending_callback_facts(request);
    assert_eq!(submitted, 1);
    assert_eq!(
        (*region).data,
        data.cast_mut(),
        "original retained source, no copy"
    );
    assert_eq!((*region).length, length);
    assert_eq!(tag, 0);
    *outcome = DrAdapterOutcomeV1 {
        tag: 0,
        count: (*native).value as usize,
        has_count: 1,
    };
    0
}

fn selection(index: usize) -> FixedSlice<usize> {
    let mut selected = FixedSlice::new(1).unwrap();
    selected.push(index).unwrap();
    selected
}

#[test]
fn aliased_standard_channels_keep_modes_ordinary_output_and_independent_group_retirement() {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "device_io::pending_tests::alias_tests::inherited_alias_child",
            "--nocapture",
        ])
        .env("DORIA_RT_TEST_ALIAS_PENDING", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut control = child.stderr.take().unwrap();
    let (send, receive) = mpsc::channel();
    let control_reader = std::thread::spawn(move || {
        let mut byte = [0; 1];
        while control.read_exact(&mut byte).is_ok() {
            let _ = send.send(byte[0]);
        }
    });
    let first = receive.recv_timeout(std::time::Duration::from_secs(10));
    if first != Ok(b'R') {
        let _ = child.kill();
        let output = child.wait_with_output().unwrap();
        panic!(
            "alias fixture did not start: {first:?}; stdout={:?}",
            output.stdout
        );
    }
    let mut stdout = child.stdout.take().unwrap();
    let output_reader = std::thread::spawn(move || {
        let mut output = std::vec::Vec::new();
        stdout.read_to_end(&mut output).unwrap();
        output
    });
    let mut count = [0; 8];
    for byte in &mut count {
        match receive.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok(value) => *byte = value,
            error => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("alias retirement stalled: {error:?}");
            }
        }
    }
    let status = child.wait().unwrap();
    control_reader.join().unwrap();
    let output = output_reader.join().unwrap();
    assert!(
        status.success(),
        "alias fixture failed: {:?}",
        std::string::String::from_utf8_lossy(&output)
    );
    assert_eq!(
        output.iter().filter(|byte| **byte == b'A').count(),
        u64::from_le_bytes(count) as usize
    );
    assert!(output.contains(&b'B') && output.contains(&b'L'));
}

#[test]
fn inherited_alias_child() {
    if std::env::var("DORIA_RT_TEST_ALIAS_PENDING").as_deref() != Ok("1") {
        return;
    }
    unsafe {
        #[cfg(unix)]
        let control_owner = crate::native_handle::OwnedHandle::from_raw(libc::dup(2));
        #[cfg(unix)]
        let control = control_owner.get();
        #[cfg(unix)]
        assert_eq!(libc::dup2(1, 2), 2);
        #[cfg(windows)]
        use crate::win32::{GetStdHandle, SetStdHandle};
        #[cfg(windows)]
        let control = GetStdHandle(-12i32 as u32);
        #[cfg(windows)]
        assert_ne!(SetStdHandle(-12i32 as u32, GetStdHandle(-11i32 as u32)), 0);
        #[cfg(unix)]
        let flags = libc::fcntl(1, libc::F_GETFL);
        #[cfg(unix)]
        assert_eq!(flags & libc::O_NONBLOCK, 0);

        let out = standard_device(StandardStream::Stdout);
        let err = standard_device(StandardStream::Stderr);
        let starts = super::super::helper::starts_for_test();
        let out_writer = Writer::new(1);
        let idle_alias = Writer::new(2);
        let err_writer = Writer::new(2);
        let mut out_set = PendingSet::new(ptr::null_mut()).unwrap();
        let mut err_set = PendingSet::new(ptr::null_mut()).unwrap();
        let out_index = out_set
            .add_source(1, out.source().value(), out_writer.adapter())
            .unwrap();
        // Equal Windows HANDLE values across distinct logical standard channels
        // are admitted even in ONE set, without owning or associating that handle.
        out_set
            .add_source(2, err.source().value(), idle_alias.adapter())
            .unwrap();
        let err_index = err_set
            .add_source(1, err.source().value(), err_writer.adapter())
            .unwrap();
        assert_eq!(super::super::helper::starts_for_test(), starts);
        let bytes = std::vec![b'A'; 1_048_576];
        let indefinite = DrWaitBudgetV1::indefinite();
        let mut out_group = out_set
            .begin_selected(
                ptr::null(),
                BufferList::from_iter([Buffer::Write(&bytes)].into_iter()).unwrap(),
                &indefinite,
                selection(out_index),
            )
            .unwrap();
        assert_eq!(super::super::helper::starts_for_test(), starts + 1);
        assert_eq!(
            crate::byte_io::write(control, b"R".as_ptr(), 1),
            Ok(crate::byte_io::WriteProgress::Accepted(1))
        );
        // Ordinary synchronous output through the physical alias keeps its old
        // blocking behavior and starts no helper, while the stdout group lives.
        assert_eq!(
            super::super::write(StandardStream::Stderr, b"L".as_ptr(), 1),
            super::super::WriteOutcome::Success
        );
        assert_eq!(super::super::helper::starts_for_test(), starts + 1);
        let mut err_group = err_set
            .begin_selected(
                ptr::null(),
                BufferList::from_iter([Buffer::Write(b"B")].into_iter()).unwrap(),
                &indefinite,
                selection(err_index),
            )
            .unwrap();
        #[cfg(unix)]
        for descriptor in [1, 2] {
            assert_eq!(libc::fcntl(descriptor, libc::F_GETFL), flags);
        }
        let count = out_group
            .next(ptr::null())
            .unwrap()
            .outcome
            .transfer
            .unwrap()
            .progress()
            .unwrap();
        assert!(count > 0 && count <= bytes.len());
        drop(out_group);
        assert!(out.try_borrow().is_some());
        assert!(
            err.try_borrow().is_none(),
            "other group retains its independent claim"
        );
        #[cfg(unix)]
        for descriptor in [1, 2] {
            assert_eq!(
                libc::fcntl(descriptor, libc::F_GETFL),
                flags,
                "one group cannot restore a shared mode under another"
            );
        }
        assert_eq!(
            err_group
                .next(ptr::null())
                .unwrap()
                .outcome
                .transfer
                .unwrap()
                .progress(),
            Some(1)
        );
        drop(err_group);
        assert_eq!(
            (
                (*out_writer.as_ptr()).completions,
                (*err_writer.as_ptr()).completions,
                (*idle_alias.as_ptr()).completions
            ),
            (1, 1, 0)
        );
        assert_eq!(
            crate::byte_io::write(control, (count as u64).to_le_bytes().as_ptr(), 8),
            Ok(crate::byte_io::WriteProgress::Accepted(8))
        );
        assert!(err.try_borrow().is_some());
    }
}
