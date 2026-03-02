//! Active group resolution and session.yml read/write.
//!
//! Reads and writes `~/.config/yconn/session.yml`. Resolves the active group
//! name (defaulting to `"connections"` when the file is absent or the key is
//! unset). Scans layer directories to discover which groups have config files,
//! used by `yconn group list`.

// Public API is consumed by CLI command modules not yet implemented.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ─── Public constants ─────────────────────────────────────────────────────────

/// The group name used when no `active_group` is set in `session.yml`.
pub const DEFAULT_GROUP: &str = "connections";

// ─── Public types ─────────────────────────────────────────────────────────────

/// The resolved active group and whether it came from `session.yml`.
pub struct ActiveGroup {
    /// The group name (never empty; defaults to [`DEFAULT_GROUP`]).
    pub name: String,
    /// `true` = read from `session.yml`; `false` = using the built-in default.
    pub from_file: bool,
}

/// A group discovered by scanning layer config directories.
pub struct GroupEntry {
    pub name: String,
    /// Which layers have a config file for this group (e.g. `["project", "user"]`).
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
            name: DEFAULT_GROUP.into(),
            from_file: false,
        });
    }

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;

    let file: SessionFile = serde_yaml::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;

    match file.active_group.filter(|s| !s.is_empty()) {
        Some(name) => Ok(ActiveGroup {
            name,
            from_file: true,
        }),
        None => Ok(ActiveGroup {
            name: DEFAULT_GROUP.into(),
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

    Ok(())
}

fn discover_in_dirs(dirs: &[(&Path, &str)]) -> Result<Vec<GroupEntry>> {
    // BTreeMap keeps groups sorted by name for stable output.
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for (dir, layer_label) in dirs {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => continue, // directory absent or unreadable — skip silently
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
                continue;
            }
            let stem = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };
            // Skip the session state file — it is not a group config.
            if stem == "session" {
                continue;
            }
            map.entry(stem).or_default().push(layer_label.to_string());
        }
    }

    Ok(map
        .into_iter()
        .map(|(name, layers)| GroupEntry { name, layers })
        .collect())
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Return the active group, reading `session.yml` via the standard path.
///
/// Returns `DEFAULT_GROUP` when the file is absent or `active_group` is unset.
pub fn active_group() -> Result<ActiveGroup> {
    read_session_at(&session_path()?)
}

/// Persist `name` as the active group in `session.yml`.
pub fn set_active_group(name: &str) -> Result<()> {
    write_session_at(&session_path()?, Some(name))
}

/// Remove `active_group` from `session.yml`, reverting to the default.
pub fn clear_active_group() -> Result<()> {
    write_session_at(&session_path()?, None)
}

/// Scan `dirs` — each `(directory, layer_label)` pair — and return all groups
/// found across them.
///
/// The caller is responsible for building the directory list (including the
/// project-layer path obtained from the upward walk). Directories that do not
/// exist are silently skipped.
///
/// Results are sorted by group name.
pub fn discover_groups(dirs: &[(&Path, &str)]) -> Result<Vec<GroupEntry>> {
    discover_in_dirs(dirs)
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
        assert_eq!(ag.name, DEFAULT_GROUP);
        assert!(!ag.from_file);
    }

    #[test]
    fn test_read_session_active_group_set() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.yml");
        fs::write(&path, "active_group: work\n").unwrap();
        let ag = read_session_at(&path).unwrap();
        assert_eq!(ag.name, "work");
        assert!(ag.from_file);
    }

    #[test]
    fn test_read_session_active_group_empty_string() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.yml");
        fs::write(&path, "active_group: \"\"\n").unwrap();
        let ag = read_session_at(&path).unwrap();
        assert_eq!(ag.name, DEFAULT_GROUP);
        assert!(!ag.from_file);
    }

    #[test]
    fn test_read_session_empty_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.yml");
        fs::write(&path, "").unwrap();
        let ag = read_session_at(&path).unwrap();
        assert_eq!(ag.name, DEFAULT_GROUP);
        assert!(!ag.from_file);
    }

    #[test]
    fn test_read_session_unknown_keys_ignored() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.yml");
        fs::write(&path, "active_group: staging\nsome_future_key: 42\n").unwrap();
        let ag = read_session_at(&path).unwrap();
        assert_eq!(ag.name, "staging");
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
        assert_eq!(ag.name, "work");
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
        assert_eq!(ag.name, DEFAULT_GROUP);
        assert!(!ag.from_file);
    }

    #[test]
    fn test_write_session_overwrite_existing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.yml");
        write_session_at(&path, Some("work")).unwrap();
        write_session_at(&path, Some("private")).unwrap();
        let ag = read_session_at(&path).unwrap();
        assert_eq!(ag.name, "private");
    }

    // ── group discovery ───────────────────────────────────────────────────────

    #[test]
    fn test_discover_no_dirs() {
        let groups = discover_in_dirs(&[]).unwrap();
        assert!(groups.is_empty());
    }

    #[test]
    fn test_discover_missing_dirs_silently_skipped() {
        let dir = TempDir::new().unwrap();
        let absent = dir.path().join("does-not-exist");
        let groups = discover_in_dirs(&[(&absent, "project")]).unwrap();
        assert!(groups.is_empty());
    }

    #[test]
    fn test_discover_single_layer() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("connections.yaml"), "").unwrap();
        fs::write(dir.path().join("work.yaml"), "").unwrap();
        let groups = discover_in_dirs(&[(dir.path(), "user")]).unwrap();
        assert_eq!(groups.len(), 2);
        let connections = groups.iter().find(|g| g.name == "connections").unwrap();
        assert_eq!(connections.layers, vec!["user"]);
        let work = groups.iter().find(|g| g.name == "work").unwrap();
        assert_eq!(work.layers, vec!["user"]);
    }

    #[test]
    fn test_discover_multiple_layers_same_group() {
        let project = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        let system = TempDir::new().unwrap();

        fs::write(project.path().join("work.yaml"), "").unwrap();
        fs::write(user.path().join("work.yaml"), "").unwrap();
        fs::write(system.path().join("connections.yaml"), "").unwrap();

        let groups = discover_in_dirs(&[
            (project.path(), "project"),
            (user.path(), "user"),
            (system.path(), "system"),
        ])
        .unwrap();

        let work = groups.iter().find(|g| g.name == "work").unwrap();
        assert_eq!(work.layers, vec!["project", "user"]);

        let connections = groups.iter().find(|g| g.name == "connections").unwrap();
        assert_eq!(connections.layers, vec!["system"]);
    }

    #[test]
    fn test_discover_skips_session_file() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("session.yml"), "").unwrap();
        fs::write(dir.path().join("connections.yaml"), "").unwrap();
        let groups = discover_in_dirs(&[(dir.path(), "user")]).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "connections");
    }

    #[test]
    fn test_discover_skips_non_yaml_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("connections.yaml"), "").unwrap();
        fs::write(dir.path().join("README.md"), "").unwrap();
        fs::write(dir.path().join("notes.txt"), "").unwrap();
        let groups = discover_in_dirs(&[(dir.path(), "user")]).unwrap();
        assert_eq!(groups.len(), 1);
    }

    #[test]
    fn test_discover_results_sorted_by_name() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("work.yaml"), "").unwrap();
        fs::write(dir.path().join("connections.yaml"), "").unwrap();
        fs::write(dir.path().join("private.yaml"), "").unwrap();
        let groups = discover_in_dirs(&[(dir.path(), "user")]).unwrap();
        let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, vec!["connections", "private", "work"]);
    }
}
