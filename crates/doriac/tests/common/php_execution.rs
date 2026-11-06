use std::process::Command;

use crate::execution_directory::ExecutionDirectory;

/// Keep the script alive until its PHP process has exited.
pub struct Script {
    directory: ExecutionDirectory,
}

impl Script {
    pub fn new(body: &str) -> Self {
        let directory = ExecutionDirectory::new();
        std::fs::write(
            directory.path().join("script.php"),
            format!("<?php\n{body}"),
        )
        .expect("write PHP execution fixture");
        std::fs::write(
            directory.path().join("run.php"),
            "<?php\nrequire __DIR__ . DIRECTORY_SEPARATOR . 'script.php';\n",
        )
        .expect("write PHP execution entry point");
        Self { directory }
    }

    pub fn command(&self, options: &[&str]) -> Command {
        let mut command = Command::new("php");
        command
            .args(options)
            .arg(self.directory.path().join("run.php"));
        command
    }
}
