//! Active group resolution and session.yml read/write.
//!
//! Reads and writes `~/.config/yconn/session.yml`. Resolves the active group
//! name (defaulting to `None` when the file is absent or the key is unset,
//! meaning "no group lock — show all connections by default").
//!
//! Group discovery is now performed by `LoadedConfig::discover_groups()` which
//! scans the inline `group:` fields on connection entries rather than scanning
//! the filesystem for named YAML files.

// Public API is consumed by CLI command modules not yet implemented.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ─── Public types ─────────────────────────────────────────────────────────────

/// The resolved active group lock and whether it came from `session.yml`.
pub struct ActiveGroup {
    /// The locked group name, or `None` when no group is locked (show all).
    pub name: Option<String>,
    /// `true` = read from `session.yml`; `false` = using the built-in default.
    pub from_file: bool,
}

/// A group discovered by scanning connection `group:` fields across all layers.
pub struct GroupEntry {
    pub name: String,
    /// Which layers contain connections tagged with this group.
    pub layers: Vec<String>,
}

// ─── serde type for session.yml ──────────────────────────────────────────────

/// Wire type for `~/.config/yconn/session.yml`.
///
/// Unknown keys are silently ignored by serde's default behaviour, which
/// preserves forward compatibility with future versions.
#[derive(Deserialize, Serialize, Default)]
struct SessionFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active_group: Option<String>,
}

// ─── Path helpers ─────────────────────────────────────────────────────────────

fn session_path() -> Result<PathBuf> {
    let base = dirs::config_dir().context("cannot determine user config directory")?;
    Ok(base.join("yconn").join("session.yml"))
}

// ─── Private I/O helpers (also used directly by tests) ───────────────────────

pub(crate) fn read_session_at(path: &Path) -> Result<ActiveGroup> {
    if !path.exists() {
        return Ok(ActiveGroup {
            name: None,
            from_file: false,
        });
    }

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;

    let file: SessionFile = serde_yaml::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;

    match file.active_group.filter(|s| !s.is_empty()) {
        Some(name) => Ok(ActiveGroup {
            name: Some(name),
            from_file: true,
        }),
        None => Ok(ActiveGroup {
            name: None,
            from_file: false,
        }),
    }
}

pub(crate) fn write_session_at(path: &Path, group: Option<&str>) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }

    let file = SessionFile {
        active_group: group.map(str::to_owned),
    };
    let content = serde_yaml::to_string(&file).context("failed to serialise session file")?;

    std::fs::write(path, content).with_context(|| format!("failed to write {}", path.display()))?;

    set_private_permissions(path)?;

    Ok(())
}

/// Set 0o600 permissions on `path` so it is not world-readable.
///
/// No-op on non-Unix platforms.
#[cfg(unix)]
fn set_private_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(path, perms)
        .with_context(|| format!("failed to set permissions on {}", path.display()))
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Return the active group lock, reading `session.yml` via the standard path.
///
/// Returns `name: None` when the file is absent or `active_group` is unset,
/// meaning no group is locked and all connections should be shown by default.
pub fn active_group() -> Result<ActiveGroup> {
    read_session_at(&session_path()?)
}

/// Persist `name` as the active group in `session.yml`.
pub fn set_active_group(name: &str) -> Result<()> {
    write_session_at(&session_path()?, Some(name))
}

/// Remove `active_group` from `session.yml`, reverting to the default (no lock).
pub fn clear_active_group() -> Result<()> {
    write_session_at(&session_path()?, None)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // ── session read ──────────────────────────────────────────────────────────

    #[test]
    fn test_read_session_file_absent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.yml");
        let ag = read_session_at(&path).unwrap();
        assert!(ag.name.is_none());
        assert!(!ag.from_file);
    }

    #[test]
    fn test_read_session_active_group_set() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.yml");
        fs::write(&path, "active_group: work\n").unwrap();
        let ag = read_session_at(&path).unwrap();
        assert_eq!(ag.name.as_deref(), Some("work"));
        assert!(ag.from_file);
    }

    #[test]
    fn test_read_session_active_group_empty_string() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.yml");
        fs::write(&path, "active_group: \"\"\n").unwrap();
        let ag = read_session_at(&path).unwrap();
        assert!(ag.name.is_none());
        assert!(!ag.from_file);
    }

    #[test]
    fn test_read_session_empty_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.yml");
        fs::write(&path, "").unwrap();
        let ag = read_session_at(&path).unwrap();
        assert!(ag.name.is_none());
        assert!(!ag.from_file);
    }

    #[test]
    fn test_read_session_unknown_keys_ignored() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.yml");
        fs::write(&path, "active_group: staging\nsome_future_key: 42\n").unwrap();
        let ag = read_session_at(&path).unwrap();
        assert_eq!(ag.name.as_deref(), Some("staging"));
        assert!(ag.from_file);
    }

    // ── session write ─────────────────────────────────────────────────────────

    #[test]
    fn test_write_session_creates_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.yml");
        assert!(!path.exists());
        write_session_at(&path, Some("work")).unwrap();
        assert!(path.exists());
        let ag = read_session_at(&path).unwrap();
        assert_eq!(ag.name.as_deref(), Some("work"));
    }

    #[test]
    fn test_write_session_creates_parent_dirs() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nested").join("dirs").join("session.yml");
        write_session_at(&path, Some("work")).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn test_write_session_clear_removes_key() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.yml");
        write_session_at(&path, Some("work")).unwrap();
        write_session_at(&path, None).unwrap();
        let ag = read_session_at(&path).unwrap();
        assert!(ag.name.is_none());
        assert!(!ag.from_file);
    }

    #[test]
    fn test_write_session_overwrite_existing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.yml");
        write_session_at(&path, Some("work")).unwrap();
        write_session_at(&path, Some("private")).unwrap();
        let ag = read_session_at(&path).unwrap();
        assert_eq!(ag.name.as_deref(), Some("private"));
    }

    #[test]
    #[cfg(unix)]
    fn test_write_session_file_has_0o600_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.yml");
        write_session_at(&path, Some("work")).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "session.yml should have 0o600 permissions");
    }

    #[test]
    #[cfg(unix)]
    fn test_write_session_clear_file_has_0o600_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.yml");
        write_session_at(&path, Some("work")).unwrap();
        write_session_at(&path, None).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "session.yml should have 0o600 permissions after clear"
        );
    }
}
