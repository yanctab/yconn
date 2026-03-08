// Handlers for `yconn groups` subcommands:
//   list    — show all groups found across all connections
//   use     — set the active group lock (persisted to ~/.config/yconn/session.yml)
//   clear   — remove active_group from session.yml, revert to no lock
//   current — print the active group name and its resolved config file paths

use std::path::Path;

use anyhow::{Context, Result};

use crate::config::LoadedConfig;
use crate::display::{GroupCurrentStatus, GroupRow, LayerCurrentInfo, Renderer};

pub fn list(cfg: &LoadedConfig, renderer: &Renderer) -> Result<()> {
    let groups = cfg.discover_groups();

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

/// Set the active group lock and warn if no connections with that group value
/// exist in any layer.
///
/// The group is always written even if no connections use it — the user can
/// follow up with `yconn connections add` to tag connections with this group.
/// Invoked as `yconn groups use <name>`.
pub fn use_group(name: &str, cfg: &LoadedConfig, renderer: &Renderer) -> Result<()> {
    let session_path = dirs::config_dir()
        .context("cannot determine user config directory")?
        .join("yconn")
        .join("session.yml");

    use_group_impl(name, &session_path, cfg, renderer)
}

/// Remove `active_group` from `session.yml`, reverting to no group lock.
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
    cfg: &LoadedConfig,
    renderer: &Renderer,
) -> Result<()> {
    crate::group::write_session_at(session_path, Some(name))?;

    // Warn if no connections in any layer carry this group tag.
    let groups = cfg.discover_groups();
    if !groups.iter().any(|g| g.name == name) {
        renderer.warn(&format!(
            "group '{name}' has no connections tagged with it in any layer — \
             tag connections with 'group: {name}' or use 'yconn connections add'"
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
        config::load_impl(cwd, None, false, user, sys).unwrap()
    }

    fn simple_conn(name: &str, host: &str) -> String {
        format!(
            "connections:\n  {name}:\n    host: {host}\n    user: user\n    auth: key\n    description: desc\n"
        )
    }

    fn conn_with_group(name: &str, host: &str, group: &str) -> String {
        format!(
            "connections:\n  {name}:\n    host: {host}\n    user: user\n    auth: key\n    description: desc\n    group: {group}\n"
        )
    }

    // ── group list ────────────────────────────────────────────────────────────

    #[test]
    fn test_group_list_no_error_empty() {
        let cwd = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), None, empty.path());
        list(&cfg, &no_color()).unwrap();
    }

    #[test]
    fn test_group_list_shows_groups_from_connections() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            &conn_with_group("work-srv", "10.0.0.1", "work"),
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), Some(user.path()), empty.path());
        // Should find "work" group from inline field
        let groups = cfg.discover_groups();
        assert!(!groups.is_empty());
        assert!(groups.iter().any(|g| g.name == "work"));
        list(&cfg, &no_color()).unwrap();
    }

    #[test]
    fn test_group_list_connections_without_group_field_not_shown() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            &simple_conn("plain-srv", "10.0.0.1"),
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), Some(user.path()), empty.path());
        // Connections without a group field don't appear in group list
        let groups = cfg.discover_groups();
        assert!(groups.is_empty());
        list(&cfg, &no_color()).unwrap();
    }

    // ── group current ─────────────────────────────────────────────────────────

    #[test]
    fn test_group_current_no_lock_no_error() {
        let cwd = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        let cfg = load(cwd.path(), None, empty.path());
        assert!(cfg.group.is_none());
        current(&cfg, &no_color()).unwrap();
    }

    #[test]
    fn test_group_current_active_group_from_file_no_error() {
        let cwd = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            &conn_with_group("work-srv", "10.0.0.1", "work"),
        );
        let empty = TempDir::new().unwrap();
        // Explicitly request the "work" group lock (as if session.yml said so)
        let cfg = config::load_impl(
            cwd.path(),
            Some("work"),
            true,
            Some(user.path()),
            empty.path(),
        )
        .unwrap();
        assert_eq!(cfg.group.as_deref(), Some("work"));
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

    /// Switch group: session.yml is written with the new group lock.
    #[test]
    fn test_use_group_writes_session() {
        let session_dir = TempDir::new().unwrap();
        let session_path = session_dir.path().join("session.yml");

        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            &conn_with_group("work-srv", "10.0.0.1", "work"),
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(
            TempDir::new().unwrap().path(),
            Some(user.path()),
            empty.path(),
        );

        use_group_impl("work", &session_path, &cfg, &no_color()).unwrap();

        let ag = group::read_session_at(&session_path).unwrap();
        assert_eq!(ag.name.as_deref(), Some("work"));
        assert!(ag.from_file);
    }

    /// Use unknown group: warning emitted but operation still succeeds and
    /// session is written (does not block).
    #[test]
    fn test_use_group_unknown_group_does_not_block() {
        let session_dir = TempDir::new().unwrap();
        let session_path = session_dir.path().join("session.yml");

        let cwd = TempDir::new().unwrap();
        let empty = TempDir::new().unwrap();
        // Empty config — no connections with any group
        let cfg = load(cwd.path(), None, empty.path());

        // Must return Ok — warning is emitted but group lock is still set.
        let result = use_group_impl("no-such-group", &session_path, &cfg, &no_color());
        assert!(result.is_ok());

        // Session was written despite the warning.
        let ag = group::read_session_at(&session_path).unwrap();
        assert_eq!(ag.name.as_deref(), Some("no-such-group"));
    }

    /// Use group that exists in connections: no warning, session written.
    #[test]
    fn test_use_group_existing_group_no_warning() {
        let session_dir = TempDir::new().unwrap();
        let session_path = session_dir.path().join("session.yml");

        let user = TempDir::new().unwrap();
        write_yaml(
            user.path(),
            "connections.yaml",
            &conn_with_group("work-srv", "10.0.0.1", "work"),
        );
        let empty = TempDir::new().unwrap();
        let cfg = load(
            TempDir::new().unwrap().path(),
            Some(user.path()),
            empty.path(),
        );

        use_group_impl("work", &session_path, &cfg, &no_color()).unwrap();

        let ag = group::read_session_at(&session_path).unwrap();
        assert_eq!(ag.name.as_deref(), Some("work"));
    }

    // ── group clear ───────────────────────────────────────────────────────────

    /// Clear group: active_group is removed from session.yml, reverts to no lock.
    #[test]
    fn test_clear_removes_active_group() {
        let session_dir = TempDir::new().unwrap();
        let session_path = session_dir.path().join("session.yml");

        // Write a group first.
        group::write_session_at(&session_path, Some("work")).unwrap();
        let ag = group::read_session_at(&session_path).unwrap();
        assert_eq!(ag.name.as_deref(), Some("work"));

        // Clear it.
        clear_impl(&session_path).unwrap();

        let ag = group::read_session_at(&session_path).unwrap();
        assert!(ag.name.is_none());
        assert!(!ag.from_file);
    }

    #[test]
    fn test_clear_on_absent_session_succeeds() {
        let session_dir = TempDir::new().unwrap();
        let session_path = session_dir.path().join("session.yml");
        // File does not exist — clear should still succeed.
        clear_impl(&session_path).unwrap();
        let ag = group::read_session_at(&session_path).unwrap();
        assert!(ag.name.is_none());
    }
}
