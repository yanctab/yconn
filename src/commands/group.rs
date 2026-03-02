// Handlers for `yconn group` subcommands:
//   list    — show all groups found across all layers
//   use     — set the active group (persisted to ~/.config/yconn/session.yml)
//   clear   — remove active_group from session.yml, revert to default
//   current — print the active group name and its resolved config file paths

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::LoadedConfig;
use crate::display::{GroupCurrentStatus, GroupRow, LayerCurrentInfo, Renderer};

pub fn list(cfg: &LoadedConfig, renderer: &Renderer) -> Result<()> {
    let user_dir = dirs::config_dir().map(|d| d.join("yconn"));
    let system_dir = PathBuf::from("/etc/yconn");

    // Build the list of (dir, label) pairs in priority order.
    let mut dirs: Vec<(PathBuf, &str)> = Vec::new();
    if let Some(ref pd) = cfg.project_dir {
        dirs.push((pd.clone(), "project"));
    }
    if let Some(ref ud) = user_dir {
        dirs.push((ud.clone(), "user"));
    }
    dirs.push((system_dir, "system"));

    let dir_refs: Vec<(&Path, &str)> = dirs.iter().map(|(p, l)| (p.as_path(), *l)).collect();
    let groups = crate::group::discover_groups(&dir_refs)?;

    let rows: Vec<GroupRow> = groups
        .iter()
        .map(|g| GroupRow {
            name: g.name.clone(),
            layers: g.layers.clone(),
        })
        .collect();

    renderer.group_list(&rows);
    Ok(())
}

pub fn current(cfg: &LoadedConfig, renderer: &Renderer) -> Result<()> {
    let session_file = dirs::config_dir()
        .map(|d| d.join("yconn").join("session.yml").display().to_string())
        .unwrap_or_else(|| "~/.config/yconn/session.yml".to_string());

    let layers: Vec<LayerCurrentInfo> = cfg
        .layers
        .iter()
        .map(|l| LayerCurrentInfo {
            label: l.layer.label().to_string(),
            path: l.path.display().to_string(),
            found: l.connection_count.is_some(),
        })
        .collect();

    let status = GroupCurrentStatus {
        active_group: cfg.group.clone(),
        session_file,
        layers,
    };

    renderer.group_current(&status);
    Ok(())
}

/// Set the active group and warn if no config file for it exists in any layer.
///
/// The group is always written even if no config file exists — the user can
/// follow up with `yconn init` to create one.
pub fn use_group(name: &str, cfg: &LoadedConfig, renderer: &Renderer) -> Result<()> {
    let session_path = dirs::config_dir()
        .context("cannot determine user config directory")?
        .join("yconn")
        .join("session.yml");

    let user_dir = dirs::config_dir().map(|d| d.join("yconn"));
    let system_dir = PathBuf::from("/etc/yconn");

    let mut dirs: Vec<(PathBuf, &str)> = Vec::new();
    if let Some(ref pd) = cfg.project_dir {
        dirs.push((pd.clone(), "project"));
    }
    if let Some(ref ud) = user_dir {
        dirs.push((ud.clone(), "user"));
    }
    dirs.push((system_dir, "system"));

    let dir_refs: Vec<(&Path, &str)> = dirs.iter().map(|(p, l)| (p.as_path(), *l)).collect();
    use_group_impl(name, &session_path, &dir_refs, renderer)
}

/// Remove `active_group` from `session.yml`, reverting to the default group.
pub fn clear() -> Result<()> {
    let session_path = dirs::config_dir()
        .context("cannot determine user config directory")?
        .join("yconn")
        .join("session.yml");
    clear_impl(&session_path)
}

// ─── Testable impls ───────────────────────────────────────────────────────────

fn use_group_impl(
    name: &str,
    session_path: &Path,
    layer_dirs: &[(&Path, &str)],
    renderer: &Renderer,
) -> Result<()> {
    crate::group::write_session_at(session_path, Some(name))?;

    let groups = crate::group::discover_groups(layer_dirs)?;
    if !groups.iter().any(|g| g.name == name) {
        renderer.warn(&format!(
            "group '{name}' has no config file in any layer — create one with 'yconn init'"
        ));
    }

    Ok(())
}

fn clear_impl(session_path: &Path) -> Result<()> {
    crate::group::write_session_at(session_path, None)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    use crate::config;
    use crate::display::Renderer;
    use crate::group;

    fn write_yaml(dir: &std::path::Path, name: &str, content: &str) {
        fs::write(dir.join(name), content).unwrap();
    }

    fn no_color() -> Renderer {
        Renderer::new(false)
    }

    fn load(
        cwd: &std::path::Path,
        user: Option<&std::path::Path>,
        sys: &std::path::Path,
    ) -> config::LoadedConfig {
        config::load_impl(cwd, "connections", false, user, sys).unwrap()
    }

    // ── group list ────────────────────────────────────────────────────────────

    #[test]
    fn test_group_list_no_error() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(&yconn, "connections.yaml", "connections: {}\n");

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        // project_dir is set when .yconn/connections.yaml exists
        assert!(cfg.project_dir.is_some());
        list(&cfg, &no_color()).unwrap();
    }

    #[test]
    fn test_group_list_no_project_dir_no_error() {
        let cwd = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), None, empty.path());
        assert!(cfg.project_dir.is_none());
        list(&cfg, &no_color()).unwrap();
    }

    #[test]
    fn test_group_list_groups_discovered_from_project() {
        let root = TempDir::new().unwrap();
        let yconn = root.path().join(".yconn");
        fs::create_dir_all(&yconn).unwrap();
        write_yaml(&yconn, "connections.yaml", "connections: {}\n");
        write_yaml(&yconn, "work.yaml", "connections: {}\n");

        let empty = TempDir::new().unwrap();
        let cfg = load(root.path(), None, empty.path());
        // Discover manually to verify the expected groups exist.
        let dirs = [(yconn.as_path(), "project")];
        let groups = crate::group::discover_groups(&dirs).unwrap();
        let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
        assert!(names.contains(&"connections"));
        assert!(names.contains(&"work"));

        list(&cfg, &no_color()).unwrap();
    }

    // ── group current ─────────────────────────────────────────────────────────

    #[test]
    fn test_group_current_default_group_no_error() {
        let cwd = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), None, empty.path());
        assert_eq!(cfg.group, "connections");
        current(&cfg, &no_color()).unwrap();
    }

    #[test]
    fn test_group_current_active_group_from_file_no_error() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "work.yaml",
            "connections:\n  srv:\n    host: h\n    user: u\n    auth: key\n    description: d\n",
        );
        let empty = TempDir::new().unwrap();
        // Explicitly request the "work" group (as if session.yml said so)
        let cfg =
            config::load_impl(cwd.path(), "work", true, Some(user.path()), empty.path()).unwrap();
        assert_eq!(cfg.group, "work");
        assert!(cfg.group_from_file);
        current(&cfg, &no_color()).unwrap();
    }

    #[test]
    fn test_group_current_layer_found_status() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            "connections:\n  srv:\n    host: h\n    user: u\n    auth: key\n    description: d\n",
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), Some(user.path()), empty.path());

        // Project not found, user found, system not found.
        assert_eq!(cfg.layers[0].connection_count, None);
        assert!(cfg.layers[1].connection_count.is_some());
        assert_eq!(cfg.layers[2].connection_count, None);
        current(&cfg, &no_color()).unwrap();
    }

    // ── group use ─────────────────────────────────────────────────────────────

    /// Switch group: session.yml is written with the new group name.
    #[test]
    fn test_use_group_writes_session() {
        let session_dir = TempDir::new().unwrap();
        let session_path = session_dir.path().join("session.yml");

        let layer_dir = TempDir::new().unwrap();
        write_yaml(layer_dir.path(), "work.yaml", "connections: {}\n");
        let dirs = [(layer_dir.path(), "user")];

        use_group_impl("work", &session_path, &dirs, &no_color()).unwrap();

        let ag = group::read_session_at(&session_path).unwrap();
        assert_eq!(ag.name, "work");
        assert!(ag.from_file);
    }

    /// Subsequent config loads use the new group after use_group writes session.
    #[test]
    fn test_use_group_subsequent_load_uses_new_group() {
        let session_dir = TempDir::new().unwrap();
        let session_path = session_dir.path().join("session.yml");

        let layer_dir = TempDir::new().unwrap();
        write_yaml(layer_dir.path(), "work.yaml", "connections: {}\n");
        let dirs = [(layer_dir.path(), "user")];

        use_group_impl("work", &session_path, &dirs, &no_color()).unwrap();

        // Reading back directly confirms the group was persisted.
        let ag = group::read_session_at(&session_path).unwrap();
        assert_eq!(ag.name, "work");
    }

    /// Use unknown group: warning emitted but operation still succeeds and
    /// session is written (does not block).
    #[test]
    fn test_use_group_unknown_group_does_not_block() {
        let session_dir = TempDir::new().unwrap();
        let session_path = session_dir.path().join("session.yml");

        let empty_layer = TempDir::new().unwrap();
        let dirs = [(empty_layer.path(), "user")];

        // Must return Ok — warning is emitted but group is still set.
        let result = use_group_impl("no-such-group", &session_path, &dirs, &no_color());
        assert!(result.is_ok());

        // Session was written despite the warning.
        let ag = group::read_session_at(&session_path).unwrap();
        assert_eq!(ag.name, "no-such-group");
    }

    /// Use group that exists in one layer but not another: still succeeds with
    /// no warning because at least one layer has the file.
    #[test]
    fn test_use_group_missing_in_some_layers_still_succeeds() {
        let session_dir = TempDir::new().unwrap();
        let session_path = session_dir.path().join("session.yml");

        let project_dir = TempDir::new().unwrap();
        write_yaml(project_dir.path(), "work.yaml", "connections: {}\n");
        let system_dir = TempDir::new().unwrap(); // work.yaml absent in system layer

        let dirs = [
            (project_dir.path(), "project"),
            (system_dir.path(), "system"),
        ];

        use_group_impl("work", &session_path, &dirs, &no_color()).unwrap();

        let ag = group::read_session_at(&session_path).unwrap();
        assert_eq!(ag.name, "work");
    }

    /// Use group with no layer dirs at all: behaves like unknown group (warning,
    /// session written).
    #[test]
    fn test_use_group_no_dirs_does_not_block() {
        let session_dir = TempDir::new().unwrap();
        let session_path = session_dir.path().join("session.yml");

        let result = use_group_impl("work", &session_path, &[], &no_color());
        assert!(result.is_ok());

        let ag = group::read_session_at(&session_path).unwrap();
        assert_eq!(ag.name, "work");
    }

    // ── group clear ───────────────────────────────────────────────────────────

    /// Clear group: active_group is removed from session.yml, reverts to default.
    #[test]
    fn test_clear_removes_active_group() {
        let session_dir = TempDir::new().unwrap();
        let session_path = session_dir.path().join("session.yml");

        // Write a group first.
        group::write_session_at(&session_path, Some("work")).unwrap();
        let ag = group::read_session_at(&session_path).unwrap();
        assert_eq!(ag.name, "work");

        // Clear it.
        clear_impl(&session_path).unwrap();

        let ag = group::read_session_at(&session_path).unwrap();
        assert_eq!(ag.name, group::DEFAULT_GROUP);
        assert!(!ag.from_file);
    }

    #[test]
    fn test_clear_on_absent_session_succeeds() {
        let session_dir = TempDir::new().unwrap();
        let session_path = session_dir.path().join("session.yml");
        // File does not exist — clear should still succeed.
        clear_impl(&session_path).unwrap();
        let ag = group::read_session_at(&session_path).unwrap();
        assert_eq!(ag.name, group::DEFAULT_GROUP);
    }
}
