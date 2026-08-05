//! Moving a directory between todo.txt and chiba's markdown, in both
//! directions.
//!
//! Migration works on the *set* of files todo.txt-cli and chiba share —
//! `todo`, `done`, and `inbox` — because converting the task file alone
//! silently orphans years of archived history sitting right next to it.
//!
//! Nothing is written until every file has been converted in memory *and*
//! verified by round-tripping the result back through the opposite converter.
//! A conversion that can't prove itself lossless touches nothing.

use std::path::{Path, PathBuf};

use crate::todo;

/// The file stems chiba and todo.txt-cli both use, in the order a summary
/// should list them.
const STEMS: [&str; 3] = ["todo", "done", "inbox"];

/// Backup suffix. A migration refuses to overwrite an existing backup rather
/// than inventing a numbering scheme.
const BAK: &str = "bak";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// todo.txt -> todo.md, adopting chiba.
    Import,
    /// todo.md -> todo.txt, handing the directory back to tuxedo.
    Eject,
}

impl Direction {
    fn source_ext(self) -> &'static str {
        match self {
            Direction::Import => "txt",
            Direction::Eject => "md",
        }
    }

    fn target_ext(self) -> &'static str {
        match self {
            Direction::Import => "md",
            Direction::Eject => "txt",
        }
    }
}

/// What a directory currently looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Neither task file exists.
    Fresh,
    /// Only `todo.md` — chiba's native state.
    Markdown,
    /// Only `todo.txt` — not migrated yet.
    TodoTxt,
    /// Both exist. There is no single source of truth, so nothing is safe to
    /// assume; the user has to say which one wins.
    Ambiguous,
}

pub fn state(dir: &Path) -> State {
    match (dir.join("todo.md").exists(), dir.join("todo.txt").exists()) {
        (false, false) => State::Fresh,
        (true, false) => State::Markdown,
        (false, true) => State::TodoTxt,
        (true, true) => State::Ambiguous,
    }
}

#[derive(Debug)]
pub enum Error {
    /// Both `todo.md` and `todo.txt` exist.
    Ambiguous,
    /// Already in the requested format.
    NothingToDo,
    /// A backup from an earlier migration is in the way.
    BackupExists(PathBuf),
    /// The converted file did not survive a round trip. Nothing was written.
    Verify { file: PathBuf, detail: String },
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Ambiguous => write!(
                f,
                "both todo.md and todo.txt exist — resolve which one is authoritative first"
            ),
            Error::NothingToDo => write!(f, "nothing to do; already in that format"),
            Error::BackupExists(p) => write!(
                f,
                "{} already exists — move it aside or pass --force",
                p.display()
            ),
            Error::Verify { file, detail } => write!(
                f,
                "{} failed verification ({detail}); nothing was written",
                file.display()
            ),
            Error::Io { path, source } => write!(f, "{}: {source}", path.display()),
        }
    }
}

/// One file's worth of work, fully prepared and verified in memory.
#[derive(Debug)]
pub struct Step {
    pub from: PathBuf,
    pub to: PathBuf,
    pub backup: PathBuf,
    /// Number of tasks carried across.
    pub tasks: usize,
    /// Non-task lines that the destination format cannot hold. Only ever
    /// non-zero when ejecting: todo.txt has nowhere to put a heading.
    pub dropped: usize,
    /// `true` for files that move without conversion (the inbox spool).
    pub renamed_only: bool,
    body: String,
}

#[derive(Debug)]
pub struct Plan {
    pub dir: PathBuf,
    pub direction: Direction,
    pub steps: Vec<Step>,
}

impl Plan {
    /// Tasks carried across. Excludes the inbox spool, whose lines are
    /// pending capture text rather than tasks.
    pub fn tasks(&self) -> usize {
        self.steps
            .iter()
            .filter(|s| !s.renamed_only)
            .map(|s| s.tasks)
            .sum()
    }

    pub fn dropped(&self) -> usize {
        self.steps.iter().map(|s| s.dropped).sum()
    }
}

/// Non-blank lines, trailing whitespace normalised. Blank lines carry no
/// meaning in todo.txt, so they're not part of what a round trip has to
/// reproduce.
fn significant(s: &str) -> Vec<&str> {
    s.lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
        .collect()
}

/// Build — but do not apply — a migration for `dir`.
///
/// Every file is converted and verified here, so a returned `Plan` is known
/// to be lossless (or, when ejecting, known exactly how lossy).
pub fn plan(dir: &Path, direction: Direction, force: bool) -> Result<Plan, Error> {
    match (state(dir), direction) {
        (State::Ambiguous, _) => return Err(Error::Ambiguous),
        (State::Fresh, _) => return Err(Error::NothingToDo),
        // Already where we're trying to get to.
        (State::Markdown, Direction::Import) | (State::TodoTxt, Direction::Eject) => {
            return Err(Error::NothingToDo);
        }
        _ => {}
    }

    let mut steps = Vec::new();
    for stem in STEMS {
        let from = dir.join(format!("{stem}.{}", direction.source_ext()));
        if !from.exists() {
            continue;
        }
        let to = dir.join(format!("{stem}.{}", direction.target_ext()));
        let backup = dir.join(format!("{stem}.{}.{BAK}", direction.source_ext()));
        if backup.exists() && !force {
            return Err(Error::BackupExists(backup));
        }
        let source = std::fs::read_to_string(&from).map_err(|source| Error::Io {
            path: from.clone(),
            source,
        })?;

        // The inbox is a plain-line capture spool, not a task list — its lines
        // are natural language awaiting the drain pipeline. Wrapping them in
        // checkboxes would be wrong, so it only changes name.
        if stem == "inbox" {
            steps.push(Step {
                from,
                to,
                backup,
                tasks: source.lines().filter(|l| !l.trim().is_empty()).count(),
                dropped: 0,
                renamed_only: true,
                body: source,
            });
            continue;
        }

        let (body, dropped) = match direction {
            Direction::Import => (todo::from_todotxt(&source), 0),
            Direction::Eject => todo::to_todotxt(&source),
        };
        verify(&from, &source, &body, direction)?;
        steps.push(Step {
            tasks: match direction {
                Direction::Import => todo::parse_doc(&body).tasks.len(),
                Direction::Eject => significant(&body).len(),
            },
            from,
            to,
            backup,
            dropped,
            renamed_only: false,
            body,
        });
    }

    if steps.is_empty() {
        return Err(Error::NothingToDo);
    }
    Ok(Plan {
        dir: dir.to_path_buf(),
        direction,
        steps,
    })
}

/// Prove the conversion lost nothing by running it back the other way.
///
/// Importing must be *exactly* reversible — every non-blank todo.txt line has
/// to come back byte-identical. Ejecting can't be, because todo.txt has no
/// place for a heading, so it instead has to preserve every task body in
/// order; the prose it drops is reported separately and survives in the backup.
fn verify(path: &Path, source: &str, converted: &str, direction: Direction) -> Result<(), Error> {
    let fail = |detail: String| {
        Err(Error::Verify {
            file: path.to_path_buf(),
            detail,
        })
    };
    match direction {
        Direction::Import => {
            let (back, _) = todo::to_todotxt(converted);
            let (before, after) = (significant(source), significant(&back));
            if before != after {
                let first = before
                    .iter()
                    .zip(after.iter())
                    .position(|(a, b)| a != b)
                    .unwrap_or(before.len().min(after.len()));
                return fail(format!(
                    "{} lines in, {} back; first difference at line {}",
                    before.len(),
                    after.len(),
                    first + 1,
                ));
            }
        }
        Direction::Eject => {
            let before: Vec<String> = todo::parse_doc(source)
                .tasks
                .iter()
                .map(|t| t.raw.clone())
                .collect();
            let after: Vec<String> = todo::parse_doc(&todo::from_todotxt(converted))
                .tasks
                .iter()
                .map(|t| t.raw.clone())
                .collect();
            if before != after {
                return fail(format!("{} tasks in, {} back", before.len(), after.len()));
            }
        }
    }
    Ok(())
}

/// Apply a verified plan.
///
/// Order matters: the new file lands *before* the old one is renamed, so an
/// interruption leaves both files present — the ambiguous state, which is
/// recoverable — rather than neither.
pub fn apply(plan: &Plan) -> Result<(), Error> {
    for step in &plan.steps {
        if step.renamed_only {
            std::fs::rename(&step.from, &step.to).map_err(|source| Error::Io {
                path: step.from.clone(),
                source,
            })?;
            continue;
        }
        todo::write_atomic(&step.to, &step.body).map_err(|source| Error::Io {
            path: step.to.clone(),
            source,
        })?;
        std::fs::rename(&step.from, &step.backup).map_err(|source| Error::Io {
            path: step.from.clone(),
            source,
        })?;
    }
    Ok(())
}

/// How many lines of `body` look like todo.txt tasks, when the file holds no
/// markdown checkboxes at all.
///
/// Used to turn "0 of 0 tasks shown" — chiba's silent, baffling response to
/// being pointed at a todo.txt — into an actionable message. Deliberately
/// strict: a line only counts if it carries a marker prose almost never has,
/// so an ordinary markdown note full of paragraphs doesn't trip it.
pub fn unmigrated_lines(body: &str) -> usize {
    let doc = todo::parse_doc(body);
    if !doc.tasks.is_empty() {
        return 0;
    }
    doc.text.iter().filter(|l| is_todotxt_shaped(l)).count()
}

fn is_todotxt_shaped(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() || t.starts_with('#') || t.starts_with("```") || t.starts_with("~~~") {
        return false;
    }
    if todo::starts_with_priority(t) || todo::starts_with_iso_date(t) || t.starts_with("x ") {
        return true;
    }
    // A bare description line is indistinguishable from prose; only count it
    // when it carries todo.txt metadata.
    t.split_whitespace().any(|tok| {
        matches!(tok.as_bytes().first(), Some(b'+' | b'@') if tok.len() > 1)
            || matches!(tok.split_once(':'), Some((k, v))
                if !k.is_empty() && !v.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric()))
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("chiba-mig-{}-{tag}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const TXT: &str = "(A) 2026-08-01 legacy +work @home due:2026-08-06\n\
                       2026-08-02 another one\n\
                       \n\
                       x 2026-08-03 2026-08-01 finished thing +work\n";

    #[test]
    fn import_converts_the_whole_file_set() {
        let dir = tmp("set");
        std::fs::write(dir.join("todo.txt"), TXT).unwrap();
        std::fs::write(dir.join("done.txt"), "x 2026-07-01 2026-06-01 old +work\n").unwrap();
        std::fs::write(dir.join("inbox.txt"), "buy milk tomorrow\n").unwrap();

        let plan = plan(&dir, Direction::Import, false).unwrap();
        apply(&plan).unwrap();

        // The archive is migrated too — leaving it behind orphans the history.
        for stem in STEMS {
            assert!(dir.join(format!("{stem}.md")).exists(), "{stem}.md missing");
            assert!(!dir.join(format!("{stem}.txt")).exists(), "{stem}.txt left");
        }
        assert!(dir.join("todo.txt.bak").exists(), "no backup kept");
        assert!(
            std::fs::read_to_string(dir.join("todo.md"))
                .unwrap()
                .starts_with("- [ ] (A)")
        );
        // The inbox is a plain spool: renamed, never wrapped in checkboxes.
        assert_eq!(
            std::fs::read_to_string(dir.join("inbox.md")).unwrap(),
            "buy milk tomorrow\n"
        );
        assert!(!dir.join("inbox.txt.bak").exists(), "spool needs no backup");
    }

    #[test]
    fn import_round_trips_losslessly() {
        let dir = tmp("lossless");
        std::fs::write(dir.join("todo.txt"), TXT).unwrap();
        let plan = plan(&dir, Direction::Import, false).unwrap();
        apply(&plan).unwrap();
        let md = std::fs::read_to_string(dir.join("todo.md")).unwrap();
        let (back, _) = todo::to_todotxt(&md);
        assert_eq!(significant(&back), significant(TXT));
        assert_eq!(plan.tasks(), 3);
    }

    #[test]
    fn eject_reverses_and_reports_prose_it_cannot_carry() {
        let dir = tmp("eject");
        std::fs::write(
            dir.join("todo.md"),
            "# Work\n\nNotes.\n\n- [ ] (A) a +work\n- [x] 2026-08-01 2026-07-01 b\n",
        )
        .unwrap();
        let plan = plan(&dir, Direction::Eject, false).unwrap();
        assert_eq!(plan.tasks(), 2);
        assert_eq!(plan.dropped(), 2, "heading + prose counted, blanks not");
        apply(&plan).unwrap();

        let txt = std::fs::read_to_string(dir.join("todo.txt")).unwrap();
        assert_eq!(txt, "(A) a +work\nx 2026-08-01 2026-07-01 b\n");
        // The prose is gone from todo.txt but not from the disk.
        let bak = std::fs::read_to_string(dir.join("todo.md.bak")).unwrap();
        assert!(bak.contains("# Work") && bak.contains("Notes."));
        assert!(!dir.join("todo.md").exists());
    }

    #[test]
    fn import_then_eject_preserves_every_task() {
        let dir = tmp("round");
        std::fs::write(dir.join("todo.txt"), TXT).unwrap();
        apply(&plan(&dir, Direction::Import, false).unwrap()).unwrap();
        std::fs::remove_file(dir.join("todo.txt.bak")).unwrap();
        apply(&plan(&dir, Direction::Eject, false).unwrap()).unwrap();
        let out = std::fs::read_to_string(dir.join("todo.txt")).unwrap();
        assert_eq!(significant(&out), significant(TXT));
    }

    #[test]
    fn both_files_present_is_refused() {
        let dir = tmp("ambiguous");
        std::fs::write(dir.join("todo.txt"), TXT).unwrap();
        std::fs::write(dir.join("todo.md"), "- [ ] a\n").unwrap();
        assert_eq!(state(&dir), State::Ambiguous);
        assert!(matches!(
            plan(&dir, Direction::Import, false),
            Err(Error::Ambiguous)
        ));
    }

    #[test]
    fn migrating_twice_is_a_no_op_not_a_disaster() {
        let dir = tmp("idempotent");
        std::fs::write(dir.join("todo.txt"), TXT).unwrap();
        apply(&plan(&dir, Direction::Import, false).unwrap()).unwrap();
        let after = std::fs::read_to_string(dir.join("todo.md")).unwrap();
        assert!(matches!(
            plan(&dir, Direction::Import, false),
            Err(Error::NothingToDo)
        ));
        assert_eq!(std::fs::read_to_string(dir.join("todo.md")).unwrap(), after);
    }

    #[test]
    fn an_existing_backup_is_never_clobbered() {
        let dir = tmp("bak");
        std::fs::write(dir.join("todo.txt"), TXT).unwrap();
        std::fs::write(dir.join("todo.txt.bak"), "precious\n").unwrap();
        assert!(matches!(
            plan(&dir, Direction::Import, false),
            Err(Error::BackupExists(_))
        ));
        // ...and the earlier backup is still intact.
        assert_eq!(
            std::fs::read_to_string(dir.join("todo.txt.bak")).unwrap(),
            "precious\n"
        );
        assert!(plan(&dir, Direction::Import, true).is_ok(), "--force wins");
    }

    #[test]
    fn planning_writes_nothing() {
        let dir = tmp("dry");
        std::fs::write(dir.join("todo.txt"), TXT).unwrap();
        let before = std::fs::read_dir(&dir).unwrap().count();
        let _plan = plan(&dir, Direction::Import, false).unwrap();
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), before);
        assert!(!dir.join("todo.md").exists(), "--dry-run must not write");
    }

    #[test]
    fn unmigrated_lines_spots_a_todotxt_and_ignores_a_note() {
        assert_eq!(unmigrated_lines(TXT), 3);
        // A markdown file that simply has no tasks yet must not be flagged.
        let note = "# Journal\n\nToday I wrote some prose about things.\n\nAnd more.\n";
        assert_eq!(unmigrated_lines(note), 0);
        // A file that already has checkboxes is migrated by definition.
        assert_eq!(
            unmigrated_lines("- [ ] a\n(A) stray legacy line +work\n"),
            0
        );
    }
}
