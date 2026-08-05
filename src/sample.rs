/// The sample file chiba writes on first run. Headings and prose here are not
/// decoration — they're the proof that non-task lines survive a write.
pub const TODO_RAW: &str = "# Work

- [ ] (A) 2026-05-01 Finish Q2 board deck +work @laptop due:2026-05-06
- [ ] (B) 2026-05-02 Review pull requests for auth refactor +work @laptop due:2026-05-07
- [ ] (B) 2026-04-30 Draft hiring rubric for senior eng +work @laptop
- [ ] 2026-04-29 Migrate analytics to new pipeline +work @laptop due:2026-05-15
- [x] 2026-05-05 2026-05-01 Submit expense report +work @laptop
- [x] 2026-05-04 2026-04-28 Renew domain registration +work @laptop

# Health

- [ ] (A) 2026-04-28 Call dentist to reschedule cleaning @phone +health due:2026-05-08
- [ ] (C) 2026-05-03 Order new running shoes +health @errands
- [ ] 2026-05-04 Schedule annual physical @phone +health

# Home

Anything that isn't a checkbox — this line, the headings above — is carried
through untouched. chiba never rewrites it.

- [ ] (C) 2026-05-03 Pay quarterly estimated taxes @home +finance due:2026-06-15
- [ ] 2026-05-09 Pay rent due:2026-05-15 rec:+1m +finance @home
- [ ] 2026-05-04 Pick up dry cleaning @errands
- [ ] 2026-04-20 Replace bathroom faucet washer +home @errands
- [ ] 2026-05-04 Send thank-you note to mentor @home
- [ ] 2026-05-02 Read \"Designing Data-Intensive Applications\" ch. 4 +learning @home

# Someday

- [ ] (B) 2026-04-15 Renew passport before summer trip +travel @errands due:2026-05-20
- [ ] 2026-05-01 Plan Saturday hike with K. @phone +personal
- [ ] 2026-04-01 Plan winter ski trip t:2026-09-01
- [x] 2026-05-03 2026-04-30 Book flights for July trip +travel @home
";
