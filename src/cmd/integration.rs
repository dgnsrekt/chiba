//! Installing chiba's [herdr](https://github.com/herdrdev/herdr) plugin.
//!
//! The plugin is two small files written to `~/.config/chiba/herdr/` and then
//! registered with `herdr plugin link`. We never touch herdr's
//! `plugins.json` directly — herdr owns that file, and editing it while the
//! server is running is unsafe.
//!
//! See `crate::herdr` for the other half: the marker files chiba leaves behind
//! for the plugin to read.

use std::path::PathBuf;
use std::process::Command;

use crate::herdr::PLUGIN_ID;

const MANIFEST: &str = include_str!("../../assets/herdr/herdr-plugin.toml");
const RESTORE: &str = include_str!("../../assets/herdr/restore.sh");

/// Bumped whenever the payload changes, so `status` can report "outdated" and
/// tell the user to reinstall. Mirrors herdr's own `HERDR_INTEGRATION_VERSION`
/// marker-comment scheme.
const VERSION: u32 = 2;
const VERSION_MARKER: &str = "CHIBA_HERDR_PLUGIN_VERSION=";

/// Where the plugin lives. Inside chiba's own config dir rather than herdr's,
/// so `chiba integration herdr --uninstall` can remove it wholesale without
/// going near anything herdr manages.
fn plugin_dir() -> Option<PathBuf> {
    Some(crate::xdg::config_home()?.join("chiba").join("herdr"))
}

/// Parse `CHIBA_HERDR_PLUGIN_VERSION=<n>` out of an installed payload.
fn installed_version(body: &str) -> Option<u32> {
    body.lines()
        .take(10)
        .find_map(|l| l.split_once(VERSION_MARKER))
        .and_then(|(_, rest)| {
            rest.trim()
                .split(|c: char| !c.is_ascii_digit())
                .next()
                .filter(|s| !s.is_empty())
                .and_then(|s| s.parse().ok())
        })
}

fn herdr_available() -> bool {
    Command::new("herdr")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn install_herdr(force: bool) -> i32 {
    let Some(dir) = plugin_dir() else {
        eprintln!("chiba: cannot resolve a config directory (is $HOME set?)");
        return 1;
    };
    if !herdr_available() {
        eprintln!("chiba: herdr not found on PATH — nothing to integrate with.");
        return 1;
    }

    let manifest_path = dir.join("herdr-plugin.toml");
    if let Ok(existing) = std::fs::read_to_string(&manifest_path)
        && installed_version(&existing) == Some(VERSION)
        && !force
    {
        println!(
            "already installed and current (v{VERSION}) — {}",
            dir.display()
        );
        println!("re-link with: herdr plugin link {}", dir.display());
        return 0;
    }

    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("chiba: creating {}: {e}", dir.display());
        return 1;
    }
    if let Err(e) = std::fs::write(&manifest_path, MANIFEST) {
        eprintln!("chiba: writing {}: {e}", manifest_path.display());
        return 1;
    }
    let script = dir.join("restore.sh");
    if let Err(e) = std::fs::write(&script, RESTORE) {
        eprintln!("chiba: writing {}: {e}", script.display());
        return 1;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755));
    }

    // `link` registers a local directory; `install` is for GitHub sources.
    // Linking an already-linked path is how an upgrade re-reads the manifest.
    match Command::new("herdr")
        .args(["plugin", "link"])
        .arg(&dir)
        .output()
    {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr);
            // Already linked is success for our purposes — the payload on disk
            // is what changed, and herdr re-reads it.
            if !err.to_lowercase().contains("already") {
                eprintln!("chiba: herdr plugin link failed: {}", err.trim());
                return 1;
            }
        }
        Err(e) => {
            eprintln!("chiba: running herdr plugin link: {e}");
            return 1;
        }
    }

    println!(
        "installed chiba's herdr plugin (v{VERSION}) — {}",
        dir.display()
    );
    println!();
    println!("Panes running chiba will now reopen it after a herdr restart.");
    println!("Takes effect on the next herdr start; verify with `herdr plugin list`.");
    0
}

pub fn uninstall_herdr() -> i32 {
    let Some(dir) = plugin_dir() else {
        eprintln!("chiba: cannot resolve a config directory (is $HOME set?)");
        return 1;
    };
    if herdr_available() {
        let _ = Command::new("herdr")
            .args(["plugin", "unlink", PLUGIN_ID])
            .output();
    }
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            eprintln!("chiba: removing {}: {e}", dir.display());
            return 1;
        }
    }
    // Drop any breadcrumbs so nothing is left pointing at panes.
    crate::herdr::clear_all_markers();
    println!("removed chiba's herdr plugin and any pane markers");
    0
}

pub fn status_herdr() -> i32 {
    let Some(dir) = plugin_dir() else {
        eprintln!("chiba: cannot resolve a config directory (is $HOME set?)");
        return 1;
    };
    let state = match std::fs::read_to_string(dir.join("herdr-plugin.toml")) {
        Err(_) => "not installed".to_string(),
        Ok(body) => match installed_version(&body) {
            Some(v) if v == VERSION => format!("current (v{v})"),
            Some(v) => {
                format!("outdated (v{v}, latest v{VERSION}) — run `chiba integration herdr`")
            }
            None => "installed, version unknown".to_string(),
        },
    };
    println!("herdr plugin: {state} ({})", dir.display());

    if !herdr_available() {
        println!("herdr:        not on PATH");
        return 0;
    }
    let linked = Command::new("herdr")
        .args(["plugin", "list"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(PLUGIN_ID))
        .unwrap_or(false);
    println!(
        "registered:   {}",
        if linked {
            "yes"
        } else {
            "no — run `chiba integration herdr`"
        }
    );
    println!(
        "markers:      {} pane(s) recorded",
        crate::herdr::marker_count()
    );
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_carries_the_version_we_ship() {
        assert_eq!(
            installed_version(MANIFEST),
            Some(VERSION),
            "manifest marker must match the const, or status always says outdated",
        );
        assert_eq!(installed_version(RESTORE), Some(VERSION));
    }

    #[test]
    fn version_parsing_handles_absent_and_malformed_markers() {
        assert_eq!(installed_version("no marker here"), None);
        assert_eq!(installed_version("# CHIBA_HERDR_PLUGIN_VERSION=7"), Some(7));
        assert_eq!(installed_version("# CHIBA_HERDR_PLUGIN_VERSION=x"), None);
        // Only the header is scanned, so a mention further down can't confuse it.
        let late = "\n".repeat(20) + "# CHIBA_HERDR_PLUGIN_VERSION=9";
        assert_eq!(installed_version(&late), None);
    }

    #[test]
    fn manifest_declares_the_id_chiba_writes_markers_for() {
        assert!(
            MANIFEST.contains(&format!("id = \"{PLUGIN_ID}\"")),
            "manifest id and crate::herdr::PLUGIN_ID must agree, or the plugin \
             reads a directory chiba never writes",
        );
    }
}
