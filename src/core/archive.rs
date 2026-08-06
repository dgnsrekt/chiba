use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use super::Store;
use super::outcome::{
    ArchiveDeleteOutcome, ArchiveOutcome, Reconcile, StoreError, UnarchiveOutcome,
};
use crate::app::ArchiveMode;
use crate::todo::{self, Task};

/// Owns the archived (`done.md`) tasks and the lifecycle around loading them
/// off-thread at startup. Fields are `pub(crate)` so the `Store` methods in this
/// file can mutate the archive directly; external callers go through the read
/// methods.
pub struct Archive {
    pub(crate) tasks: Vec<Task>,
    /// Non-task lines of the archive file. `done.md` is a markdown document
    /// like any other; rewriting it must not eat the user's headings.
    pub(crate) text: todo::Text,
    pub(crate) path: PathBuf,
    pub(crate) last_disk: String,
    pub(crate) loader: Option<Receiver<(String, todo::Doc)>>,
}

fn done_path(todo_path: &Path) -> PathBuf {
    todo_path
        .parent()
        .map(|p| p.join("done.md"))
        .unwrap_or_else(|| PathBuf::from("done.md"))
}

impl Archive {
    /// Construct an `Archive` for the sibling `done.md` of `todo_path` and
    /// spawn a worker thread to read+parse it. The first frame can render
    /// `todo.md` immediately while the loader runs in the background.
    pub fn spawn(todo_path: &Path) -> Self {
        Self::spawn_at(done_path(todo_path))
    }

    /// Like [`Archive::spawn`] but for an explicit `done.md` path (e.g. a
    /// `DONE_FILE` that isn't a sibling of the todo file).
    pub fn spawn_at(path: PathBuf) -> Self {
        let loader_path = path.clone();
        let (tx, rx) = mpsc::sync_channel::<(String, todo::Doc)>(1);
        thread::spawn(move || {
            let body = std::fs::read_to_string(&loader_path).unwrap_or_default();
            let parsed = todo::parse_doc(&body);
            let _ = tx.send((body, parsed));
        });
        Self {
            tasks: Vec::new(),
            text: todo::Text::default(),
            path,
            last_disk: String::new(),
            loader: Some(rx),
        }
    }

    /// Read and parse the sibling `done.md` inline (no background thread).
    /// Used by the one-shot CLI, where spawning a loader would be wasteful.
    pub fn load_sync(todo_path: &Path) -> Self {
        Self::load_sync_at(done_path(todo_path))
    }

    /// Like [`Archive::load_sync`] but for an explicit `done.md` path.
    pub fn load_sync_at(path: PathBuf) -> Self {
        let body = std::fs::read_to_string(&path).unwrap_or_default();
        let doc = todo::parse_doc(&body);
        Self {
            tasks: doc.tasks,
            text: doc.text,
            path,
            last_disk: body,
            loader: None,
        }
    }

    /// Test-only constructor that skips the worker thread and seeds in-memory
    /// state directly.
    #[cfg(test)]
    pub(crate) fn for_test(tasks: Vec<Task>, last_disk: String, path: PathBuf) -> Self {
        Self {
            tasks,
            text: todo::Text::default(),
            path,
            last_disk,
            loader: None,
        }
    }

    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }
}

/// Internal result of refreshing `done.md` before a mutation that writes it.
enum ArchiveRefresh {
    Ready,
    Reloaded,
    Error(std::io::Error),
}

impl Store {
    fn read_archive_body(&self) -> std::io::Result<String> {
        match std::fs::read_to_string(&self.archive.path) {
            Ok(body) => Ok(body),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(e),
        }
    }

    fn refresh_archive_for_mutation(&mut self) -> ArchiveRefresh {
        let body = match self.read_archive_body() {
            Ok(b) => b,
            Err(e) => return ArchiveRefresh::Error(e),
        };
        if body != self.archive.last_disk {
            let doc = todo::parse_doc(&body);
            self.archive.tasks = doc.tasks;
            self.archive.text = doc.text;
            self.archive.last_disk = body;
            self.archive.loader = None;
            return ArchiveRefresh::Reloaded;
        }
        self.archive.loader = None;
        ArchiveRefresh::Ready
    }

    /// Pump archive state. Returns true when the visible archive changed: the
    /// startup loader landed, or an external edit to `done.md` was picked up.
    /// Non-blocking. The caller (TUI) is responsible for any view recompute.
    pub fn poll_archive(&mut self) -> bool {
        self.poll_archive_with(ArchiveMode::File)
    }

    /// As [`Store::poll_archive`], but a no-op under `in_place` — done.md is
    /// never written in that mode, so re-reading it every tick is pure waste.
    pub fn poll_archive_with(&mut self, mode: ArchiveMode) -> bool {
        if mode == ArchiveMode::InPlace {
            return false;
        }
        let mut changed = false;
        if let Some(rx) = &self.archive.loader {
            match rx.try_recv() {
                Ok((body, doc)) => {
                    self.archive.last_disk = body;
                    self.archive.tasks = doc.tasks;
                    self.archive.text = doc.text;
                    self.archive.loader = None;
                    changed = true;
                }
                Err(TryRecvError::Empty) => return false,
                Err(TryRecvError::Disconnected) => {
                    self.archive.loader = None;
                }
            }
        }
        if !changed {
            let read = std::fs::read_to_string(&self.archive.path);
            changed = self.apply_archive_read(read);
        }
        changed
    }

    /// Apply a read result for `done.md`. `NotFound` is treated as an empty
    /// archive; any other I/O error preserves in-memory state and returns
    /// `false` rather than wiping the archive.
    pub(crate) fn apply_archive_read(&mut self, read: std::io::Result<String>) -> bool {
        let on_disk = match read {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(_) => return false,
        };
        if on_disk == self.archive.last_disk {
            return false;
        }
        let doc = todo::parse_doc(&on_disk);
        self.archive.tasks = doc.tasks;
        self.archive.text = doc.text;
        self.archive.last_disk = on_disk;
        true
    }

    pub fn archive_completed(&mut self, mode: ArchiveMode) -> ArchiveOutcome {
        match self.reconcile() {
            Reconcile::Unchanged => {}
            other => return ArchiveOutcome::Aborted(other),
        }
        let to_move: Vec<Task> = self.tasks.iter().filter(|t| t.done).cloned().collect();
        if to_move.is_empty() {
            return ArchiveOutcome::Nothing;
        }
        // In-place: the completed tasks are already where they belong. Bail
        // before any I/O — no done.md, no todo.md rewrite, no undo entry,
        // because nothing changed.
        if mode == ArchiveMode::InPlace {
            return ArchiveOutcome::InPlace {
                count: to_move.len(),
            };
        }
        // Read fresh so an external edit to done.md since startup isn't lost.
        let previous_archive_body = match self.read_archive_body() {
            Ok(b) => b,
            Err(e) => return ArchiveOutcome::Error(StoreError::ArchiveIo(e)),
        };
        let mut combined = previous_archive_body.clone();
        if !combined.is_empty() && !combined.ends_with('\n') {
            combined.push('\n');
        }
        combined.push_str(&todo::serialize(&to_move));
        // Write done.md before truncating todo.md so a failed archive can't
        // lose data; if the todo write fails, roll done.md back.
        if let Err(e) = todo::write_atomic(&self.archive.path, &combined) {
            return ArchiveOutcome::Error(StoreError::ArchiveIo(e));
        }
        // Drop the archived tasks descending so every removal keeps the text
        // anchors in step; a bulk `retain` would strand them.
        let snapshot = (self.tasks.clone(), self.text.clone());
        let done_idx: Vec<usize> = self
            .tasks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.done)
            .map(|(i, _)| i)
            .rev()
            .collect();
        for idx in done_idx {
            self.task_remove(idx);
        }
        let remaining_body = todo::serialize_doc(&self.tasks, &self.text);
        if let Err(e) = todo::write_atomic(&self.file_path, &remaining_body) {
            let _ = todo::write_atomic(&self.archive.path, &previous_archive_body);
            (self.tasks, self.text) = snapshot;
            return ArchiveOutcome::Error(StoreError::Write(e));
        }
        let count = to_move.len();
        self.history.push(snapshot);
        self.last_disk = remaining_body;
        let doc = todo::parse_doc(&combined);
        self.archive.tasks = doc.tasks;
        self.archive.text = doc.text;
        self.archive.last_disk = combined;
        self.archive.loader = None;
        ArchiveOutcome::Archived { count }
    }

    /// Move an archived task back into the live list. `archive_idx` indexes
    /// `self.archive.tasks()`.
    pub fn unarchive(&mut self, archive_idx: usize) -> UnarchiveOutcome {
        match self.reconcile() {
            Reconcile::Unchanged => {}
            other => return UnarchiveOutcome::Aborted(other),
        }
        match self.refresh_archive_for_mutation() {
            ArchiveRefresh::Ready => {}
            ArchiveRefresh::Reloaded => return UnarchiveOutcome::DoneReloaded,
            ArchiveRefresh::Error(e) => return UnarchiveOutcome::Error(StoreError::ArchiveIo(e)),
        }
        if archive_idx >= self.archive.tasks.len() {
            return UnarchiveOutcome::OutOfRange;
        }
        let mut task = self.archive.tasks[archive_idx].clone();
        if let Err(e) = task.unmark_done() {
            return UnarchiveOutcome::Error(StoreError::Parse(e));
        }
        let mut new_archive = self.archive.tasks.clone();
        new_archive.remove(archive_idx);
        // Keep the archive's own prose: `done.md` is a markdown document, and
        // rewriting it from tasks alone is the exact data loss this fork exists
        // to prevent.
        let mut new_text = self.archive.text.clone();
        new_text.on_remove(archive_idx);
        let archive_body = todo::serialize_doc(&new_archive, &new_text);
        if let Err(e) = todo::write_atomic(&self.archive.path, &archive_body) {
            return UnarchiveOutcome::Error(StoreError::ArchiveIo(e));
        }
        self.archive.tasks = new_archive;
        self.archive.text = new_text;
        self.archive.last_disk = archive_body;
        self.push_history();
        self.task_push(task);
        if let Err(e) = self.persist() {
            return UnarchiveOutcome::Error(e);
        }
        UnarchiveOutcome::Unarchived
    }

    /// Permanently remove an archived task from `done.md`.
    pub fn archive_delete(&mut self, archive_idx: usize) -> ArchiveDeleteOutcome {
        match self.refresh_archive_for_mutation() {
            ArchiveRefresh::Ready => {}
            ArchiveRefresh::Reloaded => return ArchiveDeleteOutcome::DoneReloaded,
            ArchiveRefresh::Error(e) => {
                return ArchiveDeleteOutcome::Error(StoreError::ArchiveIo(e));
            }
        }
        if archive_idx >= self.archive.tasks.len() {
            return ArchiveDeleteOutcome::OutOfRange;
        }
        let mut new_archive = self.archive.tasks.clone();
        new_archive.remove(archive_idx);
        // Keep the archive's own prose: `done.md` is a markdown document, and
        // rewriting it from tasks alone is the exact data loss this fork exists
        // to prevent.
        let mut new_text = self.archive.text.clone();
        new_text.on_remove(archive_idx);
        let archive_body = todo::serialize_doc(&new_archive, &new_text);
        if let Err(e) = todo::write_atomic(&self.archive.path, &archive_body) {
            return ArchiveDeleteOutcome::Error(StoreError::ArchiveIo(e));
        }
        self.archive.tasks = new_archive;
        self.archive.text = new_text;
        self.archive.last_disk = archive_body;
        ArchiveDeleteOutcome::Deleted
    }

    pub(crate) fn persist(&mut self) -> Result<(), StoreError> {
        let body = todo::serialize_doc(&self.tasks, &self.text);
        match todo::write_atomic(&self.file_path, &body) {
            Ok(()) => {
                self.last_disk = body;
                Ok(())
            }
            Err(e) => Err(StoreError::Write(e)),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::core::Store;
    use crate::core::test_support::{build_store, md, write_md};
    use std::time::{Duration, Instant};

    fn dir_for(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("chiba-archive-test-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn archive_writes_done_file_then_truncates_todo() {
        let dir = dir_for("ok");
        let todo_path = dir.join("todo.md");
        let raw = "(A) 2026-05-01 keep this +work\n\
                   x 2026-05-05 2026-05-01 archive this +work\n";
        write_md(&todo_path, raw).unwrap();
        let mut store = Store::open_sync(todo_path.clone(), md(raw), "2026-05-06".into());
        assert!(matches!(
            store.archive_completed(ArchiveMode::File),
            ArchiveOutcome::Archived { count: 1 }
        ));
        let done = std::fs::read_to_string(dir.join("done.md")).unwrap();
        assert!(done.contains("archive this"));
        let todo = std::fs::read_to_string(&todo_path).unwrap();
        assert!(todo.contains("keep this"));
        assert!(!todo.contains("archive this"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn archive_appends_to_existing_done_file() {
        let dir = dir_for("append");
        let todo_path = dir.join("todo.md");
        write_md(dir.join("done.md"), "x 2026-04-01 2026-03-01 prior\n").unwrap();
        let raw = "x 2026-05-05 2026-05-01 fresh +work\n";
        write_md(&todo_path, raw).unwrap();
        let mut store = Store::open_sync(todo_path, md(raw), "2026-05-06".into());
        store.archive_completed(ArchiveMode::File);
        let done = std::fs::read_to_string(dir.join("done.md")).unwrap();
        assert!(done.contains("prior"));
        assert!(done.contains("fresh"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn archive_nothing_when_no_completed() {
        let mut store = build_store("a\nb\n");
        assert!(matches!(
            store.archive_completed(ArchiveMode::File),
            ArchiveOutcome::Nothing
        ));
    }

    fn wait_archive_loaded(store: &mut Store) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while store.archive.loader.is_some() && Instant::now() < deadline {
            let _ = store.poll_archive();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(store.archive.loader.is_none());
    }

    #[test]
    fn archive_loader_populates_archived_from_done_file() {
        let dir = dir_for("loader");
        let todo_path = dir.join("todo.md");
        write_md(
            dir.join("done.md"),
            "x 2026-05-01 2026-04-01 first\nx 2026-05-02 2026-04-15 second\n",
        )
        .unwrap();
        write_md(&todo_path, "(A) 2026-05-06 still open\n").unwrap();
        let mut store = Store::new(
            todo_path,
            md("(A) 2026-05-06 still open\n"),
            "2026-05-06".into(),
        );
        wait_archive_loaded(&mut store);
        assert_eq!(store.archive.len(), 2);
        assert!(
            store
                .archive
                .tasks()
                .iter()
                .any(|t| t.raw.contains("first"))
        );
        assert_eq!(store.tasks().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn archive_completed_populates_in_memory_archived() {
        let dir = dir_for("memsync");
        let todo_path = dir.join("todo.md");
        let raw = "x 2026-05-05 2026-05-01 done one\nx 2026-05-06 2026-05-01 done two\n";
        write_md(&todo_path, raw).unwrap();
        let mut store = Store::new(todo_path, md(raw), "2026-05-06".into());
        store.archive_completed(ArchiveMode::File);
        assert_eq!(store.archive.len(), 2);
        let _ = store.poll_archive();
        std::thread::sleep(Duration::from_millis(20));
        let _ = store.poll_archive();
        assert_eq!(store.archive.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn poll_archive_detects_external_done_edit() {
        let dir = dir_for("external");
        let todo_path = dir.join("todo.md");
        write_md(&todo_path, "(A) 2026-05-06 a\n").unwrap();
        write_md(dir.join("done.md"), "").unwrap();
        let mut store = Store::new(todo_path, md("(A) 2026-05-06 a\n"), "2026-05-06".into());
        wait_archive_loaded(&mut store);
        assert!(store.archive.is_empty());
        write_md(
            dir.join("done.md"),
            "x 2026-05-05 2026-05-01 added externally\n",
        )
        .unwrap();
        assert!(store.poll_archive());
        assert_eq!(store.archive.len(), 1);
        assert!(store.archive.tasks()[0].raw.contains("added externally"));
        assert!(!store.poll_archive());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn poll_archive_preserves_archived_on_io_error() {
        let mut store = build_store("a\n");
        let path = store.archive.path().to_path_buf();
        store.archive = Archive::for_test(
            todo::parse_file(&md("x 2026-05-01 2026-04-01 prior\n")),
            md("x 2026-05-01 2026-04-01 prior\n"),
            path,
        );
        let err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        assert!(!store.apply_archive_read(Err(err)));
        assert_eq!(store.archive.len(), 1);
    }

    #[test]
    fn archive_delete_refreshes_done_txt_before_writing() {
        let dir = dir_for("delete-refresh");
        let todo_path = dir.join("todo.md");
        let done_path = dir.join("done.md");
        write_md(&todo_path, "open\n").unwrap();
        write_md(&done_path, "x 2026-05-01 2026-04-01 stale\n").unwrap();
        let mut store = Store::new(todo_path, md("open\n"), "2026-05-06".into());
        wait_archive_loaded(&mut store);
        write_md(
            &done_path,
            "x 2026-05-01 2026-04-01 stale\nx 2026-05-02 2026-04-02 external\n",
        )
        .unwrap();
        assert!(matches!(
            store.archive_delete(0),
            ArchiveDeleteOutcome::DoneReloaded
        ));
        let done = std::fs::read_to_string(&done_path).unwrap();
        assert!(done.contains("stale"));
        assert!(done.contains("external"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unarchive_recomplete_does_not_duplicate_recurring_successor() {
        let dir = dir_for("rec-roundtrip");
        let todo_path = dir.join("todo.md");
        let raw = "Water plants due:2026-05-06 rec:1d\n";
        write_md(&todo_path, raw).unwrap();
        let mut store = Store::new(todo_path, md(raw), "2026-05-06".into());
        store.toggle_complete(0);
        assert_eq!(store.tasks().len(), 2);
        store.archive_completed(ArchiveMode::File);
        assert_eq!(store.tasks().len(), 1);
        assert_eq!(store.archive.len(), 1);
        store.unarchive(0);
        assert_eq!(store.tasks().len(), 2);
        let idx = store
            .tasks()
            .iter()
            .position(|t| !t.done && t.due.as_deref() == Some("2026-05-06"))
            .unwrap();
        store.toggle_complete(idx);
        assert_eq!(store.tasks().len(), 2);
        let next_count = store
            .tasks()
            .iter()
            .filter(|t| !t.done && t.due.as_deref() == Some("2026-05-07"))
            .count();
        assert_eq!(next_count, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persist_reports_write_failure() {
        let mut store = build_store("a\n");
        let missing_parent = std::env::temp_dir()
            .join(format!("chiba-missing-parent-{}", std::process::id()))
            .join("todo.md");
        let _ = std::fs::remove_dir_all(missing_parent.parent().unwrap());
        store.file_path = missing_parent;
        assert!(store.persist().is_err());
    }

    #[test]
    fn unarchive_preserves_prose_in_the_done_file() {
        let dir = dir_for("done-prose");
        let todo_path = dir.join("todo.md");
        let done_path = dir.join("done.md");
        let done_body = "# Archived\n\nStuff I finished.\n\n\
                         - [x] 2026-05-01 2026-04-01 first\n\
                         - [x] 2026-05-02 2026-04-02 second\n";
        std::fs::write(&todo_path, "- [ ] live\n").unwrap();
        std::fs::write(&done_path, done_body).unwrap();
        let mut store = Store::new(todo_path, "- [ ] live\n".to_string(), "2026-05-06".into());
        wait_archive_loaded(&mut store);
        assert_eq!(store.archive.len(), 2);

        store.unarchive(0);

        let after = std::fs::read_to_string(&done_path).unwrap();
        assert!(after.contains("# Archived"), "heading eaten:\n{after}");
        assert!(after.contains("Stuff I finished."), "prose eaten:\n{after}");
        assert!(!after.contains("first"), "unarchived task should be gone");
        assert!(
            after.contains("second"),
            "other archived task should remain"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn archive_delete_preserves_prose_in_the_done_file() {
        let dir = dir_for("done-prose-del");
        let todo_path = dir.join("todo.md");
        let done_path = dir.join("done.md");
        std::fs::write(&todo_path, "- [ ] live\n").unwrap();
        std::fs::write(
            &done_path,
            "# Archived\n\n- [x] 2026-05-01 2026-04-01 first\n",
        )
        .unwrap();
        let mut store = Store::new(todo_path, "- [ ] live\n".to_string(), "2026-05-06".into());
        wait_archive_loaded(&mut store);

        store.archive_delete(0);

        let after = std::fs::read_to_string(&done_path).unwrap();
        assert!(after.contains("# Archived"), "heading eaten:\n{after}");
        assert!(!after.contains("first"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn in_place_archives_nothing_and_writes_nothing() {
        let dir = dir_for("in-place");
        let todo_path = dir.join("todo.md");
        let raw = "(A) 2026-05-01 keep this +work\n\
                   x 2026-05-05 2026-05-01 completed thing +work\n";
        write_md(&todo_path, raw).unwrap();
        let before = std::fs::read_to_string(&todo_path).unwrap();
        let mut store = Store::open_sync(todo_path.clone(), md(raw), "2026-05-06".into());

        let outcome = store.archive_completed(ArchiveMode::InPlace);

        assert!(
            matches!(outcome, ArchiveOutcome::InPlace { count: 1 }),
            "got {outcome:?}",
        );
        assert_eq!(
            std::fs::read_to_string(&todo_path).unwrap(),
            before,
            "todo.md must be untouched",
        );
        assert!(!dir.join("done.md").exists(), "done.md must not be created");
        assert_eq!(
            store.tasks().len(),
            2,
            "the completed task stays in the list"
        );
        assert!(
            store.history.is_empty(),
            "nothing happened, so nothing to undo"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn in_place_still_reports_nothing_when_there_is_nothing_done() {
        let dir = dir_for("in-place-empty");
        let todo_path = dir.join("todo.md");
        write_md(&todo_path, "(A) 2026-05-01 open task\n").unwrap();
        let mut store = Store::open_sync(
            todo_path,
            md("(A) 2026-05-01 open task\n"),
            "2026-05-06".into(),
        );
        assert!(matches!(
            store.archive_completed(ArchiveMode::InPlace),
            ArchiveOutcome::Nothing
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn in_place_skips_polling_the_done_file() {
        let mut store = build_store("a\n");
        // Even with a real external edit pending, in-place never looks.
        assert!(!store.poll_archive_with(ArchiveMode::InPlace));
    }
}
