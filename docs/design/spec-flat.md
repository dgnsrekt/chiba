# chiba-flat — rough spec

**Direction A.** One markdown file, one task per line, prose passes through
untouched. Fork of tuxedo at `src/todo.rs`; everything above the parser stays.

Goal: a `todo.md` that GitHub and Obsidian render as a checklist, driven by
tuxedo's TUI, without rewriting tuxedo.

Non-goal: understanding markdown. Chiba-flat *preserves* markdown it doesn't
understand. That's the whole trick.

---

## 1. File format

```markdown
# Work

- [ ] (A) 2026-08-05 Call dentist @phone +health due:2026-08-12
- [x] 2026-08-05 2026-08-01 Submit expense report +work
- [ ] Pay rent due:2026-08-15 rec:+1m t:-3d

Some prose. Chiba never touches this line.

# Home
- [ ] Water plants rec:1w
```

### Grammar

A line is a **task** iff, after stripping leading whitespace, it matches:

```
task    := bullet " " checkbox " " todotxt-body
bullet  := "-" | "*" | "+"
checkbox:= "[ ]" | "[x]" | "[X]"
```

Everything after the checkbox is parsed by the **existing** todo.txt body
grammar, unchanged:

| Token | Meaning | Source module |
|---|---|---|
| `(A)`–`(Z)` | priority | `todo.rs` |
| ISO date | creation date (or completion date, see below) | `todo.rs` |
| `+project` | project tag | `todo.rs` |
| `@context` | context tag | `todo.rs` |
| `#tag` | context tag — **new alias** for `@context` | `todo.rs` |
| `due:YYYY-MM-DD` | due date | `todo.rs`, `core/filter.rs` |
| `rec:[+]N{d,b,w,m,y}` | recurrence | `recurrence.rs` |
| `t:-3d` \| `t:YYYY-MM-DD` | threshold | `threshold.rs` |
| `note:<path>` | linked note | `note.rs` |

Keeping the metadata tokens todo.txt-shaped is deliberate laziness: `nl.rs`
(1401 lines), `recurrence.rs`, `threshold.rs`, `core/filter.rs`,
`app/visibility.rs`, and `app/autocomplete.rs` all keep working with zero edits.
Only completion state and the bullet are markdown-native.

`#tag` as a context alias is the one syntax addition — it's what people actually
type in markdown. Parsed into the same `contexts: Vec<String>`. Round-trips as
whatever the user wrote; no normalization.

### Completion

Markdown owns the state, todo.txt owns the dates:

```
- [ ] 2026-08-01 Submit expense report +work
- [x] 2026-08-05 2026-08-01 Submit expense report +work
```

`[x]` is the completion marker. The leading `x ` from todo.txt is **gone** —
when `[x]` is set, the first ISO date is the completion date and the second is
the creation date, matching todo.txt's ordering so `mark_done`/`unmark_done`
logic survives with the prefix handling swapped for checkbox flipping.

### Everything else

Any line that is not a task — heading, prose, blank, code fence, nested list,
front matter — is a **passthrough**. Stored verbatim, written back byte-for-byte,
never reordered relative to its neighbours, invisible in the TUI.

Indentation on a task line is captured and preserved but carries **no meaning**.
`  - [ ] subtask` is a task with two spaces of indent, not a child. (That's
direction B.)

## 2. Data model

The one structural change:

```rust
// src/todo.rs
pub struct Doc {
    pub tasks: Vec<Task>,
    pub text: Text,
}

/// Non-task lines, each anchored to the task ordinal it precedes.
pub struct Text { lines: Vec<(usize, String)> }

impl Text {
    pub fn on_insert(&mut self, idx: usize);  // anchors >= idx move up
    pub fn on_remove(&mut self, idx: usize);  // anchors >  idx move down
}
```

Tasks and text live in parallel rather than interleaved in one `Vec<Line>`.
That's what keeps `Store.tasks` a plain `Vec<Task>` — every inherited call site
that indexes it (filters, visibility, the CLI's 1-based ordinals) compiles
untouched.

`Task` gains two fields and keeps the rest:

```rust
pub struct Task {
    pub indent: String,      // NEW — leading whitespace, verbatim
    pub bullet: char,        // NEW — '-' | '*' | '+', preserved per-line
    pub raw: String,         // unchanged: the todo.txt body after the checkbox
    pub clean_raw: String,
    pub done: bool,
    pub done_date: Option<String>,
    pub priority: Option<char>,
    pub created_date: Option<String>,
    pub projects: Vec<String>,
    pub contexts: Vec<String>,
    pub due: Option<String>,
    pub rec: Option<String>,
    pub threshold: Option<String>,
    pub notes: Vec<String>,
}
```

Critically, `raw` still holds *only the todo.txt body*. Every existing string
surgery in `Task::mark_done`, `set_priority`, `body_after_priority`,
`strip_priority`, `body_only` operates on the same substring it always did. The
markdown wrapper (`indent`, `bullet`, checkbox) is re-emitted at serialize time.

That's the design's load-bearing decision: **the wrapper never enters `raw`.**
It's why 80 `.raw` sites across 18 files don't need auditing.

### Parse / serialize

```rust
pub fn parse_doc(s: &str) -> Doc                          // no line is ever dropped
pub fn serialize_doc(tasks: &[Task], text: &Text) -> String  // byte-identical when untouched
```

`parse_line` keeps its signature for the body and gains a thin
`parse_md_line(&str) -> Option<Task>` wrapper above it.

## 3. Anchoring and task numbering

Tuxedo's CLI uses 1-based task ordinals, stable across filter and sort. Keep
that exactly — ordinals count **tasks only**, skipping passthroughs. `chiba do 3`
is the third checkbox in the file regardless of how many headings sit between
them.

**Passthrough lines are anchored to the task ordinal they precede**, not to an
absolute line index. Anchor `n` means "emit before task `n`"; `n == tasks.len()`
is the document's tail.

> **This is a correction.** The first draft of this spec pinned passthroughs to
> absolute output positions, and the implementation followed it. That model
> loses data: deleting a task shrinks the total line count while the pinned
> indices stay put, so tasks slide up past the headings that belonged above
> them. Deleting the first task of a document moved a `# Heading` to the bottom
> of the file and promoted the following task above it. Silent, and it fired on
> every delete with text later in the file.
>
> Position-pinning isn't a quirk you document — it's a model that reorders the
> user's document. Ordinal anchoring is the fix, and it's barely more code.

The two shift rules, and why they differ:

- **Insert at `p`** — anchors `>= p` move up. Text belonging to the task that
  got pushed down travels with it, which is what puts a recurrence successor
  under its parent's heading instead of the next section's.
- **Remove at `p`** — anchors `> p` move down. Text anchored *to* the removed
  task stays where it is and now precedes whatever follows, so an emptied
  section keeps its heading.

Every length-changing mutation goes through `Store::task_insert` /
`task_remove` / `task_push` so anchors can't drift. Undo snapshots
`(Vec<Task>, Text)` — restoring tasks alone would leave anchors shifted.

Appends are the deliberate exception: `task_push` touches no anchors, so a new
task lands at the end of the file rather than before its trailing prose.

Sorting still permutes tasks only, so a sorted view can show tasks in an order
their headings don't imply. That one *is* just a quirk — it doesn't move
anything on disk, because sort is a view. Heading-aware ordering is direction B.

## 4. Archive

`done.txt` → `done.md`. Same sibling-file mechanism, same `$DONE_FILE`
override, same atomic move, same archive browser (`a`) and un-archive.
Archived lines keep their `- [x]` form so `done.md` renders too.

**`done.md` is a markdown document too.** It gets its own `Text`, parsed with
`parse_doc` and written with `serialize_doc`, exactly like the todo file — the
first cut rebuilt it from tasks alone, which ate any heading or prose the user
had put there on the first un-archive or archive-delete. Appending on archive
was already safe (it concatenates onto the raw previous body); the rewrite
paths were not.

Passthroughs are never *moved into* the archive — archiving a task carries the
task only.

## 5. File resolution & config

Mirror tuxedo, retargeted:

1. Explicit `FILE` argument
2. `$CHIBA_FILE`, then `$TODO_FILE` (compat)
3. `$CHIBA_DIR/todo.md`, then `$TODO_DIR/todo.md`
4. `./todo.md`
5. First-run prompt (TUI only)

Archive: `$DONE_FILE`, else sibling `done.md`. Inbox: sibling `inbox.md`.
Config path moves to `~/.config/chiba/`; theme and keybind formats unchanged.

**Migration:** `chiba import todo.txt` reads a todo.txt file and writes
`todo.md`. One function, wraps each line in `- [ ]`/`- [x]` and moves the
completion marker. `chiba export` goes back the other way and is lossy only for
passthroughs (which it drops, with a count printed to stderr).

## 6. Module-by-module change list

| Module | LOC | Change |
|---|---|---|
| `todo.rs` | 636 | `Document`/`Line`, `parse_md_line`, serialize, checkbox in `mark_done`/`unmark_done`, `#tag` alias | 
| `core/mutations.rs` | 721 | operate on `Document` not `Vec<Task>`; ordinal→line mapping |
| `core/archive.rs` | 514 | `done.md`, skip passthroughs |
| `core/external.rs` | 328 | reload produces `Document` |
| `ui/task_row.rs` | 549 | render checkbox segment; priority/date/tag rendering unchanged |
| `inbox.rs` | 371 | drained lines get wrapped as `- [ ]` before merge |
| `cmd/mod.rs` | 770 | ordinals resolve through `Document`; `import`/`export` subcommands |
| `cmd/json.rs` | — | add `indent`/`bullet` to the JSON shape |
| `config.rs` | 449 | new paths, `todo.md` defaults |
| `note.rs` | 254 | note template path defaults; `note:` parsing unchanged |
| `main.rs` | 1803 | wiring, first-run prompt copy |
| `tests/snapshots.rs` | 515 | regenerate |
| `sample.rs` | — | rewrite the sample file in markdown |

Untouched: `nl.rs`, `recurrence.rs`, `threshold.rs`, `search.rs`, `theme.rs`,
`keybinds.rs`, `serve/*`, `app/draft*.rs`, `app/autocomplete.rs`,
`app/visibility.rs`, `core/filter.rs`, `ui/*` except `task_row.rs`.

Estimate: ~600 lines changed, ~1500 lines of snapshot churn.

## 7. Tests

Upstream has 469 tests. Most survive because the body grammar is unchanged.

New tests that must exist before anything else:

1. **Round-trip fidelity** — parse a markdown file with headings, prose, code
   fences, front matter, nested lists, CRLF, and trailing whitespace; serialize;
   assert byte-identical output. This is the test that justifies the fork.
2. **Mutation locality** — complete one task in a 200-line document; assert every
   other line is byte-identical.
3. **Ordinal stability** — task ordinals ignore passthroughs, survive filtering.
4. **Import/export** — todo.txt → todo.md → todo.txt is identity.
5. **Anchoring under mutation** — delete, insert, append and undo against a
   document with headings, and assert the sections still hold their tasks.
   Sabotage-test these: break the shift rule and confirm they fail.
6. **Recurrence through the wrapper** — `- [ ] Pay rent due:2026-08-15 rec:+1m`
   completes to `- [x] …` plus a fresh `- [ ] … due:2026-09-15 rec:+1m`, and `u`
   undoes both.

## 8. Open questions

- **Priority rendering.** `(A)` is kept as-is (zero parser work, ugly in
  rendered markdown). Obsidian Tasks uses 🔺⏫🔼🔽⏬. Config flag later, or never.
- **`* ` vs `- ` bullets.** Preserved per-line on parse; which one do *new* tasks
  use? Config `bullet = "-"`, default `-`.
- **Front matter.** Falls out as passthrough for free. Do we ever want to read
  it (e.g. a `default_project:` key)? Not in v1.
- **Nested tasks.** Indent is preserved but meaningless. People will nest anyway
  and expect it to work. This is the pressure that eventually forces direction B
  — accept it or say no loudly in the README.
- **`#tag` vs prose.** A `#` tag must start with a letter, or every `fix #1234`
  and `PR #99` becomes a junk context in the sidebar and autocomplete. `@`
  keeps todo.txt's permissive rule for compatibility.
- **Wikilinks.** `[[note]]` in a task body currently parses as ordinary text.
  Probably fine. `note:` already covers the linking use case.
