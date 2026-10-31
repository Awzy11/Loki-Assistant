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

/// Writes a file that holds a secret. On Unix it is created with mode 0600 from the start (never briefly readable under the default
/// umask) and an existing file is tightened to 0600 too.
pub fn write_private(path: &std::path::Path, data: impl AsRef<[u8]>) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        f.write_all(data.as_ref())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, data)
    }
}

#[cfg(all(test, unix))]
mod tests {
    #[test]
    fn private_files_are_0600_new_or_existing() {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("loki-priv-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("secret");
        super::write_private(&p, "one").unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        super::write_private(&p, "two").unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "two");
        let _ = std::fs::remove_dir_all(&d);
    }

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
