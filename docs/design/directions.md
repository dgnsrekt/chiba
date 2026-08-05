# Design directions

The decision record behind chiba: what the two possible forks of
[tuxedo](https://github.com/webstonehq/tuxedo) looked like, and why A was
built. Written before any code existed — kept for the reasoning, not as
current documentation of behaviour.

Tuxedo is the mask. Chiba is who's underneath: same person, different form.
(Mamoru Chiba, 千葉 — "thousand leaves", which is also a vault of markdown files.)

Upstream is MIT, so this is a clean fork. Status at the time of writing: **spec only, no code yet.** Direction A shipped.

---

## Why fork at all

Tuxedo does one format and only one format: todo.txt. Nothing else — no SQLite,
no markdown, no Taskwarrior backend, and nothing on its roadmap. All 75 forks on
GitHub as of 2026-08-05 are still todo.txt; the only two that diverge
meaningfully ([wolffness/prumo](https://github.com/wolffness/prumo) +92 commits,
[Paradem/tuxedo](https://github.com/Paradem/tuxedo) +20) added a kanban view and
editor fixes respectively. Nobody has done markdown.

The nearest existing thing is a separate project,
[vault-tasks](https://github.com/louis-thevenet/vault-tasks) — a markdown TUI task
manager that already solved the hard part (the document model) but lacks
recurrence, thresholds, and a CLI surface.

## What we'd be keeping

Upstream, as of the fork point:

| | |
|---|---|
| Language | Rust — ratatui 0.30, crossterm 0.29, notify 8, chrono, tiny_http, qrcode |
| Size | ~23k LOC across 67 files, 469 tests (21 of them snapshot tests) |
| License | MIT, © 2026 Webstone Technologies Inc |

Worth inheriting and *not* rewriting: `nl.rs` (1401 lines of local rule-based
natural-language parsing — "every other friday show 1 day before"),
`recurrence.rs`, `threshold.rs`, the whole `ui/` tree, `serve/` (phone capture
PWA + QR), `theme.rs`, `keybinds.rs`, the command palette, and the CLI.

## The one thing that makes this hard

Tuxedo's document model is *the file **is** a flat list of tasks*:

```rust
// src/todo.rs:221
pub fn parse_file(s: &str) -> Vec<Task> {
    s.lines().filter_map(|line| parse_line(line).ok()).collect()
}

pub fn serialize(tasks: &[Task]) -> String {
    // joins t.raw with '\n' — nothing else survives
}
```

Point that at a real markdown file and the round-trip eats your document.
Headings, prose, and code fences either fail to parse and get silently dropped by
`filter_map(...ok())`, or parse as garbage tasks. The first write destroys the
file.

Everything else follows from this. `Task` carries `raw: String` as its source of
truth — mutations are string surgery on `raw` followed by a re-parse
(`replace_from_raw`), including byte-offset tricks like `after_x[11..]` to skip a
done-date. That `raw` field is touched at **80 sites across 18 files**, and ~14
files carry line-shape assumptions beyond the parser (`core/mutations.rs`,
`ui/task_row.rs`, `core/archive.rs`, `note.rs`, `inbox.rs`).

The token syntax is the easy part. The document model is the fork.

## Two directions

Both are specced in full ([flat](./spec-flat.md), [vault](./spec-vault.md)).
They are different projects wearing the same name.

### A — [flat](./spec-flat.md) ← built: one `todo.md`, prose passes through

One file, one task per line, `- [ ]` / `- [x]` checkboxes. Non-task lines are
carried verbatim as passthrough so headings and prose survive the round-trip,
but they're inert — chiba doesn't understand them, it just doesn't eat them.
Metadata tokens (`due:`, `rec:`, `t:`, `+project`, `@context`) stay
todo.txt-shaped, which means `nl.rs`, `recurrence.rs`, `threshold.rs`, and the
entire filter stack keep working untouched.

**Effort:** ~600-line diff, concentrated in `todo.rs` plus snapshot updates.
A weekend for a working binary.
**You get:** 95% of tuxedo, in a file GitHub and Obsidian render as checkboxes.
**You don't get:** multi-file vaults, real subtask semantics, heading awareness.

### B — [vault](./spec-vault.md): a folder of markdown, fully understood

Tasks are positions inside documents — indent depth, parent task, owning
heading, surrounded by text you must not touch. `Vec<Task>` becomes
`Vec<Document>`. Line-number addressing dies (you can't reorder a task out of
its heading), so the CLI needs stable IDs. Archiving, sorting, and bulk ops all
need redesigning against a tree.

**Effort:** core rewrite. Weeks, not days.
**You get:** what vault-tasks already is, plus tuxedo's UI and NL parser.
**You don't get:** a quick win.

## Recommendation

**Build A.** It's the honest weekend project and it produces something you'd
actually use tomorrow.

If you want B, port the other direction — add `rec:`, `t:`, and a CLI to
vault-tasks. Adding three features to a correct document model beats retrofitting
a correct document model into 18 files that assume flat lines.

### If B happens later

A ships on `main`. B, if it ever happens, is chiba 2.0 on a `vault` branch — and
when it ships, **A is deleted, not maintained**. B's document model strictly
subsumes A's (a flat `todo.md` is a vault of one file with `depth` ignored), so
there's no scenario where both stay alive. A long-lived parallel branch only
pays merge interest.

The code isn't an upgrade path; the **file format is**. A `todo.md` written by
chiba-flat parses cleanly as chiba-vault. Users lose nothing across the
migration — you throw away code, not data. That's what makes reusing the name
honest.

Three hedges to build into A now. Each is nearly free today and annoying to
retrofit:

1. **Fence-aware parsing** (~20 lines) — never treat `- [ ]` inside a code fence
   as a task. B requires it; A corrupts code blocks without it.
2. **Sort is a view, not a mutation.** A *can* permute lines on sort; B can't,
   because tasks belong to headings. Don't teach the muscle memory B has to take
   away.
3. **`archive_mode = in_place` supported in A**, even if `done.md` stays the
   default. It's B's default, it's a filter rather than a move, and it's what
   markdown users already do.

Don't create the `vault` branch until you're writing it. `spec-vault.md` is the
placeholder.
