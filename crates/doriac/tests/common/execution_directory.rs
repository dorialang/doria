use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub struct ExecutionDirectory(PathBuf);

impl ExecutionDirectory {
    pub fn new() -> Self {
        static NEXT_RUN: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "doria-mir-execution-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_RUN.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir(&directory).unwrap();
        Self(directory)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ExecutionDirectory {
    fn drop(&mut self) {
        // Do not replace an existing test failure with a second panic.
        let cleanup = std::fs::remove_dir_all(&self.0);
        if !std::thread::panicking() {
            cleanup.unwrap_or_else(|error| {
                panic!(
                    "failed to clean native test directory {}: {error}",
                    self.0.display()
                );
            });
        }
    }
}
