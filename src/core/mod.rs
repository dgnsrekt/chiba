//! Headless core: the durable task store, its persistence/I-O, and all task
//! mutations. Carries no view, input, or presentation state — operations return
//! structured [`outcome`] values rather than user-facing strings. Both the TUI
//! (`App` wraps a `Store`) and the CLI (`cmd`) drive this type.

use std::path::{Path, PathBuf};

use crate::todo::{self, Task};

mod archive;
mod external;
mod history;
mod mutations;

pub mod filter;
pub mod outcome;

#[cfg(test)]
pub(crate) mod test_support;

pub use archive::Archive;
pub use history::History;
pub use outcome::{
    AddOutcome, ArchiveDeleteOutcome, ArchiveOutcome, BulkCompleteOutcome, BulkDeleteOutcome,
    CompleteOutcome, DeleteOutcome, DrainReport, EditOutcome, PriorityOutcome, Reconcile,
    StoreError, TagOutcome, UnarchiveOutcome, UndoOutcome,
};

/// The durable task store. Owns the live task list, the sibling `done.md`
/// archive, undo history, and the on-disk reconciliation snapshot.
pub struct Store {
    pub(crate) tasks: Vec<Task>,
    /// Non-task lines of the todo file (headings, prose, fenced code), anchored
    /// to the tasks they precede so writing the file back never destroys or
    /// reorders them.
    pub(crate) text: todo::Text,
    pub(crate) history: History,
    pub(crate) archive: Archive,
    pub(crate) file_path: PathBuf,
    /// Snapshot of the file body the last time we read or wrote it; used by
    /// `reconcile` to detect external edits.
    pub(crate) last_disk: String,
    pub(crate) today: String,
}

impl Store {
    /// Construct a store, loading the archive (`done.md`) off-thread from the
    /// sibling of `file_path`. Used by the TUI so the first frame doesn't wait
    /// on the archive read.
    pub fn new(file_path: PathBuf, body: String, today: String) -> Self {
        let archive = Archive::spawn(&file_path);
        Self::assemble(file_path, archive, body, today)
    }

    /// Like [`Store::new`] but with an explicit `done.md` path (e.g. from a
    /// `DONE_FILE` env var that isn't a sibling of the todo file).
    pub fn new_with_done(
        file_path: PathBuf,
        done_path: PathBuf,
        body: String,
        today: String,
    ) -> Self {
        let archive = Archive::spawn_at(done_path);
        Self::assemble(file_path, archive, body, today)
    }

    /// Construct a store, loading the sibling archive synchronously (no
    /// background thread). Used by the one-shot CLI.
    pub fn open_sync(file_path: PathBuf, body: String, today: String) -> Self {
        let archive = Archive::load_sync(&file_path);
        Self::assemble(file_path, archive, body, today)
    }

    /// Like [`Store::open_sync`] but with an explicit `done.md` path.
    pub fn open_sync_with_done(
        file_path: PathBuf,
        done_path: PathBuf,
        body: String,
        today: String,
    ) -> Self {
        let archive = Archive::load_sync_at(done_path);
        Self::assemble(file_path, archive, body, today)
    }

    fn assemble(file_path: PathBuf, archive: Archive, body: String, today: String) -> Self {
        let doc = todo::parse_doc(&body);
        Self {
            tasks: doc.tasks,
            text: doc.text,
            history: History::default(),
            archive,
            file_path,
            last_disk: body,
            today,
        }
    }

    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    /// Insert a task, keeping the document's text anchors in step.
    ///
    /// Every length-changing mutation must go through these two helpers rather
    /// than touching `self.tasks` directly — an anchor left behind silently
    /// reorders the file on the next write.
    pub(crate) fn task_insert(&mut self, idx: usize, task: Task) {
        self.text.on_insert(idx);
        self.tasks.insert(idx, task);
    }

    /// Remove the task at `idx`, keeping text anchors in step.
    pub(crate) fn task_remove(&mut self, idx: usize) -> Task {
        self.text.on_remove(idx);
        self.tasks.remove(idx)
    }

    /// Append a task. Anchors are untouched: a task added at the end lands
    /// after the document's trailing prose, which is where people expect it.
    pub(crate) fn task_push(&mut self, task: Task) {
        self.tasks.push(task);
    }

    pub fn archive(&self) -> &Archive {
        &self.archive
    }

    pub fn today(&self) -> &str {
        &self.today
    }

    pub fn file_path(&self) -> &Path {
        &self.file_path
    }

    /// Cloned `raw` for the task at `abs`, or `None` if out of range.
    pub fn task_raw(&self, abs: usize) -> Option<String> {
        self.tasks.get(abs).map(|t| t.raw.clone())
    }

    /// True when at least one live task is marked done.
    pub fn has_completed(&self) -> bool {
        self.tasks.iter().any(|t| t.done)
    }

    /// Update the cached "today". Returns `true` iff the value changed, so the
    /// caller knows to recompute any date-dependent view state.
    pub fn set_today(&mut self, today: String) -> bool {
        if self.today == today {
            return false;
        }
        self.today = today;
        true
    }
}
