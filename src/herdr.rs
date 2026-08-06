//! Breadcrumbs so a [herdr](https://github.com/herdrdev/herdr) pane that was
//! running chiba comes back running chiba.
//!
//! herdr restores every pane as "a fresh shell in its saved cwd" — it only
//! relaunches programs for a hardcoded list of AI agents, via a resume command
//! baked into its own source. chiba can't join that list without a patch, so
//! instead it leaves a marker file while it runs, and a small herdr plugin
//! (`assets/herdr/`) reads those markers from its `[[startup]]` hook and types
//! `chiba` back into the matching panes.
//!
//! The marker is written on start and removed on a clean exit, which encodes
//! the distinction that matters: **quitting** chiba should not resurrect it next
//! boot, but being **killed** by a herdr restart should.
//!
//! Everything here is best-effort. chiba runs fine outside herdr, and a failure
//! to write a breadcrumb must never be visible to someone who has never heard
//! of herdr.

use std::path::{Path, PathBuf};

/// Plugin id, shared with `assets/herdr/herdr-plugin.toml`. Also the directory
/// name herdr gives the plugin under its state dir.
pub const PLUGIN_ID: &str = "dgnsrekt.chiba";

/// Marker directory: `<state>/herdr/plugins/dgnsrekt.chiba/panes/`.
///
/// This mirrors herdr's own `plugin_paths.rs` layout rather than shelling out
/// to `herdr plugin config-dir`, so writing a marker costs no subprocess on
/// every chiba launch.
fn markers_dir() -> Option<PathBuf> {
    Some(
        crate::xdg::state_home()?
            .join("herdr")
            .join("plugins")
            .join(PLUGIN_ID)
            .join("panes"),
    )
}

/// The pane chiba is running in, or `None` when not inside herdr.
///
/// Gated exactly like herdr's own agent hooks: absent env means "not our
/// business", not an error.
fn pane_id() -> Option<String> {
    if std::env::var_os("HERDR_ENV").as_deref() != Some(std::ffi::OsStr::new("1")) {
        return None;
    }
    let pane = std::env::var("HERDR_PANE_ID").ok()?;
    // Pane ids are `<workspace>:p<n>`; anything else is not something we should
    // be turning into a filename.
    valid_pane_id(&pane).then_some(pane)
}

/// Pane ids are herdr-generated and land in a filename, so keep them to a
/// conservative charset rather than trusting the environment.
fn valid_pane_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.contains(':')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == ':' || c == '_' || c == '-')
}

fn marker_path(dir: &Path, pane: &str) -> PathBuf {
    dir.join(format!("{}.json", pane.replace(':', "_")))
}

/// Record that chiba is running in this pane, with the file it has open.
///
/// No-op outside herdr. Errors are swallowed: a breadcrumb is a convenience,
/// and chiba must start regardless.
pub fn mark_running(file: &Path) {
    let (Some(pane), Some(dir)) = (pane_id(), markers_dir()) else {
        return;
    };
    let cwd = std::env::current_dir().unwrap_or_default();
    // chiba resolves its file relative to cwd, so `file` arrives relative more
    // often than not. Absolutise it — a breadcrumb that only makes sense from
    // the directory it was written in is a poor breadcrumb.
    let file = match file.is_absolute() {
        true => file.to_path_buf(),
        false => cwd.join(file),
    };
    // Hand-rolled rather than pulling in serde: three known-shaped strings, and
    // the reader is a shell script with jq.
    let body = format!(
        "{{\"pane_id\":{},\"cwd\":{},\"file\":{}}}\n",
        json_string(&pane),
        json_string(&cwd.to_string_lossy()),
        json_string(&file.to_string_lossy()),
    );
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(marker_path(&dir, &pane), body);
    }
}

/// Forget this pane — chiba exited on purpose and should not be relaunched.
pub fn clear_running() {
    let (Some(pane), Some(dir)) = (pane_id(), markers_dir()) else {
        return;
    };
    let _ = std::fs::remove_file(marker_path(&dir, &pane));
}

/// Remove every pane marker. Used by `chiba integration herdr --uninstall`,
/// so uninstalling leaves nothing pointing at panes.
pub fn clear_all_markers() {
    if let Some(dir) = markers_dir() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// How many panes currently have a marker, for `chiba integration status`.
pub fn marker_count() -> usize {
    markers_dir()
        .and_then(|d| std::fs::read_dir(d).ok())
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                .count()
        })
        .unwrap_or(0)
}

/// Minimal JSON string escaping: quotes, backslashes, and control characters.
/// Paths can legally contain any of them.
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn pane_ids_are_validated_before_becoming_filenames() {
        assert!(valid_pane_id("wK:pD"));
        assert!(valid_pane_id("w17:p1"));
        assert!(!valid_pane_id(""), "empty");
        assert!(!valid_pane_id("no-colon"), "must look like a pane id");
        assert!(!valid_pane_id("../../etc:passwd"), "no traversal");
        assert!(!valid_pane_id("wK:pD/x"), "no separators");
        assert!(!valid_pane_id(&"a:".repeat(40)), "bounded length");
    }

    #[test]
    fn marker_filename_has_no_colon() {
        // Colons are legal on unix but hostile on other filesystems and in
        // shell globs; the reader reverses this.
        let p = marker_path(Path::new("/tmp/x"), "wK:pD");
        assert_eq!(p.file_name().unwrap(), "wK_pD.json");
    }

    #[test]
    fn json_string_escapes_what_paths_can_contain() {
        assert_eq!(json_string("plain"), "\"plain\"");
        assert_eq!(json_string("quote\"here"), "\"quote\\\"here\"");
        assert_eq!(json_string("back\\slash"), "\"back\\\\slash\"");
        assert_eq!(json_string("new\nline"), "\"new\\nline\"");
        assert_eq!(json_string("bell\u{7}"), "\"bell\\u0007\"");
    }

    #[test]
    fn state_dir_follows_xdg_override() {
        // Serialised implicitly: this is the only test touching XDG_STATE_HOME.
        let prev = std::env::var_os("XDG_STATE_HOME");
        unsafe { std::env::set_var("XDG_STATE_HOME", "/tmp/chiba-xdg-test") };
        let dir = markers_dir().unwrap();
        assert_eq!(
            dir,
            Path::new("/tmp/chiba-xdg-test/herdr/plugins/dgnsrekt.chiba/panes")
        );
        match prev {
            Some(v) => unsafe { std::env::set_var("XDG_STATE_HOME", v) },
            None => unsafe { std::env::remove_var("XDG_STATE_HOME") },
        }
    }

    #[test]
    fn outside_herdr_everything_is_a_no_op() {
        let prev = std::env::var_os("HERDR_ENV");
        unsafe { std::env::remove_var("HERDR_ENV") };
        assert!(pane_id().is_none());
        // Must not panic or create anything.
        mark_running(Path::new("/tmp/todo.md"));
        clear_running();
        if let Some(v) = prev {
            unsafe { std::env::set_var("HERDR_ENV", v) }
        }
    }
}
