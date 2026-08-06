//! XDG Base Directory resolution shared between `config` and `theme` loaders.

use std::path::PathBuf;

/// Resolve an XDG base directory from `var`, falling back to `~/<fallback>`.
///
/// Per the XDG Base Directory Spec the variable MUST be an absolute path;
/// relative values are ignored. We warn so users debugging path resolution can
/// see why their override didn't take effect.
fn base_dir(var: &str, fallback: &str) -> Option<PathBuf> {
    if let Some(v) = std::env::var_os(var)
        && !v.is_empty()
    {
        let p = PathBuf::from(&v);
        if p.is_absolute() {
            return Some(p);
        }
        eprintln!(
            "chiba: ignoring non-absolute {var}={} (per XDG spec)",
            p.display()
        );
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(fallback))
}

/// Resolve the XDG base config directory (`~/.config` by default).
pub fn config_home() -> Option<PathBuf> {
    base_dir("XDG_CONFIG_HOME", ".config")
}

/// Resolve the XDG base state directory (`~/.local/state` by default).
///
/// herdr keeps per-plugin state under `<state>/herdr/plugins/<plugin-id>/`, so
/// this is how chiba finds the drop box its herdr plugin reads on startup.
pub fn state_home() -> Option<PathBuf> {
    base_dir("XDG_STATE_HOME", ".local/state")
}
