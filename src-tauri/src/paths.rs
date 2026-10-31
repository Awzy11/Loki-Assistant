//! Where Loki keeps its config and data. Linux and macOS both use the XDG layout (`~/.config/loki`, `~/.local/share/loki`, or
//! `$XDG_CONFIG_HOME` / `$XDG_DATA_HOME` when set), so the documentation, the shell scripts and the code agree on one place.
//! macOS would otherwise get `~/Library/Application Support`, which splits files between two folders. Windows uses its own folders.

use std::path::PathBuf;

#[cfg(unix)]
fn xdg(var: &str, fallback: &str) -> Option<PathBuf> {
    match std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute()) {
        Some(p) => Some(p),
        None => dirs::home_dir().map(|h| h.join(fallback)),
    }
}

/// The base folder for configuration (Loki's own files go in its `loki` subfolder).
pub fn config_dir() -> Option<PathBuf> {
    #[cfg(unix)]
    return xdg("XDG_CONFIG_HOME", ".config");
    #[cfg(not(unix))]
    return crate::paths::config_dir();
}

/// The base folder for data Loki generates.
pub fn data_dir() -> Option<PathBuf> {
    #[cfg(unix)]
    return xdg("XDG_DATA_HOME", ".local/share");
    #[cfg(not(unix))]
    return crate::paths::data_dir();
}

#[cfg(all(test, unix))]
mod tests {
    #[test]
    fn follows_the_xdg_layout() {
        let home = dirs::home_dir().unwrap();
        if std::env::var_os("XDG_CONFIG_HOME").is_none() {
            assert_eq!(super::config_dir().unwrap(), home.join(".config"));
        }
        if std::env::var_os("XDG_DATA_HOME").is_none() {
            assert_eq!(super::data_dir().unwrap(), home.join(".local/share"));
        }
    }
}
