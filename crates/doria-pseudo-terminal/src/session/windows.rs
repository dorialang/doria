//! ConPTY. The pseudo-console is a terminal of its own between the program
//! and this side: what arrives here is its rendering of the program's
//! screen, and what is typed here it turns into the program's console input.

use std::ffi::{c_void, OsStr, OsString};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::time::Duration;

use super::Exit;

type Handle = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct Coord {
    x: i16,
    y: i16,
}

#[repr(C)]
struct StartupInfo {
    size: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    x: u32,
    y: u32,
    x_size: u32,
    y_size: u32,
    x_count_chars: u32,
    y_count_chars: u32,
    fill_attribute: u32,
    flags: u32,
    show_window: u16,
    reserved2_size: u16,
    reserved2: *mut u8,
    standard_input: Handle,
    standard_output: Handle,
    standard_error: Handle,
}

#[repr(C)]
struct StartupInfoEx {
    startup: StartupInfo,
    attributes: *mut c_void,
}

#[repr(C)]
struct ProcessInformation {
    process: Handle,
    thread: Handle,
    process_id: u32,
    thread_id: u32,
}

const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;
const CREATE_UNICODE_ENVIRONMENT: u32 = 0x0000_0400;
const PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE: usize = 0x0002_0016;
const STILL_ACTIVE: u32 = 259;
const STARTF_USESTDHANDLES: u32 = 0x0000_0100;
const INVALID_HANDLE: Handle = -1_isize as Handle;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreatePipe(
        read: *mut Handle,
        write: *mut Handle,
        attributes: *const c_void,
        size: u32,
    ) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn ReadFile(
        handle: Handle,
        buffer: *mut c_void,
        length: u32,
        read: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn WriteFile(
        handle: Handle,
        buffer: *const c_void,
        length: u32,
        written: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn CreatePseudoConsole(
        size: Coord,
        input: Handle,
        output: Handle,
        flags: u32,
        console: *mut Handle,
    ) -> i32;
    fn ResizePseudoConsole(console: Handle, size: Coord) -> i32;
    fn ClosePseudoConsole(console: Handle);
    fn InitializeProcThreadAttributeList(
        list: *mut c_void,
        count: u32,
        flags: u32,
        size: *mut usize,
    ) -> i32;
    fn UpdateProcThreadAttribute(
        list: *mut c_void,
        flags: u32,
        attribute: usize,
        value: *const c_void,
        size: usize,
        previous: *mut c_void,
        returned: *mut usize,
    ) -> i32;
    fn DeleteProcThreadAttributeList(list: *mut c_void);
    fn CreateProcessW(
        application: *const u16,
        command_line: *mut u16,
        process_attributes: *const c_void,
        thread_attributes: *const c_void,
        inherit_handles: i32,
        flags: u32,
        environment: *const c_void,
        directory: *const u16,
        startup: *const StartupInfo,
        information: *mut ProcessInformation,
    ) -> i32;
    fn GetExitCodeProcess(process: Handle, code: *mut u32) -> i32;
    fn TerminateProcess(process: Handle, code: u32) -> i32;
    fn GetLastError() -> u32;
}

/// A handle another thread reads until it closes.
struct Shared(Handle);

unsafe impl Send for Shared {}

pub(super) struct Session {
    console: Option<Handle>,
    input: Handle,
    process: Handle,
    process_id: u32,
    output: Receiver<Vec<u8>>,
    reader: Option<std::thread::JoinHandle<()>>,
    /// Queries the pseudo-console has sent and this terminal has not yet
    /// answered, carried across reads.
    unanswered: Vec<u8>,
}

fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().collect()
}

/// One argument quoted the way the C runtime parses a command line.
fn quoted(argument: &OsStr, line: &mut Vec<u16>) {
    line.push(u16::from(b'"'));
    let mut backslashes = 0;
    for unit in argument.encode_wide() {
        if unit == u16::from(b'\\') {
            backslashes += 1;
        } else {
            if unit == u16::from(b'"') {
                line.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes + 1));
            }
            backslashes = 0;
        }
        line.push(unit);
    }
    line.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes));
    line.push(u16::from(b'"'));
}

fn failed(what: &str) -> ! {
    panic!("{what} failed with Win32 error {}", unsafe {
        GetLastError()
    })
}

impl Session {
    pub(super) fn start(
        program: &Path,
        arguments: &[&str],
        environment: &[(OsString, OsString)],
        columns: u16,
        rows: u16,
    ) -> Self {
        unsafe {
            let null = std::ptr::null_mut::<c_void>();
            let (mut input_read, mut input_write) = (null, null);
            let (mut output_read, mut output_write) = (null, null);
            if CreatePipe(&mut input_read, &mut input_write, std::ptr::null(), 0) == 0
                || CreatePipe(&mut output_read, &mut output_write, std::ptr::null(), 0) == 0
            {
                failed("CreatePipe");
            }
            let mut console = null;
            let created = CreatePseudoConsole(
                Coord {
                    x: columns as i16,
                    y: rows as i16,
                },
                input_read,
                output_write,
                0,
                &mut console,
            );
            assert!(created >= 0, "CreatePseudoConsole failed: {created:#x}");
            // The pseudo-console holds its own copies of its two ends.
            CloseHandle(input_read);
            CloseHandle(output_write);

            let mut size = 0;
            InitializeProcThreadAttributeList(null, 1, 0, &mut size);
            let mut attributes = vec![0_u8; size];
            if InitializeProcThreadAttributeList(attributes.as_mut_ptr().cast(), 1, 0, &mut size)
                == 0
                || UpdateProcThreadAttribute(
                    attributes.as_mut_ptr().cast(),
                    0,
                    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
                    console,
                    std::mem::size_of::<Handle>(),
                    null,
                    std::ptr::null_mut(),
                ) == 0
            {
                failed("the pseudo-console attribute");
            }

            let mut line = Vec::new();
            quoted(program.as_os_str(), &mut line);
            for argument in arguments {
                line.push(u16::from(b' '));
                quoted(OsStr::new(argument), &mut line);
            }
            line.push(0);
            let mut block = Vec::new();
            for (name, value) in environment {
                block.extend(wide(name));
                block.push(u16::from(b'='));
                block.extend(wide(value));
                block.push(0);
            }
            block.push(0);

            let mut startup: StartupInfoEx = std::mem::zeroed();
            startup.startup.size = std::mem::size_of::<StartupInfoEx>() as u32;
            startup.attributes = attributes.as_mut_ptr().cast();
            // A test runner's own output is redirected, and a child started
            // without standard handles of its own would be given those
            // redirected ones instead of the pseudo-console. Handles that are
            // explicitly invalid make it open the console it is attached to.
            startup.startup.flags = STARTF_USESTDHANDLES;
            startup.startup.standard_input = INVALID_HANDLE;
            startup.startup.standard_output = INVALID_HANDLE;
            startup.startup.standard_error = INVALID_HANDLE;
            let mut information: ProcessInformation = std::mem::zeroed();
            if CreateProcessW(
                std::ptr::null(),
                line.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
                block.as_ptr().cast(),
                std::ptr::null(),
                &startup.startup,
                &mut information,
            ) == 0
            {
                failed("CreateProcessW");
            }
            DeleteProcThreadAttributeList(attributes.as_mut_ptr().cast());
            CloseHandle(information.thread);

            let (sender, output) = channel();
            let reading = Shared(output_read);
            let reader = std::thread::spawn(move || {
                let reading = reading;
                let mut bytes = [0_u8; 4096];
                loop {
                    let mut read = 0;
                    let ok = ReadFile(
                        reading.0,
                        bytes.as_mut_ptr().cast(),
                        bytes.len() as u32,
                        &mut read,
                        std::ptr::null_mut(),
                    );
                    if ok == 0 || read == 0 || sender.send(bytes[..read as usize].to_vec()).is_err()
                    {
                        break;
                    }
                }
                CloseHandle(reading.0);
            });
            Self {
                console: Some(console),
                input: input_write,
                process: information.process,
                process_id: information.process_id,
                output,
                reader: Some(reader),
                unanswered: Vec::new(),
            }
        }
    }

    pub(super) fn write(&mut self, bytes: &[u8]) {
        let mut written = 0;
        let ok = unsafe {
            WriteFile(
                self.input,
                bytes.as_ptr().cast(),
                bytes.len() as u32,
                &mut written,
                std::ptr::null_mut(),
            )
        };
        assert!(ok != 0 && written as usize == bytes.len(), "typing failed");
    }

    pub(super) fn resize(&mut self, columns: u16, rows: u16) {
        if let Some(console) = self.console {
            let resized = unsafe {
                ResizePseudoConsole(
                    console,
                    Coord {
                        x: columns as i16,
                        y: rows as i16,
                    },
                )
            };
            assert!(resized >= 0, "ResizePseudoConsole failed: {resized:#x}");
        }
    }

    /// A pseudo-console may ask its terminal what it is and where the cursor
    /// is, and some versions wait for the answers before they pass anything
    /// on. This terminal answers like any other.
    fn answer(&mut self, bytes: &[u8]) {
        self.unanswered.extend_from_slice(bytes);
        let mut answers: Vec<&[u8]> = Vec::new();
        for (query, answer) in [
            (&b"\x1b[c"[..], &b"\x1b[?1;0c"[..]),
            (&b"\x1b[0c"[..], &b"\x1b[?1;0c"[..]),
            (&b"\x1b[6n"[..], &b"\x1b[1;1R"[..]),
        ] {
            while let Some(found) = self
                .unanswered
                .windows(query.len())
                .position(|window| window == query)
            {
                self.unanswered.drain(found..found + query.len());
                answers.push(answer);
            }
        }
        // Keep only what could still be the start of a query.
        let keep = self.unanswered.len().min(3);
        let tail = self.unanswered.split_off(self.unanswered.len() - keep);
        self.unanswered = tail;
        for answer in answers {
            self.write(answer);
        }
    }

    pub(super) fn read(&mut self, drawn: &mut Vec<u8>, wait: Duration) -> bool {
        match self.output.recv_timeout(wait) {
            Ok(bytes) => {
                self.answer(&bytes);
                drawn.extend_from_slice(&bytes);
                true
            }
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => false,
        }
    }

    pub(super) fn exited(&mut self) -> Option<Exit> {
        let mut code = 0;
        if unsafe { GetExitCodeProcess(self.process, &mut code) } == 0 {
            failed("GetExitCodeProcess");
        }
        (code != STILL_ACTIVE).then_some(Exit {
            code: Some(code as i32),
            signal: None,
        })
    }

    /// Closing the pseudo-console makes it send whatever it still holds and
    /// then close its end, which is what ends the reader.
    pub(super) fn close(&mut self, drawn: &mut Vec<u8>) {
        if let Some(console) = self.console.take() {
            let closing = Shared(console);
            let closer = std::thread::spawn(move || {
                let closing = closing;
                unsafe { ClosePseudoConsole(closing.0) }
            });
            while let Ok(bytes) = self.output.recv_timeout(Duration::from_secs(10)) {
                drawn.extend_from_slice(&bytes);
            }
            let _ = closer.join();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }

    pub(super) fn kill(&mut self) {
        unsafe { TerminateProcess(self.process, 1) };
    }

    pub(super) fn process(&self) -> u32 {
        self.process_id
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            if let Some(console) = self.console.take() {
                ClosePseudoConsole(console);
            }
            CloseHandle(self.input);
            CloseHandle(self.process);
        }
    }
}
