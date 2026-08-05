#![allow(clippy::unwrap_used)]

use super::Store;

/// Each test gets a unique path so parallel runs don't race on /tmp/x. The file
/// is seeded with `raw` so `reconcile` sees a consistent disk-vs-memory state.
pub(crate) fn test_path() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("chiba-core-test-{}-{}.md", std::process::id(), n))
}

/// Wrap a todo.txt-shaped fixture in markdown checkboxes. Fixtures stay written
/// in the compact todo.txt form — that's still exactly what `Task::raw` holds —
/// while landing on disk the way chiba actually stores them.
pub(crate) use crate::todo::from_todotxt as md;

/// `std::fs::write` for a todo.txt-shaped fixture: wraps it in markdown first.
/// Used by tests that simulate an external edit by writing the file directly.
pub(crate) fn write_md(path: impl AsRef<std::path::Path>, raw: &str) -> std::io::Result<()> {
    std::fs::write(path, md(raw))
}

/// Build a `Store` rooted at a fresh temp file seeded with `raw`. Archive loads
/// synchronously (`open_sync`), and today is fixed at 2026-05-06.
pub(crate) fn build_store(raw: &str) -> Store {
    let path = test_path();
    let body = md(raw);
    std::fs::write(&path, &body).unwrap();
    Store::open_sync(path, body, "2026-05-06".into())
}
