use std::path::Path;

/// Why a line couldn't be parsed into a `Task`. Only `Empty` exists today —
/// the parser is permissive enough that almost anything else produces a
/// (possibly weird) `Task`. Kept as an enum so we can add reasons later
/// without changing every call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    Empty,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ParseError::Empty => "empty",
        })
    }
}

/// Why a `+project` / `@context` mutation was rejected. `Invalid` covers
/// names that would break tokenization (whitespace, sigils, colons); `Parse`
/// would fire only if a constructed line failed to re-parse, which the
/// validators ensure cannot happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagError {
    Invalid,
    Parse(ParseError),
}

impl std::fmt::Display for TagError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TagError::Invalid => f.write_str("invalid name"),
            TagError::Parse(e) => write!(f, "{}", e),
        }
    }
}

/// Bullet used for tasks chiba creates itself. Parsed tasks keep whatever
/// bullet they were written with.
pub const DEFAULT_BULLET: char = '-';

#[derive(Debug, Clone)]
pub struct Task {
    /// Leading whitespace of the source line, preserved verbatim. Carries no
    /// meaning in chiba-flat — nesting is direction B (see spec-vault).
    pub indent: String,
    /// Which list bullet this line was written with: `-`, `*`, or `+`.
    pub bullet: char,
    /// The canonical todo.txt body — *without* the markdown wrapper, but
    /// *with* the leading `x ` when done. Keeping `raw` in todo.txt form is
    /// what lets every mutation, tokenizer, and filter inherited from chiba
    /// keep working untouched; the wrapper is re-applied at serialize time.
    pub raw: String,
    pub clean_raw: String,
    pub done: bool,
    pub done_date: Option<String>,
    pub priority: Option<char>,
    pub created_date: Option<String>,
    pub projects: Vec<String>,
    pub contexts: Vec<String>,
    pub due: Option<String>,
    /// Raw value of the `rec:` tag if present, e.g. `"+1m"` or `"3b"`. Stored
    /// as the unparsed string so a malformed value round-trips intact through
    /// `serialize` — only the spawn-on-complete code path needs to parse it.
    pub rec: Option<String>,
    /// Raw value of the `t:` (threshold) tag if present, e.g. `"2026-08-01"`
    /// or `"-3d"`. Stored unparsed for round-trip integrity; the visibility
    /// filter parses it on demand via `crate::threshold`.
    pub threshold: Option<String>,
    pub notes: Vec<String>,
}

pub fn parse_line(raw: &str) -> Result<Task, ParseError> {
    let line = raw.trim();
    if line.is_empty() {
        return Err(ParseError::Empty);
    }
    let mut rest: &str = line;
    let mut done = false;
    let mut done_date: Option<String> = None;

    if let Some(stripped) = strip_prefix_x(rest) {
        done = true;
        rest = stripped;
        if let Some((date, after)) = take_iso_date_prefix(rest) {
            done_date = Some(date);
            rest = after;
        }
    }

    let mut priority: Option<char> = None;
    if let Some((c, after)) = take_priority_prefix(rest) {
        priority = Some(c);
        rest = after;
    }

    let mut created_date: Option<String> = None;
    if let Some((date, after)) = take_iso_date_prefix(rest) {
        created_date = Some(date);
        rest = after;
    }

    let projects = collect_tokens(rest, '+');
    // `#tag` is accepted as an alias for `@context` — it's what people already
    // type in markdown. Both forms land in the same list and round-trip as
    // whatever the user wrote; chiba never rewrites one into the other.
    //
    // Unlike `@`, a `#` tag must start with a letter. `#` is far too common as
    // prose punctuation — `fix #1234`, `PR #99`, `channel #2` — and without
    // this every issue reference becomes a junk context in the sidebar and in
    // autocomplete. `@` keeps todo.txt's permissive rule for compatibility.
    let mut contexts = collect_tokens(rest, '@');
    contexts.extend(
        collect_tokens(rest, '#')
            .into_iter()
            .filter(|t| t.starts_with(|c: char| c.is_alphabetic())),
    );
    let due = find_kv(rest, "due");
    let rec = find_kv(rest, "rec");
    let threshold = find_kv(rest, "t");
    let notes = find_quoted_kv(rest, "note");
    let clean_raw = body_after_quoted_kv(line);

    Ok(Task {
        indent: String::new(),
        bullet: DEFAULT_BULLET,
        raw: line.to_string(),
        clean_raw,
        done,
        done_date,
        priority,
        created_date,
        projects,
        contexts,
        due,
        rec,
        threshold,
        notes,
    })
}

fn strip_prefix_x(s: &str) -> Option<&str> {
    let mut chars = s.chars();
    if chars.next()? == 'x' {
        let rest = chars.as_str();
        if rest.starts_with(' ') || rest.starts_with('\t') {
            return Some(rest.trim_start());
        }
    }
    None
}

/// Strip a leading `YYYY-MM-DD` token. Returns `(date_string, rest)` only if
/// the prefix is a *real* calendar date — `9999-99-99` and other invalid
/// month/day combos are rejected so they don't poison sort/grouping code that
/// later trusts the value.
fn take_iso_date_prefix(s: &str) -> Option<(String, &str)> {
    let candidate = s.get(..10)?;
    if chrono::NaiveDate::parse_from_str(candidate, "%Y-%m-%d").is_err() {
        return None;
    }
    if s.len() == 10 {
        return Some((candidate.to_string(), ""));
    }
    let bytes = s.as_bytes();
    if bytes[10] == b' ' || bytes[10] == b'\t' {
        return Some((candidate.to_string(), s[11..].trim_start()));
    }
    None
}

fn take_priority_prefix(s: &str) -> Option<(char, &str)> {
    let bytes = s.as_bytes();
    if bytes.len() >= 4
        && bytes[0] == b'('
        && bytes[1].is_ascii_uppercase()
        && bytes[2] == b')'
        && (bytes[3] == b' ' || bytes[3] == b'\t')
    {
        return Some((bytes[1] as char, s[4..].trim_start()));
    }
    None
}

fn collect_tokens(s: &str, sigil: char) -> Vec<String> {
    let mut out = Vec::new();
    for tok in s.split_whitespace() {
        if let Some(rest) = tok.strip_prefix(sigil)
            && !rest.is_empty()
        {
            out.push(rest.to_string());
        }
    }
    out
}

/// Find the value of `key:value` for a specific key. Returns the first hit;
/// later duplicates are ignored.
fn find_kv(s: &str, key: &str) -> Option<String> {
    for tok in s.split_whitespace() {
        if let Some((k, v)) = tok.split_once(':')
            && is_valid_key(k)
            && !v.is_empty()
            && !v.starts_with('"')
            && k == key
        {
            return Some(v.to_string());
        }
    }
    None
}

/// Find the value of `key:"value" where value can contain spaces and is enclosed in double quotes.
/// Returns the first hit; later duplicates are ignored.
fn find_quoted_kv(s: &str, key: &str) -> Vec<String> {
    let culprit = format!(r#"{key}:""#);
    let Some(st) = s.find(&culprit) else {
        return vec![];
    };
    if st > 0 {
        let prev_char = s.as_bytes()[st - 1];
        if prev_char != b' ' && prev_char != b'\t' {
            return vec![];
        }
    }
    if !is_valid_key(key) {
        return vec![];
    }
    let v_st = st + culprit.len();
    let rest = &s[v_st..];
    let Some(end) = rest.find('"') else {
        return vec![];
    };
    rest[..end].split(". ").map(str::to_owned).collect()
}

fn is_valid_key(k: &str) -> bool {
    let mut chars = k.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Non-task lines (headings, prose, blanks, fenced code), each anchored to the
/// **task ordinal it precedes**. An anchor of `n` means "emit before task `n`";
/// `n == tasks.len()` means "after the last task".
///
/// Anchoring by ordinal rather than by absolute line is what keeps a document
/// intact across mutations: delete the task above a heading and the heading
/// stays with whatever follows it, instead of the file resequencing itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Text {
    /// Ascending by anchor, in source order within an anchor.
    lines: Vec<(usize, String)>,
}

impl Text {
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// The text lines themselves, in document order, without their anchors.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.lines.iter().map(|(_, line)| line.as_str())
    }

    fn push(&mut self, anchor: usize, line: String) {
        self.lines.push((anchor, line));
    }

    /// A task was inserted at `idx`. Everything anchored at or after it belongs
    /// to the task that just got pushed down, so those anchors move with it —
    /// which is what puts a recurrence successor under its own heading rather
    /// than the next one.
    pub fn on_insert(&mut self, idx: usize) {
        for (anchor, _) in &mut self.lines {
            if *anchor >= idx {
                *anchor += 1;
            }
        }
    }

    /// The task at `idx` was removed. Text anchored *to* it stays put — it now
    /// precedes whatever follows — while later anchors shift down to match.
    pub fn on_remove(&mut self, idx: usize) {
        for (anchor, _) in &mut self.lines {
            if *anchor > idx {
                *anchor -= 1;
            }
        }
    }
}

/// A parsed markdown document: the tasks, plus every other line.
///
/// Non-task lines are carried verbatim and are invisible to the rest of chiba —
/// the point is only that writing the file back never destroys them. tuxedo's
/// `parse_file` dropped them silently via `filter_map(...ok())`, which is
/// exactly what makes it unusable on a real markdown file.
#[derive(Debug, Clone, Default)]
pub struct Doc {
    pub tasks: Vec<Task>,
    pub text: Text,
}

/// Split a markdown task line into `(indent, bullet, done, body)`.
/// Returns `None` for anything that isn't `<indent><bullet> [ ] <body>`.
fn split_checkbox(line: &str) -> Option<(&str, char, bool, &str)> {
    let trimmed = line.trim_start();
    let indent = &line[..line.len() - trimmed.len()];
    let b = trimmed.as_bytes();
    if b.len() < 5 {
        return None;
    }
    let bullet = match b[0] {
        c @ (b'-' | b'*' | b'+') => c as char,
        _ => return None,
    };
    if b[1] != b' ' || b[2] != b'[' || b[4] != b']' {
        return None;
    }
    let done = match b[3] {
        b' ' => false,
        b'x' | b'X' => true,
        _ => return None,
    };
    // A checkbox with no trailing space is still a checkbox: "- [ ]" alone.
    let body = match trimmed.get(5..) {
        Some("") => "",
        Some(rest) if rest.starts_with([' ', '\t']) => rest.trim_start(),
        Some(_) => return None,
        None => "",
    };
    Some((indent, bullet, done, body))
}

/// True if this line opens or closes a fenced code block.
fn is_fence(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("```") || t.starts_with("~~~")
}

/// Parse one markdown line into a task, or `None` if it isn't one.
pub fn parse_md_line(line: &str) -> Option<Task> {
    let (indent, bullet, done, body) = split_checkbox(line)?;
    if body.trim().is_empty() {
        return None;
    }
    // Re-canonicalize to todo.txt: the checkbox is the source of truth for
    // completion, so a checked line gets the `x ` prefix back before the
    // inherited parser sees it.
    let canonical = match (done, strip_prefix_x(body)) {
        (true, None) => format!("x {body}"),
        _ => body.to_string(),
    };
    let mut task = parse_line(&canonical).ok()?;
    task.indent = indent.to_string();
    task.bullet = bullet;
    Some(task)
}

/// Parse a whole markdown file. Nothing is ever dropped: every line lands in
/// either `tasks` or `text`.
pub fn parse_doc(s: &str) -> Doc {
    let mut doc = Doc::default();
    let mut in_fence = false;
    for line in s.lines() {
        // Anchor is the number of tasks seen so far — i.e. the ordinal of the
        // task this line sits in front of.
        let anchor = doc.tasks.len();
        if is_fence(line) {
            in_fence = !in_fence;
            doc.text.push(anchor, line.to_string());
            continue;
        }
        // `- [ ] not a task` inside a code fence stays text.
        match if in_fence { None } else { parse_md_line(line) } {
            Some(task) => doc.tasks.push(task),
            None => doc.text.push(anchor, line.to_string()),
        }
    }
    doc
}

/// Tasks only. Kept for the call sites that genuinely don't care about the
/// surrounding document (filters, tests).
pub fn parse_file(s: &str) -> Vec<Task> {
    parse_doc(s).tasks
}

/// Serialize tasks with no surrounding document.
pub fn serialize(tasks: &[Task]) -> String {
    serialize_doc(tasks, &Text::default())
}

/// Serialize tasks back into their document, re-emitting each text line before
/// the task it is anchored to. Untouched lines come out byte-identical.
pub fn serialize_doc(tasks: &[Task], text: &Text) -> String {
    let mut out = String::new();
    let mut lines = text.lines.iter().peekable();
    let emit = |s: &str, out: &mut String| {
        out.push_str(s);
        out.push('\n');
    };
    for (i, task) in tasks.iter().enumerate() {
        // `<=` rather than `==`: an anchor that somehow fell behind still gets
        // emitted here instead of being silently dropped.
        while lines.peek().is_some_and(|(anchor, _)| *anchor <= i) {
            let (_, line) = lines.next().expect("peeked");
            emit(line, &mut out);
        }
        emit(&task.to_line(), &mut out);
    }
    // Anything anchored past the last task is the document's tail.
    for (_, line) in lines {
        emit(line, &mut out);
    }
    out
}

/// Convert a todo.txt file body into chiba's markdown form: every non-empty
/// line becomes a checkbox, `x ` completion prefixes move into the box. Lines
/// that already carry a checkbox pass through, so this is idempotent.
///
/// This is `chiba import`.
pub fn from_todotxt(raw: &str) -> String {
    let mut out = String::new();
    for line in raw.lines() {
        if line.trim().is_empty() || parse_md_line(line).is_some() {
            out.push_str(line);
        } else if let Some(rest) = line.strip_prefix("x ") {
            out.push_str(&format!("{DEFAULT_BULLET} [x] {rest}"));
        } else {
            out.push_str(&format!("{DEFAULT_BULLET} [ ] {line}"));
        }
        out.push('\n');
    }
    out
}

/// Convert a chiba markdown file back to todo.txt. Returns the body and the
/// number of non-task lines dropped — headings and prose have no todo.txt
/// equivalent, and the caller is expected to say so out loud.
///
/// This is `chiba export`.
pub fn to_todotxt(raw: &str) -> (String, usize) {
    let doc = parse_doc(raw);
    let dropped = doc.text.iter().filter(|l| !l.trim().is_empty()).count();
    let mut out = String::new();
    for t in &doc.tasks {
        out.push_str(&t.raw);
        out.push('\n');
    }
    (out, dropped)
}

/// Atomically write `body` to `path` (write to .tmp sibling, rename).
pub fn write_atomic(path: &Path, body: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

impl Task {
    /// Render this task as a markdown line: `<indent><bullet> [ ] <body>`.
    /// The `x ` completion prefix lives in the checkbox, not the body, so it's
    /// stripped on the way out and re-added on the way in.
    pub fn to_line(&self) -> String {
        let (mark, body) = match self.done {
            true => ('x', strip_prefix_x(&self.raw).unwrap_or(&self.raw)),
            false => (' ', self.raw.as_str()),
        };
        format!("{}{} [{}] {}", self.indent, self.bullet, mark, body)
    }

    /// Mark this task complete as of `today`. No-op if already done.
    /// The serialized line follows todo.txt convention: `x DONE CREATED BODY`,
    /// where `BODY` has had any leading priority/created-date stripped. If the
    /// task carried no creation date, `today` is used so the line stays well-
    /// formed.
    pub fn mark_done(&mut self, today: &str) -> Result<(), ParseError> {
        if self.done {
            return Ok(());
        }
        let created = self
            .created_date
            .clone()
            .unwrap_or_else(|| today.to_string());
        let body = body_after_priority(&self.raw);
        let new_raw = format!("x {today} {created} {body}");
        self.replace_from_raw(&new_raw)
    }

    /// Reverse `mark_done`: drop the leading `x ` and the done-date token.
    /// Priority that was stripped at completion time is not recovered — the
    /// user can re-set it after un-archiving.
    pub fn unmark_done(&mut self) -> Result<(), ParseError> {
        if !self.done {
            return Ok(());
        }
        let after_x = self.raw.strip_prefix("x ").unwrap_or(&self.raw);
        let body = if self.done_date.is_some() {
            // mark_done emits "x DONE_DATE CREATED BODY". Drop the 10-char
            // date plus its trailing space.
            let bytes = after_x.as_bytes();
            if bytes.len() >= 11 && (bytes[10] == b' ' || bytes[10] == b'\t') {
                after_x[11..].trim_start().to_string()
            } else {
                after_x.to_string()
            }
        } else {
            after_x.to_string()
        };
        self.replace_from_raw(&body)
    }

    /// Set or clear this task's priority. The priority byte is replaced in
    /// place at the start of the line; nothing else changes.
    pub fn set_priority(&mut self, priority: Option<char>) -> Result<(), ParseError> {
        let body = strip_priority(&self.raw);
        let new_raw = match priority {
            Some(p) => format!("({p}) {body}"),
            None => body.to_string(),
        };
        self.replace_from_raw(&new_raw)
    }

    /// Cycle priority A → B → C → none → A. Returns the new value (for the
    /// caller to flash). Behaves like `set_priority` w.r.t. the line format.
    pub fn cycle_priority(&mut self) -> Result<Option<char>, ParseError> {
        let next = match self.priority {
            None => Some('A'),
            Some('A') => Some('B'),
            Some('B') => Some('C'),
            Some(_) => None,
        };
        self.set_priority(next)?;
        Ok(next)
    }

    /// Append `+name` to the line. Returns `Ok(true)` if added, `Ok(false)`
    /// if the project was already present.
    pub fn add_project(&mut self, name: &str) -> Result<bool, TagError> {
        self.add_tag(name, '+', |t| &t.projects)
    }

    /// Append `@name` to the line. Returns `Ok(true)` if added, `Ok(false)`
    /// if the context was already present.
    pub fn add_context(&mut self, name: &str) -> Result<bool, TagError> {
        self.add_tag(name, '@', |t| &t.contexts)
    }

    /// Remove every `@name` token from the line. Returns `Ok(true)` if any
    /// was removed, `Ok(false)` if the context was absent.
    pub fn remove_context(&mut self, name: &str) -> Result<bool, TagError> {
        if !is_valid_tag_name(name) {
            return Err(TagError::Invalid);
        }
        if !self.contexts.iter().any(|c| c == name) {
            return Ok(false);
        }
        let (at, hash) = (format!("@{name}"), format!("#{name}"));
        let new_raw = self
            .raw
            .split_whitespace()
            .filter(|tok| *tok != at && *tok != hash)
            .collect::<Vec<_>>()
            .join(" ");
        self.replace_from_raw(&new_raw).map_err(TagError::Parse)?;
        Ok(true)
    }

    fn add_tag(
        &mut self,
        name: &str,
        sigil: char,
        existing: impl Fn(&Task) -> &Vec<String>,
    ) -> Result<bool, TagError> {
        if !is_valid_tag_name(name) {
            return Err(TagError::Invalid);
        }
        if existing(self).iter().any(|x| x == name) {
            return Ok(false);
        }
        let new_raw = format!("{} {sigil}{name}", self.raw.trim_end());
        self.replace_from_raw(&new_raw).map_err(TagError::Parse)?;
        Ok(true)
    }

    /// Re-parse `raw` and overwrite self. Only mutates on success, so a
    /// failed parse leaves the task untouched. The markdown wrapper is carried
    /// across — `parse_line` only knows about the todo.txt body.
    fn replace_from_raw(&mut self, raw: &str) -> Result<(), ParseError> {
        let mut next = parse_line(raw)?;
        next.indent = std::mem::take(&mut self.indent);
        next.bullet = self.bullet;
        *self = next;
        Ok(())
    }
}

/// True if `s` begins with a `(X) ` priority token.
pub fn starts_with_priority(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 4 && b[0] == b'(' && b[1].is_ascii_uppercase() && b[2] == b')' && b[3] == b' '
}

/// True if `s` begins with a `YYYY-MM-DD` token (followed by EOL or whitespace
/// is not required here — callers use this as a hint, not a tokenizer).
pub fn starts_with_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 10
        && b[0].is_ascii_digit()
        && b[1].is_ascii_digit()
        && b[2].is_ascii_digit()
        && b[3].is_ascii_digit()
        && b[4] == b'-'
        && b[5].is_ascii_digit()
        && b[6].is_ascii_digit()
        && b[7] == b'-'
        && b[8].is_ascii_digit()
        && b[9].is_ascii_digit()
}

/// Strip a leading `(X) ` priority token if present, otherwise return the
/// input unchanged.
pub fn strip_priority(raw: &str) -> &str {
    let b = raw.as_bytes();
    if b.len() >= 4 && b[0] == b'(' && b[1].is_ascii_uppercase() && b[2] == b')' && b[3] == b' ' {
        return &raw[4..];
    }
    raw
}

/// A project/context name is valid if non-empty and contains no characters
/// that would break the todo.txt tokenization: whitespace splits a tag in
/// half, and `+`/`@`/`:` collide with the format's own sigils.
pub fn is_valid_tag_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| !c.is_whitespace() && c != '+' && c != '@' && c != ':')
}

pub fn body_after_priority(raw: &str) -> &str {
    let mut s = raw;
    if let Some(stripped) = strip_prefix_x(s) {
        s = stripped;
        if let Some((_, after)) = take_iso_date_prefix(s) {
            s = after;
        }
    }
    if let Some((_, after)) = take_priority_prefix(s) {
        s = after;
    }
    if let Some((_, after)) = take_iso_date_prefix(s) {
        s = after;
    }
    s
}

pub fn body_after_quoted_kv(raw: &str) -> String {
    let mut body = raw.to_string();
    while let Some(st) = body.find(r#":""#) {
        let before = &body[..st];
        let after = &body[st + 2..];
        let st_key = before
            .rfind(char::is_whitespace)
            .map(|i| i + 1)
            .unwrap_or(0);
        if let Some(second_aps) = after.find('"') {
            let after = after[second_aps + 1..].trim_start();
            body = format!("{}{}", &before[..st_key], after);
        } else {
            break;
        }
    }
    body.trim().to_string()
}

/// Description text only: strip the leading `x `, done/created dates, and
/// priority via `body_after_priority`, then drop every `+project`,
/// `@context`, and `key:value` token from what remains. Whitespace between
/// surviving words collapses to single spaces. Returns an owned `String`
/// because we're filtering tokens, not slicing a prefix.
pub fn body_only(raw: &str) -> String {
    let new_body = body_after_quoted_kv(raw);
    body_after_priority(&new_body)
        .split_whitespace()
        .filter(|tok| !is_meta_token(tok))
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_meta_token(tok: &str) -> bool {
    if let Some(rest) = tok.strip_prefix('+')
        && !rest.is_empty()
    {
        return true;
    }
    if let Some(rest) = tok.strip_prefix('@')
        && !rest.is_empty()
    {
        return true;
    }
    // Mirrors the `#tag` rule in `parse_line`: only letter-initial tags are
    // metadata, so `fix #1234` keeps the `#1234` in its description text.
    if let Some(rest) = tok.strip_prefix('#')
        && rest.starts_with(|c: char| c.is_alphabetic())
    {
        return true;
    }
    if let Some((k, v)) = tok.split_once(':')
        && is_valid_key(k)
        && !v.is_empty()
    {
        return true;
    }
    false
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_line_empty_returns_err() {
        assert!(matches!(parse_line(""), Err(ParseError::Empty)));
        assert!(matches!(parse_line("   "), Err(ParseError::Empty)));
        assert!(matches!(parse_line("\n"), Err(ParseError::Empty)));
    }

    #[test]
    fn parse_line_simple_input_returns_ok() {
        assert!(parse_line("Hello").is_ok());
    }

    #[test]
    fn parse_error_displays_human_message() {
        assert_eq!(format!("{}", ParseError::Empty), "empty");
    }

    #[test]
    fn parses_line_starting_with_non_ascii_after_single_byte() {
        // Regression: `take_iso_date_prefix` used byte indexing (`&s[..10]`)
        // after a byte-length check, which panicked when byte 10 fell inside
        // a multi-byte UTF-8 character. Triggered by tasks like the one
        // below, where '2' is 1 byte and the following Cyrillic chars are
        // 2 bytes each, putting byte 10 inside 'с'.
        let t = parse_line("2Написать задачи на день due:2026-05-11 rec:+1d").unwrap();
        assert_eq!(t.created_date, None);
        assert_eq!(t.due.as_deref(), Some("2026-05-11"));
        assert_eq!(t.rec.as_deref(), Some("+1d"));
    }

    #[test]
    fn rejects_invalid_calendar_dates() {
        // `9999-99-99` is well-formed lexically but not a real date —
        // earlier versions accepted it and let the bogus value flow into
        // sort/grouping code as a string. The parser now refuses.
        let t = parse_line("9999-99-99 not a date").unwrap();
        assert_eq!(t.created_date, None);
        assert!(t.raw.starts_with("9999-99-99"));
    }

    #[test]
    fn parses_priority_and_dates() {
        let t = parse_line("(A) 2026-04-28 Call dentist @phone +health due:2026-05-08").unwrap();
        assert_eq!(t.priority, Some('A'));
        assert_eq!(t.created_date.as_deref(), Some("2026-04-28"));
        assert_eq!(t.due.as_deref(), Some("2026-05-08"));
        assert_eq!(t.projects, vec!["health"]);
        assert_eq!(t.contexts, vec!["phone"]);
        assert!(!t.done);
        assert_eq!(t.rec, None);
    }

    #[test]
    fn parses_rec_tag() {
        let t = parse_line("2026-05-09 Pay rent due:2026-05-15 rec:+1m").unwrap();
        assert_eq!(t.rec.as_deref(), Some("+1m"));
        assert_eq!(t.due.as_deref(), Some("2026-05-15"));
    }

    #[test]
    fn parses_absolute_threshold_tag() {
        let t = parse_line("2026-04-01 Renew passport t:2026-08-01 +personal").unwrap();
        assert_eq!(t.threshold.as_deref(), Some("2026-08-01"));
    }

    #[test]
    fn parses_relative_threshold_tag() {
        let t = parse_line("Pay rent due:2026-06-01 t:-3d +finance").unwrap();
        assert_eq!(t.threshold.as_deref(), Some("-3d"));
        assert_eq!(t.due.as_deref(), Some("2026-06-01"));
    }

    #[test]
    fn body_only_strips_threshold_token() {
        // The "no chip" rendering choice relies on body_only filtering `t:`
        // out via is_meta_token. Asserting it here so a future change to
        // is_valid_key can't regress this without an explicit test failure.
        assert_eq!(
            body_only("2026-04-01 Renew passport t:2026-08-01 +personal"),
            "Renew passport",
        );
        assert_eq!(
            body_only("Pay rent due:2026-06-01 t:-3d +finance"),
            "Pay rent",
        );
    }

    #[test]
    fn parses_completed() {
        let t = parse_line("x 2026-05-05 2026-05-01 Submit expense report +work @laptop").unwrap();
        assert!(t.done);
        assert_eq!(t.done_date.as_deref(), Some("2026-05-05"));
        assert_eq!(t.created_date.as_deref(), Some("2026-05-01"));
        assert_eq!(t.projects, vec!["work"]);
    }

    #[test]
    fn parses_all_sample_lines() {
        let parsed = parse_file(crate::sample::TODO_RAW);
        assert_eq!(parsed.len(), 19);
        let done = parsed.iter().filter(|t| t.done).count();
        assert_eq!(done, 3);
        let with_due = parsed.iter().filter(|t| t.due.is_some()).count();
        assert_eq!(with_due, 7);
        let with_rec = parsed.iter().filter(|t| t.rec.is_some()).count();
        assert_eq!(with_rec, 1);
        let with_threshold = parsed.iter().filter(|t| t.threshold.is_some()).count();
        assert_eq!(with_threshold, 1);
    }

    #[test]
    fn body_strips_metadata() {
        let raw = "(A) 2026-05-01 Hello world";
        assert_eq!(body_after_priority(raw), "Hello world");
        let raw2 = "x 2026-05-05 2026-05-01 Hello world";
        assert_eq!(body_after_priority(raw2), "Hello world");
    }

    #[test]
    fn body_only_drops_tags_and_kv_pairs() {
        // Plain description survives unchanged.
        assert_eq!(body_only("Hello world"), "Hello world");
        // Priority + creation date prefix are stripped, +project / @context /
        // due:... are filtered out, words collapse to single spaces.
        assert_eq!(
            body_only("(A) 2026-04-28 Call dentist @phone +health due:2026-05-08"),
            "Call dentist",
        );
        // Completed lines lose `x` + done date + creation date as well.
        assert_eq!(
            body_only("x 2026-05-05 2026-05-01 Submit expense report +work @laptop"),
            "Submit expense report",
        );
        // Sigils inside a word (not at the start of a token) are not tags
        // and must be preserved.
        assert_eq!(body_only("email a+b@example.com"), "email a+b@example.com");
        // Lone sigils with no name are not valid tags either.
        assert_eq!(body_only("type @ then context"), "type @ then context");
        // Unknown key:value tokens still drop — todo.txt treats any
        // alphanumeric `key:value` as an extension, so we mirror that.
        assert_eq!(body_only("backup id:abc-123 nightly"), "backup nightly");
    }

    #[test]
    fn round_trip_preserves_raw() {
        let parsed = parse_file(crate::sample::TODO_RAW);
        let serialized = serialize(&parsed);
        let reparsed = parse_file(&serialized);
        assert_eq!(parsed.len(), reparsed.len());
        for (a, b) in parsed.iter().zip(reparsed.iter()) {
            assert_eq!(a.raw, b.raw);
        }
    }

    // ----- markdown layer -------------------------------------------------
    //
    // These five are the fork's reason to exist: tuxedo's parse_file dropped
    // every non-task line on the floor, so writing the file back destroyed a
    // real markdown document. If any of these break, chiba eats user data.

    const MESSY: &str = "\
---
title: tasks
---

# Work

Some prose with a - [ ] that isn't at the start of a line.

- [ ] (A) 2026-05-01 real task +work due:2026-05-08
* [x] 2026-05-05 2026-05-01 done with a star bullet
  - [ ] indented task #home

```sh
- [ ] not a task, this is inside a fence
```

> a blockquote
1. an ordered list item

- [ ]
- [ ] last +work
";

    #[test]
    fn round_trip_preserves_every_non_task_line() {
        let doc = parse_doc(MESSY);
        let out = serialize_doc(&doc.tasks, &doc.text);
        assert_eq!(out, MESSY, "round-trip must be byte-identical");
    }

    #[test]
    fn fenced_checkbox_is_not_a_task() {
        let doc = parse_doc(MESSY);
        assert!(
            !doc.tasks.iter().any(|t| t.raw.contains("inside a fence")),
            "a checkbox inside a code fence must stay text",
        );
        // 4 real tasks: the (A) one, the starred done one, the indented one,
        // and "last". The bare "- [ ]" has no body and stays text.
        assert_eq!(doc.tasks.len(), 4);
    }

    #[test]
    fn mutation_touches_only_its_own_line() {
        let mut doc = parse_doc(MESSY);
        doc.tasks[0].mark_done("2026-05-09").expect("mark done");
        let out = serialize_doc(&doc.tasks, &doc.text);
        let (before, after): (Vec<_>, Vec<_>) = (MESSY.lines().collect(), out.lines().collect());
        let differing: Vec<_> = before
            .iter()
            .zip(after.iter())
            .filter(|(a, b)| a != b)
            .collect();
        assert_eq!(
            differing.len(),
            1,
            "exactly one line may change: {differing:?}"
        );
        assert!(
            after
                .iter()
                .any(|l| l.starts_with("- [x] 2026-05-09 2026-05-01 real task"))
        );
    }

    #[test]
    fn wrapper_survives_mutation() {
        // Bullet and indent are re-emitted from the Task, not from `raw` —
        // a priority change must not turn "  - " into "- " or "* " into "- ".
        let mut doc = parse_doc("  * [ ] indented star task\n");
        doc.tasks[0].set_priority(Some('B')).expect("set priority");
        assert_eq!(
            serialize_doc(&doc.tasks, &doc.text),
            "  * [ ] (B) indented star task\n"
        );
    }

    #[test]
    fn import_export_is_identity_for_task_only_files() {
        let txt = "(A) 2026-05-01 a +work\nx 2026-05-05 2026-05-01 b\n2026-05-02 c @home\n";
        let md = from_todotxt(txt);
        assert!(md.starts_with("- [ ] (A)"));
        let (back, dropped) = to_todotxt(&md);
        assert_eq!(back, txt);
        assert_eq!(dropped, 0);
    }

    #[test]
    fn export_reports_dropped_prose() {
        let (_, dropped) = to_todotxt("# Heading\n\n- [ ] a task\nsome prose\n");
        assert_eq!(dropped, 2, "heading + prose counted, blank line not");
    }

    // ----- anchoring under mutation ---------------------------------------
    //
    // The first cut of this pinned text to *absolute* output positions, which
    // silently reordered the document on every delete. These lock the
    // ordinal-anchored behaviour in.

    const SECTIONED: &str = "\
# Work
- [ ] task a
- [ ] task b

# Home
- [ ] task c
";

    #[test]
    fn deleting_a_task_leaves_headings_with_their_sections() {
        let mut doc = parse_doc(SECTIONED);
        doc.text.on_remove(0);
        doc.tasks.remove(0);
        assert_eq!(
            serialize_doc(&doc.tasks, &doc.text),
            "# Work\n- [ ] task b\n\n# Home\n- [ ] task c\n",
            "task c must stay under # Home, not jump above it",
        );
    }

    #[test]
    fn deleting_the_last_task_of_a_section_keeps_the_heading_in_place() {
        let mut doc = parse_doc(SECTIONED);
        doc.text.on_remove(2);
        doc.tasks.remove(2);
        assert_eq!(
            serialize_doc(&doc.tasks, &doc.text),
            "# Work\n- [ ] task a\n- [ ] task b\n\n# Home\n",
            "an emptied section keeps its heading, at the end",
        );
    }

    #[test]
    fn an_inserted_task_lands_in_its_own_section() {
        // This is the recurrence-successor path: complete task b (index 1) and
        // the fresh copy goes in at index 2 — under # Work, where b lives.
        let mut doc = parse_doc(SECTIONED);
        doc.text.on_insert(2);
        doc.tasks
            .insert(2, parse_line("task b next").expect("parse"));
        assert_eq!(
            serialize_doc(&doc.tasks, &doc.text),
            "# Work\n- [ ] task a\n- [ ] task b\n- [ ] task b next\n\n# Home\n- [ ] task c\n",
        );
    }

    #[test]
    fn an_appended_task_goes_to_the_end_of_the_file() {
        let mut doc = parse_doc(SECTIONED);
        doc.tasks.push(parse_line("brand new").expect("parse"));
        assert_eq!(
            serialize_doc(&doc.tasks, &doc.text),
            "# Work\n- [ ] task a\n- [ ] task b\n\n# Home\n- [ ] task c\n- [ ] brand new\n",
        );
    }

    #[test]
    fn removing_every_task_still_keeps_the_prose() {
        let mut doc = parse_doc(SECTIONED);
        for _ in 0..doc.tasks.len() {
            doc.text.on_remove(0);
            doc.tasks.remove(0);
        }
        assert_eq!(serialize_doc(&doc.tasks, &doc.text), "# Work\n\n# Home\n");
    }

    #[test]
    fn hash_tags_must_start_with_a_letter() {
        // Issue and PR references are prose, not contexts.
        let t = parse_md_line("- [ ] fix #1234 and see PR #99 #home").expect("task");
        assert_eq!(t.contexts, ["home"]);
        assert_eq!(body_only(&t.raw), "fix #1234 and see PR #99");
    }

    #[test]
    fn hash_tag_is_a_context() {
        let t = parse_md_line("- [ ] water plants #home @errands").expect("task");
        // `@` tokens first, then `#` — both land in contexts.
        assert_eq!(t.contexts, ["errands", "home"]);
        // ...and neither leaks into the description text.
        assert_eq!(body_only(&t.raw), "water plants");
    }
}
