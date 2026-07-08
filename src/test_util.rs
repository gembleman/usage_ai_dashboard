//! Shared test-only helpers.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// A JSONL temp file that removes itself on drop. Used by the parser tests
/// to avoid pulling in an external tempfile dependency.
pub struct TempFile {
    pub path: PathBuf,
}

impl TempFile {
    pub fn new(lines: &[&str]) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let mut path = std::env::temp_dir();
        let unique = format!(
            "usage_ai_test_{}_{}.jsonl",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        path.push(unique);
        let mut f = std::fs::File::create(&path).unwrap();
        for line in lines {
            writeln!(f, "{}", line).unwrap();
        }
        TempFile { path }
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
