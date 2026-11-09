use super::Capture;

#[test]
fn captures_keep_at_most_their_limit_and_report_dropped_bytes() {
    unsafe {
        let mut capture = Capture::new();
        assert!(capture.append(core::ptr::null(), b"abc", 5));
        assert!(!capture.append(core::ptr::null(), b"defg", 5));
        assert!(!capture.append(core::ptr::null(), b"h", 5));
        assert_eq!(capture.length, 5);
        assert_eq!(core::slice::from_raw_parts(capture.data, 5), b"abcde");
        let mut empty = Capture::new();
        assert!(!empty.append(core::ptr::null(), b"x", 0));
        assert!(empty.append(core::ptr::null(), b"", 0));
        assert!(empty.data.is_null());
    }
}
