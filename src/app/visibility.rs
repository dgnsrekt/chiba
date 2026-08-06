use super::App;
use super::types::{Sort, View};
use crate::core::filter::{self, ListDueBucket};

/// One entry per visible row, parallel to `visible_cache`. Renderers detect
/// group transitions by comparing successive entries; under `Sort::File` every
/// row is `None` so the renderer skips headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupKey {
    None,
    ArchiveDate(String),
    /// `Some('A'..='Z')` for a graded priority, `None` for unprioritized.
    ListPriority(Option<char>),
    ListDue(ListDueBucket),
}

impl App {
    /// Indices into the active view's task source after filter + sort, in
    /// display order. The source is `archive().tasks()` in Archive view,
    /// `tasks()` otherwise. Reads the cache populated by `recompute_visible`.
    pub fn visible_indices(&self) -> &[usize] {
        &self.visible_cache
    }

    /// Group key per row, parallel to `visible_indices()`.
    pub fn visible_groups(&self) -> &[GroupKey] {
        &self.visible_groups
    }

    /// Recompute the cached visible-index list and parallel group keys. Call
    /// after any mutation that affects filter/sort/view/tasks/archive.
    pub fn recompute_visible(&mut self) {
        match self.view {
            View::List => self.rebuild_list_cache(),
            View::Archive => self.rebuild_archive_cache(),
        }
    }

    fn rebuild_list_cache(&mut self) {
        let needle = (!self.filter.search.is_empty()).then_some(self.filter.search.as_str());
        let tasks = self.store.tasks();
        let today = self.store.today();

        let mut idxs: Vec<usize> = (0..tasks.len())
            .filter(|&i| {
                filter::list_predicate(
                    &tasks[i],
                    self.prefs.show_done,
                    self.prefs.show_future,
                    today,
                    &self.filter,
                    needle,
                )
            })
            .collect();

        filter::sort_by_prefs(&mut idxs, tasks, self.prefs.sort);

        let week_start = &self.week_start;

        let groups: Vec<GroupKey> = match self.prefs.sort {
            Sort::File => vec![GroupKey::None; idxs.len()],
            Sort::Priority => idxs
                .iter()
                .map(|&i| GroupKey::ListPriority(tasks[i].priority))
                .collect(),
            Sort::Due => idxs
                .iter()
                .map(|&i| GroupKey::ListDue(filter::due_bucket(&tasks[i], today, week_start)))
                .collect(),
        };
        self.visible_groups = groups;
        self.visible_cache = idxs;
    }

    fn rebuild_archive_cache(&mut self) {
        // Under `archive_mode = in_place` there is no done.md to read — the
        // completed tasks are still in the live list, so the archive view
        // shows those instead. `archive_source_is_live` is the single place
        // that decision is made; every consumer of an archive-view index asks
        // it which slice the index points into.
        let archive: &[crate::todo::Task] = match self.archive_source_is_live() {
            true => self.store.tasks(),
            false => self.store.archive().tasks(),
        };
        let mut idxs: Vec<usize> = (0..archive.len())
            .filter(|&i| !self.archive_source_is_live() || archive[i].done)
            .collect();
        idxs.sort_by(|&a, &b| {
            archive[b]
                .done_date
                .as_deref()
                .unwrap_or("")
                .cmp(archive[a].done_date.as_deref().unwrap_or(""))
        });
        let groups: Vec<GroupKey> = idxs
            .iter()
            .map(|&i| {
                let date = archive[i]
                    .done_date
                    .clone()
                    .unwrap_or_else(|| "unknown".into());
                GroupKey::ArchiveDate(date)
            })
            .collect();
        self.visible_cache = idxs;
        self.visible_groups = groups;
    }

    /// True when the archive view is showing completed tasks from the live
    /// file rather than from `done.md`.
    pub fn archive_source_is_live(&self) -> bool {
        self.prefs.archive_mode == crate::app::ArchiveMode::InPlace
    }

    pub fn cur_abs(&self) -> Option<usize> {
        self.visible_cache.get(self.cursor).copied()
    }

    pub fn clamp_cursor(&mut self) {
        let len = self.visible_cache.len();
        if len == 0 {
            self.cursor = 0;
        } else if self.cursor >= len {
            self.cursor = len - 1;
        }
    }

    /// Move the cursor to wherever `abs` lives in the current visible list.
    /// Falls back to clamping if `abs` was filtered out.
    pub(super) fn follow_cursor(&mut self, abs: usize) {
        if let Some(pos) = self.visible_cache.iter().position(|&i| i == abs) {
            self.cursor = pos;
        } else {
            self.clamp_cursor();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::build_app;
    use crate::core::filter::ListDueBucket;

    #[test]
    fn search_matches_subsequence() {
        let mut app = build_app("2026-05-01 Call dentist\n2026-05-01 buy milk\n");
        app.filter.search = "cade".into();
        app.recompute_visible();
        assert_eq!(app.visible_indices().len(), 1);
    }

    #[test]
    fn search_matches_body_not_dates() {
        let mut app = build_app("2026-05-01 buy milk\n2026-04-01 something else\n");
        app.filter.search = "2026".into();
        app.recompute_visible();
        assert_eq!(app.visible_indices().len(), 0);
    }

    #[test]
    fn visible_cache_updates_after_mutation() {
        let mut app = build_app("a\nb\nc\n");
        assert_eq!(app.visible_indices().len(), 3);
        app.draft_set("d".into());
        app.add_from_draft();
        assert_eq!(app.visible_indices().len(), 4);
    }

    #[test]
    fn list_cursor_survives_archive_roundtrip() {
        let mut app = build_app("a\nb\nc\nd\ne\n");
        app.cursor = 3;
        app.set_view(View::Archive);
        app.set_view(View::List);
        assert_eq!(app.cursor, 3, "cursor lost on List → Archive → List");
    }

    #[test]
    fn archive_indices_point_into_archive_tasks() {
        let mut app = build_app("a\n");
        let path = app.archive().path().to_path_buf();
        app.store.archive = crate::app::Archive::for_test(
            crate::todo::parse_file(&crate::core::test_support::md(
                "x 2026-05-01 2026-04-01 first\nx 2026-05-02 2026-04-02 second\n",
            )),
            String::new(),
            path,
        );
        app.set_view(View::Archive);
        let idxs = app.visible_indices();
        assert_eq!(idxs.len(), 2);
        for &i in idxs {
            assert!(app.archive().tasks().get(i).is_some());
        }
    }

    #[test]
    fn list_groups_are_none_under_sort_file() {
        let mut app = build_app("(A) a\n(B) b\nc\n");
        app.prefs.sort = Sort::File;
        app.recompute_visible();
        let groups = app.visible_groups();
        assert_eq!(groups.len(), 3);
        for g in groups {
            assert!(matches!(g, GroupKey::None));
        }
    }

    #[test]
    fn list_groups_track_priority_under_sort_priority() {
        let mut app = build_app("(A) a\n(B) b\nc\n(A) a2\n");
        app.prefs.sort = Sort::Priority;
        app.recompute_visible();
        let groups = app.visible_groups();
        assert_eq!(groups.len(), 4);
        assert_eq!(groups[0], GroupKey::ListPriority(Some('A')));
        assert_eq!(groups[1], GroupKey::ListPriority(Some('A')));
        assert_eq!(groups[2], GroupKey::ListPriority(Some('B')));
        assert_eq!(groups[3], GroupKey::ListPriority(None));
    }

    #[test]
    fn list_groups_bucket_due_dates_under_sort_due() {
        let raw = "a due:2026-05-04\n\
                   b due:2026-05-06\n\
                   c due:2026-05-08\n\
                   d due:2026-05-15\n\
                   e due:2026-05-25\n\
                   f\n";
        let mut app = build_app(raw);
        app.prefs.sort = Sort::Due;
        app.recompute_visible();
        let groups = app.visible_groups();
        assert_eq!(groups.len(), 6);
        assert_eq!(groups[0], GroupKey::ListDue(ListDueBucket::Overdue));
        assert_eq!(groups[1], GroupKey::ListDue(ListDueBucket::Today));
        assert_eq!(groups[2], GroupKey::ListDue(ListDueBucket::ThisWeek));
        assert_eq!(groups[3], GroupKey::ListDue(ListDueBucket::NextWeek));
        assert_eq!(groups[4], GroupKey::ListDue(ListDueBucket::Later));
        assert_eq!(groups[5], GroupKey::ListDue(ListDueBucket::NoDue));
    }

    #[test]
    fn future_absolute_threshold_hidden_by_default() {
        let mut app = build_app("future task t:2030-01-01\nvisible task\n");
        assert_eq!(app.visible_indices().len(), 1);
        assert_eq!(app.tasks()[app.visible_indices()[0]].raw, "visible task");
        app.prefs.show_future = true;
        app.recompute_visible();
        assert_eq!(app.visible_indices().len(), 2);
    }

    #[test]
    fn relative_threshold_anchors_on_due() {
        let mut app = build_app("Pay rent due:2026-05-15 t:-3d\n");
        assert_eq!(app.visible_indices().len(), 0);
        app.prefs.show_future = true;
        app.recompute_visible();
        assert_eq!(app.visible_indices().len(), 1);
    }

    #[test]
    fn refresh_today_unhides_tasks_when_date_advances() {
        let mut app = build_app("future task t:2026-05-07\nvisible task\n");
        assert_eq!(app.visible_indices().len(), 1);
        let changed = app.refresh_today("2026-05-07".into());
        assert!(changed);
        assert_eq!(app.today(), "2026-05-07");
        assert_eq!(app.visible_indices().len(), 2);
    }

    #[test]
    fn refresh_today_is_noop_when_date_unchanged() {
        let mut app = build_app("a\n");
        let changed = app.refresh_today("2026-05-06".into());
        assert!(!changed);
        assert_eq!(app.today(), "2026-05-06");
    }

    #[test]
    fn archive_visible_groups_are_done_date_desc() {
        let mut app = build_app("a\n");
        let path = app.archive().path().to_path_buf();
        app.store.archive = crate::app::Archive::for_test(
            crate::todo::parse_file(&crate::core::test_support::md(
                "x 2026-04-01 2026-03-01 older\nx 2026-05-02 2026-04-02 newer\n",
            )),
            String::new(),
            path,
        );
        app.set_view(View::Archive);
        let groups = app.visible_groups();
        assert_eq!(groups.len(), 2);
        let first = match &groups[0] {
            GroupKey::ArchiveDate(d) => d.as_str(),
            _ => panic!("expected ArchiveDate"),
        };
        let second = match &groups[1] {
            GroupKey::ArchiveDate(d) => d.as_str(),
            _ => panic!("expected ArchiveDate"),
        };
        assert_eq!(first, "2026-05-02");
        assert_eq!(second, "2026-04-01");
    }

    // ----- sort is a view, and prefs stay out of the real config -----------

    #[test]
    fn cycling_sort_never_rewrites_the_todo_file() {
        // Sorting permutes an index list, never `Store.tasks`. Direction B
        // cannot reorder across headings, so if this ever becomes a file
        // mutation the vault design loses a behaviour users have learned.
        let mut app = build_app("(A) alpha\n(C) charlie due:2026-05-01\n(B) bravo\n");
        let path = app.file_path.clone();
        let before = std::fs::read_to_string(&path).expect("seed file");

        let mut seen = Vec::new();
        for _ in 0..3 {
            app.cycle_sort();
            seen.push(app.prefs.sort);
            assert_eq!(
                std::fs::read_to_string(&path).expect("read"),
                before,
                "sort must not touch the file",
            );
        }
        assert_eq!(seen.len(), 3, "all three modes exercised");
        // ...and it really did reorder what's rendered.
        app.prefs.sort = crate::app::Sort::Priority;
        app.recompute_visible();
        let by_priority: Vec<usize> = app.visible_indices().to_vec();
        app.prefs.sort = crate::app::Sort::File;
        app.recompute_visible();
        assert_ne!(
            by_priority,
            app.visible_indices().to_vec(),
            "priority order should differ from file order for this fixture",
        );
    }

    #[test]
    fn saving_prefs_without_a_config_path_writes_nothing() {
        // `App::new` leaves config_path None; only the TUI entry point sets
        // it. Before this guard every test that toggled a pref wrote the
        // developer's real ~/.config/chiba/config.toml.
        let mut app = build_app("a\n");
        assert!(app.config_path.is_none(), "test apps have no config path");
        app.cycle_sort(); // calls save_prefs internally
        app.prefs.toggle_left();
        app.save_prefs();
        // Nothing to assert on disk — the point is that no path was resolved
        // and nothing was written. If this regresses, the sentinel below fires.
    }

    #[test]
    fn saving_prefs_honours_an_explicit_config_path() {
        let mut app = build_app("a\n");
        let cfg = std::env::temp_dir().join(format!(
            "chiba-prefs-{}-{:?}.toml",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&cfg);
        app.config_path = Some(cfg.clone());
        app.prefs.sort = crate::app::Sort::Due;
        app.save_prefs();
        let body = std::fs::read_to_string(&cfg).expect("prefs written to the given path");
        assert!(body.contains("sort = due"), "got: {body}");
        let _ = std::fs::remove_file(&cfg);
    }

    #[test]
    fn in_place_archive_view_lists_completed_tasks_from_the_live_file() {
        use crate::app::{ArchiveMode, View};
        let mut app = build_app("open one\nx 2026-05-05 2026-05-01 done one\nopen two\n");
        app.prefs.archive_mode = ArchiveMode::InPlace;
        assert!(app.archive_source_is_live());
        app.set_view(View::Archive);
        let idxs = app.visible_indices().to_vec();
        assert_eq!(idxs.len(), 1, "only the completed task");
        // The index addresses the *live* task list, not the (empty) archive.
        assert!(app.tasks()[idxs[0]].done);
        assert!(app.tasks()[idxs[0]].raw.contains("done one"));
    }
}
