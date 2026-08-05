# chiba-vault — rough spec

**Direction B.** A folder of markdown files, fully understood: nesting,
headings, multiple documents, prose preserved. Tuxedo's UI and NL parser on top
of a real document model.

This is not a bigger version of [spec-flat](./spec-flat.md). It replaces the
core. Read the trade-off in the [README](./README.md) before starting.

---

## 1. What changes conceptually

Tuxedo: **a file is a list of tasks.** Task identity = position in that list.
Chiba-vault: **a task is a position inside a document.** Identity = where it
lives, and where it lives has meaning.

Consequences, all of which cost real work:

- A task has a **parent** (the task it's indented under) and an **owning
  heading**. Both are structure, not decoration.
- Tasks cannot be freely reordered. Sorting by due date across a vault is a
  *view*, not a mutation of the files.
- Line numbers are useless as identifiers. `chiba do 3` needs a different
  addressing scheme.
- Completion may cascade (parent done → children done?) or block (children open
  → parent can't close?). Somebody has to decide.
- Archiving can't mean "move to a sibling file" without ripping a subtree out of
  its context.

## 2. Vault layout

```
vault/
├── work.md
├── home.md
├── projects/
│   └── kitchen-remodel.md
└── archive/          # optional, config
```

Every `.md` file under the vault root is scanned. Ignore rules: `.chibaignore`
(gitignore syntax), plus a default skip of `.git/`, `node_modules/`,
`.obsidian/`.

## 3. Document model

```rust
pub enum Block {
    Heading { level: u8, text: String, raw: String },
    Task(Task),
    Text(String),        // prose, blank lines, anything else
    Fence(Vec<String>),  // code fence — parsed as an opaque unit so `- [ ]`
                         // inside a code block is never treated as a task
}

pub struct Document {
    pub path: PathBuf,
    pub blocks: Vec<Block>,
    pub front_matter: Option<String>,
    pub mtime: SystemTime,
}

pub struct Vault {
    pub root: PathBuf,
    pub docs: Vec<Document>,
}
```

`Task` extends the flat-spec shape with structure:

```rust
pub struct Task {
    // --- position ---
    pub doc: DocId,
    pub block_idx: usize,
    pub depth: u8,              // indent level, semantic
    pub parent: Option<TaskId>,
    pub children: Vec<TaskId>,
    pub heading_path: Vec<String>,   // ["Work", "Q3", "Launch"]

    // --- identity ---
    pub id: TaskId,             // see §4

    // --- content: unchanged from tuxedo ---
    pub raw: String,            // todo.txt body after the checkbox
    pub done: bool,
    pub priority: Option<char>,
    pub due: Option<String>,
    pub rec: Option<String>,
    pub threshold: Option<String>,
    pub projects: Vec<String>,
    pub contexts: Vec<String>,
    // ...
}
```

**Implicit projects.** `heading_path` contributes to `projects` at query time —
a task under `# Work` is in project `work` without typing `+work`. Explicit
`+project` tags still work and stack. `projects_effective()` returns the union;
`projects` stays literal so round-trips are clean.

### Fences matter

A `- [ ] not a task` inside a ```` ``` ```` block must not become a task. Flat
can get away with ignoring this (the file is yours); a vault of real notes
cannot. Parse fences as opaque blocks before scanning for tasks.

## 4. Task identity

Line numbers die. Three candidates:

| Scheme | Stable across edits | Visible in file | Verdict |
|---|---|---|---|
| `path:line` | no — external edits shift it | no | fine for a single TUI session, wrong for the CLI |
| Content hash | no — editing the task changes it | no | no |
| `id:` key in the line | yes | yes, ugly | **pick this** |

Spec: **lazy IDs.** No `id:` token is written until something needs to address
the task across a process boundary — i.e. the first time the CLI or the phone
inbox references it. Short base32, 6 chars: `id:k3f9qa`. Inside a single TUI
session, `path:line` is sufficient and nothing is written.

CLI addressing becomes `chiba do k3f9qa`, with ordinals (`chiba do 3`) still
accepted as "the 3rd task in the current default view" for interactive use.

## 5. Mutations

Every mutation is (a) local to one line, or (b) an explicit structural move.

**Local** — complete, priority, edit body, add/remove tags, set due. These
rewrite one line in one document and leave the rest byte-identical. Same
approach as flat: string surgery on `raw`, wrapper re-emitted.

**Structural** — indent/outdent (change parent), move to another heading, move
to another file. These splice a subtree: the task plus all its children, with
indent rebased. New keybinds; no upstream equivalent.

**Forbidden** — free reordering. `sort by due` is a view transform only. The
list view must make it obvious that on-screen order ≠ file order (a mode
indicator in the status bar).

### Completion semantics

Config, default in **bold**:

- `parent_completion = "cascade" | "block" | **"independent"**`
  - `cascade` — completing a parent completes all descendants
  - `block` — a parent can't be completed while children are open
  - `independent` — no relationship (start here; it surprises nobody)

Recurrence on a parent with children: spawn the whole subtree fresh, children
reset to `[ ]`. Non-obvious, needs a test.

## 6. Archive

`done.md` is wrong here — pulling a subtree out of its heading loses the context
that made it meaningful. Three modes, config `archive_mode`:

- **`in_place`** (default) — completed tasks stay where they are, hidden from
  the default view. Archiving is a filter, not a move. Zero data movement, zero
  risk, and it's what markdown users already do.
- `section` — move to a `## Done` heading at the bottom of the *same* file,
  preserving subtree shape.
- `file` — move to `archive/<original-name>.md`, prepending the source
  `heading_path` as a heading so context survives.

`core/archive.rs` (514 lines, built entirely around the two-file model) is a
rewrite in all three cases.

## 7. Indexing & watching

The vault can be thousands of files. Tuxedo re-reads one file every 250ms while
idle; that doesn't scale.

- Startup: walk the vault, parse every `.md`, build an in-memory index
  (task → doc, tag → tasks, due-bucket → tasks). Target: 5k tasks across 500
  files in under 300ms.
- Steady state: `notify` (already a dependency) watches the root. On a change
  event, reparse **only** the touched document and patch the index.
- Writes stay atomic per file (temp + rename), as upstream.
- Persistent cache (`~/.cache/chiba/index.bin` keyed on path+mtime+size) only if
  cold start measures too slow. Probably not needed. Don't build it first.

## 8. What survives from tuxedo

Unchanged: `nl.rs` (1401 lines of NL parsing), `recurrence.rs`, `threshold.rs`,
`theme.rs`, `keybinds.rs`, `serve/*` (phone capture + QR), most of `ui/`,
`search.rs`.

Rewritten: `todo.rs`, `core/mutations.rs`, `core/archive.rs`, `core/external.rs`,
`core/filter.rs` (now multi-doc), `app/mutations.rs`, `app/bulk.rs`,
`app/visibility.rs`, `cmd/mod.rs` (addressing), `cmd/json.rs`.

New: vault walker, index, fence-aware parser, subtree splice, ID allocator,
document/heading navigation UI.

Rough shape: ~8k lines rewritten or new, ~6k inherited intact. The 469 upstream
tests split roughly half survive / half rewritten.

## 9. New UI surface

Beyond upstream, the vault needs:

- **File tree pane** (toggle) — vault navigation, task counts per file
- **Heading breadcrumb** in the detail view — where does this task live
- **Tree/flat toggle** in the list — see nesting, or see a flat due-sorted view
- **Indent/outdent** keys — `>` / `<`, subtree-aware
- **Move to file/heading** — a picker, reusing `app/picker.rs`
- **Order indicator** — status bar shows when the view order ≠ file order

## 10. Do this before writing any of it

Read [vault-tasks](https://github.com/louis-thevenet/vault-tasks) (Rust,
ratatui, 86★, active as of June 2026). It already implements §2, §3, §5, and §7.
It is missing `rec:`, `t:`, the CLI surface, and tuxedo's NL parser.

Honest assessment: porting three features **into** vault-tasks is a smaller,
lower-risk job than the ~8k-line rewrite above. Chiba-vault only makes sense if
you specifically want tuxedo's UI, its NL parser, and its phone-capture flow —
and are willing to pay weeks for them.

If you can't articulate why vault-tasks isn't enough, build
[spec-flat](./spec-flat.md) instead.

## 11. Open questions

- Do implicit heading-projects pollute the project picker with every heading in
  the vault? Probably. Needs a depth limit or an opt-in.
- What happens when an external editor reorders a file mid-session and the TUI
  holds a selection? Reparse + re-resolve by `id:`, fall back to nearest line.
- Inbox drain: which file do captured tasks land in? Config `inbox_target`,
  default a dedicated `inbox.md` at the vault root.
- Does `chiba` work on a single file too, or is the vault mandatory? Single-file
  should degrade gracefully — a vault of one.
- Obsidian Tasks plugin compatibility (emoji metadata: 📅 due, 🔁 recur,
  ⏫ priority). A parse-only compat layer is plausible; two-way is a tarpit.
